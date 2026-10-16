//! Dell Axim X51v board CPLD: power rails and board identification.
//!
//! Written from `docs/hw/cpld.md`. The registers are 16 bits wide on a 2-byte
//! stride. They don't read back (measured, docs/hw/cpld.md: the latches are
//! write-only), so the driver keeps a shadow copy of everything in them
//! and never relies on reading power bits. The shadow is never assumed to
//! start at zero: it comes from what is latched ([`handover`]).

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_hal_api::RegisterBus16;

pub mod handover;

/// Register offsets.
pub mod reg {
    /// Wi-Fi power (bit 6).
    pub const PWR0: usize = 0x00;
    /// Status (read): bit 1 clear while a CompactFlash card is in.
    pub const STATUS04: usize = 0x04;
    /// Bit 1 set by Windows Mobile's reboot just before its board reset
    /// (purpose unknown; docs/hw/cpld.md).
    pub const CTL08: usize = 0x08;
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

/// Registers in the window (offsets `0x00`-`0x1E`).
pub const REGS: usize = 16;
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
const SD_POWER: u16 = 0x0001;
/// From setting the LCD-type readout's enable to reading it. Dell's driver
/// waits ~1 ms (a 1 ms sleep), AxDroid 1 ms before and 1 ms after: copied,
/// not measured (open: the device's shortest working wait).
const PANEL_ID_SETTLE_US: u32 = 1000;
/// Between the two power stages.
const CF_POWER_STEP_US: u32 = 10_000;

/// Something that can wait: the driver needs short delays in sequences.
pub trait Delay {
    /// Busy-waits or sleeps for `us` microseconds.
    fn delay_us(&mut self, us: u32);
}

/// What is latched in the CPLD, by register offset: the driver's only
/// knowledge of it, since the latches don't read back.
pub trait Shadow {
    /// What register `offset` holds.
    fn get(&self, offset: usize) -> u16;
    /// Records that `value` was written to register `offset`.
    fn put(&mut self, offset: usize, value: u16);
}

/// A shadow kept by one driver alone: for a single owner of the CPLD (the
/// flash-installed OS's driver, seeded from [`handover::DEFAULTS`] after it
/// writes them) and host tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Own(pub [u16; REGS]);

impl Shadow for Own {
    fn get(&self, offset: usize) -> u16 {
        self.0[offset / 2]
    }

    fn put(&mut self, offset: usize, value: u16) {
        self.0[offset / 2] = value;
    }
}

/// The board CPLD.
#[derive(Debug)]
pub struct Cpld<B, S> {
    bus: B,
    shadow: S,
}

impl<'a, B: RegisterBus16> Cpld<B, handover::Table<'a>> {
    /// Takes the register window with the shared table of what is latched
    /// in it ([`handover::Table::adopt`]): how every program on the device
    /// takes the CPLD. Says where the shadow came from.
    pub fn adopt(bus: B, table: &'a mut [u32; handover::WORDS]) -> (Self, handover::Source) {
        let (shadow, source) = handover::Table::adopt(table);
        (Self::new(bus, shadow), source)
    }
}

impl<B: RegisterBus16, S: Shadow> Cpld<B, S> {
    /// Takes the CPLD register window, with `shadow` saying what is
    /// latched in it (on the device: [`handover::Table`]).
    pub fn new(bus: B, shadow: S) -> Self {
        Self { bus, shadow }
    }

    /// Every latch change goes through here, out of line: the shadow's
    /// write-through (the shared table, its check) is code worth keeping
    /// once, not at every caller.
    #[inline(never)]
    fn update(&mut self, offset: usize, set: u16, clear: u16) {
        let value = (self.shadow.get(offset) & !clear) | set;
        self.shadow.put(offset, value);
        self.bus.write16(offset, value);
    }

    /// Sets bits in a register (via the shadow copy).
    pub fn set(&mut self, offset: usize, bits: u16) {
        self.update(offset, bits, 0);
    }

    /// Clears bits in a register (via the shadow copy).
    pub fn clear(&mut self, offset: usize, bits: u16) {
        self.update(offset, 0, bits);
    }

    /// Reads a register as the bus returns it: status inputs, never what
    /// was written (diagnostics; the latches are the shadow's).
    pub fn read(&mut self, offset: usize) -> u16 {
        self.bus.read16(offset)
    }

    /// What a register holds, as far as the shadow knows.
    #[must_use]
    pub fn shadow(&self, offset: usize) -> u16 {
        self.shadow.get(offset)
    }

    /// Reads the fitted panel type (sequence in docs/hw/cpld.md): the
    /// readout's enable on, a wait, the read, the enable off. The other
    /// bits in `0x1E` (battery and charger, docs/hw/cpld.md) are kept.
    pub fn panel(&mut self, delay: &mut impl Delay) -> Panel {
        self.set(reg::LCD_ID, LCD_ID_ENABLE);
        delay.delay_us(PANEL_ID_SETTLE_US);
        let id = self.bus.read16(reg::LCD_ID);
        self.clear(reg::LCD_ID, LCD_ID_ENABLE);
        Panel::from_id(id)
    }

    /// Switches SD card power.
    pub fn set_sd_power(&mut self, on: bool) {
        if on {
            self.set(reg::PWR16, SD_POWER);
        } else {
            self.clear(reg::PWR16, SD_POWER);
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
    /// on, and the shadow knows it from the hand-over ([`handover`]).
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

    /// Gives the register window and the shadow back.
    pub fn into_parts(self) -> (B, S) {
        (self.bus, self.shadow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_hal_mock::MockBus;

    /// A shadow of all zeros: what the old driver assumed, for tests that
    /// only follow one register's bits.
    fn zero() -> Own {
        Own([0; REGS])
    }

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
        let mut cpld = Cpld::new(bus, zero());
        let mut delay = Recorder::default();
        assert_eq!(cpld.panel(&mut delay), Panel::Samsung37Vga);
        assert_eq!(delay.0, [PANEL_ID_SETTLE_US], "one wait, before the read");
        let bus = cpld.into_inner();
        assert_eq!(bus.writes(), [(reg::LCD_ID, 0x0008), (reg::LCD_ID, 0x0000)]);
    }

    /// The CF slot's power follows Windows Mobile's order and leaves the
    /// SD card's and the Wi-Fi's bits in the same registers alone.
    #[test]
    fn cf_power_is_two_stages_and_keeps_other_bits() {
        let mut cpld = Cpld::new(MockBus::new(), zero());
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

    /// The device's bug (2026-10-12): resetting the CF card wrote 0x18
    /// whole from a shadow that started at zero, and the backlight went
    /// off. The class: any bit Windows Mobile left on in a register we
    /// write. Seeded from its table, every such bit survives, with nothing
    /// set by hand first.
    #[test]
    fn bits_windows_mobile_left_on_survive_our_writes() {
        let mut wm = [0u32; handover::WORDS];
        wm[1] = 0x8004; // 0x10: notification + serial guesses
        wm[3] = 0x0008; // 0x14: display
        wm[5] = 0x000D; // 0x18: serial, backlight, display
        wm[8] = 0x0003; // 0x1E: battery bits
        let (shadow, src) = handover::Table::adopt(&mut wm);
        assert_eq!(src, handover::Source::Adopted);
        let mut bus = MockBus::new();
        bus.script_reads(reg::LCD_ID, &[0x0008]);
        let mut cpld = Cpld::new(bus, shadow);
        cpld.set_cf_reset(true);
        cpld.set_cf_reset(false);
        cpld.set_cf_power(true, &mut Recorder::default());
        assert_eq!(cpld.panel(&mut Recorder::default()), Panel::Sharp37Vga);
        let bus = cpld.into_inner();
        assert_eq!(
            bus.writes(),
            [
                (reg::CTL18, 0x000F),
                (reg::CTL18, 0x000D),
                (reg::PWR16, 0x0002),
                (reg::PWR10, 0x9004),
                (reg::LCD_ID, 0x000B),
                (reg::LCD_ID, 0x0003),
            ]
        );
    }

    /// With no table to adopt (the emulator, and the flash-installed OS
    /// before its drivers run) the backlight is still known to be on.
    #[test]
    fn defaults_keep_the_backlight_through_a_cf_reset() {
        let mut nothing = [0u32; handover::WORDS];
        let (shadow, _) = handover::Table::adopt(&mut nothing);
        let mut cpld = Cpld::new(MockBus::new(), shadow);
        cpld.set_cf_reset(true);
        cpld.set_cf_reset(false);
        assert_eq!(
            cpld.into_inner().writes(),
            [(reg::CTL18, 0x0006), (reg::CTL18, 0x0004)]
        );
    }

    #[test]
    fn cf_presence_is_status_bit_1_clear() {
        let mut bus = MockBus::new();
        bus.script_reads(reg::STATUS04, &[0x00A0, 0x00A2]);
        let mut cpld = Cpld::new(bus, zero());
        assert!(cpld.cf_present());
        assert!(!cpld.cf_present());
    }

    #[test]
    fn set_and_clear_use_the_shadow_not_reads() {
        let mut bus = MockBus::new();
        bus.preset(reg::PWR16, 0xFFFF); // whatever the hardware reads as
        let mut cpld = Cpld::new(bus, zero());
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
        let mut cpld = Cpld::new(MockBus::new(), zero());
        cpld.set_sd_power(true);
        cpld.set_sd_power(false);
        assert_eq!(cpld.shadow(reg::PWR16), 0);
    }
}
