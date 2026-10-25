//! Run 3 (`RP_TABENCH_ISOLATE=1`, audit C79): run 2 (commit d7036ba3)
//! found every way wrong, the control included, the TA's lists and objects
//! outside the parameter memory it was set up with; both runs set the
//! memory manager up again without a core reset, where our renderer and
//! `rp-glesfeat` reset first. Run 3 tests that: the control with no reset
//! (run 2's), with a core reset before the manager's set-up once, and
//! before every frame; run 1's conditions both ways; the control with the
//! reset and `0x85C` bit 7 off (R37). One scene, 100 textured quads of 16
//! x 16. Each way: the CPU
//! binner's picture of the same scene under the same conditions first (the
//! reference: the binner draws right, run 1), then five TA frames, each
//! counted word by word against it, the first's TA region array summed
//! (tiles with a list), its first listed tile's list and object words and
//! ten pixels logged.

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

/// Run 1's.
const RUN1: Places = Places {
    pm: super::PM,
    pm_size: super::PM_SIZE,
    pm_table: super::PM_TABLE,
    tail: super::TAIL,
    ta_regions: super::TA_REGIONS,
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
/// Run 1's 16-bit target.
const PICTURE16: Picture = super::PICTURE;

/// One way: what differs from the control.
struct Way {
    name: &'static str,
    target32: bool,
    places: Places,
    /// The memory manager set up before every frame (run 1), or once.
    pm_each: bool,
    /// The 64 x 64 texture (run 1), or an 8 x 8 one (`rp-glesfeat`'s size).
    tex64: bool,
    /// Microseconds between the TA's done and the render's start.
    wait_us: u32,
    /// `0x85C`'s scene-end bit off for the body (audit C78, R37).
    bit7_off: bool,
    /// A core reset before the manager's set-up: never (runs 1 and 2),
    /// once for the way, or before every frame.
    reset: Reset,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reset {
    Never,
    Once,
    EachFrame,
}

/// The control: `rp-glesfeat`'s conditions.
const CONTROL: Way = Way {
    name: "",
    target32: true,
    places: GLESFEAT,
    pm_each: false,
    tex64: false,
    wait_us: 0,
    bit7_off: false,
    reset: Reset::Never,
};

/// Run 1's conditions.
const RUN1_WAY: Way = Way {
    name: "",
    target32: false,
    places: RUN1,
    pm_each: true,
    tex64: true,
    wait_us: 0,
    bit7_off: false,
    reset: Reset::Never,
};

const WAYS: [Way; 6] = [
    Way {
        name: "CONTROL, NO RESET (RUN 2)",
        ..CONTROL
    },
    Way {
        name: "CONTROL, CORE RESET FIRST",
        reset: Reset::Once,
        ..CONTROL
    },
    Way {
        name: "CONTROL, RESET EACH FRAME",
        reset: Reset::EachFrame,
        ..CONTROL
    },
    Way {
        name: "RUN 1'S, CORE RESET FIRST",
        reset: Reset::Once,
        pm_each: false,
        ..RUN1_WAY
    },
    Way {
        name: "RUN 1'S, RESET EACH FRAME",
        reset: Reset::EachFrame,
        ..RUN1_WAY
    },
    Way {
        name: "CONTROL, RESET, BIT 7 OFF",
        reset: Reset::Once,
        bit7_off: true,
        ..CONTROL
    },
];

const QUADS: usize = 100;
const SIDE: i32 = 16;
const FRAMES: usize = 5;

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

    /// Words of `pic` unlike the reference (32-bit: the colour only, the
    /// TA's clear writing alpha 0).
    fn words_off(&self, pic: Picture) -> u32 {
        let mask = if pic.format == PixelFormat::Argb8888 {
            0x00FF_FFFF
        } else {
            u32::MAX
        };
        (0..words_of(pic))
            .filter(|&k| {
                (self.mem.load(pic.at as usize + 4 * k).unwrap_or(0) ^ self.reference[k]) & mask
                    != 0
            })
            .count() as u32
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

    /// The TA's region array at `p`: its tiles with a list (header bit 25
    /// clear: version 16 read bit 25 as an empty tile), and the first
    /// one's header, list words and first object's words, logged.
    fn log_ta_tiles(&self, p: Places) {
        let tiles = (W / m3::TILE_W * (H / m3::TILE_H)) as usize;
        let at = |k: usize| self.mem.load(p.ta_regions as usize + 8 * k).unwrap_or(0);
        let ptr = |k: usize| {
            self.mem
                .load(p.ta_regions as usize + 8 * k + 4)
                .unwrap_or(0)
        };
        let listed = (0..tiles).filter(|&k| at(k) & 1 << 25 == 0).count();
        let last = (0..tiles + 1).position(|k| at(k) & m3::LAST_REGION != 0);
        log!(
            "    TA region array: {listed} of {tiles} tiles with a list (bit 25 clear); the last-entry bit at entry {last:?}; entries 0, 1: {:08x} {:08x}, {:08x} {:08x}",
            at(0),
            ptr(0),
            at(1),
            ptr(1)
        );
        let Some(k) = (0..tiles).find(|&k| at(k) & 1 << 25 == 0) else {
            return;
        };
        let list = (ptr(k) & m3::ADDR_MASK) as usize;
        let _ = write!(
            RunLog,
            "    first listed tile {k} (header {:08x}): list",
            at(k)
        );
        let mut first = None;
        for j in 0..8 {
            let w = self.mem.load(list + 4 * j).unwrap_or(m3::END);
            let _ = write!(RunLog, " {w:08x}");
            if j > 0 && first.is_none() && w != m3::END {
                first = Some(w);
            }
            if w == m3::END {
                break;
            }
        }
        log!("");
        let Some(w) = first else {
            return;
        };
        let obj = p.pm as usize + 4 * (w & 0x001F_FFFF) as usize;
        let _ = write!(
            RunLog,
            "    its second object (list word {w:08x}) at 0x{obj:x}:"
        );
        for j in 0..16 {
            let _ = write!(RunLog, " {:08x}", self.mem.load(obj + 4 * j).unwrap_or(0));
        }
        log!("");
    }
}

impl Rig {
    /// Whether tile 0's list, and the object its first pointer names
    /// (counted from the parameter memory, as the 3D stage reads it), lie
    /// inside the parameter memory at `p`.
    fn inside(&self, p: Places) -> (bool, bool) {
        let range = p.pm..p.pm + p.pm_size;
        let list = self.mem.load(p.ta_regions as usize + 4).unwrap_or(0) & m3::ADDR_MASK;
        let w = self.mem.load(list as usize).unwrap_or(m3::END);
        let obj = p.pm.wrapping_add(4 * (w & 0x001F_FFFF));
        (range.contains(&list), w != m3::END && range.contains(&obj))
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

/// Run 3. Returns (TA frames drawn like the binner, TA frames in all).
pub(super) fn run(g: &mut Rig) -> (u32, u32) {
    let Some(small) = small_texture(&mut g.mem, g.staging) else {
        log!("  no room for the 8 x 8 texture");
        return (0, 0);
    };
    let big = g.tex;
    scatter(g.xy, QUADS, SIDE);
    g.side = SIDE;
    let (mut right, mut all) = (0, 0);
    for (n, way) in WAYS.iter().enumerate() {
        rt::check_in();
        super::step_line(g, n + 1, WAYS.len(), way.name);
        let pic = if way.target32 { PICTURE32 } else { PICTURE16 };
        let target = Target {
            picture: pic,
            x: 0,
            y: 0,
        };
        let tex = if way.tex64 { big } else { small };
        log!(
            "WAY {} of {}: {} (target {}, parameter memory 0x{:x}, TA regions 0x{:x}, manager {}, texture {}, wait {} us)",
            n + 1,
            WAYS.len(),
            way.name,
            if way.target32 { "32-bit" } else { "16-bit" },
            way.places.pm,
            way.places.ta_regions,
            if way.pm_each { "each frame" } else { "once" },
            if way.tex64 { "64 x 64" } else { "8 x 8" },
            way.wait_us
        );
        // The reference: the binner's picture of the same scene here.
        let ref_places = Places {
            pm: REFERENCE.params,
            pm_size: REFERENCE.params_size,
            ..way.places
        };
        let _ = g.pm_init_at(ref_places);
        let xy = &*g.xy;
        let binned = binner::bin_with(
            &mut g.mem,
            &REFERENCE,
            Some(0),
            QUADS,
            |k| draw(xy[k], SIDE, tex),
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
        if us == 0 {
            log!(
                "  the binner's reference didn't draw ({:?}): way skipped",
                binned.err()
            );
            continue;
        }
        g.keep_picture(pic);
        log!("  reference (the binner) rendered in {us} us");
        // The TA's frames.
        let ta_layout = ta::Layout {
            tail: way.places.tail,
            tail_bytes: 0x1_0000,
            regions: way.places.ta_regions,
            width: W,
            height: H,
        };
        let from_ta = binner::Layout {
            params: way.places.pm,
            params_size: way.places.pm_size,
            regions: way.places.ta_regions,
            w: W as u16,
            h: H as u16,
        };
        let state = ta::State {
            tsp: ta::TSP_DELL_TEXTURED,
            isp: m3::ISP_ALWAYS,
            tex: tex.words(),
            clip: ta::clip(W, H),
        };
        let clear = ta::clear_stream(W, H);
        if way.reset != Reset::Never {
            reset_core(&mut g.regs, rt::delay_us);
        }
        let ready = g.pm_init_at(way.places);
        let mut line = [0u32; FRAMES];
        for (k, off) in line.iter_mut().enumerate() {
            rt::check_in();
            if way.reset == Reset::EachFrame && k > 0 {
                reset_core(&mut g.regs, rt::delay_us);
            }
            if (way.pm_each || way.reset == Reset::EachFrame) && k > 0 {
                let _ = g.pm_init_at(way.places);
            }
            let _ = ta::clear(g.mem.0, &ta_layout);
            cpu::drain_write_buffer();
            let xy = &*g.xy;
            let mut feed = ta::Feed::start(&mut g.regs, &ta_layout, CLOCK, TA_LIMIT_US);
            if way.bit7_off {
                feed.scene_end_in_body(false);
            }
            let _ = ta::draws(&[&clear])
                .chain(ta::quads_with(state, ta::Shape::DELL, QUADS, |q| {
                    ta_quad(xy[q], SIDE)
                }))
                .all(|i| feed.put(i));
            let fed = feed.finish();
            all += 1;
            if !fed.done {
                g.ta_hangs += 1;
                log!("    frame {}: the TA never said done", k + 1);
                *off = u32::MAX;
                continue;
            }
            if k == 0 {
                g.log_ta_tiles(way.places);
            }
            let (list_in, obj_in) = g.inside(way.places);
            log!(
                "    frame {}: first list inside the parameter memory {list_in}, its first object {obj_in}",
                k + 1
            );
            rt::delay_us(way.wait_us);
            let r = g.render_at(&from_ta, target, way.places);
            if r == 0 {
                *off = u32::MAX;
                continue;
            }
            *off = g.words_off(pic);
            if *off == 0 {
                right += 1;
            }
            if k == 0 {
                log!(
                    "    frame 1: TA {} us, render {r} us, {} words unlike the binner's",
                    fed.ta_us,
                    *off
                );
                let _ = write!(RunLog, "    pixels (TA / binner):");
                for (x, y) in super::isolate_samples(&*g.xy) {
                    let (got, want) = g.pixel(pic, x, y);
                    let _ = write!(RunLog, " ({x},{y}) {got:x}/{want:x}");
                }
                log!("");
            }
        }
        let drawn = line.iter().filter(|&&o| o == 0).count();
        log!(
            "  {}: memory manager ready {ready}; frames like the binner's: {drawn} of {FRAMES}; words unlike it, a frame: {line:?} (u32::MAX: hung or not done)",
            way.name
        );
    }
    (right, all)
}
