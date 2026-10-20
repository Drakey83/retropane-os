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
    fn the_divider_is_kept() {
        // WM's 0x5: divider 1, 2D only.
        assert_eq!(with_3d(0x5), 0x6);
        assert_eq!(two_d_only(0x6), 0x5);
        assert_eq!(with_3d(0x9), 0xA);
        assert!(has_3d(0xA) && !has_3d(0x9));
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
