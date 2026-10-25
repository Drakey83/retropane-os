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
