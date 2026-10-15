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

    /// 13 x 41 / 4 = 133.25 MHz (Dell's default "CoreClock"; the cores at
    /// half under `MBXCLK` divide by 2), with the timing Dell's routine
    /// gives for 7.5 ns cycles: CL3, tRC 12, tRP 4, tRCD 4, tRAS 9, tDPL 2.
    /// WM's refresh count is kept (11.7 us at this clock, more often than
    /// needed). Past the memory's rating: 25% faster in `rp-mbx3d` run 27's
    /// short pass (with CL2); a soak (version 28) decides whether it ships.
    pub const FAST: Self = Self {
        pll: 41 << 7 | 4 << 4 | 1,
        lmtype: 0xEC8,
        lmtim: 0x944C2,
        lmrefresh: 0x615,
    };

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

/// The poll interval and limit while the core PLL locks (Dell's driver
/// waits up to 50 ms; `rp-mbx3d` run 27 saw it lock at once).
const LOCK_POLL_US: u32 = 20;
const LOCK_POLLS: u32 = 2_500;

/// Sets the system clock and the memory timing to `speed`, as Dell's driver
/// changes the PLL (`0x0211faf0`): the system clock on the 13 MHz
/// reference, the memory timing and the PLL written, the lock awaited, the
/// system clock back on the PLL. The timing is written on the reference,
/// where both the old and the new are slow enough. Nothing else may use
/// the 2700G meanwhile (about half a millisecond; the display reads its
/// picture slower and may show a flicker).
///
/// Returns the lock time in microseconds, or `None` if the PLL did not lock
/// in 50 ms; then [`Speed::WM`] is put back (the PLL, awaited, and the
/// timing).
pub fn set_speed<B: RegisterBus>(
    bus: &mut B,
    speed: Speed,
    mut delay_us: impl FnMut(u32),
) -> Option<u32> {
    bus.write32(SYSCLKSRC, 0);
    delay_us(250);
    let locked = write_speed(bus, speed, &mut delay_us);
    if locked.is_none() {
        let _ = write_speed(bus, Speed::WM, &mut delay_us);
    }
    bus.write32(SYSCLKSRC, 2);
    delay_us(250);
    locked
}

/// The timing and the PLL written (on the reference clock); the lock
/// awaited: microseconds, or `None`.
fn write_speed<B: RegisterBus>(
    bus: &mut B,
    s: Speed,
    delay_us: &mut impl FnMut(u32),
) -> Option<u32> {
    bus.write32(LMTIM, s.lmtim);
    bus.write32(LMREFRESH, s.lmrefresh);
    bus.write32(LMTYPE, s.lmtype);
    bus.write32(COREPLL, s.pll);
    for k in 0..LOCK_POLLS {
        if bus.read32(PLLSTAT) & CORE_LOCKED != 0 {
            return Some(k * LOCK_POLL_US);
        }
        delay_us(LOCK_POLL_US);
    }
    None
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
        assert_eq!(Speed::FAST.khz(), 133_250);
        assert_eq!(Speed::FAST.pll, 0x14C1, "run 27's COREPLL");
        // Dell's routine (docs/hw/2700g.md): cycles = ceil(ns / period),
        // tRAS raised to tRC - tRP + 1; CL3 at 100 MHz and up.
        let lmtim = |khz: u32| {
            let cyc = |ns: u32| (ns * khz).div_ceil(1_000_000);
            let (trc, trp, trcd) = (cyc(85), cyc(25), cyc(25));
            let tras = cyc(48).max(trc - trp + 1);
            tras << 16 | trp << 12 | trcd << 8 | trc << 4 | 2
        };
        assert_eq!(lmtim(Speed::WM.khz()), Speed::WM.lmtim);
        assert_eq!(lmtim(Speed::FAST.khz()), Speed::FAST.lmtim);
        assert_eq!(Speed::FAST.lmtype, Speed::WM.lmtype & !(7 << 10) | 3 << 10);
    }

    #[test]
    fn set_speed_on_the_reference_clock() {
        let mut bus = rp_hal_mock::MockBus::new();
        bus.preset(PLLSTAT, 7);
        let mut waited = 0;
        assert_eq!(set_speed(&mut bus, Speed::FAST, |us| waited += us), Some(0));
        assert_eq!(
            bus.writes(),
            [
                (SYSCLKSRC, 0),
                (LMTIM, 0x944C2),
                (LMREFRESH, 0x615),
                (LMTYPE, 0xEC8),
                (COREPLL, 0x14C1),
                (SYSCLKSRC, 2),
            ]
        );
        assert_eq!(waited, 500);
    }

    #[test]
    fn no_lock_puts_wm_back() {
        let mut bus = rp_hal_mock::MockBus::new();
        bus.preset(PLLSTAT, 0);
        let mut waited = 0u32;
        assert_eq!(set_speed(&mut bus, Speed::FAST, |us| waited += us), None);
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
