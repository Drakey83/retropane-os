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

/// Byte-wide access to a register bank (e.g. data FIFOs that must be read
/// a byte at a time). Offsets are byte offsets.
pub trait RegisterBus8 {
    /// Reads the 8-bit register at `offset`.
    fn read8(&mut self, offset: usize) -> u8;

    /// Writes `value` to the 8-bit register at `offset`.
    fn write8(&mut self, offset: usize, value: u8);
}

/// Storage addressed in 512-byte blocks (an SD card, a disk image in tests).
pub trait BlockDevice {
    /// Reads `buf.len() / 512` blocks starting at `lba` into `buf`, whose
    /// length is a whole number of blocks.
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError>;
    /// Writes `buf` (a whole number of blocks) starting at `lba`.
    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError>;
}

impl<T: BlockDevice + ?Sized> BlockDevice for &mut T {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
        (**self).read_blocks(lba, buf)
    }
    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
        (**self).write_blocks(lba, buf)
    }
}

/// A DMA engine moving data between memory and a device FIFO, paced by the
/// device's request line. Two phases, because the transfer must be ready
/// before the device starts: arm it, start the device, then poll `done`
/// (non-blocking, so the device driver can watch the device's own errors
/// at the same time) or `abort`.
pub trait DmaEngine {
    /// Arms a transfer of `buf.len()` bytes from the FIFO at physical
    /// address `fifo` into `buf`. `buf` must not be touched until `finish`
    /// or `abort`. False if it can't (too long, already armed).
    fn arm_from_device(&mut self, request: u8, fifo: u32, buf: &mut [u8]) -> bool;
    /// Arms a transfer of `buf` into the FIFO at physical address `fifo`.
    fn arm_to_device(&mut self, request: u8, fifo: u32, buf: &[u8]) -> bool;
    /// `None` while the armed transfer runs; once it has ended, `Some(true)`
    /// (or `Some(false)` on a bus error) and the engine is disarmed.
    fn done(&mut self) -> Option<bool>;
    /// Stops the armed transfer (the device failed, say) and disarms.
    fn abort(&mut self);
}

/// No DMA: every arm is refused, so drivers use programmed I/O.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoDma;

impl DmaEngine for NoDma {
    fn arm_from_device(&mut self, _: u8, _: u32, _: &mut [u8]) -> bool {
        false
    }
    fn arm_to_device(&mut self, _: u8, _: u32, _: &[u8]) -> bool {
        false
    }
    fn done(&mut self) -> Option<bool> {
        Some(false)
    }
    fn abort(&mut self) {}
}

/// Why a block transfer failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockError {
    /// The block is past the end of the device.
    OutOfRange,
    /// The device reported an error (details in its own log, if any).
    Device,
    /// The device is write-protected.
    ReadOnly,
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

/// An I2C bus master (7-bit addresses).
pub trait I2cBus {
    /// Writes `data` to the device at `addr` in one transaction (start,
    /// address, data, stop). An empty `data` only checks the device answers.
    fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), I2cError>;
}

/// Why an I2C transaction failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum I2cError {
    /// No device acknowledged the address.
    NoDevice,
    /// The device refused a data byte.
    Nak,
    /// The bus never finished (held, or lost to another master).
    Bus,
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

    /// The largest value the field holds.
    #[must_use]
    pub const fn max(self) -> u32 {
        u32::MAX >> (32 - self.width as u32)
    }

    /// `reg` with this field replaced by `value`.
    ///
    /// # Panics
    /// In debug builds (and host tests), if `value` doesn't fit in the field:
    /// a silently truncated register value is a driver bug. Use
    /// [`try_insert`](Self::try_insert) where the value comes from outside.
    #[must_use]
    pub const fn insert(self, reg: u32, value: u32) -> u32 {
        debug_assert!(value <= self.max(), "value does not fit in the field");
        (reg & !self.mask()) | ((value << self.shift) & self.mask())
    }

    /// `reg` with this field replaced by `value`, or `None` if `value`
    /// doesn't fit.
    #[must_use]
    pub const fn try_insert(self, reg: u32, value: u32) -> Option<u32> {
        if value <= self.max() {
            Some((reg & !self.mask()) | (value << self.shift))
        } else {
            None
        }
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
    #[should_panic(expected = "does not fit")]
    fn insert_refuses_values_that_do_not_fit() {
        let _ = Field::new(0, 2).insert(0, 0b111);
    }

    #[test]
    fn try_insert_checks_the_range() {
        let f = Field::new(4, 2);
        assert_eq!(f.max(), 3);
        assert_eq!(f.try_insert(0, 3), Some(0x30));
        assert_eq!(f.try_insert(0, 4), None);
        assert_eq!(Field::new(0, 32).try_insert(0, u32::MAX), Some(u32::MAX));
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
