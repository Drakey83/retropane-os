//! Run 4 (`RP_TABENCH_ISOLATE=1`, audit C79). Runs 2 and 3 (commits
//! d7036ba3, 98124f8e) found every TA scene of our quads wrong under
//! `rp-glesfeat`'s conditions too, with or without a core reset, the scene
//! inside its parameter memory: the 3D stage draws nothing from it. Run 4
//! bisects the stream instead, under the control's conditions with a reset
//! first: from `rp-glesfeat`'s G2 and M streams (right on the X51v) to
//! run 3's 100 quads, and the quads at z 1.0 (M's and G2's z) beside our
//! z 0.0. Each frame on a target the CPU filled with a sentinel, the first
//! listed tile's objects and sampled pixels logged against what they
//! should be.

use core::fmt::Write;

use rp_drv_blit2d::{MBX, reg, reset_core};
use rp_drv_mbx3d::{self as m3, ta};
use rp_hal_api::RegisterBus;
use rp_hal_mem::{bytes_as_pixels_mut, words_as_bytes_mut};
use rp_hal_pxa27x::{cpu, rt};
use rp_hal_x51v::devices::RunLog;
use rp_mbx3d_render::binner::{self, Scratch};
use rp_mbx3d_render::{LocalMem, Picture, PixelFormat, Target, Texture, format, upload};
use rp_vram_map::{POOL, Pool};

use super::{CLOCK, H, Rig, TA_LIMIT_US, W, draw, scatter, ta_quad, us_since};

macro_rules! log {
    ($($t:tt)*) => {{ let _ = writeln!(RunLog, $($t)*); }};
}

/// Where the TA and the 3D stage work.
#[derive(Clone, Copy)]
struct Places {
    pm: u32,
    pm_size: u32,
    pm_table: u32,
    tail: u32,
    ta_regions: u32,
}

/// `rp-glesfeat`'s (`OURS_LAYOUT`, where its TA scenes drew right).
const GLESFEAT: Places = Places {
    pm: 0x0060_0000,
    pm_size: 0x0010_0000,
    pm_table: 0x0070_0000,
    tail: 0x00A0_0000,
    ta_regions: 0x00A2_0000,
};

/// The binner's reference (clear of both: parameter memory 3-4 MiB, its
/// region array below the texture pool).
const REFERENCE: binner::Layout = binner::Layout {
    params: 0x0030_0000,
    params_size: 0x0010_0000,
    regions: 0x0078_0000,
    w: W as u16,
    h: H as u16,
};

/// The 32-bit target (`rp-glesfeat`'s format), clear of every place above.
const PICTURE32: Picture = Picture::packed(0x00B0_0000, W as u16, H as u16, PixelFormat::Argb8888);

const QUADS: usize = 100;
const SIDE: i32 = 16;

impl Rig {
    /// The memory manager set up at `p`; true when it said ready.
    fn pm_init_at(&mut self, p: Places) -> bool {
        let zeros = [0u32; 64];
        for k in 0..16 * 1024 / 256 {
            self.mem.store((p.pm_table + 256 * k) as usize, &zeros);
        }
        self.mem.sync();
        let en = self.regs.read32(MBX + reg::ENABLE);
        self.regs.write32(MBX + reg::ENABLE, en | m3::PM_READY);
        self.regs.write32(MBX + reg::CLEAR, m3::PM_READY);
        for (off, v) in m3::pm_init_table(p.pm, p.pm_size, p.pm_table) {
            self.regs.write32(MBX + off, v);
        }
        let t0 = rt::ticks();
        let mut ok = false;
        while us_since(t0) < 100_000 {
            if self.regs.read32(MBX + reg::STATUS) & m3::PM_READY != 0 {
                ok = true;
                break;
            }
        }
        self.regs.write32(MBX + reg::CLEAR, m3::PM_READY);
        self.regs.write32(MBX + reg::ENABLE, en);
        ok
    }

    /// The 3D stage on `l` into `t`: its time, 0 past 2 s (then the core
    /// reset and the manager set up again at `p`).
    fn render_at(&mut self, l: &binner::Layout, t: Target, p: Places) -> u32 {
        let en = self.regs.read32(MBX + reg::ENABLE);
        self.regs.write32(MBX + reg::CLEAR, m3::RENDER_DONE);
        self.regs.write32(MBX + reg::ENABLE, en | m3::RENDER_DONE);
        let t0 = rt::ticks();
        for (off, v) in binner::start_table(l, t) {
            self.regs.write32(MBX + off, v);
        }
        let mut us = 0;
        while us_since(t0) < 2_000_000 {
            if self.regs.read32(MBX + reg::STATUS) & m3::RENDER_DONE != 0 {
                us = us_since(t0).max(1);
                break;
            }
        }
        self.regs.write32(MBX + reg::CLEAR, m3::RENDER_DONE);
        self.regs.write32(MBX + reg::ENABLE, en);
        if us == 0 {
            self.render_hangs += 1;
            reset_core(&mut self.regs, rt::delay_us);
            let ok = self.pm_init_at(p);
            log!("    render past 2 s: core reset, memory manager ready again: {ok}");
        }
        us
    }

    /// The picture `pic` kept in SDRAM as the way's reference.
    fn keep_picture(&mut self, pic: Picture) {
        for k in 0..words_of(pic) {
            self.reference[k] = self.mem.load(pic.at as usize + 4 * k).unwrap_or(0);
        }
    }

    /// The pixel at `(x, y)` of `pic`, and the reference's.
    fn pixel(&self, pic: Picture, x: u32, y: u32) -> (u32, u32) {
        let (word, shift) = if pic.format == PixelFormat::Argb8888 {
            ((y * W + x) as usize, 0)
        } else {
            (((y * W + x) / 2) as usize, 16 * (x & 1))
        };
        let got = self.mem.load(pic.at as usize + 4 * word).unwrap_or(0);
        let want = self.reference[word];
        if shift == 0 && pic.format == PixelFormat::Argb8888 {
            (got, want)
        } else {
            ((got >> shift) & 0xFFFF, (want >> shift) & 0xFFFF)
        }
    }
}

fn words_of(pic: Picture) -> usize {
    (pic.stride * u32::from(pic.h) / 4) as usize
}

/// An 8 x 8 RGB565 texture in the same pattern as the 64 x 64 one.
fn small_texture(mem: &mut super::Vram, buf: &mut [u32]) -> Option<Texture> {
    let texels = bytes_as_pixels_mut(words_as_bytes_mut(&mut buf[..32]))?;
    for (i, t) in texels.iter_mut().enumerate() {
        let (x, y) = ((i % 8) as u16, (i / 8) as u16);
        let check = if (x + y) % 2 == 0 { 0x0010 } else { 0 };
        *t = (x << 2) << 11 | (y << 3) << 5 | check;
    }
    // After the 64 x 64 one in the pool (a pool of its own: the same
    // allocation order every run).
    let mut pool = Pool::new(POOL);
    let _ = pool.alloc(64 * 64 * 2, 128);
    upload(&mut pool, mem, 0, format::RGB565, texels)
}

/// Run 4's streams: from `rp-glesfeat`'s, which drew right on the X51v,
/// toward run 3's, which didn't.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stream {
    /// G2's (2026-10-23, right at Windows Mobile's clock): the clear and a
    /// full-screen Gouraud strip of four, untextured, z 1.0.
    G2,
    /// The GLES clear alone.
    ClearAlone,
    /// M's one-unit control (2026-10-22, right): one textured triangle,
    /// Dell's texture words with format code and combine as M's, z 1.0,
    /// no clear.
    MTriangle,
    /// `n` of our quads (z 0.0, our texture words), with the clear or not.
    Quads { n: usize, clear: bool, z1: bool },
}

struct Way4 {
    name: &'static str,
    stream: Stream,
}

const WAYS4: [Way4; 8] = [
    Way4 {
        name: "G2'S STREAM",
        stream: Stream::G2,
    },
    Way4 {
        name: "THE CLEAR ALONE",
        stream: Stream::ClearAlone,
    },
    Way4 {
        name: "M'S TRIANGLE",
        stream: Stream::MTriangle,
    },
    Way4 {
        name: "ONE QUAD, NO CLEAR",
        stream: Stream::Quads {
            n: 1,
            clear: false,
            z1: false,
        },
    },
    Way4 {
        name: "ONE QUAD AND THE CLEAR",
        stream: Stream::Quads {
            n: 1,
            clear: true,
            z1: false,
        },
    },
    Way4 {
        name: "100 QUADS, NO CLEAR",
        stream: Stream::Quads {
            n: QUADS,
            clear: false,
            z1: false,
        },
    },
    Way4 {
        name: "100 QUADS AND THE CLEAR",
        stream: Stream::Quads {
            n: QUADS,
            clear: true,
            z1: false,
        },
    },
    Way4 {
        name: "100 QUADS AT Z 1.0, CLEAR",
        stream: Stream::Quads {
            n: QUADS,
            clear: true,
            z1: true,
        },
    },
];

/// What the CPU fills the target with before each frame: no picture here
/// has this colour, so a pixel left is one the render never wrote.
const SENTINEL: u32 = 0x5A5A_5A5A;
const FRAMES4: usize = 3;

/// G2's strip: the untextured state block (mask `0x10000053`: TSP, ISP,
/// clip, the bit 6 word), the strip word, four corners of x, y, z 1.0,
/// 1/w 1.0 and a colour (top left, top right, bottom left, bottom right).
fn g2_strip(clip: u32) -> [u32; 26] {
    const CORNERS: [(f32, f32, u32); 4] = [
        (0.0, 0.0, 0xFF00_0000),
        (480.0, 0.0, 0xFFFF_FFFF),
        (0.0, 640.0, 0xFFFF_E000),
        (480.0, 640.0, 0xFF10_20A0),
    ];
    let mut s = [0u32; 26];
    s[..6].copy_from_slice(&[0x1000_0053, 0x2260_0E80, m3::ISP_ALWAYS, clip, 0, ta::STRIP]);
    let one = 1.0f32.to_bits();
    for (k, &(x, y, c)) in CORNERS.iter().enumerate() {
        s[6 + 5 * k..11 + 5 * k].copy_from_slice(&[x.to_bits(), y.to_bits(), one, one, c]);
    }
    s
}

/// M's one-unit control: the textured state block with Dell's texture
/// words (format code as given, M's combine `0x86084610`), one triangle
/// (100, 100), (380, 100), (100, 540) of z 1.0.
fn m_triangle(tex_at: u32, clip: u32) -> [u32; 9 + 3 * ta::TEX_VERTEX_WORDS] {
    let mut t = ta::dell_texture(tex_at);
    t[0] = m3::tex::tex0(0, format::RGB565);
    t[2] = 0x8608_4610;
    let v = |x: f32, y: f32, u: f32, w: f32| m3::tex::Vertex {
        x,
        y,
        z: 1.0,
        rhw: 1.0,
        argb: 0xFFFF_FFFF,
        u,
        v: w,
    };
    ta::textured_stream(
        ta::TSP_DELL_TEXTURED,
        m3::ISP_ALWAYS,
        t,
        clip,
        &[
            v(100.0, 100.0, 0.0, 0.0),
            v(380.0, 100.0, 1.0, 0.0),
            v(100.0, 540.0, 0.0, 1.0),
        ],
    )
}

impl Rig {
    /// The first listed tile's first two objects (16 words each), logged.
    fn log_objects(&self, p: Places) {
        let tiles = (W / m3::TILE_W * (H / m3::TILE_H)) as usize;
        let at = |k: usize| self.mem.load(p.ta_regions as usize + 8 * k).unwrap_or(0);
        let ptr = |k: usize| {
            self.mem
                .load(p.ta_regions as usize + 8 * k + 4)
                .unwrap_or(0)
        };
        let listed = (0..tiles).filter(|&k| at(k) & 1 << 25 == 0).count();
        log!("    TA region array: {listed} of {tiles} tiles with a list");
        let Some(k) = (0..tiles).find(|&k| at(k) & 1 << 25 == 0) else {
            return;
        };
        let list = (ptr(k) & m3::ADDR_MASK) as usize;
        let _ = write!(
            RunLog,
            "    tile {k} (header {:08x}), list at 0x{list:x}:",
            at(k)
        );
        let mut words = [m3::END; 3];
        for (j, w) in words.iter_mut().enumerate() {
            *w = self.mem.load(list + 4 * j).unwrap_or(m3::END);
            let _ = write!(RunLog, " {w:08x}");
            if *w == m3::END {
                break;
            }
        }
        log!("");
        for &w in words.iter().take_while(|&&w| w != m3::END).take(2) {
            let obj = p.pm as usize + 4 * (w & 0x001F_FFFF) as usize;
            let _ = write!(RunLog, "      object {w:08x} at 0x{obj:x}:");
            for j in 0..16 {
                let _ = write!(RunLog, " {:08x}", self.mem.load(obj + 4 * j).unwrap_or(0));
            }
            log!("");
        }
    }

    /// The 32-bit target filled with [`SENTINEL`] by the CPU.
    fn fill_sentinel(&mut self) {
        let row = [SENTINEL; 64];
        for k in 0..words_of(PICTURE32) / 64 {
            self.mem.store(PICTURE32.at as usize + 256 * k, &row);
        }
        self.mem.sync();
    }
}

/// Run 4 (`RP_TABENCH_ISOLATE=1`). Returns (frames whose sampled pixels
/// all match what they should, frames in all).
pub(super) fn run4(g: &mut Rig) -> (u32, u32) {
    let Some(small) = small_texture(&mut g.mem, g.staging) else {
        log!("  no room for the 8 x 8 texture");
        return (0, 0);
    };
    scatter(g.xy, QUADS, SIDE);
    g.side = SIDE;
    let p = GLESFEAT;
    let target = Target {
        picture: PICTURE32,
        x: 0,
        y: 0,
    };
    // The binner's picture of the 100 quads on black: what each quad's
    // middle should show.
    let ref_places = Places {
        pm: REFERENCE.params,
        pm_size: REFERENCE.params_size,
        ..p
    };
    reset_core(&mut g.regs, rt::delay_us);
    let _ = g.pm_init_at(ref_places);
    let xy = &*g.xy;
    let binned = binner::bin_with(
        &mut g.mem,
        &REFERENCE,
        Some(0),
        QUADS,
        |k| draw(xy[k], SIDE, small),
        Scratch {
            starts: &mut *g.starts,
            entries: &mut *g.entries,
            objects: &mut *g.objects,
        },
    );
    let us = if binned.is_ok() {
        g.render_at(&REFERENCE, target, ref_places)
    } else {
        0
    };
    log!("  the binner's reference: rendered in {us} us");
    g.keep_picture(PICTURE32);
    let ta_layout = ta::Layout {
        tail: p.tail,
        tail_bytes: 0x1_0000,
        regions: p.ta_regions,
        width: W,
        height: H,
    };
    let from_ta = binner::Layout {
        params: p.pm,
        params_size: p.pm_size,
        regions: p.ta_regions,
        w: W as u16,
        h: H as u16,
    };
    let clip = ta::clip(W, H);
    let clear = ta::clear_stream(W, H);
    let strip = g2_strip(clip);
    let tri = m_triangle(small.at, clip);
    let state = ta::State {
        tsp: ta::TSP_DELL_TEXTURED,
        isp: m3::ISP_ALWAYS,
        tex: small.words(),
        clip,
    };
    let (mut right, mut all) = (0, 0);
    for (n, way) in WAYS4.iter().enumerate() {
        super::step_line(g, n + 1, WAYS4.len(), way.name);
        log!("WAY {} of {}: {}", n + 1, WAYS4.len(), way.name);
        reset_core(&mut g.regs, rt::delay_us);
        let ready = g.pm_init_at(p);
        for k in 0..FRAMES4 {
            rt::check_in();
            g.fill_sentinel();
            let _ = ta::clear(g.mem.0, &ta_layout);
            cpu::drain_write_buffer();
            let xy = &*g.xy;
            let mut feed = ta::Feed::start(&mut g.regs, &ta_layout, CLOCK, TA_LIMIT_US);
            let ok = match way.stream {
                Stream::G2 => ta::draws(&[&clear, &strip]).all(|i| feed.put(i)),
                Stream::ClearAlone => ta::draws(&[&clear]).all(|i| feed.put(i)),
                Stream::MTriangle => ta::draws(&[&tri]).all(|i| feed.put(i)),
                Stream::Quads { n, clear: c, z1 } => {
                    let quad = |q: usize| {
                        let mut t = ta_quad(xy[q], SIDE);
                        if z1 {
                            t.z = 1.0;
                        }
                        t
                    };
                    let head: &[&[u32]] = if c { &[&clear] } else { &[] };
                    ta::draws(head)
                        .chain(ta::quads_with(state, ta::Shape::DELL, n, quad))
                        .all(|i| feed.put(i))
                }
            };
            let fed = feed.finish();
            all += 1;
            if !(ok && fed.done) {
                g.ta_hangs += 1;
                log!("    frame {}: the TA never said done", k + 1);
                continue;
            }
            if k == 0 {
                g.log_objects(p);
            }
            let r = g.render_at(&from_ta, target, p);
            let px = |g: &Rig, x: u32, y: u32| g.pixel(PICTURE32, x, y);
            // What the right picture shows at each sample: (x, y, colour
            // or None for "anything the render wrote").
            let mut samples = [(0u32, 0u32, None::<u32>); 6];
            match way.stream {
                Stream::G2 => {
                    samples[0] = (240, 320, None);
                    samples[1] = (2, 2, None);
                    samples[2] = (477, 637, None);
                }
                Stream::ClearAlone => {
                    samples[0] = (240, 320, Some(0));
                    samples[1] = (2, 2, Some(0));
                    samples[2] = (477, 637, Some(0));
                }
                Stream::MTriangle => {
                    samples[0] = (150, 200, None);
                    samples[1] = (400, 600, Some(SENTINEL));
                    samples[2] = (2, 2, Some(SENTINEL));
                }
                Stream::Quads { n, clear: c, .. } => {
                    // Quad middles: the binner's colour there (with one
                    // quad, only the first; the others are background:
                    // black under the clear, unknown without it).
                    let bg = c.then_some(0);
                    for (j, (x, y)) in super::isolate_samples(&*g.xy).iter().take(3).enumerate() {
                        let want = if n == 1 && j > 0 {
                            bg
                        } else {
                            Some(g.pixel(PICTURE32, *x, *y).1)
                        };
                        samples[j] = (*x, *y, want);
                    }
                    samples[3] = (2, 2, bg);
                }
            }
            let mut good = r != 0;
            let _ = write!(
                RunLog,
                "    frame {}: render {r} us; pixels (got / want):",
                k + 1
            );
            for &(x, y, want) in samples.iter().filter(|s| s.0 != 0 || s.1 != 0) {
                let (got, _) = px(g, x, y);
                let ok = match want {
                    Some(w) => (got ^ w) & 0x00FF_FFFF == 0,
                    None => got != SENTINEL,
                };
                good &= ok;
                match want {
                    Some(w) => {
                        let _ = write!(RunLog, " ({x},{y}) {got:08x}/{w:08x}");
                    }
                    None => {
                        let _ = write!(RunLog, " ({x},{y}) {got:08x}/written");
                    }
                }
            }
            log!("{}", if good { " RIGHT" } else { " WRONG" });
            if good {
                right += 1;
            }
        }
        log!("  {}: memory manager ready {ready}", way.name);
    }
    (right, all)
}
