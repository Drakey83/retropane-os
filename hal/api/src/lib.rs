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
    /// Arms a transfer of `buf.len()` bytes from `port` into `buf`. `buf`
    /// must not be touched until `done` says it ended, or `abort`. False if
    /// it can't (too long, misaligned, already armed).
    fn arm_from_device(&mut self, port: Port, buf: &mut [u8]) -> bool;
    /// Arms a transfer of `buf` into `port`.
    fn arm_to_device(&mut self, port: Port, buf: &[u8]) -> bool;
    /// `None` while the armed transfer runs; once it has ended, `Some(true)`
    /// (or `Some(false)` on a bus error) and the engine is disarmed.
    fn done(&mut self) -> Option<bool>;
    /// Stops the armed transfer (the device failed, say) and disarms.
    fn abort(&mut self);
}

/// A device FIFO a DMA engine serves: the device's request line, the
/// FIFO's physical address, and the width of each access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Port {
    /// DMA request line.
    pub request: u8,
    /// Physical address of the FIFO register.
    pub addr: u32,
    /// Bytes per FIFO access.
    pub width: Width,
}

/// Bytes per FIFO access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    /// One byte.
    Byte,
    /// Two bytes.
    Half,
    /// Four bytes.
    Word,
}

/// A DMA engine looping a buffer into a device (audio): the CPU refills
/// the part the engine has passed. Positions are byte offsets in the ring.
///
/// The ring is cut into equal **pieces** ([`DmaRing::piece`]). It loops
/// without end until the CPU gives it one ([`DmaRing::end_at`], ADR-0032
/// A6): from then on the engine stops by itself at the end of the piece
/// holding the end, unless the CPU moves the end further first. So a CPU
/// that is late by any amount makes the engine fall silent, never replay
/// an old lap.
pub trait DmaRing {
    /// Starts looping `ring` into `port`. The CPU may write the ring while
    /// it runs, behind the position, then must hand what it wrote to
    /// [`DmaRing::filled`].
    /// `ring` must outlive the loop: call [`DmaRing::stop`] before reusing
    /// it. False if it can't (size, alignment, busy).
    fn start(&mut self, port: Port, ring: &[u8]) -> bool;
    /// Where in the ring the engine reads next, while started (moving, or
    /// stopped at its end).
    fn position(&mut self) -> Option<usize>;
    /// The CPU wrote `part` (a piece of the running ring): make it visible
    /// to the engine (write back the cache).
    fn filled(&mut self, part: &[u8]);
    /// Stops the loop.
    fn stop(&mut self);
    /// Bytes per piece of the running ring (0 if none runs).
    fn piece(&self) -> usize;
    /// The engine has stopped at its end (or short of it, the race in
    /// [`RingEnd::Raced`]): it moves again only through
    /// [`DmaRing::end_at`].
    fn stopped(&mut self) -> bool;
    /// The sound in the ring ends before byte `at` (the first byte not
    /// written for its time): the engine stops at the end of the piece
    /// holding `at`. The caller's part (the rule the engine's correctness
    /// rests on): everything from where the engine reads up to `at` is
    /// written for its time, the rest of `at`'s piece is silence, `at`'s
    /// piece starts past where the engine reads (by the caller's margin),
    /// and ends no further than one ring past the start of the piece the
    /// engine reads. A stopped engine is started again where it stopped.
    fn end_at(&mut self, at: usize) -> RingEnd;
}

/// What [`DmaRing::end_at`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingEnd {
    /// The engine runs on to the new end.
    Running,
    /// It had stopped at the old end (the CPU was late): started again
    /// where it stopped.
    Restarted,
    /// It had already started the old end's piece, so it would have
    /// stopped there short of the new end (the race): it was waited for
    /// (at most a piece) and started again at once.
    Raced,
    /// It is stopped at the new end already: nothing was written past it.
    Stopped,
    /// Not running a ring, an end outside it or breaking the rule, the
    /// registers making no sense, or the race's wait running out: the end
    /// may not hold (counted by the caller, never hidden).
    Failed,
}

/// No DMA: every arm is refused, so drivers use programmed I/O.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoDma;

impl DmaEngine for NoDma {
    fn arm_from_device(&mut self, _: Port, _: &mut [u8]) -> bool {
        false
    }
    fn arm_to_device(&mut self, _: Port, _: &[u8]) -> bool {
        false
    }
    fn done(&mut self) -> Option<bool> {
        Some(false)
    }
    fn abort(&mut self) {}
}

/// Memory-to-memory DMA out of a device's data window: a register that
/// answers at every address of a window (a CompactFlash card's data
/// register, 0x400-0x7FF of its common memory), read `window` bytes at a
/// time with incrementing addresses. No request line: the driver starts it
/// only when the device has the data. Two phases, as [`DmaEngine`].
pub trait WindowDma {
    /// Arms a copy of `buf.len()` bytes into `buf` from the window at
    /// physical address `src`, `window` bytes per pass. False if it can't
    /// (misaligned, busy): the driver then uses the CPU.
    fn arm_from_window(&mut self, src: u32, window: usize, buf: &mut [u8]) -> bool;
    /// `None` while it runs; `Some(true)` when done, `Some(false)` on a
    /// bus error; either way the engine is disarmed.
    fn done(&mut self) -> Option<bool>;
    /// Stops the armed copy and disarms.
    fn abort(&mut self);
}

/// An engine that may be missing: none refuses, so the driver uses the CPU.
impl<T: WindowDma> WindowDma for Option<T> {
    fn arm_from_window(&mut self, src: u32, window: usize, buf: &mut [u8]) -> bool {
        self.as_mut()
            .is_some_and(|d| d.arm_from_window(src, window, buf))
    }
    fn done(&mut self) -> Option<bool> {
        self.as_mut().map_or(Some(false), WindowDma::done)
    }
    fn abort(&mut self) {
        if let Some(d) = self {
            d.abort();
        }
    }
}

impl WindowDma for NoDma {
    fn arm_from_window(&mut self, _: u32, _: usize, _: &mut [u8]) -> bool {
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
    /// A transfer this controller can't do (the PXA27x: one with no data
    /// bytes, measured on the X51v 2026-10-18).
    Unsupported,
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

/// A decimal `u32` at compile time (0 for anything else): build-time
/// knobs read with `option_env!` (a poll pause swept on the device).
#[must_use]
pub const fn parse_u32(s: &str) -> u32 {
    let b = s.as_bytes();
    let mut i = 0;
    let mut v = 0u32;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            return 0;
        }
        v = v * 10 + (b[i] - b'0') as u32;
        i += 1;
    }
    v
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
