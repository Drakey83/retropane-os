//! The PXA27x core clocks, operating points and performance monitor
//! (ADR-0031, audit item 67).
//!
//! An operating point is the core PLL's multipliers (`CCCR` L and 2N), the
//! alternate memory clock bit (`CCCR` A) and the mode bits (`CLKCFG` turbo,
//! half-turbo, fast bus). The arithmetic is the PXA27x Developer's Manual's
//! (section 3, "Clocks and Power Manager"); the legal set, the voltages and
//! the frequency-change sequence's constraints (no SDRAM access while the
//! PLL relocks: the change runs from one I-cache line; SDRAM refresh set for
//! the slower memory clock before the change) are facts about the chip, not
//! a copy of anyone's policy. Which point to run when is RetroPane's own
//! (ADR-0031).
//!
//! Integer kHz throughout (no float: docs/perf.md).

/// Core clock configuration register (`CCCR`).
pub const CCCR: usize = 0x4130_0000;
/// Core clock status register (`CCSR`): the multipliers actually in use.
pub const CCSR: usize = 0x4130_000C;
/// SDRAM configuration (`MDCNFG`): row counts.
pub const MDCNFG: usize = 0x4800_0000;
/// SDRAM refresh control (`MDREFR`): refresh interval and clock dividers.
pub const MDREFR: usize = 0x4800_0004;

/// The 13 MHz crystal every PLL multiplies, in kHz.
pub const OSC_KHZ: u32 = 13_000;

/// `CCCR` A: memory clock = system bus clock.
pub const CCCR_A: u32 = 1 << 25;
const CCCR_2N: u32 = 0xF << 7;
const CCCR_L: u32 = 0x1F;

/// `CLKCFG` bits (CP14 c6).
pub mod clkcfg {
    /// Turbo: the core runs at the turbo (PLL) frequency.
    pub const T: u32 = 1 << 0;
    /// Frequency change: relock the core PLL to `CCCR`'s multipliers.
    pub const F: u32 = 1 << 1;
    /// Half-turbo: the core at half the turbo frequency.
    pub const HT: u32 = 1 << 2;
    /// Fast bus: system bus = run frequency (else half).
    pub const B: u32 = 1 << 3;
}

/// `MDREFR` refresh interval field.
pub const MDREFR_DRI: u32 = 0xFFF;
/// `MDREFR` SDCLK1 and SDCLK2 divide-by-2 (the SDRAM clocks; SDCLK0, the
/// synchronous static clock, is left alone).
pub const MDREFR_DB2: u32 = 1 << 17 | 1 << 19;

/// The SDRAM refresh period every row must meet (JEDEC: 64 ms).
pub const TREF_MS: u32 = 64;
/// Fastest SDRAM clock the PXA27x memory controller drives, in kHz.
pub const SDCLK_MAX_KHZ: u32 = 104_000;
/// Fastest system bus (and memory clock), in kHz.
pub const BUS_MAX_KHZ: u32 = 208_000;
/// Fastest core clock, in kHz.
pub const CORE_MAX_KHZ: u32 = 624_000;
/// The fastest turbo PLL a test past Intel's rating may ask for, in kHz:
/// L 16 x 2N 8 (cpu-full-potential.md C2, `rp-clocks`' stage C2b;
/// `docs/research/sheets/c2-cpu-overclock-test1.md`). Nothing ships past
/// [`CORE_MAX_KHZ`] (ADR-0031) until the per-device self-test is decided.
pub const PAST_RATING_MAX_KHZ: u32 = 832_000;

/// An operating point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    /// Run-mode multiplier (`CCCR` L): run = 13 MHz x L.
    pub l: u8,
    /// Turbo multiplier x 2 (`CCCR` 2N): turbo = run x 2N / 2.
    pub n2: u8,
    /// Alternate memory clock (`CCCR` A): memory clock = system bus.
    pub a: bool,
    /// Fast bus (`CLKCFG` B).
    pub b: bool,
    /// Turbo (`CLKCFG` T).
    pub turbo: bool,
    /// Half-turbo (`CLKCFG` HT).
    pub half: bool,
}

impl Point {
    /// The run frequency (core in run mode), kHz.
    #[must_use]
    pub const fn run_khz(self) -> u32 {
        OSC_KHZ * self.l as u32
    }

    /// The core PLL's turbo frequency, kHz.
    #[must_use]
    pub const fn turbo_khz(self) -> u32 {
        self.run_khz() * self.n2 as u32 / 2
    }

    /// The core clock as configured, kHz.
    #[must_use]
    pub const fn core_khz(self) -> u32 {
        match (self.turbo, self.half) {
            (true, false) => self.turbo_khz(),
            (true, true) => self.turbo_khz() / 2,
            (false, _) => self.run_khz(),
        }
    }

    /// The system bus clock, kHz.
    #[must_use]
    pub const fn bus_khz(self) -> u32 {
        if self.b {
            self.run_khz()
        } else {
            self.run_khz() / 2
        }
    }

    /// The memory controller clock, kHz (manual: A selects the system bus;
    /// otherwise run / 1, 2 or 4 by L).
    #[must_use]
    pub const fn mem_khz(self) -> u32 {
        if self.a {
            self.bus_khz()
        } else if self.l <= 10 {
            self.run_khz()
        } else if self.l <= 20 {
            self.run_khz() / 2
        } else {
            self.run_khz() / 4
        }
    }

    /// Whether the SDRAM clock must be the memory clock halved (to stay at
    /// or below [`SDCLK_MAX_KHZ`]).
    #[must_use]
    pub const fn sdram_div2(self) -> bool {
        self.mem_khz() > SDCLK_MAX_KHZ
    }

    /// The SDRAM clock, kHz.
    #[must_use]
    pub const fn sdclk_khz(self) -> u32 {
        if self.sdram_div2() {
            self.mem_khz() / 2
        } else {
            self.mem_khz()
        }
    }

    /// Within the chip's limits: L 2-31 with a 2N of 2-6, turbo PLL at most
    /// 624 MHz, bus at most 208 MHz.
    #[must_use]
    pub const fn legal(self) -> bool {
        self.l >= 2
            && self.l <= 31
            && self.n2 >= 2
            && self.n2 <= 6
            && self.turbo_khz() <= CORE_MAX_KHZ
            && self.bus_khz() <= BUS_MAX_KHZ
            && self.mem_khz() <= BUS_MAX_KHZ
    }

    /// [`Self::legal`] with the turbo multiplier allowed up to 2N 8 and the
    /// turbo PLL up to [`PAST_RATING_MAX_KHZ`] (C2's test past Intel's
    /// rating, the owner's go 2026-10-30): the bus and memory limits are
    /// kept, so only the core's turbo clock leaves the rating.
    #[must_use]
    pub const fn legal_past_rating(self) -> bool {
        self.l >= 2
            && self.l <= 31
            && self.n2 >= 2
            && self.n2 <= 8
            && self.turbo_khz() <= PAST_RATING_MAX_KHZ
            && self.bus_khz() <= BUS_MAX_KHZ
            && self.mem_khz() <= BUS_MAX_KHZ
    }

    /// Whether moving between `self` and `to` needs the core PLL relocked
    /// (`CLKCFG` F): any multiplier or memory-clock change. Turbo, half-turbo
    /// and nothing else changing switch without one.
    #[must_use]
    pub const fn needs_relock(self, to: Self) -> bool {
        self.l != to.l || self.n2 != to.n2 || self.a != to.a || self.b != to.b
    }

    /// `CCCR` with this point's multipliers, `old`'s other bits kept.
    #[must_use]
    pub const fn cccr(self, old: u32) -> u32 {
        (old & !(CCCR_A | CCCR_2N | CCCR_L))
            | if self.a { CCCR_A } else { 0 }
            | (self.n2 as u32) << 7
            | self.l as u32
    }

    /// `CLKCFG` for this point (without F).
    #[must_use]
    pub const fn clkcfg(self) -> u32 {
        (if self.turbo { clkcfg::T } else { 0 })
            | if self.half { clkcfg::HT } else { 0 }
            | if self.b { clkcfg::B } else { 0 }
    }

    /// The point running now, from `CCSR` (the multipliers in use), `CCCR`
    /// (A) and `CLKCFG`.
    #[must_use]
    pub const fn decode(cccr: u32, ccsr: u32, clkcfg: u32) -> Self {
        Self {
            l: (ccsr & CCCR_L) as u8,
            n2: ((ccsr & CCCR_2N) >> 7) as u8,
            a: cccr & CCCR_A != 0,
            b: clkcfg & clkcfg::B != 0,
            turbo: clkcfg & clkcfg::T != 0,
            half: clkcfg & clkcfg::HT != 0,
        }
    }

    /// The same point with turbo on or off.
    #[must_use]
    pub const fn with_turbo(self, on: bool) -> Self {
        Self { turbo: on, ..self }
    }

    /// The lowest core voltage the PLL's turbo frequency needs, in mV (the
    /// PXA27x electrical specification's table, as Linux's cpufreq carries
    /// it; ADR-0031 lists it as a chip fact to confirm against the
    /// datasheet). The PLL, not the mode, decides: turbo can be switched on
    /// at any moment without a voltage change.
    #[must_use]
    pub const fn min_mv(self) -> u32 {
        let f = self.turbo_khz();
        if f <= 104_000 {
            900
        } else if f <= 156_000 {
            1_000
        } else if f <= 208_000 {
            1_180
        } else if f <= 312_000 {
            1_250
        } else if f <= 416_000 {
            1_350
        } else if f <= 520_000 {
            1_450
        } else {
            1_550
        }
    }
}

const fn pt(l: u8, n2: u8, turbo: bool) -> Point {
    Point {
        l,
        n2,
        a: true,
        b: true,
        turbo,
        half: false,
    }
}

/// RetroPane's operating points, slowest first (ADR-0031). Every one keeps
/// the fast bus and the alternate memory clock: the bus and SDRAM run as
/// fast as the run frequency allows (208 MHz from 208 MHz up, so the 2700G
/// and SDRAM are never slowed by a faster core). 624 MHz is L 16 x 2N 6:
/// run 208 MHz, so turbo switches 208 <-> 624 with no PLL relock and the
/// bus stays at 208 MHz (L 24 x 2N 4 would drop the bus to 156 MHz).
pub const POINTS: [(&str, Point); 7] = [
    ("104", pt(8, 2, false)),
    ("156", pt(8, 3, true)),
    ("208", pt(16, 2, false)),
    ("312", pt(16, 3, true)),
    ("416", pt(16, 4, true)),
    ("520", pt(16, 5, true)),
    ("624", pt(16, 6, true)),
];

/// 624 MHz as the manual's table 3-7 lists it for the C5 stepping: `A` = 0,
/// so the memory clock is run / 2 = 104 MHz (SDRAM 104 MHz undivided, but
/// the static buses, the 2700G's among them, at half the clock). The 624 in
/// [`POINTS`] (`A` = 1, memory 208 MHz) is what WM runs on this unit and
/// Linux's table carries, but the manual doesn't list it: `rp-clocks`
/// measures both (memory-timing report, open question 4; ADR-0031 V4).
pub const P624_MANUAL: Point = Point {
    a: false,
    ..pt(16, 6, true)
};

/// SDRAM rows per bank from `MDCNFG` (the larger of partition pairs 0/1
/// and 2/3: DRAC bits 6-5 and 22-21, rows = 2^(11 + DRAC)). A pair counts
/// only if one of its partitions is enabled (DE bits 0-1, 16-17); with none
/// enabled (a configuration this can't read) the most rows the controller
/// supports, 16384: the shortest refresh interval, never late.
#[must_use]
pub const fn sdram_rows(mdcnfg: u32) -> u32 {
    let d0 = if mdcnfg & 0b11 != 0 {
        (mdcnfg >> 5) & 3
    } else {
        0
    };
    let d2 = if mdcnfg & 0b11 << 16 != 0 {
        (mdcnfg >> 21) & 3
    } else {
        0
    };
    if mdcnfg & (0b11 | 0b11 << 16) == 0 {
        return 1 << 14;
    }
    let d = if d0 > d2 { d0 } else { d2 };
    1 << (11 + d)
}

/// The refresh interval field (`MDREFR` DRI) that refreshes every row
/// within [`TREF_MS`] at memory clock `mem_khz`: a row every
/// `mem_khz x 64 / rows` memory clocks, DRI counting 32 of them (rounded
/// down, so never late). Derived from the SDRAM rule, not copied: Windows
/// Mobile's 48 at 208 MHz and 8192 rows is a little below this 49.
#[must_use]
pub const fn dri(mem_khz: u32, rows: u32) -> u32 {
    let interval = mem_khz * TREF_MS / rows;
    let d = interval.saturating_sub(31) / 32;
    if d > MDREFR_DRI { MDREFR_DRI } else { d }
}

/// `MDREFR` before and after a change to `to` (from `mdrefr` now): before,
/// the smaller of the two refresh intervals (the slower clock's: never late
/// during the change) and, if `to` halves the SDRAM clock, the dividers
/// already set; after, `to`'s interval and dividers.
#[must_use]
pub const fn mdrefr_around(mdrefr: u32, to: Point, rows: u32) -> (u32, u32) {
    let new = dri(to.mem_khz(), rows);
    let old = mdrefr & MDREFR_DRI;
    let pre_dri = if old < new { old } else { new };
    let mut pre = (mdrefr & !MDREFR_DRI) | pre_dri;
    let mut post = (mdrefr & !MDREFR_DRI) | new;
    if to.sdram_div2() {
        pre |= MDREFR_DB2;
        post |= MDREFR_DB2;
    } else {
        post &= !MDREFR_DB2;
    }
    (pre, post)
}

/// The clock registers as read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Raw {
    /// `CCCR`.
    pub cccr: u32,
    /// `CCSR`.
    pub ccsr: u32,
    /// `CLKCFG`.
    pub clkcfg: u32,
    /// `MDREFR`.
    pub mdrefr: u32,
    /// `MDCNFG`.
    pub mdcnfg: u32,
}

impl Raw {
    /// The point these registers describe.
    #[must_use]
    pub const fn point(self) -> Point {
        Point::decode(self.cccr, self.ccsr, self.clkcfg)
    }
}

/// The register writes of one operating-point change, worked out before
/// any is made (a pure function, so host tests can hold every change to the
/// rules; `change` only executes it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Whether the core PLL relocks (`CCCR` then `CLKCFG` with F). Without
    /// one only `clkcfg` is written: the memory clock can't change.
    pub relock: bool,
    /// `CCCR` to write first (relock only).
    pub cccr: u32,
    /// `MDREFR` just before the relock.
    pub pre: u32,
    /// `CLKCFG` to write (F included for a relock).
    pub clkcfg: u32,
    /// `MDREFR` just after the relock.
    pub first: u32,
    /// `MDREFR` 3 us later, if it differs from `first` (clearing the SDRAM
    /// divider: the longest interval goes in first, WM's stricter order;
    /// Linux writes the final value at once: a finding to work).
    pub settle: Option<u32>,
}

impl Plan {
    /// `MDREFR` once the change is complete.
    #[must_use]
    pub const fn final_mdrefr(self) -> u32 {
        match self.settle {
            Some(v) => v,
            None => self.first,
        }
    }
}

/// The plan for moving from `now` to `to`, or `None` for an illegal point.
/// Any memory-clock change is a relock, and every relock rewrites the
/// refresh interval for the new memory clock (memory-timing report: a stale
/// interval at a lower memory clock corrupts SDRAM silently).
#[must_use]
pub const fn plan(now: Raw, to: Point) -> Option<Plan> {
    if !to.legal() {
        return None;
    }
    plan_any(now, to)
}

/// [`plan`] for a point past Intel's core rating
/// ([`Point::legal_past_rating`]): test images only (ADR-0031).
#[must_use]
pub const fn plan_past_rating(now: Raw, to: Point) -> Option<Plan> {
    if !to.legal_past_rating() {
        return None;
    }
    plan_any(now, to)
}

/// [`plan`] without the legality check (its callers make it).
const fn plan_any(now: Raw, to: Point) -> Option<Plan> {
    if !now.point().needs_relock(to) {
        return Some(Plan {
            relock: false,
            cccr: now.cccr,
            pre: now.mdrefr,
            clkcfg: to.clkcfg(),
            first: now.mdrefr,
            settle: None,
        });
    }
    let (pre, post) = mdrefr_around(now.mdrefr, to, sdram_rows(now.mdcnfg));
    let undivide = pre & MDREFR_DB2 != 0 && post & MDREFR_DB2 == 0;
    Some(Plan {
        relock: true,
        cccr: to.cccr(now.cccr),
        pre,
        clkcfg: to.clkcfg() | clkcfg::F,
        first: if undivide { post | MDREFR_DRI } else { post },
        settle: if undivide { Some(post) } else { None },
    })
}

/// Cycles counted by the performance monitor's cycle counter over `ticks`
/// OS-timer ticks (3.25 MHz), as kHz: the core clock actually running.
#[must_use]
pub const fn khz_from_cycles(cycles: u32, ticks: u32) -> u32 {
    if ticks == 0 {
        return 0;
    }
    (cycles as u64 * 3_250 / ticks as u64) as u32
}

#[cfg(all(target_arch = "arm", target_os = "none"))]
mod target {
    use super::{CCCR, CCSR, MDCNFG, MDREFR, Plan, Point, Raw, clkcfg, plan};
    use crate::devmap::va;

    fn rd(phys: usize) -> u32 {
        // SAFETY: clock-manager and memory-controller configuration
        // registers, mapped as device memory by both maps; reads have no
        // side effects.
        unsafe { (va(phys) as *const u32).read_volatile() }
    }

    /// The power manager's configuration registers (no FIFOs, nothing
    /// cleared by reading): how the voltage sequencer, power I2C, sleep and
    /// wake-up were left, for the record.
    pub const POWER_REGS: [(&str, usize); 16] = [
        ("CKEN", 0x4130_0004),
        ("OSCC", 0x4130_0008),
        ("PSSR", 0x40F0_0004),
        ("PWER", 0x40F0_000C),
        ("PRER", 0x40F0_0010),
        ("PFER", 0x40F0_0014),
        ("PCFR", 0x40F0_001C),
        ("PSLR", 0x40F0_0034),
        ("PSTR", 0x40F0_0038),
        ("PVCR", 0x40F0_0040),
        ("PUCR", 0x40F0_004C),
        ("PKWR", 0x40F0_0050),
        ("PCMD0", 0x40F0_0080),
        ("PIBMR", 0x40F0_0180),
        ("PICR", 0x40F0_0190),
        ("PISR", 0x40F0_0198),
    ];

    /// Reads [`POWER_REGS`].
    #[must_use]
    pub fn power_snapshot() -> [u32; 16] {
        POWER_REGS.map(|(_, a)| rd(a))
    }

    /// `CLKCFG` (CP14 c6).
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
    pub fn read_clkcfg() -> u32 {
        let v: u32;
        // SAFETY: reading CLKCFG has no side effects.
        unsafe {
            core::arch::asm!("mrc p14, 0, {}, c6, c0, 0", out(reg) v, options(nostack, nomem))
        };
        v
    }

    /// Reads the clock and SDRAM refresh registers.
    #[must_use]
    pub fn read() -> Raw {
        Raw {
            cccr: rd(CCCR),
            ccsr: rd(CCSR),
            clkcfg: read_clkcfg(),
            mdrefr: rd(MDREFR),
            mdcnfg: rd(MDCNFG),
        }
    }

    /// Moves to `to`. A turbo/half-turbo-only change is one `CLKCFG` write
    /// (no relock, the memory clock untouched); anything else writes `CCCR`,
    /// then, from one I-cache line so nothing is fetched from SDRAM while
    /// its clock stops, the before-`MDREFR`, `CLKCFG` with F (the PLL
    /// relocks; SDRAM self-refreshes meanwhile), the after-`MDREFR`.
    /// Interrupts are masked throughout. Returns false (doing nothing) for
    /// an illegal point.
    ///
    /// The caller answers for the core voltage: `to.min_mv()` must be met.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
    pub fn change(to: Point) -> bool {
        change_by(to, plan)
    }

    /// [`change`] to a point past Intel's core rating
    /// ([`Point::legal_past_rating`]: the turbo multiplier up to 2N 8, the
    /// bus and memory clocks within their limits). **Test images only**
    /// (`rp-clocks`' C2b, the owner's go 2026-10-30): nothing ships past
    /// 624 MHz (ADR-0031). The PLL may not take such a multiplier: `CCSR`
    /// afterwards says what it runs.
    #[cfg(feature = "past-rating")]
    pub fn change_past_rating(to: Point) -> bool {
        change_by(to, super::plan_past_rating)
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
    fn change_by(to: Point, plan: fn(Raw, Point) -> Option<Plan>) -> bool {
        let was = crate::rt::irqs_enabled_now();
        crate::rt::irq_disable();
        let Some(p) = plan(read(), to) else {
            if was {
                crate::rt::irq_enable();
            }
            return false;
        };
        if p.relock {
            let mdrefr = va(MDREFR);
            // SAFETY: CCCR only takes effect at the next F write.
            unsafe { (va(CCCR) as *mut u32).write_volatile(p.cccr) };
            // SAFETY: MDREFR's refresh interval and SDRAM dividers set as
            // the memory controller requires around a memory-clock change
            // (`plan`); the CLKCFG write relocks the core PLL with CCCR's
            // (legal) multipliers. The branch dance loads the 32-byte
            // aligned line holding the three writes into the I-cache before
            // the first runs (no fetch from SDRAM during the change). The
            // MDREFR read beforehand puts its translation in the TLB.
            unsafe {
                core::arch::asm!(
                    "ldr {t}, [{m}]",
                    "b 2f",
                    ".p2align 5",
                    "1: str {pre}, [{m}]",
                    "mcr p14, 0, {cfg}, c6, c0, 0",
                    "str {post}, [{m}]",
                    "b 3f",
                    "2: b 1b",
                    "3: nop",
                    t = out(reg) _,
                    m = in(reg) mdrefr,
                    pre = in(reg) p.pre,
                    cfg = in(reg) p.clkcfg,
                    post = in(reg) p.first,
                    options(nostack),
                );
            }
            if let Some(v) = p.settle {
                crate::rt::delay_us(3);
                // SAFETY: the final MDREFR for the new memory clock.
                unsafe { (mdrefr as *mut u32).write_volatile(v) };
            }
        } else {
            set_clkcfg(p.clkcfg);
        }
        if was {
            crate::rt::irq_enable();
        }
        true
    }

    /// Puts the clocks back exactly as `raw` found them: its point, then its
    /// own refresh interval and SDRAM dividers (the point's are derived;
    /// whoever ran before may have chosen others).
    pub fn restore(raw: Raw) -> bool {
        // The multipliers in use (CCSR), not the ones last asked for: the
        // two differ only between a CCCR write and its frequency change.
        let p = raw.point();
        if !change(p) {
            return false;
        }
        // CCCR names the point running afterwards, as a relock leaves it:
        // without one, a request saved in CCCR mid-change would stay armed
        // for the next F write or wake from sleep (quality audit C42, the
        // owner's option b). `p` is legal: `change` took it.
        ask(p);
        let keep = super::MDREFR_DRI | super::MDREFR_DB2;
        let now = rd(MDREFR);
        // SAFETY: the memory clock is `raw`'s again, so its refresh interval
        // and dividers are the ones that ran at it.
        unsafe {
            (va(MDREFR) as *mut u32).write_volatile((now & !keep) | (raw.mdrefr & keep));
        }
        true
    }

    static mut SAVED: Option<Raw> = None;

    /// Reads the clocks and keeps them for [`restore_saved`] (a fault path
    /// puts the clocks back before reporting).
    pub fn save() -> Raw {
        let r = read();
        // SAFETY: single core; written once by the image's main path before
        // any change, read by its fault path.
        unsafe { core::ptr::addr_of_mut!(SAVED).write(Some(r)) };
        r
    }

    /// Puts back the clocks [`save`] kept, if any.
    pub fn restore_saved() -> bool {
        // SAFETY: as in `save`.
        match unsafe { core::ptr::addr_of!(SAVED).read() } {
            Some(r) => restore(r),
            None => false,
        }
    }

    /// Writes `CCCR` with `p`'s multipliers (its other bits kept) and
    /// nothing else: no frequency change, so the core keeps running at
    /// `CCSR`'s multipliers until the next `CLKCFG` F write (or, Intel's
    /// manual says, a wake from sleep). `CCCR` and `CCSR` then disagree, as
    /// they do between the two writes of a change: `rp-clocks` run F checks
    /// the fault path's put-back in that state (quality audit C31). Returns
    /// false, writing nothing, for an illegal point.
    pub fn ask(p: Point) -> bool {
        if !p.legal() {
            return false;
        }
        let v = p.cccr(rd(CCCR));
        // SAFETY: CCCR only takes effect at the next F write; `p` is legal,
        // so whoever applies it gets a point the rules allow.
        unsafe { (va(CCCR) as *mut u32).write_volatile(v) };
        true
    }

    /// Writes `CLKCFG` (turbo / half-turbo switch, no F unless given) from
    /// one I-cache line.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
    pub fn set_clkcfg(v: u32) {
        // SAFETY: a CLKCFG write with the multipliers unchanged switches the
        // core between run and turbo; the line is cached first as above.
        unsafe {
            core::arch::asm!(
                "b 2f",
                ".p2align 5",
                "1: mcr p14, 0, {v}, c6, c0, 0",
                "b 3f",
                "2: b 1b",
                "3: nop",
                v = in(reg) v,
                options(nostack),
            );
        }
    }

    /// Turbo on or off at the current multipliers (no relock).
    pub fn set_turbo(on: bool) {
        let c = read_clkcfg() & !(clkcfg::T | clkcfg::HT | clkcfg::F);
        set_clkcfg(if on { c | clkcfg::T } else { c });
    }

    /// Starts the performance monitor's cycle counter from zero (CP14 PMNC:
    /// enable, reset the cycle counter, count every cycle). Other counters
    /// and interrupts stay off.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
    pub fn cycles_start() {
        // SAFETY: PMNC (CP14 c0, c1) only controls the performance counters.
        unsafe {
            core::arch::asm!("mcr p14, 0, {v}, c0, c1, 0", v = in(reg) 0b101u32, options(nostack))
        };
    }

    /// The cycle counter (CP14 c1, c1): core clocks since
    /// [`cycles_start`]; it stops while the core is idle.
    #[must_use]
    #[inline(always)]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
    pub fn cycles() -> u32 {
        let v: u32;
        // SAFETY: reading CCNT has no side effects.
        unsafe {
            core::arch::asm!("mrc p14, 0, {}, c1, c1, 0", out(reg) v, options(nostack, nomem))
        };
        v
    }

    /// The core clock really running, kHz: cycles counted over `ms`
    /// milliseconds of the OS timer (busy, so the core never idles).
    #[must_use]
    pub fn measure_core_khz(ms: u32) -> u32 {
        let ticks = ms.saturating_mul(crate::rt::TICK_HZ / 1000);
        cycles_start();
        let t0 = crate::rt::ticks();
        let c0 = cycles();
        while crate::rt::ticks().wrapping_sub(t0) < ticks {}
        let c1 = cycles();
        let t1 = crate::rt::ticks();
        super::khz_from_cycles(c1.wrapping_sub(c0), t1.wrapping_sub(t0))
    }
}

#[cfg(all(target_arch = "arm", target_os = "none"))]
pub use target::*;

#[cfg(test)]
mod tests {
    use super::*;

    // Windows Mobile's state on the owner's X51v (docs/hw/probe-2026-10-11.md).
    const WM_CCCR: u32 = 0x0200_0290;
    const WM_CCSR: u32 = 0x3000_0290;
    const WM_CLKCFG: u32 = 0xB;
    const WM_MDREFR: u32 = 0x201E_E030;
    const WM_MDCNFG: u32 = 0x0ACB_2BCC;

    /// A CCCR written but not yet applied (L and 2N differ from CCSR's):
    /// the point is the one running, from CCSR, with CCCR's A bit (CCSR
    /// doesn't carry it). `restore` puts back `Raw::point`.
    #[test]
    fn the_point_is_the_running_one_when_cccr_and_ccsr_disagree() {
        // Asked for L=16 2N=6 (CCCR), running L=8 2N=2 (CCSR).
        let raw = Raw {
            cccr: CCCR_A | 6 << 7 | 16,
            ccsr: 3 << 28 | 2 << 7 | 8,
            clkcfg: WM_CLKCFG,
            ..Raw::default()
        };
        let p = raw.point();
        assert_eq!((p.l, p.n2, p.a), (8, 2, true));
        assert_eq!(p, Point::decode(raw.cccr, raw.ccsr, raw.clkcfg));
        assert_ne!(p, Point::decode(raw.cccr, raw.cccr, raw.clkcfg));
    }

    #[test]
    fn decodes_windows_mobiles_state() {
        let p = Point::decode(WM_CCCR, WM_CCSR, WM_CLKCFG);
        assert_eq!(p, POINTS[5].1);
        assert_eq!(p.run_khz(), 208_000);
        assert_eq!(p.core_khz(), 520_000);
        assert_eq!(p.bus_khz(), 208_000);
        assert_eq!(p.mem_khz(), 208_000);
        assert_eq!(p.sdclk_khz(), 104_000);
        assert_eq!(p.cccr(0), WM_CCCR);
        assert_eq!(p.clkcfg(), WM_CLKCFG & !clkcfg::F);
    }

    #[test]
    fn points_are_legal_and_ordered() {
        let mut last = 0;
        for (name, p) in POINTS {
            assert!(p.legal(), "{name}");
            assert!(p.core_khz() > last, "{name}");
            assert_eq!(name.parse::<u32>().unwrap() * 1000, p.core_khz());
            assert!(p.sdclk_khz() <= SDCLK_MAX_KHZ);
            last = p.core_khz();
        }
        // 624 keeps the full-speed bus and memory clock.
        let top = POINTS[6].1;
        assert_eq!(
            (top.bus_khz(), top.mem_khz(), top.min_mv()),
            (208_000, 208_000, 1_550)
        );
        // Turbo off at the top point: 208 MHz, no relock.
        assert_eq!(top.with_turbo(false).core_khz(), 208_000);
        assert!(!top.needs_relock(top.with_turbo(false)));
        assert!(top.needs_relock(POINTS[0].1));
    }

    #[test]
    fn refuses_illegal_points() {
        assert!(!pt(24, 6, true).legal()); // 936 MHz
        assert!(
            !Point {
                b: true,
                ..pt(24, 2, false)
            }
            .legal()
        ); // bus 312
        assert!(!pt(1, 2, false).legal());
        assert!(!pt(8, 7, true).legal());
    }

    #[test]
    fn past_the_rating_only_the_core_leaves_it() {
        // C2's test (the owner's go 2026-10-30): L 16 x 2N 7 and 8 (728 and
        // 832 MHz) with the run, bus and memory clocks as at 624.
        for (n2, khz) in [(7, 728_000), (8, 832_000)] {
            let p = pt(16, n2, true);
            assert!(!p.legal(), "{n2}: past Intel's rating");
            assert!(p.legal_past_rating(), "{n2}");
            assert_eq!(p.core_khz(), khz);
            assert_eq!(
                (p.run_khz(), p.bus_khz(), p.mem_khz(), p.sdclk_khz()),
                (208_000, 208_000, 208_000, 104_000),
                "{n2}: the memory and bus as Dell set them"
            );
            // From 624 the change relocks the PLL; the SDRAM dividers stay
            // and the refresh interval is the rule's for the same memory
            // clock (49 where Windows Mobile's is 48), nothing settled after.
            let now = raw(POINTS[6].1, WM_MDREFR);
            let pl = plan_past_rating(now, p).unwrap();
            assert!(pl.relock);
            assert_eq!(pl.cccr & 0xFFF, 16 | u32::from(n2) << 7);
            let keep = !MDREFR_DRI;
            assert_eq!(pl.first & keep, WM_MDREFR & keep);
            assert_eq!(pl.first & MDREFR_DRI, dri(208_000, sdram_rows(WM_MDCNFG)));
            assert_eq!(pl.settle, None);
            assert_eq!(plan(now, p), None, "the shipped path refuses it");
        }
        // Still refused: 2N 9 and up, a turbo past 832, a bus or memory past
        // its limit.
        assert!(!pt(16, 9, true).legal_past_rating());
        assert!(!pt(17, 8, true).legal_past_rating()); // 884 MHz
        assert!(
            !Point {
                b: true,
                ..pt(24, 2, false)
            }
            .legal_past_rating()
        ); // bus 312
    }

    #[test]
    fn refresh_interval_from_the_rule() {
        let rows = sdram_rows(WM_MDCNFG);
        assert_eq!(rows, 8192);
        assert_eq!(sdram_rows(0), 16_384); // unreadable: shortest interval
        assert_eq!(dri(208_000, rows), 49); // WM: 48
        assert_eq!(dri(104_000, rows), 24);
        // Never later than 64 ms over all rows: DRI x 32 + 31 clocks a row.
        for khz in [13_000, 104_000, 156_000, 208_000] {
            let d = dri(khz, rows);
            assert!(u64::from(d * 32 + 31) * u64::from(rows) <= u64::from(khz) * 64);
        }
    }

    #[test]
    fn mdrefr_slows_refresh_only_after_and_halves_before() {
        let rows = 8192;
        // 520 -> 104: memory 208 -> 104: the shorter interval (24) before,
        // dividers cleared only after.
        let (pre, post) = mdrefr_around(WM_MDREFR, POINTS[0].1, rows);
        assert_eq!(pre & MDREFR_DRI, 24);
        assert_eq!(pre & MDREFR_DB2, WM_MDREFR & MDREFR_DB2);
        assert_eq!((post & MDREFR_DRI, post & MDREFR_DB2), (24, 0));
        // 104 -> 624: the old interval kept before, dividers set before.
        let (pre, post) = mdrefr_around(post, POINTS[6].1, rows);
        assert_eq!((pre & MDREFR_DRI, pre & MDREFR_DB2), (24, MDREFR_DB2));
        assert_eq!(post & MDREFR_DRI, 49);
        // Other bits untouched.
        assert_eq!(
            post & !(MDREFR_DRI | MDREFR_DB2),
            WM_MDREFR & !(MDREFR_DRI | MDREFR_DB2)
        );
    }

    fn raw(p: Point, mdrefr: u32) -> Raw {
        let c = p.cccr(WM_CCCR);
        Raw {
            cccr: c,
            ccsr: c | 3 << 28,
            clkcfg: p.clkcfg(),
            mdrefr,
            mdcnfg: WM_MDCNFG,
        }
    }

    #[test]
    fn no_change_can_skip_the_refresh_rewrite() {
        let all: [Point; 8] = [
            POINTS[0].1,
            POINTS[1].1,
            POINTS[2].1,
            POINTS[3].1,
            POINTS[4].1,
            POINTS[5].1,
            POINTS[6].1,
            P624_MANUAL,
        ];
        let rows = sdram_rows(WM_MDCNFG);
        for from in all {
            for to in all {
                for turbo in [false, true] {
                    let to = to.with_turbo(turbo);
                    let mdrefr = mdrefr_around(WM_MDREFR, from, rows).1;
                    let p = plan(raw(from, mdrefr), to).unwrap();
                    let fin = p.final_mdrefr();
                    // The interval is always right for the new memory clock.
                    assert_eq!(
                        fin & MDREFR_DRI,
                        dri(to.mem_khz(), rows),
                        "{from:?} -> {to:?}"
                    );
                    // Never late during the change either.
                    assert!(p.pre & MDREFR_DRI <= dri(from.mem_khz(), rows));
                    assert!(p.pre & MDREFR_DRI <= dri(to.mem_khz(), rows));
                    // A memory-clock change is always a relock.
                    if from.mem_khz() != to.mem_khz() {
                        assert!(p.relock, "{from:?} -> {to:?}");
                    }
                    // The SDRAM divider matches the new memory clock.
                    assert_eq!(fin & MDREFR_DB2 != 0, to.sdram_div2());
                    if p.relock {
                        assert_ne!(p.clkcfg & clkcfg::F, 0);
                        assert_eq!(Point::decode(p.cccr, p.cccr, p.clkcfg), to);
                    } else {
                        assert_eq!(fin, mdrefr);
                        assert_eq!(p.clkcfg & clkcfg::F, 0);
                    }
                }
            }
        }
    }

    #[test]
    fn manual_624_halves_the_memory_clock_not_the_sdram() {
        let m = P624_MANUAL;
        assert!(m.legal());
        assert_eq!(
            (m.core_khz(), m.bus_khz(), m.mem_khz(), m.sdclk_khz()),
            (624_000, 208_000, 104_000, 104_000)
        );
        assert!(plan(raw(POINTS[6].1, WM_MDREFR), m).unwrap().relock);
        assert_eq!(plan(raw(POINTS[0].1, WM_MDREFR), pt(24, 6, true)), None);
    }

    #[test]
    fn cycle_counter_to_khz() {
        assert_eq!(khz_from_cycles(520_000 * 10, 32_500), 520_000);
        assert_eq!(khz_from_cycles(1, 0), 0);
    }
}
