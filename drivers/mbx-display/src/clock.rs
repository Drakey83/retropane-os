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

use rp_hal_api::RegisterBus;

/// The clock register's offset in the 2700G register window.
pub const MBXCLK: usize = 0x054;

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
