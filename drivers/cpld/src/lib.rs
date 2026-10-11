//! Dell Axim X51v board CPLD: power rails and board identification.
//!
//! Written from `docs/hw/cpld.md`. The registers are 16 bits wide on a 2-byte
//! stride. Whether they read back isn't known yet, so the driver keeps a
//! shadow copy of everything it writes and never relies on reading power bits,
//! the same approach the 2010 AxDroid port used.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_hal_api::RegisterBus16;

/// Register offsets.
pub mod reg {
    /// Wi-Fi power (bit 6).
    pub const PWR0: usize = 0x00;
    /// Wi-Fi power (bits 3, 13).
    pub const PWR10: usize = 0x10;
    /// Audio amplifier (bit 3, guess).
    pub const PWR14: usize = 0x14;
    /// SD power (bit 0), Wi-Fi (bit 2).
    pub const PWR16: usize = 0x16;
    /// Wi-Fi (bit 0), boot bits (1, 2), USB? (bit 3).
    pub const PWR1C: usize = 0x1C;
    /// LCD type readout: write bit 3 to enable, read bits 1:0.
    pub const LCD_ID: usize = 0x1E;
}

const REGS: usize = 16;
const LCD_ID_ENABLE: u16 = 0x0008;

/// The panel fitted, as the CPLD reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    /// 3.7" VGA, Sharp (X50v/X51v).
    Sharp37Vga,
    /// 3.5" QVGA, Sharp (X50/X51).
    Sharp35Qvga,
    /// 3.7" VGA, Samsung (X50v/X51v).
    Samsung37Vga,
    /// 3.5" QVGA, Sharp (second ID).
    Sharp35QvgaAlt,
}

impl Panel {
    /// Decodes `LCD_ID & 3`.
    #[must_use]
    pub const fn from_id(id: u16) -> Self {
        match id & 3 {
            0 => Self::Sharp37Vga,
            1 => Self::Sharp35Qvga,
            2 => Self::Samsung37Vga,
            _ => Self::Sharp35QvgaAlt,
        }
    }

    /// Whether this is a VGA (480 × 640) panel.
    #[must_use]
    pub const fn is_vga(self) -> bool {
        matches!(self, Self::Sharp37Vga | Self::Samsung37Vga)
    }

    /// A short description.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Sharp37Vga => "3.7\" Sharp VGA",
            Self::Sharp35Qvga | Self::Sharp35QvgaAlt => "3.5\" Sharp QVGA",
            Self::Samsung37Vga => "3.7\" Samsung VGA",
        }
    }
}

/// Something that can wait: the driver needs short delays in sequences.
pub trait Delay {
    /// Busy-waits or sleeps for `us` microseconds.
    fn delay_us(&mut self, us: u32);
}

/// The board CPLD.
#[derive(Debug)]
pub struct Cpld<B> {
    bus: B,
    shadow: [u16; REGS],
}

impl<B: RegisterBus16> Cpld<B> {
    /// Takes the CPLD register window. The shadow starts at zero.
    pub fn new(bus: B) -> Self {
        Self {
            bus,
            shadow: [0; REGS],
        }
    }

    fn write(&mut self, offset: usize, value: u16) {
        self.shadow[offset / 2] = value;
        self.bus.write16(offset, value);
    }

    /// Sets bits in a register (via the shadow copy).
    pub fn set(&mut self, offset: usize, bits: u16) {
        let v = self.shadow[offset / 2] | bits;
        self.write(offset, v);
    }

    /// Clears bits in a register (via the shadow copy).
    pub fn clear(&mut self, offset: usize, bits: u16) {
        let v = self.shadow[offset / 2] & !bits;
        self.write(offset, v);
    }

    /// The last value written to a register.
    #[must_use]
    pub fn shadow(&self, offset: usize) -> u16 {
        self.shadow[offset / 2]
    }

    /// Reads the fitted panel type (sequence in docs/hw/cpld.md).
    pub fn panel(&mut self, delay: &mut impl Delay) -> Panel {
        self.set(reg::LCD_ID, LCD_ID_ENABLE);
        delay.delay_us(1000);
        let id = self.bus.read16(reg::LCD_ID);
        delay.delay_us(1000);
        self.clear(reg::LCD_ID, LCD_ID_ENABLE);
        Panel::from_id(id)
    }

    /// Switches SD card power.
    pub fn set_sd_power(&mut self, on: bool) {
        if on {
            self.set(reg::PWR16, 0x0001);
        } else {
            self.clear(reg::PWR16, 0x0001);
        }
    }

    /// Gives the register window back.
    pub fn into_inner(self) -> B {
        self.bus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_hal_mock::MockBus;

    #[derive(Default)]
    struct Recorder(Vec<u32>);
    impl Delay for Recorder {
        fn delay_us(&mut self, us: u32) {
            self.0.push(us);
        }
    }

    #[test]
    fn panel_ids_decode_per_spec() {
        assert_eq!(Panel::from_id(0), Panel::Sharp37Vga);
        assert_eq!(Panel::from_id(1), Panel::Sharp35Qvga);
        assert_eq!(Panel::from_id(2), Panel::Samsung37Vga);
        assert_eq!(Panel::from_id(3), Panel::Sharp35QvgaAlt);
        assert!(Panel::from_id(0).is_vga() && Panel::from_id(2).is_vga());
        assert!(!Panel::from_id(1).is_vga() && !Panel::from_id(3).is_vga());
        assert_eq!(Panel::from_id(0xFFFE), Panel::Samsung37Vga); // only bits 1:0 count
    }

    #[test]
    fn panel_detection_follows_the_sequence() {
        let mut bus = MockBus::new();
        bus.script_reads(reg::LCD_ID, &[0x000A]); // enable bit echoed + Samsung
        let mut cpld = Cpld::new(bus);
        let mut delay = Recorder::default();
        assert_eq!(cpld.panel(&mut delay), Panel::Samsung37Vga);
        assert_eq!(delay.0, [1000, 1000]);
        let bus = cpld.into_inner();
        assert_eq!(bus.writes(), [(reg::LCD_ID, 0x0008), (reg::LCD_ID, 0x0000)]);
    }

    #[test]
    fn set_and_clear_use_the_shadow_not_reads() {
        let mut bus = MockBus::new();
        bus.preset(reg::PWR16, 0xFFFF); // whatever the hardware reads as
        let mut cpld = Cpld::new(bus);
        cpld.set(reg::PWR16, 0x0004);
        cpld.set_sd_power(true);
        cpld.clear(reg::PWR16, 0x0004);
        assert_eq!(cpld.shadow(reg::PWR16), 0x0001);
        let bus = cpld.into_inner();
        assert!(
            bus.log()
                .iter()
                .all(|a| matches!(a, rp_hal_mock::Access::Write { .. }))
        );
        assert_eq!(
            bus.writes(),
            [(reg::PWR16, 4), (reg::PWR16, 5), (reg::PWR16, 1)]
        );
    }

    #[test]
    fn sd_power_off() {
        let mut cpld = Cpld::new(MockBus::new());
        cpld.set_sd_power(true);
        cpld.set_sd_power(false);
        assert_eq!(cpld.shadow(reg::PWR16), 0);
    }
}
