use std::cell::Cell;
use std::collections::VecDeque;

use rp_hal_api::BlockError;

use super::*;

thread_local! {
    /// Card reads and writes so far on this test's thread: DMA events note
    /// it, to show what the card did while a transfer was in flight.
    static CARD_OPS: Cell<usize> = const { Cell::new(0) };
}

fn card_ops() -> usize {
    CARD_OPS.with(Cell::get)
}

/// A bulk pipe that records IN packets and plays queued OUT packets; with
/// `dma`, transfers end `delay` polls after they start.
#[derive(Default)]
struct Pipe {
    out: VecDeque<Vec<u8>>,
    ins: Vec<Vec<u8>>,
    busy: bool,
    dma: bool,
    refuse: bool,
    fail: bool,
    delay: u32,
    /// A send in flight: its packets and polls left.
    sending: Option<(Vec<u8>, u32)>,
    /// A receive in flight: polls left.
    receiving: Option<u32>,
    /// (started, ended): card operations when each transfer did.
    spans: Vec<(usize, usize)>,
    aborted: u32,
}

impl Pipe {
    fn with_dma(delay: u32) -> Self {
        Self {
            dma: true,
            delay,
            ..Self::default()
        }
    }
}

impl BulkPipe for Pipe {
    fn read_packet(&mut self, buf: &mut [u8; PACKET]) -> Option<usize> {
        let p = self.out.pop_front()?;
        buf[..p.len()].copy_from_slice(&p);
        Some(p.len())
    }
    fn can_write(&mut self) -> bool {
        if self.sending.is_some() || self.receiving.is_some() {
            return false;
        }
        // Alternate busy/free, as a real endpoint with one packet in flight.
        self.busy = !self.busy;
        !self.busy
    }
    fn write_packet(&mut self, data: &[u8]) {
        assert!(data.len() <= PACKET);
        assert!(self.sending.is_none(), "a packet written during DMA");
        self.ins.push(data.to_vec());
    }
    fn send_dma(&mut self, data: &[u8]) -> bool {
        if !self.dma || self.refuse {
            return false;
        }
        assert!(self.sending.is_none() && self.receiving.is_none());
        assert!(data.len().is_multiple_of(PACKET) && !data.is_empty());
        self.sending = Some((data.to_vec(), self.delay));
        self.spans.push((card_ops(), usize::MAX));
        true
    }
    fn receive_dma(&mut self, buf: &mut [u8]) -> bool {
        if !self.dma || self.refuse {
            return false;
        }
        assert!(self.sending.is_none() && self.receiving.is_none());
        assert!(buf.len().is_multiple_of(PACKET) && !buf.is_empty());
        let mut at = 0;
        while at < buf.len() {
            let p = self.out.pop_front().expect("host data for the DMA");
            assert_eq!(p.len(), PACKET, "DMA needs whole packets");
            buf[at..at + PACKET].copy_from_slice(&p);
            at += PACKET;
        }
        self.receiving = Some(self.delay);
        self.spans.push((card_ops(), usize::MAX));
        true
    }
    fn dma_done(&mut self) -> Option<bool> {
        if let Some((data, left)) = &mut self.sending {
            if *left > 0 {
                *left -= 1;
                return None;
            }
            for p in data.chunks(PACKET) {
                self.ins.push(p.to_vec());
            }
            self.sending = None;
        } else if let Some(left) = &mut self.receiving {
            if *left > 0 {
                *left -= 1;
                return None;
            }
            self.receiving = None;
        } else {
            panic!("dma_done with nothing in flight");
        }
        if let Some(s) = self.spans.last_mut() {
            s.1 = card_ops();
        }
        Some(!self.fail)
    }
    fn dma_abort(&mut self) {
        self.sending = None;
        self.receiving = None;
        self.aborted += 1;
    }
}

/// A disk in memory; every access is checked against its size.
struct Disk {
    data: Vec<u8>,
    reads: usize,
    writes: usize,
    read_only: bool,
    fail_reads: bool,
    /// Writes to this block are refused as write-protected (a guarded
    /// block on a writable drive); `fail_writes` fails them as errors.
    guarded: Option<u32>,
    fail_writes: bool,
    present: bool,
    /// Reads like a DMA card: `wait` runs while it reads (item 58).
    overlap: bool,
    waits: usize,
    /// Reads of any range that reaches this block fail.
    fail_from: Option<u32>,
}

impl Disk {
    fn new(blocks: usize) -> Self {
        let data = (0..blocks * BLOCK)
            .map(|i| (i / BLOCK) as u8 ^ i as u8)
            .collect();
        Self {
            data,
            reads: 0,
            writes: 0,
            read_only: false,
            fail_reads: false,
            guarded: None,
            fail_writes: false,
            present: true,
            overlap: false,
            waits: 0,
            fail_from: None,
        }
    }
}

impl BlockDevice for Disk {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
        CARD_OPS.with(|c| c.set(c.get() + 1));
        let o = lba as usize * BLOCK;
        assert!(o + buf.len() <= self.data.len(), "read past the end");
        self.reads += 1;
        let n = (buf.len() / BLOCK) as u32;
        if self.fail_reads || self.fail_from.is_some_and(|f| lba + n > f) {
            return Err(BlockError::Device);
        }
        buf.copy_from_slice(&self.data[o..o + buf.len()]);
        Ok(())
    }
    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
        CARD_OPS.with(|c| c.set(c.get() + 1));
        let o = lba as usize * BLOCK;
        assert!(o + buf.len() <= self.data.len(), "write past the end");
        self.writes += 1;
        let n = (buf.len() / BLOCK) as u32;
        if self.guarded.is_some_and(|g| (lba..lba + n).contains(&g)) {
            return Err(BlockError::ReadOnly);
        }
        if self.fail_writes {
            return Err(BlockError::Device);
        }
        self.data[o..o + buf.len()].copy_from_slice(buf);
        Ok(())
    }
}

impl Storage for Disk {
    fn blocks(&self) -> u32 {
        (self.data.len() / BLOCK) as u32
    }
    fn present(&self) -> bool {
        self.present
    }
    fn read_only(&self) -> bool {
        self.read_only
    }
    fn product(&self) -> &[u8; 16] {
        b"SD card         "
    }
    fn overlaps(&self) -> bool {
        self.overlap
    }
    fn read_blocks_while(
        &mut self,
        lba: u32,
        buf: &mut [u8],
        wait: &mut dyn FnMut(),
    ) -> Result<(), BlockError> {
        // A DMA read: the hook runs a few times before the data is in.
        for _ in 0..3 {
            wait();
            self.waits += 1;
        }
        self.read_blocks(lba, buf)
    }
}

const SERIAL: &[u8] = b"5250323630303031";

fn cbw(tag: u32, len: u32, dir_in: bool, lun: u8, cb: &[u8]) -> Vec<u8> {
    let mut c = vec![0u8; 31];
    c[..4].copy_from_slice(&0x4342_5355u32.to_le_bytes());
    c[4..8].copy_from_slice(&tag.to_le_bytes());
    c[8..12].copy_from_slice(&len.to_le_bytes());
    c[12] = if dir_in { 0x80 } else { 0 };
    c[13] = lun;
    c[14] = cb.len() as u8;
    c[15..15 + cb.len()].copy_from_slice(cb);
    c
}

struct Csw {
    tag: u32,
    residue: u32,
    status: u8,
}

/// Runs one command: CBW, OUT data, then polls until the CSW. Returns the
/// IN data and the CSW.
fn run(
    msc: &mut Msc<'_>,
    disk: &mut Disk,
    len: u32,
    dir_in: bool,
    cb: &[u8],
    data_out: &[u8],
) -> (Vec<u8>, Csw) {
    run_lun(msc, disk, 0, len, dir_in, cb, data_out)
}

fn run_lun(
    msc: &mut Msc<'_>,
    disk: &mut Disk,
    lun: u8,
    len: u32,
    dir_in: bool,
    cb: &[u8],
    data_out: &[u8],
) -> (Vec<u8>, Csw) {
    run_at(msc, disk, 1000, lun, len, dir_in, cb, data_out)
}

/// A fresh host's mount: READ CAPACITY first, as Windows, Linux and macOS
/// do, so the unit takes its view as current (no start-up away).
fn mount(msc: &mut Msc<'_>, disk: &mut Disk) {
    let (_, csw) = run(msc, disk, 8, true, &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[]);
    assert_eq!(csw.status, 0, "READ CAPACITY on mount");
}

/// TEST UNIT READY at `now_ms`: its status and the sense that follows.
fn tur_at(msc: &mut Msc<'_>, disk: &mut Disk, now_ms: u32) -> (u8, (u8, u8, u8)) {
    let (_, csw) = run_at(msc, disk, now_ms, 0, 0, false, &[0; 6], &[]);
    let (d, _) = run_at(msc, disk, now_ms, 0, 18, true, &[0x03, 0, 0, 0, 18, 0], &[]);
    (csw.status, (d[2] & 0x0F, d[12], d[13]))
}

#[allow(clippy::too_many_arguments)]
fn run_at(
    msc: &mut Msc<'_>,
    disk: &mut Disk,
    now_ms: u32,
    lun: u8,
    len: u32,
    dir_in: bool,
    cb: &[u8],
    data_out: &[u8],
) -> (Vec<u8>, Csw) {
    run_with(
        &mut Pipe::default(),
        msc,
        disk,
        now_ms,
        lun,
        len,
        dir_in,
        cb,
        data_out,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_with(
    pipe: &mut Pipe,
    msc: &mut Msc<'_>,
    disk: &mut Disk,
    now_ms: u32,
    lun: u8,
    len: u32,
    dir_in: bool,
    cb: &[u8],
    data_out: &[u8],
) -> (Vec<u8>, Csw) {
    pipe.ins.clear();
    pipe.out.push_back(cbw(0xC0FFEE, len, dir_in, lun, cb));
    for p in data_out.chunks(PACKET) {
        pipe.out.push_back(p.to_vec());
    }
    for _ in 0..100_000 {
        msc.poll(pipe, &mut [disk], now_ms);
        if let Some(last) = pipe.ins.last()
            && last.len() == 13
            && last[..4] == 0x5342_5355u32.to_le_bytes()
        {
            let last = pipe.ins.pop().unwrap();
            let data = pipe.ins.concat();
            return (
                data,
                Csw {
                    tag: u32::from_le_bytes(last[4..8].try_into().unwrap()),
                    residue: u32::from_le_bytes(last[8..12].try_into().unwrap()),
                    status: last[12],
                },
            );
        }
    }
    panic!("no CSW");
}

fn sense(msc: &mut Msc<'_>, disk: &mut Disk) -> (u8, u8, u8) {
    let (d, csw) = run(msc, disk, 18, true, &[0x03, 0, 0, 0, 18, 0], &[]);
    assert_eq!(csw.status, 0);
    (d[2], d[12], d[13])
}

fn read10(lba: u32, count: u16) -> Vec<u8> {
    let mut cb = vec![0x28, 0];
    cb.extend_from_slice(&lba.to_be_bytes());
    cb.push(0);
    cb.extend_from_slice(&count.to_be_bytes());
    cb.push(0);
    cb
}

fn write10(lba: u32, count: u16) -> Vec<u8> {
    let mut cb = read10(lba, count);
    cb[0] = 0x2A;
    cb
}

#[test]
fn inquiry_reports_a_removable_retropane_drive() {
    let mut buf = vec![0u8; 4 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(16);
    mount(&mut msc, &mut disk);
    let (d, csw) = run(&mut msc, &mut disk, 36, true, &[0x12, 0, 0, 0, 36, 0], &[]);
    assert_eq!((csw.tag, csw.residue, csw.status), (0xC0FFEE, 0, 0));
    assert_eq!(d.len(), 36);
    assert_eq!(d[1], 0x80, "removable");
    assert_eq!(&d[8..16], b"RetroPan");
    assert_eq!(&d[16..32], b"SD card         ");
    // Vital product data: the serial number.
    let (d, _) = run(
        &mut msc,
        &mut disk,
        255,
        true,
        &[0x12, 1, 0x80, 0, 255, 0],
        &[],
    );
    assert_eq!(&d[4..], SERIAL);
    let (d, _) = run(
        &mut msc,
        &mut disk,
        255,
        true,
        &[0x12, 1, 0x00, 0, 255, 0],
        &[],
    );
    assert_eq!(&d[4..6], &[0x00, 0x80]);
    // A shorter allocation is honoured, and the residue reported.
    let (d, csw) = run(&mut msc, &mut disk, 5, true, &[0x12, 0, 0, 0, 5, 0], &[]);
    assert_eq!((d.len(), csw.residue), (5, 0));
}

#[test]
fn capacity_and_format_capacities() {
    let mut buf = vec![0u8; 4 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(16);
    mount(&mut msc, &mut disk);
    let (d, _) = run(
        &mut msc,
        &mut disk,
        8,
        true,
        &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[],
    );
    assert_eq!(be32(&d[0..4]), 15);
    assert_eq!(be32(&d[4..8]), 512);
    let (d, _) = run(
        &mut msc,
        &mut disk,
        252,
        true,
        &[0x23, 0, 0, 0, 0, 0, 0, 0, 252, 0],
        &[],
    );
    assert_eq!(be32(&d[4..8]), 16);
    assert_eq!(d[8], 0x02, "formatted media");
}

#[test]
fn reads_and_writes_blocks_through_a_small_buffer() {
    let mut buf = vec![0u8; 2 * BLOCK]; // forces refills
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(16);
    mount(&mut msc, &mut disk);
    let want = disk.data[3 * BLOCK..8 * BLOCK].to_vec();
    let (d, csw) = run(&mut msc, &mut disk, 5 * 512, true, &read10(3, 5), &[]);
    assert_eq!((csw.status, csw.residue), (0, 0));
    assert_eq!(d, want);
    assert_eq!(disk.reads, 3, "2 + 2 + 1 blocks");

    let new: Vec<u8> = (0..5 * BLOCK).map(|i| (i * 7) as u8).collect();
    let (_, csw) = run(&mut msc, &mut disk, 5 * 512, false, &write10(9, 5), &new);
    assert_eq!((csw.status, csw.residue), (0, 0));
    assert_eq!(&disk.data[9 * BLOCK..14 * BLOCK], &new[..]);
    assert_eq!(msc.stats().blocks_written, 5);
    assert!(msc.host_writing(1000 + HOST_WRITE_GRACE_MS - 1));
    assert!(!msc.host_writing(1000 + HOST_WRITE_GRACE_MS));
}

#[test]
fn out_of_range_and_wrapping_requests_never_touch_the_card() {
    let mut buf = vec![0u8; 2 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(16);
    mount(&mut msc, &mut disk);
    for (lba, count) in [(15, 2), (16, 1), (u32::MAX, 2), (u32::MAX - 1, 0xFFFF)] {
        let len = u32::from(count) * 512;
        let (d, csw) = run(&mut msc, &mut disk, len, true, &read10(lba, count), &[]);
        assert_eq!(csw.status, 1, "lba {lba} count {count}");
        assert!(d.is_empty());
        assert_eq!(csw.residue, len);
        assert_eq!(sense(&mut msc, &mut disk), (0x05, 0x21, 0x00));
        let data = vec![0u8; len.min(4096) as usize];
        let (_, csw) = run(
            &mut msc,
            &mut disk,
            len.min(4096),
            false,
            &write10(lba, count),
            &data,
        );
        assert_eq!(csw.status, 1);
    }
    assert_eq!((disk.reads, disk.writes), (0, 0));
}

#[test]
fn a_mismatched_transfer_is_refused_without_card_access() {
    let mut buf = vec![0u8; 2 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(16);
    mount(&mut msc, &mut disk);
    // READ of 2 blocks, but the host asks for 512 bytes, or sends OUT.
    let (_, csw) = run(&mut msc, &mut disk, 512, true, &read10(0, 2), &[]);
    assert_eq!(csw.status, 1);
    let (_, csw) = run(&mut msc, &mut disk, 1024, false, &read10(0, 2), &[0; 1024]);
    assert_eq!(csw.status, 1);
    assert_eq!(disk.reads, 0);
}

#[test]
fn malformed_cbws_are_ignored_and_the_next_command_works() {
    let mut buf = vec![0u8; 2 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(16);
    mount(&mut msc, &mut disk);
    let mut pipe = Pipe::default();
    let mut bad = cbw(1, 0, false, 0, &[0]);
    bad[0] ^= 1;
    pipe.out.push_back(bad);
    pipe.out.push_back(vec![0x55; 30]);
    pipe.out.push_back(vec![]);
    pipe.out.push_back(vec![0xFF; 64]);
    for _ in 0..10 {
        msc.poll(&mut pipe, &mut [&mut disk], 0);
    }
    assert!(pipe.ins.is_empty());
    assert_eq!(msc.stats().bad_cbw, 4);
    let (_, csw) = run(&mut msc, &mut disk, 0, false, &[0; 6], &[]);
    assert_eq!(csw.status, 0);
}

#[test]
fn unknown_commands_and_bad_luns_fail_with_sense() {
    let mut buf = vec![0u8; 2 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(16);
    mount(&mut msc, &mut disk);
    let (_, csw) = run(&mut msc, &mut disk, 0, false, &[0xC5, 0, 0, 0, 0, 0], &[]);
    assert_eq!(csw.status, 1);
    assert_eq!(sense(&mut msc, &mut disk), (0x05, 0x20, 0x00));
    assert_eq!(
        sense(&mut msc, &mut disk),
        (0, 0, 0),
        "sense is cleared once read"
    );
    let (_, csw) = run_lun(&mut msc, &mut disk, 7, 0, false, &[0; 6], &[]);
    assert_eq!(csw.status, 1);
    assert_eq!(msc.max_lun(), 0);
}

#[test]
fn data_the_command_doesnt_use_is_drained_or_ended_short() {
    let mut buf = vec![0u8; 2 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(16);
    mount(&mut msc, &mut disk);
    // TEST UNIT READY with the host expecting 8 bytes in: an empty packet.
    let (d, csw) = run(&mut msc, &mut disk, 8, true, &[0; 6], &[]);
    assert!(d.is_empty());
    assert_eq!((csw.status, csw.residue), (0, 8));
    // ... or sending 100 bytes out: consumed.
    let (_, csw) = run(&mut msc, &mut disk, 100, false, &[0; 6], &[7; 100]);
    assert_eq!((csw.status, csw.residue), (0, 0));
    // A full-packet reply shorter than asked ends with a zero-length packet.
    let mut pipe = Pipe::default();
    pipe.out
        .push_back(cbw(2, 255, true, 0, &[0x1A, 0, 0x3F, 0, 255, 0]));
    for _ in 0..50 {
        msc.poll(&mut pipe, &mut [&mut disk], 0);
    }
    assert_eq!(pipe.ins[0].len(), 24, "header + caching page");
}

#[test]
fn write_protection_is_honoured_and_reported() {
    let mut buf = vec![0u8; 2 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(16);
    mount(&mut msc, &mut disk);
    disk.read_only = true;
    let before = disk.data.clone();
    let (_, csw) = run(&mut msc, &mut disk, 512, false, &write10(1, 1), &[9; 512]);
    assert_eq!(csw.status, 1);
    assert_eq!(sense(&mut msc, &mut disk), (0x07, 0x27, 0x00));
    assert_eq!(disk.data, before);
    let (d, _) = run(&mut msc, &mut disk, 4, true, &[0x1A, 0, 0, 0, 4, 0], &[]);
    assert_eq!(d[2] & 0x80, 0x80, "MODE SENSE write-protect bit");
}

/// A writable drive whose storage refuses one write (the loader's boot-file
/// guard) reports that write as write-protected, so the PC says "write
/// protected" rather than an I/O error; other failures stay write errors.
#[test]
fn a_refused_block_is_reported_as_write_protected() {
    let mut buf = vec![0u8; 2 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(16);
    mount(&mut msc, &mut disk);
    disk.guarded = Some(3);
    let before = disk.data.clone();
    let (_, csw) = run(&mut msc, &mut disk, 512, false, &write10(3, 1), &[9; 512]);
    assert_eq!(csw.status, 1);
    assert_eq!(sense(&mut msc, &mut disk), (0x07, 0x27, 0x00));
    assert_eq!(disk.data, before);
    let (_, csw) = run(&mut msc, &mut disk, 512, false, &write10(5, 1), &[9; 512]);
    assert_eq!(csw.status, 0, "other blocks are writable");
    disk.fail_writes = true;
    let (_, csw) = run(&mut msc, &mut disk, 512, false, &write10(6, 1), &[9; 512]);
    assert_eq!(csw.status, 1);
    assert_eq!(sense(&mut msc, &mut disk), (0x03, 0x0C, 0x00));
}

#[test]
fn a_device_side_change_takes_the_medium_away_until_the_host_notices() {
    let mut buf = vec![0u8; 2 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(16);
    mount(&mut msc, &mut disk);
    // The host has seen the medium (it may have cached the volume).
    assert_eq!(tur_at(&mut msc, &mut disk, 0).0, 0);
    msc.medium_changed(0);
    assert!(msc.away(0));
    // INQUIRY works whatever the state.
    let (_, csw) = run(&mut msc, &mut disk, 36, true, &[0x12, 0, 0, 0, 36, 0], &[]);
    assert_eq!(csw.status, 0);
    // Away: "no medium", however soon the host asks again...
    assert_eq!(tur_at(&mut msc, &mut disk, 1000), (1, (0x02, 0x3A, 0x00)));
    assert_eq!(tur_at(&mut msc, &mut disk, 1500), (1, (0x02, 0x3A, 0x00)));
    // ...until it has seen that twice and AWAY_MIN_MS has passed: then
    // back, with the change reported once.
    assert_eq!(
        tur_at(&mut msc, &mut disk, 1000 + AWAY_MIN_MS),
        (1, (0x06, 0x28, 0x00))
    );
    assert!(!msc.away(0));
    assert_eq!(tur_at(&mut msc, &mut disk, 1000 + AWAY_MIN_MS).0, 0);
    // Eject: not present until the device offers it again.
    run(
        &mut msc,
        &mut disk,
        0,
        false,
        &[0x1B, 0, 0, 0, 0x02, 0],
        &[],
    );
    assert!(msc.ejected(0));
    let (_, csw) = run(&mut msc, &mut disk, 0, false, &[0; 6], &[]);
    assert_eq!(csw.status, 1);
    assert_eq!(sense(&mut msc, &mut disk), (0x02, 0x3A, 0x00));
    msc.medium_changed(0);
    assert!(!msc.ejected(0));
    // The host saw it ejected (nothing cached): no away time, the change
    // is reported at once.
    assert!(!msc.away(0));
    assert_eq!(
        tur_at(&mut msc, &mut disk, 5000),
        (1, (0x06, 0x28, 0x00)),
        "unit attention at once"
    );
    assert_eq!(tur_at(&mut msc, &mut disk, 5000).0, 0);
}

/// The CF card class (owner, 2026-10-15: "USB Drive (J:)" with no size, a
/// broken icon for minutes): a unit the host only ever saw empty, or never
/// asked about, gets its medium at once when one arrives (a card attached
/// after enumeration, inserted, or a retry after a failure), never seconds
/// of extra "no medium" it doesn't need: it has nothing cached to drop.
#[test]
fn a_medium_arriving_where_the_host_saw_none_is_offered_at_once() {
    let mut buf = vec![0u8; 2 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(16);
    // Never asked: no away. A fresh host's mount (READ CAPACITY) hears of
    // the change once, then works.
    msc.medium_changed(0);
    assert!(!msc.away(0));
    let (_, csw) = run(
        &mut msc,
        &mut disk,
        8,
        true,
        &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[],
    );
    assert_eq!(csw.status, 1);
    assert_eq!(sense(&mut msc, &mut disk), (0x06, 0x28, 0x00));
    mount(&mut msc, &mut disk);
    assert_eq!(tur_at(&mut msc, &mut disk, 0).0, 0);
    // Seen empty, then a card arrives: unit attention, then ready.
    disk.present = false;
    assert_eq!(tur_at(&mut msc, &mut disk, 100), (1, (0x02, 0x3A, 0x00)));
    disk.present = true;
    msc.medium_changed(0);
    assert!(!msc.away(0));
    assert_eq!(tur_at(&mut msc, &mut disk, 200), (1, (0x06, 0x28, 0x00)));
    assert_eq!(tur_at(&mut msc, &mut disk, 200).0, 0);
    // Seen present, then changed: away as before (the host's cache).
    msc.medium_changed(0);
    assert!(msc.away(0));
    // A second change while away keeps it away (still unseen).
    assert_eq!(tur_at(&mut msc, &mut disk, 300).0, 1);
    msc.medium_changed(0);
    assert!(msc.away(0));
}

/// A host that isn't polling still gets the medium back, after
/// AWAY_MAX_MS; time alone (AWAY_MIN_MS) isn't enough while it hasn't
/// looked, and neither is looking without the time.
#[test]
fn a_changed_medium_comes_back_only_once_the_host_noticed_or_after_the_ceiling() {
    let mut buf = vec![0u8; 2 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(16);
    mount(&mut msc, &mut disk);
    assert_eq!(tur_at(&mut msc, &mut disk, 0).0, 0);
    msc.medium_changed(0);
    // The away time starts at the first poll after the change.
    assert_eq!(tur_at(&mut msc, &mut disk, 0).0, 1);
    assert_eq!(tur_at(&mut msc, &mut disk, AWAY_MIN_MS - 1).0, 1);
    assert!(msc.away(0), "seen twice, but too soon");
    msc.medium_changed(0);
    assert_eq!(tur_at(&mut msc, &mut disk, 100_000).0, 1);
    // Unseen, well past the minimum: still away.
    let mut pipe = Pipe::default();
    msc.poll(&mut pipe, &mut [&mut disk], 100_000 + AWAY_MAX_MS - 1);
    assert!(msc.away(0), "seen once only");
    msc.poll(&mut pipe, &mut [&mut disk], 100_000 + AWAY_MAX_MS);
    assert!(!msc.away(0), "the ceiling");
    assert_eq!(
        tur_at(&mut msc, &mut disk, 100_000 + AWAY_MAX_MS),
        (1, (0x06, 0x28, 0x00))
    );
}

#[test]
fn a_card_error_mid_read_ends_the_transfer_with_sense() {
    let mut buf = vec![0u8; 2 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(16);
    mount(&mut msc, &mut disk);
    disk.fail_reads = true;
    let (d, csw) = run(&mut msc, &mut disk, 4 * 512, true, &read10(0, 4), &[]);
    assert!(d.is_empty());
    assert_eq!((csw.status, csw.residue), (1, 4 * 512));
    assert_eq!(sense(&mut msc, &mut disk), (0x03, 0x11, 0x00));
    assert_eq!(msc.stats().io_errors, 1);
    assert_eq!(
        msc.stats().last_error,
        (0, 2),
        "first chunk of the 2-block buffer"
    );
}

#[test]
fn no_medium_means_not_ready() {
    let mut buf = vec![0u8; 2 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(16);
    mount(&mut msc, &mut disk);
    disk.present = false;
    let (_, csw) = run(&mut msc, &mut disk, 512, true, &read10(0, 1), &[]);
    assert_eq!(csw.status, 1);
    assert_eq!(sense(&mut msc, &mut disk), (0x02, 0x3A, 0x00));
    assert_eq!(disk.reads, 0);
}

/// Hostile input: thousands of random CBWs (valid signature, random
/// everything else) and random packets. Nothing may panic, every valid CBW
/// must end in a CSW, and the disk's own bounds checks must never trip.
#[test]
fn random_commands_never_panic_or_reach_past_the_card() {
    let mut buf = vec![0u8; 4 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(64);
    mount(&mut msc, &mut disk);
    let mut x: u64 = 0x2026_1012;
    let mut rnd = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for i in 0..5000 {
        let mut cb = [0u8; 16];
        for b in &mut cb {
            *b = rnd() as u8;
        }
        // Bias towards the commands that touch the card.
        cb[0] = [0x28, 0x2A, 0x12, 0x1A, 0x5A, 0x25, 0x23, 0x1B, 0x03, cb[0]][i % 10];
        let small = rnd() % 4 == 0;
        let len = if small {
            (rnd() % 300) as u32
        } else {
            (rnd() % 8) as u32 * 512
        };
        let dir_in = rnd() % 2 == 0;
        let lun = (rnd() % 3) as u8;
        let data: Vec<u8> = if dir_in {
            vec![]
        } else {
            vec![0xA5; len as usize]
        };
        let (_, csw) = run_lun(
            &mut msc,
            &mut disk,
            lun,
            len,
            dir_in,
            &cb[..(1 + rnd() % 16) as usize],
            &data,
        );
        assert!(csw.status <= 1);
        assert!(csw.residue <= len);
        // Media changes now and then.
        if i % 97 == 0 {
            msc.medium_changed(0);
        }
    }
    assert!(msc.stats().commands >= 5000);
}

/// Busy from the command's arrival until its status has gone: what tells a
/// caller not to sleep mid-transfer (the loader napped between packets of a
/// large read until it asked this).
#[test]
fn busy_from_command_to_status() {
    let mut buf = vec![0u8; 4096];
    let mut msc = Msc::new(&mut buf, b"0123456789AB", 1);
    let mut disk = Disk::new(64);
    mount(&mut msc, &mut disk);
    assert!(!msc.busy());
    let mut pipe = Pipe::default();
    pipe.out
        .push_back(cbw(1, 16 * 512, true, 0, &read10(0, 16)));
    msc.poll(&mut pipe, &mut [&mut disk], 0);
    assert!(msc.busy(), "mid-read");
    for _ in 0..10_000 {
        msc.poll(&mut pipe, &mut [&mut disk], 0);
        if !msc.busy() {
            break;
        }
    }
    assert!(!msc.busy());
    let csw = pipe.ins.last().unwrap();
    assert_eq!(csw.len(), 13, "the status went before busy cleared");
}

/// Item 58: with a card that overlaps (DMA), a long read streams exactly
/// the card's bytes while the next chunk is read behind the one going out.
#[test]
fn long_reads_overlap_and_stream_the_card_exactly() {
    let mut buf = vec![0u8; 4 * BLOCK]; // halves of 2 blocks
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(64);
    mount(&mut msc, &mut disk);
    disk.overlap = true;
    let want = disk.data.clone();
    let (d, csw) = run(&mut msc, &mut disk, 64 * 512, true, &read10(0, 64), &[]);
    assert_eq!((csw.status, csw.residue), (0, 0));
    assert!(d == want, "the stream differs from the card");
    assert_eq!(disk.reads, 32, "two blocks per read, nothing read twice");
    assert!(disk.waits > 0, "reads ran the hook");
}

/// A unit that doesn't overlap (the CF card) reads right after one that does
/// (the SD card) left the buffer's front on the second half: the data sent is
/// what was read (the PC saw all zeros for the CF drive, 2026-10-16).
#[test]
fn a_unit_without_overlap_sends_what_it_read_whatever_the_front() {
    for blocks in [1usize, 2, 3, 8] {
        for first in [3usize, 4, 5, 6, 7] {
            let mut buf = vec![0u8; 4 * BLOCK];
            let mut msc = Msc::new(&mut buf, SERIAL, 1);
            let mut disk = Disk::new(64);
            mount(&mut msc, &mut disk);
            disk.overlap = true;
            let want = disk.data.clone();
            let (d, _) = run(
                &mut msc,
                &mut disk,
                (first * 512) as u32,
                true,
                &read10(0, first as u16),
                &[],
            );
            assert!(d == want[..first * 512]);
            disk.overlap = false;
            let (d, csw) = run(
                &mut msc,
                &mut disk,
                (blocks * 512) as u32,
                true,
                &read10(40, blocks as u16),
                &[],
            );
            assert_eq!(csw.status, 0);
            assert!(
                d == want[40 * 512..(40 + blocks) * 512],
                "sent other data than read: {first} blocks, then {blocks}"
            );
        }
    }
}

/// The blocks after a bulk read are read ahead and used by the next read
/// that starts there; a write drops them, so a read after it sees the card.
#[test]
fn read_ahead_serves_the_next_read_and_a_write_drops_it() {
    let mut buf = vec![0u8; 4 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(64);
    mount(&mut msc, &mut disk);
    disk.overlap = true;
    let want = disk.data.clone();
    let (d, _) = run(&mut msc, &mut disk, 8 * 512, true, &read10(0, 8), &[]);
    assert!(d == want[..8 * 512]);
    let (d, csw) = run(&mut msc, &mut disk, 8 * 512, true, &read10(8, 8), &[]);
    assert_eq!(csw.status, 0);
    assert!(d == want[8 * 512..16 * 512]);
    assert_eq!(msc.stats().read_ahead_hits, 1);
    // Blocks 16-17 were read ahead; the host writes block 16.
    let new = vec![0xA5u8; 512];
    let (_, csw) = run(&mut msc, &mut disk, 512, false, &write10(16, 1), &new);
    assert_eq!(csw.status, 0);
    let (d, _) = run(&mut msc, &mut disk, 2 * 512, true, &read10(16, 2), &[]);
    assert!(d[..512] == new[..], "served stale read-ahead after a write");
    assert_eq!(msc.stats().read_ahead_hits, 1);
}

/// The device changes the card itself (a `put`), then says so: blocks read
/// ahead before the change are never served.
#[test]
fn a_medium_change_drops_the_read_ahead() {
    let mut buf = vec![0u8; 4 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(64);
    mount(&mut msc, &mut disk);
    disk.overlap = true;
    let _ = run(&mut msc, &mut disk, 4 * 512, true, &read10(0, 4), &[]);
    disk.data[4 * 512..5 * 512].fill(0x5A);
    msc.medium_changed(0);
    // The host notices the change (away, then back).
    let mut t = 1000;
    while tur_at(&mut msc, &mut disk, t).0 != 0 {
        t += 1000;
    }
    let (d, csw) = run_at(&mut msc, &mut disk, t, 0, 512, true, &read10(4, 1), &[]);
    assert_eq!(csw.status, 0);
    assert!(
        d.iter().all(|&b| b == 0x5A),
        "served blocks read before the change"
    );
    assert_eq!(msc.stats().read_ahead_hits, 0);
}

/// A chunk read behind the front that fails: the host gets every block
/// before it, then the transfer ends short with a read error.
#[test]
fn a_failure_behind_the_front_is_reported_after_the_good_blocks() {
    let mut buf = vec![0u8; 4 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(64);
    mount(&mut msc, &mut disk);
    disk.overlap = true;
    disk.fail_from = Some(4);
    let want = disk.data.clone();
    let (d, csw) = run(&mut msc, &mut disk, 8 * 512, true, &read10(0, 8), &[]);
    assert_eq!(csw.status, 1);
    assert!(
        d == want[..4 * 512],
        "blocks before the failure, and only those"
    );
    assert_eq!(csw.residue, 4 * 512);
    assert_eq!(sense(&mut msc, &mut disk), (0x03, 0x11, 0x00));
}

/// The stale-view class (2026-10-15, noise-ri.bin lost): an image wrote
/// the card between loader runs; Windows had re-attached without noticing
/// the device went (hard-wired pull-up) and wrote its old root directory
/// back. A host whose first poll, read or write since the drive started
/// doesn't follow a READ CAPACITY is using a view from before: the medium
/// goes away first (as after a device-side change), so it re-reads. Once
/// per unit: after that it is an ordinary drive.
#[test]
fn a_view_from_before_the_drive_started_is_sent_away_first() {
    for first in [vec![0u8; 6], write10(3, 1), read10(3, 1)] {
        let mut buf = vec![0u8; 2 * BLOCK];
        let mut msc = Msc::new(&mut buf, SERIAL, 1);
        let mut disk = Disk::new(16);
        // INQUIRY and REQUEST SENSE don't count either way.
        let (_, csw) = run(&mut msc, &mut disk, 36, true, &[0x12, 0, 0, 0, 36, 0], &[]);
        assert_eq!(csw.status, 0);
        let (len, dir_in, data) = match first[0] {
            0x2A => (512, false, vec![0xEE; 512]),
            0x28 => (512, true, Vec::new()),
            _ => (0, false, Vec::new()),
        };
        let (_, csw) = run_at(&mut msc, &mut disk, 1000, 0, len, dir_in, &first, &data);
        assert_eq!(csw.status, 1, "op {:#x}: refused", first[0]);
        assert_eq!(sense(&mut msc, &mut disk), (0x02, 0x3A, 0x00));
        assert!(msc.away(0));
        assert_eq!((disk.reads, disk.writes), (0, 0), "the card untouched");
        // Away as after a device-side change, then back with the change
        // reported once, then an ordinary drive (never sent away again).
        assert_eq!(tur_at(&mut msc, &mut disk, 1500).0, 1);
        assert_eq!(
            tur_at(&mut msc, &mut disk, 1000 + AWAY_MIN_MS),
            (1, (0x06, 0x28, 0x00))
        );
        assert_eq!(tur_at(&mut msc, &mut disk, 1000 + AWAY_MIN_MS).0, 0);
        let (_, csw) = run(
            &mut msc,
            &mut disk,
            512,
            false,
            &write10(3, 1),
            &[0xEE; 512],
        );
        assert_eq!(csw.status, 0);
    }
}

/// R22: a unit the loader vouches for (the run that just ended never took
/// its card) serves the host's first poll, read or write at once: the
/// host's cached view is kept, no away time, no unit attention; the other
/// unit is still sent away.
#[test]
fn a_vouched_unit_keeps_the_hosts_view() {
    for first in [vec![0u8; 6], read10(3, 1)] {
        let mut buf = vec![0u8; 2 * BLOCK];
        let mut msc = Msc::new(&mut buf, SERIAL, 1);
        let mut disk = Disk::new(16);
        msc.vouch(0);
        let (len, dir_in) = if first[0] == 0x28 {
            (512, true)
        } else {
            (0, false)
        };
        let (_, csw) = run_at(&mut msc, &mut disk, 1000, 0, len, dir_in, &first, &[]);
        assert_eq!(csw.status, 0, "op {:#x}: not served at once", first[0]);
        assert!(!msc.away(0));
        assert_eq!(tur_at(&mut msc, &mut disk, 1100), (0, (0, 0, 0)));
        // A device-side change afterwards still sends it away.
        msc.medium_changed(0);
        assert_eq!(tur_at(&mut msc, &mut disk, 1200).0, 1);
    }
}

/// A fresh mount (READ CAPACITY first) is never delayed, and an empty
/// slot's first look is just "no medium" (nothing to send away).
#[test]
fn a_fresh_mount_or_an_empty_slot_sees_no_start_up_away() {
    let mut buf = vec![0u8; 2 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(16);
    mount(&mut msc, &mut disk);
    assert!(!msc.away(0));
    assert_eq!(tur_at(&mut msc, &mut disk, 0).0, 0);
    let mut buf = vec![0u8; 2 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    disk.present = false;
    assert_eq!(tur_at(&mut msc, &mut disk, 0), (1, (0x02, 0x3A, 0x00)));
    assert!(!msc.away(0));
    disk.present = true;
    msc.medium_changed(0);
    assert!(!msc.away(0), "seen empty: offered at once");
    assert_eq!(tur_at(&mut msc, &mut disk, 0), (1, (0x06, 0x28, 0x00)));
    assert_eq!(tur_at(&mut msc, &mut disk, 0).0, 0);
}

/// DMA reads (item 74): each half goes out as one transfer, and the card
/// reads the next half while it is in flight; the stream is exact.
#[test]
fn dma_reads_send_a_half_while_the_card_reads_the_next() {
    let mut buf = vec![0u8; 4 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(64);
    mount(&mut msc, &mut disk);
    disk.overlap = true;
    let want = disk.data.clone();
    let mut pipe = Pipe::with_dma(5);
    let (d, csw) = run_with(
        &mut pipe,
        &mut msc,
        &mut disk,
        1000,
        0,
        64 * 512,
        true,
        &read10(0, 64),
        &[],
    );
    assert_eq!((csw.status, csw.residue), (0, 0));
    assert!(d == want, "the stream differs from the card");
    assert_eq!(msc.stats().dma_chunks, 32);
    assert_eq!(disk.reads, 32, "two blocks per read, nothing read twice");
    let overlapped = pipe.spans.iter().filter(|(a, b)| b > a).count();
    assert!(overlapped >= 30, "card reads during sends: {overlapped}");
}

/// DMA writes: each half comes in as one transfer, the next is already
/// arriving while the previous one is written to the card.
#[test]
fn dma_writes_receive_a_half_while_the_card_writes_the_last() {
    let mut buf = vec![0u8; 4 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(64);
    mount(&mut msc, &mut disk);
    let new: Vec<u8> = (0..16 * BLOCK).map(|i| (i * 13 + i / 511) as u8).collect();
    let mut pipe = Pipe::with_dma(3);
    let (_, csw) = run_with(
        &mut pipe,
        &mut msc,
        &mut disk,
        1000,
        0,
        16 * 512,
        false,
        &write10(8, 16),
        &new,
    );
    assert_eq!((csw.status, csw.residue), (0, 0));
    assert!(disk.data[8 * BLOCK..24 * BLOCK] == new[..]);
    assert_eq!(msc.stats().blocks_written, 16);
    assert_eq!(msc.stats().dma_chunks, 8);
    assert_eq!(disk.writes, 8);
    let overlapped = pipe.spans.iter().filter(|(a, b)| b > a).count();
    assert_eq!(
        overlapped, 7,
        "every receive but the first spans a card write"
    );
    assert!(msc.host_writing(1000));
    // A one-block write (smaller than a half) too.
    let one = vec![0x3Cu8; BLOCK];
    let (_, csw) = run_with(
        &mut pipe,
        &mut msc,
        &mut disk,
        1000,
        0,
        512,
        false,
        &write10(40, 1),
        &one,
    );
    assert_eq!(csw.status, 0);
    assert!(disk.data[40 * BLOCK..41 * BLOCK] == one[..]);
}

/// A pipe that refuses DMA gets packets one by one, and the data is the
/// same; a failed write still drains the host's data and reports it.
#[test]
fn refused_or_failing_dma_falls_back_or_fails_cleanly() {
    let mut buf = vec![0u8; 4 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(64);
    mount(&mut msc, &mut disk);
    disk.overlap = true;
    let want = disk.data.clone();
    let mut pipe = Pipe::with_dma(2);
    pipe.refuse = true;
    let (d, csw) = run_with(
        &mut pipe,
        &mut msc,
        &mut disk,
        1000,
        0,
        8 * 512,
        true,
        &read10(0, 8),
        &[],
    );
    assert_eq!(csw.status, 0);
    assert!(d == want[..8 * 512]);
    assert_eq!(msc.stats().dma_chunks, 0);
    let new = vec![0x77u8; 4 * BLOCK];
    let (_, csw) = run_with(
        &mut pipe,
        &mut msc,
        &mut disk,
        1000,
        0,
        4 * 512,
        false,
        &write10(0, 4),
        &new,
    );
    assert_eq!(csw.status, 0);
    assert!(disk.data[..4 * BLOCK] == new[..]);

    // A transfer that fails: the read ends with an error (the host
    // retries), and DMA is off until a reset.
    let mut buf = vec![0u8; 4 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    mount(&mut msc, &mut disk);
    let mut pipe = Pipe::with_dma(1);
    pipe.fail = true;
    let (_, csw) = run_with(
        &mut pipe,
        &mut msc,
        &mut disk,
        1000,
        0,
        8 * 512,
        true,
        &read10(0, 8),
        &[],
    );
    assert_eq!(csw.status, 1);
    assert_eq!(sense(&mut msc, &mut disk), (0x03, 0x11, 0x00));
}

/// A reset while a transfer is in flight aborts it; the next command
/// works (by DMA again).
#[test]
fn a_reset_aborts_the_transfer_in_flight() {
    let mut buf = vec![0u8; 4 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(64);
    mount(&mut msc, &mut disk);
    disk.overlap = true;
    let mut pipe = Pipe::with_dma(1000);
    pipe.out.push_back(cbw(1, 8 * 512, true, 0, &read10(0, 8)));
    for _ in 0..10 {
        msc.poll(&mut pipe, &mut [&mut disk], 1000);
    }
    assert!(pipe.sending.is_some());
    msc.reset();
    msc.poll(&mut pipe, &mut [&mut disk], 1000);
    assert_eq!(pipe.aborted, 1);
    assert!(pipe.sending.is_none());
    pipe.delay = 2;
    let want = disk.data[16 * BLOCK..20 * BLOCK].to_vec();
    let (d, csw) = run_with(
        &mut pipe,
        &mut msc,
        &mut disk,
        1000,
        0,
        4 * 512,
        true,
        &read10(16, 4),
        &[],
    );
    assert_eq!(csw.status, 0);
    assert!(d == want);
}
