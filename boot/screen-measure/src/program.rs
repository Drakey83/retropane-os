//! A measurement program: the patch list carried inside the image
//! (docs/design/display-calibration.md, C6 step 2).
//!
//! The image holds [`PROGRAM_BYTES`] of room starting with [`MAGIC`]. The PC
//! writes a program there in a copy of the image before sending it, so every
//! pass of a calibration (profile, refine, verify) is the same image with
//! different data: no rebuild between measurements. An empty room (no steps)
//! means the built-in schedule ([`crate::PATCHES`]).
//!
//! Layout, little-endian: magic (8), step count (u16), table count (u16),
//! lead in ms (u32), checksum (u32, FNV-1a over everything after the
//! header), then each table (17 words `0x00RRGGBB`, the 2700G's gamma
//! points), then each step (8 bytes: red, green, blue, table, backlight
//! u16, time u16). Table 0 is the straight table; `k` is the program's
//! table `k`. The backlight is counts of RetroPane's 432 (0 = off) or
//! [`LIGHT_FOUND`]; the time is in tens of milliseconds.
//!
//! The device checks everything before it shows anything: sizes, the
//! checksum, every table index and backlight value, each table word's
//! spare byte, and the whole run's length.

/// The start of the room; also the version.
pub const MAGIC: [u8; 8] = *b"RPMEAS01";
/// The room's size in the image.
pub const PROGRAM_BYTES: usize = 8192;
/// The most tables a program holds (besides the straight one).
pub const MAX_TABLES: usize = 8;
const HEADER: usize = 20;
const TABLE: usize = 17 * 4;
const STEP: usize = 8;
/// The most steps that fit with every table used.
pub const MAX_STEPS: usize = (PROGRAM_BYTES - HEADER - MAX_TABLES * TABLE) / STEP;
/// The backlight's cycle in counts (rp-drv-pxa-pwm: prescale 0, 30 kHz).
pub const COUNTS: u16 = 432;
/// Backlight value meaning "as the image found it".
pub const LIGHT_FOUND: u16 = 0xFFFF;
/// The shortest step: the panel must settle and the sensor read.
pub const MIN_STEP_MS: u32 = 500;
/// The longest run a program may ask for.
pub const MAX_RUN_MS: u32 = 90 * 60_000;

/// The backlight during a step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepLight {
    /// As the image found it.
    Found,
    /// Counts of [`COUNTS`] (0: off).
    Counts(u16),
}

/// One step of a program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    /// The colour shown full screen, 8 bits a channel.
    pub rgb: (u8, u8, u8),
    /// The gamma table: 0 straight, `k` the program's table `k`.
    pub table: u8,
    /// The backlight.
    pub light: StepLight,
    /// How long it shows, milliseconds (a multiple of 10).
    pub ms: u32,
}

/// Why a program was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The room doesn't start with [`MAGIC`].
    Magic,
    /// More steps or tables than allowed, or more than the room holds.
    Size,
    /// The checksum doesn't match.
    Checksum,
    /// A step names a table the program doesn't have.
    Table,
    /// A backlight value over [`COUNTS`].
    Light,
    /// A step shorter than [`MIN_STEP_MS`] or not a multiple of 10 ms.
    Time,
    /// The whole run is longer than [`MAX_RUN_MS`].
    Run,
    /// A table word with its top byte set.
    Word,
}

impl Error {
    /// For the run log.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Magic => "no program room",
            Self::Size => "too big",
            Self::Checksum => "checksum",
            Self::Table => "unknown table",
            Self::Light => "backlight over full",
            Self::Time => "step time",
            Self::Run => "run too long",
            Self::Word => "table word",
        }
    }
}

/// The empty room the image is built with.
#[must_use]
pub const fn placeholder() -> [u8; PROGRAM_BYTES] {
    let mut b = [0u8; PROGRAM_BYTES];
    let mut i = 0;
    while i < MAGIC.len() {
        b[i] = MAGIC[i];
        i += 1;
    }
    b
}

fn fnv1a(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for &b in bytes {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// A checked program.
#[derive(Clone, Copy, Debug)]
pub struct Program<'a> {
    tables: &'a [u8],
    steps: &'a [u8],
    lead_ms: u32,
}

impl<'a> Program<'a> {
    /// Checks the room. `Ok(None)`: it is empty (the built-in schedule).
    ///
    /// # Errors
    /// Anything malformed, before a single step is shown.
    pub fn parse(room: &'a [u8]) -> Result<Option<Self>, Error> {
        if room.len() < HEADER || room[..MAGIC.len()] != MAGIC {
            return Err(Error::Magic);
        }
        let steps = usize::from(u16_at(room, 8));
        let tables = usize::from(u16_at(room, 10));
        if steps == 0 && tables == 0 {
            return Ok(None);
        }
        if steps == 0 || steps > MAX_STEPS || tables > MAX_TABLES {
            return Err(Error::Size);
        }
        let end = HEADER + tables * TABLE + steps * STEP;
        if end > room.len() {
            return Err(Error::Size);
        }
        if fnv1a(&room[HEADER..end]) != u32_at(room, 16) {
            return Err(Error::Checksum);
        }
        let p = Self {
            tables: &room[HEADER..HEADER + tables * TABLE],
            steps: &room[HEADER + tables * TABLE..end],
            lead_ms: u32_at(room, 12),
        };
        if p.tables.as_chunks::<4>().0.iter().any(|w| w[3] != 0) {
            return Err(Error::Word);
        }
        let mut run = u64::from(p.lead_ms);
        for i in 0..steps {
            let s = p.raw(i);
            if usize::from(s[3]) > tables {
                return Err(Error::Table);
            }
            let light = u16_at(s, 4);
            if light != LIGHT_FOUND && light > COUNTS {
                return Err(Error::Light);
            }
            let ms = u32::from(u16_at(s, 6)) * 10;
            if ms < MIN_STEP_MS {
                return Err(Error::Time);
            }
            run += u64::from(ms);
        }
        if run > u64::from(MAX_RUN_MS) {
            return Err(Error::Run);
        }
        Ok(Some(p))
    }

    fn raw(&self, i: usize) -> &'a [u8] {
        &self.steps[i * STEP..(i + 1) * STEP]
    }

    /// Steps in the program.
    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len() / STEP
    }

    /// Never true for a parsed program; for clippy.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Before the first step, milliseconds.
    #[must_use]
    pub const fn lead_ms(&self) -> u32 {
        self.lead_ms
    }

    /// Step `i` (`i < len`).
    #[must_use]
    pub fn step(&self, i: usize) -> Step {
        let s = self.raw(i);
        let light = u16_at(s, 4);
        Step {
            rgb: (s[0], s[1], s[2]),
            table: s[3],
            light: if light == LIGHT_FOUND {
                StepLight::Found
            } else {
                StepLight::Counts(light)
            },
            ms: u32::from(u16_at(s, 6)) * 10,
        }
    }

    /// Gamma table `k` (0: straight).
    #[must_use]
    pub fn table(&self, k: u8) -> [u32; 17] {
        if k == 0 {
            return crate::straight_table();
        }
        let at = (usize::from(k) - 1) * TABLE;
        core::array::from_fn(|i| u32_at(self.tables, at + 4 * i))
    }

    /// When step `i` starts, milliseconds after the image starts.
    #[must_use]
    pub fn start_ms(&self, i: usize) -> u32 {
        self.lead_ms + (0..i).map(|j| self.step(j).ms).sum::<u32>()
    }
}

/// Writes a program into `room` (a copy of the image's room, which starts
/// with [`MAGIC`]). Returns the bytes used.
///
/// # Errors
/// The same checks the device makes.
pub fn encode(
    steps: &[Step],
    tables: &[[u32; 17]],
    lead_ms: u32,
    room: &mut [u8],
) -> Result<usize, Error> {
    if room.len() < HEADER || room[..MAGIC.len()] != MAGIC {
        return Err(Error::Magic);
    }
    if steps.is_empty() || steps.len() > MAX_STEPS || tables.len() > MAX_TABLES {
        return Err(Error::Size);
    }
    let end = HEADER + tables.len() * TABLE + steps.len() * STEP;
    if end > room.len() {
        return Err(Error::Size);
    }
    let mut at = HEADER;
    for t in tables {
        for w in t {
            if w >> 24 != 0 {
                return Err(Error::Word);
            }
            room[at..at + 4].copy_from_slice(&w.to_le_bytes());
            at += 4;
        }
    }
    for s in steps {
        if usize::from(s.table) > tables.len() {
            return Err(Error::Table);
        }
        let light = match s.light {
            StepLight::Found => LIGHT_FOUND,
            StepLight::Counts(c) if c <= COUNTS => c,
            StepLight::Counts(_) => return Err(Error::Light),
        };
        if s.ms < MIN_STEP_MS || s.ms % 10 != 0 || s.ms / 10 > u32::from(u16::MAX) {
            return Err(Error::Time);
        }
        room[at..at + 3].copy_from_slice(&[s.rgb.0, s.rgb.1, s.rgb.2]);
        room[at + 3] = s.table;
        room[at + 4..at + 6].copy_from_slice(&light.to_le_bytes());
        room[at + 6..at + 8].copy_from_slice(&((s.ms / 10) as u16).to_le_bytes());
        at += STEP;
    }
    room[8..10].copy_from_slice(&(steps.len() as u16).to_le_bytes());
    room[10..12].copy_from_slice(&(tables.len() as u16).to_le_bytes());
    room[12..16].copy_from_slice(&lead_ms.to_le_bytes());
    let sum = fnv1a(&room[HEADER..end]);
    room[16..20].copy_from_slice(&sum.to_le_bytes());
    let run = u64::from(lead_ms) + steps.iter().map(|s| u64::from(s.ms)).sum::<u64>();
    if run > u64::from(MAX_RUN_MS) {
        return Err(Error::Run);
    }
    Ok(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(rgb: (u8, u8, u8), table: u8, light: StepLight, ms: u32) -> Step {
        Step {
            rgb,
            table,
            light,
            ms,
        }
    }

    #[test]
    fn the_placeholder_is_the_built_in_schedule() {
        let room = placeholder();
        assert!(matches!(Program::parse(&room), Ok(None)));
    }

    #[test]
    fn a_program_round_trips() {
        let mut room = placeholder();
        let mut t = crate::straight_table();
        t[16] = 0x00FF_80C0;
        let steps = [
            step((1, 2, 3), 0, StepLight::Found, 4_000),
            step((255, 255, 255), 1, StepLight::Counts(432), 12_340),
            step((0, 0, 0), 1, StepLight::Counts(0), 500),
        ];
        encode(&steps, &[t], 8_000, &mut room).unwrap();
        let p = Program::parse(&room).unwrap().unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(p.lead_ms(), 8_000);
        for (i, s) in steps.iter().enumerate() {
            assert_eq!(p.step(i), *s);
        }
        assert_eq!(p.table(1), t);
        assert_eq!(p.table(0), crate::straight_table());
        assert_eq!(p.start_ms(2), 8_000 + 4_000 + 12_340);
    }

    #[test]
    fn the_device_refuses_damage_and_out_of_range_values() {
        let good = |room: &mut [u8; PROGRAM_BYTES]| {
            *room = placeholder();
            let steps = [step((9, 9, 9), 1, StepLight::Found, 1_000)];
            encode(&steps, &[crate::straight_table()], 0, room).unwrap();
        };
        let mut room = placeholder();
        good(&mut room);
        assert!(Program::parse(&room).is_ok());
        // Any changed byte after the header: checksum.
        room[30] ^= 1;
        assert_eq!(Program::parse(&room).unwrap_err(), Error::Checksum);
        // A step naming table 2 of 1, a backlight over full, a short step,
        // a table word with its spare byte set, a wrong magic: refused, even
        // with the checksum made to match.
        let patch = |at: usize, v: &[u8], want: Error| {
            let mut r = placeholder();
            good(&mut r);
            r[at..at + v.len()].copy_from_slice(v);
            let end = HEADER + TABLE + STEP;
            let sum = fnv1a(&r[HEADER..end]);
            r[16..20].copy_from_slice(&sum.to_le_bytes());
            assert_eq!(Program::parse(&r).unwrap_err(), want);
        };
        let s = HEADER + TABLE;
        patch(s + 3, &[2], Error::Table);
        patch(s + 4, &433u16.to_le_bytes(), Error::Light);
        patch(s + 6, &49u16.to_le_bytes(), Error::Time);
        patch(HEADER + 3, &[1], Error::Word);
        patch(0, b"X", Error::Magic);
        patch(8, &(MAX_STEPS as u16 + 1).to_le_bytes(), Error::Size);
        patch(10, &9u16.to_le_bytes(), Error::Size);
    }

    #[test]
    fn encode_makes_the_same_checks() {
        let mut room = placeholder();
        let t = crate::straight_table();
        let one = |s: Step| [s];
        assert_eq!(
            encode(
                &one(step((0, 0, 0), 1, StepLight::Found, 1_000)),
                &[],
                0,
                &mut room
            ),
            Err(Error::Table)
        );
        assert_eq!(
            encode(
                &one(step((0, 0, 0), 0, StepLight::Counts(433), 1_000)),
                &[t],
                0,
                &mut room
            ),
            Err(Error::Light)
        );
        assert_eq!(
            encode(
                &one(step((0, 0, 0), 0, StepLight::Found, 1_005)),
                &[t],
                0,
                &mut room
            ),
            Err(Error::Time)
        );
        assert_eq!(
            encode(
                &one(step((0, 0, 0), 0, StepLight::Found, 655_360)),
                &[t],
                0,
                &mut room
            ),
            Err(Error::Time)
        );
        let long = [step((0, 0, 0), 0, StepLight::Found, 600_000 - 10); 10];
        assert_eq!(encode(&long, &[], 0, &mut room), Err(Error::Run));
        assert_eq!(encode(&[], &[], 0, &mut room), Err(Error::Size));
        let mut w = t;
        w[0] = 0x0100_0000;
        assert_eq!(
            encode(
                &one(step((0, 0, 0), 0, StepLight::Found, 1_000)),
                &[w],
                0,
                &mut room
            ),
            Err(Error::Word)
        );
    }

    #[test]
    fn a_full_profile_fits() {
        const { assert!(MAX_STEPS >= 900) };
    }
}
