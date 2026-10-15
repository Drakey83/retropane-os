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
const SCREEN: Picture = Picture::screen(0);
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
    assert!(s.predicted_us >= predict_us(s.tiles) && s.done_us >= s.predicted_us);
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
    let pic = [Picture::screen(0), Picture::screen(1)];
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
    // Version 23's T7b, through the API: red at 0.5 writing depth, blue at
    // 0.3 or 0.8 with "greater", and "less" never passing (not even over
    // red at 0.3, where it should: open, kept as measured).
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
        (depth::LESS, 3, RED, GREY),
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
fn no_quad_sets_a_tsp_bit_that_hangs_the_core() {
    // TSP bits 16 and 19 hung the device's core (versions 21-22): no
    // combination of paint, blend and depth may set them.
    let tex = Texture {
        at: 0x0094_0000,
        code: 0,
        format: format::ARGB4444 as u8,
        u_mode: 0,
        v_mode: 0,
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
    for _ in 0..1000 {
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
    // Back on, set up again (until T9 says it's not needed), and drawing.
    let inits = |g: &Rig| g.gpu.writes.iter().filter(|w| w.0 == MBX + 0x838).count();
    let before = inits(&g);
    g.clock.set(due);
    g.set(0, Some(square(0, 0, 8, Paint::Flat(BLUE))));
    g.frame();
    assert_eq!(inits(&g), before + 1);
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
