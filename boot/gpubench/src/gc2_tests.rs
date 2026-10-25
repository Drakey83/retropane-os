//! Run GC2's shared part on the host: the model table held to the model,
//! the error descriptions at their edges.

extern crate std;

use std::string::String;

use rp_mbx3d_render::model::{Gpu, Mem, SimClock, memory};
use rp_mbx3d_render::{LocalMem as _, Picture, Poll, Renderer, Target};
use rp_vram_map::{POOL, Pool};

use super::*;

/// Every kind rendered through the model of the 3D core: its band
/// hashes, as the image takes them from its references.
fn model_hashes() -> [[u32; BANDS]; 13] {
    let m = memory();
    let mut clock = SimClock::default();
    let mut pool = Pool::new(POOL);
    let mut mem = Mem::new(m.clone());
    let mut gpu = Gpu::new(m, &clock);
    let mut r = Renderer::new(&mut pool, W as u16, H as u16).expect("renderer");
    let mut place = |n: usize, mem: &mut Mem| {
        let t = TEXTURES[n];
        let at = pool.alloc(t.bytes(), 128).expect("texture memory");
        texture_words(n, |i, w| mem.store((at + 4 * i) as usize, &[w]));
        Texture { at, ..t }
    };
    let tex = Tex {
        t565: place(0, &mut mem),
        t4444: place(1, &mut mem),
        t8888: place(2, &mut mem),
        plane: place(3, &mut mem),
    };
    let at = pool.alloc(W * H * 4, 128).expect("target memory");
    let mut out = [[0; BANDS]; 13];
    for (k, bands) in out.iter_mut().enumerate() {
        let (bg, slots) = scene(k, tex);
        r.set_background(Some(bg)).expect("background");
        for (s, d) in slots.iter().enumerate() {
            r.set(&mut mem, s, d.as_ref()).expect("slot");
        }
        r.invalidate_all();
        let t = Target {
            picture: Picture::packed(at, W as u16, H as u16, KINDS[k].1),
            x: 0,
            y: 0,
        };
        let due = r
            .render(&mut gpu, &mut mem, &mut clock, t)
            .expect("render")
            .expect("something to render");
        clock.set(due);
        loop {
            match r.poll(&mut gpu, &mut mem, &mut clock) {
                Poll::Done(_) => break,
                Poll::Wait(t) => clock.set(t),
                p => panic!("{p:?}"),
            }
        }
        for (b, h) in bands.iter_mut().enumerate() {
            *h = (0..band_words(k)).fold(HASH_START, |h, i| {
                let a = at + 4 * (b as u32 * band_words(k) + i);
                fold(h, mem.load(a as usize).expect("in memory"))
            });
        }
    }
    out
}

#[test]
fn gc2i_puts_every_16_bit_kind_inside_and_keeps_the_32_bit_ones_out() {
    let outside = 0x0045_8000;
    let mut inside = 0;
    for k in RUN {
        let at = target_at(k, outside);
        let end = at + 4 * words(k);
        if KINDS[k].1.bytes() == 2 {
            // Below the label band, wholly inside G19's last right end.
            assert_eq!(at, INSIDE_AT, "{}", KINDS[k].0);
            assert!(at >= LABEL_BAND && end <= SAFE_END, "{}", KINDS[k].0);
            inside += 1;
        } else {
            // 1.2 MiB can't fit in 696 KiB: where runs GC2 and GC2S drew.
            assert!(!fits_inside(k), "{}", KINDS[k].0);
            assert_eq!(at, outside, "{}", KINDS[k].0);
        }
    }
    assert_eq!(inside, 6);
    // The band holds the label, the safe region is G19's, the target starts
    // on a row of the 16-bit screen (so it shows row for row).
    assert_eq!((LABEL_BAND, SAFE_END), (64 * 960, 0xA_E000));
    assert!(INSIDE_AT.is_multiple_of(960));
}

#[test]
fn gc2j_puts_ft2s_texture_inside_after_the_target_and_soaks_every_inside_kind() {
    let t = TEXTURES[2];
    // FT2 reads the 8888 texture; a 32 x 32 one, 4 KiB.
    assert!(KINDS[FT2].0.contains("8888 texture"));
    assert_eq!(t.bytes(), 4 * 32 * 32);
    // A page after the 16-bit target's end (a row written past it, as the
    // emulator writes the dithered target, can't reach it), inside the
    // safe region, aligned for a texture and on a page.
    assert!(TEX_INSIDE >= target_at(0, 0) + 4 * words(0) + 960);
    assert!(TEX_INSIDE + t.bytes() <= SAFE_END);
    assert!(TEX_INSIDE.is_multiple_of(0x1000));
    // Every kind inside once with its textures where they are, FT2 once
    // more with its texture inside; nothing else.
    for k in RUN.filter(|&k| fits_inside(k)) {
        assert!(SOAK2.contains(&(k, false)), "{}", KINDS[k].0);
    }
    assert!(SOAK2.contains(&(FT2, true)));
    assert_eq!(SOAK2.len(), RUN.filter(|&k| fits_inside(k)).count() + 1);
    assert!(
        SOAK2
            .iter()
            .all(|&(k, inside)| fits_inside(k) && (!inside || k == FT2))
    );
}

#[test]
fn gc2k_ways_move_the_quad_off_the_seams_and_tell_the_two_places_apart() {
    // Way 0 is FT2's own scene.
    let t = Tex {
        t565: TEXTURES[0],
        t4444: TEXTURES[1],
        t8888: TEXTURES[2],
        plane: TEXTURES[3],
    };
    let t565 = Texture {
        format: rp_mbx3d_render::format::RGB565 as u8,
        ..TEXTURES[2]
    };
    assert_eq!(patch_scene(0, t, t565), scene(FT2, t));
    // Every way a different scene.
    for a in 0..PATCH_WAYS.len() {
        for b in a + 1..PATCH_WAYS.len() {
            assert_ne!(patch_scene(a, t, t565), patch_scene(b, t, t565), "{a} {b}");
        }
    }
    // FT2's corner (40, 600): x on a seam (8 wide), y mid tile row (16
    // tall). The shift and the move take it off every seam.
    assert_eq!(patch_corner(0), (40, 600));
    for way in [1, 2] {
        let (x, y) = patch_corner(way);
        assert!(x % 8 != 0 && y % 16 != 0, "way {way} at ({x}, {y})");
    }
    assert!(PATCH_SHIFT.0 % 8 != 0 && PATCH_SHIFT.1 % 16 != 0);
    // GC2J's patch: at GC2J's place and at the corner's place in way 0
    // (both), and in the ways that move the corner, never both at once.
    assert_eq!(patch_spot(0, 30, 605), (true, true));
    for way in [1, 2] {
        let (cx, cy) = patch_corner(way);
        for y in 590..620 {
            for x in 0..80 {
                assert!(
                    patch_spot(way, x, y) != (true, true),
                    "way {way} ({x}, {y})"
                );
            }
        }
        // A pixel where FT2's patch was relative to the corner is found.
        assert_eq!(patch_spot(way, cx - 10, cy + 5), (false, true), "way {way}");
    }
    // The ways that keep the corner: both or neither.
    for way in [3, 4] {
        assert_eq!(patch_spot(way, 30, 605), (true, true));
        assert_eq!(patch_spot(way, 200, 300), (false, false));
    }
}

#[test]
fn gc2l_sweeps_the_corner_over_rows_and_tiles_and_moves_the_target() {
    let t = Tex {
        t565: TEXTURES[0],
        t4444: TEXTURES[1],
        t8888: TEXTURES[2],
        plane: TEXTURES[3],
    };
    // Ways 0-3 are GC2K's worst way: at INSIDE_AT, 32 KiB on, at the
    // screen's 0x0 (option C), and with the tiles reversed; every target
    // inside the safe region; only way 3 reversed.
    let unused = TEXTURES[0];
    assert_eq!(sweep_scene(0, t), patch_scene(1, t, unused));
    for w in 1..4 {
        assert_eq!(sweep_scene(w, t), sweep_scene(0, t), "way {w}");
    }
    assert_eq!(SWEEP[0].corner(), (43, 605));
    assert_eq!(
        [SWEEP[0].at, SWEEP[1].at, SWEEP[2].at, SWEEP[3].at],
        [INSIDE_AT, INSIDE_AT + SWEEP_AT_OFFSET, 0, INSIDE_AT]
    );
    assert!(SWEEP.iter().all(|w| w.at + 2 * W * H <= SAFE_END));
    assert!(SWEEP.iter().enumerate().all(|(i, w)| w.reverse == (i == 3)));
    // The moved target shifts the picture in memory by whole words, and
    // not by whole rows: an address-tied fault moves on the screen.
    assert!(SWEEP_AT_OFFSET.is_multiple_of(4) && !SWEEP_AT_OFFSET.is_multiple_of(2 * W));
    // The row sweep: x 43, the bottom edge on every row of 592-607 once.
    let rows_at = SWEEP_ROWS_AT..SWEEP_ROWS_AT + 16;
    let rows: std::vec::Vec<i32> = rows_at.clone().map(|w| SWEEP[w].corner().1).collect();
    assert_eq!(rows, (592..608).collect::<std::vec::Vec<_>>());
    assert!(
        rows_at
            .clone()
            .all(|w| SWEEP[w].corner().0 == 43 && SWEEP[w].shift == (0, 0))
    );
    // The tile sweep: 16 different tiles, the corner where GC2K's worst
    // way had it in its tile (3 across, 13 down) in each, every quad on the
    // screen; GC2K's tile among them.
    assert_eq!(
        SWEEP_IN_TILE,
        (SWEEP[0].corner().0 % 8, SWEEP[0].corner().1 % 16)
    );
    let mut tiles = std::vec::Vec::new();
    assert_eq!(SWEEP_TILES_AT + 16, SWEEP.len());
    for (w, way) in SWEEP.iter().enumerate().skip(SWEEP_TILES_AT) {
        let (cx, cy) = way.corner();
        assert_eq!((cx % 8, cy % 16), SWEEP_IN_TILE, "way {w}");
        let (x, y, qw, qh) = way.quad;
        assert!(
            x >= 0 && y >= 0 && x + qw <= W as i32 && y + qh <= H as i32,
            "way {w}"
        );
        tiles.push((cx / 8, cy / 16));
    }
    tiles.sort_unstable();
    tiles.dedup();
    assert_eq!(tiles.len(), 16);
    assert!(tiles.contains(&(5, 37)));
    // Every way a different scene, place or order.
    for (a, wa) in SWEEP.iter().enumerate() {
        for (b, wb) in SWEEP.iter().enumerate().skip(a + 1) {
            assert!(wa != wb, "{a} {b}");
        }
    }
}

#[test]
fn gc2o_runs_every_scene_in_the_new_orders_and_the_first_ten_in_all_three() {
    let t = Tex {
        t565: TEXTURES[0],
        t4444: TEXTURES[1],
        t8888: TEXTURES[2],
        plane: TEXTURES[3],
    };
    // The scenes: GC2K's worst, rows 597 and 598, the mirror, six kinds,
    // the 32 sweep ways.
    assert_eq!(ORDER_SCENES[0], Scene::Sweep(0));
    assert_eq!(order_corner(ORDER_SCENES[1]).1, 597);
    assert_eq!(order_corner(ORDER_SCENES[2]).1, 598);
    assert!(
        ORDER_SCENES[4..10]
            .iter()
            .all(|s| matches!(*s, Scene::Kind(k) if fits_inside(k)))
    );
    assert!((0..32).all(|j| ORDER_SCENES[10 + j] == Scene::Sweep(SWEEP_ROWS_AT + j)));
    // The mirror: GC2K's worst left to right, its patch's corner on the
    // right, off every seam.
    assert_eq!(MIRRORED_QUAD.0 + MIRRORED_QUAD.2, 480 - SWEEP[0].quad.0);
    assert_eq!(order_corner(Scene::Mirrored), (437, 605));
    assert!(order_corner(Scene::Mirrored).0 % 8 != 0);
    assert_ne!(
        order_scene(Scene::Mirrored, t),
        order_scene(Scene::Sweep(0), t)
    );
    // Every scene once in each of its orders, never twice.
    for s in 0..42 {
        let orders: std::vec::Vec<u8> = ORDER_WAYS
            .iter()
            .filter(|w| w.0 == s)
            .map(|w| w.1)
            .collect();
        let want: &[u8] = if s < 10 {
            &[ROWS, REVERSED, COLUMNS_REVERSED]
        } else {
            &[REVERSED, COLUMNS_REVERSED]
        };
        assert_eq!(orders, want, "scene {s}");
    }
}

#[test]
fn gc2p_keeps_the_scenes_that_went_wrong_and_every_order_where_it_matters() {
    // The rows that went wrong in GC2L (595-603), each once.
    let rows: std::vec::Vec<i32> = FAST_SCENES[1..10]
        .iter()
        .map(|&s| order_corner(s).1)
        .collect();
    assert_eq!(rows, (595..604).collect::<std::vec::Vec<_>>());
    assert_eq!(FAST_SCENES[0], Scene::Sweep(0));
    assert_eq!(FAST_SCENES[10], Scene::Mirrored);
    assert_eq!(FAST_SCENES[11], Scene::Kind(FT2));
    // The all-orders scenes: GC2K's worst, rows 597 and 598, the mirror, FT2.
    let picked: std::vec::Vec<Scene> = FAST_ALL_ORDERS.iter().map(|&i| FAST_SCENES[i]).collect();
    assert_eq!(order_corner(picked[1]).1, 597);
    assert_eq!(order_corner(picked[2]).1, 598);
    assert_eq!(picked[3], Scene::Mirrored);
    assert_eq!(picked[4], Scene::Kind(FT2));
    for s in 0..FAST_SCENES.len() {
        let orders: std::vec::Vec<u8> =
            FAST_WAYS.iter().filter(|w| w.0 == s).map(|w| w.1).collect();
        let want: &[u8] = if FAST_ALL_ORDERS.contains(&s) {
            &[ROWS, REVERSED, COLUMNS_REVERSED]
        } else {
            &[REVERSED, COLUMNS_REVERSED]
        };
        assert_eq!(orders, want, "scene {s}");
    }
    // Every 16-bit kind there.
    for k in RUN.filter(|&k| fits_inside(k)) {
        assert!(FAST_SCENES.contains(&Scene::Kind(k)), "{}", KINDS[k].0);
    }
    // Enough rounds to see GC2L's normal-order rate many times: at least 10
    // wrong renders expected at row 597's 7%, so a new order's 0 means
    // something.
    assert!(f64::from(FAST_ROUNDS) * 29.0 / 397.0 >= 10.0);
}

#[test]
fn gc2q_climbs_from_133_25_to_149_50_below_the_memorys_failure() {
    let mhz: std::vec::Vec<u32> = ABOVE_PLL.iter().map(|&(m, n)| 13_000 * m / n).collect();
    assert_eq!(mhz, [133_250, 139_750, 146_250, 149_500]);
    // Rising, below GC2H2's 156 MHz, the highest GC2H's 149.50.
    assert!(mhz.windows(2).all(|w| w[0] < w[1]));
    assert!(mhz.iter().all(|&k| k < 156_000));
    // Every 16-bit kind, once, in row order.
    for k in RUN.filter(|&k| fits_inside(k)) {
        assert!(ABOVE_SCENES.contains(&Scene::Kind(k)), "{}", KINDS[k].0);
    }
    assert_eq!(ABOVE_WAYS.len(), ABOVE_SCENES.len());
    assert!(ABOVE_WAYS.iter().enumerate().all(|(i, w)| *w == (i, ROWS)));
    // The confirmation: GC2K's worst, row order.
    assert_eq!(order_corner(CONFIRM_SCENES[0]), (43, 605));
    assert_eq!(CONFIRM_WAYS, [(0, ROWS)]);
}

#[test]
fn the_model_table_is_what_the_model_renders() {
    let got = model_hashes();
    if got != MODEL {
        let mut s = String::from("[\n");
        for bands in got {
            s.push_str("    [");
            for (b, h) in bands.iter().enumerate() {
                let sep = if b == 0 { "" } else { ", " };
                s.push_str(&std::format!("{sep}0x{h:08x}"));
            }
            s.push_str("],\n");
        }
        s.push_str("]\n");
        panic!("boot/gpubench/src/gc2_model.in is out of date; the model gives:\n{s}");
    }
}

#[test]
fn every_kind_differs_from_every_other() {
    // A scene that drew nothing, or the same as another, would make the
    // references prove less than they say.
    for (a, x) in MODEL.iter().enumerate() {
        for (b, y) in MODEL.iter().enumerate().skip(a + 1) {
            assert_ne!(x, y, "kinds {a} and {b}");
        }
    }
}

#[test]
fn the_capped_scenes_keep_every_channel_below_its_top() {
    let tex = Tex {
        t565: TEXTURES[0],
        t4444: TEXTURES[1],
        t8888: TEXTURES[2],
        plane: TEXTURES[3],
    };
    for k in [7, 8] {
        let (_, slots) = scene(k, tex);
        let Some(Draw {
            paint: Paint::Gouraud(c),
            ..
        }) = slots[0]
        else {
            panic!("kind {k}: a Gouraud quad");
        };
        for argb in c {
            let ch = [argb >> 24, argb >> 16 & 0xFF, argb >> 8 & 0xFF, argb & 0xFF];
            if k == 7 {
                // RGB565: red and blue at most 30 of 31, green 62 of 63.
                assert!(
                    ch[1] >> 3 < 31 && ch[2] >> 2 < 63 && ch[3] >> 3 < 31,
                    "{argb:#010x}"
                );
            } else {
                assert!(ch.iter().all(|&v| v < 0xFF), "{argb:#010x}");
            }
        }
    }
}

#[test]
fn a_wrong_pixel_is_described_channel_by_channel() {
    let p = PixelFormat::Rgb565;
    // The device's own (FT2 fast, read 2026-10-23).
    assert_eq!(miss(p, 0xf79e, 0xf7be), Miss::Plus1, "green +1");
    assert_eq!(miss(p, 0x7bef, 0x8410), Miss::Plus1, "+1 in each, carried");
    assert_eq!(miss(p, 0x9491, 0x9492), Miss::Plus1, "blue +1");
    assert_eq!(miss(p, 0xffff, 0x0840), Miss::Wrapped, "white plus one");
    // Both directions, and each class's edges.
    assert_eq!(miss(p, 0x8410, 0x7bef), Miss::Minus1);
    assert_eq!(miss(p, 0x0000, 0x0821), Miss::Plus1);
    assert_eq!(miss(p, 0x0821, 0x0000), Miss::Minus1);
    assert_eq!(miss(p, 0x0000, 0x0002), Miss::Other, "+2");
    assert_eq!(miss(p, 0x0821, 0x1000), Miss::Other, "one up, one down");
    // Wrapped needs the top or one below it, and a drop past half.
    assert_eq!(miss(p, 0xf000, 0x0000), Miss::Wrapped, "red 30 to 0");
    assert_eq!(miss(p, 0xe800, 0x0000), Miss::Other, "red 29 to 0");
    assert_eq!(miss(p, 0xf800, 0x8000), Miss::Other, "red 31 to 16: half");
    assert_eq!(miss(p, 0xf800, 0x7800), Miss::Wrapped, "red 31 to 15");
    let q = PixelFormat::Argb8888;
    assert_eq!(miss(q, 0xFF00_0000, 0x0000_0000), Miss::Wrapped, "alpha");
    assert_eq!(miss(q, 0x1234_5678, 0x1234_5679), Miss::Plus1);
    assert_eq!(miss(q, 0x1234_5678, 0x1233_5678), Miss::Minus1);
    assert_eq!(miss(q, 0x1234_5678, 0x1234_7778), Miss::Other);
    assert_eq!(
        miss(q, 0xFFFE_FFFF, 0xFF00_0000),
        Miss::Wrapped,
        "green 254"
    );
    assert_eq!(
        miss(PixelFormat::Rgb565Dithered, 0xf79e, 0xf7be),
        Miss::Plus1
    );
}

#[test]
fn a_tally_places_and_counts_every_wrong_pixel() {
    let p = PixelFormat::Rgb565;
    let mut t = Tally::default();
    // Word 240: pixels 480 and 481, (0, 1) and (1, 1).
    t.add(p, 240, 0xf79e_ffff, 0xf7be_0840);
    // Word 241: only its high pixel, (3, 1).
    t.add(p, 241, 0x1111_2222, 0x1112_2222);
    assert_eq!(t.by, [2, 0, 1, 0]);
    assert_eq!(t.pixels(), 3);
    assert_eq!(t.bands, 1);
    assert_eq!(t.bbox, Some((0, 1, 3, 1)));
    assert_eq!(
        t.first[..t.n_first],
        [
            (0, 1, 0xffff, 0x0840),
            (1, 1, 0xf79e, 0xf7be),
            (3, 1, 0x1111, 0x1112)
        ]
    );
    // ARGB8888: a word a pixel; the last row is in the last band.
    let mut u = Tally::default();
    u.add(PixelFormat::Argb8888, W * H - 1, 0xFF00_0000, 0xFF00_0001);
    u.add(PixelFormat::Argb8888, 5, 7, 7);
    assert_eq!(u.by, [1, 0, 0, 0]);
    assert_eq!(u.bands, 1 << (BANDS - 1));
    assert_eq!(u.bbox, Some((479, 639, 479, 639)));
    // Row 16 starts band 1.
    let mut v = Tally::default();
    v.add(PixelFormat::Argb8888, 16 * W, 1, 2);
    v.add(PixelFormat::Argb8888, 16 * W - 1, 1, 2);
    assert_eq!(v.bands, 0b11);
    t.merge(&u);
    assert_eq!(t.by, [3, 0, 1, 0]);
    assert_eq!(t.bands, 1 | 1 << (BANDS - 1));
    assert_eq!(t.bbox, Some((0, 1, 479, 639)));
    assert_eq!(t.n_first, 3, "the first three kept");
    let mut e = Tally::default();
    e.merge(&u);
    assert_eq!(e.bbox, u.bbox);
    assert_eq!(e.first[..e.n_first], [(479, 639, 0xFF00_0000, 0xFF00_0001)]);
}

#[test]
fn the_textures_are_written_whole_in_order() {
    for (n, t) in TEXTURES.iter().enumerate() {
        let mut seen = 0;
        texture_words(n, |i, _| {
            assert_eq!(i, seen);
            seen += 1;
        });
        assert_eq!(seen * 4, t.bytes(), "texture {n}");
    }
    // Texel 1 of the twiddled order is (0, 1): the high half of word 0.
    texture_words(0, |i, w| {
        if i == 0 {
            assert_eq!(
                w,
                u32::from(texel565(0, 0)) | u32::from(texel565(0, 1)) << 16
            );
        }
    });
}

#[test]
fn the_plane_is_one_plane_and_its_texture_holds_its_corners() {
    for sh in [0, 8, 16, 24] {
        let ch = |c: u32| (c >> sh & 0xFF) as i32;
        assert_eq!(
            ch(PLANE[3]),
            ch(PLANE[1]) + ch(PLANE[2]) - ch(PLANE[0]),
            "channel at {sh}"
        );
    }
    let mut corners = [0u32; 4];
    texture_words(3, |i, w| {
        let (x, y) = untwiddle(i);
        if x < 2 && y < 2 {
            corners[(2 * y + x) as usize] = w;
        }
    });
    assert_eq!(corners, PLANE);
    assert!(TEXTURES[3].linear, "drawn bilinear");
    // The two runs' kinds: every kind in one of them, none in both.
    assert_eq!(
        (RUN.start, RUN.end, RUN_D.end),
        (0, RUN_D.start, KINDS.len())
    );
    // Both targets: the Gouraud kind and its texture twin.
    for (g, t) in PAIRS_D {
        assert_eq!(KINDS[g].1, KINDS[t].1);
        assert!(RUN_D.contains(&g) && RUN_D.contains(&t));
    }
}

#[test]
fn gc2t_loosens_one_field_at_a_time_from_the_rated_timing() {
    // 133.25 MHz's rated timing (Dell's routine): tRAS 9, tRP 4, tRCD 4,
    // tRC 12, tDPL 2.
    let rated = 0x944C2;
    let got: std::vec::Vec<u32> = TIM_WAYS
        .iter()
        .map(|&(_, add)| lmtim_with(rated, add).unwrap())
        .collect();
    assert_eq!(
        got,
        [
            0x944C2, 0x944C3, 0x944C4, 0x954C2, 0x945C2, 0xA44D2, 0xA55D3
        ]
    );
    // Every way but the rated one changes something; tRAS + tRP >= tRC + 1
    // held by every way.
    assert_eq!(TIM_WAYS[0].1, [0; 5]);
    for &(name, add) in &TIM_WAYS[1..] {
        let t = lmtim_with(rated, add).unwrap();
        assert_ne!(t, rated, "{name}");
        let f = |i: usize| t >> TIM_SHIFTS[i] & 0xF;
        assert!(f(0) + f(1) > f(3), "{name}");
    }
    // A field past 4 bits refused, not wrapped; bits above the fields kept.
    assert_eq!(lmtim_with(0xF_0000, [1, 0, 0, 0, 0]), None);
    assert_eq!(lmtim_with(0x0F_0000, [0, 0, 0, 0, 15]), Some(0x0F_000F));
    assert_eq!(lmtim_with(0x10_0000, [0, 0, 0, 0, 1]), Some(0x10_0001));
    // The confirmation and the climb: the rated timing first, write
    // recovery next, and everything looser last.
    assert_eq!(
        TIM_CONFIRM.map(|i| TIM_WAYS[i].0),
        ["rated", "tDPL +1", "tDPL +2", "all +1"]
    );
    assert_eq!(
        TIM_CLIMB.map(|i| TIM_WAYS[i].0),
        ["rated", "tDPL +1", "all +1"]
    );
    // At every clock the climb reaches, the loosest timing still fits.
    for &(m, n) in &ABOVE_PLL {
        let khz = 13_000 * m / n;
        let cyc = |ns: u32| (ns * khz).div_ceil(1_000_000);
        let (trc, trp) = (cyc(85), cyc(25));
        let base = (trc - trp + 1) << 16 | trp << 12 | trp << 8 | trc << 4 | 2;
        assert!(lmtim_with(base, TIM_WAYS[6].1).is_some(), "{khz}");
    }
}

#[test]
fn gc2u_busy_tiles_add_one_blended_layer_over_the_same_scene() {
    let t = Tex {
        t565: TEXTURES[0],
        t4444: TEXTURES[1],
        t8888: TEXTURES[2],
        plane: TEXTURES[3],
    };
    for (busy, base) in [(false, Scene::Kind(FT2)), (true, Scene::Sweep(0))] {
        let (bg, slots) = order_scene(Scene::Busy(busy), t);
        let (bg0, slots0) = order_scene(base, t);
        // The same background and the same first two slots: only the layer
        // is new, drawn last over the whole screen, blended, untextured.
        assert_eq!(bg, bg0);
        assert_eq!(slots[..2], slots0[..2]);
        assert_eq!(slots0[2], None);
        let layer = Draw::rect(0, 0, W as i32, H as i32, Paint::Flat(BUSY_LAYER)).over();
        assert_eq!(slots[2], Some(layer));
        // Its wrong pixels placed from the base scene's corner.
        assert_eq!(order_corner(Scene::Busy(busy)), order_corner(base));
    }
    assert_eq!(BUSY_LAYER >> 24, 0x20);
    // The ways: every scene once in row order, the lighter textures on the
    // first two (no busy layer), 100 rounds at 133.25 and 30 above.
    assert_eq!(TEXLOAD_WAYS.len(), TEXLOAD_SCENES.len());
    assert!(
        TEXLOAD_WAYS
            .iter()
            .enumerate()
            .all(|(i, w)| *w == (i, ROWS))
    );
    assert_eq!(TEXLOAD_LIGHT_WAYS, TEXLOAD_WAYS[..2]);
    assert_eq!(TEXLOAD_SCENES[..2], [Scene::Kind(FT2), Scene::Sweep(0)]);
    assert_eq!(TEXLOAD_ROUNDS.len(), ABOVE_PLL.len());
    assert_eq!(TEXLOAD_ROUNDS[0], 100);
    assert!(TEXLOAD_ROUNDS[1..].iter().all(|&r| r == 30));
}

#[test]
fn gc2v_moves_one_thing_at_a_time_from_ft2() {
    let t = Tex {
        t565: TEXTURES[0],
        t4444: TEXTURES[1],
        t8888: TEXTURES[2],
        plane: TEXTURES[3],
    };
    // Way 0 is FT2 exactly.
    assert_eq!(order_scene(Scene::Edge(0), t), scene(FT2, t));
    let base = EDGE_WAYS[0];
    for (i, e) in EDGE_WAYS.iter().enumerate().skip(1) {
        // Exactly one of the edge's row, the paint, the background changed.
        let changed = usize::from(e.y != base.y)
            + usize::from(e.paint != base.paint)
            + usize::from(e.gouraud != base.gouraud);
        assert_eq!(changed, 1, "way {i}");
        // The quad on the screen at every row.
        assert!(e.y >= 0 && e.y + FT2_QUAD.3 <= H as i32, "way {i}");
        let (bg, slots) = edge_scene(i, t);
        assert_eq!(bg, BLACK);
        let quad = if e.gouraud { slots[1] } else { slots[0] };
        let q = quad.unwrap();
        assert_eq!(
            q,
            Draw::rect(FT2_QUAD.0, e.y, FT2_QUAD.2, FT2_QUAD.3, q.paint)
        );
        assert_eq!(order_corner(Scene::Edge(i)), (FT2_QUAD.0, e.y));
    }
    // The rows: through tile row 2 (32-47) and the screen's top.
    let rows: std::vec::Vec<i32> = EDGE_WAYS[..6].iter().map(|e| e.y).collect();
    assert_eq!(rows, [40, 32, 36, 44, 47, 0]);
    // Half a texel of the side-32 texture down, every corner alike.
    if let Paint::Texture { uv, .. } = edge_scene(7, t).1[1].unwrap().paint {
        assert!(
            uv.iter()
                .all(|&(_, v)| v == UV_ONE / 64 || v == UV_ONE + UV_ONE / 64)
        );
        assert_eq!(8 << t.t8888.code, 32);
    } else {
        panic!("way 7 textured");
    }
    assert_eq!(
        EDGE_SCENES.map(|s| order_scene(s, t)),
        core::array::from_fn(|i| edge_scene(i, t))
    );
    assert!(
        EDGE_WAYS_ORDER
            .iter()
            .enumerate()
            .all(|(i, w)| *w == (i, ROWS))
    );
    // Every render was wrong at 146.25 in GC2U: ten tell.
    assert_eq!(EDGE_ROUNDS, 10);
}

#[test]
fn gc2v_hidden_layer_lies_under_the_whole_scene() {
    let t = Tex {
        t565: TEXTURES[0],
        t4444: TEXTURES[1],
        t8888: TEXTURES[2],
        plane: TEXTURES[3],
    };
    for (worst, base) in [(false, Scene::Kind(FT2)), (true, Scene::Sweep(0))] {
        let (bg, slots) = order_scene(Scene::Hidden(worst), t);
        let (bg0, slots0) = order_scene(base, t);
        assert_eq!(bg, bg0);
        // Opaque, full screen, first; the scene's own objects after it,
        // in order; the scene's first object its Gouraud background (the
        // whole screen; GC2K's worst's shifted by (3, 5), so the layer
        // shows in that strip, as its reference does).
        let layer = Draw::rect(0, 0, W as i32, H as i32, Paint::Flat(HIDDEN_LAYER));
        assert_eq!(slots[0], Some(layer));
        assert_eq!(slots[1..], slots0[..2]);
        assert_eq!(slots0[2], None);
        assert_eq!(slots0[0].map(|d| d.paint), Some(Paint::Gouraud(CORNERS)));
        assert_eq!(order_corner(Scene::Hidden(worst)), order_corner(base));
    }
    assert_eq!(HIDDEN_LAYER >> 24, 0xFF);
    assert_eq!(
        HIDDEN_SCENES,
        [Scene::Sweep(0), Scene::Busy(true), Scene::Hidden(true)]
    );
    assert!(HIDDEN_WAYS.iter().enumerate().all(|(i, w)| *w == (i, ROWS)));
    assert_eq!(HIDDEN_ROUNDS, 200);
}

#[test]
fn gc2y_cures_every_scene_with_both_of_gc2vs_cures() {
    let t = Tex {
        t565: TEXTURES[0],
        t4444: TEXTURES[1],
        t8888: TEXTURES[2],
        plane: TEXTURES[3],
    };
    for (i, &base) in CURED_BASE.iter().enumerate() {
        let (bg0, slots0) = order_scene(base, t);
        let (bg, slots) = order_scene(Scene::Cured(i), t);
        assert_eq!(bg, bg0, "{i}");
        // The hidden layer first, opaque, full screen; the scene's own two
        // objects after it, in order, every texture at texel centres.
        assert_eq!(
            slots[0],
            Some(Draw::rect(
                0,
                0,
                W as i32,
                H as i32,
                Paint::Flat(HIDDEN_LAYER)
            )),
            "{i}"
        );
        assert_eq!(slots0[2], None, "{i}: a free slot for the layer");
        for (c, o) in slots[1..].iter().zip(&slots0[..2]) {
            match (c.map(|d| d.paint), o.map(|d| d.paint)) {
                (
                    Some(Paint::Texture { tex, uv, tint }),
                    Some(Paint::Texture {
                        tex: tex0,
                        uv: uv0,
                        tint: tint0,
                    }),
                ) => {
                    assert_eq!((tex, tint), (tex0, tint0));
                    let half = UV_ONE / (2 * (8 << tex.code));
                    assert!(half > 0);
                    for (a, b) in uv.iter().zip(uv0) {
                        assert_eq!(*a, (b.0 + half, b.1 + half), "{i}");
                    }
                }
                (a, b) => assert_eq!(a, b, "{i}"),
            }
            assert_eq!(c.map(|d| d.corners), o.map(|d| d.corners), "{i}");
        }
        assert_eq!(order_corner(Scene::Cured(i)), order_corner(base), "{i}");
    }
    // FT2's quad: half of its side-32 texel, as GC2V's v shift.
    if let Some(Paint::Texture { uv, .. }) = order_scene(Scene::Cured(2), t).1[2].map(|d| d.paint) {
        assert_eq!(uv[0], (UV_ONE / 64, UV_ONE / 64));
    } else {
        panic!("FT2's quad textured");
    }
    // A scene with no free slot isn't cured.
    assert_eq!(cure(scene(4, t)), None);
    // The ways: the cured seven, then FT2 and GC2K's worst uncured.
    assert_eq!(CURED_SCENES[7..], [Scene::Kind(FT2), Scene::Sweep(0)]);
    assert!(CURED_WAYS.iter().enumerate().all(|(i, w)| *w == (i, ROWS)));
    assert_eq!(CURED_SOAK_WAYS, CURED_WAYS[..7]);
    // Every 16-bit kind of GC2's climb cured.
    for s in ABOVE_SCENES {
        assert!(CURED_BASE.contains(&s));
    }
    let mhz: std::vec::Vec<u32> = CURED_PLL.iter().map(|&(m, n)| 13_000 * m / n).collect();
    assert_eq!(mhz, [133_250, 139_750, 143_000, 146_250, 149_500]);
    assert_eq!(CURED_ROUNDS, [100, 30, 30, 30, 30]);
    assert_eq!(CURED_SOAK_MINUTES, 120);
}

#[test]
fn gc2z_fixes_shift_only_what_they_name_and_layer_in_order() {
    let t = Tex {
        t565: TEXTURES[0],
        t4444: TEXTURES[1],
        t8888: TEXTURES[2],
        plane: TEXTURES[3],
    };
    let half = |tex: Texture| UV_ONE / (2 * (8 << tex.code));
    for (f, fx) in FIXES.iter().enumerate() {
        for (b, &base) in FIX_BASE.iter().enumerate() {
            let (bg0, slots0) = order_scene(base, t);
            let (bg, slots) = order_scene_wide(Scene::Fix(b, f), t);
            assert_eq!(bg, bg0);
            let own: std::vec::Vec<Draw> = slots0.into_iter().flatten().collect();
            // Hidden layers first, then the scene's objects, then the busy
            // layer, then nothing.
            let hidden = Draw::rect(0, 0, W as i32, H as i32, Paint::Flat(HIDDEN_LAYER));
            assert!(
                slots[..fx.hidden].iter().all(|&d| d == Some(hidden)),
                "{f} {b}"
            );
            // Then the ring round each textured object, if the fix has
            // one.
            let mut ringed = 0;
            if fx.ring {
                for o in own
                    .iter()
                    .filter(|o| matches!(o.paint, Paint::Texture { .. }))
                {
                    for band in ring(o) {
                        assert_eq!(slots[fx.hidden + ringed], Some(band), "{f} {b}");
                        ringed += 1;
                    }
                }
            }
            let at = fx.hidden + ringed;
            for (k, o) in own.iter().enumerate() {
                let d = slots[at + k].unwrap();
                assert_eq!(d.corners, o.corners);
                match (d.paint, o.paint) {
                    (Paint::Texture { tex, uv, .. }, Paint::Texture { uv: uv0, .. }) => {
                        let clamped = u32::from(tex.u_mode) == address::CLAMP
                            && u32::from(tex.v_mode) == address::CLAMP;
                        let moved = !matches!(fx.tex, TexFix::ClampUv | TexFix::ClampV) || clamped;
                        let du = if moved && matches!(fx.tex, TexFix::ClampUv) {
                            half(tex)
                        } else {
                            0
                        };
                        let dv = if moved { half(tex) } else { 0 };
                        for (a, z) in uv.iter().zip(uv0) {
                            assert_eq!(*a, (z.0 + du, z.1 + dv), "{f} {b}");
                        }
                    }
                    (p, p0) => assert_eq!(p, p0),
                }
            }
            let mut n = at + own.len();
            if fx.busy {
                let busy = Draw::rect(0, 0, W as i32, H as i32, Paint::Flat(BUSY_LAYER)).over();
                assert_eq!(slots[n], Some(busy));
                n += 1;
            }
            assert!(slots[n..].iter().all(Option::is_none), "{f} {b}");
            assert_eq!(order_corner(Scene::Fix(b, f)), order_corner(base));
        }
    }
    // FT2's and GC2K's worst's textures are clamped, kinds 5 and 6's wrap:
    // the clamped-only fixes leave 5 and 6 as they are.
    for (b, &base) in FIX_BASE.iter().enumerate().skip(2) {
        let (_, plain) = order_scene_wide(base, t);
        let (_, fixed) = order_scene_wide(Scene::Fix(b, 2), t);
        assert_eq!(fixed[1..], plain[..SLOTS - 1]);
    }
    // A plain scene widened: its three slots, then nothing.
    let (bg, wide) = order_scene_wide(Scene::Sweep(0), t);
    let (bg0, three) = order_scene(Scene::Sweep(0), t);
    assert_eq!(bg, bg0);
    assert_eq!(wide[..3], three);
    assert!(wide[3..].iter().all(Option::is_none));
    // The plan: part 1 every base with the three shifts; part 2 GC2K's worst
    // with one, two, four hidden layers and one plus busy.
    assert_eq!(
        FIXES[..3]
            .iter()
            .map(|f| f.tex)
            .collect::<std::vec::Vec<_>>(),
        [TexFix::V, TexFix::ClampUv, TexFix::ClampV]
    );
    assert!(FIXES[..3].iter().all(|f| f.hidden == 1 && !f.busy));
    assert_eq!(
        FIX_TEX_SCENES,
        core::array::from_fn(|i| Scene::Fix(i % 4, i / 4))
    );
    assert!(
        FIX_TEX_WAYS
            .iter()
            .enumerate()
            .all(|(i, w)| *w == (i, ROWS))
    );
    assert_eq!(
        FIX_CORNER_SCENES.map(|s| match s {
            Scene::Fix(1, f) => (FIXES[f].hidden, FIXES[f].busy),
            _ => (0, false),
        }),
        [(1, false), (2, false), (4, false), (1, true)]
    );
    assert!(
        FIX_CORNER_WAYS
            .iter()
            .enumerate()
            .all(|(i, w)| *w == (i, ROWS))
    );
    let mhz: std::vec::Vec<u32> = FIX_TEX_PLL.iter().map(|&(m, n)| 13_000 * m / n).collect();
    assert_eq!(mhz, [143_000, 146_250, 149_500]);
    assert_eq!((FIX_TEX_ROUNDS, FIX_CORNER_ROUNDS), (30, 1000));
}

#[test]
fn gc2aa_tries_the_free_cures_and_soaks_every_kind() {
    // The two scenes: GC2K's worst, the clamped v shift, one hidden layer
    // and none.
    for (s, hidden) in FREE_SCENES.iter().zip([1, 0]) {
        let Scene::Fix(b, f) = *s else {
            panic!("a fix");
        };
        assert_eq!(FIX_BASE[b], Scene::Sweep(0));
        assert_eq!(
            FIXES[f],
            Fix {
                tex: TexFix::ClampV,
                hidden,
                busy: false,
                ring: false
            }
        );
    }
    assert_eq!(
        FREE_WAYS,
        [
            (0, ROWS),
            (0, COLUMNS_REVERSED),
            (1, COLUMNS_REVERSED),
            (0, REVERSED)
        ]
    );
    let mut p = FREE_PREFER;
    p.sort_unstable();
    assert_eq!(p, [0, 1, 2, 3]);
    assert_eq!(FREE_PREFER[3], 0, "the control last");
    let mhz: std::vec::Vec<(u32, u32)> = FREE_PLL
        .iter()
        .map(|&(m, n, r)| (13_000 * m / n, r))
        .collect();
    assert_eq!(mhz, [(139_750, 1000), (146_250, 500), (149_500, 500)]);
    // The soak: every 16-bit kind and GC2K's worst, once each, with the
    // winner's hidden layers, in each order.
    let base: std::vec::Vec<Scene> = FIX_BASE.to_vec();
    for k in [0, 1, FT2, 5, 6, 7] {
        assert!(base.contains(&Scene::Kind(k)), "{k}");
    }
    assert!(base.contains(&Scene::Sweep(0)));
    for (scenes, f) in FREE_SOAK_SCENES.iter().zip([2, 6]) {
        assert_eq!(*scenes, core::array::from_fn(|b| Scene::Fix(b, f)));
    }
    for (o, ways) in FREE_SOAK_WAYS.iter().enumerate() {
        assert!(ways.iter().enumerate().all(|(i, w)| *w == (i, o as u8)));
    }
    assert_eq!([ROWS, REVERSED, COLUMNS_REVERSED], [0, 1, 2]);
    assert_eq!(FREE_SOAK_MINUTES, 60);
}

#[test]
fn gc2ab_ranks_the_hidden_layers_on_the_light_kinds_reversed() {
    // Kinds 0 and 1 with no hidden layer, one, two; GC2K's worst with one
    // and two; every way reversed; the clamped v shift throughout.
    let want = [
        (Scene::Kind(0), 0),
        (Scene::Kind(0), 1),
        (Scene::Kind(0), 2),
        (Scene::Kind(1), 0),
        (Scene::Kind(1), 1),
        (Scene::Kind(1), 2),
        (Scene::Sweep(0), 1),
        (Scene::Sweep(0), 2),
    ];
    for (s, (base, hidden)) in LASTPIX_SCENES.iter().zip(want) {
        let Scene::Fix(b, f) = *s else {
            panic!("a fix");
        };
        assert_eq!(FIX_BASE[b], base);
        assert_eq!(FIXES[f].hidden, hidden);
        assert_eq!(FIXES[f].tex, TexFix::ClampV);
        assert!(!FIXES[f].busy);
    }
    assert!(
        LASTPIX_WAYS
            .iter()
            .enumerate()
            .all(|(i, w)| *w == (i, REVERSED))
    );
    let mhz: std::vec::Vec<u32> = LASTPIX_PLL.iter().map(|&(m, n)| 13_000 * m / n).collect();
    // One step above 149.50, below GC2H2's failing 156; then 149.50.
    assert_eq!(mhz, [152_750, 149_500]);
    assert!(mhz[0] < 156_000);
    assert_eq!(LASTPIX_ROUNDS, 800);
}

#[test]
fn gc2ac_ring_covers_the_textured_edges_once_and_stays_hidden() {
    let t = Tex {
        t565: TEXTURES[0],
        t4444: TEXTURES[1],
        t8888: TEXTURES[2],
        plane: TEXTURES[3],
    };
    // GC2K's worst's quad: (43, 45), 400 x 560.
    let quad = Draw::rect(43, 45, 400, 560, Paint::Flat(0));
    let px = |d: &Draw| {
        let c = d.corners;
        (
            c[0].x / rp_mbx3d_render::SUB,
            c[0].y / rp_mbx3d_render::SUB,
            c[2].x / rp_mbx3d_render::SUB,
            c[2].y / rp_mbx3d_render::SUB,
        )
    };
    let bands = ring(&quad).map(|b| px(&b));
    assert_eq!(
        bands,
        [
            (27, 29, 459, 61),
            (27, 589, 459, 621),
            (27, 61, 59, 589),
            (427, 61, 459, 589),
        ]
    );
    for b in ring(&quad) {
        assert_eq!(b.paint, Paint::Flat(HIDDEN_LAYER));
        assert_eq!(b.blend, None);
    }
    // Every pixel within RING_REACH of an edge is covered exactly once,
    // every other pixel not at all (edges and both directions: the
    // quad's inside and outside, the corners).
    let inside =
        |p: (i32, i32, i32, i32), x: i32, y: i32| x >= p.0 && x < p.2 && y >= p.1 && y < p.3;
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            let near = |v: i32, edge: i32| (edge - RING_REACH..edge + RING_REACH).contains(&v);
            let near_x = near(x, 43) || near(x, 443);
            let near_y = near(y, 45) || near(y, 605);
            let in_x = (43 - RING_REACH..443 + RING_REACH).contains(&x);
            let in_y = (45 - RING_REACH..605 + RING_REACH).contains(&y);
            let want = u32::from((near_x && in_y) || (near_y && in_x));
            let got = bands.iter().filter(|&&b| inside(b, x, y)).count() as u32;
            assert_eq!(got, want, "({x}, {y})");
        }
    }
    // Clipped to the screen: a full-screen quad's ring lies inside it.
    let full = Draw::rect(0, 0, W as i32, H as i32, Paint::Flat(0));
    for b in ring(&full).map(|b| px(&b)) {
        assert!(
            b.0 >= 0 && b.1 >= 0 && b.2 <= W as i32 && b.3 <= H as i32,
            "{b:?}"
        );
        assert!(b.0 < b.2 && b.1 < b.3, "{b:?}");
    }
    // The ring is hidden: GC2K's worst's Gouraud background (shifted
    // (3, 5)) covers every band, so the scene's picture is the cure-less
    // one's.
    let (_, slots) = order_scene(Scene::Sweep(0), t);
    let g = px(&slots[0].unwrap());
    for b in bands {
        assert!(
            b.0 >= g.0 && b.1 >= g.1 && b.2 <= g.2 && b.3 <= g.3,
            "{b:?}"
        );
    }
    // Ring fixes: the clamped v shift, no busy layer, 0 and 1 full layers;
    // every fixed scene still fits its slots.
    assert_eq!(
        FIXES[7],
        Fix {
            tex: TexFix::ClampV,
            hidden: 0,
            busy: false,
            ring: true
        }
    );
    assert_eq!(
        FIXES[8],
        Fix {
            tex: TexFix::ClampV,
            hidden: 1,
            busy: false,
            ring: true
        }
    );
    assert!(FIXES[..7].iter().all(|f| !f.ring));
    for (f, fx) in FIXES.iter().enumerate() {
        for (b, &base) in FIX_BASE.iter().enumerate() {
            let (_, own) = order_scene(base, t);
            let tex = own
                .iter()
                .flatten()
                .filter(|d| matches!(d.paint, Paint::Texture { .. }))
                .count();
            let need = fx.hidden
                + usize::from(fx.ring) * 4 * tex
                + own.iter().flatten().count()
                + usize::from(fx.busy);
            assert!(need <= SLOTS, "{f} {b}");
        }
    }
}

#[test]
fn gc2ac_ranks_the_ring_against_the_full_layers_at_149_50() {
    // GC2K's worst throughout: no cure, one full, the ring, the ring and
    // one full, two full.
    let want = [
        (6, 0, false),
        (2, 1, false),
        (7, 0, true),
        (8, 1, true),
        (3, 2, false),
    ];
    for (s, (f, hidden, ring)) in RING_SCENES.iter().zip(want) {
        assert_eq!(*s, Scene::Fix(1, f));
        assert_eq!(FIX_BASE[1], Scene::Sweep(0));
        assert_eq!((FIXES[f].hidden, FIXES[f].ring), (hidden, ring));
        assert_eq!(FIXES[f].tex, TexFix::ClampV);
        assert!(!FIXES[f].busy);
    }
    assert_eq!(
        RING_WAYS,
        [
            (0, ROWS),
            (1, ROWS),
            (2, ROWS),
            (3, ROWS),
            (4, ROWS),
            (1, REVERSED),
            (3, REVERSED)
        ]
    );
    assert_eq!(13_000 * RING_PLL.0 / RING_PLL.1, 149_500);
    assert_eq!(RING_ROUNDS, 500);
}

#[test]
fn gc2ad_soaks_every_kind_alternating_the_ring_and_one_full_layer_reversed() {
    // Every base scene: the ring, then one full layer, then (the textured
    // four only) both at once, in that order.
    let mut want = std::vec::Vec::new();
    for b in 0..FIX_BASE.len() {
        want.push(Scene::Fix(b, 7));
        want.push(Scene::Fix(b, 2));
        if b < NIGHT_TEXTURED {
            want.push(Scene::Fix(b, 8));
        }
    }
    assert_eq!(NIGHT_SCENES.to_vec(), want);
    assert_eq!(FIXES[8], ring_fix(1));
    // The textured four are exactly the bases with a textured object: the
    // ring draws bands there and nowhere else.
    for (b, &base) in FIX_BASE.iter().enumerate() {
        let tex = test_tex();
        let (_, slots) = order_scene(base, tex);
        let textured = slots
            .iter()
            .flatten()
            .any(|d| matches!(d.paint, Paint::Texture { .. }));
        assert_eq!(textured, b < NIGHT_TEXTURED, "base {b}");
        // Both cures at once: one full layer, four ring bands a textured
        // object, the scene's own objects, all within the renderer's slots
        // (fix_scene would panic past them).
        let objects = slots.iter().flatten().count();
        let rings = slots
            .iter()
            .flatten()
            .filter(|d| matches!(d.paint, Paint::Texture { .. }))
            .count();
        let (_, both) = fix_scene(b, 8, tex);
        assert_eq!(both.iter().flatten().count(), 1 + 4 * rings + objects, "base {b}");
    }
    assert_eq!(
        FIXES[7],
        Fix {
            tex: TexFix::ClampV,
            hidden: 0,
            busy: false,
            ring: true
        }
    );
    assert_eq!(
        FIXES[2],
        Fix {
            tex: TexFix::ClampV,
            hidden: 1,
            busy: false,
            ring: false
        }
    );
    // Every 16-bit kind (0, 1, FT2, 5, 6, 7) and GC2K's worst.
    let mut kinds: std::vec::Vec<std::string::String> =
        FIX_BASE.iter().map(|s| std::format!("{s:?}")).collect();
    let mut want: std::vec::Vec<std::string::String> = [
        Scene::Kind(0),
        Scene::Kind(1),
        Scene::Kind(FT2),
        Scene::Kind(5),
        Scene::Kind(6),
        Scene::Kind(7),
        Scene::Sweep(0),
    ]
    .iter()
    .map(|s| std::format!("{s:?}"))
    .collect();
    kinds.sort();
    want.sort();
    assert_eq!(kinds, want);
    // Every way reversed, each scene once, in turn.
    assert!(
        NIGHT_WAYS
            .iter()
            .enumerate()
            .all(|(i, w)| *w == (i, REVERSED))
    );
    assert_eq!(13_000 * NIGHT_PLL.0 / NIGHT_PLL.1, 149_500);
    assert_eq!(NIGHT_MINUTES, 120);
}

/// The last-pixel fault's suspects read the right pixels: both halves of
/// a 16-bit word, a 32-bit picture's word, the picture's edges, a tile's
/// last pixel and the same place in the tiles around it.
#[test]
fn suspects_read_the_right_pixels() {
    let f16 = PixelFormat::Rgb565;
    let f32 = PixelFormat::Argb8888;
    // Each 16-bit pixel holds its own index (low 16 bits).
    let n = (W * H) as usize;
    let pic16: std::vec::Vec<u32> = (0..n / 2)
        .map(|i| (2 * i as u32 & 0xFFFF) | ((2 * i as u32 + 1) & 0xFFFF) << 16)
        .collect();
    let idx = |x: i32, y: i32| (y as u32 * W + x as u32) & 0xFFFF;
    for (x, y) in [(0, 0), (1, 0), (54, 415), (55, 415), (479, 639), (478, 639)] {
        assert_eq!(pixel(f16, &pic16, x, y), Some(idx(x, y)), "({x},{y})");
    }
    let pic32: std::vec::Vec<u32> = (0..n as u32).collect();
    assert_eq!(pixel(f32, &pic32, 55, 415), Some(415 * W + 55));
    assert_eq!(pixel(f32, &pic32, 479, 639), Some(n as u32 - 1));
    // Off the picture, either side, and past a short buffer.
    for (x, y) in [(-1, 0), (480, 0), (0, -1), (0, 640)] {
        assert_eq!(pixel(f16, &pic16, x, y), None, "({x},{y})");
        assert_eq!(pixel(f32, &pic32, x, y), None, "({x},{y})");
    }
    assert_eq!(pixel(f16, &pic16[..10], 20, 0), None);
    // The word's pair: the other half at 16 bits, itself at 32.
    assert_eq!(pair(f16, 55, 415), (54, 415));
    assert_eq!(pair(f16, 54, 415), (55, 415));
    assert_eq!(pair(f32, 55, 415), (55, 415));
    // GC2AA's fault at (55, 415): the last place of its 8 x 16 tile; one
    // further is the next tile's first.
    assert_eq!(in_tile(55, 415), (7, 15));
    assert_eq!(in_tile(56, 416), (0, 0));
    assert_eq!(in_tile(0, 0), (0, 0));
    assert_eq!(
        tile_mates(55, 415),
        [(47, 415), (63, 415), (55, 399), (55, 431)]
    );
    // At the picture's corner the mates off it read as none.
    let m = tile_mates(0, 0);
    assert_eq!(pixel(f16, &pic16, m[0].0, m[0].1), None);
    assert_eq!(pixel(f16, &pic16, m[2].0, m[2].1), None);
    assert_eq!(pixel(f16, &pic16, m[1].0, m[1].1), Some(idx(8, 0)));
}

/// The poison is a colour no reference holds: either half of a word
/// rules a candidate out, the first left wins, none left is none.
#[test]
fn the_poison_is_in_no_reference() {
    let c = [0xF81F, 0x801F, 0x07E1];
    assert_eq!(absent16(&[0x1234_5678], &c), Some(0xF81F));
    // The low half and the high half each rule one out.
    assert_eq!(absent16(&[0x0000_F81F], &c), Some(0x801F));
    assert_eq!(absent16(&[0xF81F_0000, 0x801F_1111], &c), Some(0x07E1));
    assert_eq!(absent16(&[0xF81F_801F, 0x07E1_0000], &c), None);
    assert_eq!(absent16(&[], &c), Some(0xF81F));
    assert_eq!(absent16(&[0], &[]), None);
    // Every GC2 colour tried is a distinct 16-bit value.
    let mut p = POISON.to_vec();
    p.sort_unstable();
    p.dedup();
    assert_eq!(p.len(), POISON.len());
}

/// The four textures at their table places (no memory: the scenes' shape
/// only).
fn test_tex() -> Tex {
    Tex {
        t565: TEXTURES[0],
        t4444: TEXTURES[1],
        t8888: TEXTURES[2],
        plane: TEXTURES[3],
    }
}
