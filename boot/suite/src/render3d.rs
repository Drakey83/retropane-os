//! The 3D renderer's CPU set-up (`rp-mbx3d-render`, R88 and R78 item 3):
//! what one frame costs the CPU before the core starts, the region array,
//! each tile's list and the objects, timed under `-icount` (instructions,
//! not the bus). The bus and the core are stand-ins: every status bit reads
//! set (the memory manager ready, the render done at once), and stores into
//! 2700G memory are only checksummed, so each word stored costs about what
//! a store over the bus costs the CPU (the bus's own time is not here:
//! `rp-gpubench` GC2CP measures it on the X51v).
//!
//! The scenes are GC2CP's: a 5 x 6 warped mesh over the whole target
//! (`rp-gpubench`'s `mesh_frame`), flat or textured, at 480 x 640 and
//! 480 x 320, the tiles in the checkerboard order the divide-1 cure ships
//! with and in rows; and the shell's case, one button-sized quad changed.

use core::hint::black_box;

use rp_hal_api::RegisterBus;
use rp_mbx3d_render::{
    Clock, Depth, Draw, LocalMem, Objects, Paint, Picture, PixelFormat, Point, Poll, Renderer, SUB,
    Target, Texture, TileOrder, UV_ONE, format,
};
use rp_vram_map::{PARAMS, PM_TABLE, POOL, Pool};

use rp_hal_pxa27x::rt;
use rp_mbx3d_render::binner::{self, Scratch};

use crate::Runner;

/// Every register reads all ones (status: ready, done); writes are dropped.
struct Bus;

impl RegisterBus for Bus {
    fn read32(&mut self, _offset: usize) -> u32 {
        u32::MAX
    }
    fn write32(&mut self, _offset: usize, value: u32) {
        black_box(value);
    }
}

/// 2700G memory as a checksum of what is stored.
struct Sink(u32);

impl LocalMem for Sink {
    fn store(&mut self, _at: usize, words: &[u32]) {
        let mut s = self.0;
        for &w in words {
            s = s.rotate_left(1) ^ w;
        }
        self.0 = s;
    }
    fn load(&self, _at: usize) -> Option<u32> {
        None
    }
    fn sync(&mut self) {
        black_box(self.0);
    }
}

/// A clock that moves only when waited on.
struct Tick(u32);

impl Clock for Tick {
    fn now_us(&mut self) -> u32 {
        self.0
    }
    fn delay_us(&mut self, us: u32) {
        self.0 = self.0.wrapping_add(us);
    }
}

const COLS: i32 = 5;
const ROWS: i32 = 6;

/// `rp-gpubench`'s mesh hash.
const fn hash(a: u32, b: u32, c: u32) -> u32 {
    let mut h =
        a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77) ^ c.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h
}

/// Corner `(gx, gy)` of mesh `seed` over a `w` x `h` target, inner corners
/// moved up to 20 pixels each way (GC2BW's warp).
fn corner(seed: u32, gx: i32, gy: i32, w: i32, h: i32) -> Point {
    let span = (2 * 20 * SUB + 1) as u32;
    let mut x = w * SUB * gx / COLS;
    let mut y = h * SUB * gy / ROWS;
    if gx > 0 && gx < COLS {
        x += (hash(seed, gx as u32, gy as u32) % span) as i32 - 20 * SUB;
    }
    if gy > 0 && gy < ROWS {
        y += (hash(seed ^ 0x55, gx as u32, gy as u32) % span) as i32 - 20 * SUB;
    }
    Point { x, y }
}

/// The mesh's 30 quads, flat or textured with `tex` (three repeats across);
/// with `depth`, GC2CP's depth frame: a full-screen quad written far (slot
/// 30), the mesh "greater" with writes over it.
fn mesh(
    seed: u32,
    (w, h): (i32, i32),
    tex: Option<Texture>,
    depth: bool,
    out: &mut [Option<Draw>; 31],
) {
    let z = |z: u16| Depth {
        mode: rp_mbx3d_render::depth::GREATER,
        write: true,
        z,
    };
    for gy in 0..ROWS {
        for gx in 0..COLS {
            let c = [
                corner(seed, gx, gy, w, h),
                corner(seed, gx + 1, gy, w, h),
                corner(seed, gx + 1, gy + 1, w, h),
                corner(seed, gx, gy + 1, w, h),
            ];
            let i = (gy * COLS + gx) as u32;
            let paint = match tex {
                Some(t) => {
                    let uv =
                        |p: Point| (3 * UV_ONE / (w * SUB) * p.x, 4 * UV_ONE / (h * SUB) * p.y);
                    Paint::Texture {
                        tex: t,
                        uv: c.map(uv),
                        tint: 0xFFFF_FFFF,
                    }
                }
                None => Paint::Flat(0xFF00_0000 | ((hash(seed, i, 7) & 0x00BF_BFBF) + 0x0040_4040)),
            };
            let mut d = Draw::rect(0, 0, 1, 1, paint);
            d.corners = c;
            if depth {
                d.depth = Some(z(0xB000));
            }
            out[i as usize] = Some(d);
        }
    }
    out[30] = depth.then(|| Draw::rect(0, 0, w, h, Paint::Flat(0xFF20_4060)).with_depth(z(0x3000)));
}

/// One frame: the renderer's set-up, then its end seen at once.
fn frame(r: &mut Renderer, mem: &mut Sink, clock: &mut Tick, t: Target) {
    let mut bus = Bus;
    if let Ok(Some(due)) = r.render(&mut bus, mem, clock, t) {
        clock.0 = due;
        while let Poll::Wait(next) = r.poll(&mut bus, mem, clock) {
            clock.0 = next;
        }
    }
}

/// One benchmark: a renderer of `size` with the mesh set (its tiles in
/// `order`, its quads as `objects`, textured with `tex` or flat), then
/// `iters` frames, each the whole frame again on the scene unchanged
/// (`moving` false: GC2CP's set-up figure) or every quad moved (a game's
/// frame). One renderer alive at a time (the stack).
#[inline(never)]
fn one(
    r: &mut Runner,
    name: &'static str,
    (size, order, objects): ((u16, u16), TileOrder, Objects),
    (tex, depth): (Option<Texture>, bool),
    moving: bool,
) {
    let mut pool = Pool::new(POOL);
    let (w, h) = size;
    let Some(mut rr) = Renderer::new_in(&mut pool, w, h, PARAMS, PM_TABLE) else {
        r.check(name, false, "no renderer");
        return;
    };
    rr.set_tile_order(order);
    rr.set_objects(objects);
    let (mut mem, mut clock) = (Sink(0), Tick(0));
    let mut draws = [None; 31];
    // GC2CP's depth frame is mesh 3 (`rp-gpubench`'s `mesh_frame_in`).
    let first = if depth { 3 } else { 1 };
    mesh(first, (i32::from(w), i32::from(h)), tex, depth, &mut draws);
    for (s, d) in draws.iter().enumerate() {
        let _ = rr.set(&mut mem, s, d.as_ref());
    }
    let t = Target {
        picture: Picture::packed(0x20_0000, w, h, PixelFormat::Rgb565Dithered),
        x: 0,
        y: 0,
    };
    frame(&mut rr, &mut mem, &mut clock, t);
    let mut seed = 2;
    r.run(name, 4, &mut || {
        if moving {
            mesh(seed, (i32::from(w), i32::from(h)), tex, depth, &mut draws);
            seed += 1;
            for (s, d) in draws.iter().enumerate() {
                let _ = rr.set(&mut mem, s, d.as_ref());
            }
        } else {
            rr.invalidate_all();
        }
        frame(&mut rr, &mut mem, &mut clock, t);
    });
}

/// The shell's case: one 64 x 32 button's colour changed over the flat
/// mesh, its 16 tiles rendered.
#[inline(never)]
fn button(r: &mut Runner) {
    let mut pool = Pool::new(POOL);
    let Some(mut rr) = Renderer::new_in(&mut pool, 480, 640, PARAMS, PM_TABLE) else {
        r.check("r3d_button", false, "no renderer");
        return;
    };
    let (mut mem, mut clock) = (Sink(0), Tick(0));
    let mut draws = [None; 31];
    mesh(1, (480, 640), None, false, &mut draws);
    for (s, d) in draws.iter().enumerate() {
        let _ = rr.set(&mut mem, s, d.as_ref());
    }
    let t = Target {
        picture: Picture::packed(0x20_0000, 480, 640, PixelFormat::Rgb565Dithered),
        x: 0,
        y: 0,
    };
    frame(&mut rr, &mut mem, &mut clock, t);
    let mut c = 0u32;
    r.run("r3d_button", 20, &mut || {
        c = c.wrapping_add(0x0001_0203);
        let d = Draw::rect(200, 300, 64, 32, Paint::Flat(0xFF00_0000 | (c & 0xFF_FFFF)));
        let _ = rr.set(&mut mem, 40, Some(&d));
        frame(&mut rr, &mut mem, &mut clock, t);
    });
}

/// The renderer's CPU set-up a frame (R88; R78 item 3's object forms).
#[inline(never)]
pub fn render3d(r: &mut Runner) {
    let tex = Texture {
        at: 0x30_0000,
        code: 1,
        format: format::RGB565 as u8,
        u_mode: rp_mbx3d_render::address::WRAP as u8,
        v_mode: rp_mbx3d_render::address::WRAP as u8,
        linear: false,
        levels: 1,
    };
    let (vga, half) = ((480, 640), (480, 320));
    let (tris, quads) = (Objects::Triangles, Objects::Quads);
    let (rows, checker) = (TileOrder::Rows, TileOrder::Checker);
    let flat = (None, false);
    // name, size, order, form, (texture, depth), every quad moved
    #[rustfmt::skip]
    let runs = [
        ("r3d_still_vga_checker", vga, checker, tris, flat, false),
        ("r3d_still_vga_rows", vga, rows, tris, flat, false),
        ("r3d_still_vga_depth", vga, checker, tris, (None, true), false),
        ("r3d_still_half_checker", half, checker, tris, flat, false),
        ("r3d_moving_flat_tris", vga, checker, tris, flat, true),
        ("r3d_moving_flat_quads", vga, checker, quads, flat, true),
        ("r3d_moving_tex_tris", vga, checker, tris, (Some(tex), false), true),
        ("r3d_moving_tex_quads", vga, checker, quads, (Some(tex), false), true),
    ];
    for (name, size, order, form, paint, moving) in runs {
        one(r, name, (size, order, form), paint, moving);
    }
    bin_offscreen(r, tex);
    button(r);
}

/// Quads in the binner's scene (R78 item 4).
const BIN_QUADS: usize = 256;

/// The binner's scratch, in `.bss` (the stack holds a renderer already).
struct BinWork {
    starts: [u32; 2401],
    entries: [u16; 16 * BIN_QUADS],
    objects: [u32; BIN_QUADS],
}

static BIN_WORK: rt::Slot<BinWork> = rt::Slot::new();

/// A game's level larger than the screen (R78 item 4): [`BIN_QUADS`]
/// 32-pixel quads (flat and textured in turn) over a world four screens
/// wide and two high around the view, so about one in eight lands on the
/// screen; binned whole every frame, the view panned 4 pixels each time.
#[inline(never)]
pub fn bin_offscreen(r: &mut Runner, tex: Texture) {
    let Some(w) = BIN_WORK.put_with(|| BinWork {
        starts: [0; 2401],
        entries: [0; 16 * BIN_QUADS],
        objects: [0; BIN_QUADS],
    }) else {
        r.check("r3d_bin_offscreen", false, "scratch taken");
        return;
    };
    let l = binner::Layout {
        params: PARAMS.at,
        params_size: PARAMS.size,
        regions: 0x48_0000,
        w: 480,
        h: 640,
    };
    let mut mem = Sink(0);
    let mut pan = 0i32;
    let quad = |i: usize, pan: i32| {
        let h = hash(i as u32, 11, 5);
        let x = (h % (4 * 480)) as i32 - 3 * 480 / 2 - pan;
        let y = ((h >> 12) % (2 * 640)) as i32 - 640 / 2;
        let paint = if i % 2 == 0 {
            Paint::Flat(0xFF00_0000 | h)
        } else {
            Paint::texture(tex)
        };
        Draw::rect(x, y, 32, 32, paint)
    };
    r.run("r3d_bin_offscreen", 4, &mut || {
        pan = (pan + 4) % 480;
        let p = pan;
        let _ = binner::bin_objects(
            &mut mem,
            &l,
            Some(0xFF00_0000),
            (BIN_QUADS, |i| quad(i, p)),
            Scratch {
                starts: &mut w.starts,
                entries: &mut w.entries,
                objects: &mut w.objects,
            },
            Objects::Quads,
        );
    });
}
