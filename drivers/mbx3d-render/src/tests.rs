//! The renderer against the model of the 3D core: every device result of
//! `rp-mbx3d` versions 19-23 reproduced through the API, and the bus cost
//! of each frame checked.

extern crate std;

use std::vec::Vec;

use rp_drv_blit2d::{MBX, reg};
use rp_drv_mbx_display::clock::MBXCLK;
use rp_vram_map::{POOL, Pool, Region};

use super::model::{Gpu, Mem, SimClock, memory};
use super::*;

const GREY: u32 = 0xFF20_2020;
const RED: u32 = 0xFFFF_0000;
const BLUE: u32 = 0xFF00_00FF;
const SCREEN: Picture = Picture::bench32(0);
const FULL: Target = Target {
    picture: SCREEN,
    x: 0,
    y: 0,
};

struct Rig {
    mem: Mem,
    gpu: Gpu,
    clock: SimClock,
    pool: Pool,
    r: Renderer,
}

fn rig(w: u16, h: u16) -> Rig {
    let m = memory();
    let clock = SimClock::default();
    let mut pool = Pool::new(POOL);
    let r = Renderer::new(&mut pool, w, h).expect("renderer");
    Rig {
        mem: Mem::new(m.clone()),
        gpu: Gpu::new(m, &clock),
        clock,
        pool,
        r,
    }
}

impl Rig {
    fn set(&mut self, slot: usize, d: Option<Draw>) {
        self.r.set(&mut self.mem, slot, d.as_ref()).expect("set");
    }

    /// Renders into `t` and polls at the times it asks for until done.
    fn frame_into(&mut self, t: Target) -> FrameStats {
        let due = self
            .r
            .render(&mut self.gpu, &mut self.mem, &mut self.clock, t)
            .expect("render")
            .expect("something to render");
        self.clock.set(due);
        loop {
            match self.r.poll(&mut self.gpu, &mut self.mem, &mut self.clock) {
                Poll::Done(s) => return s,
                Poll::Wait(at) => self.clock.set(at),
                p => panic!("{p:?}"),
            }
        }
    }

    fn frame(&mut self) -> FrameStats {
        self.frame_into(FULL)
    }

    fn px(&self, x: u32, y: u32) -> u32 {
        self.mem.px(SCREEN.at, SCREEN.stride, x, y)
    }
}

fn square(x: i32, y: i32, side: i32, paint: Paint) -> Draw {
    Draw::rect(x, y, side, side, paint)
}

#[test]
fn first_frame_draws_everything_then_nothing_until_a_change() {
    let mut g = rig(480, 640);
    g.r.set_background(Some(GREY)).unwrap();
    g.set(0, Some(square(100, 100, 200, Paint::Flat(BLUE))));
    let s = g.frame();
    assert_eq!(s.tiles, 2400);
    assert_eq!(s.lists, 2, "the background alone, and with the square");
    // Edges exact: [100, 300) both ways.
    for (x, y, want) in [
        (100, 100, BLUE),
        (299, 299, BLUE),
        (200, 150, BLUE),
        (99, 150, GREY),
        (300, 150, GREY),
        (150, 99, GREY),
        (150, 300, GREY),
        (0, 0, GREY),
        (479, 639, GREY),
    ] {
        assert_eq!(g.px(x, y), want, "({x}, {y})");
    }
    // Nothing changed: no render, not a word on the bus.
    let n = g.gpu.writes.len();
    let reads = g.gpu.status_reads;
    let r = g.r.render(&mut g.gpu, &mut g.mem, &mut g.clock, FULL);
    assert_eq!(r, Ok(None));
    // The same quad again changes nothing either.
    g.set(0, Some(square(100, 100, 200, Paint::Flat(BLUE))));
    let r = g.r.render(&mut g.gpu, &mut g.mem, &mut g.clock, FULL);
    assert_eq!(r, Ok(None));
    assert_eq!(g.gpu.writes.len(), n);
    assert_eq!(g.gpu.status_reads, reads);
}

#[test]
fn a_frame_after_the_first_costs_the_start_one_status_read_and_its_clear() {
    let mut g = rig(480, 640);
    g.r.set_background(Some(GREY)).unwrap();
    g.set(0, Some(square(100, 100, 200, Paint::Flat(BLUE))));
    g.frame();
    let n = g.gpu.writes.len();
    let reads = g.gpu.status_reads;
    g.mem.1 = 0;
    g.set(0, Some(square(108, 100, 200, Paint::Flat(BLUE))));
    let s = g.frame();
    assert_eq!(
        g.gpu.writes[n..],
        [(MBX + m3::START, 1), (MBX + reg::CLEAR, m3::RENDER_DONE)]
    );
    assert_eq!(s.reg_writes, 1);
    assert_eq!(
        g.gpu.status_reads - reads,
        1,
        "polled once, at the predicted time"
    );
    // Only the tiles the square left or reached: columns 12-38, rows 6-18.
    assert_eq!(s.tiles, 27 * 13);
    assert_eq!(g.gpu.tiles, s.tiles);
    // Words: the square's two objects, 2 a tile, and the one new list
    // (the square now reaches column 38 alone with the background: the
    // list for "background and square" exists already).
    assert_eq!(s.lists, 0, "both lists reused");
    assert_eq!(s.mem_words, 34 + 2 * s.tiles);
    assert_eq!(g.mem.1, s.mem_words, "the count is what reached memory");
    assert_eq!(g.px(107, 150), GREY);
    assert_eq!(g.px(108, 150), BLUE);
    assert_eq!(g.px(307, 150), BLUE);
    assert_eq!(g.px(308, 150), GREY);
    assert!(
        s.predicted_us == predict_us(s.tiles, s.quads * quad_cost(None, false))
            && s.done_us >= s.predicted_us
    );
    assert_eq!(
        s.quads,
        26 * 13,
        "the square in all its tiles but the column it left"
    );
}

#[test]
fn a_clean_start_resets_the_core_and_writes_every_register_again() {
    let mut g = rig(480, 640);
    g.r.set_background(Some(GREY)).unwrap();
    g.set(0, Some(square(100, 100, 200, Paint::Flat(BLUE))));
    let first = g.frame();
    g.r.invalidate_all();
    assert_eq!(
        g.frame().reg_writes,
        1,
        "only the start: the shadow holds the rest"
    );
    let resets = g.r.core_resets();
    let n = g.gpu.writes.len();
    g.r.start_clean();
    g.r.invalidate_all();
    let clean = g.frame();
    assert_eq!(g.r.core_resets(), resets + 1);
    assert!(
        g.gpu.writes[n..].contains(&(MBX + reg::RESET, 0x111)),
        "the core reset"
    );
    assert_eq!(clean.reg_writes, first.reg_writes, "every register again");
    assert_eq!(clean.tiles, 2400);
    assert_eq!(g.px(150, 150), BLUE);
    assert_eq!(g.px(50, 50), GREY);
    // Only the one render: the next is back to the shadow's diff.
    g.r.invalidate_all();
    assert_eq!(g.frame().reg_writes, 1);
}

#[test]
fn unlisted_tiles_are_left_untouched() {
    let mut g = rig(480, 640);
    g.r.set_background(Some(GREY)).unwrap();
    g.set(0, Some(square(100, 100, 200, Paint::Flat(BLUE))));
    g.frame();
    // Something else (the 2D engine) draws far away.
    g.mem
        .fill(SCREEN.at, SCREEN.stride, (400, 600, 8, 16), 0xFFFF_00FF);
    g.mem
        .fill(SCREEN.at, SCREEN.stride, (96, 400, 8, 16), 0xFFFF_00FF);
    g.set(0, Some(square(100, 116, 200, Paint::Flat(RED))));
    g.frame();
    assert_eq!(g.px(400, 600), 0xFFFF_00FF);
    assert_eq!(g.px(100, 408), 0xFFFF_00FF);
    assert_eq!(g.px(150, 300), RED);
    assert_eq!(g.px(150, 110), GREY);
    // Told about it, the renderer draws over it again.
    g.r.invalidate(96, 400, 8, 16);
    let s = g.frame();
    assert_eq!(s.tiles, 1);
    assert_eq!(g.px(100, 408), GREY);
    assert_eq!(g.px(400, 600), 0xFFFF_00FF);
}

#[test]
fn a_sub_rectangle_target_lands_in_place_with_the_stride_kept() {
    // Version 20's T3: a 4 x 4 tile block at (0,0), (0,100), (64,100),
    // (8,100) and (1,100): exact corners, nothing outside.
    for (x, y) in [(0u16, 0u16), (0, 100), (64, 100), (8, 100), (1, 100)] {
        let mut g = rig(32, 64);
        g.mem.fill(SCREEN.at, SCREEN.stride, (0, 0, 480, 640), GREY);
        g.r.set_background(Some(BLUE)).unwrap();
        let s = g.frame_into(Target {
            picture: SCREEN,
            x,
            y,
        });
        assert_eq!(s.tiles, 16);
        let (x, y) = (u32::from(x), u32::from(y));
        assert_eq!(g.px(x, y), BLUE);
        assert_eq!(g.px(x + 31, y + 63), BLUE);
        assert_eq!(g.px(x + 32, y), GREY);
        assert_eq!(g.px(x, y + 64), GREY);
        if x > 0 {
            assert_eq!(g.px(x - 1, y), GREY);
        }
        if (x, y) != (0, 0) {
            assert_eq!(g.px(0, 0), GREY, "the picture's origin untouched");
        }
        assert_eq!(g.gpu.reg(0x65C), SCREEN.at + y * SCREEN.stride + 4 * x);
        assert_eq!(g.gpu.reg(0x660), SCREEN.stride / 4);
    }
}

#[test]
fn targets_that_dont_fit_are_refused() {
    let mut g = rig(32, 64);
    let off = |x, y| Target {
        picture: SCREEN,
        x,
        y,
    };
    for t in [off(449, 0), off(0, 577)] {
        assert_eq!(
            g.r.render(&mut g.gpu, &mut g.mem, &mut g.clock, t),
            Err(Error::BadTarget)
        );
    }
    let mut pool = Pool::new(POOL);
    assert!(
        Renderer::new(&mut pool, 30, 64).is_none(),
        "not whole tiles"
    );
    assert!(Renderer::new(&mut pool, 32, 60).is_none());
    assert!(Renderer::new(&mut pool, 0, 64).is_none());
    assert!(
        Renderer::new(&mut pool, 1032, 16).is_none(),
        "128 columns at most"
    );
    assert!(Renderer::new(&mut pool, 480, 640).is_some());
    assert!(Renderer::new(&mut pool, 640, 480).is_some(), "landscape");
}

#[test]
fn double_buffered_pictures_each_get_the_changes_they_missed() {
    let mut g = rig(480, 640);
    let pic = [Picture::bench32(0), Picture::bench32(1)];
    let into = |n: usize| Target {
        picture: pic[n],
        x: 0,
        y: 0,
    };
    let px = |g: &Rig, n: usize, x, y| g.mem.px(pic[n].at, pic[n].stride, x, y);
    g.r.set_background(Some(GREY)).unwrap();
    assert_eq!(g.frame_into(into(0)).tiles, 2400);
    assert_eq!(g.frame_into(into(1)).tiles, 2400, "never drawn: all of it");
    g.set(0, Some(square(0, 0, 8, Paint::Flat(RED))));
    assert_eq!(g.frame_into(into(0)).tiles, 1);
    g.set(1, Some(square(8, 0, 8, Paint::Flat(BLUE))));
    assert_eq!(g.frame_into(into(0)).tiles, 1, "the same picture again");
    // Picture 1 missed both.
    assert_eq!(g.frame_into(into(1)).tiles, 2);
    assert_eq!((px(&g, 1, 0, 0), px(&g, 1, 8, 0)), (RED, BLUE));
    g.set(0, None);
    assert_eq!(g.frame_into(into(0)).tiles, 1);
    assert_eq!(px(&g, 0, 0, 0), GREY);
    assert_eq!(g.frame_into(into(1)).tiles, 1, "the removal it missed");
    assert_eq!(px(&g, 1, 0, 0), GREY);
    assert_eq!(px(&g, 1, 8, 0), BLUE);
}

#[test]
fn gouraud_corners_and_middle() {
    let mut g = rig(480, 640);
    let c = [0xFFFF_0000, 0xFF00_FF00, 0xFF00_00FF, 0xFFFF_FFFF];
    g.set(0, Some(square(100, 100, 256, Paint::Gouraud(c))));
    g.frame();
    let near = |a: u32, b: u32| {
        (0..4).all(|i| ((a >> (8 * i)) & 0xFF).abs_diff((b >> (8 * i)) & 0xFF) <= 4)
    };
    assert!(near(g.px(100, 100), c[0]), "{:08x}", g.px(100, 100));
    assert!(near(g.px(355, 100), c[1]), "{:08x}", g.px(355, 100));
    assert!(near(g.px(355, 355), c[2]), "{:08x}", g.px(355, 355));
    assert!(near(g.px(100, 355), c[3]), "{:08x}", g.px(100, 355));
    // Flat is the first vertex's colour.
    g.set(0, Some(square(100, 100, 256, Paint::Flat(c[1]))));
    g.frame();
    assert_eq!(g.px(300, 300), c[1]);
}

#[test]
fn reversed_tiles_draw_the_same_picture_and_list_every_tile() {
    // Gouraud under a textured quad whose corner sits mid-tile (run GC2K's
    // scene, smaller): rendered whole in row order, then reversed.
    let mut g = rig(480, 640);
    let tex = columns(&mut g);
    let c = [0xFF00_0000, 0xFFFF_FFFF, 0xFF10_20A0, 0xFFFF_E000];
    g.set(0, Some(Draw::rect(0, 0, 480, 640, Paint::Gouraud(c))));
    g.set(1, Some(Draw::rect(43, 45, 400, 560, Paint::texture(tex))));
    let fwd = g.frame();
    let words = |g: &Rig| -> Vec<u32> {
        (0..480 * 640)
            .map(|i| g.mem.px(SCREEN.at, SCREEN.stride, i % 480, i / 480))
            .collect()
    };
    let a = words(&g);
    g.r.set_tiles_reversed(true);
    g.r.invalidate_all();
    let rev = g.frame();
    assert_eq!(words(&g), a);
    assert_eq!(rev.tiles, fwd.tiles);
    assert_eq!(rev.tiles, 60 * 40);
    // Columns reversed (rows still from the top): the same again.
    g.r.set_tile_order(TileOrder::ColumnsReversed);
    g.r.invalidate_all();
    let cols = g.frame();
    assert_eq!(words(&g), a);
    assert_eq!(cols.tiles, 60 * 40);
    // Back to row order: the same again.
    g.r.set_tiles_reversed(false);
    g.r.invalidate_all();
    g.frame();
    assert_eq!(words(&g), a);
}

/// An 8 x 8 RGB565 texture whose column `x` is colour `x`.
fn columns(g: &mut Rig) -> Texture {
    let texels: Vec<u16> = (0..64).map(|i| ((i % 8) as u16 + 1) * 0x0841).collect();
    upload(&mut g.pool, &mut g.mem, 0, format::RGB565, &texels).expect("upload")
}

fn column_colour(x: u32) -> u32 {
    let t = (x + 1) * 0x0841;
    let (r, gg, b) = (t >> 11, (t >> 5) & 63, t & 31);
    0xFF00_0000 | (r << 3 | r >> 2) << 16 | (gg << 2 | gg >> 4) << 8 | (b << 3 | b >> 2)
}

#[test]
fn textured_quads_sample_twiddled_texels_exactly() {
    // Version 21's checker: 8 x 8 texels over 64 x 64 pixels, texel
    // boundaries every 8 pixels.
    let mut g = rig(480, 640);
    let mut texels = [0u16; 64];
    for (i, t) in texels.iter_mut().enumerate() {
        let (x, y) = (i % 8, i / 8);
        *t = if (x + y) % 2 == 0 { 0xF800 } else { 0x001F };
    }
    texels[7] = 0x07E0; // texel (7, 0) green
    let tex = upload(&mut g.pool, &mut g.mem, 0, format::RGB565, &texels).unwrap();
    assert_eq!(tex.at % 128, 0);
    g.set(0, Some(square(100, 100, 64, Paint::texture(tex))));
    g.frame();
    assert_eq!(g.px(100, 100), RED);
    assert_eq!(g.px(107, 107), RED);
    assert_eq!(g.px(108, 100), BLUE);
    assert_eq!(g.px(100, 108), BLUE);
    assert_eq!(g.px(156, 100), 0xFF00_FF00);
    assert_eq!(g.px(163, 107), 0xFF00_FF00);
    assert_eq!(g.px(163, 108), RED, "texel (7, 1)");
    assert_eq!(g.px(164, 100), 0xFF00_0000, "the background past it");
}

#[test]
fn address_modes_wrap_mirror_and_clamp() {
    // u from 0 to 2 over 128 pixels: 8 pixels a texel. Version 22: past 1,
    // wrap repeats, mirror reflects, clamp holds the edge texel.
    for (mode, after_edge, near_end) in [
        (address::WRAP, 0, 7),
        (address::MIRROR, 7, 0),
        (address::CLAMP, 7, 7),
    ] {
        let mut g = rig(480, 640);
        let tex = columns(&mut g).with_modes(mode, address::CLAMP);
        let uv = [(0, 0), (2 * UV_ONE, 0), (2 * UV_ONE, UV_ONE), (0, UV_ONE)];
        g.set(
            0,
            Some(Draw::rect(
                0,
                0,
                128,
                64,
                Paint::Texture {
                    tex,
                    uv,
                    tint: 0xFFFF_FFFF,
                },
            )),
        );
        g.frame();
        assert_eq!(g.px(4, 10), column_colour(0));
        assert_eq!(g.px(60, 10), column_colour(7));
        assert_eq!(g.px(68, 10), column_colour(after_edge), "mode {mode}");
        assert_eq!(g.px(124, 10), column_colour(near_end), "mode {mode}");
    }
}

#[test]
fn alpha_blends_as_measured() {
    // Version 23: a flat square at alpha 0x80 over red, source alpha /
    // inverse: 0xBF7F007F; an ARGB4444 texture at alpha 8/15: 0xBF770087.
    let mut g = rig(480, 640);
    g.r.set_background(Some(RED)).unwrap();
    g.set(
        0,
        Some(square(100, 100, 200, Paint::Flat(0x8000_00FF)).over()),
    );
    let tex = upload(&mut g.pool, &mut g.mem, 0, format::ARGB4444, &[0x800F; 64]).unwrap();
    g.set(1, Some(square(320, 100, 64, Paint::texture(tex)).over()));
    g.frame();
    assert_eq!(g.px(200, 200), 0xBF7F_007F);
    assert_eq!(g.px(350, 130), 0xBF77_0087);
    // Additive (one, one): red + blue (version 21).
    g.set(
        0,
        Some(Draw {
            blend: Some((factor::ONE, factor::ONE)),
            ..square(100, 100, 200, Paint::Flat(BLUE))
        }),
    );
    g.frame();
    assert_eq!(g.px(200, 200), 0xFFFF_00FF);
}

#[test]
fn depth_larger_z_is_nearer() {
    // Version 23's T7b and rp-mbx3d-api run 2, through the API: red at 0.5
    // writing depth, blue at 0.3 or 0.8 with "greater".
    let z = |tenths: u32| (tenths * 65536 / 10) as u16;
    let red = |z0| {
        square(100, 100, 200, Paint::Flat(RED)).with_depth(Depth {
            mode: depth::ALWAYS,
            write: true,
            z: z0,
        })
    };
    let blue = |mode, z0| {
        square(200, 200, 200, Paint::Flat(BLUE)).with_depth(Depth {
            mode,
            write: true,
            z: z0,
        })
    };
    for (mode, zb, overlap, alone) in [
        (depth::GREATER, 8, BLUE, BLUE),
        (depth::GREATER, 3, RED, BLUE),
        (depth::GEQUAL, 3, RED, BLUE),
    ] {
        let mut g = rig(480, 640);
        g.r.set_background(Some(GREY)).unwrap();
        g.set(0, Some(red(z(5))));
        g.set(1, Some(blue(mode, z(zb))));
        g.frame();
        assert_eq!(g.px(250, 250), overlap, "mode {mode} z 0.{zb}");
        assert_eq!(g.px(350, 350), alone, "mode {mode} z 0.{zb}");
        assert_eq!(g.px(150, 150), RED);
    }
}

#[test]
fn depth_needs_something_to_write_it_first() {
    // Run 1 on the device: with the background not writing depth, "greater"
    // failed wherever only the background was under it (a tile's depth
    // isn't cleared). The background writes depth now; without one, a
    // quad must.
    let mut g = rig(480, 640);
    g.r.set_background(None).unwrap();
    let d = |mode, write, z| Depth { mode, write, z };
    let under = |write| square(0, 0, 480, Paint::Flat(GREY)).with_depth(d(depth::ALWAYS, write, 0));
    let blue = square(100, 100, 100, Paint::Flat(BLUE)).with_depth(d(depth::GREATER, true, 40_000));
    g.set(0, Some(under(false)));
    g.set(1, Some(blue));
    g.frame();
    assert_eq!(g.px(150, 150), GREY, "nothing wrote depth: greater fails");
    g.set(0, Some(under(true)));
    g.frame();
    assert_eq!(g.px(150, 150), BLUE);
}

#[test]
fn no_quad_sets_a_tsp_bit_that_hangs_the_core() {
    // TSP bits 16 and 19 hung the device's core (versions 21-22): no
    // combination of paint, blend and depth may set them.
    let tex = Texture {
        at: 0x0094_0000,
        code: 0,
        format: format::ARGB4444 as u8,
        u_mode: 0,
        v_mode: 0,
        linear: false,
        levels: 1,
    };
    let paints = [
        Paint::Flat(RED),
        Paint::Gouraud([RED; 4]),
        Paint::texture(tex),
    ];
    let mut seen = 0;
    for paint in paints {
        for blend in core::iter::once(None).chain((0..64).map(|k| Some((k >> 3, k & 7)))) {
            for d in core::iter::once(None).chain((0..16).map(|k| {
                Some(Depth {
                    mode: k & 7,
                    write: k >= 8,
                    z: 1,
                })
            })) {
                let (w, n, kind) = encode(&Draw {
                    blend,
                    depth: d,
                    ..square(0, 0, 8, paint)
                });
                let (tsp_at, words) = match kind {
                    Kind::Textured => (4, tex::WORDS),
                    _ => (1, m3::TRIANGLE_WORDS),
                };
                for t in [tsp_at, words + tsp_at] {
                    assert_eq!(w[t] & (1 << 16 | 1 << 19), 0, "{:08x}", w[t]);
                }
                assert_eq!(n, 2 * words);
                seen += 1;
            }
        }
    }
    assert_eq!(seen, 3 * 65 * 17);
}

#[test]
fn a_hung_render_is_recovered_and_the_next_frame_redraws_everything() {
    let mut g = rig(480, 640);
    g.r.set_background(Some(GREY)).unwrap();
    g.frame();
    g.set(0, Some(square(100, 100, 8, Paint::Flat(BLUE))));
    g.gpu.hang_next = true;
    let due =
        g.r.render(&mut g.gpu, &mut g.mem, &mut g.clock, FULL)
            .unwrap()
            .unwrap();
    g.clock.set(due);
    let mut recovered = false;
    for _ in 0..5000 {
        match g.r.poll(&mut g.gpu, &mut g.mem, &mut g.clock) {
            Poll::Wait(t) => g.clock.set(t),
            Poll::Recovered => {
                recovered = true;
                break;
            }
            p => panic!("{p:?}"),
        }
    }
    assert!(recovered);
    assert!(!g.gpu.hung(), "reset");
    assert_eq!(g.r.core_resets(), 2, "set-up, then the recovery");
    assert!(!g.r.busy());
    let s = g.frame();
    assert_eq!(s.tiles, 2400);
    assert_eq!(g.px(104, 104), BLUE);
}

#[test]
fn the_3d_clock_is_on_for_work_and_off_when_idle() {
    let mut g = rig(480, 640);
    assert_eq!(g.gpu.read32(MBXCLK), 0x5);
    g.frame();
    assert_eq!(g.gpu.read32(MBXCLK), 0x6, "on for the render");
    let now = g.clock.0.get();
    assert!(!g.r.idle(&mut g.gpu, now + 1_000));
    let due = g.r.idle_due().unwrap();
    assert!(g.r.idle(&mut g.gpu, due));
    assert_eq!(g.gpu.read32(MBXCLK), 0x5, "off after 100 ms idle");
    // Back on and drawing, nothing set up again (T9: the core keeps its
    // state across the clock off).
    let inits = |g: &Rig| g.gpu.writes.iter().filter(|w| w.0 == MBX + 0x838).count();
    let before = inits(&g);
    g.clock.set(due);
    g.set(0, Some(square(0, 0, 8, Paint::Flat(BLUE))));
    let s = g.frame();
    assert_eq!(inits(&g), before);
    assert_eq!(s.reg_writes, 1, "the start alone");
    assert_eq!(g.px(0, 0), BLUE);
}

#[test]
fn changes_wait_while_a_render_is_in_flight() {
    let mut g = rig(480, 640);
    g.r.render(&mut g.gpu, &mut g.mem, &mut g.clock, FULL)
        .unwrap()
        .unwrap();
    let d = square(0, 0, 8, Paint::Flat(BLUE));
    assert_eq!(g.r.set(&mut g.mem, 0, Some(&d)), Err(Error::Busy));
    assert_eq!(g.r.set_background(None), Err(Error::Busy));
    assert_eq!(
        g.r.render(&mut g.gpu, &mut g.mem, &mut g.clock, FULL),
        Err(Error::Busy)
    );
    assert_eq!(
        g.r.set(&mut g.mem, MAX_SLOTS, Some(&d)),
        Err(Error::Busy),
        "busy first"
    );
}

#[test]
fn slots_draw_in_order_and_out_of_range_is_refused() {
    let mut g = rig(480, 640);
    let d = square(0, 0, 8, Paint::Flat(BLUE));
    assert_eq!(
        g.r.set(&mut g.mem, MAX_SLOTS, Some(&d)),
        Err(Error::BadSlot)
    );
    // All 64 slots: a diagonal of small squares, then one over them all.
    for s in 0..MAX_SLOTS {
        let k = s as i32;
        g.set(
            s,
            Some(square(
                7 * k,
                9 * k,
                6,
                Paint::Flat(0xFF00_0000 | (s as u32 + 1)),
            )),
        );
    }
    g.frame();
    for s in 0..MAX_SLOTS as u32 {
        assert_eq!(
            g.px(7 * s + 2, 9 * s + 2),
            0xFF00_0000 | (s + 1),
            "slot {s}"
        );
    }
    g.set(63, Some(square(0, 0, 480, Paint::Flat(RED))));
    g.frame();
    assert_eq!(g.px(2, 2), RED, "the highest slot is on top");
}

#[test]
fn without_a_background_uncovered_pixels_come_out_white() {
    // The core writes whole tiles from its own buffer: the scene must
    // cover what it renders (measured: uncovered pixels white).
    let mut g = rig(480, 640);
    g.r.set_background(None).unwrap();
    g.set(0, Some(square(0, 0, 4, Paint::Flat(BLUE))));
    g.frame();
    assert_eq!(g.px(1, 1), BLUE);
    assert_eq!(g.px(6, 1), 0xFFFF_FFFF);
}

#[test]
fn lists_are_reclaimed_between_frames_and_too_much_is_refused() {
    // The smallest parameter memory the renderer accepts: lists for one
    // frame of a few quads at a time.
    let small = Region {
        name: "small",
        at: rp_vram_map::PARAMS.at,
        size: ARENA_AT + 4 * LIST_MAX as u32,
        align: 0x1000,
    };
    let m = memory();
    let mut clock = SimClock::default();
    let mut pool = Pool::new(POOL);
    let mut r = Renderer::new_in(&mut pool, 480, 640, small, rp_vram_map::PM_TABLE).unwrap();
    let mut mem = Mem::new(m.clone());
    let mut gpu = Gpu::new(m, &clock);
    let frame = |r: &mut Renderer, mem: &mut Mem, gpu: &mut Gpu, clock: &mut SimClock| {
        let due = r.render(gpu, mem, clock, FULL)?.unwrap();
        clock.set(due);
        loop {
            match r.poll(gpu, mem, clock) {
                Poll::Done(_) => return Ok(()),
                Poll::Wait(t) => clock.set(t),
                p => panic!("{p:?}"),
            }
        }
    };
    // A square moving, a flat one turning textured and back: lists keep
    // being written, and are reclaimed.
    let tex = upload(&mut pool, &mut mem, 0, format::RGB565, &[0xF800; 64]).unwrap();
    for k in 0..40 {
        let paint = if k % 2 == 0 {
            Paint::Flat(BLUE)
        } else {
            Paint::texture(tex)
        };
        r.set(&mut mem, 0, Some(&square(8 * k, 16 * k, 8, paint)))
            .unwrap();
        r.set(
            &mut mem,
            1,
            Some(&square(400 - 8 * k, 16 * k, 8, Paint::Flat(RED))),
        )
        .unwrap();
        frame(&mut r, &mut mem, &mut gpu, &mut clock).expect("fits");
        let px = |x: i32, y: i32| mem.px(SCREEN.at, SCREEN.stride, x as u32, y as u32);
        assert_eq!(
            px(8 * k + 1, 16 * k + 1),
            if k % 2 == 0 { BLUE } else { RED }
        );
        assert_eq!(px(400 - 8 * k + 1, 16 * k + 1), RED);
    }
    // 64 squares in tiles of their own: 65 lists don't fit.
    for s in 0..MAX_SLOTS {
        let k = s as i32;
        r.set(
            &mut mem,
            s,
            Some(&square(8 * k % 480, 16 * (k / 8), 8, Paint::Flat(BLUE))),
        )
        .unwrap();
    }
    assert_eq!(
        frame(&mut r, &mut mem, &mut gpu, &mut clock),
        Err(Error::TooComplex)
    );
}

#[test]
fn textures_come_from_the_pool_and_go_back() {
    let mut g = rig(480, 640);
    let free = g.pool.free_bytes();
    assert!(upload(&mut g.pool, &mut g.mem, 0, format::RGB565, &[0; 63]).is_none());
    assert!(
        upload(&mut g.pool, &mut g.mem, 0, 1, &[0; 64]).is_none(),
        "undecoded format"
    );
    assert!(upload(&mut g.pool, &mut g.mem, 8, format::RGB565, &[0; 64]).is_none());
    let t = upload(&mut g.pool, &mut g.mem, 1, format::ARGB1555, &[0; 256]).unwrap();
    assert_eq!(t.bytes(), 512);
    assert!(t.has_alpha());
    assert_eq!(t.words()[2], TEX2_ALPHA);
    assert_eq!(
        t.with_modes(address::WRAP, address::MIRROR).words()[0] >> 28,
        0b0001
    );
    assert_eq!(g.pool.free_bytes(), free - 512);
    assert!(release(&mut g.pool, t));
    assert_eq!(g.pool.free_bytes(), free);
    let regions = 2400 * 8;
    let r = core::mem::replace(&mut g.r, Renderer::new(&mut g.pool, 8, 16).unwrap());
    let before = g.pool.free_bytes();
    assert!(r.release(&mut g.pool));
    assert_eq!(g.pool.free_bytes(), before + regions);
}

#[test]
fn untwiddle_inverts_twiddle() {
    for i in 0..4096 {
        let (x, y) = untwiddle(i);
        assert_eq!(tex::twiddle(x, y), i);
    }
}

#[test]
fn the_set_up_is_the_proven_sequence() {
    // Clock on, reset, the memory manager's six pairs (`0x838` last), then
    // Dell's fourteen pairs, the target, the start (versions 19-23).
    let mut g = rig(480, 640);
    g.frame();
    let w: Vec<usize> = g
        .gpu
        .writes
        .iter()
        .map(|w| w.0)
        .filter(|&o| {
            o == MBXCLK || (MBX + 0x600..MBX + 0x900).contains(&o) || o == MBX + reg::RESET
        })
        .collect();
    let mbx = |o: &[usize]| o.iter().map(|o| MBX + o).collect::<Vec<_>>();
    let mut want = std::vec![MBXCLK, MBX + reg::RESET, MBX + reg::RESET];
    want.extend(mbx(&[0x824, 0x828, 0x82C, 0x83C, 0x6D8, 0x838]));
    want.extend(mbx(&[
        0x608, 0x60C, 0x610, 0x644, 0x650, 0x654, 0x658, 0x660, 0x6D8, 0x61C, 0x614, 0x618, 0x648,
        0x6DC, 0x65C, 0x680,
    ]));
    assert_eq!(w, want);
}

#[test]
fn the_prediction_fits_run_2s_exact_times() {
    // rp-mbx3d-api run 2 (local/rp-mbx3d-api-run2.txt): tiles, flat,
    // textured and blended quads summed over the tiles, the exact time.
    let flat = quad_cost(None, false);
    // Run 2's textured quads: 8 x 8 textures shown 8x (0.125 texels a
    // pixel).
    let textured = quad_cost(Some(2), false);
    let blended = quad_cost(None, true);
    for (tiles, cost, exact) in [
        (1, flat, 15),
        (16, 0, 93),
        (2400, 0, 12_521),
        (100, 100 * flat, 559),
        (600, 600 * flat, 3_323),
        (325, 416 * flat, 1_888),
        (2400, 512 * flat, 12_683),
        (32, 64 * flat, 218),
        (2400, 285 * textured, 12_813),
        (338, 338 * blended, 3_387),
        (2400, 378 * blended, 14_342),
    ] {
        let p = predict_us(tiles, cost);
        // 6%, or 25 us on the smallest frames (the start's own spread).
        assert!(
            p.abs_diff(exact) * 100 <= exact * 6 || p.abs_diff(exact) <= 25,
            "{tiles} tiles: {p} vs {exact}"
        );
    }
}

#[test]
fn the_prediction_fits_the_gates_limits_run() {
    // rp-gpubench run LIM (X51v, 2026-10-24, local/rp-gpubench-runLIM.txt):
    // a 16-bit target at Windows Mobile's clock, 2400 tiles, the median of
    // 3 renders (quality audit C20).
    let q = quad_cost;
    for (what, cost, exact) in [
        ("63 flat 64 x 64 quads", 2016 * q(None, false), 12_519),
        (
            "63 textured 64 x 64 quads, 1:1",
            2016 * q(Some(16), false),
            23_679,
        ),
        ("8 translucent layers", 8 * 2400 * q(None, true), 111_917),
        ("32 translucent layers", 32 * 2400 * q(None, true), 410_218),
        (
            "a 64 texture over the screen",
            2400 * q(Some(2), false),
            15_637,
        ),
        (
            "a 128 texture over the screen",
            2400 * q(Some(4), false),
            17_531,
        ),
        (
            "a 256 texture over the screen",
            2400 * q(Some(8), false),
            20_343,
        ),
        (
            "a 512 texture over the screen",
            2400 * q(Some(17), false),
            25_098,
        ),
        (
            "a 1024 texture over the screen",
            2400 * q(Some(34), false),
            30_203,
        ),
    ] {
        let p = predict_us(2400, cost);
        assert!(
            p.abs_diff(exact) * 100 <= exact * 6,
            "{what}: {p} vs {exact}"
        );
    }
}

#[test]
fn texels_a_pixel_follow_the_quads_size_and_its_textures_span() {
    let t = Texture {
        at: 0,
        code: 4, // 128 texels a side
        format: format::RGB565 as u8,
        u_mode: address::CLAMP as u8,
        v_mode: address::CLAMP as u8,
        linear: false,
        levels: 1,
    };
    let full = [(0, 0), (UV_ONE, 0), (UV_ONE, UV_ONE), (0, UV_ONE)];
    let at = |w, h, uv| {
        texels16(&Draw::rect(
            0,
            0,
            w,
            h,
            Paint::Texture {
                tex: t,
                uv,
                tint: 0xFFFF_FFFF,
            },
        ))
    };
    assert_eq!(at(128, 128, full), Some(16), "1:1");
    assert_eq!(at(64, 64, full), Some(32), "shrunk 2:1");
    assert_eq!(at(200, 260, full), Some(10), "W5's cards: 0.64");
    assert_eq!(at(128, 64, full), Some(32), "the more-sampled axis");
    let twice = [(0, 0), (2 * UV_ONE, 0), (2 * UV_ONE, UV_ONE), (0, UV_ONE)];
    assert_eq!(at(128, 128, twice), Some(32), "wrapped twice across");
    assert_eq!(at(0, 128, full), None, "no size");
    assert_eq!(texels16(&Draw::rect(0, 0, 8, 8, Paint::Flat(0))), None);
    // The table's edges: each band's first value, and the far end.
    for (r, want) in [
        (0, 5),
        (2, 5),
        (3, 8),
        (6, 13),
        (12, 21),
        (24, 30),
        (48, 38),
        (u32::MAX, 38),
    ] {
        assert_eq!(quad_cost(Some(r), false), want, "{r}");
        assert_eq!(quad_cost(Some(r), true), want + 13, "{r} translucent");
    }
    assert_eq!(quad_cost(None, false), 1);
    assert_eq!(quad_cost(None, true), 21);
}

#[test]
fn every_depth_and_alpha_mode_is_accepted_and_only_those() {
    // rp-glesfeat (2026-10-18): all eight behave; version 23's "less"
    // failure was the depth mode written into the alpha-test bits.
    let mut g = rig(480, 640);
    for mode in 0..10 {
        let want = if mode <= depth::ALWAYS {
            Ok(())
        } else {
            Err(Error::Unsupported)
        };
        let d = square(0, 0, 8, Paint::Flat(BLUE)).with_depth(Depth {
            mode,
            write: true,
            z: 1,
        });
        assert_eq!(g.r.set(&mut g.mem, 0, Some(&d)), want, "depth {mode}");
        let a = square(0, 0, 8, Paint::Flat(BLUE)).with_alpha_test(AlphaTest {
            func: mode,
            reference: 1,
        });
        assert_eq!(g.r.set(&mut g.mem, 0, Some(&a)), want, "alpha {mode}");
    }
}

#[test]
fn the_depth_mode_never_reaches_the_alpha_bits() {
    // The class of 2026-10-15's mistake: the second word carries only the
    // alpha test, whatever the depth test.
    for mode in 0..8 {
        for paint in [Paint::Flat(RED), Paint::Gouraud([RED; 4])] {
            let (w, _, _) = encode(&square(0, 0, 8, paint).with_depth(Depth {
                mode,
                write: true,
                z: 1,
            }));
            assert_eq!(w[0], m3::ISP_ALWAYS, "mode {mode}");
        }
    }
    let (w, _, _) = encode(
        &square(0, 0, 8, Paint::Flat(RED)).with_alpha_test(AlphaTest {
            func: depth::GREATER,
            reference: 128,
        }),
    );
    assert_eq!(w[0], 0x9000_0000);
}

#[test]
fn depth_less_passes_the_smaller_z() {
    // rp-glesfeat D on the device: red at 0.5 writing, the background at 0
    // writing; blue with "less" over red where its z is smaller, never
    // over the background.
    let z = |tenths: u32| (tenths * 65536 / 10) as u16;
    let red = square(100, 100, 200, Paint::Flat(RED)).with_depth(Depth {
        mode: depth::ALWAYS,
        write: true,
        z: z(5),
    });
    for (mode, zb, overlap, alone) in [
        (depth::LESS, 3, BLUE, GREY),
        (depth::LEQUAL, 3, BLUE, GREY),
        (depth::LESS, 8, RED, GREY),
        (depth::GREATER, 8, BLUE, BLUE),
    ] {
        let mut g = rig(480, 640);
        g.r.set_background(Some(GREY)).unwrap();
        g.set(0, Some(red));
        g.set(
            1,
            Some(square(200, 200, 200, Paint::Flat(BLUE)).with_depth(Depth {
                mode,
                write: true,
                z: z(zb),
            })),
        );
        g.frame();
        assert_eq!(g.px(250, 250), overlap, "mode {mode} z 0.{zb}");
        assert_eq!(g.px(350, 350), alone, "mode {mode} z 0.{zb}");
    }
}

#[test]
fn the_alpha_test_cuts_out_pixels() {
    // rp-glesfeat A on the device: alpha 64 over grey.
    for (func, reference, drawn) in [
        (depth::ALWAYS, 0, true),
        (depth::GREATER, 128, false),
        (depth::GREATER, 32, true),
        (depth::LESS, 128, true),
        (depth::LESS, 0, false),
    ] {
        let mut g = rig(480, 640);
        g.r.set_background(Some(GREY)).unwrap();
        g.set(
            0,
            Some(
                square(100, 100, 200, Paint::Flat(0x40FF_0000))
                    .with_alpha_test(AlphaTest { func, reference }),
            ),
        );
        g.frame();
        let want = if drawn { 0x40FF_0000 } else { GREY };
        assert_eq!(g.px(200, 200), want, "func {func} ref {reference}");
    }
}

#[test]
fn clock_policies_redo_what_they_say() {
    // T9's three answers: after the clock is off and on, a full set-up, the
    // registers again, or nothing. The model keeps its state across the
    // clock (whether the device does is T9's question).
    for (policy, inits, starts_only) in [
        (ClockPolicy::SetUpAgain, 1, false),
        (ClockPolicy::RewriteRegisters, 0, false),
        (ClockPolicy::KeepAll, 0, true),
    ] {
        let mut g = rig(480, 640);
        g.r.set_clock_policy(policy);
        g.frame();
        let due = g.r.idle_due().unwrap();
        assert!(g.r.idle(&mut g.gpu, due));
        g.clock.set(due);
        let n = g.gpu.writes.len();
        let count = |g: &Rig, off| {
            g.gpu.writes[n..]
                .iter()
                .filter(|w| w.0 == MBX + off)
                .count()
        };
        g.set(0, Some(square(0, 0, 8, Paint::Flat(BLUE))));
        let s = g.frame();
        assert_eq!(count(&g, 0x838), inits, "{policy:?}");
        assert_eq!(
            s.reg_writes == 1,
            starts_only,
            "{policy:?}: {}",
            s.reg_writes
        );
        assert_eq!(g.px(0, 0), BLUE, "{policy:?}");
        assert_eq!(g.gpu.read32(MBXCLK), 0x6);
    }
}

/// The checker of version 21/24: 8 x 8 red/white, texel (x, y) red when
/// x + y is even.
fn checker(g: &mut Rig) -> Texture {
    let texels: Vec<u16> = (0..64)
        .map(|i| {
            if (i % 8 + i / 8) % 2 == 0 {
                0xF800
            } else {
                0xFFFF
            }
        })
        .collect();
    upload(&mut g.pool, &mut g.mem, 0, format::RGB565, &texels).unwrap()
}

#[test]
fn bilinear_filtering_as_measured() {
    // rp-mbx3d run 24, T12: the checker over 64 px at (100,100), y 104,
    // x 100-114 step 2: point steps at 108, linear ramps.
    for (linear, want) in [
        (
            false,
            [
                0xFFFF_0000u32,
                0xFFFF_0000,
                0xFFFF_0000,
                0xFFFF_0000,
                0xFFFF_FFFF,
                0xFFFF_FFFF,
                0xFFFF_FFFF,
                0xFFFF_FFFF,
            ],
        ),
        (
            true,
            [
                0xFFFF_0000,
                0xFFFF_0000,
                0xFFFF_0000,
                0xFFFF_3F3F,
                0xFFFF_7F7F,
                0xFFFF_BFBF,
                0xFFFF_FFFF,
                0xFFFF_BFBF,
            ],
        ),
    ] {
        let mut g = rig(480, 640);
        let tex = checker(&mut g).with_linear(linear);
        assert_eq!(tex.words()[1] >> 31, u32::from(linear));
        g.set(0, Some(square(100, 100, 64, Paint::texture(tex))));
        g.frame();
        let row: Vec<u32> = (0..8).map(|k| g.px(100 + 2 * k, 104)).collect();
        assert_eq!(row, want, "linear {linear}");
    }
}

#[test]
fn texels_of_32_bits_and_pvrtc_as_measured() {
    // Run 24, T13: quadrants red/green/blue/white (alpha 0x80, drawn
    // opaque) and PVRTC solid and half-and-half blocks.
    let mut g = rig(480, 640);
    let q32: Vec<u32> = (0..64)
        .map(|i| match (i % 8 < 4, i / 8 < 4) {
            (true, true) => 0xFFFF_0000,
            (false, true) => 0xFF00_FF00,
            (true, false) => 0xFF00_00FF,
            (false, false) => 0x80FF_FFFF,
        })
        .collect();
    let t = upload32(&mut g.pool, &mut g.mem, 0, &q32).unwrap();
    assert_eq!(t.bytes(), 256);
    g.set(0, Some(square(100, 100, 64, Paint::texture(t))));
    g.frame();
    let quad = |g: &Rig| {
        [
            g.px(108, 108),
            g.px(156, 108),
            g.px(108, 156),
            g.px(156, 156),
        ]
    };
    // The white texel's alpha 0x80 comes out 0xFF: TEX2's opaque word.
    assert_eq!(
        quad(&g),
        [0xFFFF_0000, 0xFF00_FF00, 0xFF00_00FF, 0xFFFF_FFFF]
    );
    // PVRTC 4 bpp, 16 x 16: 16 blocks (modulation word, colour word).
    const RED: u32 = 0xFC00_FC00;
    const GREEN: u32 = 0x83E0_83E0;
    let solid: Vec<u32> = (0..32).map(|k| if k % 2 == 0 { 0 } else { RED }).collect();
    let p4 = upload_pvrtc(&mut g.pool, &mut g.mem, 1, format::PVRTC4, &solid).unwrap();
    assert_eq!(p4.bytes(), 128);
    g.set(0, Some(square(100, 100, 64, Paint::texture(p4))));
    g.frame();
    assert_eq!(quad(&g), [0xFFFF_0000; 4]);
    // 2 bpp: 8 blocks of 8 x 4.
    let p2 = upload_pvrtc(&mut g.pool, &mut g.mem, 1, format::PVRTC2, &solid[..16]).unwrap();
    g.set(0, Some(square(100, 100, 64, Paint::texture(p2))));
    g.frame();
    assert_eq!(quad(&g), [0xFFFF_0000; 4]);
    // Blocks 0-1 of each 4 red, 2-3 green.
    let half: Vec<u32> = (0..32)
        .map(|w| match (w % 2, (w / 2) % 4 < 2) {
            (0, _) => 0,
            (_, true) => RED,
            _ => GREEN,
        })
        .collect();
    let ph = upload_pvrtc(&mut g.pool, &mut g.mem, 1, format::PVRTC4, &half).unwrap();
    g.set(0, Some(square(100, 100, 64, Paint::texture(ph))));
    g.frame();
    assert_eq!(
        quad(&g),
        [0xFFFF_0000, 0xFF00_FF00, 0xFFFF_0000, 0xFF00_FF00]
    );
    // Wrong sizes are refused.
    assert!(upload_pvrtc(&mut g.pool, &mut g.mem, 1, format::PVRTC4, &solid[..30]).is_none());
    assert!(upload_pvrtc(&mut g.pool, &mut g.mem, 1, format::RGB565, &solid).is_none());
    assert!(upload32(&mut g.pool, &mut g.mem, 0, &q32[..60]).is_none());
}

#[test]
fn render_done_raises_the_interrupt_only_when_switched_on() {
    // Run 24, T10: 0x130 bits 15 and 2 and MINTENABLE 0x800F: one edge on
    // GPIO85 at render done; bit 2 alone: none.
    let mut g = rig(480, 640);
    g.frame();
    assert_eq!(g.gpu.irqs, 0, "off by default");
    g.r.set_interrupt(true);
    g.set(0, Some(square(0, 0, 8, Paint::Flat(BLUE))));
    g.frame();
    assert_eq!(g.gpu.irqs, 1);
    assert_eq!(g.gpu.read32(MBX + reg::ENABLE) & 0x8004, 0x8004);
    assert_eq!(g.gpu.read32(crate::MINTENABLE), crate::MINTENABLE_DELL);
    g.r.set_interrupt(false);
    g.set(0, Some(square(8, 0, 8, Paint::Flat(BLUE))));
    g.frame();
    assert_eq!(g.gpu.irqs, 1, "off again");
    assert_eq!(g.gpu.read32(MBX + reg::ENABLE) & 0x8000, 0);
}

#[test]
fn mipmaps_as_measured() {
    // rp-mbx3d run 25, T12b, layout B: a 64 x 64 texture, levels in
    // colours (64 red, 32 green, 16 blue, 8 yellow, smaller cyan), the
    // base first; a 16 px quad drew the 16 level, an 8 px quad the 8
    // level. Without levels (version 24's TEX0 bits 12-9 = 0): the base.
    let mut g = rig(480, 640);
    let mut t = Texture {
        levels: 7,
        ..upload(&mut g.pool, &mut g.mem, 3, format::RGB565, &[0; 4096]).unwrap()
    };
    let at = g.pool.alloc(t.bytes(), 128).unwrap();
    t.at = at;
    let colours: [u16; 7] = [0xF800, 0x07E0, 0x001F, 0xFFE0, 0x07FF, 0x07FF, 0x07FF];
    let mut off = 0;
    for (k, &c) in colours.iter().enumerate() {
        let side = 64u32 >> k;
        let n = (side * side).div_ceil(2) as usize;
        let w = u32::from(c) | u32::from(c) << 16;
        g.mem.store((at + off) as usize, &std::vec![w; n]);
        off += side * side * 2;
    }
    assert_eq!(
        t.words()[0] >> 9 & 15,
        6,
        "TEX0 bits 12-9: the highest level"
    );
    assert_eq!(t.words()[1] & 0xE000_0000, 0x2000_0000, "nearest mipmap");
    for (size, want) in [
        (64, 0xFFFF_0000u32),
        (32, 0xFF00_FF00),
        (16, 0xFF00_00FF),
        (8, 0xFFFF_FF00),
    ] {
        g.set(0, Some(square(200, 200, size, Paint::texture(t))));
        g.frame();
        let c = 200 + size as u32 / 2;
        assert_eq!(g.px(c, c), want, "{size} px");
    }
    let base_only = Texture { levels: 1, ..t };
    g.set(0, Some(square(200, 200, 8, Paint::texture(base_only))));
    g.frame();
    assert_eq!(g.px(204, 204), 0xFFFF_0000);
}

#[test]
fn mip_levels_are_made_by_averaging() {
    // The checker (red/white) averaged 2 x 2: RGB565 (31, 32, 16).
    let mut g = rig(480, 640);
    let texels: Vec<u16> = (0..64)
        .map(|i| {
            if (i % 8 + i / 8) % 2 == 0 {
                0xF800
            } else {
                0xFFFF
            }
        })
        .collect();
    let t = upload_mipmapped(&mut g.pool, &mut g.mem, 0, format::RGB565, &texels).unwrap();
    assert_eq!(t.levels, 4, "8 x 8 down to 1 x 1");
    assert_eq!(
        t.bytes(),
        128 + 32 + 8 + 4,
        "a 1 x 1 level takes a whole word"
    );
    g.set(0, Some(square(100, 100, 4, Paint::texture(t))));
    g.frame();
    assert_eq!(
        g.px(101, 101),
        0xFFFF_8284,
        "level 1: red and white averaged"
    );
    // Too large or not 16-bit: refused.
    assert!(upload_mipmapped(&mut g.pool, &mut g.mem, 8, format::RGB565, &texels).is_none());
    assert!(upload_mipmapped(&mut g.pool, &mut g.mem, 0, format::ARGB8888, &texels).is_none());
}

/// The performance gate's T6: mip levels are made on the CPU side, never
/// by reading 2700G memory back (12 MB/s, the bus's slowest direction),
/// and they're exactly the level-by-level 2 x 2 cascade.
#[test]
fn mip_levels_read_nothing_back_and_match_the_cascade() {
    struct NoReads {
        words: std::collections::BTreeMap<usize, u32>,
    }
    impl LocalMem for NoReads {
        fn store(&mut self, at: usize, words: &[u32]) {
            for (k, &w) in words.iter().enumerate() {
                self.words.insert(at + 4 * k, w);
            }
        }
        fn load(&self, _at: usize) -> Option<u32> {
            // Any load at all is the failure.
            panic!("upload_mipmapped read 2700G memory back");
        }
        fn sync(&mut self) {}
    }
    let side = 64usize;
    let base: Vec<u16> = (0..side * side)
        .map(|i| (i as u32).wrapping_mul(0x9E37_79B9).rotate_right(7) as u16)
        .collect();
    let mut pool = Pool::new(POOL);
    let mut mem = NoReads {
        words: std::collections::BTreeMap::new(),
    };
    let t = upload_mipmapped(&mut pool, &mut mem, 3, format::RGB565, &base).unwrap();
    // The reference: each level from the one above, as Vecs.
    let mut level = base.clone();
    let mut s = side;
    let mut offset = 0u32;
    for _ in 0..t.levels {
        for y in 0..s {
            for x in 0..s {
                let a = t.at + offset + 2 * tex::twiddle(x as u32, y as u32);
                let w = mem.words[&((a & !3) as usize)];
                let got = (if a & 2 == 0 { w } else { w >> 16 }) as u16;
                assert_eq!(got, level[y * s + x], "level size {s} at ({x}, {y})");
            }
        }
        offset += (s * s * 2).max(4) as u32;
        if s == 1 {
            break;
        }
        let n = s / 2;
        level = (0..n * n)
            .map(|i| {
                let (x, y) = (i % n, i / n);
                let at = |dx: usize, dy: usize| level[(2 * y + dy) * s + 2 * x + dx];
                average4(format::RGB565, [at(0, 0), at(1, 0), at(0, 1), at(1, 1)])
            })
            .collect();
        s = n;
    }
}

#[test]
fn sixteen_bit_targets_as_measured() {
    // rp-mbx3d run 26, S26b: a 4 x 4 tile block, quarters red, green, blue
    // and white: RGB565 (0x650 1) and ARGB1555 (0x650 3), 0x660 the stride
    // in bytes / 4.
    for (format, want) in [
        (PixelFormat::Rgb565, [0xF800u32, 0x07E0, 0x001F, 0xFFFF]),
        (PixelFormat::Argb1555, [0xFC00, 0x83E0, 0x801F, 0xFFFF]),
    ] {
        let mut g = rig(32, 64);
        g.r.set_background(None).unwrap();
        for (k, c) in [RED, 0xFF00_FF00, BLUE, 0xFFFF_FFFF]
            .into_iter()
            .enumerate()
        {
            let (x, y) = (16 * (k as i32 % 2), 32 * (k as i32 / 2));
            g.set(k, Some(Draw::rect(x, y, 16, 32, Paint::Flat(c))));
        }
        let pic = Picture::packed(0x00A0_0000, 480, 640, format);
        g.frame_into(Target {
            picture: pic,
            x: 0,
            y: 0,
        });
        assert_eq!(g.gpu.reg(0x650), format.code());
        assert_eq!(g.gpu.reg(0x660), 240, "960 bytes a line / 4");
        let p16 = |g: &Rig, x: u32, y: u32| {
            let a = (pic.at + y * pic.stride + 2 * x) as usize;
            (g.mem.load(a & !3).unwrap() >> (8 * (a & 2))) & 0xFFFF
        };
        assert_eq!(
            [
                p16(&g, 2, 2),
                p16(&g, 18, 2),
                p16(&g, 2, 34),
                p16(&g, 18, 34)
            ],
            want
        );
        assert_eq!(
            p16(&g, 32, 2),
            0,
            "the block ends at 32 px: tiles stay 8 x 16"
        );
    }
}

/// `rp-gpubench` run P (X51v, 2026-10-21): every `0x650` layout, bits 2-0
/// (codes 8-15 lay out like 0-7), each as the device wrote pixel (1, 0) of
/// the placed texture: red 8, green 0, blue `0xA0`, opaque.
#[test]
fn every_target_layout_as_run_p_measured() {
    const C: u32 = 0xFF08_00A0;
    for (code, bytes, want) in [
        (0u32, 2usize, 0x0414u32),
        (1, 2, 0x0814),
        (2, 2, 0xF00A),
        (3, 2, 0x8414),
        (4, 3, 0x0800A0),
        (5, 4, 0x0008_00A0),
        (6, 4, 0xFF08_00A0),
        (7, 2, 0xF00A),
        (9, 2, 0x0814),
        (0x0010_0006, 4, 0xFF08_00A0),
    ] {
        let mut g = rig(32, 32);
        g.set(0, Some(Draw::rect(0, 0, 32, 32, Paint::Flat(C))));
        let pic = Picture::packed(0x00A0_0000, 32, 32, PixelFormat::Argb8888);
        let t = Target {
            picture: pic,
            x: 0,
            y: 0,
        };
        g.frame_into(t);
        // The renderer writes only changed registers: run P's own way in.
        g.gpu.write32(MBX + 0x650, code);
        g.r.invalidate_all();
        g.frame_into(t);
        let row = pic.at + pic.stride;
        let got = (0..bytes).fold(0, |v, k| {
            let a = row as usize + bytes + k;
            v | ((g.mem.load(a & !3).unwrap() >> (8 * (a & 3))) & 0xFF) << (8 * k)
        });
        assert_eq!(got, want, "code {code:#x}");
    }
}

/// `rp-gpubench` run P2 (X51v, 2026-10-21): bit 3 dithers 16-bit layouts
/// by a 4 x 4 matrix from the target's first pixel, and code 3 writes the
/// top bit 1 even for a transparent pixel (P2c: alpha 0 drawn as 0xE208).
#[test]
fn dither_and_forced_alpha_bit_as_run_p2_measured() {
    // (code, colour, pixel, value): grey 76 is 9.5 steps of 8 (565 red
    // and blue), 19 of 4 (green): up where the matrix's threshold is 8 or
    // less for 5 bits, 16 or less (never 16 itself) for 6 bits.
    for (code, c, (x, y), want) in [
        (9u32, 0xFF4C_4C4Cu32, (0u32, 0u32), 9 << 11 | 19 << 5 | 9),
        (9, 0xFF4C_4C4C, (1, 0), 10 << 11 | 19 << 5 | 10),
        (9, 0xFF4C_4C4C, (3, 3), 9 << 11 | 19 << 5 | 9),
        (1, 0xFF4C_4C4C, (1, 0), 9 << 11 | 19 << 5 | 9),
        // 4444 at level 72 (4.5 steps of 16): up where the threshold is 8
        // or less.
        (10, 0xFF48_4848, (0, 0), 0xF444),
        (10, 0xFF48_4848, (1, 0), 0xF555),
        (3, 0x00C0_8040, (0, 0), 0xE208),
    ] {
        let mut g = rig(32, 32);
        g.set(0, Some(Draw::rect(0, 0, 32, 32, Paint::Flat(c))));
        let pic = Picture::packed(0x00A0_0000, 32, 32, PixelFormat::Argb8888);
        let t = Target {
            picture: pic,
            x: 0,
            y: 0,
        };
        g.frame_into(t);
        g.gpu.write32(MBX + 0x650, code);
        g.r.invalidate_all();
        g.frame_into(t);
        let a = (pic.at + y * pic.stride + 2 * x) as usize;
        let got = (g.mem.load(a & !3).unwrap() >> (8 * (a & 2))) & 0xFFFF;
        assert_eq!(got, want, "code {code}, pixel ({x}, {y})");
    }
}

/// `rp-gpubench` run T (X51v, 2026-10-21): `0x644` bit 0 halves the rows
/// on write-out, bit 1 the columns, each pixel the average of the rendered
/// ones; with run P's placed colours (red `x * 8`, green `y * 8`) the
/// device wrote row 0, 1 as green `0x04`, `0x14` (bit 0) and pixel 0, 1 as
/// red `0x04`, `0x14` (bit 1).
#[test]
fn write_out_downsample_as_run_t_measured() {
    for (v, want) in [
        (1u32, [0xFF00_04A0u32, 0xFF08_04A0, 0xFF00_14A0]),
        (2, [0xFF04_00A0, 0xFF14_00A0, 0xFF04_08A0]),
        (3, [0xFF04_04A0, 0xFF14_04A0, 0xFF04_14A0]),
    ] {
        let mut g = rig(32, 32);
        let texels: Vec<u32> = (0..1024u32)
            .map(|i| 0xFF00_00A0 | ((i % 32) << 3) << 16 | ((i / 32) << 3) << 8)
            .collect();
        let tex = upload32(&mut g.pool, &mut g.mem, 2, &texels).unwrap();
        g.set(0, Some(Draw::rect(0, 0, 32, 32, Paint::texture(tex))));
        let pic = Picture::packed(0x00A0_0000, 32, 32, PixelFormat::Argb8888);
        let t = Target {
            picture: pic,
            x: 0,
            y: 0,
        };
        g.frame_into(t);
        g.gpu.write32(MBX + 0x644, v);
        g.r.invalidate_all();
        g.frame_into(t);
        let px = |x: u32, y: u32| {
            g.mem
                .load((pic.at + y * pic.stride + 4 * x) as usize)
                .unwrap()
        };
        assert_eq!([px(0, 0), px(1, 0), px(0, 1)], want, "0x644 = {v}");
    }
}

#[test]
fn slot_words_at_finds_the_texture_words_set_stored() {
    let mut g = rig(480, 640);
    let t = upload(&mut g.pool, &mut g.mem, 1, format::RGB565, &[0; 256]).unwrap();
    g.set(5, Some(Draw::rect(0, 0, 16, 16, Paint::texture(t))));
    let at = g.r.slot_words_at(5).expect("slot");
    let words: Vec<u32> = (0..64)
        .map(|k| g.mem.load(at as usize + 4 * k).unwrap())
        .collect();
    // Both triangles carry TEX0 (the probes change it there).
    assert_eq!(words.iter().filter(|&&w| w == t.words()[0]).count(), 2);
    assert_eq!(g.r.slot_words_at(MAX_SLOTS), None);
}

/// `rp-gpubench` G4a as the device drew it: a 64 x 32 RGB565 texture
/// (TEX0's size codes apart, the rectangle twiddle order) drawn 1:1 shows
/// each texel at its own pixel.
#[test]
fn rectangular_textures_draw_as_the_device_did() {
    let mut g = rig(480, 640);
    let (bu, bv) = (6u32, 5u32);
    let at = g.pool.alloc(2 << (bu + bv), 128).unwrap();
    // Texel (x, y): green the column, red and blue the row.
    let colour = |x: u32, y: u32| (((y >> 5) & 31) << 11 | (x & 63) << 5 | (y & 31)) as u16;
    let words: Vec<u32> = (0..(1u32 << (bu + bv)) / 2)
        .map(|w| {
            let texel = |i: u32| {
                // The inverse of twiddle_rect for a wider-than-tall texture.
                let (x, y) = untwiddle(i & ((1 << (2 * bv)) - 1));
                u32::from(colour(x | (i >> (2 * bv)) << bv, y))
            };
            texel(2 * w) | texel(2 * w + 1) << 16
        })
        .collect();
    g.mem.store(at as usize, &words);
    let t = Texture {
        at,
        code: 3,
        format: format::RGB565 as u8,
        u_mode: address::CLAMP as u8,
        v_mode: address::CLAMP as u8,
        linear: false,
        levels: 1,
    };
    g.set(0, Some(Draw::rect(64, 64, 64, 32, Paint::texture(t))));
    // TEX0 as the renderer built it (64 x 64), its height code made 2.
    let slot = g.r.slot_words_at(0).unwrap();
    let square = t.words()[0];
    let rect = (square & !0x00F0_0000) | 2 << 20;
    for k in 0..64 {
        let a = slot as usize + 4 * k;
        if g.mem.load(a) == Some(square) {
            g.mem.store(a, &[rect]);
        }
    }
    g.frame_into(FULL);
    for (x, y) in [(0u32, 0u32), (37, 3), (63, 31), (32, 16), (5, 30)] {
        let p = g.px(64 + x, 64 + y);
        let c = u32::from(colour(x, y));
        let want = (((c >> 11) << 3 | (c >> 11) >> 2) << 16)
            | (((c >> 5) & 63) << 2 | ((c >> 5) & 63) >> 4) << 8
            | ((c & 31) << 3 | (c & 31) >> 2);
        assert_eq!(p & 0xFF_FFFF, want, "texel ({x},{y})");
    }
}

/// The CPU binner (P4 step 3) against the model of the 3D core.
mod binner_scenes {
    use super::*;
    use crate::binner::{self, Binned, Layout, Scratch};

    const LAYOUT: Layout = Layout {
        params: 0x0010_0000,
        params_size: 0x0010_0000,
        regions: 0x0048_0000,
        w: 480,
        h: 640,
    };
    const PM_TABLE: u32 = 0x000F_C000;

    /// A 2700G memory and core with the 3D clock on and the memory
    /// manager set up for [`LAYOUT`].
    struct Bench {
        mem: Mem,
        gpu: Gpu,
        clock: SimClock,
        starts: Vec<u32>,
        entries: Vec<u16>,
        objects: Vec<u32>,
    }

    fn bench(quads: usize) -> Bench {
        let m = memory();
        let clock = SimClock::default();
        let mut gpu = Gpu::new(m.clone(), &clock);
        rp_drv_mbx_display::clock::restore(&mut gpu, rp_drv_mbx_display::clock::with_3d(0x5));
        for (off, v) in m3::pm_init_table(LAYOUT.params, LAYOUT.params_size, PM_TABLE) {
            gpu.write32(MBX + off, v);
        }
        clock.advance(200);
        Bench {
            mem: Mem::new(m),
            gpu,
            clock,
            starts: std::vec![0; 2401],
            entries: std::vec![0; 64 * quads + 64],
            objects: std::vec![0; quads],
        }
    }

    impl Bench {
        fn bin(&mut self, bg: Option<u32>, draws: &[Draw]) -> Result<Binned, Error> {
            binner::bin(
                &mut self.mem,
                &LAYOUT,
                bg,
                draws,
                Scratch {
                    starts: &mut self.starts,
                    entries: &mut self.entries,
                    objects: &mut self.objects,
                },
            )
        }

        /// Renders whatever the region array lists; true if it finished.
        fn render(&mut self) -> bool {
            self.gpu.write32(MBX + reg::CLEAR, m3::RENDER_DONE);
            for (off, v) in binner::start_table(&LAYOUT, FULL) {
                self.gpu.write32(MBX + off, v);
            }
            // Long enough for every tile reading every object (Dell's way).
            self.clock.advance(10_000_000);
            self.gpu.read32(MBX + reg::STATUS) & m3::RENDER_DONE != 0
        }

        fn px(&self, x: u32, y: u32) -> u32 {
            self.mem.px(SCREEN.at, SCREEN.stride, x, y)
        }

        fn picture(&self) -> Vec<u32> {
            (0..640)
                .flat_map(|y| (0..480).map(move |x| (x, y)))
                .map(|(x, y)| self.px(x, y))
                .collect()
        }
    }

    /// An 8 x 8 RGB565 texture, column `x` colour `x`, from `pool`.
    fn texture_in(pool: &mut Pool, mem: &mut Mem) -> Texture {
        let texels: Vec<u16> = (0..64).map(|i| ((i % 8) as u16 + 1) * 0x0841).collect();
        upload(pool, mem, 0, format::RGB565, &texels).expect("upload")
    }

    /// [`texture_in`] a fresh pool (the bench keeps nothing else there).
    fn texture(mem: &mut Mem) -> Texture {
        texture_in(&mut Pool::new(POOL), mem)
    }

    /// `n` quads of `side` pixels scattered by a fixed sequence, some off
    /// the screen's edges, flat, Gouraud and textured in turn, some
    /// blended.
    fn scene(n: usize, side: i32, tex: Texture) -> Vec<Draw> {
        let mut s = 0x2545_F491u32;
        let mut next = |m: u32| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 8) % m
        };
        (0..n)
            .map(|k| {
                let x = next(480 + side as u32) as i32 - side / 2;
                let y = next(640 + side as u32) as i32 - side / 2;
                let c = 0xFF00_0000 | next(0x0100_0000);
                let paint = match k % 4 {
                    0 => Paint::Flat(c),
                    1 => Paint::Gouraud([c, !c | 0xFF00_0000, c ^ 0x00FF_00FF, RED]),
                    _ => Paint::texture(tex),
                };
                let d = Draw::rect(x, y, side, side, paint);
                if k % 7 == 3 { d.over() } else { d }
            })
            .collect()
    }

    #[test]
    fn a_binned_scene_draws_what_the_renderer_draws() {
        // 60 quads: within the renderer's 64 slots.
        let mut g = rig(480, 640);
        let t = texture_in(&mut g.pool, &mut g.mem);
        let draws = scene(60, 40, t);
        g.r.set_background(Some(GREY)).unwrap();
        for (k, d) in draws.iter().enumerate() {
            g.set(k, Some(*d));
        }
        g.frame();
        let want: Vec<u32> = (0..640)
            .flat_map(|y| (0..480).map(move |x| (x, y)))
            .map(|(x, y)| g.px(x, y))
            .collect();

        let mut b = bench(60);
        // The same texels at the same place in the bench's memory.
        let texels: Vec<u32> = (0..t.bytes() / 4)
            .map(|k| g.mem.load((t.at + 4 * k) as usize).unwrap())
            .collect();
        b.mem.store(t.at as usize, &texels);
        let binned = b.bin(Some(GREY), &draws).unwrap();
        assert!(b.render());
        assert_eq!(b.gpu.tiles, 2400);
        let got = b.picture();
        let wrong = got.iter().zip(&want).filter(|(a, b)| a != b).count();
        assert_eq!(wrong, 0, "pixels unlike the renderer's");
        assert_eq!(binned.tiles, 2400);
        assert_eq!(binned.region_words, 4800);
    }

    /// Dell's way, the reference: every tile lists every object.
    fn one_list(b: &mut Bench, n: usize) {
        let mut list = std::vec![m3::STRIP_ONE_TRIANGLE | (64 / 4)];
        for &p in &b.objects[..n] {
            list.extend([p, p + object_words(p)]);
        }
        list.push(m3::END);
        let at = LAYOUT.params + 0xF_0000;
        b.mem.store(at as usize, &list);
        let mut regions = std::vec![0u32; 4800];
        m3::region_array(&mut regions, 60, 40, at).unwrap();
        b.mem.store(LAYOUT.regions as usize, &regions);
    }

    fn object_words(p: u32) -> u32 {
        if p & 0xFFE0_0000 == tex::STRIP_GAME {
            tex::WORDS as u32
        } else {
            m3::TRIANGLE_WORDS as u32
        }
    }

    /// The list words for the tiles that have quads (background, two
    /// pointers an entry, the end), from the binner's tile ends.
    fn listed(b: &Bench, entries: u32) -> u32 {
        let (mut begin, mut words) = (0, 0);
        for &stop in &b.starts[..2400] {
            if stop != begin {
                words += 2 + 2 * (stop - begin);
            }
            begin = stop;
        }
        assert_eq!(begin, entries);
        words
    }

    #[test]
    fn hundreds_of_quads_binned_equal_every_tile_reading_every_object() {
        // 16 x 16 quads (a game's sprites): 250, as many as one list holds
        // in the model (512 entries).
        let mut b = bench(250);
        let t = texture(&mut b.mem);
        let draws = scene(250, 16, t);
        let binned = b.bin(Some(GREY), &draws).unwrap();
        assert!(b.render());
        let got = b.picture();
        // The lists: the shared empty one (background, end) and the rest.
        assert_eq!(binned.list_words, 2 + listed(&b, binned.entries));
        // A 16 x 16 quad touches 2-3 columns and 1-2 rows (fewer at the
        // screen's edges).
        assert!((250..=250 * 6).contains(&binned.entries), "{binned:?}");
        one_list(&mut b, 250);
        assert!(b.render());
        let want = b.picture();
        assert_eq!(got.iter().zip(&want).filter(|(a, b)| a != b).count(), 0);
    }

    #[test]
    fn quads_keep_their_draw_order_in_every_tile() {
        // Three opaque squares over one another: the last on top.
        let mut b = bench(3);
        let draws = [
            Draw::rect(100, 100, 60, 60, Paint::Flat(RED)),
            Draw::rect(120, 120, 60, 60, Paint::Flat(BLUE)),
            Draw::rect(140, 140, 60, 60, Paint::Flat(0xFF00_FF00)),
        ];
        b.bin(Some(GREY), &draws).unwrap();
        assert!(b.render());
        assert_eq!(b.px(110, 110), RED);
        assert_eq!(b.px(130, 130), BLUE);
        assert_eq!(b.px(150, 150), 0xFF00_FF00);
        assert_eq!(b.px(50, 50), GREY);
    }

    #[test]
    fn a_binned_scene_renders_again_with_nothing_written() {
        let mut b = bench(100);
        let t = texture(&mut b.mem);
        let before = b.mem.1;
        let binned = b.bin(Some(GREY), &scene(100, 16, t)).unwrap();
        // The objects, lists and region array each stored once.
        assert_eq!(b.mem.1 - before, binned.words());
        assert!(b.render());
        let first = b.picture();
        let stored = b.mem.1;
        assert!(b.render());
        assert_eq!(b.mem.1, stored, "a repeated frame stores nothing");
        assert_eq!(b.picture(), first);
    }

    #[test]
    fn no_background_leaves_empty_tiles_white_and_lists_only_the_quads() {
        let mut b = bench(1);
        let binned = b
            .bin(None, &[Draw::rect(0, 0, 8, 16, Paint::Flat(RED))])
            .unwrap();
        assert!(b.render());
        assert_eq!(b.px(4, 8), RED);
        assert_eq!(b.px(20, 8), 0xFFFF_FFFF);
        // One tile: two pointers and the end; the shared empty list: the
        // end.
        assert_eq!((binned.entries, binned.most), (1, 1));
        assert_eq!(binned.list_words, 1 + 3);
    }

    #[test]
    fn too_little_room_is_refused_not_overrun() {
        let mut b = bench(10);
        let t = texture(&mut b.mem);
        let draws = scene(10, 64, t);
        let mut try_with = |objects: usize, entries: usize, l: &Layout| {
            binner::bin(
                &mut b.mem,
                l,
                None,
                &draws,
                Scratch {
                    starts: &mut b.starts,
                    entries: &mut b.entries[..entries],
                    objects: &mut b.objects[..objects],
                },
            )
        };
        // Scratch for 9 quads' objects; entries for 5.
        assert_eq!(try_with(9, 700, &LAYOUT), Err(Error::NoMemory));
        assert_eq!(try_with(10, 5, &LAYOUT), Err(Error::NoMemory));
        // A parameter memory too small for the objects.
        let small = Layout {
            params_size: 1024,
            ..LAYOUT
        };
        assert_eq!(try_with(10, 700, &small), Err(Error::TooComplex));
        // Room for the objects, not the lists.
        let tight = Layout {
            params_size: 256 + 10 * 4 * 52 + 64,
            ..LAYOUT
        };
        assert_eq!(try_with(10, 700, &tight), Err(Error::TooComplex));
        // An unknown depth mode.
        let bad = [draws[0].with_depth(Depth {
            mode: 8,
            write: false,
            z: 0,
        })];
        assert_eq!(b.bin(None, &bad), Err(Error::Unsupported));
    }
}

/// GC2AN's orders: each lists every tile exactly once (Columns' and
/// Scrambled's first half, Checker's two passes), Checker never lists a
/// tile's neighbour next, and every order draws the same picture as row
/// order through the model.
#[test]
fn new_tile_orders_list_every_tile_once_and_draw_the_same() {
    for (cols, rows) in [(60, 40), (7, 5), (1, 1), (128, 128)] {
        let n = cols * rows;
        for order in [
            TileOrder::Columns,
            TileOrder::Checker,
            TileOrder::Scrambled,
            TileOrder::Interleaved,
        ] {
            let got: Vec<usize> = (0..2 * n)
                .filter_map(|i| crate::order_tile(order, i, cols, rows))
                .collect();
            let mut sorted = got.clone();
            sorted.sort_unstable();
            assert_eq!(
                sorted,
                (0..n).collect::<Vec<_>>(),
                "{order:?} {cols}x{rows}"
            );
            // Interleaved: no side-by-side neighbour next (rows of 4 tiles
            // or more), and the rows strictly from the top.
            if order == TileOrder::Interleaved && cols >= 4 {
                for w in got.windows(2) {
                    let (a, b) = (w[0], w[1]);
                    assert!(
                        !(a / cols == b / cols && (a % cols).abs_diff(b % cols) == 1),
                        "{cols}x{rows}: {a} then {b}"
                    );
                    assert!(b / cols >= a / cols, "top down");
                }
            }
            if order == TileOrder::Checker {
                for w in got.windows(2) {
                    let (a, b) = (w[0], w[1]);
                    let (dx, dy) = ((a % cols).abs_diff(b % cols), (a / cols).abs_diff(b / cols));
                    assert!(dx + dy != 1, "{cols}x{rows}: {a} then its neighbour {b}");
                }
            }
        }
    }
    let mut g = rig(480, 640);
    let tex = columns(&mut g);
    let c = [0xFF00_0000, 0xFFFF_FFFF, 0xFF10_20A0, 0xFFFF_E000];
    g.set(0, Some(Draw::rect(0, 0, 480, 640, Paint::Gouraud(c))));
    g.set(1, Some(Draw::rect(43, 45, 400, 560, Paint::texture(tex))));
    g.frame();
    let words = |g: &Rig| -> Vec<u32> {
        (0..480 * 640)
            .map(|i| g.mem.px(SCREEN.at, SCREEN.stride, i % 480, i / 480))
            .collect()
    };
    let a = words(&g);
    for order in [
        TileOrder::Columns,
        TileOrder::Checker,
        TileOrder::Scrambled,
        TileOrder::Interleaved,
    ] {
        g.r.set_tile_order(order);
        g.r.invalidate_all();
        let f = g.frame();
        assert_eq!(f.tiles, 60 * 40, "{order:?}");
        assert_eq!(words(&g), a, "{order:?}");
    }
}

/// GC2AP's hook: a flip changes exactly that header word of both triangles
/// of a textured slot, nothing of a flat one, and (0, 0) or a word past
/// the header nothing at all.
#[test]
fn the_header_flip_changes_only_its_word() {
    let mut g = rig(480, 640);
    let tex = columns(&mut g);
    let at = |g: &Rig, w: usize| {
        g.mem
            .load((rp_vram_map::PARAMS.at + SLOTS_AT) as usize + 4 * w)
            .unwrap()
    };
    let draw = Draw::rect(43, 45, 400, 560, Paint::texture(tex));
    g.set(0, Some(draw));
    let base: Vec<u32> = (0..2 * m3::tex::WORDS).map(|w| at(&g, w)).collect();
    for (word, mask) in [(1usize, 1u32 << 7), (4, 1 << 22), (0, 1 << 3)] {
        g.r.set_header_xor(word, mask);
        g.set(0, Some(draw));
        for (w, &b) in base.iter().enumerate() {
            let flipped = w % m3::tex::WORDS == word;
            let want = if flipped { b ^ mask } else { b };
            assert_eq!(at(&g, w), want, "word {word} mask {mask:#x}: {w}");
        }
    }
    for (word, mask) in [(0usize, 0u32), (5, 1)] {
        g.r.set_header_xor(word, mask);
        g.set(0, Some(draw));
        for (w, &b) in base.iter().enumerate() {
            assert_eq!(at(&g, w), b, "({word}, {mask}) flips nothing");
        }
    }
    // A flat slot is never flipped.
    g.r.set_header_xor(0, u32::MAX);
    let flat = Draw::rect(0, 0, 64, 64, Paint::Flat(0xFF12_3456));
    g.set(1, Some(flat));
    let f1: Vec<u32> = (0..SLOT_WORDS)
        .map(|w| {
            g.mem
                .load((rp_vram_map::PARAMS.at + SLOTS_AT + SLOT_BYTES) as usize + 4 * w)
                .unwrap()
        })
        .collect();
    g.r.set_header_xor(0, 0);
    g.set(1, Some(Draw::rect(0, 0, 64, 32, Paint::Flat(0xFF12_3456))));
    g.set(1, Some(flat));
    let f2: Vec<u32> = (0..SLOT_WORDS)
        .map(|w| {
            g.mem
                .load((rp_vram_map::PARAMS.at + SLOTS_AT + SLOT_BYTES) as usize + 4 * w)
                .unwrap()
        })
        .collect();
    assert_eq!(f1, f2);
}

/// GC2AQ's hook: the bits land in the TSP word of both triangles of a flat
/// and of a textured slot, and nowhere else; 0 changes nothing.
#[test]
fn the_tsp_bits_land_in_every_objects_tsp() {
    let mut g = rig(480, 640);
    let tex = columns(&mut g);
    let word = |g: &Rig, slot: u32, w: usize| {
        g.mem
            .load((rp_vram_map::PARAMS.at + SLOTS_AT + slot * SLOT_BYTES) as usize + 4 * w)
            .unwrap()
    };
    let flat = Draw::rect(0, 0, 64, 64, Paint::Flat(0xFF12_3456));
    let textured = Draw::rect(43, 45, 400, 560, Paint::texture(tex));
    g.set(0, Some(flat));
    g.set(1, Some(textured));
    let f0: Vec<u32> = (0..2 * m3::TRIANGLE_WORDS)
        .map(|w| word(&g, 0, w))
        .collect();
    let t0: Vec<u32> = (0..2 * m3::tex::WORDS).map(|w| word(&g, 1, w)).collect();
    g.r.set_tsp_or(1 << 14);
    g.set(0, Some(flat));
    g.set(1, Some(textured));
    for (w, &b) in f0.iter().enumerate() {
        let tsp = w % m3::TRIANGLE_WORDS == 1;
        assert_eq!(
            word(&g, 0, w),
            if tsp { b | 1 << 14 } else { b },
            "flat {w}"
        );
    }
    for (w, &b) in t0.iter().enumerate() {
        let tsp = w % m3::tex::WORDS == 4;
        assert_eq!(
            word(&g, 1, w),
            if tsp { b | 1 << 14 } else { b },
            "textured {w}"
        );
    }
    g.r.set_tsp_or(0);
    g.set(0, Some(flat));
    g.set(1, Some(textured));
    for (w, &b) in f0.iter().enumerate() {
        assert_eq!(word(&g, 0, w), b);
    }
    for (w, &b) in t0.iter().enumerate() {
        assert_eq!(word(&g, 1, w), b);
    }
}
