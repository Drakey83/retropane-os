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
        }
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
        if self.never_ready || self.wedged {
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
        let w = u16::from_le_bytes([self.buf[self.pos], self.buf[self.pos + 1]]);
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

    fn write_data(&mut self, w: u16) {
        if self.phase != Phase::WriteIn {
            return;
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
        if c.in_reset && !asserted {
            c.hard_reset();
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
        self.0.borrow_mut().read_data()
    }
    fn write16(&mut self, offset: usize, value: u16) {
        assert_eq!(offset, reg::DATA);
        self.0.borrow_mut().write_data(value);
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
    let r = CfCard::init(attr, common, &mut Lines(c.clone()), &mut |ms| waited += ms);
    assert!(matches!(r, Err((CfError::Timeout, _, _))));
    assert!(
        waited <= RESET_HOLD_MS + RESET_SETUP_MS + READY_MS + 2,
        "waited {waited} ms"
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
    let r = CfCard::init(attr, common, &mut Lines(c.clone()), &mut |ms| waited += ms);
    assert!(matches!(r, Err((CfError::Timeout, _, _))));
    assert_eq!(c.borrow().accesses, 0);
    assert!(!c.borrow().in_reset, "RESET released before giving up");
    assert!(waited <= RESET_HOLD_MS + RESET_SETUP_MS + READY_MS);
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
