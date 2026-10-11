//! Fake hardware for host-side unit tests.
//!
//! [`MockBus`] behaves like a bank of registers and records every access, so a
//! test can assert *exactly* what a driver did, in order. Reads can be scripted
//! to simulate hardware that changes state (status bits, FIFOs). Recorded MMIO
//! traces from the real device can be replayed the same way (DESIGN.md §8.2,
//! §12.2).
//!
//! This crate uses `std` and must only ever be a `[dev-dependencies]` entry;
//! `cargo xtask lint` enforces that.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, VecDeque};

use rp_hal_api::{RegisterBus, RegisterBus8, RegisterBus16};

/// One recorded register access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// A read of `offset` that returned `value`.
    Read {
        /// Register offset.
        offset: usize,
        /// Value returned to the driver.
        value: u32,
    },
    /// A write of `value` to `offset`.
    Write {
        /// Register offset.
        offset: usize,
        /// Value written by the driver.
        value: u32,
    },
}

/// A recording, scriptable register bank.
#[derive(Debug, Default)]
pub struct MockBus {
    regs: BTreeMap<usize, u32>,
    scripted: BTreeMap<usize, VecDeque<u32>>,
    log: Vec<Access>,
}

impl MockBus {
    /// An empty bus: every register reads as zero until written or scripted.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a register's current value without recording an access.
    pub fn preset(&mut self, offset: usize, value: u32) -> &mut Self {
        self.regs.insert(offset, value);
        self
    }

    /// Queues values that successive reads of `offset` return, before falling
    /// back to the register's stored value. Simulates hardware-driven changes
    /// such as a "busy" bit clearing.
    pub fn script_reads(&mut self, offset: usize, values: &[u32]) -> &mut Self {
        self.scripted
            .entry(offset)
            .or_default()
            .extend(values.iter().copied());
        self
    }

    /// Every access so far, in order.
    #[must_use]
    pub fn log(&self) -> &[Access] {
        &self.log
    }

    /// Only the writes, as `(offset, value)` pairs, in order.
    #[must_use]
    pub fn writes(&self) -> Vec<(usize, u32)> {
        self.log
            .iter()
            .filter_map(|a| match *a {
                Access::Write { offset, value } => Some((offset, value)),
                Access::Read { .. } => None,
            })
            .collect()
    }

    /// The register's current stored value.
    #[must_use]
    pub fn value(&self, offset: usize) -> u32 {
        self.regs.get(&offset).copied().unwrap_or(0)
    }

    fn check_aligned16(offset: usize) {
        assert!(
            offset.is_multiple_of(2),
            "unaligned 16-bit register offset {offset:#x}"
        );
    }

    fn check_aligned(offset: usize) {
        assert!(
            offset.is_multiple_of(4),
            "unaligned register offset {offset:#x}"
        );
    }
}

impl RegisterBus for MockBus {
    fn read32(&mut self, offset: usize) -> u32 {
        Self::check_aligned(offset);
        let value = self
            .scripted
            .get_mut(&offset)
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| self.value(offset));
        self.log.push(Access::Read { offset, value });
        value
    }

    fn write32(&mut self, offset: usize, value: u32) {
        Self::check_aligned(offset);
        self.regs.insert(offset, value);
        self.log.push(Access::Write { offset, value });
    }
}

/// 16-bit accesses share the register map and the log; values are widened.
impl RegisterBus16 for MockBus {
    fn read16(&mut self, offset: usize) -> u16 {
        Self::check_aligned16(offset);
        let value = self
            .scripted
            .get_mut(&offset)
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| self.value(offset))
            & 0xFFFF;
        self.log.push(Access::Read { offset, value });
        value as u16
    }

    fn write16(&mut self, offset: usize, value: u16) {
        Self::check_aligned16(offset);
        self.regs.insert(offset, u32::from(value));
        self.log.push(Access::Write {
            offset,
            value: u32::from(value),
        });
    }
}

/// 8-bit accesses share the register map and the log; values are widened.
impl RegisterBus8 for MockBus {
    fn read8(&mut self, offset: usize) -> u8 {
        let value = self
            .scripted
            .get_mut(&offset)
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| self.value(offset))
            & 0xFF;
        self.log.push(Access::Read { offset, value });
        value as u8
    }

    fn write8(&mut self, offset: usize, value: u8) {
        self.regs.insert(offset, u32::from(value));
        self.log.push(Access::Write {
            offset,
            value: u32::from(value),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_hal_api::Field;

    #[test]
    fn records_accesses_in_order() {
        let mut bus = MockBus::new();
        bus.write32(0x10, 7);
        let v = bus.read32(0x10);
        assert_eq!(v, 7);
        assert_eq!(
            bus.log(),
            &[
                Access::Write {
                    offset: 0x10,
                    value: 7
                },
                Access::Read {
                    offset: 0x10,
                    value: 7
                },
            ]
        );
    }

    #[test]
    fn scripted_reads_come_first_then_stored_value() {
        let mut bus = MockBus::new();
        bus.preset(0x4, 0).script_reads(0x4, &[1, 1]);
        assert_eq!(bus.read32(0x4), 1);
        assert_eq!(bus.read32(0x4), 1);
        assert_eq!(bus.read32(0x4), 0);
    }

    #[test]
    fn modify_is_a_read_then_a_write() {
        let mut bus = MockBus::new();
        bus.preset(0x8, 0xFF);
        bus.modify(0x8, Field::new(0, 4), 0);
        assert_eq!(bus.writes(), vec![(0x8, 0xF0)]);
        assert_eq!(bus.log().len(), 2);
    }

    #[test]
    fn sixteen_bit_accesses_are_logged_and_truncated() {
        let mut bus = MockBus::new();
        bus.write16(0x1E, 0x8);
        bus.script_reads(0x16, &[0x1_0005]);
        assert_eq!(bus.read16(0x1E), 0x8);
        assert_eq!(bus.read16(0x16), 0x5);
        assert_eq!(bus.writes(), vec![(0x1E, 0x8)]);
    }

    #[test]
    fn eight_bit_reads_pop_scripted_bytes() {
        let mut bus = MockBus::new();
        bus.script_reads(0x40, &[0x12, 0x134]);
        assert_eq!(bus.read8(0x40), 0x12);
        assert_eq!(bus.read8(0x40), 0x34);
        bus.write8(0x41, 0xAB);
        assert_eq!(bus.writes(), vec![(0x41, 0xAB)]);
    }

    #[test]
    #[should_panic(expected = "unaligned 16-bit")]
    fn rejects_odd_16_bit_offsets() {
        MockBus::new().read16(0x3);
    }

    #[test]
    #[should_panic(expected = "unaligned")]
    fn rejects_unaligned_offsets() {
        MockBus::new().read32(0x2);
    }
}
