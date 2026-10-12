//! USB Mass Storage, Bulk-Only Transport with the SCSI transparent command
//! set (ADR-0014): what lets Windows, macOS and Linux mount a card on the
//! device as a drive, with their built-in drivers.
//!
//! Hardware-independent: it moves 64-byte packets through a [`BulkPipe`]
//! (the USB controller's bulk endpoint pair) and blocks through [`Storage`]
//! (one per logical unit: the SD card now, the CF card later). Driven by
//! [`Msc::poll`] from the owner's main loop; no heap, no threads.
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

/// The USB controller's bulk endpoint pair for this interface.
pub trait BulkPipe {
    /// Takes the next OUT packet (at most [`PACKET`] bytes) if one arrived.
    fn read_packet(&mut self, buf: &mut [u8; PACKET]) -> Option<usize>;
    /// Whether an IN packet can be queued (the previous one has gone).
    fn can_write(&mut self) -> bool;
    /// Queues one IN packet; shorter than [`PACKET`] (even empty) ends the
    /// transfer.
    fn write_packet(&mut self, data: &[u8]);
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
    /// First block and count of the last failed transfer.
    pub last_error: (u32, u32),
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
    // Block transfer state.
    lba: u32,
    blocks_left: u32,
    buf_len: usize,
    buf_pos: usize,
    pkt: [u8; PACKET],
    last_host_write: Option<u32>,
    stats: Stats,
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
            pkt: [0; PACKET],
            last_host_write: None,
            stats: Stats::default(),
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
    }

    /// The device changed unit `lun`'s medium (a file written on the card):
    /// the host is told on its next command and re-reads the volume. Also
    /// offers an ejected medium again.
    pub fn medium_changed(&mut self, lun: usize) {
        if let Some(u) = self.units.get_mut(lun) {
            u.attention = true;
            u.ejected = false;
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
        let present = !unit.ejected && luns[self.lun].present();
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
        self.lba = lba;
        self.blocks_left = count;
        self.buf_len = 0;
        self.buf_pos = 0;
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

    fn read_step(&mut self, pipe: &mut dyn BulkPipe, luns: &mut [&mut dyn Storage]) -> bool {
        if self.buf_pos >= self.buf_len {
            if self.blocks_left == 0 {
                self.phase = Phase::Status;
                return true;
            }
            let n = (self.buf.len() / BLOCK).min(self.blocks_left as usize);
            let bytes = n * BLOCK;
            if luns[self.lun]
                .read_blocks(self.lba, &mut self.buf[..bytes])
                .is_err()
            {
                self.stats.io_errors += 1;
                self.stats.last_error = (self.lba, n as u32);
                // The host still waits for data: end its transfer short.
                self.units[self.lun].sense = Sense::READ_ERROR;
                self.failed = true;
                self.stats.failed += 1;
                self.resp_len = 0;
                self.resp_pos = 0;
                self.phase = Phase::SendResp;
                return true;
            }
            self.lba += n as u32;
            self.blocks_left -= n as u32;
            self.stats.blocks_read += n as u32;
            self.buf_len = bytes;
            self.buf_pos = 0;
        }
        if !pipe.can_write() {
            return false;
        }
        let end = self.buf_pos + PACKET;
        pipe.write_packet(&self.buf[self.buf_pos..end]);
        self.buf_pos = end;
        self.done += PACKET as u32;
        true
    }

    fn write_step(
        &mut self,
        pipe: &mut dyn BulkPipe,
        luns: &mut [&mut dyn Storage],
        now_ms: u32,
    ) -> bool {
        let Some(n) = pipe.read_packet(&mut self.pkt) else {
            return false;
        };
        self.last_host_write = Some(now_ms);
        let room = (self.blocks_left as usize * BLOCK).min(self.buf.len()) - self.buf_len;
        let take = n.min(room);
        self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&self.pkt[..take]);
        self.buf_len += take;
        self.done += n as u32;
        let chunk = (self.blocks_left as usize * BLOCK).min(self.buf.len());
        if self.buf_len == chunk {
            let blocks = (chunk / BLOCK) as u32;
            let result = if self.failed {
                Ok(())
            } else {
                luns[self.lun].write_blocks(self.lba, &self.buf[..chunk])
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
            self.buf_len = 0;
        }
        if self.blocks_left == 0 || self.done >= self.host_len {
            self.phase = Phase::Status;
        }
        true
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
