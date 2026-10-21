//! One sector onto a card the loader left set up: the card log's writes
//! from a program ([`crate::cardsink`]).
//!
//! The drivers (`rp-drv-sdcard` with `rp-drv-pxa-mmc`, `rp-drv-cf`) sit a
//! layer above the HAL (DESIGN.md §3.1), so the run log's writer, which
//! every program carries, can't use them. These two writes do what the
//! drivers do for one sector on a card already identified, through the
//! same registers in the same order: a host test runs both on one
//! recording bus and wants the same register writes (`tests.rs`), so a
//! change to the drivers' protocol shows here.
//!
//! - **SD (PXA27x MMC controller, docs/hw/sd.md):** CMD24 and its status,
//!   the block through the transmit FIFO 32 bytes a request, the data and
//!   programming done, then CMD13 until the card is ready again. Single
//!   blocks by the CPU: no DMA channel is used.
//! - **CF (memory mode, docs/hw/cf.md):** WRITE SECTORS (count 1, LBA),
//!   256 words through the data port, then the card's busy and status.
//!
//! Every wait has a limit, and a refusal or a timeout is `false`, never a
//! panic: a card log write must never stop a program.

use rp_hal_api::{RegisterBus, RegisterBus8, RegisterBus16};

/// Bytes per sector.
pub const SECTOR: usize = 512;

/// PXA27x MMC controller registers (PXA27x Developer's Manual).
mod mmc {
    pub const STRPCL: usize = 0x00;
    pub const STAT: usize = 0x04;
    pub const CMDAT: usize = 0x10;
    pub const BLKLEN: usize = 0x1C;
    pub const NUMBLK: usize = 0x20;
    pub const I_REG: usize = 0x2C;
    pub const CMD: usize = 0x30;
    pub const ARGH: usize = 0x34;
    pub const ARGL: usize = 0x38;
    pub const RES: usize = 0x3C;
    pub const TXFIFO: usize = 0x44;

    pub const STRPCL_START: u32 = 1 << 1;
    pub const STAT_TOUT_RES: u32 = 1 << 1;
    pub const STAT_DATA_DONE: u32 = 1 << 11;
    pub const STAT_PRG_DONE: u32 = 1 << 12;
    pub const STAT_END_CMD_RES: u32 = 1 << 13;
    /// R1 response format.
    pub const CMDAT_R1: u32 = 1;
    pub const CMDAT_DATA_EN: u32 = 1 << 2;
    pub const CMDAT_WRITE: u32 = 1 << 3;
    pub const CMDAT_SD_4DAT: u32 = 1 << 8;
    pub const I_TXFIFO_REQ: u32 = 1 << 6;
    pub const FIFO_BYTES: usize = 32;
}

/// SD card status (SD Physical Layer specification, R1).
const STATUS_ERRORS: u32 = 0xFD39_0008;
const STATUS_READY_FOR_DATA: u32 = 1 << 8;
const STATE_PRG: u32 = 7;

/// Status polls for a command's response.
const SPIN_LIMIT: u32 = 2_000_000;
/// Polls for the data and the card's programming.
const WRITE_SPIN_LIMIT: u32 = 60_000_000;
/// CMD13s before a card still programming counts as stuck.
const READY_TRIES: u32 = 200_000;

/// The SD card as the loader identified it.
#[derive(Clone, Copy, Debug)]
pub struct Sd {
    /// Relative card address.
    pub rca: u16,
    /// Block addressing (SDHC/SDXC).
    pub high_capacity: bool,
    /// The 4-bit bus.
    pub wide: bool,
    /// Capacity in blocks: writes past it are refused (an SDSC card's byte
    /// address would wrap onto another block).
    pub blocks: u32,
}

fn wait<B: RegisterBus>(regs: &mut B, offset: usize, mask: u32, limit: u32) -> Option<u32> {
    (0..limit)
        .map(|_| regs.read32(offset))
        .find(|s| s & mask != 0)
}

/// Sends a command (R1; `cmdat` with the data bits for a block) and
/// returns the card's status, if it answered without an error. The
/// driver's register order.
fn command<B: RegisterBus>(regs: &mut B, index: u8, arg: u32, cmdat: u32) -> Option<u32> {
    regs.write32(mmc::CMD, u32::from(index & 0x3F));
    regs.write32(mmc::ARGH, arg >> 16);
    regs.write32(mmc::ARGL, arg & 0xFFFF);
    if cmdat & mmc::CMDAT_DATA_EN != 0 {
        regs.write32(mmc::BLKLEN, SECTOR as u32);
        regs.write32(mmc::NUMBLK, 1);
    }
    regs.write32(mmc::CMDAT, cmdat);
    regs.write32(mmc::STRPCL, mmc::STRPCL_START);
    let s = wait(
        regs,
        mmc::STAT,
        mmc::STAT_END_CMD_RES | mmc::STAT_TOUT_RES,
        SPIN_LIMIT,
    )?;
    if s & mmc::STAT_TOUT_RES != 0 {
        return None;
    }
    // Nine 16-bit words; an R1's status is bits 39:8 of the response.
    let mut w = [0u32; 9];
    for word in &mut w {
        *word = regs.read32(mmc::RES) & 0xFFFF;
    }
    let status = (w[0] << 24) | (w[1] << 8) | (w[2] >> 8);
    (status & STATUS_ERRORS == 0).then_some(status)
}

/// Writes `sector` to block `lba` of the SD card on the controller
/// `regs`; whether the card took it and is ready again.
pub fn sd_write<B: RegisterBus + RegisterBus8>(
    regs: &mut B,
    card: Sd,
    lba: u32,
    sector: &[u8; SECTOR],
) -> bool {
    if lba >= card.blocks {
        return false;
    }
    let addr = if card.high_capacity {
        lba
    } else {
        lba.wrapping_mul(SECTOR as u32)
    };
    let bus = if card.wide { mmc::CMDAT_SD_4DAT } else { 0 };
    let cmdat = mmc::CMDAT_R1 | mmc::CMDAT_DATA_EN | mmc::CMDAT_WRITE | bus;
    if command(regs, 24, addr, cmdat).is_none() {
        return false;
    }
    if !sd_fifo(regs, sector) {
        return false;
    }
    if wait(regs, mmc::STAT, mmc::STAT_DATA_DONE, WRITE_SPIN_LIMIT).is_none()
        || wait(regs, mmc::STAT, mmc::STAT_PRG_DONE, WRITE_SPIN_LIMIT).is_none()
    {
        return false;
    }
    // Programmed by the controller's account; the card's own (CMD13), as
    // the driver checks after every write.
    let arg = u32::from(card.rca) << 16;
    for _ in 0..READY_TRIES {
        let Some(status) = command(regs, 13, arg, mmc::CMDAT_R1) else {
            return false;
        };
        if status & STATUS_READY_FOR_DATA != 0 && (status >> 9) & 0xF != STATE_PRG {
            return true;
        }
    }
    false
}

/// The block into the transmit FIFO, 32 bytes a request. Out of line and
/// over a slice: with the sector's length known the loop unrolls into
/// kilobytes of stores in every program.
#[inline(never)]
fn sd_fifo<B: RegisterBus + RegisterBus8>(regs: &mut B, data: &[u8]) -> bool {
    for chunk in data.chunks(mmc::FIFO_BYTES) {
        if wait(regs, mmc::I_REG, mmc::I_TXFIFO_REQ, WRITE_SPIN_LIMIT).is_none() {
            return false;
        }
        for &b in chunk {
            regs.write8(mmc::TXFIFO, b);
        }
    }
    true
}

/// The sector through the data port, a word at a time (out of line, as
/// [`sd_fifo`]).
#[inline(never)]
fn cf_port<C: RegisterBus16>(common: &mut C, data: &[u8]) {
    for pair in data.as_chunks::<2>().0 {
        common.write16(ata::DATA, u16::from_le_bytes(*pair));
    }
}

/// CompactFlash task file, memory mode (CF specification).
mod ata {
    pub const DATA: usize = 0x0;
    pub const COUNT: usize = 0x2;
    pub const LBA0: usize = 0x3;
    pub const LBA1: usize = 0x4;
    pub const LBA2: usize = 0x5;
    pub const HEAD: usize = 0x6;
    pub const STATUS_COMMAND: usize = 0x7;

    pub const ST_BSY: u8 = 0x80;
    pub const ST_DF: u8 = 0x20;
    pub const ST_DRQ: u8 = 0x08;
    pub const ST_ERR: u8 = 0x01;
    /// LBA addressing, device 0.
    pub const HEAD_LBA: u8 = 0xE0;
    pub const WRITE_SECTORS: u8 = 0x30;
    /// Polls for the card to take a command or ready a sector.
    pub const SPIN_LIMIT: u32 = 2_000_000;
    /// Polls while the card programs its flash.
    pub const WRITE_SPIN_LIMIT: u32 = 20_000_000;
}

fn idle<C: RegisterBus8>(common: &mut C, limit: u32) -> Option<u8> {
    (0..limit)
        .map(|_| common.read8(ata::STATUS_COMMAND))
        .find(|s| s & ata::ST_BSY == 0)
}

/// Writes `sector` to sector `lba` of the CF card in the socket whose
/// common memory is `common` (`sectors` its capacity); whether the card
/// took it.
pub fn cf_write<C: RegisterBus8 + RegisterBus16>(
    common: &mut C,
    sectors: u32,
    lba: u32,
    sector: &[u8; SECTOR],
) -> bool {
    if lba >= sectors {
        return false;
    }
    if idle(common, ata::SPIN_LIMIT).is_none() {
        return false;
    }
    common.write8(ata::COUNT, 1);
    common.write8(ata::LBA0, lba as u8);
    common.write8(ata::LBA1, (lba >> 8) as u8);
    common.write8(ata::LBA2, (lba >> 16) as u8);
    common.write8(ata::HEAD, ata::HEAD_LBA | ((lba >> 24) as u8 & 0x0F));
    common.write8(ata::STATUS_COMMAND, ata::WRITE_SECTORS);
    let mut ready = false;
    for _ in 0..ata::SPIN_LIMIT {
        let s = common.read8(ata::STATUS_COMMAND);
        if s & ata::ST_BSY != 0 {
            continue;
        }
        if s & (ata::ST_ERR | ata::ST_DF) != 0 {
            return false;
        }
        if s & ata::ST_DRQ != 0 {
            ready = true;
            break;
        }
    }
    if !ready {
        return false;
    }
    cf_port(common, sector);
    idle(common, ata::WRITE_SPIN_LIMIT).is_some_and(|s| s & (ata::ST_ERR | ata::ST_DF) == 0)
}

#[cfg(test)]
mod tests;
