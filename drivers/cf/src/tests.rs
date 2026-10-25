//! Tests against a simulated card: attribute memory with a CIS and the
//! configuration registers, common memory with an ATA task file over a
//! sector store, and faults to inject.

use std::cell::RefCell;
use std::rc::Rc;

use super::*;

/// The simulated card, shared by its two windows.
struct Card {
    cis: Vec<u8>,
    config_base: usize,
    cor: u8,
    disk: Vec<u8>,
    regs: [u8; 16],
    /// Data phase: the sector buffer, where in it, and what happens at its end.
    buf: Vec<u8>,
    pos: usize,
    phase: Phase,
    /// Sectors still to move in the current command, and the next LBA.
    left: u32,
    lba: u32,
    identify: [u16; 256],
    write_cache: bool,
    supports_cache_off: bool,
    /// Faults.
    error_at: Option<u32>,
    never_ready: bool,
    busy_reads: u32,
    commands: Vec<u8>,
    /// Held in reset by the RESET line.
    in_reset: bool,
    /// Left wedged by an earlier owner (the device's hand-over finding):
    /// busy, with attribute memory reading garbage, until a hardware reset.
    wedged: bool,
    /// The READY line never rises (a dead card or a missing line).
    ready_line_dead: bool,
    /// Accesses to either window (a card in reset must not see any).
    accesses: u32,
    /// Hardware resets seen.
    resets: u32,
    /// The data window (0x400-0x7FF) doesn't alias the data register (a
    /// card or socket that breaks the CF rule): it reads garbage.
    window_broken: bool,
    /// A 32-bit load is one card cycle (one data word), as QEMU's socket
    /// model had it before rp-v0.16.0.
    window_one_cycle: bool,
    /// Data reads by kind: the 16-bit port, the 32-bit window.
    port_reads: u32,
    window_reads: u32,
    /// The simulated clock (µs): every wait adds to it.
    now_us: u64,
    /// When RESET was asserted and released.
    reset_at: Option<(u64, u64)>,
    /// The socket's common-memory timing (`MCMEM`), set by [`Socket`].
    mcmem: u32,
    /// A cycle shorter than this many memory-clock cycles is too fast for
    /// the card: its data reads come back wrong, or (`too_fast_hangs`) it
    /// stays busy.
    too_fast_below: u32,
    /// The card misreads at exactly this timing (its fields, `MCMEM`'s
    /// low 20 bits) and no other: a timing the search never passed.
    garble_at: Option<u32>,
    /// Writes need a longer cycle than this (a card slower to write than to
    /// read): shorter ones store wrong data.
    write_below: u32,
    too_fast_hangs: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Idle,
    ReadOut,
    WriteIn,
}

const SECTORS: u32 = 2048;

fn cis_for(func: u8) -> Vec<u8> {
    // Tuples on even bytes: FUNCID (fixed disk), CONFIG (base 0x200), END.
    let tuples: &[&[u8]] = &[
        &[0x01, 0x03, 0xD9, 0x01, 0xFF],             // DEVICE
        &[0x21, 0x02, func, 0x01],                   // FUNCID
        &[0x1A, 0x05, 0x01, 0x03, 0x00, 0x02, 0x0F], // CONFIG: RASZ 2, base 0x0200
        &[0xFF],
    ];
    let mut cis = vec![0u8; 0x200];
    let mut at = 0;
    for t in tuples {
        for &b in *t {
            cis[at] = b;
            at += 2;
        }
    }
    cis
}

impl Card {
    fn new() -> Self {
        let mut identify = [0u16; 256];
        identify[0] = 0x848A;
        identify[49] = 1 << 9;
        identify[60] = (SECTORS & 0xFFFF) as u16;
        identify[61] = (SECTORS >> 16) as u16;
        let model = b"RetroPane Test CF                       ";
        for i in 0..20 {
            identify[27 + i] = u16::from_be_bytes([model[2 * i], model[2 * i + 1]]);
        }
        Self {
            cis: cis_for(0x04),
            config_base: 0x200,
            cor: 0,
            disk: vec![0; SECTORS as usize * SECTOR],
            regs: [0; 16],
            buf: Vec::new(),
            pos: 0,
            phase: Phase::Idle,
            left: 0,
            lba: 0,
            identify,
            write_cache: true,
            supports_cache_off: true,
            error_at: None,
            never_ready: false,
            busy_reads: 0,
            commands: Vec::new(),
            in_reset: false,
            wedged: false,
            ready_line_dead: false,
            accesses: 0,
            resets: 0,
            window_broken: false,
            window_one_cycle: false,
            port_reads: 0,
            window_reads: 0,
            now_us: 0,
            reset_at: None,
            mcmem: 0xA5A0_0000 | timing::Timing::WINDOWS_MOBILE.mcmem(),
            too_fast_below: 0,
            garble_at: None,
            write_below: 0,
            too_fast_hangs: false,
        }
    }

    /// The socket's timing is shorter than the card can follow.
    fn too_fast(&self) -> bool {
        timing::Timing::from_mcmem(self.mcmem).cycle_cycles() < self.too_fast_below
            || self.garble_at == Some(self.mcmem & 0xF_FFFF)
    }

    /// A hardware reset: back to the power-on state (memory mode, idle).
    fn hard_reset(&mut self) {
        self.resets += 1;
        self.cor = 0;
        self.regs = [0; 16];
        self.phase = Phase::Idle;
        self.buf.clear();
        self.busy_reads = 0;
        self.wedged = false;
    }

    fn status(&mut self) -> u8 {
        if self.never_ready || self.wedged || self.too_fast() && self.too_fast_hangs {
            return ST_BSY;
        }
        if self.busy_reads > 0 {
            self.busy_reads -= 1;
            return ST_BSY;
        }
        let mut s = ST_DRDY | self.regs[reg::STATUS_COMMAND] & ST_ERR;
        if self.phase != Phase::Idle {
            s |= ST_DRQ;
        }
        s
    }

    fn fail(&mut self, err: u8) {
        self.regs[reg::ERROR_FEATURES] = err;
        self.regs[reg::STATUS_COMMAND] = ST_ERR;
        self.phase = Phase::Idle;
    }

    fn start_sector(&mut self) {
        if self.error_at == Some(self.lba) {
            self.fail(0x40); // uncorrectable
            return;
        }
        let o = self.lba as usize * SECTOR;
        match self.phase {
            Phase::ReadOut => self.buf = self.disk[o..o + SECTOR].to_vec(),
            _ => self.buf = vec![0; SECTOR],
        }
        self.pos = 0;
    }

    fn command(&mut self, cmd: u8) {
        // A command written while data is still waiting to be read is
        // ignored: the card stays in its data phase (as QEMU's IDE model
        // does; ATA leaves it undefined).
        if self.phase == Phase::ReadOut {
            return;
        }
        self.commands.push(cmd);
        self.regs[reg::STATUS_COMMAND] = 0;
        self.regs[reg::ERROR_FEATURES] = 0;
        let r = &self.regs;
        self.lba = u32::from(r[reg::LBA0])
            | u32::from(r[reg::LBA1]) << 8
            | u32::from(r[reg::LBA2]) << 16
            | u32::from(r[reg::HEAD] & 0x0F) << 24;
        self.left = if r[reg::COUNT] == 0 {
            256
        } else {
            u32::from(r[reg::COUNT])
        };
        self.busy_reads = 3;
        match cmd {
            CMD_IDENTIFY => {
                self.buf = self.identify.iter().flat_map(|w| w.to_le_bytes()).collect();
                self.pos = 0;
                self.left = 1;
                self.phase = Phase::ReadOut;
            }
            CMD_READ_SECTORS | CMD_WRITE_SECTORS => {
                if self.lba + self.left > SECTORS {
                    self.fail(0x10); // ID not found
                    return;
                }
                self.phase = if cmd == CMD_READ_SECTORS {
                    Phase::ReadOut
                } else {
                    Phase::WriteIn
                };
                self.start_sector();
            }
            CMD_SET_FEATURES => {
                if self.regs[reg::ERROR_FEATURES] == 0 && !self.supports_cache_off {
                    self.fail(0x04);
                } else if self.supports_cache_off {
                    self.write_cache = false;
                } else {
                    self.fail(0x04);
                }
            }
            _ => self.fail(0x04), // aborted
        }
    }

    fn read_data(&mut self) -> u16 {
        if self.phase != Phase::ReadOut {
            return 0xFFFF;
        }
        let mut w = u16::from_le_bytes([self.buf[self.pos], self.buf[self.pos + 1]]);
        if self.too_fast() {
            w ^= 0x0101;
        }
        self.pos += 2;
        if self.pos == self.buf.len() {
            self.left -= 1;
            self.lba += 1;
            if self.left == 0 {
                self.phase = Phase::Idle;
            } else {
                self.busy_reads = 2;
                self.start_sector();
            }
        }
        w
    }

    fn write_data(&mut self, mut w: u16) {
        if self.phase != Phase::WriteIn {
            return;
        }
        // Not the read's 0x0101, which would cancel it on reading back.
        if timing::Timing::from_mcmem(self.mcmem).cycle_cycles() < self.write_below {
            w ^= 0x0202;
        }
        self.buf[self.pos..self.pos + 2].copy_from_slice(&w.to_le_bytes());
        self.pos += 2;
        if self.pos == SECTOR {
            let o = self.lba as usize * SECTOR;
            let data = std::mem::take(&mut self.buf);
            self.disk[o..o + SECTOR].copy_from_slice(&data);
            self.left -= 1;
            self.lba += 1;
            self.busy_reads = 5; // programming
            if self.left == 0 {
                self.phase = Phase::Idle;
            } else {
                self.start_sector();
            }
        }
    }
}

#[derive(Clone)]
struct Attr(Rc<RefCell<Card>>);
#[derive(Clone)]
struct Common(Rc<RefCell<Card>>);

/// The socket's RESET and READY lines, wired to the simulated card.
struct Lines(Rc<RefCell<Card>>);

impl CfLines for Lines {
    fn set_reset(&mut self, asserted: bool) {
        let mut c = self.0.borrow_mut();
        let now = c.now_us;
        if c.in_reset && !asserted {
            c.hard_reset();
            if let Some((at, _)) = c.reset_at {
                c.reset_at = Some((at, now));
            }
        }
        if asserted && !c.in_reset {
            c.reset_at = Some((now, now));
        }
        c.in_reset = asserted;
    }
    fn ready(&mut self) -> bool {
        let c = self.0.borrow();
        !c.in_reset && !c.ready_line_dead && !c.wedged
    }
}

impl RegisterBus8 for Attr {
    fn read8(&mut self, offset: usize) -> u8 {
        let mut c = self.0.borrow_mut();
        c.accesses += 1;
        assert!(!c.in_reset, "attribute memory read during reset");
        if c.wedged {
            return 0x5A; // garbage, as the device showed
        }
        if offset == c.config_base {
            return c.cor;
        }
        c.cis.get(offset).copied().unwrap_or(0)
    }
    fn write8(&mut self, offset: usize, value: u8) {
        let mut c = self.0.borrow_mut();
        c.accesses += 1;
        assert!(!c.in_reset, "attribute memory written during reset");
        if offset == c.config_base && !c.wedged {
            c.cor = value;
        }
    }
}

impl RegisterBus8 for Common {
    fn read8(&mut self, offset: usize) -> u8 {
        let mut c = self.0.borrow_mut();
        c.accesses += 1;
        match offset {
            reg::STATUS_COMMAND => c.status(),
            o if o < 16 => c.regs[o],
            _ => 0xFF,
        }
    }
    fn write8(&mut self, offset: usize, value: u8) {
        let mut c = self.0.borrow_mut();
        c.accesses += 1;
        match offset {
            reg::STATUS_COMMAND => c.command(value),
            o if o < 16 => c.regs[o] = value,
            _ => {}
        }
    }
}

impl RegisterBus16 for Common {
    fn read16(&mut self, offset: usize) -> u16 {
        assert_eq!(offset, reg::DATA);
        let mut c = self.0.borrow_mut();
        c.port_reads += 1;
        c.read_data()
    }
    fn write16(&mut self, offset: usize, value: u16) {
        assert_eq!(offset, reg::DATA);
        self.0.borrow_mut().write_data(value);
    }
}

/// 32-bit loads: only across the data window, two card cycles each (the
/// PXA splits them), low half first.
impl RegisterBus for Common {
    fn read32(&mut self, offset: usize) -> u32 {
        assert!(
            (reg::DATA_WINDOW..0x800).contains(&offset) && offset.is_multiple_of(4),
            "32-bit load outside the data window: {offset:#x}"
        );
        let mut c = self.0.borrow_mut();
        c.accesses += 1;
        c.window_reads += 1;
        let lo = c.read_data();
        if c.window_one_cycle {
            return u32::from(lo);
        }
        let hi = c.read_data();
        if c.window_broken {
            return 0xFFFF_0000 ^ offset as u32;
        }
        u32::from(lo) | u32::from(hi) << 16
    }
    fn write32(&mut self, offset: usize, _: u32) {
        panic!("32-bit store at {offset:#x}: writes use the 16-bit port");
    }
}

/// A DMA engine copying from the card's data window as the PXA's does
/// (a sector of half-word reads), or misbehaving as told.
struct FakeDma {
    card: Rc<RefCell<Card>>,
    armed: bool,
    /// Refuse every arm (the driver then loads 32-bit words itself).
    refuse: bool,
    /// Copy wrong bytes (a DMA that reads the window wrongly).
    corrupt: bool,
    /// Report a bus error after this many good sectors.
    fail_after: Option<u32>,
    sectors: u32,
    src: Vec<u32>,
}

impl FakeDma {
    fn new(card: &Rc<RefCell<Card>>) -> Self {
        Self {
            card: card.clone(),
            armed: false,
            refuse: false,
            corrupt: false,
            fail_after: None,
            sectors: 0,
            src: Vec::new(),
        }
    }
}

impl WindowDma for FakeDma {
    fn arm_from_window(&mut self, src: u32, window: usize, buf: &mut [u8]) -> bool {
        assert!(!self.armed, "armed twice");
        assert_eq!(window, SECTOR);
        if self.refuse || !(buf.as_ptr() as usize).is_multiple_of(32) {
            return false;
        }
        self.src.push(src);
        let mut c = self.card.borrow_mut();
        for pair in buf.as_chunks_mut::<2>().0 {
            *pair = c.read_data().to_le_bytes();
        }
        if self.corrupt {
            buf[7] ^= 0x10;
        }
        self.armed = true;
        true
    }
    fn done(&mut self) -> Option<bool> {
        assert!(self.armed, "polled while disarmed");
        self.armed = false;
        self.sectors += 1;
        Some(self.fail_after.is_none_or(|n| self.sectors <= n))
    }
    fn abort(&mut self) {
        self.armed = false;
    }
}

fn card() -> (Rc<RefCell<Card>>, Attr, Common) {
    let c = Rc::new(RefCell::new(Card::new()));
    (c.clone(), Attr(c.clone()), Common(c))
}

/// Takes the card the way the loader does: lines wired, no real waits.
fn init(
    c: &Rc<RefCell<Card>>,
    attr: Attr,
    common: Common,
) -> Result<CfCard<Attr, Common>, (CfError, Attr, Common)> {
    CfCard::init(attr, common, &mut Lines(c.clone()), &mut no_wait()).map(|(cf, _)| cf)
}

/// A card whose sectors hold a pattern (each different), for data checks.
fn card_with_data() -> (Rc<RefCell<Card>>, Attr, Common) {
    let r = card();
    for (i, b) in r.0.borrow_mut().disk.iter_mut().enumerate() {
        *b = (i * 7 + i / 512) as u8;
    }
    r
}

fn no_wait() -> impl FnMut(u32) {
    |_| {}
}

#[test]
fn identifies_a_storage_card_in_memory_mode_with_its_cache_off() {
    let (c, attr, common) = card();
    let cf = init(&c, attr, common).ok().unwrap();
    assert_eq!(cf.sectors(), SECTORS);
    assert_eq!(cf.model(), b"RetroPane Test CF");
    assert!(cf.write_cache_off());
    let c = c.borrow();
    assert_eq!(c.cor, 0, "memory mode (configuration index 0)");
    assert!(!c.write_cache);
    assert_eq!(c.regs[reg::DEVICE_CONTROL] & CTL_NIEN, CTL_NIEN);
}

#[test]
fn reads_and_writes_sectors_including_runs_over_256() {
    let (c, attr, common) = card_with_data();
    let mut cf = init(&c, attr, common).ok().unwrap();
    let mut buf = vec![0u8; 300 * SECTOR];
    cf.read_blocks(100, &mut buf).unwrap();
    assert_eq!(buf[..], c.borrow().disk[100 * SECTOR..400 * SECTOR]);
    let data: Vec<u8> = (0..300 * SECTOR).map(|i| (i * 13) as u8).collect();
    cf.write_blocks(1000, &data).unwrap();
    assert_eq!(c.borrow().disk[1000 * SECTOR..1300 * SECTOR], data[..]);
    // Two commands each way: 256 sectors (count 0), then 44.
    let cmds = c.borrow().commands.clone();
    assert_eq!(
        cmds.iter().filter(|&&x| x == CMD_READ_SECTORS).count(),
        2,
        "{cmds:x?}"
    );
    assert_eq!(cmds.iter().filter(|&&x| x == CMD_WRITE_SECTORS).count(), 2);
}

/// A card the loader attached, taken over for a program's card log: no
/// reset or IDENTIFY, sectors written through the port, its end kept.
#[test]
fn an_adopted_card_writes_without_being_reset() {
    let (c, attr, common) = card();
    let (attr, common) = init(&c, attr, common).ok().unwrap().into_inner();
    let commands = c.borrow().commands.len();
    let resets = c.borrow().resets;
    let mut cf = CfCard::adopt(attr, common, SECTORS);
    let data = [0x6Bu8; SECTOR];
    cf.write_blocks(77, &data).unwrap();
    assert_eq!(c.borrow().disk[77 * SECTOR..78 * SECTOR], data[..]);
    assert_eq!(c.borrow().commands[commands..], [CMD_WRITE_SECTORS]);
    assert_eq!(c.borrow().resets, resets, "no reset");
    assert_eq!(cf.write_blocks(SECTORS, &data), Err(BlockError::OutOfRange));
}

#[test]
fn transfers_past_the_end_are_refused_before_the_card_sees_them() {
    let (c, attr, common) = card();
    let mut cf = init(&c, attr, common).ok().unwrap();
    let before = c.borrow().commands.len();
    let mut buf = vec![0u8; 2 * SECTOR];
    assert_eq!(
        cf.read_blocks(SECTORS - 1, &mut buf),
        Err(BlockError::OutOfRange)
    );
    assert_eq!(cf.write_blocks(u32::MAX, &buf), Err(BlockError::OutOfRange));
    assert_eq!(
        cf.read_blocks(0, &mut buf[..100]),
        Err(BlockError::OutOfRange)
    );
    assert_eq!(c.borrow().commands.len(), before);
    cf.read_blocks(SECTORS - 2, &mut buf).unwrap();
}

#[test]
fn a_bad_sector_is_an_error_with_the_card_usable_after() {
    let (c, attr, common) = card_with_data();
    let mut cf = init(&c, attr, common).ok().unwrap();
    c.borrow_mut().error_at = Some(50);
    let mut buf = vec![0u8; 8 * SECTOR];
    assert_eq!(cf.read_blocks(48, &mut buf), Err(BlockError::Device));
    assert_eq!(cf.last_error(), 0x40);
    c.borrow_mut().error_at = None;
    cf.read_blocks(48, &mut buf).unwrap();
    assert_eq!(buf[..], c.borrow().disk[48 * SECTOR..56 * SECTOR]);
}

#[test]
fn io_cards_and_missing_cis_are_left_alone() {
    let (c, attr, common) = card();
    c.borrow_mut().cis = cis_for(0x06); // a network card
    match init(&c, attr, common) {
        Err((CfError::NotStorage, _, _)) => {}
        _ => panic!("an I/O card was taken"),
    }
    assert!(
        c.borrow().commands.is_empty(),
        "nothing sent to an I/O card"
    );
    assert_eq!(c.borrow().cor, 0);
    let (c, attr, common) = card();
    c.borrow_mut().cis = vec![0xFF; 0x200];
    assert!(matches!(
        init(&c, attr, common),
        Err((CfError::NotStorage, _, _))
    ));
}

#[test]
fn a_card_that_never_gets_ready_times_out() {
    let (c, attr, common) = card();
    c.borrow_mut().never_ready = true;
    let mut waited = 0u32;
    let r = CfCard::init(attr, common, &mut Lines(c.clone()), &mut |us| waited += us);
    assert!(matches!(r, Err((CfError::Timeout, _, _))));
    assert!(
        waited <= RESET_HOLD_US + (RESET_SETUP_MS + READY_MS + 2) * 1000,
        "waited {waited} us"
    );
}

/// The device's finding (docs/hw/cf.md): a card an earlier owner left
/// wedged reads garbage and stays busy. Without the RESET line it can't be
/// taken (the old driver failed like this); with it, it comes back.
#[test]
fn a_card_left_wedged_by_an_earlier_owner_is_reset_and_taken() {
    let (c, attr, common) = card_with_data();
    c.borrow_mut().wedged = true;
    match CfCard::init(attr, common, &mut NoLines, &mut no_wait()) {
        Err((CfError::NotStorage, _, _)) => {}
        Err((e, _, _)) => panic!("{e:?}"),
        Ok(_) => panic!("a wedged card can't be taken without a reset"),
    }
    let (attr, common) = (Attr(c.clone()), Common(c.clone()));
    let (mut cf, seen) = CfCard::init(attr, common, &mut Lines(c.clone()), &mut no_wait())
        .ok()
        .unwrap();
    assert_eq!(c.borrow().resets, 1);
    assert_eq!(
        seen,
        ResetSeen {
            busy_in_reset: true,
            ready_ms: 0
        }
    );
    let mut buf = vec![0u8; SECTOR];
    cf.read_blocks(0, &mut buf).unwrap();
    assert_eq!(buf[..], c.borrow().disk[..SECTOR]);
}

/// A card whose READY never rises is given up without either window being
/// touched (a card in reset or busy may hold the bus), within the bound.
#[test]
fn a_card_that_never_comes_out_of_reset_is_never_accessed() {
    let (c, attr, common) = card();
    c.borrow_mut().ready_line_dead = true;
    let mut waited = 0u32;
    let r = CfCard::init(attr, common, &mut Lines(c.clone()), &mut |us| waited += us);
    assert!(matches!(r, Err((CfError::Timeout, _, _))));
    assert_eq!(c.borrow().accesses, 0);
    assert!(!c.borrow().in_reset, "RESET released before giving up");
    assert!(waited <= RESET_HOLD_US + (RESET_SETUP_MS + READY_MS) * 1000);
}

/// Item 4 of the storage report: RESET is a pulse of at least the CF
/// specification's 10 µs, with margin, not Windows Mobile's 100 ms; the
/// rest of the attach waits on the card (READY, then BSY), not the clock.
#[test]
fn reset_is_a_short_pulse_then_the_card_is_waited_for() {
    let (c, attr, common) = card();
    let clock = c.clone();
    let (_, seen) = CfCard::init(attr, common, &mut Lines(c.clone()), &mut |us| {
        clock.borrow_mut().now_us += u64::from(us);
    })
    .ok()
    .unwrap();
    let (on, off) = c.borrow().reset_at.unwrap();
    let held = off - on;
    assert!((50..=1000).contains(&held), "RESET held {held} us");
    assert_eq!(seen.ready_ms, 0);
    // The whole attach: the pulse, the set-up time, the soft reset's
    // millisecond; no other fixed wait.
    let total = c.borrow().now_us;
    assert!(total <= 22_000, "attach waited {total} us");
}

/// The window paths: 32-bit loads once IDENTIFY reads the same through
/// them as through the port, then DMA once it does too; every byte right.
#[test]
fn reads_take_the_fastest_path_that_reads_identify_right() {
    let (c, attr, common) = card_with_data();
    let mut cf = init(&c, attr, common).ok().unwrap();
    assert_eq!(cf.read_path(), ReadPath::Window32);
    assert_eq!(cf.path_mismatches(), 0);
    let mut buf = vec![0u8; 300 * SECTOR];
    let ports = c.borrow().port_reads;
    cf.read_blocks(100, &mut buf).unwrap();
    assert_eq!(buf[..], c.borrow().disk[100 * SECTOR..400 * SECTOR]);
    assert_eq!(c.borrow().port_reads, ports, "no 16-bit reads");
    assert_eq!(c.borrow().window_reads % 128, 0);
    // With DMA: its copy is checked against the port, then used.
    let cf = cf.with_dma(FakeDma::new(&c), 0x3C00_0000);
    let mut cf = cf;
    assert_eq!(cf.read_path(), ReadPath::Dma);
    let window = c.borrow().window_reads;
    let mut aligned = Aligned([0; 64 * SECTOR]);
    cf.read_blocks(7, &mut aligned.0).unwrap();
    assert_eq!(aligned.0[..], c.borrow().disk[7 * SECTOR..71 * SECTOR]);
    assert_eq!(c.borrow().window_reads, window, "every sector by DMA");
    let (_, _, dma) = cf.into_parts();
    assert!(dma.src.iter().all(|&s| s == 0x3C00_0400));
    assert_eq!(dma.sectors, 1 + 64, "the check, then each sector");
}

/// `init_with` (the engine from the start: one copy of the driver per
/// image) ends where `init` then `with_dma` does: the same path, the same
/// check, the same reads.
#[test]
fn init_with_an_engine_matches_init_then_with_dma() {
    let (c, attr, common) = card_with_data();
    let lines = &mut Lines(c.clone());
    let (cf, _) = CfCard::init_with(
        attr,
        common,
        FakeDma::new(&c),
        Some(0x3C00_0000),
        lines,
        &mut no_wait(),
    )
    .ok()
    .unwrap();
    let mut cf = cf;
    assert_eq!(cf.read_path(), ReadPath::Dma);
    assert_eq!(cf.path_mismatches(), 0);
    let mut aligned = Aligned([0; 64 * SECTOR]);
    cf.read_blocks(7, &mut aligned.0).unwrap();
    assert_eq!(aligned.0[..], c.borrow().disk[7 * SECTOR..71 * SECTOR]);
    let (_, _, dma) = cf.into_parts();
    assert_eq!(dma.sectors, 1 + 64, "the check, then each sector");
    // No address: the engine isn't tried; a broken window keeps the port
    // and the engine comes back untried.
    let (c, attr, common) = card_with_data();
    let lines = &mut Lines(c.clone());
    let (cf, _) = CfCard::init_with(attr, common, FakeDma::new(&c), None, lines, &mut no_wait())
        .ok()
        .unwrap();
    assert_eq!(cf.read_path(), ReadPath::Window32);
    let (c, attr, common) = card_with_data();
    c.borrow_mut().window_broken = true;
    let lines = &mut Lines(c.clone());
    let (cf, _) = CfCard::init_with(
        attr,
        common,
        FakeDma::new(&c),
        Some(0x3C00_0000),
        lines,
        &mut no_wait(),
    )
    .ok()
    .unwrap();
    assert_eq!(
        (cf.read_path(), cf.path_mismatches()),
        (ReadPath::Port16, 1)
    );
    assert_eq!(cf.into_parts().2.sectors, 0, "DMA not tried");
}

#[repr(C, align(32))]
struct Aligned([u8; 64 * SECTOR]);

/// A card (or socket) whose window doesn't repeat the data register keeps
/// the 16-bit port: the check catches it before any sector is read wrong.
#[test]
fn a_window_that_reads_wrong_keeps_the_port() {
    let (c, attr, common) = card_with_data();
    c.borrow_mut().window_broken = true;
    let cf = init(&c, attr, common).ok().unwrap();
    assert_eq!(cf.read_path(), ReadPath::Port16);
    assert_eq!(cf.path_mismatches(), 1);
    let mut cf = cf.with_dma(FakeDma::new(&c), 0x3C00_0000);
    assert_eq!(cf.read_path(), ReadPath::Port16, "DMA not tried");
    let mut buf = vec![0u8; 8 * SECTOR];
    let windows = c.borrow().window_reads;
    cf.read_blocks(3, &mut buf).unwrap();
    assert_eq!(buf[..], c.borrow().disk[3 * SECTOR..11 * SECTOR]);
    assert_eq!(c.borrow().window_reads, windows);
}

/// Class guard (found in the emulator, 2026-10-16): a window path that
/// moves too little leaves the card mid-transfer, and the next command read
/// the leftovers (sector 0 came back as IDENTIFY's tail). The check drains
/// what's left and refuses the path; sectors read right after it.
#[test]
fn a_window_that_moves_too_little_is_refused_and_leaves_nothing_behind() {
    let (c, attr, common) = card_with_data();
    c.borrow_mut().window_one_cycle = true;
    let mut cf = init(&c, attr, common).ok().unwrap();
    assert_eq!(cf.read_path(), ReadPath::Port16);
    assert_eq!(cf.path_mismatches(), 1);
    let mut buf = vec![0u8; 3 * SECTOR];
    cf.read_blocks(0, &mut buf).unwrap();
    assert_eq!(buf[..], c.borrow().disk[..3 * SECTOR]);
}

/// A DMA engine that copies wrongly is never used; one that refuses a
/// buffer costs the 32-bit path for that sector, counted; one that fails
/// mid-transfer kills the card (its data phase is unknown), so the next
/// attach resets it.
#[test]
fn a_dma_that_misreads_refuses_or_fails_is_handled() {
    let (c, attr, common) = card_with_data();
    let cf = init(&c, attr, common).ok().unwrap();
    let mut dma = FakeDma::new(&c);
    dma.corrupt = true;
    let mut cf = cf.with_dma(dma, 0);
    assert_eq!(cf.read_path(), ReadPath::Window32);
    assert!(!cf.set_read_path(ReadPath::Dma), "an unproven path");
    assert_eq!(cf.read_path(), ReadPath::Window32);
    assert_eq!(cf.path_mismatches(), 1);
    let mut aligned = Aligned([0; 64 * SECTOR]);
    cf.read_blocks(0, &mut aligned.0).unwrap();
    assert_eq!(aligned.0[..], c.borrow().disk[..64 * SECTOR]);

    let (attr, common) = cf.into_inner();
    let cf = init(&c, attr, common).ok().unwrap();
    let mut cf = cf.with_dma(FakeDma::new(&c), 0);
    assert_eq!(cf.read_path(), ReadPath::Dma);
    // Diagnostics may step down to a slower proven path, never up past one.
    assert!(cf.set_read_path(ReadPath::Port16));
    let mut buf = vec![0u8; 2 * SECTOR];
    cf.read_blocks(9, &mut buf).unwrap();
    assert_eq!(buf[..], c.borrow().disk[9 * SECTOR..11 * SECTOR]);
    assert!(cf.set_read_path(ReadPath::Dma));
    // An odd buffer (not whole cache lines): refused, read by the CPU.
    let mut odd = vec![0u8; 4 * SECTOR + 1];
    let at = if (odd.as_ptr() as usize).is_multiple_of(32) {
        1
    } else {
        0
    };
    cf.read_blocks(5, &mut odd[at..at + 4 * SECTOR]).unwrap();
    assert_eq!(
        odd[at..at + 4 * SECTOR],
        c.borrow().disk[5 * SECTOR..9 * SECTOR]
    );
    assert_eq!(cf.dma_refused(), 4);

    let (attr, common, _) = cf.into_parts();
    let cf = init(&c, attr, common).ok().unwrap();
    let mut dma = FakeDma::new(&c);
    dma.fail_after = Some(3);
    let mut cf = cf.with_dma(dma, 0);
    assert_eq!(cf.read_path(), ReadPath::Dma);
    assert_eq!(cf.read_blocks(0, &mut aligned.0), Err(BlockError::Device));
    assert!(cf.is_dead());
}

/// Class guard for the stalled USB loop: a card that stops answering costs
/// one timeout; every later transfer fails at once without touching it.
#[test]
fn a_card_that_stops_answering_costs_one_timeout() {
    let (c, attr, common) = card_with_data();
    let mut cf = init(&c, attr, common).ok().unwrap();
    c.borrow_mut().never_ready = true;
    let mut buf = vec![0u8; SECTOR];
    assert_eq!(cf.read_blocks(0, &mut buf), Err(BlockError::Device));
    assert!(cf.is_dead());
    let before = c.borrow().accesses;
    for _ in 0..100 {
        assert_eq!(cf.read_blocks(0, &mut buf), Err(BlockError::Device));
        assert_eq!(cf.write_blocks(0, &buf), Err(BlockError::Device));
    }
    assert_eq!(c.borrow().accesses, before, "a dead card was polled again");
    // Taken anew (the loader's cf command), it is reset and works again.
    c.borrow_mut().never_ready = false;
    let (attr, common) = cf.into_inner();
    let mut cf = init(&c, attr, common).ok().unwrap();
    assert!(!cf.is_dead());
    cf.read_blocks(0, &mut buf).unwrap();
}

/// A refused sector is the card answering: it stays in use.
#[test]
fn a_device_error_does_not_kill_the_card() {
    let (c, attr, common) = card_with_data();
    let mut cf = init(&c, attr, common).ok().unwrap();
    c.borrow_mut().error_at = Some(3);
    let mut buf = vec![0u8; SECTOR];
    assert!(cf.read_blocks(3, &mut buf).is_err());
    assert!(!cf.is_dead());
}

#[test]
fn a_card_without_lba_or_cache_control_is_handled() {
    let (c, attr, common) = card();
    c.borrow_mut().identify[49] = 0;
    assert!(matches!(
        init(&c, attr, common),
        Err((CfError::BadIdentify, _, _))
    ));
    let (c, attr, common) = card();
    c.borrow_mut().supports_cache_off = false;
    let mut cf = init(&c, attr, common).ok().unwrap();
    assert!(!cf.write_cache_off());
    let mut buf = vec![0u8; SECTOR];
    cf.read_blocks(0, &mut buf).unwrap();
}

/// Hostile cards: random CIS bytes and random IDENTIFY words. Nothing may
/// panic or loop; whatever capacity is accepted is what the card said.
#[test]
fn hostile_cards_never_panic_or_hang() {
    let mut x: u64 = 0xCF00_2026;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for i in 0..3000 {
        let (c, attr, common) = card();
        {
            let mut c = c.borrow_mut();
            if i % 2 == 0 {
                for _ in 0..1 + rnd() % 8 {
                    let at = rnd() as usize % 0x80;
                    c.cis[at] = rnd() as u8;
                }
            } else {
                for _ in 0..1 + rnd() % 8 {
                    let at = rnd() as usize % 256;
                    c.identify[at] = rnd() as u16;
                }
            }
        }
        if let Ok(mut cf) = init(&c, attr, common) {
            let id = c.borrow().identify;
            assert_eq!(cf.sectors(), u32::from(id[60]) | u32::from(id[61]) << 16);
            let mut buf = vec![0u8; SECTOR];
            let _ = cf.read_blocks(cf.sectors().saturating_sub(1), &mut buf);
        }
    }
}

/// The socket's timing register as the loader hands it to
/// [`CfCard::tune`]: kept in the simulated card, every write recorded.
struct Socket {
    card: Rc<RefCell<Card>>,
    writes: Vec<u32>,
    /// The card stops answering at the first write (a card that dies).
    kills: bool,
}

impl SocketTiming for Socket {
    fn read(&mut self) -> u32 {
        self.card.borrow().mcmem
    }
    fn write(&mut self, value: u32) {
        let mut c = self.card.borrow_mut();
        c.mcmem = value;
        c.never_ready |= self.kills;
        self.writes.push(value);
    }
}

fn socket(c: &Rc<RefCell<Card>>) -> Socket {
    Socket {
        card: c.clone(),
        writes: Vec::new(),
        kills: false,
    }
}

/// A CIS whose DEVICE tuple rates 250 ns and, when given, a DEVICE_OC tuple
/// (its condition bytes, then the device ID and speed bytes) after it.
fn cis_rated(oc: Option<&[u8]>) -> Vec<u8> {
    let mut tuples: Vec<Vec<u8>> = vec![vec![0x01, 0x03, 0xD9, 0x01, 0xFF]];
    if let Some(body) = oc {
        let mut t = vec![0x1C, body.len() as u8];
        t.extend_from_slice(body);
        tuples.push(t);
    }
    tuples.push(vec![0x21, 0x02, 0x04, 0x01]);
    tuples.push(vec![0x1A, 0x05, 0x01, 0x03, 0x00, 0x02, 0x0F]);
    tuples.push(vec![0xFF]);
    let mut cis = vec![0u8; 0x200];
    let mut at = 0;
    for b in tuples.iter().flatten() {
        cis[at] = *b;
        at += 2;
    }
    cis
}

/// The owner's card's 3.3 V rating: 80 ns (its CIS read on the X51v,
/// storage-speed.md A1).
const OC_80NS: &[u8] = &[0x02, 0xDF, 0x79, 0x01, 0xFF];

fn rated_card(oc: Option<&[u8]>) -> (Rc<RefCell<Card>>, CfCard<Attr, Common>) {
    let (c, attr, common) = card_with_data();
    c.borrow_mut().cis = cis_rated(oc);
    let cf = init(&c, attr, common).ok().unwrap();
    (c, cf)
}

/// Every sector the card holds, read through the driver, matches the disk.
fn reads_right(c: &Rc<RefCell<Card>>, cf: &mut CfCard<Attr, Common>) {
    let mut buf = vec![0u8; 64 * SECTOR];
    for lba in (0..SECTORS).step_by(64) {
        cf.read_blocks(lba, &mut buf).unwrap();
        let o = lba as usize * SECTOR;
        assert!(
            buf == c.borrow().disk[o..o + buf.len()],
            "sectors at {lba} misread"
        );
    }
}

#[test]
fn the_cis_rating_prefers_the_3v3_tuple() {
    assert_eq!(rated_card(None).1.rated_ns(), Some(250));
    assert_eq!(rated_card(Some(OC_80NS)).1.rated_ns(), Some(80));
    // Two condition bytes (bit 7: another follows), then the speed.
    let two = [0x82, 0x02, 0xDF, 0x79];
    assert_eq!(rated_card(Some(&two)).1.rated_ns(), Some(80));
    // A 3.3 V tuple with no speed leaves the 5 V one.
    assert_eq!(rated_card(Some(&[0x02, 0xD0])).1.rated_ns(), Some(250));
    // No speed tuple at all: no rating, and the card is still measured
    // (ruling M doesn't need one).
    let (c, attr, common) = card_with_data();
    c.borrow_mut().cis = cis_for(0x04);
    c.borrow_mut().cis[0] = 0x00; // the DEVICE tuple's code turned to NULL
    c.borrow_mut().cis[2] = 0x00;
    let mut cf = init(&c, attr, common).ok().unwrap();
    assert_eq!(cf.rated_ns(), None);
    assert!(matches!(cf.tune(&mut socket(&c)), Tuned::Applied { .. }));
    // An adopted card has no CIS read.
    let (_, attr, common) = card_with_data();
    assert_eq!(CfCard::adopt(attr, common, SECTORS).rated_ns(), None);
}

/// The search: from the timing the socket had, each field shortened to the
/// card's limit, then the margin. A card that follows down to 20 cycles:
/// the strobe to 0 (24), the hold to 4 (20), the set-up stays (19 fails).
fn tuned(below: u32, hangs: bool) -> (Rc<RefCell<Card>>, CfCard<Attr, Common>, Socket, Tuned) {
    let (c, cf) = rated_card(Some(OC_80NS));
    c.borrow_mut().too_fast_below = below;
    c.borrow_mut().too_fast_hangs = hangs;
    let mut cf = cf;
    let mut s = socket(&c);
    let t = cf.tune(&mut s);
    (c, cf, s, t)
}

fn cycles(reg: u32) -> u32 {
    timing::Timing::from_mcmem(reg).cycle_cycles()
}

#[test]
fn the_search_finds_the_cards_limit_and_backs_off_one_and_a_half_times() {
    let (c, mut cf, s, t) = tuned(20, false);
    let Tuned::Applied {
        was,
        now,
        limit,
        clamped,
    } = t
    else {
        panic!("{t:?}");
    };
    assert_eq!(was, 0xA5A0_0000 | timing::Timing::WINDOWS_MOBILE.mcmem());
    let lim = timing::Timing::from_mcmem(limit);
    assert_eq!(
        lim,
        timing::Timing {
            set: 8,
            asst: 0,
            hold: 4
        }
    );
    assert_eq!(cycles(limit), 20);
    assert_eq!(now & 0xF_FFFF, timing::with_margin(lim).0.mcmem());
    assert_eq!(
        (now >> 20, limit >> 20),
        (0xA5A, 0xA5A),
        "the other bits kept"
    );
    assert!(cycles(now) * 10 >= cycles(limit) * 15);
    assert!(!clamped);
    // The truth first: the slowest timing; the last write is the result.
    assert_eq!(s.writes[0] & 0xF_FFFF, timing::Timing::SLOWEST.mcmem());
    assert_eq!(s.writes.last(), Some(&now));
    assert_eq!(c.borrow().mcmem, now);
    assert!(!cf.is_dead());
    reads_right(&c, &mut cf);
}

/// A card that hangs when timed too fast (a wait runs out, which marks it
/// dead) answers again at the last right step: the same limit, alive.
#[test]
fn a_card_that_hangs_below_its_limit_is_measured_the_same() {
    let (c, mut cf, _, t) = tuned(20, true);
    let Tuned::Applied { limit, .. } = t else {
        panic!("{t:?}");
    };
    assert_eq!(cycles(limit), 20);
    assert!(!cf.is_dead());
    reads_right(&c, &mut cf);
}

/// A card slower than Windows Mobile's timing (it misreads there): the
/// search starts from the socket's slowest and the card gets more time.
#[test]
fn a_card_slower_than_windows_mobiles_timing_gets_more_time() {
    let (c, mut cf, _, t) = tuned(60, false);
    let Tuned::Applied { now, limit, .. } = t else {
        panic!("{t:?}");
    };
    assert_eq!(cycles(limit), 60);
    assert!(cycles(now) >= 90);
    assert!(cycles(now) > timing::Timing::WINDOWS_MOBILE.cycle_cycles());
    reads_right(&c, &mut cf);
}

/// The final check fails (the card misreads at the margin's timing alone):
/// the old timing goes back and the card reads right there.
#[test]
fn a_timing_the_final_check_refuses_is_put_back() {
    let (c, cf) = rated_card(Some(OC_80NS));
    let mut cf = cf;
    let lim = timing::Timing {
        set: 8,
        asst: 0,
        hold: 4,
    };
    c.borrow_mut().too_fast_below = 20;
    c.borrow_mut().garble_at = Some(timing::with_margin(lim).0.mcmem());
    let was = c.borrow().mcmem;
    let mut s = socket(&c);
    let t = cf.tune(&mut s);
    let Tuned::Refused {
        was: kept, card_ok, ..
    } = t
    else {
        panic!("{t:?}");
    };
    assert_eq!((kept, card_ok), (was, true));
    assert_eq!(c.borrow().mcmem, was);
    assert!(!cf.is_dead());
    reads_right(&c, &mut cf);
}

/// A card that dies at the first change: the old timing back, the card
/// dead (one timeout, then left for the loader to take anew), and a dead
/// card isn't tuned again.
#[test]
fn a_card_that_dies_is_left_dead_at_the_old_timing() {
    let (c, mut cf) = rated_card(Some(OC_80NS));
    let was = c.borrow().mcmem;
    let mut s = socket(&c);
    s.kills = true;
    assert!(matches!(cf.tune(&mut s), Tuned::Kept(_)));
    assert_eq!(c.borrow().mcmem, was);
    assert!(cf.is_dead());
    assert!(matches!(cf.tune(&mut s), Tuned::Kept(_)));
}

/// A card unreadable even at the slowest timing: nothing kept but the old
/// timing.
#[test]
fn a_card_unreadable_at_the_slowest_keeps_the_old_timing() {
    let (c, mut cf) = rated_card(Some(OC_80NS));
    c.borrow_mut().never_ready = true;
    let was = c.borrow().mcmem;
    let mut s = socket(&c);
    assert!(matches!(cf.tune(&mut s), Tuned::Kept(_)));
    assert_eq!(c.borrow().mcmem, was);
    assert_eq!(s.writes.last(), Some(&was));
}

thread_local! {
    /// The card [`set_mcmem`] times (a `fn(u32)` can't capture it).
    static TIMED: RefCell<Option<Rc<RefCell<Card>>>> = const { RefCell::new(None) };
}

fn set_mcmem(v: u32) {
    TIMED.with(|t| {
        if let Some(c) = t.borrow().as_ref() {
            c.borrow_mut().mcmem = v;
        }
    });
}

/// A tuned card writes at the timing writes were proven at: without
/// `write_at` the simulated card (wrong when timed too fast, writes too)
/// stores garbage at the read timing; with it, every block lands, and the
/// read timing is back after each write.
#[test]
fn a_tuned_card_writes_at_the_write_timing() {
    let (c, mut cf, _, t) = tuned(20, false);
    // Reads right from 20 cycles (tuned to 30), writes only from 40.
    c.borrow_mut().write_below = 40;
    let Tuned::Applied { was, now, .. } = t else {
        panic!("{t:?}");
    };
    let block = [0x5Au8; SECTOR];
    let mut back = [0u8; SECTOR];
    cf.write_blocks(7, &block).unwrap();
    cf.read_blocks(7, &mut back).unwrap();
    assert_ne!(back, block, "the sim must garble writes at the read timing");
    TIMED.with(|t| *t.borrow_mut() = Some(c.clone()));
    cf.write_at(set_mcmem, now, was);
    cf.write_blocks(9, &block).unwrap();
    assert_eq!(c.borrow().mcmem, now, "the read timing back after a write");
    cf.read_blocks(9, &mut back).unwrap();
    assert_eq!(back, block);
    TIMED.with(|t| *t.borrow_mut() = None);
}
