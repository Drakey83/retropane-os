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

/// Where the sky moves: the whole weather panel, under its words and its
/// picture (the first try kept to the picture's box, which the picture
/// all but covers: the owner saw only a smudge or two).
pub(crate) const AREA: Rect = Rect::new(17, 53, 446, 194);
const FALL: Rect = AREA;

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
/// Clouds with nothing falling: near white by night, a blue grey on the
/// light glass.
const CLOUD_DARK: Argb = 0xFFE4_ECF6;
const CLOUD_LIGHT: Argb = 0xFF7C_92AE;

/// Whether `c` is one of the sky's colours (the light look keeps them).
#[must_use]
pub const fn is_ink(c: Argb) -> bool {
    matches!(
        c,
        RAIN_DARK | SNOW_DARK | RAIN_LIGHT | SNOW_LIGHT | CLOUD_DARK | CLOUD_LIGHT
    )
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

/// A cloud: a soft, uneven mass with wispy edges, lit from above, made
/// offline by tools/shell-assets/sky_cloud.py (fractal noise bent by noise
/// in a soft envelope) to sit with VClouds' realistic pictures; the first,
/// lobed one looked "too cartoony" (the owner, 2026-11-01). Made at
/// [`FADES`] strengths, so a cloud fades in and out by its glyph, all in
/// the sky's one atlas.
const CLOUD_W: usize = 112;
const CLOUD_H: usize = 40;
/// The clouds' shapes (a long low bank, a taller heap, two lumps, a thin
/// wisp: the owner, "we need like 3 or 4 variations") and the strengths
/// each fades through: 16 glyphs, ~290 KB of the 2D engine's 512 KB glyph
/// memory beside the screen's fonts (~200 KB).
const SHAPES: usize = 4;
const FADES: usize = 4;
/// How long a cloud is seen (ms), its fades included.
const CLOUD_LIFE: i64 = 12_000;
const CLOUD_ALL: &[u8; SHAPES * CLOUD_W * CLOUD_H] = include_bytes!("sky-cloud.alpha");

/// Cloud `shape` at strength `level` of [`FADES`].
const fn cloud_cover(shape: usize, level: usize) -> [u8; CLOUD_W * CLOUD_H] {
    let mut c = [0u8; CLOUD_W * CLOUD_H];
    let mut i = 0;
    while i < c.len() {
        c[i] = (CLOUD_ALL[shape * CLOUD_W * CLOUD_H + i] as usize * (level + 1) / FADES) as u8;
        i += 1;
    }
    c
}

static DROP: [u8; DROP_W * DROP_H] = drop_cover();
static FLAKE_PX: [u8; FLAKE * FLAKE] = flake_cover();
static CLOUDS: [[u8; CLOUD_W * CLOUD_H]; SHAPES * FADES] = {
    let mut all = [[0u8; CLOUD_W * CLOUD_H]; SHAPES * FADES];
    let mut k = 0;
    while k < SHAPES * FADES {
        all[k] = cloud_cover(k / FADES, k % FADES);
        k += 1;
    }
    all
};

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

const fn cloud_glyph(k: usize) -> Glyph {
    Glyph {
        w: CLOUD_W as u8,
        h: CLOUD_H as u8,
        left: -((CLOUD_W / 2) as i8),
        top: (CLOUD_H / 2) as i8,
        advance: 0,
        cover: &CLOUDS[k],
    }
}

static GLYPHS: [Glyph; 9 + SHAPES * FADES] = [
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
    cloud_glyph(0),
    cloud_glyph(1),
    cloud_glyph(2),
    cloud_glyph(3),
    cloud_glyph(4),
    cloud_glyph(5),
    cloud_glyph(6),
    cloud_glyph(7),
    cloud_glyph(8),
    cloud_glyph(9),
    cloud_glyph(10),
    cloud_glyph(11),
    cloud_glyph(12),
    cloud_glyph(13),
    cloud_glyph(14),
    cloud_glyph(15),
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
/// Cloud `shape` at strength `k` (0 faintest).
const fn cloud(shape: usize, k: usize) -> u8 {
    b' ' + 9 + (shape * FADES + k) as u8
}

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

/// Cloud `g` of `clouds` at `t` ms: its birth number (every cloud's
/// births counted in order: cloud g's are at (c x clouds - g) x life /
/// clouds) and how long it has lived.
const fn birth(t: i64, g: i64, clouds: i64) -> (i64, i64) {
    let at = t + g * CLOUD_LIFE / clouds;
    (
        at.div_euclid(CLOUD_LIFE) * clouds - g,
        at.rem_euclid(CLOUD_LIFE),
    )
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

/// The sky test's weathers: its notice, the weather code, the cloud cover.
const SKY_TESTS: [(&str, u8, u8); 6] = [
    ("Sky: the forecast's", 0, 0),
    ("Sky test: rain", 63, 90),
    ("Sky test: heavy rain", 65, 100),
    ("Sky test: snow", 73, 90),
    ("Sky test: storm", 95, 100),
    ("Sky test: cloudy", 3, 80),
];

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
            && (self.fetched_at.is_some() || self.sky_forced || self.sky_test > 0)
    }

    /// The test image's "Sky test" (the owner, 2026-11-01: "I would
    /// appreciate a way to test it"): each tap the next weather drawn over
    /// the real forecast's sky, then the forecast's again; its name in a
    /// notice.
    pub(crate) fn next_sky_test(&mut self) {
        self.sky_test = (self.sky_test + 1) % SKY_TESTS.len() as u8;
        self.show_notice(SKY_TESTS[usize::from(self.sky_test)].0);
        self.dirty(AREA);
    }

    /// The sky drawn: the forecast's now, or the sky test's.
    pub(crate) fn sky_now(&self) -> rp_weather::Sky {
        let mut sky = self.wx.now.sky;
        if let Some(&(_, wmo, cover)) = SKY_TESTS
            .get(usize::from(self.sky_test))
            .filter(|_| self.sky_test > 0)
        {
            sky.wmo = wmo;
            sky.cover = cover;
        }
        sky
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
        let w = weather_of(self.sky_now());
        let t = i64::from(self.now);
        let (aw, ah) = (i32::from(FALL.w), i32::from(FALL.h));
        f.clip(Some(AREA));
        let mut buf = [0u8; 48];
        let ink = match (self.light(), w.rain > 0, w.snow) {
            (false, true, _) => RAIN_DARK,
            (false, _, true) => SNOW_DARK,
            (false, ..) => CLOUD_DARK,
            (true, true, _) => RAIN_LIGHT,
            (true, _, true) => SNOW_LIGHT,
            (true, ..) => CLOUD_LIGHT,
        };

        // The wind: blowing toward from + 180; across the screen, its
        // east-west part (px a second from km/h, at least a slow drift).
        let kmh = i32::from(self.wx.now.sky.wind) / 10;
        let toward = i32::from(self.wx.now.wind_from) + 180;
        let across = sin1024(toward);
        let speed = (kmh * 2).clamp(6, 70);
        let vx = speed * across / 1024;
        let vx = if vx == 0 { 4 } else { vx };

        // Clouds: born one after another, evenly spaced, each living
        // [`CLOUD_LIFE`], so those on screen are always the last few born;
        // a cloud's shape is its birth number round the four, so no two
        // seen together are ever the same shape (the owner, 2026-11-01:
        // "guarantees 2 of the similar cloud shapes dont twin up"), and its
        // height lane the same way, so they don't stack either. Each drifts
        // with the wind (slower than the rain, as far clouds look), fades
        // in and out over ~1.5 s through its strengths, and is placed so it
        // never reaches the area's edge while seen (the first try's puffs
        // wrapped round and were cut off: "they just blink out of
        // existence").
        let cw = i64::from(AREA.w);
        let drift = i64::from((kmh / 2 + 3).clamp(3, 18)) * if across < 0 { -1 } else { 1 };
        let flash = w.storm && t % 5200 < 140;
        let clouds = i64::from(w.clouds.max(1));
        for g in 0..clouds {
            let (j, u) = birth(t, g, clouds);
            let s = scatter(j as u32);
            let travel = drift * CLOUD_LIFE / 1000;
            let room = (cw - CLOUD_W as i64 - travel.abs()).max(0);
            let start = (CLOUD_W as i64 / 2) + i64::from(s) % (room + 1);
            let x = if drift < 0 {
                start + travel.abs()
            } else {
                start
            } + drift * u / 1000;
            let lane = 150 / clouds;
            let y = 30 + j.rem_euclid(clouds) * lane + i64::from(s >> 9) % (lane / 2 + 1);
            // Its strength: up over the first 1.5 s, down over the last.
            let near = u.min(CLOUD_LIFE - u);
            let k = (near * FADES as i64 / 1500).min(FADES as i64);
            if k == 0 {
                continue;
            }
            let shape = j.rem_euclid(SHAPES as i64) as usize;
            buf[0] = cloud(shape, k as usize - 1);
            // The storm's flash: the first cloud lit up (its own shape
            // twice over, in place).
            let n = if flash && g == 0 {
                buf[1] = cloud(shape, FADES - 1);
                2
            } else {
                1
            };
            f.text(
                (i64::from(AREA.x) + x) as u16,
                (i64::from(AREA.y) + y) as u16,
                &SKY,
                ink,
                &buf[..n],
            );
        }
        f.clip(Some(FALL));

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
                FALL.x,
                (i32::from(FALL.y) + y).max(0) as u16,
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

#[cfg(test)]
mod twins {
    use super::*;

    /// No two clouds seen together share a shape, at any moment, for any
    /// number of clouds (the owner's photo, 2026-11-01: two alike side by
    /// side): the shapes of the clouds alive at `t`, as the scene picks them.
    #[test]
    fn clouds_seen_together_never_share_a_shape() {
        for clouds in 1..=3i64 {
            for t in (0..200_000i64).step_by(37) {
                let mut seen = [false; SHAPES];
                for g in 0..clouds {
                    let (j, u) = birth(t, g, clouds);
                    if u.min(CLOUD_LIFE - u) * FADES as i64 / 1500 == 0 {
                        continue;
                    }
                    let shape = j.rem_euclid(SHAPES as i64) as usize;
                    assert!(
                        !seen[shape],
                        "{clouds} clouds at {t} ms: shape {shape} twice"
                    );
                    seen[shape] = true;
                }
            }
        }
    }
}
