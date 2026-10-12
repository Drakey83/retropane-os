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
    };
    let video_fmt = fmt == 9 || fmt >= 0xC;
    let on = scadr >> 31 != 0
        && p.bpp != 0
        && video_fmt == video
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
                let s = pixel(p, vram, px, py);
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
