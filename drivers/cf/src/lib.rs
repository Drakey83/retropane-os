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
//! Reads take the fastest path that proves itself on this card at attach
//! ([`ReadPath`]): in memory mode the card answers its data register at
//! every address of 0x400-0x7FF, so 32-bit loads stream across that window
//! (+18% on the owner's card), and a memory-to-memory DMA copy from it
//! (+25%, [`CfCard::with_dma`]) moves a sector with the CPU waiting only
//! for the end (docs/research/storage-speed.md). IDENTIFY is read through
//! the 16-bit port and again through each faster path; a path is used only
//! if its bytes match, so a card or socket that doesn't alias the window
//! keeps the 16-bit port. The socket's timing is the caller's to tune:
//! [`CfCard::tune`] moves it to the card's own rated speed (its CIS) once a
//! read at the new timing matches the same read at the old one (the
//! owner's ruling A, reminder R7; [`timing`]).
//!
//! Security (docs/security.md): everything the card says is checked before
//! use (CIS walks are bounded, IDENTIFY fields validated, transfers kept
//! inside the reported capacity), every wait has a limit, and a card that
//! misbehaves gives an error, never a panic or a hang.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_hal_api::{
    BlockDevice, BlockError, NoDma, RegisterBus, RegisterBus8, RegisterBus16, WindowDma,
};

/// Bytes per sector.
pub const SECTOR: usize = 512;

/// Task-file registers, memory mode (byte offsets in common memory).
pub mod timing;

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
    /// Memory mode: the data register again at every address of
    /// 0x400-0x7FF (CF specification, memory-mapped addressing).
    pub const DATA_WINDOW: usize = 0x400;
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
/// The card's speed at 5 V, and under other conditions (3.3 V, the X51v's
/// socket: preferred).
const CISTPL_DEVICE: u8 = 0x01;
const CISTPL_DEVICE_OC: u8 = 0x1C;
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

/// Spins between the DMA wait's polls of the controller (the gate's C11).
/// On SD, polling the DMA back to back slowed the transfer it waited for:
/// a 200-spin pause made SD DMA reads 28% faster (`rp_drv_pxa_mmc`'s
/// `POLL_SPIN`, X51v 2026-10-22). Whether CF's window reads do the same is
/// not measured: 0 (back to back, as before) until `rp-storfast`'s C2 is
/// run with `RP_CF_POLL_SPIN` at build time (0 against 200), then the
/// measured value. The limit counts passes, so a pause stretches the
/// backstop in time only.
const POLL_SPIN: u32 = match option_env!("RP_CF_POLL_SPIN") {
    Some(s) => rp_hal_api::parse_u32(s),
    None => 0,
};
/// Polls after a write, while the card programs its flash.
const WRITE_SPIN_LIMIT: u32 = 20_000_000;
/// Milliseconds a card may take to become ready after reset (the CF
/// specification allows rotating Microdrives far longer than flash).
const READY_MS: u32 = 3000;

/// RESET held for, in µs: ten times the 10 µs minimum pulse (tw(RESET), CF
/// specification; card makers' data sheets agree). Windows Mobile's driver
/// on the X51v held it 100 ms, a thousand times longer, for no reason any
/// source gives, and every attach waited for it.
const RESET_HOLD_US: u32 = 100;
/// After RESET is released, before the card's lines mean anything (PC Card
/// reset set-up time; data sheets give 20 to 100 ms, and the X51v's READY
/// line doesn't fall during reset, so nothing shorter can be observed:
/// left as it is, an open question in docs/hw/cf.md).
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
/// bounded. `delay_us` waits microseconds.
pub fn hard_reset(
    lines: &mut dyn CfLines,
    delay_us: &mut dyn FnMut(u32),
) -> Result<ResetSeen, CfError> {
    lines.set_reset(true);
    delay_us(RESET_HOLD_US);
    let busy_in_reset = !lines.ready();
    lines.set_reset(false);
    delay_us(RESET_SETUP_MS * 1000);
    for ready_ms in 0..READY_MS {
        if lines.ready() {
            return Ok(ResetSeen {
                busy_in_reset,
                ready_ms,
            });
        }
        delay_us(1000);
    }
    Err(CfError::Timeout)
}

/// How sectors come off the card (slowest first).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReadPath {
    /// 16-bit reads of the data register: every card.
    Port16,
    /// 32-bit loads across the data window.
    Window32,
    /// A DMA copy from the data window.
    Dma,
}

impl ReadPath {
    /// A short name for logs.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Port16 => "16-bit port",
            Self::Window32 => "32-bit window",
            Self::Dma => "DMA from the window",
        }
    }
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
pub struct CfCard<A, C, D = NoDma> {
    attr: A,
    common: C,
    dma: D,
    /// The data window's physical address, for the DMA engine.
    window_phys: u32,
    /// How sectors are read (chosen at attach, see [`ReadPath`]).
    path: ReadPath,
    /// The fastest path the attach check proved on this card.
    proven: ReadPath,
    /// Faster paths that read IDENTIFY differently from the 16-bit port.
    path_mismatches: u8,
    /// Sectors the DMA engine refused (a buffer not in whole cache
    /// lines), read by 32-bit loads instead.
    dma_refused: u32,
    sectors: u32,
    model: [u8; 40],
    /// The write cache was turned off (the card accepted SET FEATURES).
    cache_off: bool,
    /// The last error register the card reported.
    last_error: u8,
    /// The card stopped answering (a wait ran out): every later transfer
    /// fails at once, so a dead card costs one timeout, not one per retry.
    dead: bool,
    /// The common memory's speed its CIS rates, ns (`None`: no rating).
    rated_ns: Option<u32>,
}

/// What the CIS walk found: the configuration registers' base, and the
/// card's rated speed ([`timing::cis_speed_ns`]: the 3.3 V tuple
/// `CISTPL_DEVICE_OC` first, else `CISTPL_DEVICE`).
struct Cis {
    config: u32,
    rated_ns: Option<u32>,
}

/// The socket's common-memory timing register (`MCMEM` for the CF socket),
/// for [`CfCard::tune`]: the driver reads and writes it, and learns the
/// memory clock's cycle.
pub trait SocketTiming {
    /// The register as it is.
    fn read(&mut self) -> u32;
    /// Sets the register.
    fn write(&mut self, value: u32);
    /// The memory clock's cycle, picoseconds.
    fn clk_ps(&mut self) -> u32;
}

/// `MCMEM`'s timing fields (SET, ASST, HOLD: bits 19-0); the rest is kept.
const MCMEM_FIELDS: u32 = (1 << 20) - 1;

/// What [`CfCard::tune`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tuned {
    /// The card's own timing, proven by the read-back check.
    Applied {
        /// The register before.
        was: u32,
        /// The register now.
        now: u32,
        /// The card's rated speed, ns.
        rated_ns: u32,
        /// The card is slower than the socket can wait for: its slowest.
        clamped: bool,
    },
    /// The card's timing read differently from the old one: the old one is
    /// back. `card_ok`: the card reads as before at it (else it's marked
    /// dead, to be taken anew).
    Refused {
        /// The register kept.
        was: u32,
        /// The timing tried.
        tried: u32,
        /// The card's rated speed, ns.
        rated_ns: u32,
        /// The card reads as it did at the old timing.
        card_ok: bool,
    },
    /// Nothing changed, and why.
    Kept(&'static str),
}

/// What the CIS says: the configuration registers' base, if a fixed disk,
/// and the card's rated speed.
fn walk_cis(attr: &mut dyn RegisterBus8) -> Result<Cis, CfError> {
    let mut at = 0usize;
    let mut fixed_disk = false;
    let mut config = None;
    let (mut speed_5v, mut speed_oc) = (None, None);
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
        // A tuple's first bytes (at most four), for the speed tuples.
        let mut body = [0u8; 4];
        let n = usize::from(link).min(body.len());
        if matches!(code, CISTPL_DEVICE | CISTPL_DEVICE_OC) {
            for (k, b) in body[..n].iter_mut().enumerate() {
                *b = attr.read8(data + 2 * k);
            }
        }
        match code {
            CISTPL_DEVICE => speed_5v = timing::cis_speed_ns(&body[..n]),
            // Its condition bytes first (bit 7: another follows).
            CISTPL_DEVICE_OC => {
                let skip = body[..n]
                    .iter()
                    .position(|b| b & 0x80 == 0)
                    .map_or(n, |k| k + 1);
                speed_oc = timing::cis_speed_ns(&body[skip..n]);
            }
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
        Some(base) if fixed_disk && base < CONFIG_LIMIT && base % 2 == 0 => Ok(Cis {
            config: base,
            rated_ns: speed_oc.or(speed_5v),
        }),
        _ => Err(CfError::NotStorage),
    }
}

impl<A: RegisterBus8, C: RegisterBus8 + RegisterBus16 + RegisterBus> CfCard<A, C> {
    /// Takes over a card someone else attached (the loader, for a
    /// program's card log: `rp_hal_x51v::cardlog`): no reset, no CIS walk,
    /// no IDENTIFY, so the card stays as it was; `sectors` is its capacity
    /// (transfers past it are refused). Sectors move through the 16-bit
    /// data port.
    pub fn adopt(attr: A, common: C, sectors: u32) -> Self {
        Self {
            attr,
            common,
            dma: NoDma,
            window_phys: 0,
            path: ReadPath::Port16,
            proven: ReadPath::Port16,
            path_mismatches: 0,
            dma_refused: 0,
            sectors,
            model: [b' '; 40],
            cache_off: false,
            last_error: 0,
            dead: false,
            rated_ns: None,
        }
    }

    /// Takes the card in the socket: `attr` and `common` are the socket's
    /// attribute and common memory windows (powered, timings set), `lines`
    /// its RESET and READY lines, `delay_us` waits (microseconds). The card
    /// is reset first ([`hard_reset`]). The windows come back with the
    /// error if it can't be used.
    pub fn init(
        attr: A,
        common: C,
        lines: &mut dyn CfLines,
        delay_us: &mut dyn FnMut(u32),
    ) -> Result<(Self, ResetSeen), (CfError, A, C)> {
        Self::init_with(attr, common, NoDma, None, lines, delay_us)
            .map_err(|(e, a, c, _)| (e, a, c))
    }

    /// Adds a DMA engine for reads, `common_phys` being the physical
    /// address of the socket's common memory (the engine sees physical
    /// addresses). The engine reads IDENTIFY against the 16-bit port
    /// first and is used only if it reads the same; a card that fails the
    /// check keeps the path it has.
    pub fn with_dma<E: WindowDma>(self, dma: E, common_phys: u32) -> CfCard<A, C, E> {
        let mut card = CfCard {
            attr: self.attr,
            common: self.common,
            dma,
            window_phys: common_phys + reg::DATA_WINDOW as u32,
            path: self.path,
            proven: self.proven,
            path_mismatches: self.path_mismatches,
            dma_refused: 0,
            sectors: self.sectors,
            model: self.model,
            cache_off: self.cache_off,
            last_error: self.last_error,
            dead: self.dead,
            rated_ns: self.rated_ns,
        };
        card.try_dma();
        card
    }
}

impl<A: RegisterBus8, C: RegisterBus8 + RegisterBus16 + RegisterBus, D: WindowDma> CfCard<A, C, D> {
    /// [`CfCard::init`] then [`CfCard::with_dma`] in one, for a caller that
    /// has its engine at hand: the same steps, but the card is built with
    /// its engine's type from the start, so an image carries one copy of
    /// the driver, not one per engine type (the loader carried two,
    /// 2026-10-15). `common_phys` `None`: the engine isn't tried. The
    /// windows and the engine come back with the error.
    pub fn init_with(
        mut attr: A,
        common: C,
        dma: D,
        common_phys: Option<u32>,
        lines: &mut dyn CfLines,
        delay_us: &mut dyn FnMut(u32),
    ) -> Result<(Self, ResetSeen), (CfError, A, C, D)> {
        let seen = match hard_reset(lines, delay_us) {
            Ok(s) => s,
            Err(e) => return Err((e, attr, common, dma)),
        };
        let cis = match walk_cis(&mut attr) {
            Ok(c) => c,
            Err(e) => return Err((e, attr, common, dma)),
        };
        let mut card = Self {
            attr,
            common,
            dma,
            window_phys: 0,
            path: ReadPath::Port16,
            proven: ReadPath::Port16,
            path_mismatches: 0,
            dma_refused: 0,
            sectors: 0,
            model: [b' '; 40],
            cache_off: false,
            last_error: 0,
            dead: false,
            rated_ns: cis.rated_ns,
        };
        if let Err(e) = card.start(cis.config as usize, delay_us) {
            return Err((e, card.attr, card.common, card.dma));
        }
        if let Some(phys) = common_phys {
            card.window_phys = phys + reg::DATA_WINDOW as u32;
            card.try_dma();
        }
        Ok((card, seen))
    }

    /// Moves reads to the engine if it reads IDENTIFY as the 16-bit port
    /// does; a card that fails the check keeps the path it has.
    fn try_dma(&mut self) {
        if !self.dead && self.path == ReadPath::Window32 {
            match self.identify_matches(ReadPath::Dma) {
                Ok(true) => {
                    self.path = ReadPath::Dma;
                    self.proven = ReadPath::Dma;
                }
                Ok(false) | Err(CfError::Device(_)) => self.path_mismatches += 1,
                Err(e) => {
                    let _ = self.failed(e);
                }
            }
        }
    }
    fn start(&mut self, config: usize, delay_us: &mut dyn FnMut(u32)) -> Result<(), CfError> {
        // Configuration Option Register: soft reset, then index 0 (memory
        // mode).
        self.attr.write8(config, 0x80);
        delay_us(1000);
        self.attr.write8(config, 0x00);
        self.common.write8(reg::DEVICE_CONTROL, CTL_NIEN);
        let mut ready = false;
        for _ in 0..READY_MS {
            let s = self.status();
            if s & ST_BSY == 0 && s & ST_DRDY != 0 {
                ready = true;
                break;
            }
            delay_us(1000);
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
        // The faster path, if it reads IDENTIFY as the port did.
        // A card that refuses the second IDENTIFY keeps the port; one that
        // stops answering is no use either way.
        match self.identify_matches(ReadPath::Window32) {
            Ok(true) => {
                self.path = ReadPath::Window32;
                self.proven = ReadPath::Window32;
            }
            Ok(false) | Err(CfError::Device(_)) => self.path_mismatches += 1,
            Err(e) => return Err(e),
        }
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

    /// The common memory's speed the card's CIS rates, ns (`None`: no
    /// rating, or a card taken by [`CfCard::adopt`]).
    #[must_use]
    pub fn rated_ns(&self) -> Option<u32> {
        self.rated_ns
    }

    /// Moves the socket's common-memory timing to the card's own rated speed
    /// ([`timing::for_card`]: the CF specification's minimums with a 1.5x
    /// margin; faster than Windows Mobile's one table for a fast card,
    /// slower for a slow one), kept only if the card reads the same at it:
    /// IDENTIFY and its first, middle and last sectors, read at the old
    /// timing, then twice at the new one, all alike. Otherwise the old
    /// timing is put back and the card read again at it. The attribute
    /// memory's timing (read a few hundred bytes per attach) is left alone.
    pub fn tune(&mut self, socket: &mut dyn SocketTiming) -> Tuned {
        let Some(rated_ns) = self.rated_ns else {
            return Tuned::Kept("the card's CIS gives no speed");
        };
        if self.dead {
            return Tuned::Kept("the card isn't answering");
        }
        let was = socket.read();
        let (t, clamped) = timing::for_card(rated_ns, socket.clk_ps());
        let now = (was & !MCMEM_FIELDS) | t.mcmem();
        if now == was {
            return Tuned::Kept("already at the card's timing");
        }
        let Ok(reference) = self.fingerprint() else {
            return Tuned::Kept("the card couldn't be read at the timing it had");
        };
        socket.write(now);
        let first = self.fingerprint();
        let second = self.fingerprint();
        if first == Ok(reference) && second == Ok(reference) {
            return Tuned::Applied {
                was,
                now,
                rated_ns,
                clamped,
            };
        }
        socket.write(was);
        // A wait that ran out at the new timing marked the card dead; at
        // the old one it gets its chance again.
        self.dead = false;
        let card_ok = self.fingerprint() == Ok(reference);
        self.dead = !card_ok;
        Tuned::Refused {
            was,
            tried: now,
            rated_ns,
            card_ok,
        }
    }

    /// A 64-bit FNV-1a hash of IDENTIFY (through the 16-bit port) and the
    /// first, middle and last sectors (through the read path), for
    /// [`CfCard::tune`]'s comparisons without holding the sectors.
    fn fingerprint(&mut self) -> Result<u64, BlockError> {
        let mut hash: u64 = 0xCBF2_9CE4_8422_2325;
        let mut mix = |bytes: &[u8]| {
            for &b in bytes {
                hash = (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01B3);
            }
        };
        let mut sector = Sector([0; SECTOR]);
        self.command(CMD_IDENTIFY, 0, 0)
            .map_err(|e| self.failed(e))?;
        self.wait_drq().map_err(|e| self.failed(e))?;
        self.read_sector(&mut sector.0, ReadPath::Port16)
            .map_err(|e| self.failed(e))?;
        if !self.finish_read().map_err(|e| self.failed(e))? {
            return Err(BlockError::Device);
        }
        mix(&sector.0);
        let last = self.sectors.saturating_sub(1);
        for lba in [0, self.sectors / 2, last] {
            self.read_blocks(lba, &mut sector.0)?;
            mix(&sector.0);
        }
        Ok(hash)
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

    /// Gives the windows and the DMA engine back.
    pub fn into_parts(self) -> (A, C, D) {
        (self.attr, self.common, self.dma)
    }

    /// How sectors are read.
    #[must_use]
    pub fn read_path(&self) -> ReadPath {
        self.path
    }

    /// Reads by `path` from now on, if the attach check proved it or a
    /// faster one (diagnostics: a device test compares the paths). `false`,
    /// and nothing changed, for an unproven path.
    pub fn set_read_path(&mut self, path: ReadPath) -> bool {
        let ok = path <= self.proven;
        if ok {
            self.path = path;
        }
        ok
    }

    /// Faster paths that read IDENTIFY differently from the 16-bit port
    /// (so aren't used on this card).
    #[must_use]
    pub fn path_mismatches(&self) -> u8 {
        self.path_mismatches
    }

    /// Sectors the DMA engine refused (buffers not in whole cache lines),
    /// read by 32-bit loads instead.
    #[must_use]
    pub fn dma_refused(&self) -> u32 {
        self.dma_refused
    }

    /// Reads IDENTIFY through the 16-bit port and through `path`, and says
    /// whether they agree byte for byte.
    fn identify_matches(&mut self, path: ReadPath) -> Result<bool, CfError> {
        let mut port = Sector([0; SECTOR]);
        self.command(CMD_IDENTIFY, 0, 0)?;
        self.wait_drq()?;
        self.read_sector(&mut port.0, ReadPath::Port16)?;
        self.finish(SPIN_LIMIT)?;
        let mut other = Sector([0x5A; SECTOR]);
        self.command(CMD_IDENTIFY, 0, 0)?;
        self.wait_drq()?;
        let used = self.read_sector(&mut other.0, path)?;
        let whole = self.finish_read()?;
        Ok(used == path && whole && port.0 == other.0)
    }

    /// Ends a data-in command: the card must have no data left (DRQ clear).
    /// A path that moved too little (a window that takes one card cycle
    /// per 32-bit load, as the emulator once did) leaves the card mid-
    /// transfer, and the next command would read the leftovers: what's
    /// left is drained through the port and reported as `false`.
    fn finish_read(&mut self) -> Result<bool, CfError> {
        let s = self.wait_idle(SPIN_LIMIT)?;
        if s & (ST_ERR | ST_DF) != 0 {
            return Err(self.error());
        }
        if s & ST_DRQ == 0 {
            return Ok(true);
        }
        // At most a sector's words: a card that keeps asking is broken.
        for _ in 0..SECTOR / 2 {
            let _ = self.common.read16(reg::DATA);
            if self.status() & ST_DRQ == 0 {
                return Ok(false);
            }
        }
        Err(CfError::Timeout)
    }

    /// Moves one sector whose data the card has ready (DRQ) by `path`;
    /// returns the path used (a DMA refusal falls back to the window).
    fn read_sector(&mut self, out: &mut [u8; SECTOR], path: ReadPath) -> Result<ReadPath, CfError> {
        if path == ReadPath::Dma {
            if self.dma.arm_from_window(self.window_phys, SECTOR, out) {
                for _ in 0..SPIN_LIMIT {
                    // Empty at the default (0: back to back, unmeasured).
                    #[allow(clippy::reversed_empty_ranges)]
                    for k in 0..POLL_SPIN {
                        core::hint::black_box(k);
                    }
                    match self.dma.done() {
                        Some(true) => return Ok(ReadPath::Dma),
                        // A bus error mid-sector leaves the card's data
                        // phase unknown: it is taken anew (a reset).
                        Some(false) => return Err(CfError::Timeout),
                        None => {}
                    }
                }
                self.dma.abort();
                return Err(CfError::Timeout);
            }
            self.dma_refused = self.dma_refused.wrapping_add(1);
        }
        if path >= ReadPath::Window32 {
            for (k, w) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                *w = self.common.read32(reg::DATA_WINDOW + 4 * k).to_le_bytes();
            }
            return Ok(ReadPath::Window32);
        }
        for pair in out.as_chunks_mut::<2>().0 {
            *pair = self.common.read16(reg::DATA).to_le_bytes();
        }
        Ok(ReadPath::Port16)
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

/// A sector in whole cache lines (a DMA target).
#[repr(C, align(32))]
struct Sector([u8; SECTOR]);

impl<A: RegisterBus8, C: RegisterBus8 + RegisterBus16 + RegisterBus, D: WindowDma> BlockDevice
    for CfCard<A, C, D>
{
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
                self.read_sector(sector, self.path)
                    .map_err(|e| self.failed(e))?;
            }
            // Data left over means the bytes read were wrong, too.
            if !self.finish_read().map_err(|e| self.failed(e))? {
                return Err(BlockError::Device);
            }
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
