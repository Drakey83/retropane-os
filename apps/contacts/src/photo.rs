//! A person's picture, kept small (contacts-rule0.md "Photos"; the owner
//! on the X51v, 2026-11-15: "tapping on the name icon in contacts doenst
//! give you the option to assign a picture").
//!
//! Iterate, don't imitate (rule 0): Windows Mobile 5 kept a contact's
//! picture as the JPEG file it was given (tens of kilobytes, a decoder to
//! show it); iOS and Android keep a full photo and a thumbnail. The
//! organiser's whole store is 256 KB, so ours keeps only what the screen
//! shows: the 96 px disc, nothing outside the circle, brightness at five
//! bits a pixel (dithered) and colour at a quarter of the resolution, four
//! bits a part, as the eye and video have it (4:2:0). About 6.9 KB a
//! person, no decoder beyond shifts and adds, and the 16-bit screen shows
//! no more than it holds.

use rp_app::Argb;

/// The picture's side, in pixels.
pub const SIDE: usize = 96;
/// The bytes the network's answer holds: the square in RGB565, little
/// endian, row by row.
pub const RAW: usize = SIDE * SIDE * 2;
/// The colour grid's side (a value for 2 x 2 pixels).
const HALF: usize = SIDE / 2;
/// The bytes of a page a kept picture is written in (a record of the
/// organiser's store each): it is packed a page at a time and read from
/// its pages where they lie, so no part of the app holds a whole one (in
/// the app's state it made the program's first frame 7,880 bytes, past
/// the stack rule's 4,096).
pub const PAGE: usize = 1_900;
/// The format's first byte.
const VERSION: u8 = 1;

/// The pixels of row `y` inside the disc: from, to (exclusive).
fn span(y: usize) -> (usize, usize) {
    let dy = 2 * y as i32 - (SIDE as i32 - 1);
    let mut x0 = 0usize;
    while x0 < HALF {
        let dx = 2 * x0 as i32 - (SIDE as i32 - 1);
        if dx * dx + dy * dy <= (SIDE * SIDE) as i32 {
            break;
        }
        x0 += 1;
    }
    (x0, SIDE - x0)
}

/// A raw pixel's red, green and blue, 0-255.
fn rgb(raw: &[u8], x: usize, y: usize) -> (i32, i32, i32) {
    let at = (y * SIDE + x) * 2;
    let p = u32::from(raw[at]) | u32::from(raw[at + 1]) << 8;
    let (r, g, b) = ((p >> 11) & 31, (p >> 5) & 63, p & 31);
    (
        (r << 3 | r >> 2) as i32,
        (g << 2 | g >> 4) as i32,
        (b << 3 | b >> 2) as i32,
    )
}

/// The square `raw` ([`RAW`] bytes) packed and handed to `keep` a page
/// at a time ([`PAGE`] bytes, the last one shorter): `None` for an answer
/// of another size, else whether every page was kept.
pub fn encode(raw: &[u8], keep: impl FnMut(&[u8]) -> bool) -> Option<bool> {
    if raw.len() != RAW {
        return None;
    }
    Some(encode_with(&|x, y| rgb(raw, x, y), keep))
}

/// [`encode`] from any source of the square's pixels (red, green, blue
/// 0-255 at x, y): a picture read from a JPEG file into picture memory.
/// Whether every page was kept.
pub fn encode_with(
    px: &dyn Fn(usize, usize) -> (i32, i32, i32),
    mut keep: impl FnMut(&[u8]) -> bool,
) -> bool {
    let rgb = |_: &[u8], x: usize, y: usize| px(x, y);
    let raw: &[u8] = &[];
    let mut out = [0u8; PAGE];
    let mut n = 0;
    let mut kept = true;
    let mut push = |b: u8, out: &mut [u8; PAGE], n: &mut usize| {
        out[*n] = b;
        *n += 1;
        if *n == PAGE {
            kept &= keep(out);
            *n = 0;
        }
    };
    push(VERSION, &mut out, &mut n);
    let (mut acc, mut bits) = (0u32, 0u32);
    for y in 0..SIDE {
        let (x0, x1) = span(y);
        for x in x0..x1 {
            let (r, g, b) = rgb(raw, x, y);
            let luma = (77 * r + 150 * g + 29 * b) >> 8;
            // Ordered dither, 2 x 2: five bits read as eight.
            let d = [[0, 128], [192, 64]][y & 1][x & 1];
            let y5 = ((luma * 31 + d) / 255).min(31) as u32;
            acc = acc << 5 | y5;
            bits += 5;
            while bits >= 8 {
                bits -= 8;
                push((acc >> bits) as u8, &mut out, &mut n);
            }
        }
    }
    if bits > 0 {
        push((acc << (8 - bits)) as u8, &mut out, &mut n);
    }
    for cy in 0..HALF {
        for cx in 0..HALF {
            let (mut r, mut g, mut b) = (0, 0, 0);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = rgb(raw, cx * 2 + dx, cy * 2 + dy);
                r += p.0;
                g += p.1;
                b += p.2;
            }
            let (r, g, b) = (r / 4, g / 4, b / 4);
            let cb = (-43 * r - 85 * g + 128 * b) >> 8;
            let cr = (128 * r - 107 * g - 21 * b) >> 8;
            // Steps of 17 either side of none, so grey stays grey (16
            // levels from 0 put no colour at 136 of 255: a tint).
            let four = |c: i32| (((c + 8 + 17 * 8).div_euclid(17) - 8).clamp(-8, 7) + 8) as u8;
            push(four(cb) << 4 | four(cr), &mut out, &mut n);
        }
    }
    let _ = push;
    if n > 0 {
        kept &= keep(&out[..n]);
    }
    kept
}

/// The kept picture in `pages` (as [`encode`] handed them, in order)
/// drawn into `px`, a disc `size` pixels across (`size * size` pixels,
/// clear outside the disc, its edge softened): each pixel the mean of
/// those it covers. `false` for pages that are not a picture.
pub fn decode(pages: &[&[u8]], size: usize, px: &mut [Argb]) -> bool {
    let len: usize = pages.iter().map(|p| p.len()).sum();
    let whole = pages.iter().rev().skip(1).all(|p| p.len() == PAGE);
    let byte = |i: usize| -> u8 {
        pages
            .get(i / PAGE)
            .and_then(|p| p.get(i % PAGE))
            .copied()
            .unwrap_or(0)
    };
    if !whole
        || byte(0) != VERSION
        || size == 0
        || size > SIDE
        || px.len() < size * size
        || len < 1 + HALF * HALF
    {
        return false;
    }
    let chroma = len - HALF * HALF;
    let px = &mut px[..size * size];
    px.fill(0);
    // Sums in each pixel while the picture is read: five bits a channel
    // (nine source pixels at most fall in one: 279 of 511) and a count.
    let (mut at, mut acc, mut bits) = (1usize, 0u32, 0u32);
    for y in 0..SIDE {
        let (x0, x1) = span(y);
        for x in x0..x1 {
            while bits < 5 {
                acc = acc << 8 | u32::from(byte(at));
                at += 1;
                bits += 8;
            }
            bits -= 5;
            let luma = ((acc >> bits) & 31) as i32 * 255 / 31;
            let c = byte(chroma + (y / 2) * HALF + x / 2);
            let (cb, cr) = ((i32::from(c >> 4) - 8) * 17, (i32::from(c & 15) - 8) * 17);
            // (Rounded: cut off, a picture came back 4 of 255 darker.)
            let five = |v: i32| (((v.clamp(0, 255) * 31 + 127) / 255) as u32).min(31);
            let r = five(luma + ((359 * cr) >> 8));
            let g = five(luma - ((88 * cb + 183 * cr) >> 8));
            let b = five(luma + ((454 * cb) >> 8));
            let to = (y * size / SIDE) * size + x * size / SIDE;
            px[to] += 1 << 27 | r << 18 | g << 9 | b;
        }
    }
    let edge = (size as i32 - 2) * (size as i32 - 2);
    for (i, p) in px.iter_mut().enumerate() {
        let n = *p >> 27;
        if n == 0 {
            continue;
        }
        let eight = |v: u32| {
            let v = ((v & 511) + n / 2) / n;
            v << 3 | v >> 2
        };
        let (dx, dy) = (
            2 * (i % size) as i32 - (size as i32 - 1),
            2 * (i / size) as i32 - (size as i32 - 1),
        );
        let alpha = if dx * dx + dy * dy > edge { 0x90 } else { 0xFF };
        *p = alpha << 24 | eight(*p >> 18) << 16 | eight(*p >> 9) << 8 | eight(*p);
    }
    true
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec;

    /// A square of colour: red rising to the right, green down, blue 96.
    fn square() -> std::vec::Vec<u8> {
        let mut raw = vec![0u8; RAW];
        for y in 0..SIDE {
            for x in 0..SIDE {
                let (r, g, b) = ((x * 255 / 95) as u16, (y * 255 / 95) as u16, 96u16);
                let p = (r >> 3) << 11 | (g >> 2) << 5 | b >> 3;
                raw[(y * SIDE + x) * 2..][..2].copy_from_slice(&p.to_le_bytes());
            }
        }
        raw
    }

    /// `raw` packed: its pages.
    fn packed(raw: &[u8]) -> std::vec::Vec<std::vec::Vec<u8>> {
        let mut out = std::vec::Vec::new();
        assert_eq!(
            encode(raw, |page| {
                out.push(page.to_vec());
                true
            }),
            Some(true)
        );
        out
    }

    #[test]
    fn a_picture_packs_small_and_comes_back_close() {
        let raw = square();
        let out = packed(&raw);
        let n: usize = out.iter().map(|p| p.len()).sum();
        assert!(
            n < 7_000 && out.len() == 4,
            "{n} bytes in {} pages",
            out.len()
        );
        assert_eq!(encode(&raw[..100], |_| true), None);
        assert_eq!(encode(&raw, |_| false), Some(false), "a page refused");
        let pages: std::vec::Vec<&[u8]> = out.iter().map(|p| &p[..]).collect();
        let mut px = vec![0u32; SIDE * SIDE];
        assert!(decode(&pages, SIDE, &mut px));
        // Clear outside the disc, whole inside; colours within the
        // format's steps (brightness dithered: compared over 4 x 4).
        assert_eq!(px[0], 0);
        assert_eq!(px[SIDE * 48 + 48] >> 24, 0xFF);
        let mut worst = 0;
        for by in (16..80).step_by(4) {
            for bx in (16..80).step_by(4) {
                let mut got = [0i32; 3];
                let mut want = [0i32; 3];
                for y in by..by + 4 {
                    for x in bx..bx + 4 {
                        let p = px[y * SIDE + x];
                        let w = rgb(&raw, x, y);
                        for (k, v) in [w.0, w.1, w.2].into_iter().enumerate() {
                            got[k] += ((p >> (16 - 8 * k)) & 255) as i32;
                            want[k] += v;
                        }
                    }
                }
                for k in 0..3 {
                    worst = worst.max((got[k] - want[k]).abs() / 16);
                }
            }
        }
        assert!(worst <= 20, "a block is {worst} of 255 off");
    }

    #[test]
    fn a_picture_draws_smaller_for_the_list() {
        let raw = square();
        let out = packed(&raw);
        let pages: std::vec::Vec<&[u8]> = out.iter().map(|p| &p[..]).collect();
        for size in [40usize, 64] {
            let mut px = vec![0u32; size * size];
            assert!(decode(&pages, size, &mut px));
            assert_eq!(px[0], 0, "{size}: the corner is clear");
            let mid = px[size * (size / 2) + size / 2];
            assert_eq!(mid >> 24, 0xFF);
            // The middle is the square's middle colour.
            assert!(((mid >> 16) & 255).abs_diff(128) < 24, "{mid:08X}");
            // Every pixel inside the disc is drawn.
            let holes = (0..size * size)
                .filter(|i| {
                    let (dx, dy) = (
                        2 * (i % size) as i32 - (size as i32 - 1),
                        2 * (i / size) as i32 - (size as i32 - 1),
                    );
                    dx * dx + dy * dy < (size as i32 - 3) * (size as i32 - 3) && px[*i] == 0
                })
                .count();
            assert_eq!(holes, 0, "{size}");
        }
        assert!(!decode(&[&[9, 9, 9]], 40, &mut [0u32; 1600]));
        // A page missing: no picture, never a wrong one.
        assert!(!decode(&pages[1..], 40, &mut [0u32; 1600]));
    }
}
