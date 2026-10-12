use super::*;
use crate::dirty::Dirty;

fn rng(seed: u32) -> impl FnMut() -> u32 {
    let mut x = seed;
    move || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    }
}

#[test]
fn divide_by_255_without_a_divide_is_exact_in_both_lanes() {
    for x in 0..=255 * 255 {
        let want = (x + 127) / 255;
        assert_eq!(div255(x), want, "{x}");
        // The other lane carrying its own value never disturbs this one.
        let y = 255 * 255 - x;
        assert_eq!(
            div255_lanes(x << 16 | y),
            want << 16 | ((y + 127) / 255),
            "{x}"
        );
    }
}

/// Colours that cover every channel extreme, plus a few in between.
const COLOURS: [u32; 8] = [
    0x00_0000, 0xFF_FFFF, 0xFF_0000, 0x00_FF00, 0x00_00FF, 0x12_3456, 0x80_7F81, 0xFE_01FE,
];

#[test]
fn over_takes_its_short_paths_with_exactly_the_reference_result() {
    // Every pair of alphas, with colours at the extremes.
    for sa in 0..=255u32 {
        for da in 0..=255u32 {
            for (i, &s) in COLOURS.iter().enumerate() {
                let d = COLOURS[(i * 3 + 1) % COLOURS.len()];
                let (s, d) = (sa << 24 | s, da << 24 | d);
                assert_eq!(over(s, d), over_reference(s, d), "{s:#010x} over {d:#010x}");
            }
        }
    }
    let mut r = rng(11);
    for _ in 0..200_000 {
        let (s, d) = (r(), r());
        assert_eq!(over(s, d), over_reference(s, d), "{s:#010x} over {d:#010x}");
    }
}

#[test]
fn over_obeys_the_rules_of_glass() {
    let glass = 0x80_FF_FF_FF;
    // Solid replaces, clear keeps, anything over clear is itself.
    assert_eq!(over(0xFF12_3456, glass), 0xFF12_3456);
    assert_eq!(over(0x0012_3456, glass), glass);
    assert_eq!(over(glass, CLEAR), glass);
    assert_eq!(over(0x0012_3456, 0x0065_4321), CLEAR);
    // Over an opaque pixel the result is opaque, and it's the plain mix the
    // chip itself does.
    let mix = |s: u32, d: u32, k: u32| (s * k + d * (255 - k) + 127) / 255;
    for a in 0..=255 {
        let o = over(a << 24 | 0x00C8_6420, 0xFF10_2030);
        assert_eq!(o >> 24, 255);
        assert_eq!(o >> 16 & 0xFF, mix(0xC8, 0x10, a));
        assert_eq!(o & 0xFF, mix(0x20, 0x30, a));
    }
    // Text over glass: more opaque than the glass, never less.
    let mut r = rng(5);
    for _ in 0..50_000 {
        let (s, d) = (r(), r());
        let o = over(s, d) >> 24;
        assert!(o >= (s >> 24).max(d >> 24), "{s:#x} {d:#x}");
    }
}

#[test]
fn the_fast_dither_is_the_approved_one_for_every_value_and_place() {
    for v in 0..=255u32 {
        // Each channel takes every value somewhere, with the others busy.
        let c = v << 24 | (255 - v) << 16 | (v ^ 0x5A) << 8 | ((v * 7) & 0xFF);
        for y in 0..4 {
            for x in 0..4 {
                let tt = threshold(x, y) * 0x0001_0001;
                assert_eq!(
                    dither_fast(c, tt),
                    to_4444_dithered(c, x, y),
                    "{c:#x} {x},{y}"
                );
            }
        }
    }
    let mut r = rng(9);
    for _ in 0..100_000 {
        let (c, x, y) = (r(), r() as usize, r() as usize);
        assert_eq!(
            dither_fast(c, threshold(x, y) * 0x0001_0001),
            to_4444_dithered(c, x, y)
        );
    }
}

#[test]
fn the_dither_keeps_the_ends_exact_and_averages_to_the_value() {
    for y in 0..4 {
        for x in 0..4 {
            assert_eq!(to_4444_dithered(0xFFFF_FFFF, x, y), 0xFFFF);
            assert_eq!(to_4444_dithered(0, x, y), 0);
        }
    }
    // Over the 16 places, a channel averages to within half a 4-bit step
    // of its 8-bit value: smooth gradients, no bands.
    for v in 0..=255u32 {
        let sum: u32 = (0..16)
            .map(|i| u32::from(to_4444_dithered(v, i % 4, i / 4) & 0xF))
            .sum();
        let mean = f64::from(sum) / 16.0 * 17.0;
        assert!((mean - f64::from(v)).abs() <= 8.5, "{v}: {mean}");
    }
}

const W: u32 = 48;
const H: u32 = 20;

fn random_layer(px: &mut [Argb], seed: u32) -> Layer<'_> {
    let mut r = rng(seed);
    px.iter_mut().for_each(|p| *p = r());
    Layer::new(px, W, H, W as usize + 3).unwrap()
}

fn buffer() -> Vec<Argb> {
    vec![0; ((W + 3) * H) as usize]
}

#[test]
fn layers_refuse_buffers_too_small_or_misaligned() {
    let mut px = vec![0u32; 99];
    assert!(Layer::new(&mut px, 10, 10, 10).is_none());
    assert!(Layer::new(&mut px, 10, 10, 9).is_none());
    assert!(Layer::new(&mut px, 9, 11, 9).is_some());
    let mut words = vec![0u32; 101];
    let bytes = rp_hal_mem::words_as_bytes_mut(&mut words);
    assert!(Layer::in_bytes(&mut bytes[1..], 10, 10).is_none());
    assert!(Layer::in_bytes(&mut bytes[4..], 10, 10).is_some());
    assert!(Layer::in_bytes(&mut bytes[8..], 10, 10).is_none());
}

#[test]
fn fills_and_gradients_set_exactly_their_clipped_rectangle() {
    let (mut a, mut b) = (buffer(), buffer());
    let mut l = random_layer(&mut a, 1);
    let mut want = random_layer(&mut b, 1);
    let cols: Vec<Argb> = (0..30).map(|k| k * 0x0801_0203).collect();
    for (r, c) in [
        (Rect::new(3, 2, 10, 5), 0x80FF_FFFF),
        (Rect::new(40, 15, 100, 100), CLEAR), // clipped right and bottom
        (Rect::new(u32::MAX, 0, 9, 9), 0xFFFF_FFFF), // hostile: nothing
    ] {
        l.fill(r, c);
        let r = r.intersect(&want.bounds());
        for y in r.y..r.bottom() {
            want.span(r.x, y, r.w).iter_mut().for_each(|p| *p = c);
        }
    }
    l.fill_columns(Rect::new(30, 4, 40, 3), &cols);
    for y in 4..7 {
        // 18 columns fit before the edge; column k is cols[k].
        want.span(30, y, 18).copy_from_slice(&cols[..18]);
    }
    l.fill_columns(Rect::new(0, 10, 99, 2), &cols[..5]); // only 5 columns
    for y in 10..12 {
        want.span(0, y, 5).copy_from_slice(&cols[..5]);
    }
    assert!(l.px == want.px, "pixels outside the rectangles changed");
}

#[test]
fn icons_and_glyphs_go_over_pixel_by_pixel_like_the_reference() {
    let (mut a, mut b) = (buffer(), buffer());
    let mut l = random_layer(&mut a, 2);
    let before = random_layer(&mut b, 2);
    let mut r = rng(3);
    let icon: Vec<Argb> = (0..13 * 7).map(|_| r()).collect();
    let cover: Vec<u8> = (0..13 * 7).map(|_| r() as u8).collect();
    let img = ArgbImage {
        px: &icon,
        w: 13,
        h: 7,
    };
    let mask = Mask {
        cover: &cover,
        w: 13,
        h: 7,
    };
    let color = 0xC0E0_D0B0;
    l.draw_over(&img, 40, 2); // clipped at the right
    l.draw_mask(&mask, color, 5, 15); // clipped at the bottom
    for y in 0..H {
        for x in 0..W {
            let old = before.get(x, y).unwrap();
            let want = if (40..W).contains(&x) && (2..9).contains(&y) {
                over(icon[((y - 2) * 13 + x - 40) as usize], old)
            } else if (5..18).contains(&x) && (15..H).contains(&y) {
                let c = u32::from(cover[((y - 15) * 13 + x - 5) as usize]);
                if c == 0 {
                    old
                } else {
                    over_reference(((0xC0 * c + 127) / 255) << 24 | 0xE0_D0B0, old)
                }
            } else {
                old
            };
            assert_eq!(l.get(x, y), Some(want), "{x},{y}");
        }
    }
    // Short buffers draw nothing rather than panicking.
    let short = ArgbImage {
        px: &icon[..5],
        w: 13,
        h: 7,
    };
    l.draw_over(&short, 0, 0);
    l.draw_mask(&Mask { cover: &[], ..mask }, color, 0, 0);
}

#[test]
fn a_flush_converts_with_the_dither_fixed_to_the_screen() {
    let mut a = buffer();
    let l = random_layer(&mut a, 4);
    let v = l.view();
    let stride = W as usize * 2 + 6;
    let mut full = vec![0xAAu8; stride * H as usize];
    let n = flush(
        &v,
        PlaneFormat::Argb4444Dithered,
        &mut full,
        stride,
        &[v.bounds()],
    );
    assert_eq!(n, u64::from(W * H));
    for y in 0..H as usize {
        for x in 0..W as usize {
            let got = u16::from_le_bytes([full[y * stride + x * 2], full[y * stride + x * 2 + 1]]);
            let want = to_4444_dithered(v.get(x as u32, y as u32).unwrap(), x, y);
            assert_eq!(got, want, "{x},{y}");
        }
        assert!(
            full[y * stride + W as usize * 2..][..6]
                .iter()
                .all(|&b| b == 0xAA)
        );
    }
    // Small dirty flushes in odd places write the same bytes as the full
    // one there, and nothing else.
    let mut part = vec![0xAAu8; full.len()];
    let mut d = Dirty::<4>::new(v.bounds());
    d.add(Rect::new(7, 3, 5, 4));
    d.add(Rect::new(29, 11, 13, 9));
    flush(
        &v,
        PlaneFormat::Argb4444Dithered,
        &mut part,
        stride,
        d.rects(),
    );
    for y in 0..H {
        for x in 0..W {
            let at = y as usize * stride + x as usize * 2;
            let inside = d.rects().iter().any(|r| r.contains(&Rect::new(x, y, 1, 1)));
            let want = if inside {
                &full[at..at + 2]
            } else {
                &[0xAA, 0xAA][..]
            };
            assert_eq!(&part[at..at + 2], want, "{x},{y}");
        }
    }
}

#[test]
fn an_8888_flush_is_a_plain_copy_and_a_short_plane_stops_it() {
    let mut a = buffer();
    let l = random_layer(&mut a, 6);
    let v = l.view();
    let stride = W as usize * 4;
    let mut fb = vec![0u8; stride * H as usize];
    let r = Rect::new(2, 1, 10, 3);
    assert_eq!(flush(&v, PlaneFormat::Argb8888, &mut fb, stride, &[r]), 30);
    for y in 0..H {
        for x in 0..W {
            let at = (y * W + x) as usize * 4;
            let got = u32::from_le_bytes(fb[at..at + 4].try_into().unwrap());
            let inside = r.contains(&Rect::new(x, y, 1, 1));
            assert_eq!(got, if inside { v.get(x, y).unwrap() } else { 0 });
        }
    }
    // A plane too short for the rectangle: stops at the end, no panic.
    let mut short = vec![0u8; stride * 2];
    let n = flush(
        &v,
        PlaneFormat::Argb4444Dithered,
        &mut short,
        stride,
        &[v.bounds()],
    );
    assert!(n < u64::from(W * H));
}
