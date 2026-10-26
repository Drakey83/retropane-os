//! Run 8 (`RP_TABENCH_ISOLATE=1`, audit C81): run 7 (commit 1458a1c4) set
//! the memory manager up once a set and 148 of 150 TA renders drew right;
//! the two that hung were each the first render after a set-up, as run 6's
//! three were. Run 8 measures fixes for that first render, every one of them
//! paid once after a set-up and never per frame: none (the control), a
//! warm-up scene of our background strip alone rendered and thrown away, a
//! warm-up of one textured quad, the core's status read again and 1 ms
//! waited after the manager says ready, and the warm-up strip after that
//! wait. Twenty set-ups a way (a core reset, then the manager), each followed
//! by its fix and two real frames of run 7's set 1 (100 quads of 16 x 16,
//! Dell's feed, our background strip), each counted against the binner's
//! picture. A render past 200 ms (the scene takes 14 ms) is a hang. Logged:
//! each way's hangs on the first and second real frame, the warm-ups' own
//! hangs, and what the fix costs a set-up.

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

/// Run 1's and run 7's.
const PLACES: Places = Places {
    pm: super::PM,
    pm_size: super::PM_SIZE,
    pm_table: super::PM_TABLE,
    tail: super::TAIL,
    ta_regions: super::TA_REGIONS,
};

/// The binner's reference (its own parameter memory and region array, as
/// run 7's binner).
const REFERENCE: binner::Layout = super::BIN;

/// Run 7's 16-bit target.
const PICTURE: Picture = super::PICTURE;
const TARGET: Target = super::FULL;

/// What follows a set-up before the first real frame.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fix {
    /// Nothing (run 7's).
    None,
    /// Our background strip alone through the TA, rendered, thrown away.
    WarmStrip,
    /// One textured quad through the TA, rendered, thrown away.
    WarmQuad,
    /// The status read again and 1 ms waited after the manager's ready.
    Settle,
    /// [`Fix::Settle`], then [`Fix::WarmStrip`].
    SettleWarmStrip,
}

struct Way {
    name: &'static str,
    fix: Fix,
}

const WAYS: [Way; 5] = [
    Way {
        name: "NO FIX (RUN 7'S)",
        fix: Fix::None,
    },
    Way {
        name: "WARM-UP: THE STRIP",
        fix: Fix::WarmStrip,
    },
    Way {
        name: "WARM-UP: ONE QUAD",
        fix: Fix::WarmQuad,
    },
    Way {
        name: "SETTLE: STATUS + 1 MS",
        fix: Fix::Settle,
    },
    Way {
        name: "SETTLE, THEN THE STRIP",
        fix: Fix::SettleWarmStrip,
    },
];

const QUADS: usize = 100;
const SIDE: i32 = 16;
/// Set-ups a way.
const SETUPS: usize = 20;
/// Real frames after each set-up.
const FRAMES: usize = 2;
/// A render past this is a hang (the scene renders in ~14 ms).
const HANG_US: u32 = 200_000;
/// Warm-up tries before a set-up counts as failed.
const WARM_TRIES: usize = 3;

/// What one render did.
struct Rendered {
    /// Its time; 0 if it hung.
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

    /// A core reset, then the manager set up at `p`.
    fn set_up(&mut self, p: Places) -> bool {
        reset_core(&mut self.regs, rt::delay_us);
        self.pm_init_at(p)
    }

    /// The 3D stage on `l` into the target, given up on past `limit_us`
    /// (then the core is left for the caller to set up again).
    fn render_within(&mut self, l: &binner::Layout, limit_us: u32) -> Rendered {
        let en = self.regs.read32(MBX + reg::ENABLE);
        self.regs.write32(MBX + reg::CLEAR, m3::RENDER_DONE);
        self.regs.write32(MBX + reg::ENABLE, en | m3::RENDER_DONE);
        let t0 = rt::ticks();
        for (off, v) in binner::start_table(l, TARGET) {
            self.regs.write32(MBX + off, v);
        }
        let mut us = 0;
        let mut status = 0;
        while us_since(t0) < limit_us {
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
}

fn words_of(pic: Picture) -> usize {
    (pic.stride * u32::from(pic.h) / 4) as usize
}

/// Run 8. Returns (real frames drawn like the binner, real frames in all).
pub(super) fn run8(g: &mut Rig) -> (u32, u32) {
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
    scatter(g.xy, QUADS, SIDE);
    g.side = SIDE;
    // The reference: the binner's picture of the scene on the same black.
    let _ = g.set_up(p);
    let xy = &*g.xy;
    let binned = binner::bin_with(
        &mut g.mem,
        &REFERENCE,
        Some(super::BLACK),
        QUADS,
        |k| draw(xy[k], SIDE, tex),
        Scratch {
            starts: &mut *g.starts,
            entries: &mut *g.entries,
            objects: &mut *g.objects,
        },
    );
    let r = if binned.is_ok() {
        g.render_within(&REFERENCE, 2_000_000).us
    } else {
        0
    };
    log!("the binner's reference: rendered in {r} us");
    if r == 0 {
        log!("  no reference: run 8 not run");
        return (0, 0);
    }
    g.keep_reference();
    let (mut right, mut all) = (0, 0);
    for (w, way) in WAYS.iter().enumerate() {
        super::step_line(g, w + 1, WAYS.len(), way.name);
        log!("WAY {} of {}: {}", w + 1, WAYS.len(), way.name);
        let mut hung = [0u32; FRAMES];
        let mut wrong = [0u32; FRAMES];
        let (mut warm_hangs, mut failed, mut cost_max, mut cost_sum) = (0u32, 0u32, 0u32, 0u32);
        for k in 0..SETUPS {
            rt::check_in();
            // The set-up and its fix, timed; a warm-up that hangs is set up
            // again and tried again (the recovery a driver would do).
            let t0 = rt::ticks();
            let mut ready = false;
            for _ in 0..WARM_TRIES {
                if !g.set_up(p) {
                    continue;
                }
                if matches!(way.fix, Fix::Settle | Fix::SettleWarmStrip) {
                    let _ = g.regs.read32(MBX + reg::STATUS);
                    rt::delay_us(1_000);
                }
                let warm: Option<&[u32]> = match way.fix {
                    Fix::WarmStrip | Fix::SettleWarmStrip => Some(&strip),
                    _ => None,
                };
                let warm_quad = way.fix == Fix::WarmQuad;
                if warm.is_none() && !warm_quad {
                    ready = true;
                    break;
                }
                let _ = ta::clear(g.mem.0, &ta_layout);
                cpu::drain_write_buffer();
                let xy = &*g.xy;
                let mut feed = ta::Feed::start(&mut g.regs, &ta_layout, CLOCK, TA_LIMIT_US);
                let _ = match warm {
                    Some(s) => ta::draws(&[s]).all(|i| feed.put(i)),
                    None => ta::quads_with(state, ta::Shape::DELL, 1, |q| ta_quad(xy[q], SIDE))
                        .all(|i| feed.put(i)),
                };
                let fed = feed.finish();
                if fed.done && g.render_within(&from_ta, HANG_US).us != 0 {
                    ready = true;
                    break;
                }
                warm_hangs += 1;
            }
            let cost = us_since(t0);
            cost_max = cost_max.max(cost);
            cost_sum += cost;
            if !ready {
                failed += 1;
                log!("    set-up {}: not ready after {WARM_TRIES} tries", k + 1);
                continue;
            }
            for f in 0..FRAMES {
                let _ = ta::clear(g.mem.0, &ta_layout);
                cpu::drain_write_buffer();
                let xy = &*g.xy;
                let mut feed = ta::Feed::start(&mut g.regs, &ta_layout, CLOCK, TA_LIMIT_US);
                let _ = ta::draws(&[&strip])
                    .chain(ta::quads_with(state, ta::Shape::DELL, QUADS, |q| {
                        ta_quad(xy[q], SIDE)
                    }))
                    .all(|i| feed.put(i));
                let fed = feed.finish();
                all += 1;
                if !fed.done {
                    g.ta_hangs += 1;
                    hung[f] += 1;
                    log!(
                        "    set-up {}, frame {}: the TA never said done",
                        k + 1,
                        f + 1
                    );
                    break;
                }
                let r = g.render_within(&from_ta, HANG_US);
                if r.us == 0 {
                    hung[f] += 1;
                    log!(
                        "    set-up {}, frame {} HUNG (status 0x{:08x})",
                        k + 1,
                        f + 1,
                        r.status
                    );
                    // The next set-up starts from a reset anyway.
                    break;
                }
                if g.off_reference() == 0 {
                    right += 1;
                } else {
                    wrong[f] += 1;
                }
            }
        }
        log!(
            "  {}: of {SETUPS} set-ups, frame 1 hung {}, frame 2 hung {}; wrong pictures {} and {}; warm-ups hung {warm_hangs}; set-ups never ready {failed}; the set-up with its fix {} us on average, {cost_max} us at most",
            way.name,
            hung[0],
            hung[1],
            wrong[0],
            wrong[1],
            cost_sum / SETUPS as u32
        );
    }
    (right, all)
}
