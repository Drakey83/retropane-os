//! The living sky behind the Today screen's weather picture, to try (W6 in
//! docs/design/weather-flows.md, "decided for v1": the clouds drift at the
//! forecast's wind speed and direction, rain or snow falls at its
//! strength; ui.md's signature idea 2; the owner's ask of 2026-11-01 to see
//! it on the X51v). [`LIVING_SKY`] off reverts to the still picture.
//!
//! Iterate (rule 0): phone weather apps play a canned video loop per
//! condition (megabytes of frames, the same rain whatever the forecast
//! says); ui.md planned it on the 3D core's compositor, which isn't built
//! yet. Ours is computed from the forecast itself each frame and costs
//! almost nothing: the drops, flakes and cloud puffs are the glyphs of a
//! small font made by the build ([`SKY`]), because text is what the 2D
//! engine blends over what is under it (a fill sets the layer) and its
//! masks stay on the GPU once uploaded, one colour, one atlas. A frame is a
//! dozen text runs, ~10 a second, only while the forecast is a real one
//! (never the sample's: nothing pretends).

use rp_draw::{Argb, Font, Frame, Glyph, Rect};

use super::{Screen, Shell};

/// Off: the Today picture stands still (the owner may revert, 2026-11-01).
pub const LIVING_SKY: bool = true;

/// How often the sky moves (ms): ~12 Hz, in ui.md's 10-15 Hz.
pub const STEP_MS: u32 = 80;

/// Where the sky moves: the weather panel's right part, behind its picture.
pub(crate) const AREA: Rect = Rect::new(212, 53, 251, 194);

/// The sky's colours, one a frame (its glyphs' coverage gives each its
/// strength), chosen for each look and left alone by the light look's
/// mapping ([`is_ink`]): mapped like text, rain and snow came out black on
/// the white glass (the owner, 2026-11-01: "rain and snow arent black").
/// Dark look: pale blue rain, white snow; light look: a mid blue rain, an
/// icy blue snow (white would vanish on its white glass).
const RAIN_DARK: Argb = 0xFFB4_D2F2;
const SNOW_DARK: Argb = 0xFFF6_FAFF;
const RAIN_LIGHT: Argb = 0xFF4E_8CD0;
const SNOW_LIGHT: Argb = 0xFF9E_C2EA;

/// Whether `c` is one of the sky's colours (the light look keeps them).
#[must_use]
pub const fn is_ink(c: Argb) -> bool {
    matches!(c, RAIN_DARK | SNOW_DARK | RAIN_LIGHT | SNOW_LIGHT)
}

/// A raindrop: a 2 x 14 streak, faint at its top.
const DROP_W: usize = 2;
const DROP_H: usize = 14;
const fn drop_cover() -> [u8; DROP_W * DROP_H] {
    let mut c = [0u8; DROP_W * DROP_H];
    let mut y = 0;
    while y < DROP_H {
        let a = (40 + 170 * y / (DROP_H - 1)) as u8;
        c[y * DROP_W] = a;
        c[y * DROP_W + 1] = a / 2;
        y += 1;
    }
    c
}

/// A snowflake: a soft dot, 6 across.
const FLAKE: usize = 6;
const fn flake_cover() -> [u8; FLAKE * FLAKE] {
    let mut c = [0u8; FLAKE * FLAKE];
    let mut i = 0;
    while i < FLAKE * FLAKE {
        let (x, y) = ((i % FLAKE) as i32 * 2 - 5, (i / FLAKE) as i32 * 2 - 5);
        let d = x * x + y * y;
        c[i] = if d >= 36 {
            0
        } else {
            (235 - d * 235 / 36) as u8
        };
        i += 1;
    }
    c
}

/// A cloud puff: a soft ellipse 96 x 34, its edge fading out (squared, so
/// overlapping puffs build up into a cloud).
const PUFF_W: usize = 96;
const PUFF_H: usize = 34;
const fn puff_cover() -> [u8; PUFF_W * PUFF_H] {
    let mut c = [0u8; PUFF_W * PUFF_H];
    let mut i = 0;
    while i < PUFF_W * PUFF_H {
        // Distance from the centre, the ellipse's edge at 1024.
        let x = ((i % PUFF_W) as i64 * 2 + 1 - PUFF_W as i64) * 1024 / PUFF_W as i64;
        let y = ((i / PUFF_W) as i64 * 2 + 1 - PUFF_H as i64) * 1024 / PUFF_H as i64;
        let d = (x * x + y * y) / 1024;
        if d < 1024 {
            let f = 1024 - d;
            c[i] = (f * f / 1024 * 70 / 1024) as u8;
        }
        i += 1;
    }
    c
}

static DROP: [u8; DROP_W * DROP_H] = drop_cover();
static FLAKE_PX: [u8; FLAKE * FLAKE] = flake_cover();
static PUFF: [u8; PUFF_W * PUFF_H] = puff_cover();

const fn space(advance: u8) -> Glyph {
    Glyph {
        w: 0,
        h: 0,
        left: 0,
        top: 0,
        advance,
        cover: &[],
    }
}

/// The sky's glyphs: spaces of 1 to 64 px (any gap in a few bytes), then a
/// drop, a flake and a puff, each centred on the pen and the baseline and
/// moving it nowhere.
static GLYPHS: [Glyph; 10] = [
    space(1),
    space(2),
    space(4),
    space(8),
    space(16),
    space(32),
    space(64),
    Glyph {
        w: DROP_W as u8,
        h: DROP_H as u8,
        left: 0,
        top: (DROP_H / 2) as i8,
        advance: 0,
        cover: &DROP,
    },
    Glyph {
        w: FLAKE as u8,
        h: FLAKE as u8,
        left: -(FLAKE as i8 / 2),
        top: (FLAKE / 2) as i8,
        advance: 0,
        cover: &FLAKE_PX,
    },
    Glyph {
        w: PUFF_W as u8,
        h: PUFF_H as u8,
        left: -(PUFF_W as i8 / 2),
        top: (PUFF_H / 2) as i8,
        advance: 0,
        cover: &PUFF,
    },
];

/// The sky's font (its line spans every glyph, for the renderers' bounds).
pub static SKY: Font = Font {
    line: 40,
    ascent: 20,
    first: b' ',
    glyphs: &GLYPHS,
};

const RAIN: u8 = b' ' + 7;
const SNOW: u8 = b' ' + 8;
const CLOUD: u8 = b' ' + 9;

/// A run of particles at x offsets `xs` (ascending, from the run's start):
/// each gap in spaces, then the particle.
fn run(buf: &mut [u8; 48], xs: &[i32], what: u8) -> usize {
    let mut n = 0;
    let mut at = 0;
    for &x in xs {
        let mut gap = (x - at).max(0);
        at += gap;
        let mut k = 6u8;
        while gap > 0 && n < buf.len() - 1 {
            let step = 1i32 << k;
            if gap >= step {
                buf[n] = b' ' + k;
                n += 1;
                gap -= step;
            } else {
                k -= 1;
            }
        }
        if n < buf.len() {
            buf[n] = what;
            n += 1;
        }
    }
    n
}

/// A number from `i` that looks random (the same each frame).
const fn scatter(i: u32) -> u32 {
    let mut h = i.wrapping_mul(0x9E37_79B9) ^ 0x5EED;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^ (h >> 12)
}

/// sin(`deg`) x 1024 (Bhaskara's approximation, within 0.2%).
const fn sin1024(deg: i32) -> i32 {
    let d = deg.rem_euclid(360);
    let (x, sign) = if d > 180 { (d - 180, -1) } else { (d, 1) };
    sign * 4096 * x * (180 - x) / (40_500 - x * (180 - x))
}

/// What the sky shows: rain (its strength 1-3), snow, a storm's flashes,
/// how cloudy (0-3 groups of puffs).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Weather {
    rain: u8,
    snow: bool,
    storm: bool,
    clouds: u8,
}

fn weather_of(sky: rp_weather::Sky) -> Weather {
    let (rain, snow) = match sky.wmo {
        51 | 53 | 56 | 61 | 80 => (1, false),
        55 | 57 | 63 | 66 | 81 => (2, false),
        65 | 67 | 82 | 95..=99 => (3, false),
        71..=77 | 85 | 86 => (0, true),
        _ => (0, false),
    };
    let clouds = match sky.cover {
        0..=19 => 1,
        20..=59 => 2,
        _ => 3,
    };
    Weather {
        rain,
        snow,
        storm: matches!(sky.wmo, 95..=99),
        clouds,
    }
}

impl Shell {
    /// Whether the sky moves now: on the Today screen, nothing over it, a
    /// real forecast (or the previews' word, [`Shell::preview_sky`]).
    #[must_use]
    pub fn sky_live(&self) -> bool {
        LIVING_SKY
            && self.screen == Screen::Today
            && !self.bare
            && self.align.is_none()
            && (self.fetched_at.is_some() || self.sky_forced)
    }

    /// The previews' sky moving with the sample forecast (the image's
    /// moves only with a real one).
    pub fn preview_sky(&mut self, on: bool) {
        self.sky_forced = on;
        self.dirty(AREA);
    }

    /// The sky's next step if due at `now`: its area marked to draw.
    pub(crate) fn sky_step(&mut self, now: u32) -> bool {
        if !self.sky_live() || now.wrapping_sub(self.sky_at) < STEP_MS {
            return false;
        }
        self.sky_at = now;
        self.dirty(AREA);
        true
    }

    /// The sky at `self.now`, behind the weather picture: cloud puffs
    /// drifting with the wind, then the rain or snow falling through them,
    /// all cut to [`AREA`].
    pub(crate) fn sky_scene(&self, f: &mut Frame) {
        if !self.sky_live() {
            return;
        }
        let w = weather_of(self.wx.now.sky);
        let t = i64::from(self.now);
        let (aw, ah) = (i32::from(AREA.w), i32::from(AREA.h));
        f.clip(Some(AREA));
        let mut buf = [0u8; 48];
        let ink = match (self.light(), w.snow) {
            (false, false) => RAIN_DARK,
            (false, true) => SNOW_DARK,
            (true, false) => RAIN_LIGHT,
            (true, true) => SNOW_LIGHT,
        };

        // The wind: blowing toward from + 180; across the screen, its
        // east-west part (px a second from km/h, at least a slow drift).
        let kmh = i32::from(self.wx.now.sky.wind) / 10;
        let toward = i32::from(self.wx.now.wind_from) + 180;
        let across = sin1024(toward);
        let speed = (kmh * 2).clamp(6, 70);
        let vx = speed * across / 1024;
        let vx = if vx == 0 { 4 } else { vx };

        // Clouds: groups of puffs on their own heights, wrapping round.
        let span = aw + PUFF_W as i32 * 2;
        let flash = w.storm && t % 5200 < 140;
        for g in 0..u32::from(w.clouds) + u32::from(flash) * 2 {
            let s = scatter(g + 11);
            let y = 24 + (s % 70) as i32 + g as i32 * 26;
            let pace = vx * (3 + (s >> 8) as i32 % 3) / 4;
            let x0 = ((s >> 16) as i64 + t * i64::from(pace) / 1000).rem_euclid(i64::from(span))
                as i32
                - PUFF_W as i32;
            let xs = [0, 40 + (s % 20) as i32, 70 + (s >> 4) as i32 % 30];
            let n = run(&mut buf, &xs, CLOUD);
            // From the area's left less a puff (x0 >= -96): never off screen.
            f.text(
                (i32::from(AREA.x) + x0).max(0) as u16,
                (i32::from(AREA.y) + y) as u16,
                &SKY,
                ink,
                &buf[..n],
            );
        }

        // Rain or snow: lanes of a few particles each, at their own phase.
        let (lanes, per, fall, what) = if w.rain > 0 {
            (
                8 + 2 * u32::from(w.rain),
                1 + u32::from(w.rain),
                260 + 70 * i64::from(w.rain),
                RAIN,
            )
        } else if w.snow {
            (10, 2, 38, SNOW)
        } else {
            return f.clip(None);
        };
        for lane in 0..lanes {
            let s = scatter(lane + 101);
            let y = ((s >> 7) as i64 + t * fall / 1000).rem_euclid(i64::from(ah + 20)) as i32 - 10;
            let sway = if what == SNOW {
                sin1024(((t / 12) as i32 + (s % 360) as i32) % 360) * 6 / 1024
            } else {
                0
            };
            let drift = (t * i64::from(vx) / 4000).rem_euclid(i64::from(aw)) as i32;
            let mut xs = [0i32; 4];
            for (k, x) in xs.iter_mut().enumerate().take(per as usize) {
                *x = ((scatter(lane * 7 + k as u32) % aw as u32) as i32 + drift + sway)
                    .rem_euclid(aw);
            }
            let xs = &mut xs[..per as usize];
            xs.sort_unstable();
            let n = run(&mut buf, xs, what);
            f.text(
                AREA.x,
                (i32::from(AREA.y) + y).max(0) as u16,
                &SKY,
                ink,
                &buf[..n],
            );
        }
        f.clip(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_puts_each_particle_at_its_place() {
        let mut buf = [0u8; 48];
        let n = run(&mut buf, &[0, 37, 200], RAIN);
        let mut x = 0;
        let mut at = [0i32; 3];
        let mut k = 0;
        for &c in &buf[..n] {
            if c == RAIN {
                at[k] = x;
                k += 1;
            } else {
                x += i32::from(SKY.glyph(c).unwrap().advance);
            }
        }
        assert_eq!(at, [0, 37, 200]);
    }

    #[test]
    fn the_sine_is_close() {
        for (d, want) in [
            (0, 0),
            (30, 512),
            (90, 1024),
            (180, 0),
            (270, -1024),
            (330, -512),
        ] {
            assert!((sin1024(d) - want).abs() <= 3, "{d}: {}", sin1024(d));
        }
    }
}
