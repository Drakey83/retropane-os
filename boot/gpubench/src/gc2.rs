//! Run GC2's scenes and checks (ROADMAP step 3, the owner's robustness
//! rules, 2026-10-23), shared by the image and its host test: the test
//! renders every scene through the model of the 3D core
//! (`rp-mbx3d-render`'s `model` feature) and holds [`MODEL`] to it, and the
//! image checks its references at Windows Mobile's clock against
//! [`MODEL`] (rule 1). Run GC's seven kinds are the first seven here.

use rp_mbx3d_render::{Draw, Paint, PixelFormat, Texture, UV_ONE, address, format, untwiddle};

/// The pictures' size.
pub const W: u32 = 480;
pub const H: u32 = 640;
/// A band: rows hashed together and where a wrong pixel is placed
/// (rule 7).
pub const BAND_ROWS: u32 = 16;
pub const BANDS: usize = (H / BAND_ROWS) as usize;

/// The kinds of drawing: (name, the target's format).
pub const KINDS: [(&str, PixelFormat); 9] = [
    ("RGB565 Gouraud", PixelFormat::Rgb565),
    ("RGB565 dithered Gouraud", PixelFormat::Rgb565Dithered),
    ("ARGB8888 Gouraud", PixelFormat::Argb8888),
    ("RGB565 Gouraud + 8888 texture (FT2)", PixelFormat::Rgb565),
    ("ARGB8888 GS scene", PixelFormat::Argb8888),
    ("RGB565 bilinear 565 texture", PixelFormat::Rgb565),
    ("RGB565 Gouraud + blended 4444", PixelFormat::Rgb565),
    ("RGB565 Gouraud capped", PixelFormat::Rgb565),
    ("ARGB8888 Gouraud capped", PixelFormat::Argb8888),
];

const GREY: u32 = 0xFF20_2020;
const WHITE: u32 = 0xFFFF_FFFF;
const BLACK: u32 = 0xFF00_0000;
const BLUE: u32 = 0xFF10_20A0;
const YELLOW: u32 = 0xFFFF_E000;

/// The Gouraud corners (top left, top right, bottom right, bottom left).
const CORNERS: [u32; 4] = [BLACK, WHITE, BLUE, YELLOW];
/// The same, every colour channel one RGB565 step below its top (red and
/// blue at most 30 of 31, `0xF7`; green 62 of 63, `0xFB`), so a +1 step
/// can't wrap at white (the roadmap's idea (c)).
const CORNERS_565_CAPPED: [u32; 4] = [BLACK, 0xFFF7_FBF7, BLUE, 0xFFF7_E000];
/// The same for ARGB8888: every channel, alpha too, at most `0xFE`.
const CORNERS_8888_CAPPED: [u32; 4] = [0xFE00_0000, 0xFEFE_FEFE, 0xFE10_20A0, 0xFEFE_E000];

/// The textures the scenes use: RGB565 (side 128, drawn nearest and
/// bilinear), ARGB4444 (side 128), ARGB8888 (side 32, clamped).
#[derive(Clone, Copy)]
pub struct Tex {
    pub t565: Texture,
    pub t4444: Texture,
    pub t8888: Texture,
}

const fn texture(code: u8, f: u32, mode: u32) -> Texture {
    Texture {
        at: 0,
        code,
        format: f as u8,
        u_mode: mode as u8,
        v_mode: mode as u8,
        linear: false,
        levels: 1,
    }
}

/// [`Tex`]'s textures as described, not yet placed (`at` 0): the caller
/// takes memory for each and writes [`texture_words`] there.
pub const TEXTURES: [Texture; 3] = [
    texture(4, format::RGB565, address::WRAP),
    texture(4, format::ARGB4444, address::WRAP),
    texture(2, format::ARGB8888, address::CLAMP),
];

/// Texture `n` of [`TEXTURES`] word by word in its twiddled order:
/// `put(word index, word)`.
pub fn texture_words(n: usize, mut put: impl FnMut(u32, u32)) {
    let t = TEXTURES[n];
    let texels = t.side() * t.side();
    if n == 2 {
        for i in 0..texels {
            let (x, y) = untwiddle(i);
            put(i, texel8888(x, y));
        }
        return;
    }
    let f = if n == 0 { texel565 } else { texel4444 };
    for k in 0..texels / 2 {
        let (x0, y0) = untwiddle(2 * k);
        let (x1, y1) = untwiddle(2 * k + 1);
        put(k, u32::from(f(x0, y0)) | u32::from(f(x1, y1)) << 16);
    }
}

/// The RGB565 texture's texel: stripes and a ramp.
pub fn texel565(x: u32, y: u32) -> u16 {
    let r = (x >> 4) & 31;
    let gr = ((x + y) >> 3) & 63;
    let b = (y >> 4) & 31;
    (r << 11 | gr << 5 | b) as u16
}

/// The ARGB4444 texture's texel: alpha 8/15 (half see-through), stripes.
pub fn texel4444(x: u32, y: u32) -> u16 {
    (0x8000 | ((x >> 5) & 15) << 8 | ((y >> 5) & 15) << 4 | ((x ^ y) >> 4) & 15) as u16
}

/// The ARGB8888 texture's texel: red `x * 8`, green `y * 8`, blue `0xA0`.
pub const fn texel8888(x: u32, y: u32) -> u32 {
    0xFF00_00A0 | (x << 3) << 16 | (y << 3) << 8
}

/// The whole target covered by `tex` at `scale` texels a pixel, in
/// sixteenths (16: 1:1).
fn cover(tex: Texture, scale: i32, blend: bool) -> Draw {
    let side = tex.side() as i32;
    let u1 = (W as i32 * scale / 16) * (UV_ONE / side);
    let v1 = (H as i32 * scale / 16) * (UV_ONE / side);
    let d = Draw::rect(
        0,
        0,
        W as i32,
        H as i32,
        Paint::Texture {
            tex,
            uv: [(0, 0), (u1, 0), (u1, v1), (0, v1)],
            tint: WHITE,
        },
    );
    if blend { d.over() } else { d }
}

fn flat(c: u32, blend: bool) -> Draw {
    let d = Draw::rect(0, 0, W as i32, H as i32, Paint::Flat(c));
    if blend { d.over() } else { d }
}

/// Kind `k`'s scene: the background and three slots.
#[must_use]
pub fn scene(k: usize, t: Tex) -> (u32, [Option<Draw>; 3]) {
    let shade = |c| Draw::rect(0, 0, W as i32, H as i32, Paint::Gouraud(c));
    let gouraud = shade(CORNERS);
    let mut lin = t.t565;
    lin.linear = true;
    match k {
        0..=2 => (BLACK, [Some(gouraud), None, None]),
        3 => (
            BLACK,
            [
                Some(gouraud),
                Some(Draw::rect(40, 40, 400, 560, Paint::texture(t.t8888))),
                None,
            ],
        ),
        4 => (
            GREY,
            [
                Some(cover(t.t565, 16, false)),
                Some(flat(0x8040_A0C0, true)),
                Some(Draw::rect(100, 200, 96, 96, Paint::Flat(0xFFFF_2000))),
            ],
        ),
        5 => (GREY, [Some(cover(lin, 11, false)), None, None]),
        6 => (BLACK, [Some(gouraud), Some(cover(t.t4444, 16, true)), None]),
        7 => (BLACK, [Some(shade(CORNERS_565_CAPPED)), None, None]),
        _ => (BLACK, [Some(shade(CORNERS_8888_CAPPED)), None, None]),
    }
}

/// Words in kind `k`'s picture.
#[must_use]
pub const fn words(k: usize) -> u32 {
    W * H * KINDS[k].1.bytes() / 4
}

/// Words in one band of kind `k`'s picture.
#[must_use]
pub const fn band_words(k: usize) -> u32 {
    words(k) / BANDS as u32
}

/// The bands' hash: FNV-1a over words, from [`HASH_START`].
#[must_use]
pub const fn fold(h: u32, w: u32) -> u32 {
    (h ^ w).wrapping_mul(0x0100_0193)
}

pub const HASH_START: u32 = 0x811C_9DC5;

/// Each kind's band hashes as the model renders it (the host test holds
/// this table to the model; regenerate it from the test's message).
pub const MODEL: [[u32; BANDS]; 9] = include!("gc2_model.in");

/// How a wrong pixel differs from the reference (rule 7). Channel by
/// channel (alpha too, on ARGB8888): a channel that held its top value or
/// one below and dropped by more than half its range wrapped; otherwise
/// every channel 0 or +1 is "+1 step", 0 or -1 "-1 step", anything else
/// "other".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Miss {
    Plus1 = 0,
    Minus1 = 1,
    Wrapped = 2,
    Other = 3,
}

/// The channels of a pixel in `f`: (shift, bits).
const fn channels(f: PixelFormat) -> &'static [(u32, u32)] {
    match f {
        PixelFormat::Argb8888 => &[(24, 8), (16, 8), (8, 8), (0, 8)],
        PixelFormat::Rgb565 | PixelFormat::Rgb565Dithered => &[(11, 5), (5, 6), (0, 5)],
        PixelFormat::Argb1555 => &[(15, 1), (10, 5), (5, 5), (0, 5)],
    }
}

/// How `got` differs from `want` (two different pixels in `f`).
#[must_use]
pub fn miss(f: PixelFormat, want: u32, got: u32) -> Miss {
    let (mut up, mut down, mut wrapped) = (false, false, false);
    let mut other = false;
    for &(shift, bits) in channels(f) {
        let max = (1i32 << bits) - 1;
        let w = (want >> shift) as i32 & max;
        let g = (got >> shift) as i32 & max;
        match g - w {
            0 => {}
            1 => up = true,
            -1 => down = true,
            d if w >= max - 1 && -d > max / 2 => wrapped = true,
            _ => other = true,
        }
    }
    if wrapped {
        Miss::Wrapped
    } else if other || (up && down) {
        Miss::Other
    } else if up {
        Miss::Plus1
    } else {
        Miss::Minus1
    }
}

/// The wrong pixels of one or more renders of a kind, described (rule 7).
#[derive(Clone, Copy, Default)]
pub struct Tally {
    /// Pixels by [`Miss`].
    pub by: [u32; 4],
    /// Bit `b`: a wrong pixel in band `b` (16 rows).
    pub bands: u64,
    /// The wrong pixels' bounding box: x0, y0, x1, y1 (inclusive).
    pub bbox: Option<(u16, u16, u16, u16)>,
    /// The first three: x, y, the reference's pixel, the read one.
    pub first: [(u16, u16, u32, u32); 3],
    pub n_first: usize,
}

impl Tally {
    /// Every wrong pixel.
    #[must_use]
    pub fn pixels(&self) -> u32 {
        self.by.iter().sum()
    }

    /// Word `i` of a picture in `f` read as `got` where the reference
    /// holds `want`: each differing pixel in it counted.
    pub fn add(&mut self, f: PixelFormat, i: u32, want: u32, got: u32) {
        let per = if f.bytes() == 4 { 1 } else { 2 };
        let (bits, mask) = if per == 1 {
            (32, u32::MAX)
        } else {
            (16, 0xFFFF)
        };
        for p in 0..per {
            let (a, b) = (want >> (bits * p) & mask, got >> (bits * p) & mask);
            if a == b {
                continue;
            }
            let at = i * per + p;
            let (x, y) = ((at % W) as u16, (at / W) as u16);
            self.by[miss(f, a, b) as usize] += 1;
            self.bands |= 1 << (u32::from(y) / BAND_ROWS);
            self.bbox = Some(match self.bbox {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
            if self.n_first < self.first.len() {
                self.first[self.n_first] = (x, y, a, b);
                self.n_first += 1;
            }
        }
    }

    /// `o`'s pixels added to these (the first three kept from whichever
    /// came first).
    pub fn merge(&mut self, o: &Self) {
        for (a, b) in self.by.iter_mut().zip(o.by) {
            *a += b;
        }
        self.bands |= o.bands;
        self.bbox = match (self.bbox, o.bbox) {
            (None, b) | (b, None) => b,
            (Some((a0, b0, a1, b1)), Some((c0, d0, c1, d1))) => {
                Some((a0.min(c0), b0.min(d0), a1.max(c1), b1.max(d1)))
            }
        };
        for &f in &o.first[..o.n_first] {
            if self.n_first < self.first.len() {
                self.first[self.n_first] = f;
                self.n_first += 1;
            }
        }
    }
}

#[cfg(test)]
#[path = "gc2_tests.rs"]
mod tests;
