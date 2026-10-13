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
    /// Status (read): bit 1 clear while a CompactFlash card is in.
    pub const STATUS04: usize = 0x04;
    /// Wi-Fi power (bits 3, 13), CompactFlash power stage 2 (bit 12).
    pub const PWR10: usize = 0x10;
    /// Audio amplifier (bit 3, guess).
    pub const PWR14: usize = 0x14;
    /// SD power (bit 0), CompactFlash power stage 1 (bit 1), Wi-Fi (bit 2).
    pub const PWR16: usize = 0x16;
    /// Serial (bit 0), CompactFlash RESET (bit 1, set = held in reset),
    /// backlight enable (bit 2), display (bit 3).
    pub const CTL18: usize = 0x18;
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

/// CompactFlash bits (docs/hw/cf.md "Slot power and reset").
const CF_PRESENT_N: u16 = 0x0002;
const CF_POWER_1: u16 = 0x0002;
const CF_POWER_2: u16 = 0x1000;
const CF_RESET: u16 = 0x0002;
const BACKLIGHT_ENABLE: u16 = 0x0004;
/// Between the two power stages.
const CF_POWER_STEP_US: u32 = 10_000;

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

    /// Whether a CompactFlash card is in the slot (status bit 1 clear).
    pub fn cf_present(&mut self) -> bool {
        self.bus.read16(reg::STATUS04) & CF_PRESENT_N == 0
    }

    /// Switches the CompactFlash slot's power, in Windows Mobile's two
    /// stages: on is `0x16` bit 1, 10 ms, `0x10` bit 12; off the reverse.
    pub fn set_cf_power(&mut self, on: bool, delay: &mut impl Delay) {
        if on {
            self.set(reg::PWR16, CF_POWER_1);
            delay.delay_us(CF_POWER_STEP_US);
            self.set(reg::PWR10, CF_POWER_2);
        } else {
            self.clear(reg::PWR10, CF_POWER_2);
            delay.delay_us(CF_POWER_STEP_US);
            self.clear(reg::PWR16, CF_POWER_1);
        }
    }

    /// Switches the backlight's enable (with the PWM's duty, docs/hw/
    /// backlight.md, it is what lights the panel). Windows Mobile leaves it
    /// on; the registers don't read back, so whoever writes `0x18` first
    /// must set it or the backlight goes off (found on the device).
    pub fn set_backlight_enable(&mut self, on: bool) {
        if on {
            self.set(reg::CTL18, BACKLIGHT_ENABLE);
        } else {
            self.clear(reg::CTL18, BACKLIGHT_ENABLE);
        }
    }

    /// Holds (`true`) or releases the CompactFlash card's RESET line.
    pub fn set_cf_reset(&mut self, held: bool) {
        if held {
            self.set(reg::CTL18, CF_RESET);
        } else {
            self.clear(reg::CTL18, CF_RESET);
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

    /// The CF slot's power follows Windows Mobile's order and leaves the
    /// SD card's and the Wi-Fi's bits in the same registers alone.
    #[test]
    fn cf_power_is_two_stages_and_keeps_other_bits() {
        let mut cpld = Cpld::new(MockBus::new());
        let mut delay = Recorder::default();
        cpld.set_sd_power(true);
        cpld.set(reg::PWR10, 0x2008); // Wi-Fi
        cpld.set_cf_power(true, &mut delay);
        cpld.set_cf_reset(true);
        cpld.set_cf_reset(false);
        cpld.set_cf_power(false, &mut delay);
        assert_eq!(delay.0, [10_000, 10_000]);
        assert_eq!(cpld.shadow(reg::PWR16), 0x0001, "SD power kept");
        assert_eq!(cpld.shadow(reg::PWR10), 0x2008, "Wi-Fi bits kept");
        let bus = cpld.into_inner();
        assert_eq!(
            bus.writes(),
            [
                (reg::PWR16, 0x0001),
                (reg::PWR10, 0x2008),
                (reg::PWR16, 0x0003),
                (reg::PWR10, 0x3008),
                (reg::CTL18, 0x0002),
                (reg::CTL18, 0x0000),
                (reg::PWR10, 0x2008),
                (reg::PWR16, 0x0001),
            ]
        );
    }

    /// The device's bug: resetting the CF card wrote 0x18 whole and the
    /// backlight went off. With the enable set first, the reset keeps it.
    #[test]
    fn cf_reset_keeps_the_backlight_on() {
        let mut cpld = Cpld::new(MockBus::new());
        cpld.set_backlight_enable(true);
        cpld.set_cf_reset(true);
        cpld.set_cf_reset(false);
        let bus = cpld.into_inner();
        assert_eq!(
            bus.writes(),
            [
                (reg::CTL18, 0x0004),
                (reg::CTL18, 0x0006),
                (reg::CTL18, 0x0004)
            ]
        );
    }

    #[test]
    fn cf_presence_is_status_bit_1_clear() {
        let mut bus = MockBus::new();
        bus.script_reads(reg::STATUS04, &[0x00A0, 0x00A2]);
        let mut cpld = Cpld::new(bus);
        assert!(cpld.cf_present());
        assert!(!cpld.cf_present());
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
