//! Run 6 (`RP_TABENCH_ISOLATE=1`, audit C81): why about one render in
//! three of a TA-built scene hung the 3D stage in run 5 (commit 62f3dd1d),
//! the pictures otherwise right. Across runs 2-5 the hangs came where the
//! memory manager was set up again before every frame (run 5: 45 of 150;
//! run 4, set up once a way: 0 of 24), mostly a way's first frame. Run 6
//! measures it under run 5's conditions (16-bit target, run 1's places, the
//! 64 x 64 texture, our black background strip), on run 5's two most
//! hang-prone scenes (100 quads of 16 x 16 as one list; 1,000 of 64 x 64
//! Dell's way), ten frames a way: the set-up once a way, or every frame
//! (with the core reset, without it, with a 1 ms wait before the feed),
//! and a 1 ms wait between the TA's done and the render. Each frame counted
//! against the binner's picture; each hang logged with the core's status
//! and the TA's region array beside a frame that drew.

use core::fmt::Write;

use rp_drv_blit2d::{MBX, reg, reset_core};
use rp_drv_mbx3d::{self as m3, ta};
use rp_hal_api::RegisterBus;
use rp_hal_pxa27x::{cpu, rt};
use rp_hal_x51v::devices::RunLog;
use rp_mbx3d_render::binner::{self, Scratch};
use rp_mbx3d_render::{LocalMem, Picture, Target};

use super::{CLOCK, H, Rig, TA_LIMIT_US, W, background_strip, draw, scatter, ta_quad, us_since};

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

/// Run 1's and run 5's.
const PLACES: Places = Places {
    pm: super::PM,
    pm_size: super::PM_SIZE,
    pm_table: super::PM_TABLE,
    tail: super::TAIL,
    ta_regions: super::TA_REGIONS,
};

/// The binner's reference (its own parameter memory and region array, as
/// run 5's binner).
const REFERENCE: binner::Layout = super::BIN;

/// Run 5's 16-bit target.
const PICTURE: Picture = super::PICTURE;
const TARGET: Target = super::FULL;

/// How the memory manager is set up for the TA's frames.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SetUp {
    /// Once for the way, after a core reset (run 4's).
    Once,
    /// Before every frame, after a core reset (run 5's).
    EachReset,
    /// Before every frame, no reset.
    EachNoReset,
    /// Before every frame after a reset, then 1 ms before the feed.
    EachResetWait,
}

struct Way {
    name: &'static str,
    setup: SetUp,
    /// Microseconds between the TA's done and the render's start.
    wait_us: u32,
}

const WAYS: [Way; 6] = [
    Way {
        name: "SET UP ONCE A WAY",
        setup: SetUp::Once,
        wait_us: 0,
    },
    Way {
        name: "RESET + SET UP EACH FRAME",
        setup: SetUp::EachReset,
        wait_us: 0,
    },
    Way {
        name: "SET UP EACH FRAME, NO RESET",
        setup: SetUp::EachNoReset,
        wait_us: 0,
    },
    Way {
        name: "EACH FRAME, 1 MS BEFORE FEED",
        setup: SetUp::EachResetWait,
        wait_us: 0,
    },
    Way {
        name: "EACH FRAME, 1 MS BEFORE RENDER",
        setup: SetUp::EachReset,
        wait_us: 1_000,
    },
    Way {
        name: "ONCE A WAY, 1 MS BEFORE RENDER",
        setup: SetUp::Once,
        wait_us: 1_000,
    },
];

/// The scenes: (quads, side, shape, name), run 5's most hang-prone.
const SCENES: [(usize, i32, ta::Shape, &str); 2] = [
    (
        100,
        16,
        ta::Shape {
            batching: ta::Batching::Once,
            primitive: ta::Primitive::List,
            ..ta::Shape::DELL
        },
        "100 X 16 PX, ONE LIST",
    ),
    (1_000, 64, ta::Shape::DELL, "1000 X 64 PX, DELL'S"),
];

const FRAMES: usize = 10;

/// What one render did.
struct Rendered {
    /// Its time; 0 if it hung (past 2 s).
    us: u32,
    /// The core's status (`0x12c`) when it finished or was given up on.
    status: u32,
}

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

    /// The 3D stage on `l` into the target; past 2 s the core is reset and
    /// the manager set up again at `p` (after the status is read).
    fn render_probe(&mut self, l: &binner::Layout, p: Places) -> Rendered {
        let en = self.regs.read32(MBX + reg::ENABLE);
        self.regs.write32(MBX + reg::CLEAR, m3::RENDER_DONE);
        self.regs.write32(MBX + reg::ENABLE, en | m3::RENDER_DONE);
        let t0 = rt::ticks();
        for (off, v) in binner::start_table(l, TARGET) {
            self.regs.write32(MBX + off, v);
        }
        let mut us = 0;
        let mut status = 0;
        while us_since(t0) < 2_000_000 {
            status = self.regs.read32(MBX + reg::STATUS);
            if status & m3::RENDER_DONE != 0 {
                us = us_since(t0).max(1);
                break;
            }
        }
        self.regs.write32(MBX + reg::CLEAR, m3::RENDER_DONE);
        self.regs.write32(MBX + reg::ENABLE, en);
        if us == 0 {
            self.render_hangs += 1;
            reset_core(&mut self.regs, rt::delay_us);
            let _ = self.pm_init_at(p);
        }
        Rendered { us, status }
    }

    /// The target kept in SDRAM as the scene's reference.
    fn keep_reference(&mut self) {
        for k in 0..words_of(PICTURE) {
            self.reference[k] = self.mem.load(PICTURE.at as usize + 4 * k).unwrap_or(0);
        }
    }

    /// Words of the target unlike the reference.
    fn off_reference(&self) -> u32 {
        (0..words_of(PICTURE))
            .filter(|&k| {
                self.mem.load(PICTURE.at as usize + 4 * k).unwrap_or(0) != self.reference[k]
            })
            .count() as u32
    }

    /// The TA's region array at `p`: tiles with a list (header bit 25
    /// clear), the last entry's place, the first tile's list words.
    fn ta_summary(&self, p: Places) -> (usize, Option<usize>, [u32; 3]) {
        let tiles = (W / m3::TILE_W * (H / m3::TILE_H)) as usize;
        let at = |k: usize| self.mem.load(p.ta_regions as usize + 8 * k).unwrap_or(0);
        let listed = (0..tiles).filter(|&k| at(k) & 1 << 25 == 0).count();
        let last = (0..tiles + 1).position(|k| at(k) & m3::LAST_REGION != 0);
        let list = self.mem.load(p.ta_regions as usize + 4).unwrap_or(0) & m3::ADDR_MASK;
        let mut words = [m3::END; 3];
        for (j, w) in words.iter_mut().enumerate() {
            *w = self.mem.load(list as usize + 4 * j).unwrap_or(m3::END);
        }
        (listed, last, words)
    }
}

fn words_of(pic: Picture) -> usize {
    (pic.stride * u32::from(pic.h) / 4) as usize
}

/// Run 6. Returns (frames drawn like the binner, frames in all).
pub(super) fn run6(g: &mut Rig) -> (u32, u32) {
    let tex = g.tex;
    let p = PLACES;
    let ta_layout = ta::Layout {
        tail: p.tail,
        tail_bytes: 0x1_0000,
        regions: p.ta_regions,
        width: W,
        height: H,
    };
    let from_ta = binner::Layout {
        regions: p.ta_regions,
        ..REFERENCE
    };
    let state = ta::State {
        tsp: ta::TSP_DELL_TEXTURED,
        isp: m3::ISP_ALWAYS,
        tex: tex.words(),
        clip: ta::clip(W, H),
    };
    let strip = background_strip();
    let (mut right, mut all) = (0, 0);
    let steps = SCENES.len() * WAYS.len();
    for (s, &(n, side, shape, scene)) in SCENES.iter().enumerate() {
        scatter(g.xy, n, side);
        g.side = side;
        // The reference: the binner's picture of the scene, on the same
        // black (its background).
        reset_core(&mut g.regs, rt::delay_us);
        let _ = g.pm_init_at(p);
        let xy = &*g.xy;
        let binned = binner::bin_with(
            &mut g.mem,
            &REFERENCE,
            Some(super::BLACK),
            n,
            |k| draw(xy[k], side, tex),
            Scratch {
                starts: &mut *g.starts,
                entries: &mut *g.entries,
                objects: &mut *g.objects,
            },
        );
        let r = if binned.is_ok() {
            g.render_probe(&REFERENCE, p).us
        } else {
            0
        };
        log!(
            "SCENE {} of {}: {scene}; the binner's reference rendered in {r} us",
            s + 1,
            SCENES.len()
        );
        if r == 0 {
            log!("  no reference: scene skipped");
            continue;
        }
        g.keep_reference();
        for (w, way) in WAYS.iter().enumerate() {
            super::step_line(g, s * WAYS.len() + w + 1, steps, way.name);
            reset_core(&mut g.regs, rt::delay_us);
            let ready = g.pm_init_at(p);
            let (mut drawn, mut hung) = (0, 0);
            let mut logged_drawn = false;
            let mut line = [0u32; FRAMES];
            for (k, off) in line.iter_mut().enumerate() {
                rt::check_in();
                if way.setup != SetUp::Once && k > 0 {
                    if way.setup != SetUp::EachNoReset {
                        reset_core(&mut g.regs, rt::delay_us);
                    }
                    let _ = g.pm_init_at(p);
                    if way.setup == SetUp::EachResetWait {
                        rt::delay_us(1_000);
                    }
                }
                let _ = ta::clear(g.mem.0, &ta_layout);
                cpu::drain_write_buffer();
                let xy = &*g.xy;
                let mut feed = ta::Feed::start(&mut g.regs, &ta_layout, CLOCK, TA_LIMIT_US);
                let _ = ta::draws(&[&strip])
                    .chain(ta::quads_with(state, shape, n, |q| ta_quad(xy[q], side)))
                    .all(|i| feed.put(i));
                let fed = feed.finish();
                all += 1;
                if !fed.done {
                    g.ta_hangs += 1;
                    log!("    frame {}: the TA never said done", k + 1);
                    *off = u32::MAX;
                    continue;
                }
                rt::delay_us(way.wait_us);
                let summary = g.ta_summary(p);
                let r = g.render_probe(&from_ta, p);
                if r.us == 0 {
                    hung += 1;
                    *off = u32::MAX;
                    log!(
                        "    frame {} HUNG: status 0x{:08x} at 2 s; TA done in {} us; TA tiles listed {}, last entry {:?}, tile 0's list {:08x} {:08x} {:08x}",
                        k + 1,
                        r.status,
                        fed.ta_us,
                        summary.0,
                        summary.1,
                        summary.2[0],
                        summary.2[1],
                        summary.2[2]
                    );
                    continue;
                }
                *off = g.off_reference();
                if *off == 0 {
                    drawn += 1;
                    right += 1;
                }
                if !logged_drawn {
                    logged_drawn = true;
                    log!(
                        "    frame {} drew in {} us (status 0x{:08x}), {} words unlike the binner's; TA done in {} us; TA tiles listed {}, last entry {:?}, tile 0's list {:08x} {:08x} {:08x}",
                        k + 1,
                        r.us,
                        r.status,
                        *off,
                        fed.ta_us,
                        summary.0,
                        summary.1,
                        summary.2[0],
                        summary.2[1],
                        summary.2[2]
                    );
                }
            }
            log!(
                "  {scene}, {}: manager ready {ready}; {drawn} of {FRAMES} like the binner's, {hung} hung; words unlike it a frame: {line:?} (u32::MAX: hung)",
                way.name
            );
        }
    }
    (right, all)
}
