//! Where the shadow of the CPLD's write-only latches comes from, and where
//! it lives (docs/hw/cpld.md "Shadow at hand-over").
//!
//! The latches don't read back, so whoever writes a register must know
//! every bit already in it. Windows Mobile keeps that knowledge in one table
//! in RAM that all its drivers share: ten 32-bit slots at physical
//! [`TABLE_PA`], slot `i` for register [`WM_SLOT_REGS`]`[i]`. RetroPane's
//! programs share one table too, in the same 40 bytes:
//!
//! - **At HaRET's hand-off** the table is Windows Mobile's. It is checked
//!   ([`seed`]) and, if it passes, its values become the shadow: nothing
//!   Windows Mobile left on is switched off by accident (the backlight
//!   blackout of 2026-10-12 was a shadow that started at zero).
//! - **From then on** it is RetroPane's: [`MAGIC`], the sixteen registers
//!   and a check word, rewritten on every change ([`Table`]). The loader
//!   and every image it starts use the same table, so a CF reset in one
//!   program can't undo the SD power another set.
//! - **When neither is there** (the emulator, a damaged table, and in the
//!   end the flash-installed OS, where no Windows Mobile ran) the shadow is
//!   [`DEFAULTS`]: RetroPane's own known-good state.

use crate::{REGS, Shadow, reg};

/// Physical address of the table (Windows Mobile's, then RetroPane's).
/// Below every RetroPane image (`0xA8008000`) and above HaRET's boot tags
/// (`0xA8000100`); nothing else of RetroPane's uses it (docs/hw/memory-map.md).
pub const TABLE_PA: u32 = 0xA800_1320;
/// The table's size in 32-bit words (Windows Mobile's ten slots; RetroPane's
/// format fits the same 40 bytes).
pub const WORDS: usize = 10;

/// The register in each of Windows Mobile's slots (its drivers' shared
/// set/clear helpers index them in this order, docs/hw/cpld.md).
pub const WM_SLOT_REGS: [usize; WORDS] = [
    0x02,
    reg::PWR10,
    0x12,
    reg::PWR14,
    reg::PWR16,
    reg::CTL18,
    0x1A,
    reg::PWR1C,
    reg::LCD_ID,
    reg::CTL08,
];

/// RetroPane's table starts with this word (`"RPCS"`).
pub const MAGIC: u32 = u32::from_le_bytes(*b"RPCS");

/// RetroPane's known-good state when there is no shadow to adopt: the
/// backlight enable (`0x18` bit 2) and SD card power (`0x16` bit 0), the
/// two bits RetroPane needs on that are confirmed on the device. Every
/// other bit starts off and is set by the driver that owns it. Wi-Fi and
/// the CF slot are off until asked for. The display's sequence bits
/// (`0x12`, `0x14` bit 3, `0x18` bit 3: guesses from Dell's drivers) are
/// not here: at HaRET's hand-off Windows Mobile's display is running and
/// RetroPane never writes those registers; in the flash-installed OS the
/// display driver's own power-up sequence sets them (item 72).
pub const DEFAULTS: [u16; REGS] = {
    let mut v = [0; REGS];
    v[reg::CTL18 / 2] = crate::BACKLIGHT_ENABLE;
    v[reg::PWR16 / 2] = crate::SD_POWER;
    v
};

/// Where a seeded shadow came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// RetroPane's own table (a program before this one wrote it).
    Resumed,
    /// Windows Mobile's table, which passed every check.
    Adopted,
    /// [`DEFAULTS`]: there was no table to trust, for this reason.
    Defaults(Reject),
}

impl Source {
    /// A short description.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Resumed => "RetroPane's table",
            Self::Adopted => "WM's table",
            Self::Defaults(r) => r.name(),
        }
    }
}

/// Why Windows Mobile's table wasn't adopted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    /// A slot's upper half isn't zero: Windows Mobile's helpers only ever
    /// store 16-bit values, so this isn't its table (or was overwritten).
    WideSlot,
    /// The backlight enable is off. HaRET runs with Windows Mobile's
    /// screen lit, so a real table has it on; an all-zero one (the
    /// emulator, the flash-installed OS) fails here.
    BacklightOff,
    /// A bit that is only ever set for a moment is set: the LCD-type
    /// readout enable (`0x1E` bit 3) or the reboot bit (`0x08` bit 1).
    Momentary,
}

impl Reject {
    /// A short description.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::WideSlot => "defaults (WM table: a slot over 16 bits)",
            Self::BacklightOff => "defaults (WM table: backlight off)",
            Self::Momentary => "defaults (WM table: a momentary bit on)",
        }
    }
}

/// Bits Windows Mobile sets only for a moment, by register.
const MOMENTARY: [(usize, u16); 2] = [(reg::LCD_ID, 0x0008), (reg::CTL08, 0x0002)];

/// The check word of RetroPane's table: FNV-1a over the words before it,
/// so a stray write anywhere in the 40 bytes is caught.
fn check(words: &[u32]) -> u32 {
    words
        .iter()
        .fold(0x811C_9DC5_u32, |h, &w| (h ^ w).wrapping_mul(0x0100_0193))
}

/// RetroPane's table for `values`.
#[must_use]
pub fn encode(values: &[u16; REGS]) -> [u32; WORDS] {
    let mut w = [0; WORDS];
    w[0] = MAGIC;
    for (i, pair) in values.as_chunks::<2>().0.iter().enumerate() {
        w[1 + i] = u32::from(pair[0]) | u32::from(pair[1]) << 16;
    }
    w[WORDS - 1] = check(&w[..WORDS - 1]);
    w
}

/// The shadow `words` describe, and where it came from: RetroPane's table
/// if it is one and intact, else Windows Mobile's if it passes every
/// check, else [`DEFAULTS`].
#[must_use]
pub fn seed(words: &[u32; WORDS]) -> ([u16; REGS], Source) {
    if words[0] == MAGIC && words[WORDS - 1] == check(&words[..WORDS - 1]) {
        let mut v = [0; REGS];
        for (i, &w) in words[1..WORDS - 1].iter().enumerate() {
            v[2 * i] = w as u16;
            v[2 * i + 1] = (w >> 16) as u16;
        }
        return (v, Source::Resumed);
    }
    match from_wm(words) {
        Ok(v) => (v, Source::Adopted),
        Err(r) => (DEFAULTS, Source::Defaults(r)),
    }
}

fn from_wm(words: &[u32; WORDS]) -> Result<[u16; REGS], Reject> {
    if words.iter().any(|&w| w >> 16 != 0) {
        return Err(Reject::WideSlot);
    }
    let mut v = [0; REGS];
    for (&reg, &w) in WM_SLOT_REGS.iter().zip(words) {
        v[reg / 2] = w as u16;
    }
    if v[reg::CTL18 / 2] & crate::BACKLIGHT_ENABLE == 0 {
        return Err(Reject::BacklightOff);
    }
    if MOMENTARY.iter().any(|&(r, bits)| v[r / 2] & bits != 0) {
        return Err(Reject::Momentary);
    }
    Ok(v)
}

/// The shared table as a [`Shadow`]: every change is written through, so
/// the next program (or this one after a restart) starts from it.
#[derive(Debug)]
pub struct Table<'a> {
    words: &'a mut [u32; WORDS],
    values: [u16; REGS],
}

impl<'a> Table<'a> {
    /// Seeds the shadow from the table at `words` (see [`seed`]) and makes
    /// it RetroPane's from now on.
    pub fn adopt(words: &'a mut [u32; WORDS]) -> (Self, Source) {
        let (values, source) = seed(words);
        *words = encode(&values);
        (Self { words, values }, source)
    }
}

impl Shadow for Table<'_> {
    fn get(&self, offset: usize) -> u16 {
        self.values[offset / 2]
    }

    fn put(&mut self, offset: usize, value: u16) {
        self.values[offset / 2] = value;
        *self.words = encode(&self.values);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Windows Mobile table as HaRET leaves it: backlight and display
    /// bits on, SD powered, a battery bit in 0x1E's low bits (values
    /// shaped like the device's; the measured one goes in docs/hw/cpld.md).
    fn wm() -> [u32; WORDS] {
        let mut w = [0; WORDS];
        w[1] = 0x0004; // 0x10
        w[2] = 0x0005; // 0x12
        w[3] = 0x0008; // 0x14
        w[4] = 0x0001; // 0x16: SD
        w[5] = 0x000C; // 0x18: backlight + display
        w[8] = 0x0002; // 0x1E: battery bit, readout enable clear
        w
    }

    #[test]
    fn windows_mobiles_table_is_adopted_slot_by_slot() {
        let (v, src) = seed(&wm());
        assert_eq!(src, Source::Adopted);
        assert_eq!(v[reg::PWR10 / 2], 0x0004);
        assert_eq!(v[0x12 / 2], 0x0005);
        assert_eq!(v[reg::CTL18 / 2], 0x000C);
        assert_eq!(v[reg::LCD_ID / 2], 0x0002);
        assert_eq!(v[0], 0, "0x00 has no slot");
        let mut w = wm();
        w[9] = 0x0000; // 0x08 is the last slot
        w[0] = 0x0040; // 0x02 the first
        assert_eq!(seed(&w).0[reg::CTL08 / 2], 0);
        assert_eq!(seed(&w).0[0x02 / 2], 0x0040);
    }

    #[test]
    fn a_table_that_isnt_windows_mobiles_falls_back_to_defaults() {
        // Nothing there (the emulator; the flash-installed OS).
        assert_eq!(
            seed(&[0; WORDS]),
            (DEFAULTS, Source::Defaults(Reject::BacklightOff))
        );
        let mut w = wm();
        w[4] |= 0x0001_0000;
        assert_eq!(seed(&w).1, Source::Defaults(Reject::WideSlot));
        let mut w = wm();
        w[8] |= 0x0008; // readout enable left on
        assert_eq!(seed(&w).1, Source::Defaults(Reject::Momentary));
        let mut w = wm();
        w[9] = 0x0002; // reboot bit
        assert_eq!(seed(&w).1, Source::Defaults(Reject::Momentary));
        assert_eq!(
            DEFAULTS[reg::CTL18 / 2],
            0x0004,
            "the backlight stays on in every fallback"
        );
    }

    #[test]
    fn retropanes_table_round_trips_and_catches_stray_writes() {
        let mut v = [0u16; REGS];
        for (i, x) in v.iter_mut().enumerate() {
            *x = (0x1111 * i as u16).wrapping_add(3);
        }
        let w = encode(&v);
        assert_eq!(w[0], MAGIC);
        assert_eq!(seed(&w), (v, Source::Resumed));
        for k in 0..WORDS {
            let mut bad = w;
            bad[k] ^= 1 << (k % 32);
            assert_ne!(seed(&bad).1, Source::Resumed, "word {k} changed");
        }
    }

    #[test]
    fn adopting_takes_the_table_over_and_writes_changes_through() {
        let mut words = wm();
        let (mut t, src) = Table::adopt(&mut words);
        assert_eq!(src, Source::Adopted);
        t.put(reg::PWR16, 0x0003);
        let _ = t;
        // The next program finds RetroPane's table with the change in it.
        let (again, src) = Table::adopt(&mut words);
        assert_eq!(src, Source::Resumed);
        assert_eq!(again.get(reg::PWR16), 0x0003);
        assert_eq!(again.get(reg::CTL18), 0x000C);
    }
}
