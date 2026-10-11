//! Driver for the PXA27x UARTs.
//!
//! The PXA27x UARTs are 16550-compatible with registers on a 4-byte stride,
//! plus PXA-specific bits: IER.UUE must be set to enable the unit
//! (PXA27x Developer's Manual, UART chapter). The UART clock is 14.7456 MHz.
//!
//! This is RetroPane's earliest debug channel. On real hardware the FFUART is
//! reachable through the optional serial cable (DESIGN.md §10b); in the
//! emulator it's QEMU's `-serial` port.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_hal_api::{Field, RegisterBus};

/// Register offsets (4-byte stride).
pub mod reg {
    /// Transmit holding (write) / receive buffer (read) / divisor low (DLAB=1).
    pub const THR: usize = 0x00;
    /// Interrupt enable / divisor high (DLAB=1).
    pub const IER: usize = 0x04;
    /// Line control.
    pub const LCR: usize = 0x0C;
    /// Line status.
    pub const LSR: usize = 0x14;
}

/// IER: UART unit enable (PXA-specific).
pub const IER_UUE: Field = Field::bit(6);
/// LCR: divisor latch access.
pub const LCR_DLAB: Field = Field::bit(7);
/// LCR: word length (0b11 = 8 bits).
pub const LCR_WLS: Field = Field::new(0, 2);
/// LSR: transmit holding register empty.
pub const LSR_TDRQ: Field = Field::bit(5);

/// UART input clock.
pub const UART_CLOCK_HZ: u32 = 14_745_600;

/// Divisor for `baud`, rounded to nearest.
///
/// # Panics
/// If `baud` is zero or so high/low that the divisor doesn't fit in 16 bits.
#[must_use]
pub const fn divisor(baud: u32) -> u16 {
    assert!(baud > 0, "baud must be non-zero");
    let div = (UART_CLOCK_HZ + 8 * baud) / (16 * baud);
    assert!(div >= 1 && div <= 0xFFFF, "baud rate out of range");
    div as u16
}

/// How many times [`PxaUart::write_byte`] polls for transmitter space before
/// declaring the UART dead. One byte at 115200 baud takes 87 µs; this is
/// milliseconds of polling, so a working UART never gets near it.
pub const TX_POLL_LIMIT: u32 = 100_000;

/// A PXA27x UART on some register bus.
#[derive(Debug)]
pub struct PxaUart<B> {
    bus: B,
    /// Cleared when the transmitter never became ready: on a real X51v the
    /// FFUART may be unclocked or unconnected, and debug output must never
    /// hang the system. Further output is dropped without polling.
    alive: bool,
}

impl<B: RegisterBus> PxaUart<B> {
    /// Takes ownership of the UART's register bank. Call [`init`](Self::init)
    /// before use unless firmware already configured it.
    pub fn new(bus: B) -> Self {
        Self { bus, alive: true }
    }

    /// Configures `baud`, 8N1, interrupts off, unit enabled.
    pub fn init(&mut self, baud: u32) {
        let div = divisor(baud);
        self.bus.write32(reg::IER, 0);
        self.bus.write32(reg::LCR, LCR_DLAB.mask());
        self.bus.write32(reg::THR, u32::from(div & 0xFF));
        self.bus.write32(reg::IER, u32::from(div >> 8));
        self.bus.write32(reg::LCR, LCR_WLS.insert(0, 0b11));
        self.bus.write32(reg::IER, IER_UUE.mask());
    }

    /// Sends one byte, waiting (boundedly) for space in the transmitter. If
    /// the transmitter never becomes ready, the UART is marked dead and this
    /// and all later bytes are dropped.
    pub fn write_byte(&mut self, byte: u8) {
        if !self.alive {
            return;
        }
        let mut polls = 0;
        while self.bus.read_field(reg::LSR, LSR_TDRQ) == 0 {
            polls += 1;
            if polls == TX_POLL_LIMIT {
                self.alive = false;
                return;
            }
        }
        self.bus.write32(reg::THR, u32::from(byte));
    }

    /// Whether the transmitter has been responding.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.alive
    }

    /// Gives the register bank back.
    pub fn into_inner(self) -> B {
        self.bus
    }
}

impl<B: RegisterBus> core::fmt::Write for PxaUart<B> {
    /// Writes `s`, translating `\n` to `\r\n` for terminals.
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for b in s.bytes() {
            if b == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(b);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt::Write;
    use rp_hal_mock::MockBus;

    fn ready_bus() -> MockBus {
        let mut bus = MockBus::new();
        bus.preset(reg::LSR, LSR_TDRQ.mask());
        bus
    }

    #[test]
    fn divisor_for_common_rates() {
        assert_eq!(divisor(115_200), 8);
        assert_eq!(divisor(9_600), 96);
        assert_eq!(divisor(921_600), 1);
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn divisor_rejects_impossible_rates() {
        let _ = divisor(10_000_000);
    }

    #[test]
    fn init_programs_divisor_then_8n1_then_enables_unit() {
        let mut uart = PxaUart::new(MockBus::new());
        uart.init(115_200);
        assert_eq!(
            uart.into_inner().writes(),
            vec![
                (reg::IER, 0),
                (reg::LCR, 0x80), // DLAB on
                (reg::THR, 8),    // divisor low
                (reg::IER, 0),    // divisor high
                (reg::LCR, 0x03), // 8N1, DLAB off
                (reg::IER, 0x40), // UUE
            ]
        );
    }

    #[test]
    fn a_dead_transmitter_never_hangs() {
        // LSR reads 0 forever: an unclocked or missing UART.
        let mut uart = PxaUart::new(MockBus::new());
        write!(uart, "hello").unwrap();
        assert!(!uart.is_alive());
        let bus = uart.into_inner();
        // Polled up to the limit once, then gave up on every later byte.
        assert_eq!(bus.log().len(), TX_POLL_LIMIT as usize);
        assert!(bus.writes().is_empty());
    }

    #[test]
    fn write_byte_waits_for_transmitter() {
        let mut bus = ready_bus();
        bus.script_reads(reg::LSR, &[0, 0]); // busy twice, then ready
        let mut uart = PxaUart::new(bus);
        uart.write_byte(b'A');
        let bus = uart.into_inner();
        let lsr_reads = bus
            .log()
            .iter()
            .filter(|a| {
                matches!(
                    a,
                    rp_hal_mock::Access::Read {
                        offset: reg::LSR,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(lsr_reads, 3);
        assert_eq!(bus.writes(), vec![(reg::THR, u32::from(b'A'))]);
    }

    #[test]
    fn newlines_become_crlf() {
        let mut uart = PxaUart::new(ready_bus());
        uart.write_str("hi\n").unwrap();
        let sent: Vec<u32> = uart
            .into_inner()
            .writes()
            .into_iter()
            .map(|(_, v)| v)
            .collect();
        assert_eq!(sent, b"hi\r\n".map(u32::from).to_vec());
    }
}
