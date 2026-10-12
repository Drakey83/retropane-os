//! PXA27x interrupt controller and OS timer register logic, generic over
//! [`RegisterBus`] so it's unit-tested with the mock (PXA27x Developer's
//! Manual: interrupt controller and OS timer chapters; IRQ numbers match QEMU's
//! `pxa.h`, which follows the manual).

use rp_hal_api::RegisterBus;

/// Interrupt sources used so far.
pub mod irq {
    /// Keypad controller.
    pub const KEYPAD: u32 = 4;
    /// GPIO 0 edge detect.
    pub const GPIO0: u32 = 8;
    /// GPIO 1 edge detect.
    pub const GPIO1: u32 = 9;
    /// GPIO 2..120 edge detect.
    pub const GPIO_X: u32 = 10;
    /// MMC/SD controller.
    pub const MMC: u32 = 23;
    /// SSP1.
    pub const SSP1: u32 = 24;
    /// OS timer channels 4-11 (channel 4: the soft run limit).
    pub const OST_4_11: u32 = 7;
    /// OS timer match 1 (the tick).
    pub const OST1: u32 = 27;
    /// OS timer match 2 (the spare channel, [`super::SpareTimer`]).
    pub const OST2: u32 = 28;
}

/// The identifier a GPIO pin's edge interrupt is signalled under
/// ([`Dispatch::handle`]'s callback, `IrqLine::id`): after the 64 possible
/// controller sources.
#[must_use]
pub const fn gpio_line(pin: u8) -> u32 {
    64 + pin as u32
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
    /// Match 2.
    pub const OSMR2: usize = 0x08;
    /// Match 3 (the watchdog's).
    pub const OSMR3: usize = 0x0C;
    /// Watchdog enable: bit 0 resets the chip on a match-3 (set-only).
    pub const OWER: usize = 0x18;
    /// Counter 0.
    pub const OSCR0: usize = 0x10;
    /// Status (write 1 to clear).
    pub const OSSR: usize = 0x14;
    /// Interrupt enable.
    pub const OIER: usize = 0x1C;
    /// Counter 4 (channel 4's own counter, at the resolution `OMCR4` sets).
    pub const OSCR4: usize = 0x40;
    /// Match 4.
    pub const OSMR4: usize = 0x80;
    /// Match control 4: bits 2-0 the resolution (2: 1 ms ticks).
    pub const OMCR4: usize = 0xC0;
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

/// GPIO edge-detect registers, one per bank of 32 pins (PXA27x manual: the
/// fourth bank sits at +0x100).
pub mod gpio_reg {
    /// Rising-edge detect enable.
    pub const GRER: [usize; 4] = [0x30, 0x34, 0x38, 0x130];
    /// Falling-edge detect enable.
    pub const GFER: [usize; 4] = [0x3C, 0x40, 0x44, 0x13C];
    /// Edge detect status (write 1 to clear).
    pub const GEDR: [usize; 4] = [0x48, 0x4C, 0x50, 0x148];
}

/// Highest GPIO number.
pub const MAX_GPIO: u8 = 120;

/// Which edges of a GPIO raise its interrupt.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edge {
    /// Low to high.
    Rising,
    /// High to low.
    Falling,
    /// Either.
    Both,
}

/// The GPIO edge-detect registers. ADR-0007 rule 7: these belong to the
/// interrupt layer; GPIO drivers own direction and level only.
pub struct GpioEdges<B>(pub B);

impl<B: RegisterBus> GpioEdges<B> {
    fn bank(pin: u8) -> (usize, u32) {
        debug_assert!(pin <= MAX_GPIO, "GPIO {pin} doesn't exist");
        (usize::from(pin / 32), 1 << (pin % 32))
    }

    /// Enables edge detection on `pin`.
    pub fn arm(&mut self, pin: u8, edge: Edge) {
        let (b, bit) = Self::bank(pin);
        let set = |bus: &mut B, reg: usize, on: bool| {
            let v = bus.read32(reg);
            bus.write32(reg, if on { v | bit } else { v & !bit });
        };
        set(&mut self.0, gpio_reg::GRER[b], edge != Edge::Falling);
        set(&mut self.0, gpio_reg::GFER[b], edge != Edge::Rising);
    }

    /// Disables edge detection on `pin` (its interrupt is masked).
    pub fn disarm(&mut self, pin: u8) {
        let (b, bit) = Self::bank(pin);
        for reg in [gpio_reg::GRER[b], gpio_reg::GFER[b]] {
            let v = self.0.read32(reg) & !bit;
            self.0.write32(reg, v);
        }
    }

    /// Reads and clears the edge-detect status of every bank.
    pub fn take_events(&mut self) -> [u32; 4] {
        let mut ev = [0; 4];
        for (e, &reg) in ev.iter_mut().zip(&gpio_reg::GEDR) {
            *e = self.0.read32(reg);
            if *e != 0 {
                self.0.write32(reg, *e);
            }
        }
        ev
    }
}

/// Interrupt ownership and signalling (ADR-0007): which sources are claimed,
/// which have fired, and which fired with no owner. The first-level handler
/// only masks and signals; owners service their device in their own thread.
#[derive(Default)]
pub struct Dispatch {
    claimed: u64,
    claimed_gpio: [u32; 4],
    signalled: u64,
    signalled_gpio: [u32; 4],
    /// Unowned interrupt-controller sources that fired (now masked).
    pub unclaimed: u64,
    /// Unowned GPIO edges that fired (edge detection now off).
    pub unclaimed_gpio: [u32; 4],
}

/// The GPIO summary interrupts: serviced through the edge registers.
pub const GPIO_SOURCES: u64 = (1 << irq::GPIO0) | (1 << irq::GPIO1) | (1 << irq::GPIO_X);

impl Dispatch {
    /// No claims.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            claimed: 0,
            claimed_gpio: [0; 4],
            signalled: 0,
            signalled_gpio: [0; 4],
            unclaimed: 0,
            unclaimed_gpio: [0; 4],
        }
    }

    /// Claims interrupt-controller source `n`. False if it's taken or is a
    /// GPIO summary source (claim the pin instead).
    pub fn claim(&mut self, n: u32) -> bool {
        if n >= 40 {
            return false;
        }
        let bit = 1u64 << n;
        if self.claimed & bit != 0 || GPIO_SOURCES & bit != 0 {
            return false;
        }
        self.claimed |= bit;
        true
    }

    /// Claims GPIO `pin`'s edge interrupt. False if it's taken.
    pub fn claim_gpio(&mut self, pin: u8) -> bool {
        if pin > MAX_GPIO {
            return false;
        }
        let (b, bit) = (usize::from(pin / 32), 1u32 << (pin % 32));
        if self.claimed_gpio[b] & bit != 0 {
            return false;
        }
        self.claimed_gpio[b] |= bit;
        true
    }

    /// The first-level handler for the `pending` sources (the caller has
    /// already serviced its own, e.g. the tick, and left them out). Each
    /// claimed source or pin that fires is masked, recorded, and passed to
    /// `signalled` (controller source `n`, or [`gpio_line`] for a pin), so
    /// the kernel can wake the thread that waits for it.
    pub fn handle<I: RegisterBus, G: RegisterBus>(
        &mut self,
        intc: &mut Intc<I>,
        gpio: &mut GpioEdges<G>,
        pending: u64,
        signalled: &mut dyn FnMut(u32),
    ) {
        if pending & GPIO_SOURCES != 0 {
            let ev = gpio.take_events();
            for (b, &e) in ev.iter().enumerate() {
                let mut rest = e;
                while rest != 0 {
                    let i = rest.trailing_zeros();
                    rest &= rest - 1;
                    gpio.disarm((b as u32 * 32 + i) as u8);
                    if self.claimed_gpio[b] & (1 << i) != 0 {
                        self.signalled_gpio[b] |= 1 << i;
                        signalled(gpio_line((b as u32 * 32 + i) as u8));
                    } else {
                        self.unclaimed_gpio[b] |= 1 << i;
                    }
                }
            }
        }
        let mut rest = pending & !GPIO_SOURCES;
        while rest != 0 {
            let n = rest.trailing_zeros();
            rest &= rest - 1;
            intc.disable(n);
            if self.claimed & (1 << n) != 0 {
                self.signalled |= 1 << n;
                signalled(n);
            } else {
                self.unclaimed |= 1 << n;
            }
        }
    }

    /// Whether any claimed source or pin is signalled.
    #[must_use]
    pub fn any_signalled(&self) -> bool {
        self.signalled != 0 || self.signalled_gpio.iter().any(|&w| w != 0)
    }

    /// Takes (and clears) source `n`'s signal.
    pub fn take_signal(&mut self, n: u32) -> bool {
        let bit = 1u64 << n;
        let was = self.signalled & bit != 0;
        self.signalled &= !bit;
        was
    }

    /// Takes (and clears) GPIO `pin`'s signal.
    pub fn take_gpio_signal(&mut self, pin: u8) -> bool {
        let (b, bit) = (usize::from(pin / 32), 1u32 << (pin % 32));
        let was = self.signalled_gpio[b] & bit != 0;
        self.signalled_gpio[b] &= !bit;
        was
    }
}

/// OS timer match channel 2: a one-shot timer interrupt for kernel use
/// (the scheduler's alarm is channel 1). Its interrupt is claimed and
/// waited for like any device's (`rt::take_irq(irq::OST2)`).
pub struct SpareTimer<B>(pub B);

impl<B: RegisterBus> SpareTimer<B> {
    /// Fires the channel's interrupt when the counter reaches `at`.
    pub fn arm(&mut self, at: u32) {
        self.0.write32(ost_reg::OSSR, 1 << 2);
        self.0.write32(ost_reg::OSMR2, at);
        let oier = self.0.read32(ost_reg::OIER) | (1 << 2);
        self.0.write32(ost_reg::OIER, oier);
    }

    /// Acknowledges a match (the owner's part of servicing it).
    pub fn ack(&mut self) {
        self.0.write32(ost_reg::OSSR, 1 << 2);
    }

    /// The counter now.
    pub fn now(&mut self) -> u32 {
        self.0.read32(ost_reg::OSCR0)
    }
}

/// The watchdog: OS timer match 3 with `OWER.WME`. Once armed it can't be
/// disarmed (until a reset): it must be fed before every deadline or the
/// chip resets, which is the point: a hung system comes back by itself.
pub struct Watchdog<B>(pub B);

impl<B: RegisterBus> Watchdog<B> {
    /// Arms (if not yet) and sets the deadline `timeout` ticks from now.
    pub fn feed(&mut self, timeout: u32) {
        let now = self.0.read32(ost_reg::OSCR0);
        self.0.write32(ost_reg::OSMR3, now.wrapping_add(timeout));
        self.0.write32(ost_reg::OWER, 1);
    }

    /// Whether it's armed (feeding is then mandatory).
    pub fn armed(&mut self) -> bool {
        self.0.read32(ost_reg::OWER) & 1 != 0
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

    /// Arms match 1 as a one-shot alarm at counter value `at` (tickless
    /// scheduling). Returns false if `at` has already passed (no interrupt
    /// would come until the counter wraps), so the caller doesn't sleep.
    pub fn alarm(&mut self, at: u32) -> bool {
        self.0.write32(ost_reg::OSSR, 1 << 1);
        self.0.write32(ost_reg::OSMR1, at);
        let oier = self.0.read32(ost_reg::OIER) | (1 << 1);
        self.0.write32(ost_reg::OIER, oier);
        // Checked after arming: if the deadline is reached now, the match may
        // have been missed.
        let now = self.0.read32(ost_reg::OSCR0);
        at.wrapping_sub(now).wrapping_sub(1) < u32::MAX / 2
    }

    /// Disables the match-1 interrupt.
    pub fn stop(&mut self) {
        let oier = self.0.read32(ost_reg::OIER) & !(1 << 1);
        self.0.write32(ost_reg::OIER, oier);
        self.0.write32(ost_reg::OSSR, 1 << 1);
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
    extern crate std;
    use std::vec::Vec;

    #[test]
    fn feeding_the_watchdog_moves_its_deadline_and_arms_it() {
        let mut w = Watchdog(MockBus::new());
        w.0.preset(ost_reg::OSCR0, 1_000);
        assert!(!w.armed());
        w.feed(500);
        assert_eq!(w.0.value(ost_reg::OSMR3), 1_500);
        assert!(w.armed());
        w.0.preset(ost_reg::OSCR0, u32::MAX - 10);
        w.feed(100);
        assert_eq!(w.0.value(ost_reg::OSMR3), 89, "wraps with the counter");
    }

    #[test]
    fn the_spare_timer_arms_and_acknowledges_channel_2_only() {
        let mut t = SpareTimer(MockBus::new());
        t.0.preset(ost_reg::OIER, 1 << 1); // the alarm's enable stays
        t.arm(7000);
        assert_eq!(t.0.value(ost_reg::OSMR2), 7000);
        assert_eq!(t.0.value(ost_reg::OIER), (1 << 1) | (1 << 2));
        t.ack();
        assert_eq!(t.0.writes().last(), Some(&(ost_reg::OSSR, 1 << 2)));
    }

    #[test]
    fn alarms_in_the_past_are_reported() {
        let mut t = OsTimer(MockBus::new());
        t.0.preset(ost_reg::OSCR0, 1000);
        assert!(t.alarm(5000), "future deadline: armed");
        assert_eq!(t.0.value(ost_reg::OSMR1), 5000);
        assert_eq!(t.0.value(ost_reg::OIER) & 2, 2);
        assert!(!t.alarm(1000), "now: too late to wait for");
        assert!(!t.alarm(900), "past");
        t.0.preset(ost_reg::OSCR0, u32::MAX - 10);
        assert!(t.alarm(20), "after the wrap is the future");
    }

    #[test]
    fn gpio_edges_arm_and_disarm_only_their_pin() {
        let mut g = GpioEdges(MockBus::new());
        g.0.preset(gpio_reg::GRER[2], 1)
            .preset(gpio_reg::GFER[2], 1);
        g.arm(94, Edge::Falling); // bank 2, bit 30
        assert_eq!(g.0.value(gpio_reg::GRER[2]), 1);
        assert_eq!(g.0.value(gpio_reg::GFER[2]), 1 | (1 << 30));
        g.arm(94, Edge::Both);
        assert_eq!(g.0.value(gpio_reg::GRER[2]), 1 | (1 << 30));
        g.disarm(94);
        assert_eq!(g.0.value(gpio_reg::GRER[2]), 1);
        assert_eq!(g.0.value(gpio_reg::GFER[2]), 1);
        g.arm(100, Edge::Rising); // bank 3 lives at +0x100
        assert_eq!(g.0.value(0x130), 1 << 4);
    }

    #[test]
    fn take_events_clears_what_it_read() {
        let mut g = GpioEdges(MockBus::new());
        g.0.preset(gpio_reg::GEDR[2], 1 << 30);
        assert_eq!(g.take_events(), [0, 0, 1 << 30, 0]);
        assert_eq!(
            g.0.writes(),
            [(gpio_reg::GEDR[2], 1 << 30)],
            "write-1-to-clear, banks with events only"
        );
    }

    #[test]
    fn claimed_sources_are_masked_and_signalled() {
        let mut d = Dispatch::new();
        assert!(d.claim(irq::KEYPAD));
        assert!(!d.claim(irq::KEYPAD), "claimed once");
        assert!(!d.claim(irq::GPIO_X), "GPIO summaries are claimed per pin");
        assert!(!d.claim(40));
        let mut intc = Intc(MockBus::new());
        intc.0
            .preset(intc_reg::ICMR, (1 << irq::KEYPAD) | (1 << irq::MMC));
        let mut gpio = GpioEdges(MockBus::new());
        let mut woken = Vec::new();
        d.handle(
            &mut intc,
            &mut gpio,
            (1 << irq::KEYPAD) | (1 << irq::MMC),
            &mut |n| woken.push(n),
        );
        assert_eq!(woken, [irq::KEYPAD], "only the claimed source is passed on");
        assert_eq!(intc.0.value(intc_reg::ICMR), 0, "both masked");
        assert!(d.take_signal(irq::KEYPAD));
        assert!(!d.take_signal(irq::KEYPAD), "signal consumed");
        assert_eq!(d.unclaimed, 1 << irq::MMC, "unowned, recorded");
        assert!(
            gpio.0.log().is_empty(),
            "no GPIO access without a GPIO interrupt"
        );
    }

    #[test]
    fn gpio_pins_are_demultiplexed() {
        let mut d = Dispatch::new();
        assert!(d.claim_gpio(94));
        assert!(!d.claim_gpio(94));
        assert!(!d.claim_gpio(121));
        let mut intc = Intc(MockBus::new());
        let mut gpio = GpioEdges(MockBus::new());
        gpio.0
            .preset(gpio_reg::GEDR[2], (1 << 30) | (1 << 1))
            .preset(gpio_reg::GFER[2], (1 << 30) | (1 << 1));
        let mut woken = Vec::new();
        d.handle(&mut intc, &mut gpio, 1 << irq::GPIO_X, &mut |n| {
            woken.push(n)
        });
        assert_eq!(woken, [gpio_line(94)], "the claimed pin, by its line id");
        assert!(d.any_signalled());
        assert!(d.take_gpio_signal(94));
        assert!(!d.take_gpio_signal(94));
        assert!(!d.any_signalled());
        assert_eq!(d.unclaimed_gpio[2], 1 << 1, "GPIO 65 had no owner");
        assert_eq!(gpio.0.value(gpio_reg::GFER[2]), 0, "both pins disarmed");
        assert!(
            intc.0.log().is_empty(),
            "the summary interrupt stays enabled"
        );
    }

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
