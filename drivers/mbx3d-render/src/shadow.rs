//! A copy of the 3D core's state registers (ADR-0026 D4): a register is
//! written only when its value changes, so a frame that changes nothing but
//! the tiles costs the start write alone, where Dell's driver rewrites ~29
//! registers through a function call each.
//!
//! The registers keep their values between frames (and even between
//! images: docs/hw/2700g.md); a core reset loses them, so [`Shadow::forget`]
//! follows every reset. Triggers (the start `0x680`, the memory manager's
//! `0x838`) are never shadowed: writing them is the point.

use rp_drv_blit2d::MBX;
use rp_hal_api::RegisterBus;

/// The registers kept (offsets in the MBX block): Dell's start table's
/// state, the target, and the memory manager's set-up.
const KEPT: [usize; 19] = [
    0x608, 0x60C, 0x610, 0x614, 0x618, 0x61C, 0x644, 0x648, 0x650, 0x654, 0x658, 0x65C, 0x660,
    0x6D8, 0x6DC, 0x824, 0x828, 0x82C, 0x83C,
];

/// The last value written to each kept register, if known.
#[derive(Clone, Debug, Default)]
pub struct Shadow {
    known: [Option<u32>; KEPT.len()],
    /// Register writes that reached the bus (for the frame statistics).
    pub writes: u32,
}

impl Shadow {
    /// Nothing known.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            known: [None; KEPT.len()],
            writes: 0,
        }
    }

    /// Writes `v` to MBX register `off` unless it is kept and already
    /// holds `v`. True if it wrote.
    pub fn write<B: RegisterBus>(&mut self, bus: &mut B, off: usize, v: u32) -> bool {
        match KEPT.iter().position(|&k| k == off) {
            Some(i) if self.known[i] == Some(v) => false,
            Some(i) => {
                self.known[i] = Some(v);
                self.put(bus, off, v)
            }
            None => self.put(bus, off, v),
        }
    }

    fn put<B: RegisterBus>(&mut self, bus: &mut B, off: usize, v: u32) -> bool {
        bus.write32(MBX + off, v);
        self.writes += 1;
        true
    }

    /// Forgets every value (after a core reset, or the clock off and on
    /// until device test T9 says the registers survive it).
    pub fn forget(&mut self) {
        self.known = [None; KEPT.len()];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_only_changes() {
        let mut bus = rp_hal_mock::MockBus::new();
        let mut s = Shadow::new();
        assert!(s.write(&mut bus, 0x608, 5));
        assert!(!s.write(&mut bus, 0x608, 5), "unchanged");
        assert!(s.write(&mut bus, 0x608, 6));
        assert!(s.write(&mut bus, 0x680, 1), "the start is never kept");
        assert!(s.write(&mut bus, 0x680, 1));
        s.forget();
        assert!(s.write(&mut bus, 0x608, 6), "forgotten after a reset");
        assert_eq!(
            bus.writes(),
            [
                (MBX + 0x608, 5),
                (MBX + 0x608, 6),
                (MBX + 0x680, 1),
                (MBX + 0x680, 1),
                (MBX + 0x608, 6)
            ]
        );
        assert_eq!(s.writes, 5);
    }
}
