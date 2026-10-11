//! RetroPane hardware abstraction layer: traits.
//!
//! Drivers depend on these traits, never on a concrete board. On the device the
//! traits are implemented by `rp-hal-pxa27x`; in host unit tests by
//! `rp-hal-mock`, which records every register access so tests can assert the
//! exact sequence a driver performed (see DESIGN.md §3.1 and §12).

#![no_std]
#![forbid(unsafe_code)]

/// A bank of 32-bit memory-mapped registers, addressed by byte offset from the
/// bank's base.
///
/// Offsets must be 4-byte aligned. Implementations decide what an access *means*
/// (a volatile load on hardware, a recorded event in tests).
pub trait RegisterBus {
    /// Reads the register at `offset`.
    fn read32(&mut self, offset: usize) -> u32;

    /// Writes `value` to the register at `offset`.
    fn write32(&mut self, offset: usize, value: u32);

    /// Read-modify-write: replaces the bits of `field` in the register at
    /// `offset` with `value`, leaving all other bits untouched.
    fn modify(&mut self, offset: usize, field: Field, value: u32) {
        let current = self.read32(offset);
        self.write32(offset, field.insert(current, value));
    }

    /// Reads just the bits of `field` from the register at `offset`.
    fn read_field(&mut self, offset: usize, field: Field) -> u32 {
        field.extract(self.read32(offset))
    }
}

/// A bank of 16-bit memory-mapped registers (e.g. the X51v CPLD), addressed
/// by byte offset. Offsets must be 2-byte aligned.
pub trait RegisterBus16 {
    /// Reads the 16-bit register at `offset`.
    fn read16(&mut self, offset: usize) -> u16;

    /// Writes `value` to the 16-bit register at `offset`.
    fn write16(&mut self, offset: usize, value: u16);
}

/// A digital output pin.
pub trait OutputPin {
    /// Drives the pin high.
    fn set_high(&mut self);
    /// Drives the pin low.
    fn set_low(&mut self);
}

/// A digital input pin.
pub trait InputPin {
    /// Whether the pin currently reads high.
    fn is_high(&mut self) -> bool;
}

/// A full-duplex byte-wide SPI bus (chip select handled separately).
pub trait SpiBus {
    /// Sends `byte` and returns the byte received in the same frame.
    fn transfer(&mut self, byte: u8) -> u8;
}

/// A contiguous bit field inside a 32-bit register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    shift: u8,
    width: u8,
}

impl Field {
    /// A field of `width` bits starting at bit `shift`.
    ///
    /// # Panics
    /// If the field is empty or extends past bit 31. Fields are normally
    /// `const`, so this fails at compile time.
    #[must_use]
    pub const fn new(shift: u8, width: u8) -> Self {
        assert!(width >= 1, "field must be at least one bit wide");
        assert!(shift as u32 + width as u32 <= 32, "field exceeds 32 bits");
        Self { shift, width }
    }

    /// A single-bit field.
    #[must_use]
    pub const fn bit(n: u8) -> Self {
        Self::new(n, 1)
    }

    /// The field's bits, in place.
    #[must_use]
    pub const fn mask(self) -> u32 {
        (u32::MAX >> (32 - self.width as u32)) << self.shift
    }

    /// The field's value within `reg`, shifted down to bit 0.
    #[must_use]
    pub const fn extract(self, reg: u32) -> u32 {
        (reg & self.mask()) >> self.shift
    }

    /// `reg` with this field replaced by `value`. Bits of `value` that don't fit
    /// in the field are discarded.
    #[must_use]
    pub const fn insert(self, reg: u32, value: u32) -> u32 {
        (reg & !self.mask()) | ((value << self.shift) & self.mask())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_covers_exactly_the_field() {
        assert_eq!(Field::new(4, 3).mask(), 0b0111_0000);
        assert_eq!(Field::bit(31).mask(), 0x8000_0000);
        assert_eq!(Field::new(0, 32).mask(), u32::MAX);
    }

    #[test]
    fn extract_and_insert_round_trip() {
        let f = Field::new(8, 4);
        let reg = f.insert(0xFFFF_FFFF, 0x5);
        assert_eq!(reg, 0xFFFF_F5FF);
        assert_eq!(f.extract(reg), 0x5);
    }

    #[test]
    fn insert_discards_overflowing_bits() {
        let f = Field::new(0, 2);
        assert_eq!(f.insert(0, 0b111), 0b11);
    }

    struct Fake(u32);
    impl RegisterBus for Fake {
        fn read32(&mut self, _: usize) -> u32 {
            self.0
        }
        fn write32(&mut self, _: usize, v: u32) {
            self.0 = v;
        }
    }

    #[test]
    fn modify_preserves_other_bits() {
        let mut bus = Fake(0xA5A5_A5A5);
        bus.modify(0, Field::new(4, 4), 0x0);
        assert_eq!(bus.0, 0xA5A5_A505);
        assert_eq!(bus.read_field(0, Field::new(0, 4)), 0x5);
    }
}
