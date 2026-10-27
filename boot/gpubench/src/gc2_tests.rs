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
        assert_eq!(
            both.iter().flatten().count(),
            1 + 4 * rings + objects,
            "base {b}"
        );
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
        .map(|i| ((2 * i as u32) & 0xFFFF) | (((2 * i as u32 + 1) & 0xFFFF) << 16))
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

/// GC2AE's source search: the nearest place holding a value, ties to the
/// first in row order, the pixel itself left out, the window's edges and
/// the picture's both counted right.
#[test]
fn the_source_search_finds_the_nearest_and_counts() {
    let f = PixelFormat::Rgb565;
    let n = (W * H) as usize;
    let mut pic = std::vec![0u32; n / 2];
    let mut put = |x: u32, y: u32, v: u32| {
        let at = (y * W + x) as usize;
        let sh = 16 * (at % 2);
        pic[at / 2] = (pic[at / 2] & !(0xFFFF << sh)) | (v << sh);
    };
    put(100, 100, 7); // the pixel itself: never its own source
    put(100, 84, 7); // one tile above
    put(104, 100, 7); // nearer: 4 to the right
    put(100, 140, 7); // the window's edge (ry 40)
    put(100, 141, 7); // past it
    assert_eq!(nearest(f, &pic, (100, 100), 7, (24, 40)), (Some((4, 0)), 3));
    // Equal distances: the first in row order (above before below).
    let mut pic2 = std::vec![0u32; n / 2];
    let mut put2 = |x: u32, y: u32, v: u32| {
        let at = (y * W + x) as usize;
        let sh = 16 * (at % 2);
        pic2[at / 2] = (pic2[at / 2] & !(0xFFFF << sh)) | (v << sh);
    };
    put2(50, 34, 9);
    put2(50, 66, 9);
    assert_eq!(
        nearest(f, &pic2, (50, 50), 9, (24, 40)),
        (Some((0, -16)), 2)
    );
    // Nothing in the window; at the picture's corner the window is clipped.
    assert_eq!(nearest(f, &pic2, (400, 400), 9, (24, 40)), (None, 0));
    assert_eq!(nearest(f, &pic, (0, 0), 7, (24, 40)), (None, 0));
    assert_eq!(count(f, &pic, 7), 5);
    assert_eq!(count(f, &pic, 0), W * H - 5);
}

/// GC2AE's plan: R36's fault generator (FT2, both cures), its clocks as
/// named, and the margin's seven scenes with one full layer, reversed.
#[test]
fn gc2ae_traces_the_both_cures_way_and_checks_the_margin() {
    assert_eq!(TRACE_SCENES, [NIGHT_SCENES[2]]);
    assert_eq!(TRACE_SCENES[0], Scene::Fix(0, 8));
    assert_eq!(TRACE_REVERSED, [(0, REVERSED)]);
    assert_eq!(TRACE_ROWS, [(0, ROWS)]);
    for ((m, n), name) in TRACE_STEPS {
        let khz = 13_000 * m / n;
        assert_eq!(
            std::format!("{}.{:02} / 1", khz / 1000, khz % 1000 / 10),
            name
        );
    }
    for (b, s) in MARGIN_SCENES.iter().enumerate() {
        assert_eq!(*s, Scene::Fix(b, 2));
        assert_eq!(MARGIN_WAYS[b], (b, REVERSED));
    }
    assert_eq!(MARGIN_SCENES.len(), FIX_BASE.len());
    // At R36's rate for the fault generator (160 of 3,299), the trace
    // expects about 29 faults and each condition about 19: enough to tell
    // a rate halved or zeroed apart from no change.
    const { assert!(TRACE_RENDERS * 160 / 3_299 >= 20) };
    const { assert!(TRACE_CONDITION * 160 / 3_299 >= 15) };
    // The margin at least R36's renders a scene in a tenth of the time.
    const { assert!(MARGIN_ROUNDS >= 3_299 / 11) };
}

/// GC2AG's untextured way: every textured object flat, nothing else moved,
/// no texture left in the scene, and the same objects in the same slots.
#[test]
fn untextured_takes_every_texture_out_and_nothing_else() {
    let (_, slots) = fix_scene(0, 8, test_tex());
    let flat = untextured(slots);
    assert!(
        slots
            .iter()
            .flatten()
            .any(|d| matches!(d.paint, Paint::Texture { .. }))
    );
    for (a, b) in slots.iter().zip(flat.iter()) {
        match (a, b) {
            (None, None) => {}
            (Some(a), Some(b)) => {
                if matches!(a.paint, Paint::Texture { .. }) {
                    assert_eq!(b.paint, Paint::Flat(UNTEXTURED));
                    assert_eq!(
                        Draw {
                            paint: a.paint,
                            ..*b
                        },
                        *a,
                        "only the paint changed"
                    );
                } else {
                    assert_eq!(a, b);
                }
            }
            _ => panic!("a slot appeared or went"),
        }
    }
}

/// GC2AH's geometry ways: no texture anywhere, the layers first, the ring
/// at its reach (16 puts the bands' right edge on the x 456 tile seam, 12
/// mid-tile at x 452), FT2's other objects last and unchanged.
#[test]
fn gc2ah_geometry_ways_are_built_as_named() {
    let t = test_tex();
    let (_, base) = order_scene(FIX_BASE[0], t);
    let others: std::vec::Vec<Draw> = base
        .iter()
        .flatten()
        .filter(|d| !matches!(d.paint, Paint::Texture { .. }))
        .copied()
        .collect();
    for (w, gw) in GEO.iter().enumerate() {
        let (_, s) = geo_scene(w, t);
        let s: std::vec::Vec<Draw> = s.iter().flatten().copied().collect();
        assert!(
            !s.iter().any(|d| matches!(d.paint, Paint::Texture { .. })),
            "way {w}"
        );
        let bands = if gw.reach == 0 { 0 } else { 4 };
        assert_eq!(s.len(), gw.layers + bands + others.len(), "way {w}");
        assert!(
            s[..gw.layers]
                .iter()
                .all(|d| d.paint == Paint::Flat(HIDDEN_LAYER))
        );
        assert_eq!(&s[gw.layers + bands..], &others[..], "way {w}");
    }
    let edge = |w: usize| {
        let (_, s) = geo_scene(w, t);
        let top = s
            .iter()
            .flatten()
            .nth(GEO[w].layers)
            .copied()
            .expect("the top band");
        top.corners
            .map(|p| p.x / rp_mbx3d_render::SUB)
            .into_iter()
            .max()
            .unwrap_or(0)
    };
    assert_eq!(edge(0), 456);
    assert_eq!(edge(0) % 8, 0, "on a tile seam");
    assert_eq!(edge(4), 452);
    assert_ne!(edge(4) % 8, 0, "mid-tile");
    assert_eq!(GEO_WAYS[..2], [(0, REVERSED), (0, ROWS)]);
    assert_eq!(
        GEO[3],
        GeoWay {
            layers: 2,
            reach: 0
        }
    );
    // Each scene is its way, and every way's scene is run.
    for (i, sc) in GEO_SCENES.iter().enumerate() {
        assert_eq!(*sc, Scene::Geo(i));
        assert!(GEO_WAYS.iter().any(|w| w.0 == i), "scene {i} run");
    }
    // At GC2AG's untextured rate (17 of 400) way 0 expects about 17 faults
    // in its rounds: enough to place them; none in way 3 is then telling.
    const { assert!(GEO_ROUNDS >= 400) };
}

/// GC2AI's ways: way 0 is GC2AG's untextured fault generator object for
/// object; each other way moves only the flat quad (or draws it first),
/// the layer and the ring always where FT2's quad puts them.
#[test]
fn gc2ai_moves_only_the_quad() {
    let t = test_tex();
    let (_, generator) = fix_scene(0, 8, t);
    let generator: std::vec::Vec<Draw> = untextured(generator).iter().flatten().copied().collect();
    let (_, w0) = quad_scene(0, t);
    let w0: std::vec::Vec<Draw> = w0.iter().flatten().copied().collect();
    assert_eq!(w0, generator, "way 0 is GC2AG's untextured scene");
    let is_quad = |d: &Draw| d.paint == Paint::Flat(UNTEXTURED);
    for (w, qw) in QUAD_WAYS.iter().enumerate() {
        let (_, s) = quad_scene(w, t);
        let s: std::vec::Vec<Draw> = s.iter().flatten().copied().collect();
        assert_eq!(s.len(), w0.len(), "way {w}");
        let (x0, y0, x1, y1) = qw.rect;
        let quad = Draw::rect(x0, y0, x1 - x0, y1 - y0, Paint::Flat(UNTEXTURED));
        assert_eq!(s.iter().filter(|d| is_quad(d)).count(), 1, "way {w}");
        assert_eq!(s.iter().position(&is_quad) == Some(0), qw.first, "way {w}");
        assert!(s.contains(&quad), "way {w}");
        let rest: std::vec::Vec<Draw> = s.iter().filter(|d| !is_quad(d)).copied().collect();
        let rest0: std::vec::Vec<Draw> = w0.iter().filter(|d| !is_quad(d)).copied().collect();
        assert_eq!(rest, rest0, "way {w}: only the quad changes");
    }
    for (i, sc) in QUAD_SCENES.iter().enumerate() {
        assert_eq!(*sc, Scene::Quad(i));
        assert_eq!(QUAD_ORDER[i], (i, REVERSED));
    }
    const { assert!(QUAD_ROUNDS >= 400) };
}

/// GC2AJ's sweep: every one of the 128 places of an 8 x 16 tile is some
/// rectangle's, every rectangle's four corners sit at its place, the
/// rectangles keep apart and on the screen, and each scene is the layer,
/// FT2's Gouraud and the six rectangles; the flat plan ends with GC2AI's
/// faulting way as its control.
#[test]
fn gc2aj_sweeps_every_corner_place_of_a_tile() {
    let mut seen = [false; 128];
    for i in 0..CORNER_SCENES_EACH {
        let mut rects = std::vec::Vec::new();
        for j in 0..CORNER_RECTS {
            let ((x0, y0, x1, y1), k) = corner_rect(i, j);
            seen[k] = true;
            let (a, c) = ((k % 8) as i32, (k / 8) as i32);
            for x in [x0, x1] {
                assert_eq!(x.rem_euclid(8), a, "scene {i} rect {j}");
            }
            for y in [y0, y1] {
                assert_eq!(y.rem_euclid(16), c, "scene {i} rect {j}");
            }
            assert!(x0 >= 0 && y0 >= 0 && x1 <= W as i32 && y1 <= H as i32);
            assert!(x1 - x0 >= 16 && y1 - y0 >= 32, "edges in different tiles");
            rects.push((x0, y0, x1, y1));
        }
        for (p, a) in rects.iter().enumerate() {
            for b in &rects[p + 1..] {
                assert!(
                    a.2 <= b.0 || b.2 <= a.0 || a.3 <= b.1 || b.3 <= a.1,
                    "scene {i} overlap"
                );
            }
        }
        for textured in [false, true] {
            let (_, s) = corner_scene(textured, i, test_tex());
            let s: std::vec::Vec<Draw> = s.iter().flatten().copied().collect();
            assert_eq!(s.len(), 2 + CORNER_RECTS);
            assert_eq!(s[0].paint, Paint::Flat(HIDDEN_LAYER));
            assert!(matches!(s[1].paint, Paint::Gouraud(_)));
            assert!(
                s[2..]
                    .iter()
                    .all(|d| matches!(d.paint, Paint::Texture { .. }) == textured)
            );
        }
    }
    assert!(seen.iter().all(|&b| b), "every place swept");
    assert_eq!(CORNER_FLAT[CORNER_SCENES_EACH], Scene::Quad(0));
    assert_eq!(CORNER_FLAT[3], Scene::Corner(false, 3));
    assert_eq!(CORNER_TEX[3], Scene::Corner(true, 3));
    assert!(
        CORNER_WAYS
            .iter()
            .enumerate()
            .all(|(i, w)| *w == (i, REVERSED))
    );
    const { assert!(CORNER_ROUNDS >= 200) };
    const { assert!(CORNER_SCENES_EACH < 42) };
}

/// GC2BB's low-overdraw scene is the flat corner scene with only its
/// hidden layer (slot 0) taken out: the same picture, a third of the
/// overdraw.
#[test]
fn gc2bb_no_layer_scene_is_the_corner_scene_without_slot_0() {
    let t = test_tex();
    for i in [0, 20, CORNER_SCENES_EACH - 1] {
        let (bg, with) = order_scene_wide(Scene::Corner(false, i), t);
        let (bg2, without) = order_scene_wide(Scene::CornerNoLayer(i), t);
        assert_eq!(bg, bg2);
        assert!(with[0].is_some(), "the corner scene's hidden layer");
        assert!(without[0].is_none(), "taken out");
        assert_eq!(
            with[1..],
            without[1..],
            "scene {i}: everything else the same"
        );
    }
}

/// GC2AK's ladder: the u and v sweep is GC2AJ's textured sweep with only
/// the shift changed; its settings are the named clocks, fastest first,
/// divide 1 then 149.50 / 2.
#[test]
fn gc2ak_sweeps_uv_at_each_setting() {
    let t = test_tex();
    for (i, sc) in CORNER_UV.iter().enumerate() {
        assert_eq!(*sc, Scene::CornerUv(i));
        let (_, v) = corner_scene(true, i, t);
        let (_, uv) = corner_uv_scene(i, t);
        let (v, uv): (std::vec::Vec<Draw>, std::vec::Vec<Draw>) = (
            v.iter().flatten().copied().collect(),
            uv.iter().flatten().copied().collect(),
        );
        assert_eq!(v.len(), uv.len());
        assert_eq!(v[..2], uv[..2], "the layer and the Gouraud unchanged");
        for (a, b) in v[2..].iter().zip(&uv[2..]) {
            assert_eq!(a.corners, b.corners);
            let (Paint::Texture { uv: ua, .. }, Paint::Texture { uv: ub, .. }) = (a.paint, b.paint)
            else {
                panic!("textured");
            };
            // v the same as GC2AJ's; u moved half a texel in.
            assert_eq!(ua.map(|p| p.1), ub.map(|p| p.1));
            assert_ne!(ua.map(|p| p.0), ub.map(|p| p.0));
        }
    }
    let mut last = u32::MAX;
    for ((m, n), div, name) in LADDER_STEPS {
        let khz = 13_000 * m / n;
        assert_eq!(
            std::format!("{}.{:02} / {div}", khz / 1000, khz % 1000 / 10),
            name
        );
        if div == 1 {
            assert!(khz < last, "fastest first");
            last = khz;
        }
    }
    assert_eq!(LADDER_STEPS[5].1, 2);
    const { assert!(LADDER_ROUNDS >= 100) };
}

/// GC2AL's cures: five rectangles a scene cover all 128 places, the same
/// rectangles in every cure, only the layers or the blended layer differ,
/// and every scene fits the renderer's slots.
#[test]
fn gc2al_cures_differ_only_in_their_cure() {
    let t = test_tex();
    let mut seen = [false; 128];
    for i in 0..CURE_SCENES {
        for j in 0..CURE_RECTS {
            seen[corner_rect_of(CURE_RECTS, i, j).1] = true;
        }
        let scene = |c| -> std::vec::Vec<Draw> {
            let (_, s) = cure_scene(c, i, t);
            s.iter().flatten().copied().collect()
        };
        assert_eq!(CURES, 3, "one layer, two layers, the blended layer");
        let (one, two, busy) = (scene(0), scene(1), scene(2));
        assert_eq!(one.len(), 2 + CURE_RECTS);
        assert_eq!(two.len(), 3 + CURE_RECTS);
        assert_eq!(busy.len(), 3 + CURE_RECTS);
        assert!(two.len() <= SLOTS && busy.len() <= SLOTS);
        assert_eq!(two[0], two[1], "two full layers");
        assert_eq!(two[1..], one[..], "then the same scene");
        assert_eq!(busy[..one.len()], one[..], "the same scene first");
        let last = busy[one.len()];
        assert_eq!(last.paint, Paint::Flat(INVISIBLE));
        assert_ne!(last, flat(INVISIBLE, false), "blended, not opaque");
        assert!(
            one[2..]
                .iter()
                .all(|d| matches!(d.paint, Paint::Texture { .. }))
        );
    }
    assert!(seen.iter().all(|&b| b), "every place swept");
    for i in 0..CURE_SCENES {
        assert_eq!(CURE_ONE[i], Scene::Cure(0, i));
        assert_eq!(CURE_TWO[i], Scene::Cure(1, i));
        assert_eq!(CURE_BUSY[i], Scene::Cure(2, i));
        assert_eq!((CURE_REVERSED[i], CURE_ROWS[i]), ((i, REVERSED), (i, ROWS)));
    }
    assert_eq!(corner_rect_of(CORNER_RECTS, 3, 4), corner_rect(3, 4));
    const { assert!(CURE_SCENES < 42 && CURE_ROUNDS >= 100) };
}

/// GC2AM: the phase scenes move only the one rectangle's u, a step at a
/// time; the safe ratios' best phases keep every sample at the promised
/// margin and 5.75:1 can't do better than 1/46 of a texel; the ratio
/// scenes' rectangles keep apart and on the screen.
#[test]
fn gc2am_measures_the_error_and_keeps_the_margin() {
    let t = test_tex();
    let u_of = |n: usize| {
        let (_, s) = phase_scene(n, t);
        let d = s[2].expect("the rectangle");
        let Paint::Texture { uv, .. } = d.paint else {
            panic!("textured");
        };
        (d.corners, uv)
    };
    let (c0, uv0) = u_of(0);
    for n in 1..PHASES {
        let (c, uv) = u_of(n);
        assert_eq!(c, c0, "the rectangle never moves");
        for (a, b) in uv0.iter().zip(&uv) {
            assert_eq!((b.0 - a.0, b.1), (n as i32 * PHASE_STEP, a.1));
        }
    }
    // The sweep spans one period of the pattern at 32 texels on 184 px.
    let period = UV_ONE / 32 / 23;
    assert!((PHASES as i32 - 1) * PHASE_STEP >= period);
    assert_eq!(PHASE_RECT.2 - PHASE_RECT.0, 184);
    // The best phase's margin, in 65,536ths of a texel, over 200 pixels.
    let at = |px: i64, tx: i64, p: i64| margin(px, tx, p, 200);
    let one = 65_536i64;
    for (px, tx) in RATIOS {
        let ph = i64::from(best_phase(px, 32)) * 32;
        assert_eq!(
            at(i64::from(px), i64::from(tx), ph),
            one / (2 * i64::from(px)),
            "{px}:{tx}"
        );
    }
    // At 5.75:1 (23 pixels for 4 texels) no phase beats 1/46 of a texel.
    let best = (0..one / 23).map(|p| at(23, 4, p)).max().unwrap_or(0);
    assert!(best <= one / 46 + 1 && best >= one / 46 - 1, "{best}");
    // The ratio scenes: five rectangles apart, on the screen, at size.
    for r in 0..RATIOS.len() {
        for (i, sc) in RATIO_SCENES[r].iter().enumerate() {
            assert_eq!(*sc, Scene::Ratio(r, i));
            let (_, s) = ratio_scene(r, i, t);
            let rects: std::vec::Vec<Draw> = s[2..].iter().flatten().copied().collect();
            assert_eq!(rects.len(), CURE_RECTS);
            let (px, tx) = RATIOS[r];
            for d in &rects {
                let xs = d.corners.map(|p| p.x / rp_mbx3d_render::SUB);
                let ys = d.corners.map(|p| p.y / rp_mbx3d_render::SUB);
                let w = xs.iter().max().unwrap() - xs.iter().min().unwrap();
                assert_eq!(w, 32 * px / tx);
                assert!(
                    *xs.iter().max().unwrap() <= W as i32 && *ys.iter().max().unwrap() <= H as i32
                );
            }
        }
    }
    assert!(
        PHASE_SCENES
            .iter()
            .enumerate()
            .all(|(i, s)| *s == Scene::Phase(i))
    );
    assert!(
        PHASE_WAYS
            .iter()
            .enumerate()
            .all(|(i, w)| *w == (i, REVERSED))
    );
    const { assert!(PHASES < 42 && PHASE_ROUNDS >= 3 && RATIO_ROUNDS >= 30) };
}

/// GC2AN's plan: the untextured fault generator (GC2AG's) in all six
/// orders, each order once, the baseline (reversed) first.
#[test]
fn gc2an_tries_every_order_on_the_fault_generator() {
    assert_eq!(ORDER_TRY_SCENES, [Scene::Fix(0, 8)]);
    let orders: std::vec::Vec<u8> = ORDER_TRY.iter().map(|w| w.1).collect();
    assert_eq!(
        orders,
        [
            REVERSED,
            ROWS,
            COLUMNS_REVERSED,
            COLUMNS,
            CHECKER,
            SCRAMBLED
        ]
    );
    assert!(ORDER_TRY.iter().all(|w| w.0 == 0));
    // At GC2AG's 17 of 400, an order with none in 400 is a real change.
    const { assert!(ORDER_TRY_ROUNDS >= 400) };
}

/// GC2AO: the centre phase keeps the most margin any phase can (1/(2p)
/// texel at p / q pixels a texel), 1:1 needs none, the proof scenes
/// differ only in the layer and the paint, and the textured ones carry
/// the centre phase in u and v.
#[test]
fn gc2ao_puts_every_sample_clear_and_builds_the_proof() {
    let one = 65_536i64;
    let margin = |px: i64, tx: i64, ph: i64| {
        (0..px)
            .map(|m| {
                let f = (ph + (2 * m + 1) * tx * one / (2 * px)).rem_euclid(one);
                f.min(one - f)
            })
            .min()
            .unwrap_or(one)
    };
    // 184 pixels for 32 texels: 23 / 4, the best is 1/46 texel.
    let u = best_centre_phase(184, 32);
    assert!(
        (margin(184, 32, u) - one / 46).abs() <= 64,
        "{}",
        margin(184, 32, u)
    );
    // 160 pixels for 32: 5 / 1, the best is 1/10 texel.
    let v = best_centre_phase(160, 32);
    assert!((margin(160, 32, v) - one / 10).abs() <= 64);
    // 1:1 samples sit at texel centres with no phase; 1:2 needs half.
    assert_eq!(best_centre_phase(32, 32), 0);
    assert_eq!(margin(32, 32, 0), one / 2);
    assert_eq!(margin(16, 32, best_centre_phase(16, 32)), one / 2);
    let t = test_tex();
    for i in 0..CORNER_SCENES_EACH {
        let get = |layer, tex| -> std::vec::Vec<Draw> {
            let (_, s) = proof_scene(layer, tex, i, t);
            s.iter().flatten().copied().collect()
        };
        let (fl, tl, fb, tb) = (
            get(true, false),
            get(true, true),
            get(false, false),
            get(false, true),
        );
        assert_eq!(fl.len(), 2 + CORNER_RECTS);
        assert_eq!(fb.len(), 1 + CORNER_RECTS);
        assert_eq!(fl[0].paint, Paint::Flat(HIDDEN_LAYER));
        assert_eq!(fl[1..], fb[..], "the layer the only difference");
        assert_eq!(tl[1..], tb[..]);
        for (a, b) in fl[2..].iter().zip(&tl[2..]) {
            assert_eq!(a.corners, b.corners);
            assert_eq!(a.paint, Paint::Flat(UNTEXTURED));
            let Paint::Texture { uv, .. } = b.paint else {
                panic!("textured");
            };
            assert_eq!(uv[0], ((u * 2048 / one) as i32, (v * 2048 / one) as i32));
        }
        assert_eq!(PROOF_TEX_LAYER[i], Scene::Proof(true, true, i));
        assert_eq!(PROOF_FLAT_BARE[i], Scene::Proof(false, false, i));
        assert_eq!(
            (PROOF_CHECKER[i], PROOF_INTERLEAVED[i]),
            ((i, CHECKER), (i, INTERLEAVED))
        );
        assert_eq!(PROOF_COLUMNS[i], (i, COLUMNS));
    }
    assert_eq!(PROOF_FLAT_LAYER[0], Scene::Proof(true, false, 0));
    assert_eq!(PROOF_TEX_BARE[0], Scene::Proof(false, true, 0));
    const { assert!(PROOF_ROUNDS >= 100) };
}

/// Renders `scene` on `r` through the model and hashes the whole target.
#[allow(clippy::too_many_arguments)]
fn model_scene_hash(
    r: &mut Renderer,
    gpu: &mut Gpu,
    mem: &mut Mem,
    clock: &mut SimClock,
    at: u32,
    scene: Scene,
    tex: Tex,
) -> u32 {
    let (bg, slots) = order_scene_wide(scene, tex);
    r.set_background(Some(bg)).expect("background");
    for (s, d) in slots.iter().enumerate() {
        r.set(mem, s, d.as_ref()).expect("slot");
    }
    r.invalidate_all();
    let t = Target {
        picture: Picture::packed(at, W as u16, H as u16, KINDS[FT2].1),
        x: 0,
        y: 0,
    };
    let due = r
        .render(gpu, mem, clock, t)
        .expect("render")
        .expect("something to render");
    clock.set(due);
    loop {
        match r.poll(gpu, mem, clock) {
            Poll::Done(_) => break,
            Poll::Wait(t) => clock.set(t),
            p => panic!("{p:?}"),
        }
    }
    (0..words(FT2)).fold(HASH_START, |h, i| {
        fold(h, mem.load((at + 4 * i) as usize).expect("in memory"))
    })
}

/// GC2BC's set-up on the host: scene 20 rendered, the ring scene rendered
/// on the same renderer and on a second one sharing the manager's block,
/// then scene 20 again must be the same picture (on the X51v's emulator
/// check it wasn't: every render of scene 20 after the ring scene wrong:
/// the first renderer's shadow of the core's registers was stale, so it
/// drew the second's lists; `core_taken` at each switch is the fix).
#[test]
fn gc2bc_scene_20_survives_the_ring_scene_on_both_renderers() {
    let m = memory();
    let mut clock = SimClock::default();
    let mut pool = Pool::new(POOL);
    let mut mem = Mem::new(m.clone());
    let mut gpu = Gpu::new(m, &clock);
    let mut r = Renderer::new(&mut pool, W as u16, H as u16).expect("renderer");
    let ondie = rp_vram_map::Region {
        name: "on-die lists",
        at: INSIDE_AT + 2 * W * H,
        size: SAFE_END - (INSIDE_AT + 2 * W * H),
        align: 0x1000,
    };
    let mut r2 = Renderer::new_in(&mut pool, W as u16, H as u16, ondie, rp_vram_map::PM_TABLE)
        .expect("on-die renderer");
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
    let at = INSIDE_AT;
    let scene = CORNER_FLAT[20];
    let ring = TRACE_SCENES[0];
    let a = model_scene_hash(&mut r, &mut gpu, &mut mem, &mut clock, at, scene, tex);
    let ring_r = model_scene_hash(&mut r, &mut gpu, &mut mem, &mut clock, at, ring, tex);
    r2.core_taken();
    let ring_r2 = model_scene_hash(&mut r2, &mut gpu, &mut mem, &mut clock, at, ring, tex);
    assert_eq!(ring_r, ring_r2, "the on-die renderer draws the ring scene");
    r.core_taken();
    let b = model_scene_hash(&mut r, &mut gpu, &mut mem, &mut clock, at, scene, tex);
    assert_eq!(a, b, "scene 20 again after the ring scene");
    let c = model_scene_hash(&mut r, &mut gpu, &mut mem, &mut clock, at, scene, tex);
    assert_eq!(a, c, "scene 20 a third time");
}

/// The bisect of the test above: the ring scene on the same renderer only.
#[test]
fn gc2bc_bisect_ring_on_the_same_renderer_only() {
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
    let at = INSIDE_AT;
    let a = model_scene_hash(
        &mut r,
        &mut gpu,
        &mut mem,
        &mut clock,
        at,
        CORNER_FLAT[20],
        tex,
    );
    let _ = model_scene_hash(
        &mut r,
        &mut gpu,
        &mut mem,
        &mut clock,
        at,
        TRACE_SCENES[0],
        tex,
    );
    let b = model_scene_hash(
        &mut r,
        &mut gpu,
        &mut mem,
        &mut clock,
        at,
        CORNER_FLAT[20],
        tex,
    );
    assert_eq!(a, b, "scene 20 after the ring scene on one renderer");
}

/// The other half: a second renderer sharing the manager's block renders
/// in between, the first renderer's scene unchanged.
#[test]
fn gc2bc_bisect_second_renderer_in_between_only() {
    let m = memory();
    let mut clock = SimClock::default();
    let mut pool = Pool::new(POOL);
    let mut mem = Mem::new(m.clone());
    let mut gpu = Gpu::new(m, &clock);
    let mut r = Renderer::new(&mut pool, W as u16, H as u16).expect("renderer");
    let ondie = rp_vram_map::Region {
        name: "on-die lists",
        at: INSIDE_AT + 2 * W * H,
        size: SAFE_END - (INSIDE_AT + 2 * W * H),
        align: 0x1000,
    };
    let mut r2 = Renderer::new_in(&mut pool, W as u16, H as u16, ondie, rp_vram_map::PM_TABLE)
        .expect("on-die renderer");
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
    let at = INSIDE_AT;
    let a = model_scene_hash(
        &mut r,
        &mut gpu,
        &mut mem,
        &mut clock,
        at,
        CORNER_FLAT[20],
        tex,
    );
    let _ = model_scene_hash(
        &mut r2,
        &mut gpu,
        &mut mem,
        &mut clock,
        at,
        CORNER_FLAT[20],
        tex,
    );
    let b = model_scene_hash(
        &mut r,
        &mut gpu,
        &mut mem,
        &mut clock,
        at,
        CORNER_FLAT[20],
        tex,
    );
    assert_eq!(a, b, "scene 20 after the second renderer drew it");
}

/// Renders `(bg, slots)` on `r` through the model into a 480 x `h` target
/// at `at` and returns its words.
fn model_render_words(
    r: &mut Renderer,
    gpu: &mut Gpu,
    mem: &mut Mem,
    clock: &mut SimClock,
    at: u32,
    h: u16,
    (bg, slots): (u32, [Option<Draw>; SLOTS]),
) -> std::vec::Vec<u32> {
    r.set_background(Some(bg)).expect("background");
    for (s, d) in slots.iter().enumerate() {
        r.set(mem, s, d.as_ref()).expect("slot");
    }
    r.invalidate_all();
    let t = Target {
        picture: Picture::packed(at, W as u16, h, KINDS[FT2].1),
        x: 0,
        y: 0,
    };
    let due = r
        .render(gpu, mem, clock, t)
        .expect("render")
        .expect("something to render");
    clock.set(due);
    loop {
        match r.poll(gpu, mem, clock) {
            Poll::Done(_) => break,
            Poll::Wait(t) => clock.set(t),
            p => panic!("{p:?}"),
        }
    }
    (0..(W as usize * h as usize / 2))
        .map(|i| mem.load((at + 4 * i as u32) as usize).expect("in memory"))
        .collect()
}

/// GC2BD's strip: scene 20 shifted up 224 rows into a 480 x 96 target is
/// the full scene's rows 224-319 word for word (the shift keeps the
/// tile-relative geometry: 224 is 14 tile rows).
#[test]
fn gc2bd_strip_is_the_full_scenes_rows() {
    let m = memory();
    let mut clock = SimClock::default();
    let mut pool = Pool::new(POOL);
    let mut mem = Mem::new(m.clone());
    let mut gpu = Gpu::new(m, &clock);
    let mut r = Renderer::new(&mut pool, W as u16, H as u16).expect("renderer");
    let strip_params = rp_vram_map::Region {
        name: "strip lists",
        at: 0x0034_0000,
        size: 0x0010_0000,
        align: 0x1000,
    };
    let mut rs = Renderer::new_in(&mut pool, W as u16, 96, strip_params, rp_vram_map::PM_TABLE)
        .expect("strip renderer");
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
    let scene = order_scene_wide(CORNER_FLAT[20], tex);
    let full = model_render_words(
        &mut r, &mut gpu, &mut mem, &mut clock, INSIDE_AT, H as u16, scene,
    );
    let (bg, slots) = scene;
    rs.core_taken();
    let strip = model_render_words(
        &mut rs,
        &mut gpu,
        &mut mem,
        &mut clock,
        480 * 2 * 640,
        96,
        (bg, shifted(slots, -224)),
    );
    let row0 = 224 * 240;
    assert_eq!(strip.len(), 96 * 240);
    assert_eq!(
        &full[row0..row0 + 96 * 240],
        &strip[..],
        "the strip is rows 224-319"
    );
    // And not by accident: the rows above differ.
    assert_ne!(&full[0..96 * 240], &strip[..]);
}

/// GC2BE's two half-height renderers, one with its lists at 0x96000 on
/// the die (96 KiB) and one with them in SDRAM, alternating on one core
/// with `core_taken` at every switch, each drawing scene 20's rows 0-319
/// into B and A: every render the same picture (the emulator check drew
/// the on-die one wrong; the model says whether the renderer does).
#[test]
fn gc2be_half_renderers_alternate_on_one_core() {
    let m = memory();
    let mut clock = SimClock::default();
    let mut pool = Pool::new(POOL);
    let mut mem = Mem::new(m.clone());
    let mut gpu = Gpu::new(m, &clock);
    let sdram = rp_vram_map::Region {
        name: "half lists in SDRAM",
        at: 0x0034_0000,
        size: 0x0010_0000,
        align: 0x1000,
    };
    let ondie = rp_vram_map::Region {
        name: "half lists on the die",
        at: 0x0009_6000,
        size: SAFE_END - 0x0009_6000,
        align: 0x1000,
    };
    let mut rh = Renderer::new_in(&mut pool, W as u16, 320, sdram, rp_vram_map::PM_TABLE).unwrap();
    let mut rd = Renderer::new_in(&mut pool, W as u16, 320, ondie, rp_vram_map::PM_TABLE).unwrap();
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
    let scene = order_scene_wide(CORNER_FLAT[20], tex);
    let (a_at, b_at) = (0u32, 480 * 2 * 320);
    let want = model_render_words(&mut rh, &mut gpu, &mut mem, &mut clock, b_at, 320, scene);
    let mut last = false;
    for i in 0..8 {
        let on_die = i % 2 == 1;
        let r: &mut Renderer = if on_die { &mut rd } else { &mut rh };
        if on_die != last {
            r.core_taken();
            last = on_die;
        }
        let at = if i % 4 < 2 { b_at } else { a_at };
        let got = model_render_words(r, &mut gpu, &mut mem, &mut clock, at, 320, scene);
        assert_eq!(got, want, "render {i} (on the die: {on_die}) at 0x{at:x}");
    }
}

/// GC2BG's content set is every 16-bit kind (each fits the safe region),
/// its checker ways list every scene once in the checker order, and its
/// soak holds the kinds, the ring scene and the faulting corner scenes.
#[test]
fn gc2bg_proof_lists_are_the_16_bit_kinds_and_the_checker_order() {
    let mut seen = 0;
    for s in KINDS16 {
        let Scene::Kind(k) = s else { panic!("a kind") };
        assert_eq!(KINDS[k].1.bytes(), 2, "kind {k} is 16-bit");
        assert!(fits_inside(k), "kind {k} fits the safe region");
        seen += 1;
    }
    let all16 = (0..KINDS.len())
        .filter(|&k| KINDS[k].1.bytes() == 2)
        .count();
    assert_eq!(seen, all16, "every 16-bit kind");
    let w: [(usize, u8); 23] = checker_ways();
    for (i, &(s, o)) in w.iter().enumerate() {
        assert_eq!((s, o), (i, CHECKER));
    }
    assert!(PROOF_SOAK.contains(&Scene::Fix(0, 8)));
    assert!(PROOF_SOAK.contains(&Scene::Corner(false, 20)));
    assert!(PROOF_SOAK.len() <= ORDER_SCENES.len());
}

/// GC2BH's shifted kinds: the shift moves every clamped texture's
/// coordinates half a texel in u and v and changes nothing else; a
/// wrapping texture (kind 6) and an untextured kind stay as they are.
#[test]
fn gc2bh_kind_uv_shifts_clamped_textures_only() {
    let t = test_tex();
    for k in [FT2, 10, 5, 6, 0] {
        let (bg, raw) = order_scene(Scene::Kind(k), t);
        let (bg2, uv) = order_scene(Scene::KindUv(k), t);
        assert_eq!(bg, bg2);
        for (a, b) in raw.iter().zip(&uv) {
            match (a, b) {
                (Some(a), Some(b)) => {
                    assert_eq!(a.corners, b.corners, "kind {k}: the geometry the same");
                    assert_eq!(*b, shift_texture(*a, TexFix::ClampUv), "kind {k}");
                }
                (None, None) => {}
                _ => panic!("kind {k}: a slot appeared or went"),
            }
        }
    }
    let (_, ft2) = order_scene(Scene::Kind(FT2), t);
    let (_, ft2uv) = order_scene(Scene::KindUv(FT2), t);
    assert_ne!(ft2, ft2uv, "FT2's clamped texture is shifted");
    // The run's two lists are the same kinds, shifted and not, and every
    // one textured and 16-bit.
    for (a, b) in TEXEL_KINDS.iter().zip(&TEXEL_KINDS_UV) {
        let (Scene::Kind(k), Scene::KindUv(k2)) = (*a, *b) else {
            panic!("a kind and its shifted twin")
        };
        assert_eq!(k, k2);
        assert_eq!(KINDS[k].1.bytes(), 2, "kind {k} is 16-bit");
        let (_, slots) = order_scene(*a, t);
        assert!(
            slots
                .iter()
                .flatten()
                .any(|d| matches!(d.paint, Paint::Texture { .. })),
            "kind {k} is textured"
        );
    }
}
