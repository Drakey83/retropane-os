//! USB Mass Storage, Bulk-Only Transport with the SCSI transparent command
//! set (ADR-0014): what lets Windows, macOS and Linux mount a card on the
//! device as a drive, with their built-in drivers.
//!
//! Hardware-independent: it moves 64-byte packets through a [`BulkPipe`]
//! (the USB controller's bulk endpoint pair) and blocks through [`Storage`]
//! (one per logical unit: the SD card now, the CF card later). Driven by
//! [`Msc::poll`] from the owner's main loop; no heap, no threads.
//!
//! A pipe with DMA (item 74) moves a command's blocks a buffer half at a
//! time with no CPU work per packet: a read sends one half while the card
//! reads the next into the other; a write receives one half while the CPU
//! writes the other to the card, so the host isn't held off during card
//! writes. Without DMA, packets go one by one as before.
//!
//! Security (docs/security.md): everything from the host is checked before
//! use: the CBW's signature, length, LUN and direction, every LBA and block
//! count with overflow-checked arithmetic. Malformed or unsupported
//! commands fail with sense data; nothing panics and no access goes past a
//! unit's end.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_hal_api::{BlockDevice, BlockError};

/// Bulk packet size (full speed).
pub const PACKET: usize = 64;
/// Block size.
pub const BLOCK: usize = 512;
const CBW_SIGNATURE: u32 = 0x4342_5355; // "USBC"
const CSW_SIGNATURE: u32 = 0x5342_5355; // "USBS"
const CBW_LEN: usize = 31;
/// How long after the host's last write a device-side write is refused.
pub const HOST_WRITE_GRACE_MS: u32 = 2000;

/// A changed medium stays away at least this long...
pub const AWAY_MIN_MS: u32 = 2000;
/// ...and until the host has been told "no medium" this many times (its
/// polling noticed: Windows drops its cached view only then; a unit
/// attention alone left it showing the old files, docs/overnight.md 65)...
pub const AWAY_SEEN: u8 = 2;
/// ...or at most this long, for a host that isn't looking.
pub const AWAY_MAX_MS: u32 = 10_000;

/// The USB controller's bulk endpoint pair for this interface.
pub trait BulkPipe {
    /// Takes the next OUT packet (at most [`PACKET`] bytes) if one arrived.
    fn read_packet(&mut self, buf: &mut [u8; PACKET]) -> Option<usize>;
    /// Whether an IN packet can be queued (the previous one has gone).
    fn can_write(&mut self) -> bool;
    /// Queues one IN packet; shorter than [`PACKET`] (even empty) ends the
    /// transfer.
    fn write_packet(&mut self, data: &[u8]);
    /// Lets the controller move what is queued (called while the storage
    /// is busy reading, so packets keep flowing). Nothing by default.
    fn service(&mut self) {}
    /// Starts sending `data` (whole packets) to the host by DMA. `data`
    /// stays untouched until [`dma_done`](Self::dma_done) says it ended.
    /// False if the pipe can't (no DMA, a misaligned buffer): the caller
    /// sends packets itself.
    fn send_dma(&mut self, data: &[u8]) -> bool {
        let _ = data;
        false
    }
    /// Starts receiving `buf.len()` bytes (whole packets) from the host by
    /// DMA, on the same terms.
    fn receive_dma(&mut self, buf: &mut [u8]) -> bool {
        let _ = buf;
        false
    }
    /// The transfer started by `send_dma` or `receive_dma`: `None` while it
    /// runs, then `Some(true)` (moved; the endpoint is the CPU's again) or
    /// `Some(false)` (it failed).
    fn dma_done(&mut self) -> Option<bool> {
        Some(true)
    }
    /// Stops a transfer in progress (a reset abandoned its command).
    fn dma_abort(&mut self) {}
}

/// The storage behind one logical unit.
pub trait Storage: BlockDevice {
    /// Capacity in blocks.
    fn blocks(&self) -> u32;
    /// A medium is in the slot.
    fn present(&self) -> bool {
        true
    }
    /// The medium is write-protected.
    fn read_only(&self) -> bool {
        false
    }
    /// The product name in INQUIRY (16 ASCII bytes, space-padded).
    fn product(&self) -> &[u8; 16] {
        b"Storage card    "
    }
    /// Whether [`read_blocks_while`](Self::read_blocks_while) really runs
    /// `wait` while it reads (DMA): only then does the drive read ahead
    /// and overlap reads with sending, which otherwise would only add
    /// waiting. False by default.
    fn overlaps(&self) -> bool {
        false
    }
    /// Reads blocks, running `wait` while the storage waits for them (a
    /// card read by DMA: the drive sends the previous chunk meanwhile). By
    /// default it just reads.
    fn read_blocks_while(
        &mut self,
        lba: u32,
        buf: &mut [u8],
        wait: &mut dyn FnMut(),
    ) -> Result<(), BlockError> {
        let _ = wait;
        self.read_blocks(lba, buf)
    }
}

/// Most logical units supported.
pub const MAX_LUNS: usize = 2;

/// SCSI sense data: key, additional sense code and qualifier.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sense {
    /// Sense key.
    pub key: u8,
    /// Additional sense code.
    pub asc: u8,
    /// Additional sense code qualifier.
    pub ascq: u8,
}

impl Sense {
    const NOT_PRESENT: Self = Self::new(0x02, 0x3A, 0x00);
    const MEDIUM_CHANGED: Self = Self::new(0x06, 0x28, 0x00);
    const READ_ERROR: Self = Self::new(0x03, 0x11, 0x00);
    const WRITE_ERROR: Self = Self::new(0x03, 0x0C, 0x00);
    const WRITE_PROTECTED: Self = Self::new(0x07, 0x27, 0x00);
    const INVALID_COMMAND: Self = Self::new(0x05, 0x20, 0x00);
    const OUT_OF_RANGE: Self = Self::new(0x05, 0x21, 0x00);
    const INVALID_FIELD: Self = Self::new(0x05, 0x24, 0x00);
    const NO_LUN: Self = Self::new(0x05, 0x25, 0x00);

    const fn new(key: u8, asc: u8, ascq: u8) -> Self {
        Self { key, asc, ascq }
    }
}

/// Per-unit state the host sees.
#[derive(Clone, Copy, Debug, Default)]
struct Unit {
    sense: Sense,
    /// The device changed the medium: the next command reports it.
    attention: bool,
    /// The host ejected the medium: not present until offered again.
    ejected: bool,
    /// The device changed the medium and it is away (not present) until
    /// the host has noticed; when it went (set by the first poll after).
    away: Option<Option<u32>>,
    /// "No medium" answers the host has had while away.
    away_seen: u8,
    /// The host's last medium-dependent command found a medium: it may
    /// have cached the volume. If not (it saw the unit empty, or never
    /// asked), a change needs no away time: the host has nothing to drop.
    shown: bool,
    /// The host's view of this unit is known to be current: it read the
    /// capacity (a fresh mount) or its first look was answered (see
    /// [`Msc::new`]). Until then, a poll, read or write is a view from
    /// before this function started.
    vouched: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Waiting for a command.
    Command,
    /// Sending `resp[pos..len]`.
    SendResp,
    /// Reading blocks to the host.
    ReadBlocks,
    /// Receiving blocks from the host.
    WriteBlocks,
    /// Consuming OUT data the command doesn't use.
    Discard,
    /// The data phase is over: send the status.
    Status,
}

/// Counters, for the screen and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Commands received.
    pub commands: u32,
    /// Commands that failed.
    pub failed: u32,
    /// Malformed CBWs ignored.
    pub bad_cbw: u32,
    /// Blocks read for the host.
    pub blocks_read: u32,
    /// Blocks written by the host.
    pub blocks_written: u32,
    /// Block transfers the storage failed.
    pub io_errors: u32,
    /// Block transfers moved by the pipe's DMA (halves of the buffer).
    pub dma_chunks: u32,
    /// The largest block count one READ/WRITE asked for (how much the
    /// host moves per command).
    pub max_blocks: u32,
    /// First block and count of the last failed transfer.
    pub last_error: (u32, u32),
    /// Reads that found their first chunk already read ahead.
    pub read_ahead_hits: u32,
}

/// The mass-storage function.
pub struct Msc<'b> {
    buf: &'b mut [u8],
    serial: &'static [u8],
    luns: usize,
    units: [Unit; MAX_LUNS],
    phase: Phase,
    // The command being run.
    tag: u32,
    lun: usize,
    host_len: u32,
    host_in: bool,
    done: u32,
    failed: bool,
    // Response buffer for small replies.
    resp: [u8; PACKET],
    resp_len: usize,
    resp_pos: usize,
    // Block transfer state. Reads use the buffer as two halves: one goes
    // to the host (the front: `buf_len` bytes, `buf_pos` sent) while the
    // next chunk is read into the other (`back_len` bytes ready), so the
    // card and the USB link work at the same time.
    lba: u32,
    blocks_left: u32,
    buf_len: usize,
    buf_pos: usize,
    front: usize,
    back_len: usize,
    /// The back half holds blocks read ahead, past the last read: (unit,
    /// first block, blocks). Dropped on any write, medium change or reset.
    ahead: Option<(usize, u32, u32)>,
    /// The chunk read behind the front failed (reported once the front
    /// has gone).
    back_failed: bool,
    pkt: [u8; PACKET],
    last_host_write: Option<u32>,
    stats: Stats,
    /// A DMA transfer is in flight: a read's front half going out.
    send_dma: bool,
    /// A write's half being received by DMA: (half, bytes).
    recv_dma: Option<(usize, usize)>,
    /// Bytes of the write command not yet received from the host.
    recv_left: usize,
    /// The pipe refused or failed DMA: packets one by one from now on.
    no_dma: bool,
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

impl<'b> Msc<'b> {
    /// A function with `luns` logical units (1..=[`MAX_LUNS`]), staging
    /// block transfers in `buf` (a whole number of blocks; bigger means
    /// fewer, longer card commands), reporting `serial` (upper-case hex, as
    /// the transport specification asks) in INQUIRY.
    ///
    /// The host may still hold a view of the cards from before this
    /// function started (the device's pull-up is hard-wired, so Windows
    /// can re-attach without noticing the device went, and keep its cached
    /// FAT and directories), while something else wrote them meanwhile (an
    /// image, a crash report, Windows Mobile). Writing that view back
    /// loses files. So a unit's first poll, read or write that doesn't
    /// follow a READ CAPACITY (which a fresh mount always sends first) is
    /// answered "no medium" and the medium goes away as after
    /// [`medium_changed`](Self::medium_changed): the host drops its view
    /// and re-reads before it writes. A fresh host sees no delay.
    ///
    /// # Panics
    /// If `buf` holds no whole block or `luns` is out of range.
    pub fn new(buf: &'b mut [u8], serial: &'static [u8], luns: usize) -> Self {
        assert!(buf.len() >= BLOCK, "staging buffer smaller than a block");
        assert!((1..=MAX_LUNS).contains(&luns), "unsupported LUN count");
        Self {
            buf,
            serial,
            luns,
            units: [Unit::default(); MAX_LUNS],
            phase: Phase::Command,
            tag: 0,
            lun: 0,
            host_len: 0,
            host_in: false,
            done: 0,
            failed: false,
            resp: [0; PACKET],
            resp_len: 0,
            resp_pos: 0,
            lba: 0,
            blocks_left: 0,
            buf_len: 0,
            buf_pos: 0,
            front: 0,
            back_len: 0,
            ahead: None,
            back_failed: false,
            pkt: [0; PACKET],
            last_host_write: None,
            stats: Stats::default(),
            send_dma: false,
            recv_dma: None,
            recv_left: 0,
            no_dma: false,
        }
    }

    /// The answer to the class request Get Max LUN.
    #[must_use]
    pub fn max_lun(&self) -> u8 {
        (self.luns - 1) as u8
    }

    /// Bulk-Only Mass Storage Reset (class request) or a USB bus reset:
    /// abandon the command in progress and wait for the next.
    pub fn reset(&mut self) {
        self.phase = Phase::Command;
        self.ahead = None;
        // DMA is tried again (a transfer in flight is aborted at the next
        // poll).
        self.no_dma = false;
    }

    /// The device changed unit `lun`'s medium (a file written on the card,
    /// a card in or out, its protection): the medium goes away until the
    /// host has noticed ([`AWAY_SEEN`] "no medium" answers and
    /// [`AWAY_MIN_MS`], or [`AWAY_MAX_MS`] with no host looking), then
    /// comes back with a unit attention, so the host drops everything it
    /// cached and re-reads the volume. A unit attention alone isn't enough
    /// for Windows. A unit the host last saw empty (or never asked about)
    /// skips the away time: it has nothing cached, so the medium is
    /// offered at once with the unit attention (a card inserted, or one
    /// attached after the host enumerated, shows up without seconds of
    /// "no medium"). Also offers an ejected medium again.
    pub fn medium_changed(&mut self, lun: usize) {
        self.ahead = None;
        if let Some(u) = self.units.get_mut(lun) {
            u.ejected = false;
            u.away_seen = 0;
            if u.shown || u.away.is_some() {
                u.attention = false;
                u.away = Some(None);
            } else {
                u.attention = true;
            }
        }
    }

    /// Whether unit `lun`'s changed medium is still away from the host.
    #[must_use]
    pub fn away(&self, lun: usize) -> bool {
        self.units.get(lun).is_some_and(|u| u.away.is_some())
    }

    /// Brings changed media back once the host has noticed them gone.
    fn settle_away(&mut self, now_ms: u32) {
        for u in &mut self.units {
            match u.away {
                Some(None) => u.away = Some(Some(now_ms)),
                Some(Some(since)) => {
                    let gone = now_ms.wrapping_sub(since);
                    if (u.away_seen >= AWAY_SEEN && gone >= AWAY_MIN_MS) || gone >= AWAY_MAX_MS {
                        u.away = None;
                        u.attention = true;
                    }
                }
                None => {}
            }
        }
    }

    /// Whether a command is under way (data or status still to move): the
    /// caller should keep polling rather than sleep.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.phase != Phase::Command
    }

    /// Whether the host wrote to any unit within [`HOST_WRITE_GRACE_MS`] of
    /// `now_ms`: the device must not write the card meanwhile.
    #[must_use]
    pub fn host_writing(&self, now_ms: u32) -> bool {
        self.phase == Phase::WriteBlocks
            || self
                .last_host_write
                .is_some_and(|t| now_ms.wrapping_sub(t) < HOST_WRITE_GRACE_MS)
    }

    /// Whether the host ejected unit `lun` (the device may use it freely).
    #[must_use]
    pub fn ejected(&self, lun: usize) -> bool {
        self.units.get(lun).is_some_and(|u| u.ejected)
    }

    /// Counters.
    #[must_use]
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Moves as much as the pipe takes right now; call from the main loop.
    /// `now_ms` is any millisecond clock (wrapping).
    pub fn poll(&mut self, pipe: &mut dyn BulkPipe, luns: &mut [&mut dyn Storage], now_ms: u32) {
        self.settle_away(now_ms);
        // A reset abandoned the command a transfer was moving.
        if self.phase == Phase::Command && (self.send_dma || self.recv_dma.is_some()) {
            pipe.dma_abort();
            self.send_dma = false;
            self.recv_dma = None;
        }
        // Bounded, so the caller's loop keeps servicing everything else.
        for _ in 0..64 {
            let progressed = match self.phase {
                Phase::Command => self.take_command(pipe, luns),
                Phase::SendResp => self.send_resp(pipe),
                Phase::ReadBlocks => self.read_step(pipe, luns),
                Phase::WriteBlocks => self.write_step(pipe, luns, now_ms),
                Phase::Discard => self.discard_step(pipe),
                Phase::Status => self.send_status(pipe),
            };
            if !progressed {
                break;
            }
        }
    }

    fn take_command(&mut self, pipe: &mut dyn BulkPipe, luns: &mut [&mut dyn Storage]) -> bool {
        let Some(n) = pipe.read_packet(&mut self.pkt) else {
            return false;
        };
        let c = &self.pkt[..n];
        if n != CBW_LEN || u32::from_le_bytes([c[0], c[1], c[2], c[3]]) != CBW_SIGNATURE {
            // Not a valid CBW: ignored (the host's reset recovery follows).
            self.stats.bad_cbw += 1;
            return true;
        }
        self.tag = u32::from_le_bytes([c[4], c[5], c[6], c[7]]);
        self.host_len = u32::from_le_bytes([c[8], c[9], c[10], c[11]]);
        self.host_in = c[12] & 0x80 != 0;
        self.lun = usize::from(c[13] & 0x0F);
        let cb_len = usize::from(c[14] & 0x1F).clamp(1, 16);
        let mut cb = [0u8; 16];
        cb[..cb_len].copy_from_slice(&c[15..15 + cb_len]);
        self.done = 0;
        self.failed = false;
        self.stats.commands += 1;
        self.execute(&cb, luns);
        true
    }

    /// Ends the command with failure `sense` and no (more) data.
    fn fail(&mut self, sense: Sense) {
        if let Some(u) = self.units.get_mut(self.lun) {
            u.sense = sense;
            // A changed medium's absence, seen by the host.
            if sense == Sense::NOT_PRESENT && u.away.is_some() {
                u.away_seen = u.away_seen.saturating_add(1);
            }
        }
        self.failed = true;
        self.stats.failed += 1;
        self.no_data();
    }

    /// The command moves no data: skip (or consume) the host's data phase.
    fn no_data(&mut self) {
        self.phase = if self.host_len > self.done && !self.host_in {
            Phase::Discard
        } else if self.host_len > self.done && self.host_in {
            // The host expects data: a short (empty) packet ends its transfer.
            self.resp_len = 0;
            self.resp_pos = 0;
            Phase::SendResp
        } else {
            Phase::Status
        };
    }

    /// Replies with `self.resp[..len]`, cut to what the host asked for.
    fn reply(&mut self, len: usize) {
        if !self.host_in {
            self.fail(Sense::INVALID_FIELD);
            return;
        }
        self.resp_len = len.min(self.host_len as usize);
        self.resp_pos = 0;
        self.phase = Phase::SendResp;
    }

    fn execute(&mut self, cb: &[u8; 16], luns: &mut [&mut dyn Storage]) {
        let op = cb[0];
        if self.lun >= self.luns.min(luns.len()) {
            self.fail(Sense::NO_LUN);
            return;
        }
        // INQUIRY and REQUEST SENSE work whatever the unit's state.
        match op {
            0x12 => return self.inquiry(cb, &*luns[self.lun]),
            0x03 => return self.request_sense(cb),
            _ => {}
        }
        let unit = self.units[self.lun];
        if unit.attention {
            self.units[self.lun].attention = false;
            self.fail(Sense::MEDIUM_CHANGED);
            return;
        }
        let present = !unit.ejected && unit.away.is_none() && luns[self.lun].present();
        if !unit.vouched && matches!(op, 0x00 | 0x25 | 0x28 | 0x2A | 0x2F | 0x35) {
            let u = &mut self.units[self.lun];
            u.vouched = true;
            if present && op != 0x25 {
                // A view from before this function started: away first.
                u.away = Some(None);
                u.away_seen = 0;
                u.shown = false;
                self.fail(Sense::NOT_PRESENT);
                return;
            }
        }
        self.units[self.lun].shown = present;
        let blocks = luns[self.lun].blocks();
        let read_only = luns[self.lun].read_only();
        match op {
            // TEST UNIT READY, VERIFY(10), SYNCHRONIZE CACHE(10): nothing
            // cached (writes go straight to the card), nothing to verify
            // beyond the card's own ECC.
            0x00 | 0x2F | 0x35 => self.need_medium(present, Self::no_data),
            // PREVENT/ALLOW MEDIUM REMOVAL: accepted (the card has no lock).
            0x1E => self.no_data(),
            // START STOP UNIT: eject (LoEj, !Start) hands the card back to
            // the device; load (LoEj, Start) takes it again.
            0x1B => {
                if cb[4] & 0x02 != 0 {
                    self.units[self.lun].ejected = cb[4] & 0x01 == 0;
                }
                self.no_data();
            }
            0x25 => self.need_medium(present, |m| {
                m.resp[..4].copy_from_slice(&blocks.saturating_sub(1).to_be_bytes());
                m.resp[4..8].copy_from_slice(&(BLOCK as u32).to_be_bytes());
                m.reply(8);
            }),
            0x23 => self.format_capacities(present, blocks),
            0x1A | 0x5A => self.mode_sense(op, cb, present, read_only),
            0x28 | 0x2A => {
                let lba = be32(&cb[2..6]);
                let count = u32::from(be16(&cb[7..9]));
                self.block_command(op == 0x2A, lba, count, present, blocks, read_only);
            }
            _ => self.fail(Sense::INVALID_COMMAND),
        }
    }

    fn need_medium(&mut self, present: bool, then: impl FnOnce(&mut Self)) {
        if present {
            then(self);
        } else {
            self.fail(Sense::NOT_PRESENT);
        }
    }

    fn inquiry(&mut self, cb: &[u8; 16], lun: &dyn Storage) {
        self.resp = [0; PACKET];
        if cb[1] & 0x01 != 0 {
            // Vital product data: the supported pages, and the serial number.
            match cb[2] {
                0x00 => {
                    self.resp[3] = 2;
                    self.resp[4] = 0x00;
                    self.resp[5] = 0x80;
                    self.reply(6);
                }
                0x80 => {
                    let n = self.serial.len().min(PACKET - 4);
                    self.resp[1] = 0x80;
                    self.resp[3] = n as u8;
                    self.resp[4..4 + n].copy_from_slice(&self.serial[..n]);
                    self.reply(4 + n);
                }
                _ => self.fail(Sense::INVALID_FIELD),
            }
            return;
        }
        if cb[2] != 0 {
            self.fail(Sense::INVALID_FIELD);
            return;
        }
        self.resp[1] = 0x80; // removable: Windows' quick-removal policy
        self.resp[2] = 0x04; // SPC-2
        self.resp[3] = 0x02;
        self.resp[4] = 31;
        self.resp[8..16].copy_from_slice(b"RetroPan");
        self.resp[16..32].copy_from_slice(lun.product());
        self.resp[32..36].copy_from_slice(b"26.0");
        self.reply(36);
    }

    fn request_sense(&mut self, cb: &[u8; 16]) {
        let s = core::mem::take(&mut self.units[self.lun].sense);
        let alloc = usize::from(cb[4]);
        self.resp = [0; PACKET];
        self.resp[0] = 0x70; // current, fixed format
        self.resp[2] = s.key;
        self.resp[7] = 10;
        self.resp[12] = s.asc;
        self.resp[13] = s.ascq;
        self.reply(18.min(alloc));
    }

    fn format_capacities(&mut self, present: bool, blocks: u32) {
        self.resp = [0; PACKET];
        self.resp[3] = 8;
        self.resp[4..8].copy_from_slice(&blocks.to_be_bytes());
        // 2: formatted media; 3: no media (maximum capacity unknown).
        self.resp[8] = if present { 0x02 } else { 0x03 };
        self.resp[9..12].copy_from_slice(&[0, 0x02, 0x00]);
        self.reply(12);
    }

    fn mode_sense(&mut self, op: u8, cb: &[u8; 16], present: bool, read_only: bool) {
        if !present {
            self.fail(Sense::NOT_PRESENT);
            return;
        }
        let page = cb[2] & 0x3F;
        let wp = if read_only { 0x80 } else { 0 };
        self.resp = [0; PACKET];
        let head = if op == 0x1A { 4 } else { 8 };
        let mut len = head;
        // The caching page, so hosts know there is no write cache to flush.
        if page == 0x08 || page == 0x3F {
            self.resp[len] = 0x08;
            self.resp[len + 1] = 0x12;
            len += 20;
        } else if page != 0x1C && page != 0x00 {
            // Pages we don't have: an empty list is a valid answer for the
            // informational-exceptions page and page 0; anything else isn't.
            self.fail(Sense::INVALID_FIELD);
            return;
        }
        if op == 0x1A {
            self.resp[0] = (len - 1) as u8;
            self.resp[2] = wp;
            self.reply(len.min(usize::from(cb[4])));
        } else {
            self.resp[1] = (len - 2) as u8;
            self.resp[3] = wp;
            self.reply(len.min(usize::from(be16(&cb[7..9]))));
        }
    }

    fn block_command(
        &mut self,
        write: bool,
        lba: u32,
        count: u32,
        present: bool,
        blocks: u32,
        read_only: bool,
    ) {
        if !present {
            self.fail(Sense::NOT_PRESENT);
            return;
        }
        // Overflow-checked: lba + count must stay inside the unit.
        if lba.checked_add(count).is_none_or(|end| end > blocks) {
            self.fail(Sense::OUT_OF_RANGE);
            return;
        }
        if write && read_only {
            self.fail(Sense::WRITE_PROTECTED);
            return;
        }
        // The host must ask for exactly this transfer in the right
        // direction; anything else is a phase error (status 2), with no
        // card access.
        let bytes = u64::from(count) * BLOCK as u64;
        if u64::from(self.host_len) != bytes || (count > 0 && self.host_in == write) {
            self.failed = true;
            self.stats.failed += 1;
            self.units[self.lun].sense = Sense::INVALID_FIELD;
            self.no_data();
            return;
        }
        self.stats.max_blocks = self.stats.max_blocks.max(count);
        self.lba = lba;
        self.blocks_left = count;
        self.recv_left = count as usize * BLOCK;
        self.buf_len = 0;
        self.buf_pos = 0;
        self.back_len = 0;
        self.back_failed = false;
        // Blocks read ahead count only for a read starting exactly where
        // they do, on the same unit; anything else drops them (a write may
        // be about to change them).
        let ahead = self.ahead.take();
        if !write
            && let Some((unit, first, n)) = ahead
            && unit == self.lun
            && first == lba
        {
            let take = n.min(count);
            self.back_len = take as usize * BLOCK;
            self.lba += take;
            self.blocks_left -= take;
            self.stats.read_ahead_hits += 1;
        }
        self.phase = if count == 0 {
            Phase::Status
        } else if write {
            Phase::WriteBlocks
        } else {
            Phase::ReadBlocks
        };
    }

    fn send_resp(&mut self, pipe: &mut dyn BulkPipe) -> bool {
        if !pipe.can_write() {
            return false;
        }
        let end = (self.resp_pos + PACKET).min(self.resp_len);
        pipe.write_packet(&self.resp[self.resp_pos..end]);
        let n = end - self.resp_pos;
        self.resp_pos = end;
        self.done += n as u32;
        // A full packet that ends the reply short of what the host asked
        // for needs a zero-length packet after it; a short one ends it.
        if self.resp_pos >= self.resp_len && (n < PACKET || self.done >= self.host_len) {
            self.phase = Phase::Status;
        }
        true
    }

    /// Bytes per half of the buffer (whole blocks; the whole buffer if it
    /// holds fewer than two blocks, which leaves no room to overlap).
    fn half(&self) -> usize {
        let h = self.buf.len() / 2 / BLOCK * BLOCK;
        if h == 0 {
            self.buf.len() / BLOCK * BLOCK
        } else {
            h
        }
    }

    /// Reads `n` blocks from `lba` into half `back`, sending the front half
    /// meanwhile (if `pump`).
    fn read_into(
        &mut self,
        back: usize,
        lba: u32,
        n: usize,
        pump: bool,
        pipe: &mut dyn BulkPipe,
        lun: &mut dyn Storage,
    ) -> Result<(), BlockError> {
        if !pump && back == self.front && self.buf.len() / 2 < n * BLOCK {
            // No overlap: the whole buffer is one chunk, as before.
            return lun.read_blocks(lba, &mut self.buf[..n * BLOCK]);
        }
        let half = self.half();
        let Msc {
            buf,
            buf_pos,
            buf_len,
            done,
            front,
            ..
        } = self;
        let (lo, hi) = buf.split_at_mut(half);
        let (into, from): (&mut [u8], &[u8]) = if back == 0 { (lo, hi) } else { (hi, lo) };
        let from = &from[..(*buf_len).min(from.len())];
        let into = &mut into[..n * BLOCK];
        if !pump || back == *front {
            return lun.read_blocks(lba, into);
        }
        lun.read_blocks_while(lba, into, &mut || {
            while *buf_pos < from.len() && pipe.can_write() {
                pipe.write_packet(&from[*buf_pos..*buf_pos + PACKET]);
                *buf_pos += PACKET;
                *done += PACKET as u32;
            }
            pipe.service();
        })
    }

    /// Reads the command's next chunk (at most `max` bytes) into half
    /// `into`, sending the front meanwhile if `pump`: its block count, or
    /// `None` if it didn't read (recorded as the last error).
    fn next_chunk(
        &mut self,
        into: usize,
        max: usize,
        pump: bool,
        pipe: &mut dyn BulkPipe,
        lun: &mut dyn Storage,
    ) -> Option<usize> {
        let n = (max / BLOCK).min(self.blocks_left as usize);
        if self.read_into(into, self.lba, n, pump, pipe, lun).is_err() {
            self.stats.last_error = (self.lba, n as u32);
            return None;
        }
        self.lba += n as u32;
        self.blocks_left -= n as u32;
        self.stats.blocks_read += n as u32;
        Some(n)
    }

    fn read_step(&mut self, pipe: &mut dyn BulkPipe, luns: &mut [&mut dyn Storage]) -> bool {
        let half = self.half();
        let lun = &mut *luns[self.lun];
        let overlap = half < self.buf.len() && lun.overlaps();
        if self.buf_pos >= self.buf_len {
            if self.back_failed {
                // The chunk read behind failed: reported now, when the host
                // has had everything before it.
                self.back_failed = false;
                return self.read_failed();
            }
            if self.back_len == 0 && self.blocks_left > 0 {
                // Nothing read behind (a command's start): read the front
                // (half the buffer if the other half follows, else all of
                // it), as if it had been read behind.
                let max = if overlap { half } else { self.buf.len() };
                // A unit that doesn't overlap uses the buffer from its start
                // (what the send below reads); the front a unit that does left
                // on the second half would send what was never read.
                if !overlap {
                    self.front = 0;
                }
                let Some(n) = self.next_chunk(self.front, max, false, pipe, lun) else {
                    return self.read_failed();
                };
                self.back_len = n * BLOCK;
                self.front ^= usize::from(overlap);
            }
            if self.back_len == 0 {
                self.phase = Phase::Status;
                return true;
            }
            // The next chunk is ready: it becomes the front.
            self.front ^= usize::from(overlap);
            self.buf_len = core::mem::take(&mut self.back_len);
            self.buf_pos = 0;
        }
        // The front goes out by DMA if the pipe can (no CPU per packet, and
        // it keeps going while the card reads the next chunk below).
        if !self.send_dma && !self.no_dma && self.buf_pos == 0 && self.buf_len > 0 {
            let start = self.front * half * usize::from(overlap);
            if pipe.send_dma(&self.buf[start..start + self.buf_len]) {
                self.send_dma = true;
            } else {
                self.no_dma = true;
            }
        }
        // While the front goes out, read the next chunk into the back: the
        // rest of this command, or else (a bulk read just ended) the blocks
        // after it, kept in case the host reads on.
        if overlap && self.back_len == 0 && self.buf_pos == 0 {
            let back = self.front ^ 1;
            if self.blocks_left > 0 {
                match self.next_chunk(back, half, true, pipe, lun) {
                    Some(n) => self.back_len = n * BLOCK,
                    None => {
                        self.blocks_left = 0;
                        self.back_failed = true;
                    }
                }
            } else if self.buf_len == half && self.ahead.is_none() {
                let n = (half / BLOCK) as u32;
                if self
                    .lba
                    .checked_add(n)
                    .is_some_and(|end| end <= lun.blocks())
                    && self
                        .read_into(back, self.lba, n as usize, true, pipe, lun)
                        .is_ok()
                {
                    self.ahead = Some((self.lun, self.lba, n));
                }
            }
        }
        if self.send_dma {
            match pipe.dma_done() {
                None => return false,
                Some(ok) => {
                    self.send_dma = false;
                    if !ok {
                        // What reached the host is unknown: end the transfer
                        // with an error (the host retries) and stop using DMA.
                        self.no_dma = true;
                        self.blocks_left = 0;
                        return self.read_failed();
                    }
                    self.done += (self.buf_len - self.buf_pos) as u32;
                    self.buf_pos = self.buf_len;
                    self.stats.dma_chunks += 1;
                }
            }
        }
        if self.buf_pos >= self.buf_len {
            return true;
        }
        if !pipe.can_write() {
            return false;
        }
        let start = self.front * half * usize::from(overlap) + self.buf_pos;
        pipe.write_packet(&self.buf[start..start + PACKET]);
        self.buf_pos += PACKET;
        self.done += PACKET as u32;
        true
    }

    /// A chunk didn't read: the host still waits for data, so its transfer
    /// is ended short and the command fails with a read error.
    fn read_failed(&mut self) -> bool {
        self.stats.io_errors += 1;
        self.units[self.lun].sense = Sense::READ_ERROR;
        self.failed = true;
        self.stats.failed += 1;
        self.resp_len = 0;
        self.resp_pos = 0;
        self.phase = Phase::SendResp;
        true
    }

    fn write_step(
        &mut self,
        pipe: &mut dyn BulkPipe,
        luns: &mut [&mut dyn Storage],
        now_ms: u32,
    ) -> bool {
        // By DMA a half at a time, from a chunk boundary (nothing received
        // by packets yet), unless the pipe has refused it.
        if self.recv_dma.is_some() || (!self.no_dma && self.buf_len == 0 && self.recv_left > 0) {
            return self.write_step_dma(pipe, luns, now_ms);
        }
        let Some(n) = pipe.read_packet(&mut self.pkt) else {
            return false;
        };
        self.last_host_write = Some(now_ms);
        let room = (self.blocks_left as usize * BLOCK).min(self.buf.len()) - self.buf_len;
        let take = n.min(room);
        self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&self.pkt[..take]);
        self.buf_len += take;
        self.done += n as u32;
        self.recv_left = self.recv_left.saturating_sub(take);
        let chunk = (self.blocks_left as usize * BLOCK).min(self.buf.len());
        if self.buf_len == chunk {
            self.store(0, chunk, luns);
            self.buf_len = 0;
        }
        if self.blocks_left == 0 || self.done >= self.host_len {
            self.phase = Phase::Status;
        }
        true
    }

    /// Bytes per DMA receive: a half (the other is written meanwhile), or
    /// the whole buffer if it has no halves.
    fn recv_chunk(&self) -> (usize, bool) {
        let h = self.buf.len() / 2 / BLOCK * BLOCK;
        if h == 0 {
            (self.buf.len() / BLOCK * BLOCK, false)
        } else {
            (h, true)
        }
    }

    /// Starts receiving the next chunk into half `into`; false if the pipe
    /// refused (packets from now on).
    fn arm_receive(&mut self, pipe: &mut dyn BulkPipe, into: usize) -> bool {
        let (size, _) = self.recv_chunk();
        let len = self.recv_left.min(size);
        let at = into * size;
        if pipe.receive_dma(&mut self.buf[at..at + len]) {
            self.recv_dma = Some((into, len));
            true
        } else {
            self.no_dma = true;
            false
        }
    }

    fn write_step_dma(
        &mut self,
        pipe: &mut dyn BulkPipe,
        luns: &mut [&mut dyn Storage],
        now_ms: u32,
    ) -> bool {
        let Some((half, len)) = self.recv_dma else {
            return self.arm_receive(pipe, 0);
        };
        let ok = match pipe.dma_done() {
            None => return false,
            Some(ok) => ok,
        };
        self.recv_dma = None;
        self.last_host_write = Some(now_ms);
        if !ok {
            // How much arrived is unknown: fail the command and drain the
            // rest by packets.
            self.no_dma = true;
            self.units[self.lun].sense = Sense::WRITE_ERROR;
            self.failed = true;
            self.stats.failed += 1;
            self.phase = Phase::Discard;
            return true;
        }
        self.done += len as u32;
        self.recv_left -= len;
        self.stats.dma_chunks += 1;
        // The next chunk comes into the other half while this one is
        // written to the card.
        let (_, halves) = self.recv_chunk();
        let mut next_armed = false;
        if self.recv_left > 0 && halves {
            next_armed = self.arm_receive(pipe, half ^ 1);
        }
        let (size, _) = self.recv_chunk();
        self.store(half * size, len, luns);
        if self.blocks_left == 0 {
            self.phase = Phase::Status;
        } else if !halves && self.recv_left > 0 {
            self.arm_receive(pipe, 0);
        } else if !next_armed && self.recv_left > 0 {
            // Refused: the rest comes by packets into the buffer's start.
            self.buf_len = 0;
        }
        true
    }

    /// Writes `len` bytes of the buffer from `at` to the card at the
    /// command's next block (skipped once the command has failed: the
    /// host's data is still taken, the failure reported at the end).
    fn store(&mut self, at: usize, len: usize, luns: &mut [&mut dyn Storage]) {
        {
            let chunk = len;
            let blocks = (chunk / BLOCK) as u32;
            let result = if self.failed {
                Ok(())
            } else {
                luns[self.lun].write_blocks(self.lba, &self.buf[at..at + chunk])
            };
            if let Err(e) = result {
                self.stats.io_errors += 1;
                self.stats.last_error = (self.lba, blocks);
                // Keep taking the host's data; report the failure at the end:
                // a write the storage refuses (a guarded block) as
                // write-protected, so the PC says so instead of "I/O error".
                self.units[self.lun].sense = if e == BlockError::ReadOnly {
                    Sense::WRITE_PROTECTED
                } else {
                    Sense::WRITE_ERROR
                };
                self.failed = true;
                self.stats.failed += 1;
            }
            if !self.failed {
                self.stats.blocks_written += blocks;
            }
            self.lba += blocks;
            self.blocks_left -= blocks;
        }
    }

    fn discard_step(&mut self, pipe: &mut dyn BulkPipe) -> bool {
        let Some(n) = pipe.read_packet(&mut self.pkt) else {
            return false;
        };
        self.done += n as u32;
        if self.done >= self.host_len || n < PACKET {
            self.phase = Phase::Status;
        }
        true
    }

    fn send_status(&mut self, pipe: &mut dyn BulkPipe) -> bool {
        if !pipe.can_write() {
            return false;
        }
        let mut csw = [0u8; 13];
        csw[..4].copy_from_slice(&CSW_SIGNATURE.to_le_bytes());
        csw[4..8].copy_from_slice(&self.tag.to_le_bytes());
        csw[8..12].copy_from_slice(&self.host_len.saturating_sub(self.done).to_le_bytes());
        csw[12] = u8::from(self.failed);
        pipe.write_packet(&csw);
        self.phase = Phase::Command;
        true
    }
}

#[cfg(test)]
mod tests;
