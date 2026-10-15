//! A model of the 2700G's MBX 3D core, as measured on the X51v (`rp-mbx3d`
//! versions 3-23, ADR-0024/0026), for host tests: it reads the region
//! array, object lists and objects from memory exactly where the start
//! table says, and renders each listed tile into the target.
//!
//! Modelled (measured): 8 x 16 tiles written whole, uncovered pixels white;
//! unlisted tiles untouched; the target `0x65C` at any pixel offset with
//! the stride `0x660`; the flat (17-word) and textured (26-word) one-
//! triangle objects behind strip pointers `0x40800000` / `0x41000000`; flat
//! shading from the first vertex, Gouraud with TSP bit 22; twiddled point-
//! sampled textures in the four decoded formats, wrap/mirror/clamp from
//! TEX0 bits 31-28; blending with TSP bit 13 and the factors in bits 31-26
//! (each term `value x factor >> 8`, alpha factor `a` and `256 - a`: fits
//! the device's `0xBF7F007F` and `0xBF770087` exactly); depth compare and
//! write in TSP bits 11-9 and 7 (a tile's depth undefined until written
//! while `0x6DC` bit 0 is clear: run 1), larger z nearer ("greater", "greater or
//! equal", "always"; "less" and "less or equal" never passed, even where
//! they should have: kept as measured), each tile starting at `0x6DC`; render done (status bit 2) after ~16 us + 5.2 us a tile; the
//! parameter memory manager ready (bit 6) 164 us after `0x838`, only with
//! the 3D clock on; TSP bits 16 and 19 hang the core until a reset.
//!
//! Assumed (not measured; listed in docs/hw/emulator.md): pixel centres at
//! +0.5 with a top-left fill rule (each pixel of a shared edge drawn once);
//! Gouraud and modulate rounding; 5/6-bit texel channels widened by
//! repeating their top bits; both windings drawn; `0x654`/`0x658` not
//! clipping; the core's state kept across the 3D clock off is measured
//! (T9). Anything else in the words panics, so a test sees the
//! renderer ask for something no device run has shown.

extern crate std;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::vec::Vec;

use rp_drv_blit2d::{MBX, reg};
use rp_drv_mbx_display::clock::MBXCLK;
use rp_drv_mbx3d::{self as m3, tex};
use rp_hal_api::RegisterBus;

use crate::{Clock, LocalMem, predict_us, quad_cost};

/// 2700G local memory, shared by the model and the renderer.
pub type Shared = Rc<RefCell<Vec<u8>>>;

/// 16 MiB of zeroed local memory.
#[must_use]
pub fn memory() -> Shared {
    Rc::new(RefCell::new(std::vec![0; 16 << 20]))
}

/// The renderer's view of [`Shared`] memory; counts the words stored.
pub struct Mem(pub Shared, pub u32);

impl Mem {
    /// A view of `m`.
    #[must_use]
    pub fn new(m: Shared) -> Self {
        Self(m, 0)
    }

    /// The 32-bit pixel at `(x, y)` of a picture at `at`, `stride` bytes a
    /// line.
    #[must_use]
    pub fn px(&self, at: u32, stride: u32, x: u32, y: u32) -> u32 {
        self.load((at + y * stride + 4 * x) as usize)
            .unwrap_or(0xDEAD_BEEF)
    }

    /// Fills a `w` x `h` rectangle of a picture with `c` (as the 2D engine
    /// would).
    pub fn fill(&mut self, at: u32, stride: u32, (x, y, w, h): (u32, u32, u32, u32), c: u32) {
        let mut m = self.0.borrow_mut();
        for yy in y..y + h {
            for xx in x..x + w {
                let a = (at + yy * stride + 4 * xx) as usize;
                m[a..a + 4].copy_from_slice(&c.to_le_bytes());
            }
        }
    }
}

impl LocalMem for Mem {
    fn store(&mut self, at: usize, words: &[u32]) {
        let mut m = self.0.borrow_mut();
        for (k, w) in words.iter().enumerate() {
            m[at + 4 * k..at + 4 * k + 4].copy_from_slice(&w.to_le_bytes());
        }
        self.1 += words.len() as u32;
    }

    fn load(&self, at: usize) -> Option<u32> {
        let m = self.0.borrow();
        Some(u32::from_le_bytes(m.get(at..at + 4)?.try_into().ok()?))
    }

    fn sync(&mut self) {}
}

/// A clock that moves only when waited on or told to.
#[derive(Clone, Default)]
pub struct SimClock(pub Rc<Cell<u32>>);

impl SimClock {
    /// Moves time on by `us`.
    pub fn advance(&self, us: u32) {
        self.0.set(self.0.get().wrapping_add(us));
    }

    /// Moves time to `t`.
    pub fn set(&self, t: u32) {
        self.0.set(t);
    }
}

impl Clock for SimClock {
    fn now_us(&mut self) -> u32 {
        self.0.get()
    }

    fn delay_us(&mut self, us: u32) {
        self.advance(us);
    }
}

/// The register window with the 3D core behind it.
pub struct Gpu {
    mem: Shared,
    now: Rc<Cell<u32>>,
    regs: BTreeMap<usize, u32>,
    status: u32,
    done_at: Option<u32>,
    ready_at: Option<u32>,
    pm_set: bool,
    hung: bool,
    /// Every write, in order.
    pub writes: Vec<(usize, u32)>,
    /// Status reads.
    pub status_reads: u32,
    /// The next render hangs (as TSP bit 16 did on the device).
    pub hang_next: bool,
    /// Renders started.
    pub renders: u32,
    /// Tiles the last render covered.
    pub tiles: u32,
    /// Render-done edges on the interrupt line (GPIO85), counted when a
    /// status read or clear notices the render finished.
    pub irqs: u32,
}

const RENDER_DONE: u32 = m3::RENDER_DONE;
const PM_READY: u32 = m3::PM_READY;

impl Gpu {
    /// A core over `mem`, on the clock `clock`; `MBXCLK` as Windows Mobile
    /// leaves it (2D only).
    #[must_use]
    pub fn new(mem: Shared, clock: &SimClock) -> Self {
        let mut regs = BTreeMap::new();
        regs.insert(MBXCLK, 0x5);
        Self {
            mem,
            now: clock.0.clone(),
            regs,
            status: 0,
            done_at: None,
            ready_at: None,
            pm_set: false,
            hung: false,
            writes: Vec::new(),
            status_reads: 0,
            hang_next: false,
            renders: 0,
            tiles: 0,
            irqs: 0,
        }
    }

    /// An MBX register's last written value.
    #[must_use]
    pub fn reg(&self, off: usize) -> u32 {
        self.regs.get(&(MBX + off)).copied().unwrap_or(0)
    }

    /// Whether the core is hung (until a reset).
    #[must_use]
    pub fn hung(&self) -> bool {
        self.hung
    }

    fn passed(&self, t: u32) -> bool {
        self.now.get().wrapping_sub(t) < 1 << 31
    }

    fn tick(&mut self) {
        if self.done_at.is_some_and(|t| self.passed(t)) {
            self.done_at = None;
            self.status |= RENDER_DONE;
            // An edge on GPIO85 with the MBX core's master and render-done
            // enables and MINTENABLE's master and MBX bits (version 24).
            let both = 0x8000 | RENDER_DONE;
            let en = self.regs.get(&(MBX + reg::ENABLE)).copied().unwrap_or(0);
            let mint = self.regs.get(&crate::MINTENABLE).copied().unwrap_or(0);
            if en & both == both && mint & 0x8004 == 0x8004 {
                self.irqs += 1;
            }
        }
        if self.ready_at.is_some_and(|t| self.passed(t)) {
            self.ready_at = None;
            self.status |= PM_READY;
        }
    }

    fn clock_3d(&self) -> bool {
        self.regs.get(&MBXCLK).is_some_and(|&v| v & 3 == 2)
    }

    fn word(m: &[u8], a: u32) -> u32 {
        let a = a as usize;
        u32::from_le_bytes(m[a..a + 4].try_into().expect("4 bytes"))
    }

    /// Renders the region array: returns the tiles rendered and the
    /// objects drawn beyond the first in each (stops early if an object
    /// hangs the core).
    fn render(&mut self) -> (u32, u32) {
        let mem = self.mem.clone();
        let mut m = mem.borrow_mut();
        let regions = self.reg(0x608) & m3::ADDR_MASK;
        let params = self.reg(0x60C);
        let target = self.reg(0x65C);
        let stride = self.reg(0x660) * 4;
        // Bit 0 clear: the tiles' depth isn't cleared (`rp-mbx3d-api` run 1:
        // "greater" failed wherever nothing had written depth), so it starts
        // undefined: no compare passes but "always".
        let r6dc = self.reg(0x6DC);
        let clear_z = if r6dc & 1 != 0 {
            f64::from(f32::from_bits(r6dc & !1))
        } else {
            f64::NAN
        };
        let mut quads = 0u32;
        let mut n = 0;
        for k in 0..4096 {
            let w0 = Self::word(&m, regions + 8 * k);
            let list = Self::word(&m, regions + 8 * k + 4) & m3::ADDR_MASK;
            let (tx, ty) = (w0 & 127, (w0 >> 8) & 127);
            let mut tile = Tile {
                x: tx * m3::TILE_W,
                y: ty * m3::TILE_H,
                c: [0xFFFF_FFFF; 128],
                z: [clear_z; 128],
            };
            for j in 0..512 {
                let p = Self::word(&m, list + 4 * j);
                if p >> 28 == 0xF {
                    break;
                }
                let at = params + (p & 0x001F_FFFF) * 4;
                let o = match p & 0xFFE0_0000 {
                    m3::STRIP_ONE_TRIANGLE => Obj::flat(&m, at),
                    tex::STRIP_GAME => Obj::textured(&m, at),
                    f => panic!("strip pointer flags {f:#010x} never measured"),
                };
                // TEX1 bit 30 without bit 29 (mipmap "linear", no levels)
                // hung the core too (version 24).
                let mip_hang = o.tex.is_some_and(|t| t[1] & 0x6000_0000 == 0x4000_0000);
                if o.tsp & (1 << 16 | 1 << 19) != 0 || mip_hang {
                    // TSP bits 16 and 19 hung the device's core.
                    self.hung = true;
                    return (n, 0);
                }
                o.draw(&m, &mut tile);
                // Time: every object but the tile's first (the
                // background), each half a quad (run 2's fit).
                if j > 0 {
                    quads += quad_cost(o.tex.is_some(), o.tsp & m3::TSP_BLEND != 0);
                }
            }
            for py in 0..m3::TILE_H {
                for px in 0..m3::TILE_W {
                    let a = (target + (tile.y + py) * stride + 4 * (tile.x + px)) as usize;
                    let c = tile.c[(py * m3::TILE_W + px) as usize];
                    m[a..a + 4].copy_from_slice(&c.to_le_bytes());
                }
            }
            n += 1;
            if w0 & m3::LAST_REGION != 0 {
                return (n, quads / 2);
            }
        }
        panic!("region array without a last entry");
    }
}

impl RegisterBus for Gpu {
    fn read32(&mut self, off: usize) -> u32 {
        if off == MBX + reg::STATUS {
            self.status_reads += 1;
            self.tick();
            return self.status;
        }
        self.regs.get(&off).copied().unwrap_or(0)
    }

    fn write32(&mut self, off: usize, v: u32) {
        self.writes.push((off, v));
        self.regs.insert(off, v);
        match off {
            o if o == MBX + reg::CLEAR => {
                self.tick();
                self.status &= !v;
            }
            o if o == MBX + reg::RESET && v == 0x111 => {
                self.status = 0;
                self.done_at = None;
                self.ready_at = None;
                self.pm_set = false;
                self.hung = false;
            }
            o if o == MBX + 0x838 && v == 1 && self.clock_3d() => {
                self.pm_set = true;
                self.ready_at = Some(self.now.get().wrapping_add(164));
            }
            o if o == MBX + m3::START && v == 1 => {
                // Nothing happens without the clock and the memory manager
                // (version 1), or once hung (version 21).
                if !self.clock_3d() || !self.pm_set || self.hung {
                    return;
                }
                self.renders += 1;
                if core::mem::take(&mut self.hang_next) {
                    self.hung = true;
                    return;
                }
                let (n, quads) = self.render();
                self.tiles = n;
                if !self.hung {
                    self.done_at = Some(self.now.get().wrapping_add(predict_us(n, quads)));
                }
            }
            _ => {}
        }
    }
}

struct Tile {
    x: u32,
    y: u32,
    c: [u32; 128],
    z: [f64; 128],
}

#[derive(Clone, Copy)]
struct V {
    x: f64,
    y: f64,
    z: f64,
    argb: u32,
    u: f64,
    v: f64,
}

struct Obj {
    tsp: u32,
    tex: Option<[u32; 3]>,
    v: [V; 3],
}

fn f(w: u32) -> f64 {
    f64::from(f32::from_bits(w))
}

impl Obj {
    fn flat(m: &[u8], at: u32) -> Self {
        let w = |k: u32| Gpu::word(m, at + 4 * k);
        let v = [0, 1, 2].map(|k| V {
            x: f(w(2 + 2 * k)),
            y: f(w(3 + 2 * k)),
            z: f(w(8 + k)),
            argb: w(14 + k),
            u: 0.0,
            v: 0.0,
        });
        Self {
            tsp: w(1),
            tex: None,
            v,
        }
    }

    fn textured(m: &[u8], at: u32) -> Self {
        let w = |k: u32| Gpu::word(m, at + 4 * k);
        let v = [0, 1, 2].map(|k| V {
            x: f(w(5 + 2 * k)),
            y: f(w(6 + 2 * k)),
            z: f(w(11 + k)),
            argb: w(17 + 3 * k),
            u: f(w(18 + 3 * k)),
            v: f(w(19 + 3 * k)),
        });
        Self {
            tsp: w(4),
            tex: Some([w(1), w(2), w(3)]),
            v,
        }
    }

    fn draw(&self, m: &[u8], t: &mut Tile) {
        let e = |a: V, b: V, x: f64, y: f64| (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x);
        let v = self.v;
        let area = e(v[0], v[1], v[2].x, v[2].y);
        if area == 0.0 {
            return;
        }
        let s = area.signum();
        for py in 0..m3::TILE_H {
            for px in 0..m3::TILE_W {
                let (x, y) = (f64::from(t.x + px) + 0.5, f64::from(t.y + py) + 0.5);
                let mut w = [0.0; 3];
                let mut inside = true;
                for i in 0..3 {
                    let (a, b) = (v[(i + 1) % 3], v[(i + 2) % 3]);
                    let ei = e(a, b, x, y) * s;
                    // The edge as the normalised winding runs it.
                    let (dx, dy) = ((b.x - a.x) * s, (b.y - a.y) * s);
                    let tie = dy > 0.0 || (dy == 0.0 && dx < 0.0);
                    inside &= ei > 0.0 || (ei == 0.0 && tie);
                    w[i] = ei * s / area;
                }
                if !inside {
                    continue;
                }
                let k = (py * m3::TILE_W + px) as usize;
                let z = w[0] * v[0].z + w[1] * v[1].z + w[2] * v[2].z;
                let mode = (self.tsp >> 9) & 7;
                let pass = match mode {
                    // "Less" and "less or equal" never passed on the device
                    // and, in partial renders, hid an "always" object below
                    // (run 3, S8): the renderer refuses them.
                    1 | 3 => panic!("depth mode {mode}: broken on the device (ADR-0026)"),
                    4 => z > t.z[k],
                    6 => z >= t.z[k],
                    7 => true,
                    m => panic!("depth mode {m} never measured"),
                };
                if !pass {
                    continue;
                }
                let mut src = if self.tsp & m3::TSP_GOURAUD != 0 {
                    let ch = |sh: u32| {
                        let c = (0..3)
                            .map(|i| w[i] * f64::from((v[i].argb >> sh) & 0xFF))
                            .sum::<f64>();
                        ((c + 0.5).floor().clamp(0.0, 255.0) as u32) << sh
                    };
                    ch(24) | ch(16) | ch(8) | ch(0)
                } else {
                    v[0].argb
                };
                if let Some(tw) = self.tex {
                    // Filtered textures sample at the pixel's corner, not
                    // its centre (version 24: the checker's ramp 0x3F,
                    // 0x7F, 0xBF fits only that).
                    let w = if tw[1] & tex::TEX1_LINEAR != 0 {
                        let (cx, cy) = (x - 0.5, y - 0.5);
                        [0, 1, 2].map(|i| {
                            let (a, b) = (v[(i + 1) % 3], v[(i + 2) % 3]);
                            e(a, b, cx, cy) / area
                        })
                    } else {
                        w
                    };
                    let u = w[0] * v[0].u + w[1] * v[1].u + w[2] * v[2].u;
                    let vv = w[0] * v[0].v + w[1] * v[1].v + w[2] * v[2].v;
                    // TEX2 bit 13 (Dell's opaque combine word) makes the
                    // texel's alpha 1 (version 23: texture alpha blended
                    // only with it clear; version 24: a 32-bit texel at
                    // alpha 0x80 and a black PVRTC block drew alpha 0xFF).
                    let level = mip_level(
                        tw,
                        |px, py| {
                            let w = [0, 1, 2].map(|i| {
                                let (a, b) = (v[(i + 1) % 3], v[(i + 2) % 3]);
                                e(a, b, px, py) / area
                            });
                            (
                                w[0] * v[0].u + w[1] * v[1].u + w[2] * v[2].u,
                                w[0] * v[0].v + w[1] * v[1].v + w[2] * v[2].v,
                            )
                        },
                        x,
                        y,
                    );
                    let mut t = sample(m, tw, u, vv, level);
                    if tw[2] & (1 << 13) != 0 {
                        t |= 0xFF00_0000;
                    }
                    src = modulate(t, src);
                }
                t.c[k] = blend(self.tsp, src, t.c[k]);
                if self.tsp & (1 << 7) == 0 {
                    t.z[k] = z;
                }
            }
        }
    }
}

/// The texture's colour at `(u, v)` (0 to 1 across it), ARGB8888: point
/// sampled, or bilinear with TEX1 bit 31 (version 24: weights from the
/// texel centres, each channel truncated).
fn sample(m: &[u8], tw: [u32; 3], u: f64, v: f64, level: u32) -> u32 {
    let tex0 = tw[0];
    let side_u = (8i64 << ((tex0 >> 24) & 15)) >> level;
    let side_v = (8i64 << ((tex0 >> 20) & 15)) >> level;
    let wrap = |i: i64, side: i64, mode: u32| match mode {
        tex::WRAP => i.rem_euclid(side),
        tex::MIRROR => {
            let p = i.rem_euclid(2 * side);
            if p >= side { 2 * side - 1 - p } else { p }
        }
        tex::CLAMP => i.clamp(0, side - 1),
        _ => panic!("address mode 3 broke sampling on the device"),
    };
    let (mu, mv) = (tex0 >> 30, (tex0 >> 28) & 3);
    let at = |x: i64, y: i64| {
        texel(
            m,
            tw,
            level,
            wrap(x, side_u, mu) as u32,
            wrap(y, side_v, mv) as u32,
        )
    };
    if tw[1] & tex::TEX1_LINEAR == 0 {
        let x = (u * side_u as f64).floor() as i64;
        let y = (v * side_v as f64).floor() as i64;
        return at(x, y);
    }
    let (tu, tv) = (u * side_u as f64 - 0.5, v * side_v as f64 - 0.5);
    let (x0, y0) = (tu.floor() as i64, tv.floor() as i64);
    let (fx, fy) = (tu - x0 as f64, tv - y0 as f64);
    let q = [
        at(x0, y0),
        at(x0 + 1, y0),
        at(x0, y0 + 1),
        at(x0 + 1, y0 + 1),
    ];
    (0..4)
        .map(|i| {
            let sh = 8 * i;
            let c = |k: usize| f64::from((q[k] >> sh) & 0xFF);
            let top = c(0) * (1.0 - fx) + c(1) * fx;
            let bottom = c(2) * (1.0 - fx) + c(3) * fx;
            ((top * (1.0 - fy) + bottom * fy).floor() as u32).min(255) << sh
        })
        .fold(0, |a, b| a | b)
}

/// The mip level a pixel at `(x, y)` samples: with TEX1 bits 30-29 = 1
/// (the nearest level), the one whose texels match the pixels (the larger
/// of the steps in texels across and down: `rp-mbx3d` version 25, a 64 x
/// 64 texture over 16 and 8 px drew levels 2 and 3), up to TEX0 bits 12-9
/// (0: the base only, version 24). Between powers of two the nearest in
/// log2 is assumed. `uv` gives the texture coordinates at a point.
fn mip_level(tw: [u32; 3], uv: impl Fn(f64, f64) -> (f64, f64), x: f64, y: f64) -> u32 {
    if tw[1] & 0x6000_0000 != tex::TEX1_MIP_NEAREST {
        return 0;
    }
    let size = f64::from(8u32 << ((tw[0] >> 24) & 15));
    let (u0, v0) = uv(x, y);
    let (u1, v1) = uv(x + 1.0, y);
    let (u2, v2) = uv(x, y + 1.0);
    let step = ((u1 - u0).hypot(v1 - v0)).max((u2 - u0).hypot(v2 - v0)) * size;
    let top = ((tw[0] >> 9) & 15).min(((tw[0] >> 24) & 15) + 3);
    if step <= 1.0 {
        return 0;
    }
    (step.log2().round() as u32).min(top)
}

/// Texel `(x, y)` of mip level `level` of the texture, ARGB8888 (levels
/// after the base, each after the larger one: version 25).
fn texel(m: &[u8], [tex0, tex1, _]: [u32; 3], level: u32, x: u32, y: u32) -> u32 {
    let side = 8usize << ((tex0 >> 24) & 15);
    let texel_bytes = if (tex0 >> tex::FORMAT_SHIFT) & 0x1F == tex::ARGB8888 {
        4
    } else {
        2
    };
    let base = ((tex1 & 0x01FF_FFFF) << 7) as usize
        + (0..level as usize)
            .map(|k| (side >> k) * (side >> k) * texel_bytes)
            .sum::<usize>();
    let word = |a: usize| u32::from_le_bytes([m[a], m[a + 1], m[a + 2], m[a + 3]]);
    let wide = |c: u32, bits: u32| match bits {
        5 => c << 3 | c >> 2,
        6 => c << 2 | c >> 4,
        4 => c * 17,
        3 => c << 5 | c << 2 | c >> 1,
        2 => c * 85,
        _ => unreachable!(),
    };
    let argb = |a: u32, r: u32, g: u32, b: u32| a << 24 | r << 16 | g << 8 | b;
    let format = (tex0 >> tex::FORMAT_SHIFT) & 0x1F;
    match format {
        tex::ARGB8888 => return word(base + 4 * tex::twiddle(x, y) as usize),
        tex::PVRTC4 | tex::PVRTC2 => {
            // Approximate (exact for solid blocks, as version 24 drew
            // them): the block's colour A, modulation and the blending
            // between neighbouring blocks left out. Blocks in twiddled
            // order (PVRTC's; version 24 couldn't tell it from row order).
            let bw = if format == tex::PVRTC4 { 4 } else { 8 };
            let b = tex::twiddle(x / bw, y / 4) as usize;
            let c = word(base + 8 * b + 4) & 0xFFFF;
            return if c & 0x8000 != 0 {
                argb(
                    255,
                    wide((c >> 10) & 31, 5),
                    wide((c >> 5) & 31, 5),
                    wide((c >> 1) & 15, 4),
                )
            } else {
                argb(
                    wide((c >> 12) & 7, 3),
                    wide((c >> 8) & 15, 4),
                    wide((c >> 4) & 15, 4),
                    wide((c >> 1) & 7, 3),
                )
            };
        }
        _ => {}
    }
    let a = base + 2 * tex::twiddle(x, y) as usize;
    let t = u32::from(u16::from_le_bytes([m[a], m[a + 1]]));
    match format {
        tex::ARGB1555 => argb(
            if t & 0x8000 != 0 { 255 } else { 0 },
            wide((t >> 10) & 31, 5),
            wide((t >> 5) & 31, 5),
            wide(t & 31, 5),
        ),
        tex::RGB565 => argb(
            255,
            wide(t >> 11, 5),
            wide((t >> 5) & 63, 6),
            wide(t & 31, 5),
        ),
        tex::ARGB4444 => argb(
            wide(t >> 12, 4),
            wide((t >> 8) & 15, 4),
            wide((t >> 4) & 15, 4),
            wide(t & 15, 4),
        ),
        tex::RGB332 => argb(
            255,
            wide((t >> 5) & 7, 3),
            wide((t >> 2) & 7, 3),
            wide(t & 3, 2),
        ),
        fm => panic!("texel format {fm} not decoded on the device"),
    }
}

fn modulate(t: u32, c: u32) -> u32 {
    (0..4)
        .map(|i| {
            let sh = 8 * i;
            ((((t >> sh) & 0xFF) * ((c >> sh) & 0xFF) + 127) / 255) << sh
        })
        .fold(0, |a, b| a | b)
}

fn blend(tsp: u32, s: u32, d: u32) -> u32 {
    let (fs, fd) = ((tsp >> 29) & 7, (tsp >> 26) & 7);
    // Without bit 13 alpha acts as 1 (version 21).
    let on = tsp & m3::TSP_BLEND != 0;
    let sa = if on { s >> 24 } else { 256 };
    let da = if on { d >> 24 } else { 256 };
    (0..4)
        .map(|i| {
            let sh = 8 * i;
            let (sc, dc) = ((s >> sh) & 0xFF, (d >> sh) & 0xFF);
            let factor = |code: u32, other: u32| match code {
                0 => 0,
                1 => 256,
                2 => other,
                3 => 256 - other,
                4 => sa,
                5 => 256 - sa.min(256),
                6 => da,
                _ => 256 - da.min(256),
            };
            let v = ((sc * factor(fs, dc)) >> 8) + ((dc * factor(fd, sc)) >> 8);
            v.min(255) << sh
        })
        .fold(0, |a, b| a | b)
}
