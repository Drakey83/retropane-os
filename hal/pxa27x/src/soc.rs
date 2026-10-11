//! PXA27x interrupt controller and OS timer register logic, generic over
//! [`RegisterBus`] so it's unit-tested with the mock (PXA27x Developer's
//! Manual: interrupt controller and OS timer chapters; IRQ numbers match QEMU's
//! `pxa.h`, which follows the manual).

use rp_hal_api::RegisterBus;

/// Interrupt sources used so far.
pub mod irq {
    /// Keypad controller.
    pub const KEYPAD: u32 = 4;
    /// GPIO 2..120 edge detect.
    pub const GPIO_X: u32 = 10;
    /// MMC/SD controller.
    pub const MMC: u32 = 23;
    /// SSP1.
    pub const SSP1: u32 = 24;
    /// OS timer match 1 (the tick).
    pub const OST1: u32 = 27;
}

/// Interrupt controller registers.
pub mod intc_reg {
    /// IRQ pending (sources 0-31).
    pub const ICIP: usize = 0x00;
    /// Mask (1 = enabled), sources 0-31.
    pub const ICMR: usize = 0x04;
    /// Level (0 = IRQ, 1 = FIQ), sources 0-31.
    pub const ICLR: usize = 0x08;
    /// Control: bit 0 DIM (only unmasked sources wake from idle).
    pub const ICCR: usize = 0x14;
    /// IRQ pending, sources 32-39.
    pub const ICIP2: usize = 0x9C;
    /// Mask, sources 32-39.
    pub const ICMR2: usize = 0xA0;
    /// Level, sources 32-39.
    pub const ICLR2: usize = 0xA4;
}

/// OS timer registers.
pub mod ost_reg {
    /// Match 1.
    pub const OSMR1: usize = 0x04;
    /// Counter 0.
    pub const OSCR0: usize = 0x10;
    /// Status (write 1 to clear).
    pub const OSSR: usize = 0x14;
    /// Interrupt enable.
    pub const OIER: usize = 0x1C;
}

/// The interrupt controller.
pub struct Intc<B>(pub B);

impl<B: RegisterBus> Intc<B> {
    /// Everything masked and routed to IRQ (not FIQ); only unmasked sources
    /// wake the core from idle.
    pub fn reset(&mut self) {
        self.0.write32(intc_reg::ICMR, 0);
        self.0.write32(intc_reg::ICMR2, 0);
        self.0.write32(intc_reg::ICLR, 0);
        self.0.write32(intc_reg::ICLR2, 0);
        self.0.write32(intc_reg::ICCR, 1);
    }

    fn mask_reg(n: u32) -> (usize, u32) {
        if n < 32 {
            (intc_reg::ICMR, 1 << n)
        } else {
            (intc_reg::ICMR2, 1 << (n - 32))
        }
    }

    /// Unmasks source `n` (0-39).
    pub fn enable(&mut self, n: u32) {
        let (r, bit) = Self::mask_reg(n);
        let v = self.0.read32(r) | bit;
        self.0.write32(r, v);
    }

    /// Masks source `n` (0-39).
    pub fn disable(&mut self, n: u32) {
        let (r, bit) = Self::mask_reg(n);
        let v = self.0.read32(r) & !bit;
        self.0.write32(r, v);
    }

    /// Pending, unmasked IRQ sources as a 40-bit mask.
    pub fn pending(&mut self) -> u64 {
        u64::from(self.0.read32(intc_reg::ICIP))
            | (u64::from(self.0.read32(intc_reg::ICIP2) & 0xFF) << 32)
    }
}

/// The OS timer, used for the periodic tick on match register 1.
pub struct OsTimer<B>(pub B);

impl<B: RegisterBus> OsTimer<B> {
    /// Arms match 1 `period` ticks from now and enables its interrupt.
    pub fn start(&mut self, period: u32) {
        let now = self.0.read32(ost_reg::OSCR0);
        self.0.write32(ost_reg::OSSR, 1 << 1);
        self.0.write32(ost_reg::OSMR1, now.wrapping_add(period));
        let oier = self.0.read32(ost_reg::OIER) | (1 << 1);
        self.0.write32(ost_reg::OIER, oier);
    }

    /// Acknowledges a match-1 interrupt and re-arms it `period` ticks after the
    /// previous match, so the tick doesn't drift with interrupt latency. If the
    /// next deadline has already passed, it's moved to `period` from now.
    pub fn rearm(&mut self, period: u32) {
        self.0.write32(ost_reg::OSSR, 1 << 1);
        let next = self.0.read32(ost_reg::OSMR1).wrapping_add(period);
        let now = self.0.read32(ost_reg::OSCR0);
        // `next` is in the past if it's more than half the counter range behind.
        let next = if next.wrapping_sub(now) > u32::MAX / 2 {
            now.wrapping_add(period)
        } else {
            next
        };
        self.0.write32(ost_reg::OSMR1, next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_hal_mock::MockBus;

    #[test]
    fn reset_masks_everything_and_sets_dim() {
        let mut i = Intc(MockBus::new());
        i.reset();
        let b = i.0;
        assert_eq!(b.value(intc_reg::ICMR), 0);
        assert_eq!(b.value(intc_reg::ICCR), 1);
    }

    #[test]
    fn enable_and_disable_touch_only_their_bit() {
        let mut bus = MockBus::new();
        bus.preset(intc_reg::ICMR, 1 << 4);
        let mut i = Intc(bus);
        i.enable(irq::OST1);
        i.enable(35);
        assert_eq!(i.0.value(intc_reg::ICMR), (1 << 4) | (1 << 27));
        assert_eq!(i.0.value(intc_reg::ICMR2), 1 << 3);
        i.disable(irq::KEYPAD);
        assert_eq!(i.0.value(intc_reg::ICMR), 1 << 27);
    }

    #[test]
    fn pending_combines_both_banks() {
        let mut bus = MockBus::new();
        bus.preset(intc_reg::ICIP, 1 << 27)
            .preset(intc_reg::ICIP2, 0x101);
        assert_eq!(Intc(bus).pending(), (1 << 27) | (1 << 32));
    }

    #[test]
    fn start_arms_match1_relative_to_now() {
        let mut bus = MockBus::new();
        bus.preset(ost_reg::OSCR0, 1000);
        let mut t = OsTimer(bus);
        t.start(16_250);
        assert_eq!(t.0.value(ost_reg::OSMR1), 17_250);
        assert_eq!(t.0.value(ost_reg::OIER), 2);
    }

    #[test]
    fn rearm_keeps_cadence_and_recovers_from_overruns() {
        let mut bus = MockBus::new();
        bus.preset(ost_reg::OSMR1, 10_000)
            .preset(ost_reg::OSCR0, 10_050);
        let mut t = OsTimer(bus);
        t.rearm(1000);
        assert_eq!(t.0.value(ost_reg::OSMR1), 11_000); // no drift
        t.0.preset(ost_reg::OSCR0, 13_500); // missed two ticks
        t.rearm(1000);
        assert_eq!(t.0.value(ost_reg::OSMR1), 14_500); // resynchronised
        assert!(t.0.writes().contains(&(ost_reg::OSSR, 2)));
    }

    #[test]
    fn rearm_handles_counter_wrap() {
        let mut bus = MockBus::new();
        bus.preset(ost_reg::OSMR1, u32::MAX - 100)
            .preset(ost_reg::OSCR0, u32::MAX - 50);
        let mut t = OsTimer(bus);
        t.rearm(1000);
        assert_eq!(
            t.0.value(ost_reg::OSMR1),
            (u32::MAX - 100).wrapping_add(1000)
        );
    }
}
