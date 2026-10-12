//! CompactFlash storage cards (docs/hw/cf.md), in memory mode: the mode
//! every CF storage card must support, with the ATA task-file registers in
//! the socket's common-memory window and the card's Card Information
//! Structure (CIS) and configuration registers in attribute memory.
//!
//! [`CfCard::init`] first resets the card through the socket's RESET line
//! and waits for its READY line ([`CfLines`]): an earlier owner (Windows
//! Mobile, or a RetroPane run before a loader hand-over) may have left it in
//! any state, so nothing about it is trusted. Then it reads the CIS (only fixed-disk cards are taken: a Wi-Fi
//! or modem card in the slot is left alone), selects memory mode, turns the
//! card's interrupts off (the driver polls), identifies it (ATA IDENTIFY
//! DEVICE: LBA28 capacity, model) and turns its write cache off, so the
//! file systems' crash-safe write order holds on the card itself. Then
//! [`BlockDevice`] reads and writes sectors with READ / WRITE SECTORS.
//!
//! Security (docs/security.md): everything the card says is checked before
//! use (CIS walks are bounded, IDENTIFY fields validated, transfers kept
//! inside the reported capacity), every wait has a limit, and a card that
//! misbehaves gives an error, never a panic or a hang.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_hal_api::{BlockDevice, BlockError, RegisterBus8, RegisterBus16};

/// Bytes per sector.
pub const SECTOR: usize = 512;

/// Task-file registers, memory mode (byte offsets in common memory).
mod reg {
    pub const DATA: usize = 0x0;
    pub const ERROR_FEATURES: usize = 0x1;
    pub const COUNT: usize = 0x2;
    pub const LBA0: usize = 0x3;
    pub const LBA1: usize = 0x4;
    pub const LBA2: usize = 0x5;
    pub const HEAD: usize = 0x6;
    pub const STATUS_COMMAND: usize = 0x7;
    pub const DEVICE_CONTROL: usize = 0xE;
}

const ST_BSY: u8 = 0x80;
const ST_DRDY: u8 = 0x40;
const ST_DF: u8 = 0x20;
const ST_DRQ: u8 = 0x08;
const ST_ERR: u8 = 0x01;
/// Device control: interrupts off.
const CTL_NIEN: u8 = 0x02;
/// Head register: LBA addressing, device 0.
const HEAD_LBA: u8 = 0xE0;

const CMD_READ_SECTORS: u8 = 0x20;
const CMD_WRITE_SECTORS: u8 = 0x30;
const CMD_IDENTIFY: u8 = 0xEC;
const CMD_SET_FEATURES: u8 = 0xEF;
const FEATURE_WRITE_CACHE_OFF: u8 = 0x82;

/// CIS tuple codes (CF specification).
const CISTPL_NULL: u8 = 0x00;
const CISTPL_CONFIG: u8 = 0x1A;
const CISTPL_FUNCID: u8 = 0x21;
const CISTPL_END: u8 = 0xFF;
/// `CISTPL_FUNCID` function: fixed disk.
const FUNC_FIXED_DISK: u8 = 0x04;
/// Attribute memory the CIS walk may read (bytes; tuples on even bytes).
const CIS_LIMIT: usize = 0x400;
/// The configuration registers' base must be inside the attribute window.
const CONFIG_LIMIT: u32 = 0x1000;

/// Status polls for a command to start or a sector to be ready.
const SPIN_LIMIT: u32 = 2_000_000;
/// Polls after a write, while the card programs its flash.
const WRITE_SPIN_LIMIT: u32 = 20_000_000;
/// Milliseconds a card may take to become ready after reset (the CF
/// specification allows rotating Microdrives far longer than flash).
const READY_MS: u32 = 3000;

/// RESET held for (the specification asks 10 µs).
const RESET_HOLD_MS: u32 = 1;
/// After RESET is released, before the card's lines mean anything (PC Card
/// reset set-up time).
const RESET_SETUP_MS: u32 = 20;

/// The socket's control lines, as the board wires them.
pub trait CfLines {
    /// Drives the card's RESET line (asserted: the card is held in reset).
    fn set_reset(&mut self, asserted: bool);
    /// The card's READY line (memory mode: RDY/-BSY, high when ready).
    fn ready(&mut self) -> bool;
}

/// A board without the lines (or a test of the soft path): no hardware
/// reset, the card counted ready at once.
pub struct NoLines;

impl CfLines for NoLines {
    fn set_reset(&mut self, _: bool) {}
    fn ready(&mut self) -> bool {
        true
    }
}

/// What the hardware reset saw (diagnostics: proves the board's lines).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResetSeen {
    /// READY read low while RESET was held: the lines are the card's.
    pub busy_in_reset: bool,
    /// Milliseconds after the set-up time until READY went high.
    pub ready_ms: u32,
}

/// Resets the card through its RESET line and waits for READY, touching
/// neither window: a card in reset or busy may hold the bus. Every wait is
/// bounded.
pub fn hard_reset(
    lines: &mut dyn CfLines,
    delay_ms: &mut dyn FnMut(u32),
) -> Result<ResetSeen, CfError> {
    lines.set_reset(true);
    delay_ms(RESET_HOLD_MS);
    let busy_in_reset = !lines.ready();
    lines.set_reset(false);
    delay_ms(RESET_SETUP_MS);
    for ready_ms in 0..READY_MS {
        if lines.ready() {
            return Ok(ResetSeen {
                busy_in_reset,
                ready_ms,
            });
        }
        delay_ms(1);
    }
    Err(CfError::Timeout)
}

/// Why a card couldn't be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CfError {
    /// The CIS doesn't describe a fixed disk (an I/O card, or no CIS).
    NotStorage,
    /// The card never became ready, or a transfer never finished.
    Timeout,
    /// The card refused a command: its error register.
    Device(u8),
    /// IDENTIFY gave no usable LBA capacity.
    BadIdentify,
}

impl CfError {
    /// A short name for logs and replies.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::NotStorage => "not a storage card",
            Self::Timeout => "timeout",
            Self::Device(_) => "card error",
            Self::BadIdentify => "no LBA capacity",
        }
    }
}

/// A CompactFlash storage card in memory mode.
pub struct CfCard<A, C> {
    attr: A,
    common: C,
    sectors: u32,
    model: [u8; 40],
    /// The write cache was turned off (the card accepted SET FEATURES).
    cache_off: bool,
    /// The last error register the card reported.
    last_error: u8,
    /// The card stopped answering (a wait ran out): every later transfer
    /// fails at once, so a dead card costs one timeout, not one per retry.
    dead: bool,
}

/// What the CIS says: the configuration registers' base, if a fixed disk.
fn walk_cis(attr: &mut dyn RegisterBus8) -> Result<u32, CfError> {
    let mut at = 0usize;
    let mut fixed_disk = false;
    let mut config = None;
    // Every tuple takes at least one byte pair: the walk ends by CIS_LIMIT.
    while at < CIS_LIMIT {
        let code = attr.read8(at);
        if code == CISTPL_END {
            break;
        }
        if code == CISTPL_NULL {
            at += 2;
            continue;
        }
        let link = attr.read8(at + 2);
        if link == 0xFF {
            break;
        }
        let data = at + 4;
        match code {
            CISTPL_FUNCID if link >= 1 => {
                fixed_disk = attr.read8(data) == FUNC_FIXED_DISK;
            }
            CISTPL_CONFIG if link >= 3 => {
                let size = attr.read8(data);
                let rasz = usize::from(size & 3) + 1;
                if link as usize >= 2 + rasz {
                    let mut base = 0u32;
                    for k in 0..rasz {
                        base |= u32::from(attr.read8(data + 4 + 2 * k)) << (8 * k);
                    }
                    config = Some(base);
                }
            }
            _ => {}
        }
        at = data + 2 * usize::from(link);
    }
    match config {
        Some(base) if fixed_disk && base < CONFIG_LIMIT && base % 2 == 0 => Ok(base),
        _ => Err(CfError::NotStorage),
    }
}

impl<A: RegisterBus8, C: RegisterBus8 + RegisterBus16> CfCard<A, C> {
    /// Takes the card in the socket: `attr` and `common` are the socket's
    /// attribute and common memory windows (powered, timings set), `lines`
    /// its RESET and READY lines, `delay_ms` waits. The card is reset first
    /// ([`hard_reset`]). The windows come back with the error if it can't be
    /// used.
    pub fn init(
        mut attr: A,
        common: C,
        lines: &mut dyn CfLines,
        delay_ms: &mut dyn FnMut(u32),
    ) -> Result<(Self, ResetSeen), (CfError, A, C)> {
        let seen = match hard_reset(lines, delay_ms) {
            Ok(s) => s,
            Err(e) => return Err((e, attr, common)),
        };
        let config = match walk_cis(&mut attr) {
            Ok(c) => c,
            Err(e) => return Err((e, attr, common)),
        };
        let mut card = Self {
            attr,
            common,
            sectors: 0,
            model: [b' '; 40],
            cache_off: false,
            last_error: 0,
            dead: false,
        };
        match card.start(config as usize, delay_ms) {
            Ok(()) => Ok((card, seen)),
            Err(e) => Err((e, card.attr, card.common)),
        }
    }

    fn start(&mut self, config: usize, delay_ms: &mut dyn FnMut(u32)) -> Result<(), CfError> {
        // Configuration Option Register: soft reset, then index 0 (memory
        // mode).
        self.attr.write8(config, 0x80);
        delay_ms(1);
        self.attr.write8(config, 0x00);
        self.common.write8(reg::DEVICE_CONTROL, CTL_NIEN);
        let mut ready = false;
        for _ in 0..READY_MS {
            let s = self.status();
            if s & ST_BSY == 0 && s & ST_DRDY != 0 {
                ready = true;
                break;
            }
            delay_ms(1);
        }
        if !ready {
            return Err(CfError::Timeout);
        }
        self.command(CMD_IDENTIFY, 0, 0)?;
        self.wait_drq()?;
        let mut id = [0u16; 256];
        for w in &mut id {
            *w = self.common.read16(reg::DATA);
        }
        self.finish(SPIN_LIMIT)?;
        // Word 49 bit 9: LBA supported; words 60-61: LBA28 sectors.
        let sectors = u32::from(id[60]) | u32::from(id[61]) << 16;
        if id[49] & (1 << 9) == 0 || sectors == 0 || sectors > 0x0FFF_FFFF {
            return Err(CfError::BadIdentify);
        }
        self.sectors = sectors;
        // Words 27-46: the model, two characters per word, first in the high byte.
        for (i, w) in id[27..47].iter().enumerate() {
            for (k, b) in w.to_be_bytes().into_iter().enumerate() {
                self.model[2 * i + k] = if b.is_ascii_graphic() || b == b' ' {
                    b
                } else {
                    b'?'
                };
            }
        }
        // Write cache off, so a write is on the card when it completes (the
        // file systems' crash-safe order depends on it). Cards without a
        // cache refuse the command; that's fine.
        self.common
            .write8(reg::ERROR_FEATURES, FEATURE_WRITE_CACHE_OFF);
        self.cache_off = self
            .command(CMD_SET_FEATURES, 0, 0)
            .and_then(|()| self.finish(SPIN_LIMIT))
            .is_ok();
        Ok(())
    }

    /// Sectors on the card.
    #[must_use]
    pub fn sectors(&self) -> u32 {
        self.sectors
    }

    /// The card's model string (trailing spaces trimmed).
    #[must_use]
    pub fn model(&self) -> &[u8] {
        let end = self
            .model
            .iter()
            .rposition(|&b| b != b' ')
            .map_or(0, |i| i + 1);
        &self.model[..end]
    }

    /// Whether the card accepted turning its write cache off.
    #[must_use]
    pub fn write_cache_off(&self) -> bool {
        self.cache_off
    }

    /// The last error register the card reported (0: none).
    #[must_use]
    pub fn last_error(&self) -> u8 {
        self.last_error
    }

    /// The card stopped answering: it is not used again until it is taken
    /// anew ([`CfCard::into_inner`], then [`CfCard::init`], which resets it).
    #[must_use]
    pub fn is_dead(&self) -> bool {
        self.dead
    }

    /// The status and error registers as they are now (diagnostics: what
    /// a card that stopped answering says about itself).
    pub fn registers(&mut self) -> (u8, u8) {
        (self.status(), self.common.read8(reg::ERROR_FEATURES))
    }

    /// Gives the windows back.
    pub fn into_inner(self) -> (A, C) {
        (self.attr, self.common)
    }

    fn status(&mut self) -> u8 {
        self.common.read8(reg::STATUS_COMMAND)
    }

    /// Waits for the card to take a command, then issues it.
    fn command(&mut self, cmd: u8, lba: u32, count: u8) -> Result<(), CfError> {
        self.wait_idle(SPIN_LIMIT)?;
        self.common.write8(reg::COUNT, count);
        self.common.write8(reg::LBA0, lba as u8);
        self.common.write8(reg::LBA1, (lba >> 8) as u8);
        self.common.write8(reg::LBA2, (lba >> 16) as u8);
        self.common
            .write8(reg::HEAD, HEAD_LBA | ((lba >> 24) as u8 & 0x0F));
        self.common.write8(reg::STATUS_COMMAND, cmd);
        Ok(())
    }

    fn wait_idle(&mut self, limit: u32) -> Result<u8, CfError> {
        for _ in 0..limit {
            let s = self.status();
            if s & ST_BSY == 0 {
                return Ok(s);
            }
        }
        Err(CfError::Timeout)
    }

    /// Waits for a sector's data to be ready (or the card to give up).
    fn wait_drq(&mut self) -> Result<(), CfError> {
        for _ in 0..SPIN_LIMIT {
            let s = self.status();
            if s & ST_BSY != 0 {
                continue;
            }
            if s & (ST_ERR | ST_DF) != 0 {
                return Err(self.error());
            }
            if s & ST_DRQ != 0 {
                return Ok(());
            }
        }
        Err(CfError::Timeout)
    }

    /// Waits for the command to end and checks it succeeded.
    fn finish(&mut self, limit: u32) -> Result<(), CfError> {
        let s = self.wait_idle(limit)?;
        if s & (ST_ERR | ST_DF) != 0 {
            return Err(self.error());
        }
        Ok(())
    }

    fn error(&mut self) -> CfError {
        self.last_error = self.common.read8(reg::ERROR_FEATURES);
        CfError::Device(self.last_error)
    }

    /// A transfer failed: a card that ran out a wait is dead from now on (a
    /// refused sector is the card answering, so it stays in use).
    fn failed(&mut self, e: CfError) -> BlockError {
        if e == CfError::Timeout {
            self.dead = true;
        }
        BlockError::Device
    }

    /// Checks a transfer stays on the card.
    fn range(&self, lba: u32, len: usize) -> Result<u32, BlockError> {
        if !len.is_multiple_of(SECTOR) {
            return Err(BlockError::OutOfRange);
        }
        let n = u32::try_from(len / SECTOR).map_err(|_| BlockError::OutOfRange)?;
        match lba.checked_add(n) {
            Some(end) if end <= self.sectors => Ok(n),
            _ => Err(BlockError::OutOfRange),
        }
    }
}

impl<A: RegisterBus8, C: RegisterBus8 + RegisterBus16> BlockDevice for CfCard<A, C> {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
        self.range(lba, buf.len())?;
        if self.dead {
            return Err(BlockError::Device);
        }
        // At most 256 sectors per command (a count of 0 means 256).
        for (i, run) in buf.chunks_mut(256 * SECTOR).enumerate() {
            let at = lba + (i * 256) as u32;
            let count = (run.len() / SECTOR) as u8; // 256 wraps to 0
            self.command(CMD_READ_SECTORS, at, count)
                .map_err(|e| self.failed(e))?;
            for sector in run.as_chunks_mut::<SECTOR>().0 {
                self.wait_drq().map_err(|e| self.failed(e))?;
                for pair in sector.as_chunks_mut::<2>().0 {
                    *pair = self.common.read16(reg::DATA).to_le_bytes();
                }
            }
            self.finish(SPIN_LIMIT).map_err(|e| self.failed(e))?;
        }
        Ok(())
    }

    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
        self.range(lba, buf.len())?;
        if self.dead {
            return Err(BlockError::Device);
        }
        for (i, run) in buf.chunks(256 * SECTOR).enumerate() {
            let at = lba + (i * 256) as u32;
            let count = (run.len() / SECTOR) as u8;
            self.command(CMD_WRITE_SECTORS, at, count)
                .map_err(|e| self.failed(e))?;
            for sector in run.as_chunks::<SECTOR>().0 {
                self.wait_drq().map_err(|e| self.failed(e))?;
                for pair in sector.as_chunks::<2>().0 {
                    self.common.write16(reg::DATA, u16::from_le_bytes(*pair));
                }
            }
            // Done when the card has programmed the last sector.
            self.finish(WRITE_SPIN_LIMIT).map_err(|e| self.failed(e))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
