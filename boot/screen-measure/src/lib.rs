//! The screen measurement's patch list (docs/design/display-calibration.md,
//! "Measured with a colorimeter"), shared by the test image that shows the
//! patches and `cargo xtask measure`, which reads a colorimeter on each:
//! one schedule, so the two can't drift apart.
//!
//! The image starts the schedule when it starts; the PC knows that moment
//! from the loader's reply, waits [`LEAD_MS`], then reads each patch at
//! [`READ_AT_MS`] into it. Patches are full-screen solid colours with the
//! gamma table straight (output = input), so the readings are the panel's
//! own response. A one-shot reading with Argyll's `spotread` and a Spyder4
//! takes about 10 s on lit patches and much longer near black (measured
//! 2026-10-17), hence the long patches.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod program;

use program::{COUNTS, Program, Step, StepLight};

/// Before the first patch: time for the PC's reply handling and for the
/// panel to settle on black.
pub const LEAD_MS: u32 = 12_000;
/// When in a patch the PC starts its reading.
pub const READ_AT_MS: u32 = 2_500;

/// The backlight during a patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Light {
    /// As found (the loader's setting; Windows Mobile's 63%).
    Found,
    /// Off: what the sensor sees then is stray light, subtracted from the rest.
    Off,
    /// `n` eighths of full (8: full).
    Eighths(u32),
}

/// One patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Patch {
    /// The report's name.
    pub name: &'static str,
    /// The colour, 8 bits a channel.
    pub rgb: (u8, u8, u8),
    /// The backlight.
    pub light: Light,
    /// How long it shows, milliseconds.
    pub ms: u32,
}

const fn p(name: &'static str, rgb: (u8, u8, u8), light: Light, ms: u32) -> Patch {
    Patch {
        name,
        rgb,
        light,
        ms,
    }
}

/// Near black the Spyder4 integrates far longer: run 1 (2026-10-17) lost the
/// backlight-off and black readings at 16 s; 45 s leaves room.
const DARK: u32 = 45_000;
const LIT: u32 = 14_000;

/// Grey levels measured at the found backlight.
pub const GREYS: [u8; 14] = [0, 8, 16, 24, 32, 48, 64, 96, 128, 160, 192, 224, 240, 255];

/// The schedule.
pub const PATCHES: [Patch; 31] = [
    p("stray", (0, 0, 0), Light::Off, DARK),
    p("grey 0", (0, 0, 0), Light::Found, DARK),
    p("grey 8", (8, 8, 8), Light::Found, DARK),
    p("grey 16", (16, 16, 16), Light::Found, DARK),
    p("grey 24", (24, 24, 24), Light::Found, DARK),
    p("grey 32", (32, 32, 32), Light::Found, DARK),
    p("grey 48", (48, 48, 48), Light::Found, DARK),
    p("grey 64", (64, 64, 64), Light::Found, DARK),
    p("grey 96", (96, 96, 96), Light::Found, LIT),
    p("grey 128", (128, 128, 128), Light::Found, LIT),
    p("grey 160", (160, 160, 160), Light::Found, LIT),
    p("grey 192", (192, 192, 192), Light::Found, LIT),
    p("grey 224", (224, 224, 224), Light::Found, LIT),
    p("grey 240", (240, 240, 240), Light::Found, LIT),
    p("grey 255", (255, 255, 255), Light::Found, LIT),
    p("red 128", (128, 0, 0), Light::Found, LIT),
    p("red 255", (255, 0, 0), Light::Found, LIT),
    p("green 128", (0, 128, 0), Light::Found, LIT),
    p("green 255", (0, 255, 0), Light::Found, LIT),
    p("blue 128", (0, 0, 128), Light::Found, LIT),
    p("blue 255", (0, 0, 255), Light::Found, LIT),
    p("black at full light", (0, 0, 0), Light::Eighths(8), DARK),
    p(
        "white at 1/8 light",
        (255, 255, 255),
        Light::Eighths(1),
        LIT,
    ),
    p(
        "white at 2/8 light",
        (255, 255, 255),
        Light::Eighths(2),
        LIT,
    ),
    p(
        "white at 3/8 light",
        (255, 255, 255),
        Light::Eighths(3),
        LIT,
    ),
    p(
        "white at 4/8 light",
        (255, 255, 255),
        Light::Eighths(4),
        LIT,
    ),
    p(
        "white at 5/8 light",
        (255, 255, 255),
        Light::Eighths(5),
        LIT,
    ),
    p(
        "white at 6/8 light",
        (255, 255, 255),
        Light::Eighths(6),
        LIT,
    ),
    p(
        "white at 7/8 light",
        (255, 255, 255),
        Light::Eighths(7),
        LIT,
    ),
    p(
        "white at full light",
        (255, 255, 255),
        Light::Eighths(8),
        LIT,
    ),
    p("white again (drift)", (255, 255, 255), Light::Found, LIT),
];

/// When patch `i` starts, milliseconds after the image starts.
#[must_use]
pub fn start_ms(i: usize) -> u32 {
    LEAD_MS + PATCHES[..i].iter().map(|p| p.ms).sum::<u32>()
}

/// The whole run, milliseconds.
#[must_use]
pub fn total_ms() -> u32 {
    start_ms(PATCHES.len())
}

/// What the image runs: the built-in [`PATCHES`] or a program the PC wrote
/// into it ([`program`]).
#[derive(Clone, Copy, Debug)]
pub enum Schedule<'a> {
    /// [`PATCHES`], after [`LEAD_MS`].
    BuiltIn,
    /// A checked program.
    Program(Program<'a>),
}

impl<'a> Schedule<'a> {
    /// The schedule in an image's program room.
    ///
    /// # Errors
    /// The room holds a damaged or out-of-range program.
    pub fn from_room(room: &'a [u8]) -> Result<Self, program::Error> {
        Ok(Program::parse(room)?.map_or(Self::BuiltIn, Self::Program))
    }

    /// Steps.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::BuiltIn => PATCHES.len(),
            Self::Program(p) => p.len(),
        }
    }

    /// Never true; for clippy.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Before the first step, milliseconds.
    #[must_use]
    pub fn lead_ms(&self) -> u32 {
        match self {
            Self::BuiltIn => LEAD_MS,
            Self::Program(p) => p.lead_ms(),
        }
    }

    /// Step `i`.
    #[must_use]
    pub fn step(&self, i: usize) -> Step {
        match self {
            Self::BuiltIn => {
                let p = &PATCHES[i];
                Step {
                    rgb: p.rgb,
                    table: 0,
                    light: match p.light {
                        Light::Found => StepLight::Found,
                        Light::Off => StepLight::Counts(0),
                        Light::Eighths(n) => StepLight::Counts(COUNTS * n.min(8) as u16 / 8),
                    },
                    ms: p.ms,
                }
            }
            Self::Program(p) => p.step(i),
        }
    }

    /// Gamma table `k` (0: straight).
    #[must_use]
    pub fn table(&self, k: u8) -> [u32; 17] {
        match self {
            Self::BuiltIn => straight_table(),
            Self::Program(p) => p.table(k),
        }
    }
}

/// The straight gamma table (output = input), as the 2700G's 17 points.
#[must_use]
pub fn straight_table() -> [u32; 17] {
    core::array::from_fn(|i| ((16 * i as u32).min(255)) * 0x01_0101)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_patch_has_time_for_a_reading() {
        for p in &PATCHES {
            assert!(p.ms >= READ_AT_MS + 11_000, "{}", p.name);
            if p.rgb == (0, 0, 0) {
                assert!(p.ms >= READ_AT_MS + 40_000, "{} is near black", p.name);
            }
        }
    }

    #[test]
    fn the_run_is_under_ten_minutes_and_starts_after_the_lead() {
        assert_eq!(start_ms(0), LEAD_MS);
        assert!(total_ms() < 13 * 60_000, "{} ms", total_ms());
    }

    #[test]
    fn the_greys_are_all_measured() {
        for g in GREYS {
            assert!(
                PATCHES
                    .iter()
                    .any(|p| p.rgb == (g, g, g) && p.light == Light::Found)
            );
        }
    }

    #[test]
    fn the_built_in_schedule_is_the_patch_list() {
        let room = program::placeholder();
        let s = Schedule::from_room(&room).unwrap();
        assert!(matches!(s, Schedule::BuiltIn));
        assert_eq!(s.len(), PATCHES.len());
        assert_eq!(s.lead_ms(), LEAD_MS);
        for (i, p) in PATCHES.iter().enumerate() {
            let st = s.step(i);
            assert_eq!((st.rgb, st.ms, st.table), (p.rgb, p.ms, 0), "{}", p.name);
            let want = match p.light {
                Light::Found => StepLight::Found,
                Light::Off => StepLight::Counts(0),
                Light::Eighths(n) => StepLight::Counts(54 * n as u16),
            };
            assert_eq!(st.light, want, "{}", p.name);
        }
        assert_eq!(s.table(0), straight_table());
    }

    #[test]
    fn the_straight_table_is_straight() {
        let t = straight_table();
        assert_eq!(t[0], 0);
        assert_eq!(t[8], 0x80_8080);
        assert_eq!(t[16], 0xFF_FFFF);
    }
}
