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

/// How often the sky moves (ms): 30 a second, the owner's floor for
/// anything that moves (the first build's 80 ms, ui.md's 10-15 Hz, measured
/// ~10 fps on the X51v and read as ~5: "i specifically said ... i didnt
/// want anything under 30 fps"). A step cost ~4 ms of CPU on the X51v
/// (the run of 2026-11-01): ~12% at 30 a second.
pub const STEP_MS: u32 = 33;

/// Where the sky moves: the whole weather panel, under its words and its
/// picture (the first try kept to the picture's box, which the picture
/// all but covers: the owner saw only a smudge or two).
pub(crate) const AREA: Rect = Rect::new(17, 53, 446, 194);
const FALL: Rect = AREA;

/// The sky's colours, one a frame (its glyphs' coverage gives each its
/// strength), chosen for each look and left alone by the light look's
/// mapping ([`is_ink`]): mapped like text, rain and snow came out black on
/// the white glass (the owner, 2026-11-01: "rain and snow arent black").
/// Dark look: pale blue rain, white snow; light look: a deep blue rain, a
/// steel blue snow, slate clouds, dark on its light glass (the owner's
/// runs, 2026-11-01: "i cant see anything at all hardly with the living
/// sky" in light, with white snow and clouds and with blue-grey ones).
const RAIN_DARK: Argb = 0xFFB4_D2F2;
const SNOW_DARK: Argb = 0xFFF6_FAFF;
const RAIN_LIGHT: Argb = 0xFF24_5C9E;
const SNOW_LIGHT: Argb = 0xFF6E_90C0;
/// The clouds: near white by night, a dark grey on the light glass (a mid
/// grey "arent showing up great on light mode", the owner, 2026-11-01).
const CLOUD_DARK: Argb = 0xFFE4_ECF6;
const CLOUD_LIGHT: Argb = 0xFF56_5A60;

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

/// The clouds: eight whole clouds, four long and four small or double (the
/// owner, 2026-11-01: "make 3 of the clouds long as the 2 you have now, and
/// 3 small or double ... or 4"; earlier: one whole cloud "looks much better", clouds built of puffs
/// "look like cobbled together smaller clouds", "small ones can work along
/// side bigger ones"), soft uneven masses with wispy edges lit from above,
/// made offline by tools/shell-assets/sky_cloud.py (fractal noise bent by
/// noise in soft envelopes) to sit with VClouds' realistic pictures. They
/// never fade: each drifts in under one edge of the weather panel and out
/// under the other, as clouds cross a window (fades by glyph strength
/// stepped, the owner: "the fade animation is still jittery").
const BIG: (usize, usize) = (128, 40);
const SMALL: (usize, usize) = (80, 32);
const SHAPES: usize = 8;
const CLOUD_ALL: &[u8; 4 * BIG.0 * BIG.1 + 4 * SMALL.0 * SMALL.1] =
    include_bytes!("sky-cloud.alpha");

/// Big cloud `k`'s coverage.
const fn big(k: usize) -> [u8; BIG.0 * BIG.1] {
    let mut c = [0u8; BIG.0 * BIG.1];
    let mut i = 0;
    while i < c.len() {
        c[i] = CLOUD_ALL[k * BIG.0 * BIG.1 + i];
        i += 1;
    }
    c
}

/// Small cloud `k`'s coverage.
const fn small(k: usize) -> [u8; SMALL.0 * SMALL.1] {
    let mut c = [0u8; SMALL.0 * SMALL.1];
    let mut i = 0;
    while i < c.len() {
        c[i] = CLOUD_ALL[4 * BIG.0 * BIG.1 + k * SMALL.0 * SMALL.1 + i];
        i += 1;
    }
    c
}

/// The order clouds come in: long and small by turns.
const ORDER: [usize; SHAPES] = [0, 4, 1, 5, 2, 6, 3, 7];

static DROP: [u8; DROP_W * DROP_H] = drop_cover();
static FLAKE_PX: [u8; FLAKE * FLAKE] = flake_cover();
static BIG_PX: [[u8; BIG.0 * BIG.1]; 4] = [big(0), big(1), big(2), big(3)];
static SMALL_PX: [[u8; SMALL.0 * SMALL.1]; 4] = [small(0), small(1), small(2), small(3)];

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

const fn cloud_glyph(w: usize, h: usize, cover: &'static [u8]) -> Glyph {
    Glyph {
        w: w as u8,
        h: h as u8,
        // Its pen its right edge: entering from the left the pen is never
        // off the screen (a centred pen went below 0 and was held there, the
        // cloud standing still for its first ~50 px: the motion test).
        left: -(w as i16) as i8,
        top: (h / 2) as i8,
        advance: 0,
        cover,
    }
}

/// The sky's glyphs: spaces of 1 to 64 px (any gap in a few bytes), then a
/// drop and a flake, each centred on the pen and the baseline and moving it
/// nowhere.
static GLYPHS: [Glyph; 9] = [
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
];

/// The clouds, a font of their own: its atlas only in the clouds' colour
/// (in the sky's font, the rain's colour made a second copy).
static CLOUD_GLYPHS: [Glyph; SHAPES] = [
    cloud_glyph(BIG.0, BIG.1, &BIG_PX[0]),
    cloud_glyph(BIG.0, BIG.1, &BIG_PX[1]),
    cloud_glyph(BIG.0, BIG.1, &BIG_PX[2]),
    cloud_glyph(BIG.0, BIG.1, &BIG_PX[3]),
    cloud_glyph(SMALL.0, SMALL.1, &SMALL_PX[0]),
    cloud_glyph(SMALL.0, SMALL.1, &SMALL_PX[1]),
    cloud_glyph(SMALL.0, SMALL.1, &SMALL_PX[2]),
    cloud_glyph(SMALL.0, SMALL.1, &SMALL_PX[3]),
];

/// The sky's font: rain and snow (its line spans every glyph, for the
/// renderers' bounds).
pub static SKY: Font = Font {
    line: 40,
    ascent: 20,
    first: b' ',
    glyphs: &GLYPHS,
};

const RAIN: u8 = b' ' + 7;
const SNOW: u8 = b' ' + 8;

/// The clouds' font (its line spans a large cloud).
pub static CLOUDS: Font = Font {
    line: 60,
    ascent: 30,
    first: b'A',
    glyphs: &CLOUD_GLYPHS,
};

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

/// Cloud `g` of `clouds` crossing in `life` steps, at sky step `t`: its
/// birth number (every cloud's births counted in order: cloud g's are at
/// (c x clouds - g) x life / clouds) and how far across it is.
const fn birth(t: i64, g: i64, clouds: i64, life: i64) -> (i64, i64) {
    let at = t + g * life / clouds;
    (at.div_euclid(life) * clouds - g, at.rem_euclid(life))
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
    // Two or three at once (the owner, 2026-11-01: "two or 3, larger,
    // clouds on the screen at once").
    let clouds = if sky.cover < 50 { 2 } else { 3 };
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

    /// How long until the sky's next step (ms) while it moves, for the
    /// image's wait: woken when it is due, not a fixed sleep after it (a
    /// fixed 80 ms sleep plus the frame gave ~99 ms steps).
    #[must_use]
    pub fn sky_due_in(&self, now: u32) -> Option<u32> {
        self.sky_live()
            .then(|| STEP_MS.saturating_sub(now.wrapping_sub(self.sky_at)).max(1))
    }

    /// Whether only the sky changed since the last frame (the image then
    /// keeps the Weather app it composed ahead: nothing of it changed).
    #[must_use]
    pub fn only_sky_changed(&self) -> bool {
        self.damaged > 0
            && self.damage[..self.damaged]
                .iter()
                .all(|d| AREA.intersect(d) == *d)
    }

    /// The sky's next step if due at `now`: its area marked to draw.
    pub(crate) fn sky_step(&mut self, now: u32) -> bool {
        if !self.sky_live() || now.wrapping_sub(self.sky_at) < STEP_MS {
            return false;
        }
        self.sky_at = now;
        self.sky_n = self.sky_n.wrapping_add(1);
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
        // The clouds' own grey and the rain's or snow's colour (two
        // atlases: one colour for both made the light look's clouds blue
        // in rain, the owner's run, 2026-11-01).
        let cloud_ink = if self.light() {
            CLOUD_LIGHT
        } else {
            CLOUD_DARK
        };
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

        // Clouds: one after another across the panel, evenly spaced, each
        // drifting in under one edge and out under the other (the clip is
        // the panel's inside, its glass edge the window frame): those on
        // screen are always the last two or three to come in. Each takes
        // the next shape in turn of eight, large and small by turns, so
        // none seen together are alike, and its own height lane. A whole
        // pixel every few steps, evenly (by the clock, under a pixel a
        // step, a cloud hopped; a pixel every step it was "racing across
        // the screen"): one every 4 steps (7.5 px/s), every 2 in a wind of
        // 20 km/h and over.
        let pace = if kmh >= 20 { 2 } else { 4 };
        let dir = if across < 0 { -1 } else { 1 };
        let steps = i64::from(self.sky_n);
        let flash = w.storm && t % 5200 < 140;
        let clouds = i64::from(w.clouds.max(1));
        let path = i64::from(AREA.w) + BIG.0 as i64;
        let life = path * pace;
        for g in 0..clouds {
            let (j, u) = birth(steps, g, clouds, life);
            let s = scatter(j as u32);
            let shape = ORDER[j.rem_euclid(SHAPES as i64) as usize];
            // The cloud's right edge (its pen): from the panel's left edge
            // (all of it hidden) to past its right edge by its width.
            let along = u / pace;
            let x = if dir > 0 {
                along
            } else {
                i64::from(AREA.w) + BIG.0 as i64 - along
            };
            let lane = 120 / clouds;
            let y = 44 + j.rem_euclid(clouds) * lane + i64::from(s >> 9) % (lane / 3 + 1);
            let glyph = [b'A' + shape as u8; 2];
            // The storm's flash: a cloud twice over, lit.
            let n = if flash && g == 0 { 2 } else { 1 };
            f.text(
                (i64::from(AREA.x) + x).max(0) as u16,
                (i64::from(AREA.y) + y) as u16,
                &CLOUDS,
                cloud_ink,
                &glyph[..n],
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
mod lanes {
    use super::*;

    /// Clouds seen together are in different lanes and of different
    /// shapes, at any moment, for two or three clouds (shapes come in a
    /// round of eight, so three in a row are never alike).
    #[test]
    fn clouds_seen_together_never_share_a_lane_or_a_shape() {
        let life = (i64::from(AREA.w) + BIG.0 as i64) * 4;
        for clouds in 1..=3i64 {
            for t in (0..20_000i64).step_by(7) {
                let mut lanes = [false; 3];
                let mut shapes = [false; SHAPES];
                for g in 0..clouds {
                    let (j, _) = birth(t, g, clouds, life);
                    let lane = j.rem_euclid(clouds) as usize;
                    let shape = ORDER[j.rem_euclid(SHAPES as i64) as usize];
                    assert!(!lanes[lane] && !shapes[shape], "{clouds} clouds at {t}");
                    lanes[lane] = true;
                    shapes[shape] = true;
                }
            }
        }
    }
}
