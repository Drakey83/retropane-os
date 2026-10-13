//! PXA27x GPIO controller (docs/hw/touch.md, "PXA27x GPIO").
//!
//! [`Gpio`] owns the register block; [`GpioPin`] handles borrow it through a
//! `RefCell` so several pins can be handed to different drivers at once.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::cell::RefCell;

use rp_hal_api::{InputPin, OutputPin, RegisterBus};

/// Highest GPIO number on the PXA27x.
pub const MAX_PIN: u8 = 120;

#[derive(Clone, Copy)]
enum Kind {
    Level,
    Direction,
    Set,
    Clear,
}

/// Offset of the register of `kind` covering `pin`, and the pin's bit.
fn reg(kind: Kind, pin: u8) -> (usize, u32) {
    assert!(pin <= MAX_PIN, "GPIO {pin} doesn't exist");
    let bank = usize::from(pin / 32);
    let (low_base, bank3) = match kind {
        Kind::Level => (0x00, 0x100),
        Kind::Direction => (0x0C, 0x10C),
        Kind::Set => (0x18, 0x118),
        Kind::Clear => (0x24, 0x124),
    };
    let off = if bank < 3 { low_base + 4 * bank } else { bank3 };
    (off, 1 << (pin % 32))
}

/// Offset of the function register covering `pin`, and its field's shift.
fn function_field(pin: u8) -> (usize, u32) {
    assert!(pin <= MAX_PIN, "GPIO {pin} doesn't exist");
    (0x54 + 4 * usize::from(pin / 16), u32::from(pin % 16) * 2)
}

/// The GPIO controller.
#[derive(Debug)]
pub struct Gpio<B> {
    regs: B,
}

impl<B: RegisterBus> Gpio<B> {
    /// Takes the GPIO register block.
    pub fn new(regs: B) -> Self {
        Self { regs }
    }

    /// Makes `pin` an output driving `high`. The level is set before the
    /// direction so the pin never glitches.
    pub fn make_output(&mut self, pin: u8, high: bool) {
        self.write(pin, high);
        let (off, bit) = reg(Kind::Direction, pin);
        let v = self.regs.read32(off) | bit;
        self.regs.write32(off, v);
    }

    /// Makes `pin` an input.
    pub fn make_input(&mut self, pin: u8) {
        let (off, bit) = reg(Kind::Direction, pin);
        let v = self.regs.read32(off) & !bit;
        self.regs.write32(off, v);
    }

    /// Drives an output pin.
    pub fn write(&mut self, pin: u8, high: bool) {
        let (off, bit) = reg(if high { Kind::Set } else { Kind::Clear }, pin);
        self.regs.write32(off, bit);
    }

    /// Reads a pin's level.
    pub fn read(&mut self, pin: u8) -> bool {
        let (off, bit) = reg(Kind::Level, pin);
        self.regs.read32(off) & bit != 0
    }

    /// Sets `pin`'s function: 0 a plain GPIO, 1-3 the peripheral functions
    /// the PXA27x manual lists for it. `GAFRn_L/U` at `0x54 + 4 * (pin / 16)`,
    /// two bits a pin. Set the level and direction first (`make_output`) when
    /// taking a pin back as an output, so it never glitches.
    ///
    /// # Panics
    /// If the pin doesn't exist or `function` is over 3.
    pub fn set_function(&mut self, pin: u8, function: u8) {
        assert!(function <= 3, "GPIO function {function} doesn't exist");
        let (off, shift) = function_field(pin);
        let v = self.regs.read32(off) & !(3 << shift) | u32::from(function) << shift;
        self.regs.write32(off, v);
    }

    /// `pin`'s function as set now (0 a plain GPIO).
    pub fn function(&mut self, pin: u8) -> u8 {
        let (off, shift) = function_field(pin);
        (self.regs.read32(off) >> shift & 3) as u8
    }

    /// Gives the register block back.
    pub fn into_inner(self) -> B {
        self.regs
    }
}

/// One pin, sharing the controller with other pins.
pub struct GpioPin<'a, B> {
    gpio: &'a RefCell<Gpio<B>>,
    pin: u8,
}

impl<'a, B: RegisterBus> GpioPin<'a, B> {
    /// A handle for `pin`. Configure direction with [`Gpio::make_output`] /
    /// [`Gpio::make_input`] first.
    pub fn new(gpio: &'a RefCell<Gpio<B>>, pin: u8) -> Self {
        assert!(pin <= MAX_PIN, "GPIO {pin} doesn't exist");
        Self { gpio, pin }
    }
}

impl<B: RegisterBus> OutputPin for GpioPin<'_, B> {
    fn set_high(&mut self) {
        self.gpio.borrow_mut().write(self.pin, true);
    }
    fn set_low(&mut self) {
        self.gpio.borrow_mut().write(self.pin, false);
    }
}

impl<B: RegisterBus> InputPin for GpioPin<'_, B> {
    fn is_high(&mut self) -> bool {
        self.gpio.borrow_mut().read(self.pin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_hal_mock::MockBus;

    #[test]
    fn register_layout_matches_spec() {
        assert_eq!(reg(Kind::Level, 0), (0x00, 1));
        assert_eq!(reg(Kind::Direction, 24), (0x0C, 1 << 24));
        assert_eq!(reg(Kind::Set, 94), (0x20, 1 << 30));
        assert_eq!(reg(Kind::Clear, 63), (0x28, 1 << 31));
        assert_eq!(reg(Kind::Level, 96), (0x100, 1));
        assert_eq!(reg(Kind::Direction, 120), (0x10C, 1 << 24));
    }

    #[test]
    fn function_fields_match_the_manual() {
        // GPIO24: GAFR0_U bits 17:16; GPIO99: GAFR3_L bits 7:6 (as the
        // board's reset code uses it); GPIO120: GAFR3_U bits 17:16.
        assert_eq!(function_field(24), (0x58, 16));
        assert_eq!(function_field(99), (0x6C, 6));
        assert_eq!(function_field(120), (0x70, 16));
        // Windows Mobile's GAFR0_U (probe 2026-10-11): GPIO24 is SSP1's frame.
        let mut g = Gpio::new(MockBus::new());
        g.regs.preset(0x58, 0x001A_851A);
        assert_eq!(g.function(24), 2);
        g.set_function(24, 0);
        assert_eq!(g.function(24), 0);
        // Only GPIO24's two bits changed.
        assert_eq!(g.into_inner().value(0x58), 0x0018_851A);
    }

    #[test]
    #[should_panic(expected = "doesn't exist")]
    fn rejects_missing_pins() {
        let _ = reg(Kind::Level, 121);
    }

    #[test]
    fn make_output_sets_level_before_direction() {
        let mut g = Gpio::new(MockBus::new());
        g.make_output(24, true);
        assert_eq!(g.into_inner().writes(), [(0x18, 1 << 24), (0x0C, 1 << 24)]);
    }

    #[test]
    fn make_input_clears_only_its_direction_bit() {
        let mut bus = MockBus::new();
        bus.preset(0x14, 0xFFFF_FFFF);
        let mut g = Gpio::new(bus);
        g.make_input(94);
        assert_eq!(g.into_inner().value(0x14), !(1 << 30));
    }

    #[test]
    fn pins_share_the_controller() {
        let mut bus = MockBus::new();
        bus.preset(0x08, 1 << 30); // GPIO94 high
        let gpio = RefCell::new(Gpio::new(bus));
        let mut cs = GpioPin::new(&gpio, 24);
        let mut pen = GpioPin::new(&gpio, 94);
        cs.set_low();
        assert!(pen.is_high());
        cs.set_high();
        let w = gpio.into_inner().into_inner().writes();
        assert_eq!(w, [(0x24, 1 << 24), (0x18, 1 << 24)]);
    }
}
