//! What the display controller shows, computed from its registers and
//! memory: the reference model of docs/hw/2700g.md "Plane composition".
//!
//! Host tests and `cargo xtask emu` compare the emulator's screen with this
//! exactly; the device run (`rp-blend`) checks the spec itself. Kept apart
//! from the emulator's C model on purpose: two implementations of one
//! written rule, so a slip in either shows.
//!
//! The rule: each plane's pixel is read as 8-bit `(r, g, b, a)`; planes are
//! mixed from the bottom slot up, starting from `BGCOLOR`:
//! `out = (src * k + out * (255 - k) + 127) / 255`, where `k` is 255
//! (opaque), the pixel's alpha, 255 minus it (inverse), or the plane's
//! global alpha. YUV is BT.601 studio range.
//!
//! Gamma (graphics plane, while `GSCTRL` bit 26 is set): each 8-bit lane
//! of the plane's pixel goes through the 17-entry table `GFXGAM0`-`16`
//! (`0x00RRGGBB`, one 8-bit point per lane every 16 levels, entry 16 at
//! 256) before mixing, linear between points ([`gamma`]). The table's form
//! and Windows Mobile's grey ramp are measured (`rp-nightlight`,
//! 2026-10-16); the interpolation is this model's assumption.

use crate::layers::reg as lreg;
use crate::reg;

/// A pixel as the mixer sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha (255 opaque).
    pub a: u8,
}

/// BT.601 studio-range YCbCr to RGB, in the integer form the model uses.
#[must_use]
pub fn yuv_to_rgb(y: u8, u: u8, v: u8) -> (u8, u8, u8) {
    let c = i32::from(y) - 16;
    let d = i32::from(u) - 128;
    let e = i32::from(v) - 128;
    let clamp = |x: i32| x.clamp(0, 255) as u8;
    (
        clamp((298 * c + 409 * e + 128) >> 8),
        clamp((298 * c - 100 * d - 208 * e + 128) >> 8),
        clamp((298 * c + 516 * d + 128) >> 8),
    )
}

/// RGB to BT.601 studio-range YCbCr (the encoder's half of the pair).
#[must_use]
pub const fn rgb_to_yuv(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    (
        (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8,
        (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128) as u8,
        (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128) as u8,
    )
}

/// Entries in the gamma table.
pub const GAMMA_ENTRIES: usize = 17;

/// Windows Mobile's gamma table as it leaves it on the X51v (a grey ramp,
/// `rp-nightlight` 2026-10-16).
pub const WM_GAMMA: [u32; GAMMA_ENTRIES] = {
    const L: [u32; GAMMA_ENTRIES] = [
        0x00, 0x0F, 0x1F, 0x2F, 0x3F, 0x4F, 0x5F, 0x6F, 0x7F, 0x8E, 0x9E, 0xAE, 0xBE, 0xCE, 0xDE,
        0xEE, 0xFD,
    ];
    let mut t = [0; GAMMA_ENTRIES];
    let mut i = 0;
    while i < GAMMA_ENTRIES {
        t[i] = L[i] * 0x01_0101;
        i += 1;
    }
    t
};

/// Lane `shift` (16 red, 8 green, 0 blue) of `v` through `table`: linear
/// between the points at `16 * i`, truncated toward zero.
#[must_use]
pub fn gamma(table: &[u32; GAMMA_ENTRIES], v: u8, shift: u32) -> u8 {
    let lane = |i: usize| (table[i] >> shift & 0xFF) as i32;
    let (i, f) = (usize::from(v >> 4), i32::from(v & 15));
    (lane(i) + (lane(i + 1) - lane(i)) * f / 16) as u8
}

/// Each lane of `t` scaled by `s`/256 (`s` up to 256): the picture
/// dimmed through the gamma table, as brightness's picture stage
/// (rp-drv-pxa-pwm's `split`) and `rp-nightlight` use it.
#[must_use]
pub fn scale_gamma(t: &[u32; GAMMA_ENTRIES], s: u32) -> [u32; GAMMA_ENTRIES] {
    core::array::from_fn(|i| {
        let lane = |shift: u32| ((t[i] >> shift & 0xFF) * s.min(256) / 256) << shift;
        lane(16) | lane(8) | lane(0)
    })
}

/// One step of the mix: `src` over `dst` with weight `k` (0..=255).
#[must_use]
pub const fn mix(src: u8, dst: u8, k: u8) -> u8 {
    let (s, d, k) = (src as u32, dst as u32, k as u32);
    ((s * k + d * (255 - k) + 127) / 255) as u8
}

#[derive(Clone, Copy)]
struct Plane {
    fmt: u32,
    bpp: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    base: usize,
    stride: usize,
    ubase: usize,
    vbase: usize,
    uvhalf: bool,
    csc: bool,
    mode: u32,
    slot: u32,
    glalpha: u8,
    gamma: Option<[u32; GAMMA_ENTRIES]>,
}

const fn bytes_per_pixel(fmt: u32) -> usize {
    match fmt {
        4 | 5 | 7 | 0xC..=0xF => 2,
        6 => 3,
        8 => 4,
        9 => 1,
        _ => 0,
    }
}

fn plane(read: &impl Fn(usize) -> u32, vram_len: usize, video: bool) -> Option<Plane> {
    let pick = |g: usize, v: usize| read(if video { v } else { g });
    let sctrl = pick(reg::GSCTRL, lreg::VSCTRL);
    let scadr = pick(reg::GSCADR, lreg::VSCADR);
    let sadr = pick(reg::GSADR, lreg::VSADR);
    let bbase = pick(reg::GBBASE, lreg::VBBASE);
    let vubase = read(lreg::VUBASE);
    let fmt = sctrl >> 27 & 0xF;
    let p = Plane {
        fmt,
        bpp: bytes_per_pixel(fmt),
        x: (sadr >> 11 & 0x7FF) as usize,
        y: (sadr & 0x7FF) as usize,
        width: (sctrl >> 11 & 0x7FF) as usize + 1,
        height: (sctrl & 0x7FF) as usize + 1,
        base: (scadr & 0x7F_FFFF) as usize * 16,
        stride: ((sadr >> 22 & 0x3FF) as usize + 1) * 16,
        ubase: (vubase & 0xFF_FFFF) as usize * 8,
        vbase: (read(lreg::VVBASE) & 0xFF_FFFF) as usize * 8,
        uvhalf: vubase >> 31 != 0,
        csc: sctrl >> 25 & 1 != 0,
        mode: scadr >> 27 & 3,
        slot: scadr >> 24 & 3,
        glalpha: (bbase >> 24) as u8,
        gamma: (!video && sctrl >> 26 & 1 != 0)
            .then(|| core::array::from_fn(|i| read(reg::GFXGAM0 + 4 * i))),
    };
    // YUV formats are the video plane's alone; the video plane also takes
    // the graphics plane's RGB formats (run D on the X51v, 2026-10-19:
    // `VSCTRL` 7 showed an RGB565 card exactly, 4, 5, 6 and 8 read as
    // ARGB4444, ARGB1555, RGB888 and ARGB8888).
    let video_fmt = fmt == 9 || fmt >= 0xC;
    let on = scadr >> 31 != 0
        && p.bpp != 0
        && (video || !video_fmt)
        && (!video || read(lreg::VOVRCLK) & 1 != 0)
        && p.base + (p.height - 1) * p.stride + p.width * p.bpp <= vram_len;
    if !on {
        return None;
    }
    if fmt == 9 {
        let rows = if p.uvhalf {
            p.height.div_ceil(2)
        } else {
            p.height
        };
        let end = (rows - 1) * (p.stride / 2) + p.width.div_ceil(2);
        if p.ubase + end > vram_len || p.vbase + end > vram_len {
            return None;
        }
    }
    Some(p)
}

fn pixel(p: &Plane, vram: &[u8], px: usize, py: usize) -> Rgba {
    let row = p.base + py * p.stride;
    let le16 = |o: usize| u16::from_le_bytes([vram[o], vram[o + 1]]);
    let x4 = |v: u16, s: u32| ((v >> s & 0xF) * 17) as u8;
    let (y, u, v) = match p.fmt {
        4 => {
            let v = le16(row + px * 2);
            return Rgba {
                r: x4(v, 8),
                g: x4(v, 4),
                b: x4(v, 0),
                a: x4(v, 12),
            };
        }
        5 => {
            let v = le16(row + px * 2);
            let c = |s: u32| ((v >> s & 0x1F) << 3) as u8;
            return Rgba {
                r: c(10),
                g: c(5),
                b: c(0),
                a: if v & 0x8000 != 0 { 255 } else { 0 },
            };
        }
        7 => {
            let v = le16(row + px * 2);
            return Rgba {
                r: ((v >> 11 & 0x1F) << 3) as u8,
                g: ((v >> 5 & 0x3F) << 2) as u8,
                b: ((v & 0x1F) << 3) as u8,
                a: 255,
            };
        }
        6 => {
            let q = row + px * 3;
            return Rgba {
                r: vram[q + 2],
                g: vram[q + 1],
                b: vram[q],
                a: 255,
            };
        }
        8 => {
            let q = row + px * 4;
            return Rgba {
                r: vram[q + 2],
                g: vram[q + 1],
                b: vram[q],
                a: vram[q + 3],
            };
        }
        9 => {
            let c = if p.uvhalf { py / 2 } else { py } * (p.stride / 2) + px / 2;
            (vram[row + px], vram[p.ubase + c], vram[p.vbase + c])
        }
        fmt => {
            let m = &vram[row + (px & !1) * 2..][..4];
            let odd = px & 1;
            match fmt {
                0xC => (m[1 + 2 * odd], m[0], m[2]),
                0xD => (m[1 + 2 * odd], m[2], m[0]),
                0xE => (m[2 * odd], m[1], m[3]),
                _ => (m[2 * odd], m[3], m[1]),
            }
        }
    };
    let (r, g, b) = if p.csc {
        yuv_to_rgb(y, u, v)
    } else {
        (y, u, v)
    };
    Rgba { r, g, b, a: 255 }
}

/// The panel's active size: the display-enable window, else the graphics
/// plane's size, else 480 × 640.
#[must_use]
pub fn screen_size(read: impl Fn(usize) -> u32, vram_len: usize) -> (usize, usize) {
    let span = |v: u32| (v & 0xFFF) as i32 - (v >> 16 & 0xFFF) as i32;
    let (w, h) = (span(read(reg::DHDET)), span(read(reg::DVDET)));
    if (1..=2048).contains(&w) && (1..=2048).contains(&h) {
        return (w as usize, h as usize);
    }
    plane(&read, vram_len, false).map_or((480, 640), |p| (p.width, p.height))
}

/// Computes the screen into `out` (RGB888, `w * h * 3` bytes for
/// [`screen_size`]'s `(w, h)`), from register values (`read`) and the
/// 2700G's memory.
///
/// # Panics
/// If `out` is shorter than the screen.
pub fn compose(read: impl Fn(usize) -> u32, vram: &[u8], out: &mut [u8]) {
    let (w, h) = screen_size(&read, vram.len());
    let sync = read(reg::DSCTRL) >> 31 != 0;
    let bg = read(lreg::BGCOLOR);
    let planes = [
        plane(&read, vram.len(), false),
        plane(&read, vram.len(), true),
    ];
    let mut order: [Option<Plane>; 2] = [None; 2];
    let mut n = 0;
    for slot in 0..4 {
        for p in planes.iter().flatten().filter(|p| p.slot == slot) {
            order[n] = Some(*p);
            n += 1;
        }
    }
    for y in 0..h {
        for x in 0..w {
            let mut c = [(bg >> 16) as u8, (bg >> 8) as u8, bg as u8];
            if !sync {
                c = [0; 3];
            }
            for p in order.iter().flatten().filter(|_| sync) {
                let (Some(px), Some(py)) = (x.checked_sub(p.x), y.checked_sub(p.y)) else {
                    continue;
                };
                if px >= p.width || py >= p.height {
                    continue;
                }
                let mut s = pixel(p, vram, px, py);
                if let Some(t) = &p.gamma {
                    s.r = gamma(t, s.r, 16);
                    s.g = gamma(t, s.g, 8);
                    s.b = gamma(t, s.b, 0);
                }
                let k = match p.mode {
                    3 => s.a,
                    1 => 255 - s.a,
                    2 => p.glalpha,
                    _ => 255,
                };
                c = [mix(s.r, c[0], k), mix(s.g, c[1], k), mix(s.b, c[2], k)];
            }
            out[(y * w + x) * 3..][..3].copy_from_slice(&c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yuv_round_trip_is_close_for_every_grey_and_primary() {
        for (r, g, b) in [
            (0, 0, 0),
            (255, 255, 255),
            (255, 0, 0),
            (0, 255, 0),
            (0, 0, 255),
            (128, 64, 200),
        ] {
            let (y, u, v) = rgb_to_yuv(r, g, b);
            let (r2, g2, b2) = yuv_to_rgb(y, u, v);
            for (a, z) in [(r, r2), (g, g2), (b, b2)] {
                assert!(a.abs_diff(z) <= 3, "{r},{g},{b} -> {r2},{g2},{b2}");
            }
        }
        // Studio range: black is 16, white 235.
        assert_eq!(rgb_to_yuv(0, 0, 0), (16, 128, 128));
        assert_eq!(rgb_to_yuv(255, 255, 255), (235, 128, 128));
    }

    #[test]
    fn gamma_is_linear_between_the_points() {
        let identity: [u32; GAMMA_ENTRIES] =
            core::array::from_fn(|i| (i as u32 * 16).min(255) * 0x01_0101);
        for v in 0..=239u8 {
            assert_eq!(gamma(&identity, v, 8), v);
        }
        // Windows Mobile's ramp: black stays black, white loses a little.
        assert_eq!(gamma(&WM_GAMMA, 0, 16), 0);
        assert_eq!(gamma(&WM_GAMMA, 16, 16), 0x0F);
        assert_eq!(gamma(&WM_GAMMA, 255, 0), 0xEE + 15 * 15 / 16);
        // Lanes are apart: red only.
        let red: [u32; GAMMA_ENTRIES] = core::array::from_fn(|i| identity[i] & 0xFF_0000);
        assert_eq!((gamma(&red, 200, 16), gamma(&red, 200, 8)), (200, 0));
    }

    #[test]
    fn compose_applies_gamma_to_the_graphics_plane_only_while_enabled() {
        // A 1 x 1 RGB565 white graphics plane, sync on.
        let vram = [0xFF, 0xFF];
        let mut regs = [0u32; 0x4000 / 4];
        regs[reg::GSCTRL / 4] = 7 << 27;
        regs[reg::GSCADR / 4] = 1 << 31;
        regs[reg::DSCTRL / 4] = 1 << 31;
        let half = scale_gamma(&WM_GAMMA, 128);
        assert_eq!(half[16], 0x7E_7E7E);
        assert_eq!(scale_gamma(&WM_GAMMA, 256), WM_GAMMA);
        for (i, w) in half.iter().enumerate() {
            regs[reg::GFXGAM0 / 4 + i] = *w;
        }
        let shot = |regs: &[u32]| {
            let mut out = [0u8; 3];
            compose(|o| regs[o / 4], &vram, &mut out);
            out
        };
        assert_eq!(shot(&regs), [248, 252, 248]);
        regs[reg::GSCTRL / 4] |= 1 << 26;
        assert_eq!(
            shot(&regs),
            [
                gamma(&half, 248, 16),
                gamma(&half, 252, 8),
                gamma(&half, 248, 0)
            ]
        );
        assert!(shot(&regs)[1] < 128);
    }

    #[test]
    fn the_video_plane_takes_rgb_and_its_per_pixel_alpha_over_the_graphics_plane() {
        // Weather sheet L2: an RGB565 screen (graphics, slot 0, opaque) and
        // a 2 x 1 ARGB8888 window on the video plane above it (slot 1,
        // per-pixel alpha) at x 1: one clear pixel, one half-alpha red.
        let mut vram = vec![0u8; 64];
        for px in 0..3 {
            vram[px * 2..px * 2 + 2].copy_from_slice(&0x001Fu16.to_le_bytes()); // blue
        }
        vram[16..20].copy_from_slice(&[0, 0, 0, 0]); // clear
        vram[20..24].copy_from_slice(&[0, 0, 255, 128]); // B G R A: red, half
        let mut regs = [0u32; 0x4000 / 4];
        regs[reg::GSCTRL / 4] = 7 << 27 | 2 << 11; // RGB565, 3 x 1
        regs[reg::GSCADR / 4] = 1 << 31; // slot 0, opaque
        regs[lreg::VSCTRL / 4] = 8 << 27 | 1 << 11; // ARGB8888, 2 x 1
        regs[lreg::VSCADR / 4] = 1 << 31 | 3 << 27 | 1 << 24 | 1; // per-pixel, slot 1, at byte 16
        regs[lreg::VSADR / 4] = 1 << 11; // stride 16 bytes, x 1
        regs[lreg::VOVRCLK / 4] = 1;
        regs[reg::DSCTRL / 4] = 1 << 31;
        let mut out = [0u8; 9];
        compose(|o| regs[o / 4], &vram, &mut out);
        assert_eq!(&out[0..3], &[0, 0, 248], "outside the window: the screen");
        assert_eq!(
            &out[3..6],
            &[0, 0, 248],
            "alpha 0: the screen shows through"
        );
        assert_eq!(
            &out[6..9],
            &[mix(255, 0, 128), 0, mix(0, 248, 128)],
            "half alpha: mixed"
        );
        // A YUV format stays the video plane's alone.
        regs[reg::GSCTRL / 4] = 0xC << 27 | 2 << 11;
        assert!(plane(&|o| regs[o / 4], vram.len(), false).is_none());
        regs[lreg::VSCTRL / 4] = 0xC << 27 | 1 << 11;
        assert!(plane(&|o| regs[o / 4], vram.len(), true).is_some());
    }

    #[test]
    fn mix_ends_are_exact() {
        for s in [0, 1, 127, 254, 255] {
            for d in [0, 99, 255] {
                assert_eq!(mix(s, d, 255), s);
                assert_eq!(mix(s, d, 0), d);
            }
        }
        assert_eq!(mix(255, 0, 128), 128);
    }
}
