use super::dirty::{Dirty, flush, widen};
use super::*;

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
fn the_one_multiply_blend_equals_blending_each_channel() {
    // Every alpha, and colours covering every channel extreme.
    let mut r = rng(7);
    for a in 0..=32 {
        for _ in 0..4000 {
            let (s, d) = (r() as Px, r() as Px);
            assert_eq!(
                blend(s, d, a),
                blend_reference(s, d, a),
                "{s:#x} {d:#x} {a}"
            );
        }
        for s in [0u16, 0xFFFF, 0xF800, 0x07E0, 0x001F] {
            for d in [0u16, 0xFFFF, 0xF800, 0x07E0, 0x001F] {
                assert_eq!(blend(s, d, a), blend_reference(s, d, a));
            }
        }
    }
    assert_eq!(
        blend(0x1234, 0xABCD, 0),
        0xABCD,
        "clear keeps the destination"
    );
    assert_eq!(blend(0x1234, 0xABCD, 32), 0x1234, "solid replaces it");
    assert_eq!((alpha5(0), alpha5(255), alpha5(128)), (0, 32, 16));
}

#[test]
fn rectangles_clip_and_never_overflow() {
    let screen = Rect::new(0, 0, 480, 640);
    assert_eq!(
        Rect::new(470, 630, 20, 20).intersect(&screen),
        Rect::new(470, 630, 10, 10)
    );
    assert!(Rect::new(480, 0, 5, 5).intersect(&screen).is_empty());
    // Hostile values: saturate, clip to nothing, no panic.
    let huge = Rect::new(u32::MAX - 1, u32::MAX, u32::MAX, u32::MAX);
    assert!(huge.intersect(&screen).is_empty());
    assert_eq!(huge.right(), u32::MAX);
    let mut r = rng(3);
    for _ in 0..20_000 {
        let a = Rect::new(r() % 700, r() % 700, r() % 700, r() % 700);
        let b = Rect::new(r(), r(), r(), r());
        let i = a.intersect(&b);
        assert!(i.is_empty() || (a.contains(&i) && b.contains(&i)));
        let u = a.union(&screen);
        assert!(u.contains(&a) && u.contains(&screen));
    }
}

/// A surface of `w` x `h` over a fresh buffer with a stride of `w + pad`.
fn buffer(w: u32, h: u32, pad: u32) -> Vec<Px> {
    vec![0xAAAA; ((w + pad) * h) as usize]
}

#[test]
fn surfaces_refuse_buffers_too_small() {
    let mut px = buffer(10, 10, 0);
    assert!(Surface::new(&mut px, 10, 10, 10).is_some());
    assert!(Surface::new(&mut px, 10, 11, 10).is_none());
    assert!(Surface::new(&mut px, 10, 10, 9).is_none(), "stride < width");
    assert!(
        Surface::new(&mut px, 10, 10, usize::MAX).is_none(),
        "overflow"
    );
    assert!(Surface::new(&mut [], 0, 0, 0).is_some());
}

#[test]
fn fills_stay_inside_their_clipped_rectangle() {
    let mut px = buffer(32, 20, 4);
    let mut s = Surface::new(&mut px, 32, 20, 36).unwrap();
    s.fill(Rect::new(30, 18, 10, 10), 0x1234);
    for y in 0..20 {
        for x in 0..32 {
            let want = if x >= 30 && y >= 18 { 0x1234 } else { 0xAAAA };
            assert_eq!(s.get(x, y), Some(want), "{x},{y}");
        }
    }
    // The padding past each row is never touched.
    assert!(
        px.chunks(36)
            .all(|row| row[32..].iter().all(|&p| p == 0xAAAA))
    );
}

#[test]
fn glass_tint_matches_a_per_pixel_reference() {
    let mut g = Gradient::new();
    let from = Tint {
        color: rgb(6, 8, 11),
        alpha: 190,
    };
    let to = Tint {
        color: rgb(78, 86, 92),
        alpha: 107,
    };
    g.set(200, from, to);
    let mut r = rng(11);
    let mut px: Vec<Px> = (0..480 * 64).map(|_| r() as Px).collect();
    let before = px.clone();
    let area = Rect::new(37, 5, 260, 40); // wider than the gradient: clipped to 200
    Surface::new(&mut px, 480, 64, 480).unwrap().tint(area, &g);
    for y in 0..64u32 {
        for x in 0..480u32 {
            let i = (y * 480 + x) as usize;
            let inside = (37..237).contains(&x) && (5..45).contains(&y);
            if !inside {
                assert_eq!(px[i], before[i], "outside changed at {x},{y}");
                continue;
            }
            // The column's colour and alpha, as `set` defines them.
            let t = x - 37;
            let lerp = |p: u32, q: u32| (p * (199 - t) + q * t + 99) / 199;
            let c = |p: Px, sh: u32, m: u32| (u32::from(p) >> sh) & m;
            let col = (lerp(c(from.color, 11, 31), c(to.color, 11, 31)) << 11
                | lerp(c(from.color, 5, 63), c(to.color, 5, 63)) << 5
                | lerp(c(from.color, 0, 31), c(to.color, 0, 31))) as Px;
            let a = alpha5(lerp(u32::from(from.alpha), u32::from(to.alpha)) as u8);
            assert_eq!(px[i], blend_reference(col, before[i], a), "{x},{y}");
        }
    }
}

#[test]
fn blend_fill_and_alpha_images_match_the_reference() {
    let mut r = rng(5);
    let (w, h) = (40u32, 30u32);
    let mut px: Vec<Px> = (0..w * h).map(|_| r() as Px).collect();
    let before = px.clone();
    let icon: Vec<Px> = (0..12 * 12).map(|_| r() as Px).collect();
    let alpha: Vec<u8> = (0..12 * 12)
        .map(|k| [0, 255, (r() % 256) as u8][k % 3])
        .collect();
    let img = AlphaImage {
        px: &icon,
        alpha: &alpha,
        w: 12,
        h: 12,
    };
    let mut s = Surface::new(&mut px, w, h, w as usize).unwrap();
    s.draw_alpha(&img, 34, 24); // hangs off the right and bottom: clipped
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let want = if x >= 34 && y >= 24 {
                let k = ((y - 24) * 12 + (x - 34)) as usize;
                blend_reference(icon[k], before[i], alpha5(alpha[k]))
            } else {
                before[i]
            };
            assert_eq!(s.get(x, y), Some(want), "{x},{y}");
        }
    }
    // An image whose buffers are too short draws nothing.
    let bad = AlphaImage { w: 13, ..img };
    let snapshot: Vec<Px> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| s.get(x, y).unwrap())
        .collect();
    s.draw_alpha(&bad, 0, 0);
    s.blend_fill(Rect::new(0, 0, 2, 1), 0xFFFF, 128);
    assert_eq!(s.get(0, 0), Some(blend_reference(0xFFFF, snapshot[0], 16)));
    assert_eq!(s.get(2, 0), Some(snapshot[2]));
}

#[test]
fn copies_and_scrolls_move_exactly_the_pixels_asked() {
    let (w, h) = (24u32, 16u32);
    let src_px: Vec<Px> = (0..w * h).map(|k| k as Px).collect();
    let src = View::new(&src_px, w, h, w as usize).unwrap();
    let mut px = buffer(w, h, 0);
    let mut s = Surface::new(&mut px, w, h, w as usize).unwrap();
    s.copy_from(&src, Rect::new(2, 3, 30, 5), 20, 14); // clipped twice
    for y in 0..h {
        for x in 0..w {
            let want = if x >= 20 && y >= 14 {
                ((3 + y - 14) * w + 2 + x - 20) as Px
            } else {
                0xAAAA
            };
            assert_eq!(s.get(x, y), Some(want), "{x},{y}");
        }
    }
    // Scrolling against a model, every direction, overlapping.
    let mut r = rng(9);
    for _ in 0..2000 {
        let mut px: Vec<Px> = (0..w * h).map(|k| k as Px).collect();
        let model = px.clone();
        let from = Rect::new(r() % w, r() % h, r() % w, r() % h);
        let (dx, dy) = (r() % w, r() % h);
        Surface::new(&mut px, w, h, w as usize)
            .unwrap()
            .copy_within(from, dx, dy);
        let f = from.intersect(&Rect::new(0, 0, w, h));
        let to = Rect::new(dx, dy, f.w, f.h).intersect(&Rect::new(0, 0, w, h));
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                let inside = !to.is_empty() && to.contains(&Rect::new(x, y, 1, 1));
                let want = if inside {
                    model[((f.y + y - dy) * w + f.x + x - dx) as usize]
                } else {
                    model[i]
                };
                assert_eq!(px[i], want, "{from:?} to {dx},{dy} at {x},{y}");
            }
        }
    }
}

#[test]
fn photo_rows_convert_with_a_dither_that_averages_out() {
    // A smooth grey ramp: each 4x4 block averages within a fraction of a
    // 16-bit step of the true value (so gradients don't band), and white
    // and black stay exact.
    let mut px = buffer(256, 4, 0);
    let mut s = Surface::new(&mut px, 256, 4, 256).unwrap();
    let ramp: Vec<u8> = (0..=255u8).flat_map(|v| [v, v, v]).collect();
    for y in 0..4 {
        s.put_rgb888_row(0, y, &ramp);
    }
    assert_eq!(s.get(255, 0), Some(0xFFFF));
    assert_eq!(s.get(0, 3), Some(0));
    for bx in 0..64u32 {
        let mut sum = 0.0;
        for y in 0..4 {
            for x in bx * 4..bx * 4 + 4 {
                sum += f64::from(s.get(x, y).unwrap() >> 11);
            }
        }
        let mean = sum / 16.0;
        let truth = f64::from(bx * 4 + 1) + 0.5; // the block's mean grey
        assert!(
            (mean - truth * 31.0 / 255.0).abs() < 0.6,
            "block {bx}: {mean}"
        );
    }
    // Clipped: a row past the right edge, and one below the bottom.
    s.put_rgb888_row(250, 0, &ramp);
    s.put_rgb888_row(0, 9, &ramp);
}

#[test]
fn dirty_rectangles_cover_every_change_and_stay_few() {
    let screen = Rect::new(0, 0, 480, 640);
    let mut r = rng(21);
    for round in 0..300 {
        let mut d = Dirty::<8>::new(screen);
        let mut model = vec![false; 480 * 640];
        let n = 1 + r() % 30;
        let mut added = 0u64;
        for _ in 0..n {
            let a = Rect::new(r() % 500, r() % 660, 1 + r() % 90, 1 + r() % 60);
            let c = a.intersect(&screen);
            added += widen(c).intersect(&screen).area();
            for y in c.y..c.bottom() {
                for x in c.x..c.right() {
                    model[(y * 480 + x) as usize] = true;
                }
            }
            d.add(a);
        }
        assert!(d.rects().len() <= 8);
        for (i, &changed) in model.iter().enumerate() {
            if changed {
                let p = Rect::new(i as u32 % 480, i as u32 / 480, 1, 1);
                assert!(
                    d.rects().iter().any(|q| q.contains(&p)),
                    "round {round}: missed {p:?}"
                );
            }
        }
        for q in d.rects() {
            assert!(screen.contains(q) && q.x % 2 == 0 && q.w % 2 == 0, "{q:?}");
        }
        // Merging never covers a silly amount more than what changed, unless
        // the list overflowed and had to merge far-apart areas.
        if n <= 8 {
            assert!(
                d.area() <= added * 2 + 8 * dirty::MERGE_SLACK,
                "round {round}"
            );
        }
    }
    // A clock tick: one small rectangle, merged with its neighbour digit.
    let mut d = Dirty::<8>::new(screen);
    d.add(Rect::new(400, 10, 9, 14));
    d.add(Rect::new(410, 10, 9, 14));
    assert_eq!(d.rects(), &[Rect::new(400, 10, 20, 14)]);
}

#[test]
fn flushes_write_only_the_dirty_words() {
    let (w, h) = (48u32, 20u32);
    let px: Vec<Px> = (0..w * h).map(|k| 0x8000 | k as Px).collect();
    let view = View::new(&px, w, h, w as usize).unwrap();
    let stride = w as usize * 2 + 8; // the framebuffer's own stride
    let mut fb = vec![0x5Au8; stride * h as usize];
    let mut d = Dirty::<4>::new(view.bounds());
    d.add(Rect::new(3, 2, 5, 3)); // odd x and width: widened to 2..8
    d.add(Rect::new(40, 15, 20, 20)); // clipped
    let written = flush(&view, &mut fb, stride, d.rects());
    assert_eq!(written, 6 * 3 + 8 * 5);
    for y in 0..h {
        for x in 0..w {
            let at = y as usize * stride + x as usize * 2;
            let got = u16::from_le_bytes([fb[at], fb[at + 1]]);
            let dirty = d.rects().iter().any(|q| q.contains(&Rect::new(x, y, 1, 1)));
            let want = if dirty {
                px[(y * w + x) as usize]
            } else {
                0x5A5A
            };
            assert_eq!(got, want, "{x},{y}");
        }
        assert!(
            fb[y as usize * stride + 96..(y as usize + 1) * stride]
                .iter()
                .all(|&b| b == 0x5A)
        );
    }
    // A framebuffer too small for a rectangle: stops, doesn't panic.
    let mut tiny = vec![0u8; 10];
    assert_eq!(flush(&view, &mut tiny, stride, d.rects()), 0);
}
