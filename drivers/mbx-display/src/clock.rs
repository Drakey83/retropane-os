//! The MBX's clock enable (system block `MBXCLK`, `0x054`) and a gate that
//! keeps the 3D core's clock on only while it has work (ADR-0026 D8).
//!
//! `MBXCLK` bits 1-0 are the enable (0 none, 1 2D only, 2 2D and 3D; Linux
//! mbxfb's `MBXCLK_EN`), bits 3-2 the divider. Windows Mobile idles it at
//! `0x5` (2D only) and Dell's driver writes the 3D enable before 3D work;
//! the 3D core drew only after it was set (`rp-mbx3d` version 3).
//!
//! Measured (ADR-0026 T9, `rp-mbx3d-api` run 4, 2026-10-15): the 3D core
//! keeps its state (the parameter memory manager's set-up, its registers)
//! across a clock off and on, so a caller that switches the clock on
//! ([`Gate::work`] returning true) needs to redo nothing.
//!
//! **The MBX clocks off (enable 0) is fatal to touch** (quality audit C43,
//! `rp-gpubench` run FS, X51v 2026-10-22): queued 2D work pauses and
//! finishes once the clock is back, but one read of an MBX register with
//! it off hung the CPU's bus until the watchdog reset the device. So the
//! clocks go off only through [`stop`], whose [`Stopped`] holds the only
//! handle to the register bus until it puts the clock back: code that
//! would touch a register meanwhile doesn't compile. Nothing else writes
//! `MBXCLK` but this module (`cargo xtask lint`: [`restore`] for a value
//! kept from before).
//!
//! The system clock (the core PLL) is a set-up option: [`Speed`] and
//! [`set_speed`]. It clocks both MBX cores (through `MBXCLK`'s divider)
//! and the 2700G's local SDRAM, so a speed carries the memory timing for
//! that clock. The default is Windows Mobile's [`Speed::WM`].

use rp_hal_api::RegisterBus;

/// The clock register's offset in the 2700G register window.
pub const MBXCLK: usize = 0x054;

/// The core PLL: `13 MHz x M / (N x 2^P)`, M bits 12-7, N 6-4, P 3-1,
/// enable 0 (Linux mbxfb's names).
pub const COREPLL: usize = 0x038;
/// The system clock's source: 0 the 13 MHz reference, 2 the core PLL.
pub const SYSCLKSRC: usize = 0x02C;
/// The PLLs' lock bits; bit 2 the core PLL's.
pub const PLLSTAT: usize = 0x040;
/// `PLLSTAT`'s core-lock bit.
pub const CORE_LOCKED: u32 = 1 << 2;
/// The local SDRAM's type (CAS latency bits 12-10, bank size 9-8, row size
/// 7-4, column size 3-0).
pub const LMTYPE: usize = 0x1014;
/// The local SDRAM's timing in system-clock cycles (tRAS bits 19-16, tRP
/// 15-12, tRCD 11-8, tRC 7-4, tDPL 3-0).
pub const LMTIM: usize = 0x1018;
/// The local SDRAM's refresh interval in system-clock cycles.
pub const LMREFRESH: usize = 0x101C;

/// A system clock and the local-memory timing that goes with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Speed {
    /// `COREPLL`.
    pub pll: u32,
    /// `LMTYPE`.
    pub lmtype: u32,
    /// `LMTIM`.
    pub lmtim: u32,
    /// `LMREFRESH`.
    pub lmrefresh: u32,
}

impl Speed {
    /// Windows Mobile's, the default: 13 x 23 / 3 = 99.67 MHz, CL2, tRAS 7,
    /// tRP 3, tRCD 3, tRC 9, tDPL 2, refresh every 1557 cycles (15.6 us).
    /// Dell's SDRAM table for the X51v rates the memory to 100 MHz, so this
    /// is its rated maximum (`docs/hw/2700g.md`).
    pub const WM: Self = Self {
        pll: 0xBB1,
        lmtype: 0xAC8,
        lmtim: 0x73392,
        lmrefresh: 0x615,
    };

    /// The owner's option A (2026-10-15): 13 x 23 / 4 = 74.75 MHz, for
    /// `MBXCLK` divide by 1 (the cores at 74.75 MHz, under the 75 MHz the
    /// cores are run at most; 50% faster than WM's 49.8), the memory 25%
    /// slower and inside its rating. Its timing from the SDRAM's ratings
    /// (tRC 85, tRP and tRCD 25, tRAS 48 ns; CL2 below 100 MHz) at 13.4 ns
    /// cycles: tRC 7, tRP 2, tRCD 2, tRAS 7 - 2 + 1 = 6, tDPL 2; refresh
    /// every 1167 cycles (15.6 us, 64 ms over 4096 rows). Nothing runs the
    /// memory past its 100 MHz rating.
    pub const A75: Self = Self {
        pll: 23 << 7 | 4 << 4 | 1,
        lmtype: 0xAC8,
        lmtim: 0x62272,
        lmrefresh: 0x48F,
    };

    /// The core PLL value `pll` with the memory timing Dell's routine gives
    /// for its clock (docs/hw/2700g.md, "The 2700G's local memory sets the
    /// clock limit"): each timing the SDRAM's rating (tRC 85, tRP and tRCD
    /// 25, tRAS 48 ns) in whole cycles rounded up, tRAS raised to
    /// tRC - tRP + 1, tDPL 2; CL2 up to 100 MHz, CL3 above (`LMTYPE`
    /// `0xAC8` / `0xEC8`); a refresh every 15.6 us (64 ms over 4096 rows).
    /// It gives Windows Mobile's values at 99.67 MHz and Dell's own at
    /// 133.25 MHz (`0x944C2`, CL3). Past 100 MHz the memory runs over its
    /// rating: for the clock experiments (docs/research/2700g-full-potential.md
    /// P2), never a default.
    #[must_use]
    pub const fn rated(pll: u32) -> Self {
        let khz = Self {
            pll,
            lmtype: 0,
            lmtim: 0,
            lmrefresh: 0,
        }
        .khz();
        let (trc, trp, trcd) = (cycles(85, khz), cycles(25, khz), cycles(25, khz));
        let tras = if cycles(48, khz) > trc - trp + 1 {
            cycles(48, khz)
        } else {
            trc - trp + 1
        };
        Self {
            pll,
            lmtype: if khz > 100_000 { 0xEC8 } else { 0xAC8 },
            lmtim: tras << 16 | trp << 12 | trcd << 8 | trc << 4 | 2,
            lmrefresh: khz * 15_625 / 1_000_000,
        }
    }

    /// The core PLL value `pll` as ADR-0035 sets it: [`Speed::rated`]'s
    /// timings with CL2 at every clock (ADR-0035's "CL2 always": Dell's CL3
    /// above 100 MHz drew no 3D picture, run G1, and `rp-glesfeat` G3's
    /// first run took it by mistake and rendered nothing at 133.25 MHz).
    /// What a clock step for 3D work uses; `rated` stays for experiments
    /// that compare the two.
    #[must_use]
    pub const fn ladder(pll: u32) -> Self {
        let mut s = Self::rated(pll);
        s.lmtype = Self::WM.lmtype;
        s
    }

    /// The system clock in kHz (by mbxfb's formula).
    #[must_use]
    pub const fn khz(self) -> u32 {
        let (m, n, p) = (
            (self.pll >> 7) & 63,
            (self.pll >> 4) & 7,
            (self.pll >> 1) & 7,
        );
        if n == 0 { 0 } else { 13_000 * m / (n << p) }
    }
}

/// `ns` nanoseconds in whole cycles of a `khz` clock, rounded up.
const fn cycles(ns: u32, khz: u32) -> u32 {
    (ns * khz).div_ceil(1_000_000)
}

/// The poll interval and limit while the core PLL locks (Dell's driver
/// waits up to 50 ms).
const LOCK_POLL_US: u32 = 2;
const LOCK_POLLS: u32 = 25_000;
/// How long a PLL write may take to show as "not locked" (`rp-gpubench`
/// run G3: `PLLSTAT` read the old "locked" once right after the write, then
/// dropped, and locked again 16 us later; a write that keeps the frequency
/// may never drop).
const DROP_POLLS: u32 = 50;
/// A wait after the lock before the system clock goes back on the PLL.
const LOCK_SETTLE_US: u32 = 2_000;
/// A wait after each memory-controller write, as Linux mbxfb's set-up
/// waits 1 ms after every register write. Run G3 found writes to `LMTYPE`
/// and `LMREFRESH` lost when written back to back on the reference clock
/// (read back unchanged).
const WRITE_SETTLE_US: u32 = 1_000;
/// Tries per register write before it's given up as unwritten.
const WRITE_TRIES: u32 = 3;

/// What [`set_speed`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// The core PLL's lock time, microseconds.
    pub lock_us: u32,
    /// Writes that read back wrong and were written again.
    pub rewrites: u32,
    /// Registers still wrong after [`WRITE_TRIES`]: the speed isn't
    /// what was asked (a caller must treat it as a failure).
    pub unwritten: u32,
}

/// Sets the system clock and the memory timing to `speed`, as Dell's driver
/// changes the PLL (`0x0211faf0`): the system clock on the 13 MHz
/// reference, the memory timing written, each write read back (and
/// written again until it holds), the PLL written and its lock awaited
/// (first its drop, then the lock), a settle, the system clock back on the
/// PLL. Nothing else may use the 2700G meanwhile (a few milliseconds; the
/// display reads its picture slower and may show a flicker).
///
/// Returns what happened, or `None` if the PLL did not lock in 50 ms; then
/// [`Speed::WM`] is put back (the PLL, awaited, and the timing).
pub fn set_speed<B: RegisterBus>(
    bus: &mut B,
    speed: Speed,
    mut delay_us: impl FnMut(u32),
) -> Option<Applied> {
    bus.write32(SYSCLKSRC, 0);
    delay_us(250);
    let applied = write_speed(bus, speed, &mut delay_us);
    if applied.is_none() {
        let _ = write_speed(bus, Speed::WM, &mut delay_us);
    }
    delay_us(LOCK_SETTLE_US);
    bus.write32(SYSCLKSRC, 2);
    delay_us(250);
    applied
}

/// Writes `value` to `reg`, waits, reads it back; again until it holds.
fn write_checked<B: RegisterBus>(
    bus: &mut B,
    reg: usize,
    value: u32,
    delay_us: &mut impl FnMut(u32),
    a: &mut Applied,
) {
    for t in 0..WRITE_TRIES {
        bus.write32(reg, value);
        delay_us(WRITE_SETTLE_US);
        if bus.read32(reg) == value {
            a.rewrites += t;
            return;
        }
    }
    a.rewrites += WRITE_TRIES - 1;
    a.unwritten += 1;
}

/// The timing written and checked, the PLL written and its lock awaited
/// (on the reference clock): what happened, or `None` if it never locked.
fn write_speed<B: RegisterBus>(
    bus: &mut B,
    s: Speed,
    delay_us: &mut impl FnMut(u32),
) -> Option<Applied> {
    let mut a = Applied::default();
    write_checked(bus, LMTIM, s.lmtim, delay_us, &mut a);
    write_checked(bus, LMREFRESH, s.lmrefresh, delay_us, &mut a);
    write_checked(bus, LMTYPE, s.lmtype, delay_us, &mut a);
    let mut polls = 0;
    for _ in 0..WRITE_TRIES {
        bus.write32(COREPLL, s.pll);
        // The drop (or nothing, for an unchanged frequency), then the lock.
        for _ in 0..DROP_POLLS {
            if bus.read32(PLLSTAT) & CORE_LOCKED == 0 {
                break;
            }
            delay_us(LOCK_POLL_US);
        }
        loop {
            if bus.read32(PLLSTAT) & CORE_LOCKED != 0 {
                break;
            }
            polls += 1;
            if polls >= LOCK_POLLS {
                return None;
            }
            delay_us(LOCK_POLL_US);
        }
        if bus.read32(COREPLL) == s.pll {
            a.lock_us = polls * LOCK_POLL_US;
            return Some(a);
        }
        a.rewrites += 1;
    }
    a.unwritten += 1;
    a.lock_us = polls * LOCK_POLL_US;
    Some(a)
}

impl Speed {
    /// The clock and memory registers as they read now (what the 2700G
    /// holds, not what was written).
    pub fn now<B: RegisterBus>(bus: &mut B) -> Self {
        Self {
            pll: bus.read32(COREPLL),
            lmtype: bus.read32(LMTYPE),
            lmtim: bus.read32(LMTIM),
            lmrefresh: bus.read32(LMREFRESH),
        }
    }
}

/// The CPU's bus timing to the 2700G: CS3's VLIO fields (RDF, RDN, RRR),
/// written by the image (`rp_hal_pxa27x::diag::set_cs3_vlio`: the PXA27x's
/// memory controller is not this driver's).
pub type Cs3 = (u32, u32, u32);
/// CS3 as Windows Mobile leaves it (Dell's tier for 99 MHz and up).
pub const CS3_WM: Cs3 = (8, 4, 2);
/// Dell's tier for 25-99 MHz, the long one: safe at any clock, so it is
/// set while the clock moves.
pub const CS3_LONG: Cs3 = (13, 9, 7);

/// A rung of ADR-0035's ladder: the system clock with its memory timing,
/// and the cores' divider (1-4, `MBXCLK` bits 3-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rung {
    /// The system clock and memory timing.
    pub speed: Speed,
    /// The cores' divider.
    pub div: u32,
}

impl Rung {
    /// Windows Mobile's: 99.67 MHz, the cores at divide 2 (49.83 MHz).
    pub const WM: Self = Self {
        speed: Speed::WM,
        div: 2,
    };

    /// ADR-0035's top rung for 16-bit targets (the owner's ruling
    /// 2026-10-29): 13 x 23 / 2 = 149.50 MHz, the cores at divide 2 (74.75
    /// MHz), CL2 ([`Speed::ladder`]); `rp-gpubench`'s `gc2_step(23, 2, 0, 2)`.
    pub const TOP: Self = Self {
        speed: Speed::ladder(23 << 7 | 2 << 4 | 1),
        div: 2,
    };

    /// The CS3 tier for the rung's system clock (Dell's: the long one below
    /// 99 MHz).
    #[must_use]
    pub const fn cs3(self) -> Cs3 {
        if self.speed.khz() < 99_000 {
            CS3_LONG
        } else {
            CS3_WM
        }
    }
}

/// A clock step's first half: the long bus timing (safe at any clock),
/// the cores at divide 2, then the PLL and its memory timing by
/// [`set_speed`] (skipped when they already read as `speed`, so a
/// divider-only step changes nothing else). `None` if the PLL didn't lock
/// (Windows Mobile's speed is then back); a caller treats `unwritten` as a
/// failure too (ADR-0035 rule 4).
pub fn begin<B: RegisterBus>(
    bus: &mut B,
    speed: Speed,
    mut delay_us: impl FnMut(u32),
    mut set_cs3: impl FnMut(Cs3),
) -> Option<Applied> {
    set_cs3(CS3_LONG);
    let clk = bus.read32(MBXCLK);
    restore(bus, (clk & 3) | 1 << 2);
    delay_us(1_000);
    if Speed::now(bus) == speed {
        return Some(Applied::default());
    }
    set_speed(bus, speed, &mut delay_us)
}

/// A clock step's second half: the cores' divider `div`, then the bus
/// timing of the tier `cs3` (the enable kept).
pub fn finish<B: RegisterBus>(
    bus: &mut B,
    div: u32,
    cs3: Cs3,
    mut delay_us: impl FnMut(u32),
    mut set_cs3: impl FnMut(Cs3),
) {
    let clk = bus.read32(MBXCLK);
    restore(bus, (clk & 3) | (div.clamp(1, 4) - 1) << 2);
    delay_us(1_000);
    set_cs3(cs3);
}

/// Puts `rung` in place, the way every clock run has (`rp-gpubench`'s
/// clock steps, ADR-0035): [`begin`], then [`finish`]. What [`set_speed`]
/// did, or `None` if the PLL didn't lock (Windows Mobile's speed back, the
/// cores at divide 2, the long bus timing kept: the caller puts everything
/// back with [`put_back`]). Nothing else may use the 2700G meanwhile.
pub fn step<B: RegisterBus>(
    bus: &mut B,
    rung: Rung,
    mut delay_us: impl FnMut(u32),
    mut set_cs3: impl FnMut(Cs3),
) -> Option<Applied> {
    let a = begin(bus, rung.speed, &mut delay_us, &mut set_cs3)?;
    finish(bus, rung.div, rung.cs3(), delay_us, set_cs3);
    Some(a)
}

/// Windows Mobile's clock and bus timing back, wherever a step left them
/// (no state kept, so a crash path can call it): the long bus timing, the
/// cores at divide 2, the PLL always written again (run G1: registers put
/// back, the 2700G still not working until it was), Windows Mobile's bus
/// timing. What [`set_speed`] did.
pub fn put_back<B: RegisterBus>(
    bus: &mut B,
    mut delay_us: impl FnMut(u32),
    mut set_cs3: impl FnMut(Cs3),
) -> Option<Applied> {
    set_cs3(CS3_LONG);
    let clk = bus.read32(MBXCLK);
    restore(bus, (clk & 3) | 1 << 2);
    delay_us(1_000);
    let a = set_speed(bus, Speed::WM, &mut delay_us);
    set_cs3(CS3_WM);
    a
}

/// The MBX clocks stopped (`MBXCLK` enable 0, the divider kept) by
/// [`stop`]: it holds the register bus, so nothing can read or write an MBX
/// register while the clocks are off (the bus would hang: C43). Dropped or
/// [`resumed`](Stopped::resume), it writes back the value it found.
///
/// The guard is the compiler's: a register touched while the clocks are
/// stopped doesn't build.
///
/// ```compile_fail
/// use rp_drv_mbx_display::clock::stop;
/// use rp_hal_api::RegisterBus;
/// let mut bus = rp_hal_mock::MockBus::new();
/// let s = stop(&mut bus);
/// let _status = bus.read32(0x12c); // the bus is held: error[E0499]
/// s.resume();
/// ```
#[must_use = "dropping it puts the clocks straight back"]
pub struct Stopped<'a, B: RegisterBus> {
    bus: &'a mut B,
    was: u32,
}

impl<B: RegisterBus> Stopped<'_, B> {
    /// `MBXCLK` as [`stop`] found it.
    #[must_use]
    pub fn found(&self) -> u32 {
        self.was
    }

    /// The clocks back on as found.
    pub fn resume(self) {}

    /// The clocks back on as `mbxclk` (its divider and enable, an enable of
    /// 0 written as 1, as [`restore`]): a divider changed while the clocks
    /// are off, Dell's order for the cores' divider (`ddi_2700g`
    /// `0x02120370`, docs/hw/2700g.md "Dell's register writes").
    pub fn resume_as(mut self, mbxclk: u32) {
        self.was = never_off(mbxclk);
    }

    /// A display register read while the clocks are off: only the display
    /// block (`0x2000`-`0x23FF`: the planes, the scan line, `DSIG`, the
    /// event and underrun flags); `None` for any other offset. The display
    /// has its own clock: `rp-gpubench` run WG on the X51v (M7, 2026-10-30,
    /// gpubench-0248/-0249/-0251) read `DVLNUM`, `DSIG` and `DINTRS` 2,700
    /// times with `MBXCLK` enable 0 and with `MBXCLK` 0, and the display
    /// held every refresh; an MBX core register read is what hung (run FS).
    pub fn display_read32(&mut self, offset: usize) -> Option<u32> {
        DISPLAY_BLOCK
            .contains(&offset)
            .then(|| self.bus.read32(offset))
    }
}

/// The display block's registers, readable while the MBX clocks are off
/// ([`Stopped::display_read32`]).
pub const DISPLAY_BLOCK: core::ops::Range<usize> = 0x2000..0x2400;

/// [`stop`] as Dell's off path writes it: the whole register 0, the
/// divider too (`ddi_2700g`, REA #39), until the returned [`Stopped`] is
/// dropped or resumed (the value found put back).
pub fn stop_whole<B: RegisterBus>(bus: &mut B) -> Stopped<'_, B> {
    let was = bus.read32(MBXCLK);
    bus.write32(MBXCLK, 0);
    Stopped { bus, was }
}

impl<B: RegisterBus> Drop for Stopped<'_, B> {
    fn drop(&mut self) {
        self.bus.write32(MBXCLK, self.was);
    }
}

/// Stops the MBX clocks (enable 0, the divider kept) until the returned
/// [`Stopped`] is dropped or resumed. Queued 2D work pauses and finishes
/// afterwards (run FS); the bus is held so no register is touched meanwhile.
pub fn stop<B: RegisterBus>(bus: &mut B) -> Stopped<'_, B> {
    let was = bus.read32(MBXCLK);
    bus.write32(MBXCLK, was & !3);
    Stopped { bus, was }
}

/// Writes `MBXCLK` = `mbxclk`, a value kept from before (the loader's
/// put-back, a test image's run): the divider and the enable as given,
/// except that an enable of 0 is written as 1 (2D on): the clocks go off
/// only through [`stop`]. Returns what was written.
pub fn restore<B: RegisterBus>(bus: &mut B, mbxclk: u32) -> u32 {
    let v = never_off(mbxclk);
    bus.write32(MBXCLK, v);
    v
}

/// `mbxclk` as [`restore`] writes it: an enable of 0 (the clocks off) as 1
/// (2D on). A record of the board keeps this, so what is put back reads
/// back as recorded (the emulator starts `MBXCLK` at 0; the X51v at
/// Windows Mobile's `0x5`).
#[must_use]
pub const fn never_off(mbxclk: u32) -> u32 {
    if mbxclk & 3 == 0 { mbxclk | 1 } else { mbxclk }
}

/// `MBXCLK` with the 2D and 3D clocks on, the divider kept.
#[must_use]
pub const fn with_3d(mbxclk: u32) -> u32 {
    (mbxclk & !3) | 2
}

/// `MBXCLK` with the 2D clock only, the divider kept.
#[must_use]
pub const fn two_d_only(mbxclk: u32) -> u32 {
    (mbxclk & !3) | 1
}

/// Whether the 3D clock is on.
#[must_use]
pub const fn has_3d(mbxclk: u32) -> bool {
    mbxclk & 3 == 2
}

/// Keeps the 3D clock on while 3D work comes and switches it off after
/// `idle_us` without any. Times are microseconds from any wrapping clock.
#[derive(Clone, Copy, Debug)]
pub struct Gate {
    idle_us: u32,
    last: Option<u32>,
}

impl Gate {
    /// A gate that switches off after `idle_us` without work.
    #[must_use]
    pub const fn new(idle_us: u32) -> Self {
        Self {
            idle_us,
            last: None,
        }
    }

    /// Before 3D work at `now_us`: the 3D clock on. True if it was off
    /// (the caller then redoes the 3D set-up; see the module notes).
    pub fn work<B: RegisterBus>(&mut self, bus: &mut B, now_us: u32) -> bool {
        self.last = Some(now_us);
        let v = bus.read32(MBXCLK);
        if has_3d(v) {
            return false;
        }
        bus.write32(MBXCLK, with_3d(v));
        true
    }

    /// When idle at `now_us`: the 3D clock off once `idle_us` has passed
    /// since the last work. True if it switched off now.
    pub fn idle<B: RegisterBus>(&mut self, bus: &mut B, now_us: u32) -> bool {
        let Some(last) = self.last else {
            return false;
        };
        if now_us.wrapping_sub(last) < self.idle_us {
            return false;
        }
        self.last = None;
        let v = bus.read32(MBXCLK);
        if !has_3d(v) {
            return false;
        }
        bus.write32(MBXCLK, two_d_only(v));
        true
    }

    /// When the gate wants to be asked again (for a tickless timer), if
    /// the clock is on.
    #[must_use]
    pub fn due(&self) -> Option<u32> {
        self.last.map(|t| t.wrapping_add(self.idle_us))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ladder_runs_cl2_at_every_clock_with_dells_timings() {
        // ADR-0035: CL2 always; the rest Dell's rule.
        for pll in [0xBC1, 0xBB1, 41 << 7 | 4 << 4 | 1, 23 << 7 | 2 << 4 | 1] {
            let (l, r) = (Speed::ladder(pll), Speed::rated(pll));
            assert_eq!(l.lmtype, 0xAC8, "pll 0x{pll:x}");
            assert_eq!((l.pll, l.lmtim, l.lmrefresh), (r.pll, r.lmtim, r.lmrefresh));
        }
        // Below 100 MHz the ladder is Dell's own; above it Dell's would be
        // CL3, which the ladder never is.
        assert_eq!(Speed::ladder(0xBB1), Speed::WM);
        assert_eq!(Speed::ladder(0xBC1), Speed::A75);
        assert_eq!(Speed::rated(41 << 7 | 4 << 4 | 1).lmtype, 0xEC8);
    }

    #[test]
    fn the_divider_is_kept() {
        // WM's 0x5: divider 1, 2D only.
        assert_eq!(with_3d(0x5), 0x6);
        assert_eq!(two_d_only(0x6), 0x5);
        assert_eq!(with_3d(0x9), 0xA);
        assert!(has_3d(0xA) && !has_3d(0x9));
    }

    #[test]
    fn stop_holds_the_bus_and_puts_the_clock_back() {
        let mut bus = rp_hal_mock::MockBus::new();
        bus.preset(MBXCLK, 0x5);
        {
            let s = stop(&mut bus);
            assert_eq!(s.found(), 0x5);
            // `bus.read32(0x12c)` here doesn't compile: `s` holds the bus.
        }
        assert_eq!(bus.writes(), [(MBXCLK, 0x4), (MBXCLK, 0x5)]);
        bus.preset(MBXCLK, 0xA);
        stop(&mut bus).resume();
        assert_eq!(&bus.writes()[2..], [(MBXCLK, 0x8), (MBXCLK, 0xA)]);
    }

    #[test]
    fn stopped_reads_the_display_block_only() {
        let mut bus = rp_hal_mock::MockBus::new();
        bus.preset(MBXCLK, 0x5);
        bus.preset(0x2184, 0xABCD);
        {
            let mut s = stop_whole(&mut bus);
            assert_eq!(s.display_read32(0x2184), Some(0xABCD));
            assert_eq!(s.display_read32(0x2308), Some(0));
            // The MBX core (status at 0x412c), the system block (ODFBSTAT
            // 0x068), MBXCLK itself and the block's edges: refused.
            for off in [0x412C, 0x068, MBXCLK, 0x1FFC, 0x2400] {
                assert_eq!(s.display_read32(off), None, "0x{off:x}");
            }
        }
        assert_eq!(bus.writes(), [(MBXCLK, 0), (MBXCLK, 0x5)]);
    }

    #[test]
    fn resume_as_changes_the_divider_with_the_clocks_off() {
        let mut bus = rp_hal_mock::MockBus::new();
        // Divide 2 with both clocks, to divide 1 with the 2D clock (G5's
        // clock order): off, then the new value; never left off.
        bus.preset(MBXCLK, 0x6);
        stop(&mut bus).resume_as(0x2);
        bus.preset(MBXCLK, 0x6);
        stop(&mut bus).resume_as(0x0);
        assert_eq!(
            bus.writes(),
            [(MBXCLK, 0x4), (MBXCLK, 0x2), (MBXCLK, 0x4), (MBXCLK, 0x1)]
        );
    }

    #[test]
    fn restore_never_leaves_the_clocks_off() {
        let mut bus = rp_hal_mock::MockBus::new();
        assert_eq!(restore(&mut bus, 0x5), 0x5);
        assert_eq!(restore(&mut bus, 0x6), 0x6);
        assert_eq!(restore(&mut bus, 0x4), 0x5, "enable 0 written as 2D on");
        assert_eq!(bus.writes(), [(MBXCLK, 0x5), (MBXCLK, 0x6), (MBXCLK, 0x5)]);
    }

    #[test]
    fn on_for_work_off_after_idle() {
        let mut bus = rp_hal_mock::MockBus::new();
        bus.preset(MBXCLK, 0x5);
        let mut g = Gate::new(100_000);
        assert_eq!(g.due(), None);
        assert!(g.work(&mut bus, 1_000), "switched on");
        assert_eq!(bus.writes(), [(MBXCLK, 0x6)]);
        bus.preset(MBXCLK, 0x6);
        assert!(!g.work(&mut bus, 50_000), "already on");
        assert_eq!(g.due(), Some(150_000));
        assert!(!g.idle(&mut bus, 149_999), "not idle long enough");
        assert!(g.idle(&mut bus, 150_000), "switched off");
        assert_eq!(bus.writes().last(), Some(&(MBXCLK, 0x5)));
        assert_eq!(g.due(), None);
        bus.preset(MBXCLK, 0x5);
        assert!(!g.idle(&mut bus, 999_999), "nothing to switch off");
    }

    #[test]
    fn speeds_as_computed() {
        assert_eq!(Speed::WM.khz(), 99_666);
        assert_eq!(Speed::A75.khz(), 74_750);
        assert!(Speed::A75.khz() <= 75_000 && Speed::WM.khz() <= 100_000);
        // Dell's routine (docs/hw/2700g.md): cycles = ceil(ns / period),
        // tRAS raised to tRC - tRP + 1.
        let lmtim = |khz: u32| {
            let cyc = |ns: u32| (ns * khz).div_ceil(1_000_000);
            let (trc, trp, trcd) = (cyc(85), cyc(25), cyc(25));
            let tras = cyc(48).max(trc - trp + 1);
            tras << 16 | trp << 12 | trcd << 8 | trc << 4 | 2
        };
        // Refresh: 64 ms over 4096 rows, whole cycles, rounded down.
        let refresh = |khz: u32| khz * 15_625 / 1_000_000;
        for s in [Speed::WM, Speed::A75] {
            assert_eq!(lmtim(s.khz()), s.lmtim);
            assert_eq!(refresh(s.khz()), s.lmrefresh);
            assert_eq!(s.lmtype >> 10 & 7, 2, "CL2 below 100 MHz");
            assert_eq!(Speed::rated(s.pll), s, "rated gives the known speeds");
        }
        // Dell's own 133.25 MHz timing (13 x 41 / 4): CL3, 0x944C2.
        let fast = Speed::rated(41 << 7 | 4 << 4 | 1);
        assert_eq!(fast.khz(), 133_250);
        assert_eq!((fast.lmtim, fast.lmtype), (0x944C2, 0xEC8));
    }

    #[test]
    fn a_rung_step_moves_the_bus_long_first_and_sets_the_divider_last() {
        // The sequence every clock run used (`rp-gpubench`'s
        // `clock_apply`): CS3 long, the cores at divide 2, the PLL and its
        // timing, the rung's divider, then the rung's CS3 tier.
        let mut bus = rp_hal_mock::MockBus::new();
        bus.preset(PLLSTAT, 7).preset(MBXCLK, 0x6); // 3D on, divide 2
        let mut cs3 = std::vec::Vec::new();
        let a = step(&mut bus, Rung::TOP, |_| (), |c| cs3.push(c)).unwrap();
        assert_eq!(a.unwritten, 0);
        assert_eq!(cs3, [CS3_LONG, CS3_WM], "149.50 MHz: Dell's fast tier");
        let w = bus.writes();
        assert_eq!(w.first(), Some(&(MBXCLK, 0x6)), "divide 2, enable kept");
        assert!(w.contains(&(COREPLL, 23 << 7 | 2 << 4 | 1)));
        assert_eq!(w.last(), Some(&(MBXCLK, 0x6)), "the rung's divide 2");
        assert_eq!(Rung::TOP.speed.khz(), 149_500);
        assert_eq!(Rung::TOP.speed.lmtype, 0xAC8, "CL2 always (ADR-0035)");
        // Already at the speed: only the divider moves (no PLL write).
        let mut bus = rp_hal_mock::MockBus::new();
        bus.preset(PLLSTAT, 7).preset(MBXCLK, 0x6);
        for (r, v) in [
            (COREPLL, Speed::A75.pll),
            (LMTYPE, Speed::A75.lmtype),
            (LMTIM, Speed::A75.lmtim),
            (LMREFRESH, Speed::A75.lmrefresh),
        ] {
            bus.preset(r, v);
        }
        let mut cs3 = std::vec::Vec::new();
        let a75 = Rung {
            speed: Speed::A75,
            div: 1,
        };
        assert_eq!(
            step(&mut bus, a75, |_| (), |c| cs3.push(c)),
            Some(Applied::default())
        );
        assert_eq!(bus.writes(), [(MBXCLK, 0x6), (MBXCLK, 0x2)]);
        assert_eq!(cs3, [CS3_LONG, CS3_LONG], "74.75 MHz: the long tier");
    }

    #[test]
    fn put_back_always_writes_the_pll_again() {
        // Run G1: the registers read back right and the 2700G still didn't
        // work until the PLL was written again.
        let mut bus = rp_hal_mock::MockBus::new();
        bus.preset(PLLSTAT, 7).preset(MBXCLK, 0x2);
        for (r, v) in [
            (COREPLL, Speed::WM.pll),
            (LMTYPE, Speed::WM.lmtype),
            (LMTIM, Speed::WM.lmtim),
            (LMREFRESH, Speed::WM.lmrefresh),
        ] {
            bus.preset(r, v);
        }
        let mut cs3 = std::vec::Vec::new();
        assert!(put_back(&mut bus, |_| (), |c| cs3.push(c)).is_some());
        assert!(bus.writes().contains(&(COREPLL, Speed::WM.pll)));
        assert_eq!(bus.writes().first(), Some(&(MBXCLK, 0x6)));
        assert_eq!(cs3, [CS3_LONG, CS3_WM]);
        assert_eq!(Rung::WM.cs3(), CS3_WM);
    }

    #[test]
    fn set_speed_on_the_reference_clock() {
        let mut bus = rp_hal_mock::MockBus::new();
        bus.preset(PLLSTAT, 7);
        let mut waited = 0;
        assert_eq!(
            set_speed(&mut bus, Speed::A75, |us| waited += us),
            Some(Applied::default())
        );
        assert_eq!(
            bus.writes(),
            [
                (SYSCLKSRC, 0),
                (LMTIM, 0x62272),
                (LMREFRESH, 0x48F),
                (LMTYPE, 0xAC8),
                (COREPLL, 0xBC1),
                (SYSCLKSRC, 2),
            ]
        );
        // The settle after each memory write; the drop polls (the mock's
        // PLL never drops), the lock settle.
        assert_eq!(
            waited,
            500 + 3 * WRITE_SETTLE_US + DROP_POLLS * LOCK_POLL_US + LOCK_SETTLE_US
        );
    }

    #[test]
    fn a_dropped_write_is_written_again() {
        // Run G3: LMREFRESH read back unchanged after a write.
        let mut bus = rp_hal_mock::MockBus::new();
        bus.preset(PLLSTAT, 7);
        bus.script_reads(LMREFRESH, &[0x615]);
        let a = set_speed(&mut bus, Speed::A75, |_| ()).unwrap();
        assert_eq!((a.rewrites, a.unwritten), (1, 0));
        let w = bus.writes();
        assert_eq!(w.iter().filter(|x| **x == (LMREFRESH, 0x48F)).count(), 2);
    }

    #[test]
    fn a_write_that_never_holds_is_reported() {
        let mut bus = rp_hal_mock::MockBus::new();
        bus.preset(PLLSTAT, 7);
        bus.script_reads(LMTYPE, &[0xEC8, 0xEC8, 0xEC8]);
        let a = set_speed(&mut bus, Speed::A75, |_| ()).unwrap();
        assert_eq!((a.rewrites, a.unwritten), (2, 1));
    }

    #[test]
    fn the_lock_is_awaited_after_its_drop() {
        // G3's trace: the old "locked" once, then not locked, then locked.
        let mut bus = rp_hal_mock::MockBus::new();
        bus.script_reads(PLLSTAT, &[0x7, 0xB, 0xB, 0xB, 0xF]);
        bus.preset(PLLSTAT, 0xF);
        let a = set_speed(&mut bus, Speed::A75, |_| ()).unwrap();
        assert_eq!(a.lock_us, 2 * LOCK_POLL_US);
    }

    #[test]
    fn no_lock_puts_wm_back() {
        let mut bus = rp_hal_mock::MockBus::new();
        bus.preset(PLLSTAT, 0);
        let mut waited = 0u32;
        assert_eq!(set_speed(&mut bus, Speed::A75, |us| waited += us), None);
        let w = bus.writes();
        assert_eq!(
            &w[5..9],
            [
                (LMTIM, 0x73392),
                (LMREFRESH, 0x615),
                (LMTYPE, 0xAC8),
                (COREPLL, 0xBB1)
            ]
        );
        assert_eq!(w.last(), Some(&(SYSCLKSRC, 2)), "back on the PLL even so");
        assert!(waited >= 100_000, "50 ms awaited twice");
    }

    #[test]
    fn idle_time_survives_the_clock_wrapping() {
        let mut bus = rp_hal_mock::MockBus::new();
        bus.preset(MBXCLK, 0x5);
        let mut g = Gate::new(100);
        g.work(&mut bus, u32::MAX - 10);
        bus.preset(MBXCLK, 0x6);
        assert!(!g.idle(&mut bus, 50));
        assert!(g.idle(&mut bus, 89));
    }
}
