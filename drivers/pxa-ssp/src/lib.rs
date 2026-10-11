//! PXA27x SSP serial port in SPI mode (docs/hw/touch.md, "PXA27x SSP").

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_hal_api::{Field, RegisterBus, SpiBus};

/// Register offsets.
pub mod reg {
    /// Control 0.
    pub const SSCR0: usize = 0x00;
    /// Control 1.
    pub const SSCR1: usize = 0x04;
    /// Status.
    pub const SSSR: usize = 0x08;
    /// Data FIFO.
    pub const SSDR: usize = 0x10;
}

const SSCR0_DSS: Field = Field::new(0, 4);
const SSCR0_FRF: Field = Field::new(4, 2);
const SSCR0_SSE: Field = Field::bit(7);
const SSCR0_SCR: Field = Field::new(8, 12);
const SSSR_TNF: Field = Field::bit(2);
const SSSR_RNE: Field = Field::bit(3);

/// SSP input clock.
pub const SSP_CLOCK_HZ: u32 = 13_000_000;

/// Slowest serial clock the SSP can make: 13 MHz / 4096.
pub const SSP_MIN_HZ: u32 = SSP_CLOCK_HZ.div_ceil(4096);

/// The serial clock rate divisor (SCR) for at most `hz`. Rates above
/// [`SSP_CLOCK_HZ`] give the fastest clock, which still honours "at most".
///
/// # Panics
/// In debug builds, if `hz` is below [`SSP_MIN_HZ`]: no divisor can make a
/// clock that slow, and silently running faster than asked could break the
/// device on the other end.
#[must_use]
pub fn scr_for(hz: u32) -> u32 {
    debug_assert!(hz >= SSP_MIN_HZ, "SPI clock below the SSP's minimum");
    let hz = hz.clamp(SSP_MIN_HZ, SSP_CLOCK_HZ);
    SSP_CLOCK_HZ.div_ceil(hz) - 1
}

/// An SSP port configured for 8-bit SPI mode 0.
#[derive(Debug)]
pub struct Ssp<B> {
    regs: B,
}

impl<B: RegisterBus> Ssp<B> {
    /// Takes the port's registers. Its clock must already be enabled.
    pub fn new(regs: B) -> Self {
        Self { regs }
    }

    /// Configures 8-bit Motorola SPI, mode 0, at most `max_hz`, and enables it.
    pub fn configure_spi(&mut self, max_hz: u32) {
        let sscr0 = SSCR0_DSS.insert(0, 7);
        let sscr0 = SSCR0_FRF.insert(sscr0, 0);
        let sscr0 = SSCR0_SCR.insert(sscr0, scr_for(max_hz));
        self.regs.write32(reg::SSCR0, sscr0);
        self.regs.write32(reg::SSCR1, 0);
        self.regs.write32(reg::SSCR0, SSCR0_SSE.insert(sscr0, 1));
    }

    /// Gives the registers back.
    pub fn into_inner(self) -> B {
        self.regs
    }
}

impl<B: RegisterBus> SpiBus for Ssp<B> {
    fn transfer(&mut self, byte: u8) -> u8 {
        while self.regs.read_field(reg::SSSR, SSSR_TNF) == 0 {}
        self.regs.write32(reg::SSDR, u32::from(byte));
        while self.regs.read_field(reg::SSSR, SSSR_RNE) == 0 {}
        self.regs.read32(reg::SSDR) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "below the SSP's minimum")]
    fn refuses_impossibly_slow_clocks() {
        let _ = scr_for(SSP_MIN_HZ - 1);
    }
    use rp_hal_mock::MockBus;

    #[test]
    fn clock_divisor() {
        assert_eq!(scr_for(13_000_000), 0);
        assert_eq!(scr_for(2_000_000), 6); // 1.86 MHz
        assert_eq!(scr_for(2_600_000), 4); // exactly 2.6 MHz
        assert_eq!(scr_for(SSP_MIN_HZ), 0xFFF);
        assert_eq!(scr_for(50_000_000), 0);
    }

    #[test]
    fn configure_sets_format_then_enables() {
        let mut ssp = Ssp::new(MockBus::new());
        ssp.configure_spi(2_600_000);
        let w = ssp.into_inner().writes();
        assert_eq!(w.len(), 3);
        assert_eq!(w[0], (reg::SSCR0, (4 << 8) | 7));
        assert_eq!(w[1], (reg::SSCR1, 0));
        assert_eq!(w[2], (reg::SSCR0, (4 << 8) | 0x80 | 7));
    }

    #[test]
    fn transfer_waits_for_fifos() {
        let mut bus = MockBus::new();
        // TNF: busy once, then ready; RNE: empty twice, then data.
        bus.script_reads(reg::SSSR, &[0, 0x4, 0, 0, 0x8]);
        bus.script_reads(reg::SSDR, &[0x5A]);
        let mut ssp = Ssp::new(bus);
        assert_eq!(ssp.transfer(0xA5), 0x5A);
        assert_eq!(ssp.into_inner().writes(), [(reg::SSDR, 0xA5)]);
    }
}
