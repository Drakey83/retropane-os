use std::collections::VecDeque;

use rp_hal_api::BlockError;

use super::*;

/// A bulk pipe that records IN packets and plays queued OUT packets.
#[derive(Default)]
struct Pipe {
    out: VecDeque<Vec<u8>>,
    ins: Vec<Vec<u8>>,
    busy: bool,
}

impl BulkPipe for Pipe {
    fn read_packet(&mut self, buf: &mut [u8; PACKET]) -> Option<usize> {
        let p = self.out.pop_front()?;
        buf[..p.len()].copy_from_slice(&p);
        Some(p.len())
    }
    fn can_write(&mut self) -> bool {
        // Alternate busy/free, as a real endpoint with one packet in flight.
        self.busy = !self.busy;
        !self.busy
    }
    fn write_packet(&mut self, data: &[u8]) {
        assert!(data.len() <= PACKET);
        self.ins.push(data.to_vec());
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
        }
    }
}

impl BlockDevice for Disk {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
        let o = lba as usize * BLOCK;
        assert!(o + buf.len() <= self.data.len(), "read past the end");
        self.reads += 1;
        if self.fail_reads {
            return Err(BlockError::Device);
        }
        buf.copy_from_slice(&self.data[o..o + buf.len()]);
        Ok(())
    }
    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
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
    let mut pipe = Pipe::default();
    pipe.out.push_back(cbw(0xC0FFEE, len, dir_in, lun, cb));
    for p in data_out.chunks(PACKET) {
        pipe.out.push_back(p.to_vec());
    }
    for _ in 0..100_000 {
        msc.poll(&mut pipe, &mut [disk], 1000);
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
fn a_device_side_change_is_reported_once_and_eject_hands_the_card_back() {
    let mut buf = vec![0u8; 2 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(16);
    msc.medium_changed(0);
    // INQUIRY doesn't consume the attention; the next other command does.
    let (_, csw) = run(&mut msc, &mut disk, 36, true, &[0x12, 0, 0, 0, 36, 0], &[]);
    assert_eq!(csw.status, 0);
    let (_, csw) = run(&mut msc, &mut disk, 0, false, &[0; 6], &[]);
    assert_eq!(csw.status, 1);
    assert_eq!(sense(&mut msc, &mut disk), (0x06, 0x28, 0x00));
    let (_, csw) = run(&mut msc, &mut disk, 0, false, &[0; 6], &[]);
    assert_eq!(csw.status, 0);
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
    let (_, csw) = run(&mut msc, &mut disk, 0, false, &[0; 6], &[]);
    assert_eq!(csw.status, 1, "unit attention first");
    let (_, csw) = run(&mut msc, &mut disk, 0, false, &[0; 6], &[]);
    assert_eq!(csw.status, 0);
}

#[test]
fn a_card_error_mid_read_ends_the_transfer_with_sense() {
    let mut buf = vec![0u8; 2 * BLOCK];
    let mut msc = Msc::new(&mut buf, SERIAL, 1);
    let mut disk = Disk::new(16);
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
