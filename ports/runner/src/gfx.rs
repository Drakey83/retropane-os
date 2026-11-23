//! The 2700G for the program's 3D frames (`rp_gfx_*`, `rp_port::gfx`;
//! docs/design/ref-rp.md step 3): the tile accelerator set up once, each
//! frame's checked words fed to it behind a background, the 3D core
//! drawing straight into the screen on show, the next frame made by the
//! CPU while it draws.
//!
//! Iterate, don't imitate (rule 0). Dell's GLES driver waits for each
//! render, and ADR-0034's option A draws off screen and has the 2D engine
//! copy the picture in (~5 ms a frame). Here the 3D core draws into the
//! screen at `0x0` (option C: G21 on the X51v, right at every clock where
//! off screen failed above 74.75 / 1; no copy), and the CPU waits for a
//! render only when the next frame is ready before it is done. Every 16th
//! render is waited for at once, so the run's log has the render's own
//! time beside the feed's.
//!
//! 2700G memory while the program has the screen (this runner's: the
//! panel's screen at `0x0` is the target; Game Mode's own layout goes into
//! `rp-vram-map` with the OS's port of this, R291):
//!
//! | Where | What |
//! |---|---|
//! | `0x000000` | the screen, 480 x 640 RGB565 (the safe region) |
//! | `0x0FC000` | the parameter memory manager's block (16 KiB) |
//! | `0x100000` | the TA's tail buffer (64 KiB) |
//! | `0x120000` | the TA's region array |
//! | `0x140000` | kit 113: the second scene's tail buffer (64 KiB) |
//! | `0x150000` | kit 113: the second scene's region array |
//! | `0x200000` | the TA's parameter memory (5 MiB, below the manager's 8 MiB reach) |
//! | `0x800000` | the program's texture memory (7 MiB) |

use core::fmt::{Arguments, Write};

use rp_drv_blit2d::{MBX, reg};
use rp_drv_mbx_display::clock::{self, MBXCLK, with_3d};
use rp_drv_mbx_display::console::Console;
use rp_drv_mbx_display::layers::Layer;
use rp_drv_mbx_display::{PixelFormat as PlaneFormat, Plane, ack_events, graphics_set};
use rp_drv_mbx3d::{self as m3, ta};
use rp_drv_pxa_keypad::{Button, Pressed};
use rp_hal_api::RegisterBus;
use rp_hal_cabi::{Errno, Gfx};
use rp_hal_mem::{load32_volatile, store32_volatile};
use rp_hal_pxa27x::{Mmio, cpu, rt};
use rp_mbx3d_render::{Picture, PixelFormat, Target, TileOrder, binner, fixed, order_tile};
use rp_port::gfx::{Piece, Screen, check, send, send_runs};

use super::panel;

mod chain;
mod framelog;
mod pmwatch;
mod probe4f;
use chain::Settled;
use framelog::Kind as FrameKind;
pub use framelog::{FILE as FRAME_LOG_FILE, FrameLog, ROOM as FRAME_LOG_ROOM};
use pmwatch::{F_OVERLAP, F_RENDERING, F_TA_BUSY, Moment};
pub use pmwatch::{FILE as PM_WATCH_FILE, PmWatch, ROOM as PM_WATCH_ROOM};

/// R340's next measure ([`FrameLog`]): the chip's two stages frame by
/// frame, put in by the runner and taken back for the card.
pub static FRAME_LOG: rt::Global<Option<FrameLog>> = rt::Global::new(None);

fn frame_ev(k: FrameKind, frame: u32, a: u32, b: u32) {
    let t = rt::ticks();
    FRAME_LOG.with(|l| {
        if let Some(l) = l.as_mut() {
            l.put(k, frame, t, a, b);
        }
    });
}

/// R340's measure ([`PmWatch`]): the runner puts it in before the program
/// runs and takes it back after, for the card.
pub static PM_WATCH: rt::Global<Option<PmWatch>> = rt::Global::new(None);

/// R340: a snapshot of the manager's block and the TA block (taken out of
/// [`PM_WATCH`] while it reads: ~2 ms of bus reads with IRQs on).
fn pm_snap(m: Moment, regs: &mut Mmio, o: &Open, flags: u32, log: fn(Arguments<'_>)) {
    let Some(mut w) = PM_WATCH.with(Option::take) else {
        return;
    };
    if w.wants(m) {
        let flags = flags
            | if o.rendering.is_some() {
                F_RENDERING
            } else {
                0
            }
            | if o.overlap { F_OVERLAP } else { 0 };
        w.take(
            m,
            regs,
            o.vram,
            o.lay.pm.table,
            (o.pm_frame, us_since(o.pm_opened) / 1000, flags),
            log,
        );
    }
    PM_WATCH.with(|s| *s = Some(w));
}

/// R340's fits check: the parameter memory's pages (the ring's, 0x201 to
/// 0x6FF: `TA_PM`'s 5 MiB less the manager's header page) and the room
/// kept spare.
const PM_PAGES: u32 = (PM_SIZE >> 12) - 1;
const PM_MARGIN: u32 = 64;
/// The ring's first page.
const PM_FIRST: u32 = (PM >> 12) + 1;

/// R340 (run 286): the TA's page cursor, entry 0 of the manager's block
/// (16-bit links: the next page the TA takes). One word read.
fn pm_cursor(o: &Open) -> Option<u32> {
    load32_volatile(o.vram, o.lay.pm.table as usize).map(|w| w & 0xFFFF)
}

/// Pages from cursor `a` to cursor `b` round the ring.
fn ring_dist(a: u32, b: u32) -> u32 {
    let (a, b) = (a.wrapping_sub(PM_FIRST), b.wrapping_sub(PM_FIRST));
    if a >= PM_PAGES || b >= PM_PAGES {
        return 0;
    }
    (b + PM_PAGES - a) % PM_PAGES
}

/// R340: snapshots of a game's steady frames, not the loading screens'.
const PM_SNAP_FROM: u32 = 300;
pub use chain::{CHAIN_BYTES, Chain};
pub use probe4f::{Probe, WALK_BYTES};

/// ADR-0040 step 7g6's probe of the strip header's bits: 0 off, 1 bit 17
/// (16.16 vertex words), 2 bit 21 (no z word), 3 both (`-rpta17`,
/// `-rpta21`, `-rpta1721`; the CPU feed only: with `-rpnochain`).
pub static TA_PROBE: rt::Global<u32> = rt::Global::new(0);

/// A float word as 16.16 fixed point, rounded, saturated.
fn fix16(w: u32) -> u32 {
    let s = f32::from_bits(w) * 65536.0;
    (if s >= 0.0 {
        (s + 0.5) as i32
    } else {
        (s - 0.5) as i32
    }) as u32
}

const W: u32 = 480;
const H: u32 = 640;
/// The screen's bytes a line.
const STRIDE: usize = W as usize * 2;
const PM_TABLE: u32 = 0x000F_C000;
const TAIL: u32 = 0x0010_0000;
const TAIL_BYTES: u32 = 0x1_0000;
const TA_REGIONS: u32 = 0x0012_0000;
const PM: u32 = 0x0020_0000;
const PM_SIZE: u32 = 0x0050_0000;
/// Kit 113: the second scene's tail buffer and region array (the TA takes
/// the next frame into these while the 3D stage draws the last from the
/// first; the parameter memory and its manager are shared).
const TAIL_B: u32 = 0x0014_0000;
const REGIONS_B: u32 = 0x0015_0000;
const _: () = assert!(TAIL_B.is_multiple_of(0x2_0000) && TAIL_B + TAIL_BYTES <= REGIONS_B);
const _: () = assert!(REGIONS_B + REGION_BYTES <= PM && CHK_REGIONS + REGION_BYTES <= TAIL_B);
const _: () = assert!((PM + PM_SIZE) as usize <= m3::PM_REACH);
/// The program's texture memory.
const TEX_AT: u32 = 0x0080_0000;
const TEX_BYTES: u32 = 0x0070_0000;
/// The safe region (ADR-0034): the screen.
const SAFE: u32 = 0x000B_0000;
/// What the TA's parameter memory stays clear of.
const KEEP: [(u32, u32); 4] = [
    (0, SAFE),
    (TAIL, TAIL_BYTES),
    (TA_REGIONS, 0x4B08),
    (TEX_AT, TEX_BYTES),
];
const TA: ta::Layout = ta::Layout {
    tail: TAIL,
    tail_bytes: TAIL_BYTES,
    regions: TA_REGIONS,
    width: W,
    height: H,
};
const TA_PM: ta::ParamMemory = ta::ParamMemory {
    at: PM,
    size: PM_SIZE,
    table: PM_TABLE,
};
/// The 3D stage on the TA's region array, into the screen.
const FROM_TA: binner::Layout = binner::Layout {
    params: PM,
    params_size: PM_SIZE,
    regions: TA_REGIONS,
    w: W as u16,
    h: H as u16,
};
const SCREEN: Target = Target {
    picture: Picture::packed(0, W as u16, H as u16, PixelFormat::Rgb565),
    x: 0,
    y: 0,
};
const CLOCK: ta::Clock = ta::Clock {
    now: rt::ticks,
    ticks_per_ms: rt::TICK_HZ / 1000,
};
/// Where the TA works and what the 3D stage reads (R295's cells move the
/// small parts onto the die).
struct Lay {
    name: &'static str,
    ta: ta::Layout,
    pm: ta::ParamMemory,
    from: binner::Layout,
    keep: [(u32, u32); 4],
}

/// The layout above: everything but the screen in SDRAM.
const LAY_SDRAM: Lay = Lay {
    name: "all in SDRAM",
    ta: TA,
    pm: TA_PM,
    from: FROM_TA,
    keep: KEEP,
};

/// The frame-rate bar (`-rpfpsbar`, the owner 2026-11-16: "add an fps
/// counter"): the 3D frame stops this many rows short of the bottom and
/// the runner writes the rate, the clock and the hangs there. ref_rp
/// draws no 2D yet (the engine's own counter would not show), and the 3D
/// core paints every row it owns each frame, so the bar's rows are the
/// CPU's alone.
const BAR_ROWS: u32 = 16;
const DRAW_H: u32 = H - BAR_ROWS;
const LAY_BAR: Lay = Lay {
    name: "all in SDRAM, the bottom 16 rows the frame-rate bar's",
    ta: ta::Layout {
        height: DRAW_H,
        ..TA
    },
    pm: TA_PM,
    from: binner::Layout {
        h: DRAW_H as u16,
        ..FROM_TA
    },
    keep: KEEP,
};
/// R343's picker (`-rpcurves`, the owner after run 276: "it wasnt
/// titled clear enough"): a band of 48 rows (the landscape picture's left
/// edge, 48 pixels wide) the 3D frame leaves to the CPU, where the curve's
/// name stands big and upright as the owner holds the device, its letters
/// stacked down the band; the frame rate under it.
const PICK_ROWS: u32 = 48;
const PICK_H: u32 = H - PICK_ROWS;
const LAY_PICK: Lay = Lay {
    name: "all in SDRAM, the bottom 48 rows the curve picker's",
    ta: ta::Layout {
        height: PICK_H,
        ..TA
    },
    pm: TA_PM,
    from: binner::Layout {
        h: PICK_H as u16,
        ..FROM_TA
    },
    keep: KEEP,
};
const _: () = assert!(PICK_H.is_multiple_of(m3::TILE_H) && PICK_H.is_multiple_of(16));
const SCREEN_BAR: Target = Target {
    picture: Picture::packed(0, W as u16, DRAW_H as u16, PixelFormat::Rgb565),
    x: 0,
    y: 0,
};

/// The screen's bytes: the rest of the safe region (the die) is free
/// while the program has the screen.
const SCREEN_BYTES: u32 = W * H * 2;
/// The region array, (tiles + 1) x 8 bytes.
const REGION_BYTES: u32 = 0x4B08;
const DIE_REGIONS: u32 = SCREEN_BYTES;
const DIE_TABLE: u32 = 0x0009_C000;
/// 128 KiB aligned, as the TA's tail must be.
const DIE_TAIL: u32 = 0x000A_0000;
const _: () = assert!(DIE_REGIONS + REGION_BYTES <= DIE_TABLE && DIE_TAIL + TAIL_BYTES <= SAFE);
const _: () = assert!(DIE_TAIL.is_multiple_of(0x2_0000) && DIE_TABLE.is_multiple_of(0x4000));
/// The region array, the manager's block and the tail on the die (R295
/// cell 5); the lists stay in SDRAM (they are larger than the die).
#[allow(
    dead_code,
    reason = "run 18's cell 5 (no cure); kept for R295's next cells"
)]
const LAY_DIE: Lay = Lay {
    name: "region array, manager block and tail on the die",
    ta: ta::Layout {
        tail: DIE_TAIL,
        tail_bytes: TAIL_BYTES,
        regions: DIE_REGIONS,
        width: W,
        height: H,
    },
    pm: ta::ParamMemory {
        at: PM,
        size: PM_SIZE,
        table: DIE_TABLE,
    },
    from: binner::Layout {
        params: PM,
        params_size: PM_SIZE,
        regions: DIE_REGIONS,
        w: W as u16,
        h: H as u16,
    },
    keep: [
        (0, SCREEN_BYTES),
        (DIE_TAIL, TAIL_BYTES),
        (DIE_REGIONS, REGION_BYTES),
        (TEX_AT, TEX_BYTES),
    ],
};

/// R295's diagnostic ladder (`-rpladder`, docs/research/sheets/r295-ta-path-136.md):
/// one cell's clock, layout and render words.
struct Cell {
    /// The core PLL's M, N, P; the cores at divide 1.
    pll: (u32, u32, u32),
    lay: &'static Lay,
    /// Render words put in place of the start table's.
    words: &'static [(usize, u32)],
    what: &'static str,
    frames: u32,
    /// `LMTIM` in place of the ladder's for the clock (run 22).
    lmtim: Option<u32>,
    /// What of the frame goes to the chip (run 23).
    part: Part,
    /// After a hang, one background-only render before the next frame
    /// (run 26: whether the core's recovery is what makes hangs come in
    /// bursts).
    warm: bool,
    /// Run 28: the TA's region array copied in the checker order (the
    /// binner's tile-race cure, GC2BF) on every other frame.
    checker: bool,
    /// Run 31: ADR-0035 E's alpha-0 overlay (a full-screen quad of alpha
    /// 0 blended over the scene, pacing each tile's write-out: R121, 0 of
    /// 32,592 renders wrong in an hour) on every other frame.
    overlay: bool,
}

/// Where the checker-ordered copy of the region array goes (SDRAM, clear
/// of the TA's own array and of the parameter memory).
const CHK_REGIONS: u32 = 0x0012_8000;
const _: () = assert!(CHK_REGIONS >= TA_REGIONS + REGION_BYTES && CHK_REGIONS + REGION_BYTES <= PM);
/// A region entry's bit: the last.
const REGION_LAST: u32 = 1 << 31;

/// Run 23's split of the frame by kind of work: which path stalls the
/// core at 136.50 / 1.
#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code, reason = "each run's cells use some")]
enum Part {
    /// The frame as the program made it.
    All,
    /// Every texture shrunk to 8 x 8 (TEX0's size codes 0): the same
    /// geometry and states, almost no texel reads.
    TinyTextures,
    /// The models' strips and fans left out (a state with Gouraud, TSP bit
    /// 22, is a model's).
    NoModels,
    /// The world's left out (a state without TSP bit 22).
    NoWorld,
    /// The models' fans left out (run 24).
    NoModelFans,
    /// The models' strips left out, their fans kept.
    NoModelStrips,
    /// The models' alpha test made "always" (ISP bits 31-21).
    ModelsNoAlphaTest,
    /// The models flat, not Gouraud (TSP bit 22 cleared).
    ModelsFlat,
    /// The models' strips whose corners all lie within 3 x 3 pixels left
    /// out (run 26: distant models' tiny strips).
    TinyModelStrips,
    /// The models' strips holding a sliver (a triangle of under a quarter
    /// of a pixel's area) left out.
    ThinModelStrips,
}

/// The most words a strip holds (255 corners of 7 words).
const STRIP_MOST: usize = 255 * 7;

/// Whether a strip's corners (`c`: 7 words each, x and y first, floats)
/// should be left out for `part`.
fn leave_out(part: Part, c: &[u32]) -> bool {
    let xy = |k: usize| (f32::from_bits(c[7 * k]), f32::from_bits(c[7 * k + 1]));
    let n = c.len() / 7;
    if n < 3 {
        return false;
    }
    match part {
        Part::TinyModelStrips => {
            let (mut x0, mut y0) = xy(0);
            let (mut x1, mut y1) = (x0, y0);
            for k in 1..n {
                let (x, y) = xy(k);
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
            x1 - x0 < 3.0 && y1 - y0 < 3.0
        }
        Part::ThinModelStrips => (0..n - 2).any(|k| {
            let (ax, ay) = xy(k);
            let (bx, by) = xy(k + 1);
            let (cx, cy) = xy(k + 2);
            ((bx - ax) * (cy - ay) - (by - ay) * (cx - ax)).abs() < 0.5
        }),
        _ => false,
    }
}

/// TSP bit 22: Gouraud, which only the models' states carry.
const TSP_GOURAUD: u32 = 1 << 22;

/// The words of a checked frame with `part` applied, to `put` (`send`'s
/// pieces: a state block's eight words, a primitive's words and its end).
struct Split<F> {
    part: Part,
    put: F,
    /// Words of the state block still to come (TSP is the first).
    state_left: u8,
    /// The block's words after its first (7, or 10 for a two-texture block).
    state_total: u8,
    /// Inside a primitive: dropped or not.
    in_prim: Option<bool>,
    /// The primitives after the last state are dropped.
    dropping: bool,
    /// The last state was a model's.
    model: bool,
    /// A model strip held back until its end (runs 26's parts): its word
    /// and corners.
    held: Option<(u32, usize)>,
    buf: [u32; STRIP_MOST],
    /// Strips left out by `leave_out` this frame.
    left_out: u32,
}

impl<F: FnMut(Piece) -> bool> Split<F> {
    fn put(&mut self, p: Piece) -> bool {
        match p {
            Piece::End => {
                let drop = self.in_prim.take().unwrap_or(false);
                if let Some((word, n)) = self.held.take() {
                    if leave_out(self.part, &self.buf[..n]) {
                        self.left_out += 1;
                        return true;
                    }
                    return (self.put)(Piece::Word(word))
                        && self.buf[..n].iter().all(|&v| (self.put)(Piece::Word(v)))
                        && (self.put)(p);
                }
                drop || (self.put)(p)
            }
            Piece::Word(w) => {
                if let Some((word, n)) = self.held {
                    if let Some(slot) = self.buf.get_mut(n) {
                        *slot = w;
                    }
                    self.held = Some((word, (n + 1).min(STRIP_MOST)));
                    return true;
                }
                if let Some(drop) = self.in_prim {
                    return drop || (self.put)(p);
                }
                if self.state_left > 0 {
                    let k = 1 + self.state_total - self.state_left;
                    self.state_left -= 1;
                    let w = match (k, self.part) {
                        (1, _) => {
                            let model = w & TSP_GOURAUD != 0;
                            self.model = model;
                            self.dropping = match self.part {
                                Part::NoModels => model,
                                Part::NoWorld => !model,
                                _ => false,
                            };
                            if model && self.part == Part::ModelsFlat {
                                w & !TSP_GOURAUD
                            } else {
                                w
                            }
                        }
                        (2, Part::ModelsNoAlphaTest) if self.model => {
                            (w & 0x001F_FFFF) | 0xE000_0000
                        }
                        (3, Part::TinyTextures) => w & !(0xFF << 20),
                        _ => w,
                    };
                    return (self.put)(Piece::Word(w));
                }
                if w == rp_port::gfx::STATE || w == rp_port::gfx::STATE2 {
                    self.state_left = if w == rp_port::gfx::STATE2 { 10 } else { 7 };
                    self.state_total = self.state_left;
                    return (self.put)(p);
                }
                // A strip's or a fan's word.
                let fan = w == rp_port::gfx::FAN;
                let drop = self.dropping
                    || (self.model && self.part == Part::NoModelFans && fan)
                    || (self.model && self.part == Part::NoModelStrips && !fan);
                self.in_prim = Some(drop);
                if !drop
                    && self.model
                    && matches!(self.part, Part::TinyModelStrips | Part::ThinModelStrips)
                {
                    self.held = Some((w, 0));
                    return true;
                }
                drop || (self.put)(p)
            }
        }
    }
}

impl Cell {
    const fn rung(&self) -> clock::Rung {
        let (m, n, p) = self.pll;
        let mut speed = clock::Speed::ladder(m << 7 | n << 4 | p << 1 | 1);
        if let Some(t) = self.lmtim {
            speed.lmtim = t;
        }
        clock::Rung { speed, div: 1 }
    }
}

const P108: (u32, u32, u32) = (25, 3, 0);
const P117: (u32, u32, u32) = (27, 3, 0);
#[allow(dead_code, reason = "run 19's cells")]
const P133: (u32, u32, u32) = (41, 4, 0);
#[allow(dead_code, reason = "runs 18-31's cells")]
const P136: (u32, u32, u32) = (21, 2, 0);
/// The render words Dell's GLES driver renders TA scenes with (rp-wmprobe
/// run 2, docs/hw/2700g.md; GC2F tried them on the binner's wrong pixels
/// only).
#[allow(dead_code, reason = "runs 18-19's cells")]
const DELL_GLES: [(usize, u32); 7] = [
    (0x610, 0x0003_0001),
    (0x614, 0x1E3C_E500),
    (0x618, 0x1E3C_E500),
    (0x61C, 0x0002_0007),
    (0x648, 1),
    (0x6D8, 0x0900_0000),
    (0x6DC, 0x3F80_0001),
];
const NONE: &[(usize, u32)] = &[];
const fn cell(
    pll: (u32, u32, u32),
    lay: &'static Lay,
    words: &'static [(usize, u32)],
    what: &'static str,
    frames: u32,
) -> Cell {
    Cell {
        pll,
        lay,
        words,
        what,
        frames,
        lmtim: None,
        part: Part::All,
        warm: false,
        checker: false,
        overlay: false,
    }
}

#[allow(dead_code, reason = "run 31's cells")]
const fn overlaid(c: Cell) -> Cell {
    Cell { overlay: true, ..c }
}

#[allow(dead_code, reason = "run 28's cells")]
const fn checkered(c: Cell) -> Cell {
    Cell { checker: true, ..c }
}

const fn warmed(c: Cell) -> Cell {
    Cell { warm: true, ..c }
}

#[allow(dead_code, reason = "runs 23-26's cells")]
const fn split(c: Cell, part: Part) -> Cell {
    Cell { part, ..c }
}

#[allow(dead_code, reason = "run 22's cell (no cure)")]
const fn timed(c: Cell, lmtim: u32) -> Cell {
    Cell {
        lmtim: Some(lmtim),
        ..c
    }
}

/// The TA's parameter memory from the die's first free byte on, running
/// on into SDRAM (run 22: whether the lists the manager fills first, on
/// the die, cut the hangs); its tables moved out of its way, below the
/// texture memory.
const PMD_AT: u32 = SCREEN_BYTES;
const PMD_SIZE: u32 = 0x0060_0000 - PMD_AT;
const PMD_TAIL: u32 = 0x007E_0000;
const PMD_REGIONS: u32 = 0x007F_0000;
const PMD_TABLE: u32 = 0x007F_C000;
const _: () = assert!(PMD_AT + PMD_SIZE <= PMD_TAIL && PMD_REGIONS + REGION_BYTES <= PMD_TABLE);
const _: () = assert!(PMD_TAIL.is_multiple_of(0x2_0000) && PMD_AT.is_multiple_of(0x1000));
#[allow(dead_code, reason = "run 22's cell (no cure)")]
const LAY_PM_DIE: Lay = Lay {
    name: "parameter memory from the die on",
    ta: ta::Layout {
        tail: PMD_TAIL,
        tail_bytes: TAIL_BYTES,
        regions: PMD_REGIONS,
        width: W,
        height: H,
    },
    pm: ta::ParamMemory {
        at: PMD_AT,
        size: PMD_SIZE,
        table: PMD_TABLE,
    },
    from: binner::Layout {
        params: PMD_AT,
        params_size: PMD_SIZE,
        regions: PMD_REGIONS,
        w: W as u16,
        h: H as u16,
    },
    keep: [
        (0, SCREEN_BYTES),
        (PMD_TAIL, TAIL_BYTES),
        (PMD_REGIONS, REGION_BYTES),
        (TEX_AT, TEX_BYTES),
    ],
};

/// Marker pixels written before each ladder render (magenta, which no
/// frame of the run draws): which of them the core drew over by the time
/// a render is called hung says whether it finished and only its done
/// bit was lost, or how far through the tiles (rows, top first) it got.
const MARKS: [(u32, u32); 5] = [(0, 0), (478, 160), (0, 320), (478, 480), (478, 639)];
/// Two magenta pixels (a word: each mark's `x` is even).
const MARK: u32 = 0xF81F_F81F;
/// Dell's GLES render words but one (run 19: which of them cut the hangs).
const fn dell_but(off: usize, keep: u32) -> [(usize, u32); 7] {
    let mut w = DELL_GLES;
    let mut k = 0;
    while k < w.len() {
        if w[k].0 == off {
            w[k].1 = keep;
        }
        k += 1;
    }
    w
}
#[allow(dead_code, reason = "run 19's cells")]
const BUT_610: [(usize, u32); 7] = dell_but(0x610, 0);
#[allow(dead_code, reason = "run 19's cells")]
const BUT_614: [(usize, u32); 7] = {
    let mut w = dell_but(0x614, 0);
    w[2] = (0x618, 0);
    w
};
#[allow(dead_code, reason = "run 19's cells")]
const BUT_61C: [(usize, u32); 7] = dell_but(0x61C, 0x0002_0004);
#[allow(dead_code, reason = "run 19's cells")]
const BUT_648: [(usize, u32); 7] = dell_but(0x648, 0);
#[allow(dead_code, reason = "run 19's cells")]
const BUT_6D8: [(usize, u32); 7] = dell_but(0x6D8, 0x0100_0000);
#[allow(dead_code, reason = "run 19's cells")]
const BUT_6DC: [(usize, u32); 7] = dell_but(0x6DC, 0);
/// Run 27: `0x664`'s per-tile wait (bits 0-3; GC2AT: the binner's tile
/// race cut 83% at 15, 2.8 us a step a render) against the TA path's
/// stalls, every other frame.
#[allow(dead_code, reason = "run 27's cells")]
const WAIT_15: &[(usize, u32)] = &[(0x664, 15)];
#[allow(dead_code, reason = "run 27's cells")]
const WAIT_8: &[(usize, u32)] = &[(0x664, 8)];
#[allow(dead_code, reason = "run 27's cells")]
const WAIT_4: &[(usize, u32)] = &[(0x664, 4)];
/// Run 32's fine ladder: the rungs between 117.00 (clean in 40, run 18)
/// and 133.25 / 1 (1 hang in 53 over 10 minutes, run 29), each long
/// enough to call clean or not; the warm-up after a hang as normal play
/// has it.
const P121: (u32, u32, u32) = (28, 3, 0);
const P125: (u32, u32, u32) = (29, 3, 0);
const P130: (u32, u32, u32) = (40, 4, 0);
const CELLS: [Cell; 6] = [
    cell(P108, &LAY_SDRAM, NONE, "control", 40),
    warmed(cell(P117, &LAY_SDRAM, NONE, "fine ladder", 1200)),
    warmed(cell(P121, &LAY_SDRAM, NONE, "fine ladder", 1200)),
    warmed(cell(P125, &LAY_SDRAM, NONE, "fine ladder", 1200)),
    warmed(cell(P130, &LAY_SDRAM, NONE, "fine ladder", 1200)),
    warmed(cell(
        P117,
        &LAY_SDRAM,
        NONE,
        "fine ladder again, warm (heat)",
        400,
    )),
];
/// The ladder starts after this many frames (the map drawn and steady).
const LADDER_FROM: u32 = 32;
/// A ladder render past this has hung (13.2 ms at 108.33 / 1).
const LADDER_LIMIT_US: u32 = 50_000;
/// The render block read at a hang and after a good render.
const BLOCK_AT: usize = 0x600;
const BLOCK_WORDS: usize = 64;

fn read_block(regs: &mut Mmio) -> [u32; BLOCK_WORDS] {
    let mut b = [0u32; BLOCK_WORDS];
    for (k, v) in b.iter_mut().enumerate() {
        let off = BLOCK_AT + 4 * k;
        // The start register is written only.
        if off != m3::START {
            *v = regs.read32(MBX + off);
        }
    }
    b
}

/// One cell's counts.
struct Ladder {
    cell: usize,
    n: u32,
    hangs: u32,
    ta_hangs: u32,
    ok: u32,
    render_us: u32,
    max_us: u32,
    pattern: [u8; 160],
    ok_block: Option<[u32; BLOCK_WORDS]>,
    hang_block: Option<[u32; BLOCK_WORDS]>,
    /// Hangs by which marks the core had drawn over (bit k: `MARKS[k]`).
    marks: [u16; 32],
    /// Hangs on the frames that had the cell's change (every other one).
    hangs_b: u32,
    /// Strips the change left out, all its frames together.
    left_out: u32,
}

impl Ladder {
    const fn new(cell: usize) -> Self {
        Self {
            cell,
            n: 0,
            hangs: 0,
            ta_hangs: 0,
            ok: 0,
            render_us: 0,
            max_us: 0,
            pattern: [b' '; 160],
            ok_block: None,
            hang_block: None,
            marks: [0; 32],
            hangs_b: 0,
            left_out: 0,
        }
    }

    fn mark(&mut self, c: u8) {
        if let Some(p) = self.pattern.get_mut(self.n as usize) {
            *p = c;
        }
        self.n += 1;
    }
}

/// The TA's time from its start to done, the feeding included (a frame
/// of c1a0 is ~70,000 words: ~8 ms of bus).
const TA_LIMIT_US: u32 = 500_000;
/// A render past this has hung: the core is reset. A render takes 13.2
/// ms at 108.33 / 1 and 26 at Windows Mobile's clock; R295's runs saw
/// none finish late (most 16.4 ms at 136.50), so a hang costs 50 ms, not
/// 200.
const RENDER_LIMIT_US: u32 = 50_000;
/// The cores' clock [`RENDER_LIMIT_US`] was set for: the game step's
/// 125.67 / 1.
const LIMIT_CORE_KHZ: u32 = 125_667;

/// A render's limit at the rung the 2700G runs (`None`: Windows Mobile's,
/// 99.67 / 2): [`RENDER_LIMIT_US`] times how much slower the cores run
/// than at the game step, never less. Run 280: after R344's rebuild at
/// Windows Mobile's clock (the cores at ~50 MHz) renders took 35-46 ms on
/// average where they took 15-22 at 117.00 / 1, and every one past 50 ms
/// was called a hang (832 of them, the frame thrown away: the owner's
/// flashing, 19 fps).
fn render_limit_us(rung: Option<clock::Rung>) -> u32 {
    let core = rung
        .map_or(clock::Speed::WM.khz() / 2, clock::Rung::core_khz)
        .max(1);
    let scaled = u64::from(RENDER_LIMIT_US) * u64::from(LIMIT_CORE_KHZ) / u64::from(core);
    scaled.clamp(u64::from(RENDER_LIMIT_US), 400_000) as u32
}
/// R343 (the owner, 2026-11-20: "the blacks looking more mid tone grey
/// then actually black"). Iterate, don't imitate (rule 0): the vendor and
/// textbook lever is a gamma curve (Windows Mobile leaves the table
/// straight; Half-Life's GL its gamma and brightness), and the first picker
/// offered three that lift the darks, which makes a grey black greyer.
/// Measured instead (run 287's last frame both ways on the X51v): the
/// 2700G's picture is as dark as the software renderer's (median 28 of
/// 255 against 26, the darkest 1% at 2.3 in both): the pixels are black,
/// the panel shows them grey: the LCD's light through a black pixel, at
/// the 100% backlight R341 sets. A table can't make a pixel darker than
/// 0, but the backlight can. The same frames peak low (99% of pixels at
/// 74 or under, 99.99% at 165), so ours dims the backlight and gives the
/// light back in the table: slope (100 / level)^(1 / 2.2) at the bottom
/// (the panel taken as gamma 2.2, the midtones as bright as at 100%),
/// rolled off to 255 (g t / (1 + (g - 1) t), so a muzzle flash never
/// clips): the blacks darker by the backlight's own ratio, no cost a frame
/// (the scan-out applies the table), and less power. The "S" looks add a
/// contrast curve under it (half straight, half smoothstep: the darks
/// lower, the midtones where Half-Life's frames live a little higher).
/// Each look is a backlight level and its table. The picker (the owner
/// after run 288: "took to long, hard to compare"): a press of Record
/// steps to the next look, holding it shows NOW until it is let go
/// (a photo editor's before and after, at the same spot), at the game's
/// own clock ([`curve_key`], `-rpcurves`; the name in [`LAY_PICK`]'s
/// band).
const CURVES: [(&str, u32, [u8; 17]); 4] = [
    (
        "NOW",
        100,
        [
            0, 16, 32, 48, 64, 80, 96, 112, 128, 143, 159, 175, 191, 207, 223, 239, 255,
        ],
    ),
    (
        "M70",
        100,
        [
            0, 31, 55, 75, 94, 110, 126, 141, 155, 169, 182, 195, 208, 220, 232, 244, 255,
        ],
    ),
    (
        "M65",
        100,
        [
            0, 36, 61, 82, 100, 117, 133, 147, 161, 174, 187, 199, 211, 222, 233, 244, 255,
        ],
    ),
    (
        "M75",
        100,
        [
            0, 26, 49, 69, 87, 104, 120, 135, 150, 164, 178, 192, 205, 218, 230, 243, 255,
        ],
    ),
];

/// R343: the look a game gets without the picker: M75 with Half-Life's
/// `brightness` 1 (the light lifted, not the picture), the owner's pick
/// from run 292 ("i think i like m75"); M70 his pick from run 291; before it M60, his pick from run 290 ("m60 looks the best too me with the highlights and
/// viewability but the midtones are now lost"): a whole-picture curve
/// trades the midtones' separation for the darks' lift; the next step
/// lifts the light, not the picture (Half-Life's own split: its
/// `brightness` lifts the lightmaps and keeps the textures' contrast).
const LOOK_DEFAULT: usize = 3;

/// R343, the owner's ruling (2026-11-20: "do the smart slider"): one
/// "Picture brightness" setting in the game's options, each step moving
/// the light and the screen's curve together. Iterate, don't imitate
/// (rule 0): Half-Life has two sliders, gamma and brightness, that bake a
/// 1998 formula into every lightmap and need a restart on some renderers,
/// and a player can set them to a muddy or washed-out pair; a phone has
/// one brightness. Ours is one path with no bad place on it: the dark end
/// is the picture as the game made it (a straight table), the middle is
/// the owner's pick on the X51v (M75), the bright end a stronger curve
/// for daylight (gamma 0.55), every step between a blend of its two
/// neighbours; the 2700G's scan-out applies it, so a step shows at once
/// and costs no frame time. The game lifts its light with the same step
/// (ref_rp: `rp_picture`). The two ends are a first fit: the owner's eye
/// on the X51v judges them.
pub const PICTURE_STEPS: u8 = 8;
const PICTURE_MID: u8 = 4;
const PICTURE_DAY: [u8; 17] = [
    0, 55, 81, 102, 119, 134, 149, 162, 174, 186, 197, 208, 218, 227, 237, 246, 255,
];
/// The step's table, once the game has asked for one ([`picture_table`]).
static PICTURE: rt::Global<Option<[u8; 17]>> = rt::Global::new(None);

/// Step `level`'s table: between the straight table and the default
/// look's below the middle, between the default look's and the daylight
/// one's above it.
#[must_use]
pub fn picture_table(level: u8) -> [u8; 17] {
    let level = level.min(PICTURE_STEPS);
    let (a, b, t) = if level <= PICTURE_MID {
        (&CURVES[0].2, &CURVES[LOOK_DEFAULT].2, level)
    } else {
        (&CURVES[LOOK_DEFAULT].2, &PICTURE_DAY, level - PICTURE_MID)
    };
    let (t, n) = (u32::from(t), u32::from(PICTURE_MID));
    core::array::from_fn(|i| ((u32::from(a[i]) * (n - t) + u32::from(b[i]) * t + n / 2) / n) as u8)
}

/// R341/R343: the backlight while a game plays and its registers as found
/// (`CR`, `PCR`, `DCR`, put back at the close): each look of [`CURVES`]
/// sets its level.
pub type Light = (rp_drv_pxa_pwm::Backlight<Mmio>, [u32; 3]);

/// R343's picker: whether `-rpcurves` runs, the Record button held at the
/// last scan, and its presses so far (the keypad's scans count them, the
/// frames step a curve for each).
static CURVE_KEY: rt::Global<(bool, bool, u32, u32, bool)> =
    rt::Global::new((false, false, 0, 0, false));
/// R343: Record held this long (0.4 s) is a compare, not a step.
const HOLD_TICKS: u32 = rt::TICK_HZ / 1000 * 400;

/// R343's picker on (`-rpcurves`): from then on the Record button steps
/// the curves and the game never sees it (its own job, the weapon before,
/// waits: the owner judges dark corners, not weapons; every run without
/// `-rpcurves` keeps the button's job).
pub fn curve_key_on() {
    CURVE_KEY.with(|k| k.0 = true);
}

/// The buttons held as the game gets them: with the picker on, Record
/// taken out and each new press counted.
#[must_use]
pub fn curve_key(held: Pressed) -> Pressed {
    CURVE_KEY.with(|k| {
        if !k.0 {
            return held;
        }
        let r = held.contains(Button::Record);
        let now = rt::ticks();
        if r && !k.1 {
            k.3 = now;
        } else if r && !k.4 && now.wrapping_sub(k.3) >= HOLD_TICKS {
            // held: NOW shown until it is let go
            k.4 = true;
        } else if !r && k.1 {
            if !k.4 {
                k.2 = k.2.wrapping_add(1);
            }
            k.4 = false;
        }
        k.1 = r;
        Button::ALL
            .into_iter()
            .filter(|&b| b != Button::Record && held.contains(b))
            .fold(Pressed::NONE, Pressed::with)
    })
}

/// One pixel of the picker's band at the landscape point (`lx` across
/// from the picture's left edge, `ly` down from its top): the runner's
/// turn (`RP_TURN_RIGHT`: the device's top edge at the player's right) puts
/// it at the upright screen's (`ly`, `H - 1 - lx`).
fn band_px(rows: &mut [u8], lx: u32, ly: u32, c: u16) {
    if lx >= PICK_ROWS || ly >= W {
        return;
    }
    let r = H - 1 - lx - PICK_H;
    let at = ((r * W + ly) * 2) as usize;
    if let Some(p) = rows.get_mut(at..at + 2) {
        p.copy_from_slice(&c.to_le_bytes());
    }
}

/// `text` upright in the band at scale `s` from (`lx0`, `ly0`): its
/// letters stacked down the band, or side by side across it.
fn band_text(rows: &mut [u8], (lx0, ly0): (u32, u32), s: u32, text: &str, c: u16, stacked: bool) {
    use rp_drv_mbx_display::font::{HEIGHT, WIDTH, glyph};
    let (gw, gh) = (WIDTH as u32, HEIGHT as u32);
    for (i, ch) in text.chars().enumerate() {
        let i = i as u32;
        let (x0, y0) = if stacked {
            (lx0, ly0 + i * (gh + 2) * s)
        } else {
            (lx0 + i * (gw + 1) * s, ly0)
        };
        for (gy, &bits) in glyph(ch).iter().enumerate() {
            for gx in 0..gw {
                if bits & (1 << (gw - 1 - gx)) == 0 {
                    continue;
                }
                for d in 0..s * s {
                    band_px(rows, x0 + gx * s + d % s, y0 + gy as u32 * s + d / s, c);
                }
            }
        }
    }
}
/// The gamma table's use bit in `GSCTRL` (Windows Mobile leaves it set).
const GSCTRL_GAMMA: u32 = 1 << 26;

/// R340's escalation: chains failed in a row, or the CPU's own feeds
/// (frames too large for the chain), before the 2700G's full reset and
/// rebuild (R344: run 278 proved a core reset at Windows Mobile's clock
/// does not clear the wedge; the rebuild puts that clock back too,
/// ADR-0035 rule 6).
const CHAIN_FAILS_OFF: u32 = 2;
const CPU_FAILS_CLOCK: u32 = 3;
/// R344: rebuilds said one by one (then only counted).
const REBUILDS_SAID: u32 = 10;
/// R344: the least time between rebuilds (between them a failed chain
/// takes the core's reset only), doubled after each rebuild no finished
/// chain followed, up to [`REBUILD_GAP_MOST_US`], and back to the least at
/// the next finished chain: a wedge a rebuild cures costs ~0.25 s, one it
/// cannot (or the emulator, whose chains never say done) a few rebuilds a
/// minute at most, never the run.
const REBUILD_GAP_US: u32 = 2_000_000;
const REBUILD_GAP_MOST_US: u32 = 64_000_000;

/// R344: whether a rebuild may run now: none yet, or the last `gap` ago.
fn rebuild_due(last: Option<(u32, u32)>) -> bool {
    last.is_none_or(|(t, gap)| us_since(t) >= gap)
}
/// R340: hangs said one by one (then only in the five-second counts).
const HANGS_SAID: u32 = 20;
/// Every this many frames the render is waited for at once and timed.
const TIMED_EVERY: u32 = 16;
/// The background: opaque black to the eye, but one step of blue off it
/// (RGB565 `0x0001`: 8 is the 5-bit channel's 1 by its top bits), so a
/// pixel nothing drew over is told from a dark texel: ref_rp's hole check
/// counts them (the black walls, the owner, runs 37-39).
const BLACK: u32 = 0xFF00_0008;

/// The display's underrun flag (`DINTRS` bit 4, the graphics stream's
/// fetch run short), cleared Dell's way; whether it was set (ADR-0035
/// rule 8: checked after every clock change).
const GFX_UNDERRUN: u32 = 1 << 4;

/// R340: the bus time-out (`rp_port::gfx::SYSCFG`, why and whose way
/// there) and the timed status read.
use rp_port::gfx::{SYSCFG, bus_timed_out, syscfg_timeout_on};

/// R340: the 3D core's events Dell's interrupt handler acts on and ours
/// never did (`ddi_2700g` `0x021192b4`, read 2026-11-20; Windows Mobile
/// enables them all, `0x130` = `0x87F5`; `0x12c` reads raw whatever the
/// enables, run FI, so ours enables them only to match): bit 5, after which Dell writes `0x804` = 1 at once if the 3D
/// stage is idle, else as soon as its render ends (`0x02119088`); bits 7
/// and 8 (the manager's and a context store's done); bit 9, after which
/// Dell's GLES writes `0x834` = 1 and waits for bits 6 and 7. The reading:
/// bit 5 is the TA asking for parameter memory back, which kit 113's
/// overlap (two scenes in one parameter memory) makes possible in a heavy
/// scene: unanswered, the TA stops taking words, the chain's DMA into it
/// never ends and the chip's host path wedges (R340: every free play with
/// overlap stalled at 1-11 minutes, run 279 without it none in 11.5).
const EV_TA_MEM: u32 = 1 << 5;
const EV_7: u32 = 1 << 7;
const EV_8: u32 = 1 << 8;
const EV_9: u32 = 1 << 9;
const EVENTS: u32 = EV_TA_MEM | EV_7 | EV_8 | EV_9;
/// The TA block's word Dell writes 1 to after bit 5.
const TA_MEM_BACK: usize = 0x804;

/// R340's next lever (`-rpmemback`, run 295: the parameter memory is never
/// short, so bit 5 is not the memory running out): with the overlap on,
/// `0x804` = 1 after every render the runner sees end, as if the TA had
/// asked (Dell's driver writes it only then, but runs one scene at a time
/// through its own queue: whether the manager needs telling that a render
/// let its scene go is what this run asks); the writes counted.
pub static MEM_BACK: rt::Global<(bool, u32)> = rt::Global::new((false, 0));

/// R340 (run 297's findings): what the texture-write guard counted since
/// the open ([`Screen::quiet`], `rp_gfx_quiet`).
#[derive(Clone, Copy)]
pub struct Quiet {
    /// `-rptexhold S`: the program's texture writes held while the TA and
    /// a render are both at work, for the first S seconds after the open
    /// (0: the whole run); `None`: never held.
    pub hold_s: Option<u32>,
    /// Writes that waited for the render, and the time waited.
    held: u32,
    held_us: u32,
    /// Looks with the hold on, and those that found both at work.
    looks: u32,
    /// Writes skipped (the chip lost: each word would wait a time-out).
    skipped: u32,
    /// Chains cut by the pump at a silent chip; the last one's descriptor
    /// and time since its arm.
    cuts: u32,
    cut_at: (u32, u32),
    /// R340 after run 330 (18 bit 9 events with no chain in flight and a
    /// render running, a kind no run since the strict order had shown):
    /// the program's texture writes so far ([`Screen::quiet`] calls), so
    /// each event can say how many came while its render ran.
    calls: u32,
}
pub static QUIET: rt::Global<Quiet> = rt::Global::new(Quiet {
    hold_s: None,
    held: 0,
    held_us: 0,
    looks: 0,
    skipped: 0,
    cuts: 0,
    cut_at: (0, 0),
    calls: 0,
});

/// R340 (2026-11-21): the core's event bit 9, the lead after runs
/// 293-298. Counted since run 281 and never answered: 5 events in the run
/// that never stalled (295, standing), 111 to 1,768 in the runs that did
/// (298, 297, 293). Dell's handler turns it into a flag (`ddi_2700g`
/// `0x021192b4`, `+0xc4`) that its GLES acts on before it feeds the TA
/// again (`libGLES_CL` `0x0227038c`): the scene ended early (`0xF0000000`
/// into the TA with `0x85C` bit 7), the TA's done waited for, `0x834` = 1,
/// bits 6 and 7 waited for, then the render. Dell also asks the TA's FIFO
/// for room before every word; ours feeds a whole frame by DMA and lets
/// the chip hold the bus while its FIFO is full. The reading: a TA that
/// raised bit 9 has stopped taking words until it is answered; the DMA
/// fills the FIFO behind it (a stall begins 40 to 80 descriptors into
/// the TA's words: runs 276, 278, 298), the next write holds the bus for
/// good, and every other access, the status reads too, times out behind
/// it: the silent chip. Unproven: what is kept here is where in a chain
/// each event comes, and at a stall how long before it the last one was;
/// and a lever (`-rpev9`): `0x834` = 1 once no render is in flight, as
/// bit 5's `0x804`.
#[derive(Clone, Copy)]
pub struct Ev9 {
    /// `-rpev9`: answered with `0x834` = 1.
    pub answer: bool,
    owed: bool,
    answered: u32,
    /// Events seen with no chain in flight, in a chain's first 128
    /// descriptors, and later in it.
    idle: u32,
    early: u32,
    late: u32,
    /// The last event: when (OS timer ticks) and at which descriptor
    /// (`u32::MAX`: no chain).
    last: Option<(u32, u32)>,
}
pub static EV9: rt::Global<Ev9> = rt::Global::new(Ev9 {
    answer: false,
    owed: false,
    answered: 0,
    idle: 0,
    early: 0,
    late: 0,
    last: None,
});
const TA_EV9_ANSWER: usize = 0x834;

/// R352 after run 332 (the owner's clip: the steel ribs on the far top of
/// c2a5f's walls flash in and out, the nearer ones steady: a few units in
/// front of their wall at 1,500 to 3,000 units, ours sends depth as 1/z,
/// there 3e-4 to 7e-4 apart by 1e-6). The TA's start table copies Dell's
/// `0x854`/`0x858` = 1.0e-5, read as "a depth or clipping epsilon" and
/// never measured: if it is a depth tolerance, 1/z values that close
/// compare equal and the later primitive wins in some tiles, the flashing.
/// `-rpeps S`: from S seconds into the run both words 1.0e-9 instead, so
/// one hands-free run shows the same view both ways (the first switch
/// logged). `None`: Dell's.
pub static TA_EPS: rt::Global<Option<(u32, u32, bool)>> = rt::Global::new(None);
/// 1.0e-9 as a float.
pub const TA_EPS_SMALL: u32 = 0x3089_705F;

/// The value for the TA's `0x854`/`0x858` now, if `-rpeps` has switched.
pub fn ta_eps_now(log: fn(Arguments<'_>)) -> Option<u32> {
    TA_EPS.with(|e| match e {
        Some((t0, s, said)) if us_since(*t0) / 1_000_000 >= *s => {
            if !*said {
                *said = true;
                log(format_args!(
                    "rp-gfx: R352 -rpeps: the TA's 0x854/0x858 now 1.0e-9 (Dell's 1.0e-5 until {} s)",
                    *s
                ));
            }
            Some(TA_EPS_SMALL)
        }
        _ => None,
    })
}

/// R340, the rule run 310 measured (the console over the menu, each frame
/// fed its first N strips): the 2700G's event bit 9 comes exactly when
/// the TA runs longer than the render in flight has left (none while the
/// TA took 7.5 ms or less, in every frame from 8.8 ms, the render there
/// about 7.7 ms; the text's half alone, a 6.8 ms TA, none): a render
/// ending under a running TA. The stalls are that moment going wrong (one
/// in a few hundred of them in play: runs 297 and 305). Iterate, don't
/// imitate (rule 0): Dell's driver never has the two together (one scene
/// at a time through its queue: the TA, then its render), which costs the
/// overlap's 12 fps (run 279); kit 113 ran them together always. Ours
/// keeps them together wherever the render will outlast the TA (the
/// common case in a game: the TA's 7 to 10 ms inside a 10 to 25 ms
/// render) and, only when the render in flight would end first, waits
/// those few milliseconds for it before arming the TA. The two times are
/// the last frame's own, measured (frames follow each other closely); the
/// events left over are the rule's misses, counted as before.
/// `-rpnoholdend`: kit 113 as it was.
///
/// The wait itself (run 312: the rule waited inside the frame call in
/// 16,524 of 23,750 frames, 4.6 ms each: 12.7% of the run, 40.0 fps where
/// run 297 had 45.7). Iterate, don't imitate (rule 0): a driver that must
/// order two chip stages blocks the caller until the first is over (Dell's
/// queue does, and so did this rule's first build); but nothing in the
/// wait needs the CPU: the frame's chain is pointers into buffers the
/// program keeps until its next frame call anyway. So ours builds the
/// chain at the call, touches no chip register, and returns; the pump
/// (the 1 ms tick and the program's clock reads) arms it the moment the
/// render in flight is seen ended, and the next frame call arms it first
/// if no pump got there. The TA still never starts under a render that
/// would end first, and the game's CPU makes its next frame meanwhile.
/// `-rpnodefer`: the wait inside the call, as run 312 had it.
#[derive(Clone, Copy)]
pub struct HoldEnd {
    pub on: bool,
    /// The arm put off to the pump (`-rpnodefer`: waited for in the call).
    pub defer: bool,
    /// A TA armed beside a render the estimate says will outlast it
    /// (`-rpbeside`; runs 312 and 314's way). Off: no TA ever starts while
    /// a render runs, so none can end under one. Run 314 (the owner's
    /// play, the wait put off): the estimate is the last frame's render
    /// and TA times, and 18,387 frames armed on it gave 871 bit 9 events
    /// and 2 stalls (run 312: 2,718 and 49): a guess at how long the chip
    /// will take, wrong for a few frames in a hundred when the view turns.
    /// Rule 0: the vendor's queue and this rule's first builds order the
    /// two stages by a prediction or by blocking the caller; with the arm
    /// put off, strict order costs the CPU nothing (it builds the next
    /// frame meanwhile), so ours takes the order that is certain.
    pub beside_ok: bool,
    /// Frames whose TA waited for the render to end, and the time waited
    /// (from the frame call to the arm).
    held: u32,
    held_us: u32,
    /// Of them: the frames the next frame call had to wait for (no pump
    /// saw the render's end first), and that wait.
    in_call: u32,
    in_call_us: u32,
    /// Frames armed beside a render that would outlast the TA.
    beside: u32,
    /// The margin's measure, each held frame judged once its own TA is
    /// done: the render really ended `d` after the call, the TA then took
    /// `ta`. `needed`: d < ta (armed at the call, the render would have
    /// ended under it). `unsure`: within the pump's 1 ms of it.
    /// `needless`: d >= ta + 1 ms (the render would have outlasted the
    /// TA), and by how much in all.
    needed: u32,
    unsure: u32,
    needless: u32,
    needless_us: u32,
}
pub static HOLD_END: rt::Global<HoldEnd> = rt::Global::new(HoldEnd {
    on: true,
    defer: true,
    beside_ok: false,
    held: 0,
    held_us: 0,
    in_call: 0,
    in_call_us: 0,
    beside: 0,
    needed: 0,
    unsure: 0,
    needless: 0,
    needless_us: 0,
});
/// The margin on the estimate: the pump sees a render's end up to 1 ms
/// late, and a frame's TA may run a little longer than the last one's.
const END_MARGIN_US: u32 = 2_500;

/// R340 (run 305): a frame's primitives fed by class, solid first, then
/// blended ([`chain::Chain`]'s build); `-rpnosort` the program's order.
pub static CLASS_SORT: rt::Global<bool> = rt::Global::new(true);

/// R340's search (`-rpbisect`, runs 303-307: bit 9 comes once a frame
/// while the console's text is drawn, and two guesses at why were wrong).
/// Rule 0: a vendor would read the chip's documents for the event's
/// meaning; ours, without them, lets the driver find the primitive: once
/// the frames are steady (800 strips or more), each frame is fed only up
/// to a limit, the limit halved by whether the events still come (72
/// frames a step), down to the one primitive whose presence starts them;
/// its words and its state block's are logged. Then the frames whole
/// again.
#[derive(Clone, Copy)]
pub struct Bisect {
    pub on: bool,
    /// Strips of a frame fed (`u32::MAX`: all).
    pub limit: u32,
    /// Strips skipped at a frame's start (the search's last steps: which
    /// strips, not how many).
    pub skip: u32,
    /// Since the step began: the TA's time summed and counted (arm to
    /// done), and frames armed with a render still in flight.
    pub ta_us: u32,
    pub ta_n: u32,
    pub over: u32,
    /// A frame's strips, as the last build counted them.
    pub total: u32,
    /// The last strip fed and the state block over it, as built.
    pub last: [u32; 28],
    lo: u32,
    hi: u32,
    n: u32,
    steady: u32,
    ev0: u32,
    phase: u8,
}
pub static BISECT: rt::Global<Bisect> = rt::Global::new(Bisect {
    on: false,
    limit: u32::MAX,
    skip: 0,
    ta_us: 0,
    ta_n: 0,
    over: 0,
    total: 0,
    last: [0; 28],
    lo: 0,
    hi: 0,
    n: 0,
    steady: 0,
    ev0: 0,
    phase: 0,
});

/// One frame of the search: `ev9` the events so far.
fn bisect_step(ev9: u32, pages: [u32; 2], log: fn(Arguments<'_>)) {
    BISECT.with(|b| {
        if !b.on || b.phase == 3 {
            return;
        }
        if b.phase == 0 {
            // steady frames first (the console over the menu)
            b.steady = if b.total >= 800 { b.steady + 1 } else { 0 };
            if b.steady < 300 {
                return;
            }
            b.phase = 1;
            b.lo = 0;
            b.hi = b.total;
            b.limit = b.total;
            b.n = 0;
            log(format_args!(
                "rp-gfx: R340 bisect: steady frames of {} strips; the search begins (whole frames first)",
                b.total
            ));
            return;
        }
        b.n += 1;
        if b.n == 8 {
            b.ev0 = ev9;
            b.ta_us = 0;
            b.ta_n = 0;
            b.over = 0;
        }
        if b.n < 72 {
            return;
        }
        let ev = ev9.wrapping_sub(b.ev0);
        b.n = 0;
        let ta = b.ta_us / b.ta_n.max(1);
        if b.phase >= 4 {
            // which strips, not how many
            log(format_args!(
                "rp-gfx: R340 bisect: strips {} to {} of {} fed ({}): {ev} events in 64 frames; the TA {ta} us a frame, a render in flight at {} of 64 arms; scenes {} and {} pages",
                b.skip + 1,
                b.limit.min(b.total),
                b.total,
                match b.phase {
                    4 => "the second half",
                    5 => "the last 300: text",
                    _ => "the first 300: the menu",
                },
                b.over,
                pages[0],
                pages[1]
            ));
            match b.phase {
                4 => {
                    b.phase = 5;
                    b.skip = b.total.saturating_sub(300);
                }
                5 => {
                    b.phase = 6;
                    b.skip = 0;
                    b.limit = 300;
                }
                _ => {
                    b.phase = 3;
                    b.skip = 0;
                    b.limit = u32::MAX;
                }
            }
            return;
        }
        if b.phase == 2 {
            // the last step fed strips up to `hi`: its last one is the one
            let l = b.last;
            log(format_args!(
                "rp-gfx: R340 bisect: the events begin with strip {} of {} ({ev} events in 64 frames with it): its state block {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}; the strip {:08x}, its first words {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
                b.hi, b.total, l[0], l[1], l[2], l[3], l[4], l[5], l[6], l[7], l[8], l[9], l[10], l[11],
                l[12], l[13], l[14], l[15], l[16], l[17], l[18], l[19], l[20], l[21], l[22], l[23], l[24], l[25], l[26], l[27]
            ));
            b.phase = 4;
            b.limit = b.total;
            b.skip = b.total / 2;
            return;
        }
        log(format_args!(
            "rp-gfx: R340 bisect: {} of {} strips fed: {ev} events in 64 frames; the TA {ta} us a frame, a render in flight at {} of 64 arms; the last two scenes took {} and {} pages",
            b.limit, b.total, b.over, pages[0], pages[1]
        ));
        let present = ev >= 8;
        if b.limit == b.total && b.lo == 0 && b.hi == b.total && !present {
            log(format_args!("rp-gfx: R340 bisect: no events with whole frames: nothing to search"));
            b.phase = 3;
            b.limit = u32::MAX;
            return;
        }
        if b.limit != b.total || b.hi != b.total || b.lo != 0 {
            if present {
                b.hi = b.limit;
            } else {
                b.lo = b.limit;
            }
        }
        if b.hi - b.lo <= 1 {
            b.phase = 2;
            b.limit = b.hi;
        } else {
            b.limit = (b.lo + b.hi) / 2;
        }
    });
}

/// R340: the lever's write after a render seen ending.
fn mem_back(regs: &mut Mmio, overlap: bool) {
    if overlap
        && MEM_BACK.with(|m| {
            if m.0 {
                m.1 += 1;
            }
            m.0
        })
    {
        regs.write32(MBX + TA_MEM_BACK, 1);
    }
}

/// R340 (runs 281-286): reads of 1 ms or more ([`MUTE_SLOW_TICKS`]), this many in a
/// row: the chip's host path is wedged, every access will wait as long,
/// and only the full rebuild clears it. A length can't tell it (run 281's
/// slowest at the wedge 9.9 ms, runs 283-284's under 3 ms, and the 1 ms
/// pump starved the program either way); a run of them can: an answered
/// read takes under 2 us on the X51v, the emulator's slow ones (~1 ms at
/// most) come one at a time. Kept by [`status`] (whether the rebuild is
/// owed, the time-outs in a row), taken by the pump and the frame.
const MUTE_ROW: u32 = 4;
/// R340 (run 286): a read counts toward [`MUTE_ROW`] at 1 ms or longer:
/// in play the X51v's slowest answered reads took up to 470 us (the DMA's
/// bursts ahead of them), four of those in a row called a healthy chip
/// wedged; a read the pump can starve the program with takes the tick's
/// whole millisecond.
const MUTE_SLOW_TICKS: u32 = 3_250;
static MUTE: rt::Global<(bool, u32)> = rt::Global::new((false, 0));

/// R340: whether reads found the chip wedged since the last look.
fn mute_take() -> bool {
    MUTE.with(|m| core::mem::replace(&mut m.0, false))
}

/// R340: the slowest status read since the last report (OS timer ticks),
/// and the reads ended by the time-out.
static READS: rt::Global<(u32, u32)> = rt::Global::new((0, 0));

/// R340: [`READS`] as the logs say them: the slowest in microseconds, and
/// the time-outs.
fn reads() -> (u32, u32) {
    READS.with(|r| (r.0 * 4 / 13, r.1))
}

/// R340: the 3D core's status word, or `None` if the read was ended by
/// the bus time-out (the chip did not answer: the word is no answer, and
/// the waits' own limits decide the hang).
fn status_word(regs: &mut Mmio) -> (u32, bool) {
    let t0 = rt::ticks();
    let v = regs.read32(MBX + reg::STATUS);
    let d = rt::ticks().wrapping_sub(t0);
    let slow = bus_timed_out(d);
    MUTE.with(|m| {
        m.1 = if d >= MUTE_SLOW_TICKS {
            m.1.saturating_add(1)
        } else {
            0
        };
        if m.1 >= MUTE_ROW {
            m.0 = true;
        }
    });
    READS.with(|r| {
        r.0 = r.0.max(d);
        r.1 = r.1.wrapping_add(u32::from(slow));
    });
    (v, slow)
}

/// R340: the 3D core's status word, or `None` if the read was ended by
/// the bus time-out ([`status_word`]).
fn status(regs: &mut Mmio) -> Option<u32> {
    let (v, slow) = status_word(regs);
    (!slow).then_some(v)
}

/// R343: look `k`: its backlight level and its gamma table, each point
/// grey (`0x00RRGGBB`); look 0 is the table as found, word for word.
fn curve_set(regs: &mut Mmio, k: usize, found: &[u32; 17]) {
    // the game's own step of the brightness path in the default look's place
    let picture = PICTURE.with(|p| *p).filter(|_| k == LOOK_DEFAULT);
    let points = picture.as_ref().unwrap_or(&CURVES[k].2);
    for (i, &v) in points.iter().enumerate() {
        let w = if k == 0 {
            found[i]
        } else {
            u32::from(v) * 0x01_0101
        };
        regs.write32(rp_drv_mbx_display::reg::GFXGAM0 + 4 * i, w);
    }
}

fn underrun_take(regs: &mut Mmio) -> bool {
    ack_events(regs, GFX_UNDERRUN) & GFX_UNDERRUN != 0
}

/// R344: run Z's reset and rebuild, then the 3D clock on as the open
/// turns it on and the underrun flag cleared (ADR-0035 rule 8). Leaves
/// Linux's `SYSCFG` (the bus time-out on); the close puts the found back.
/// The PXA's CS3 bus timing the long way through the reset (the chip on
/// its reset clocks), Windows Mobile's after (the clock it rebuilds to),
/// as `clock::put_back` does.
fn rebuild_chip(regs: &mut Mmio) -> rp_drv_mbx_display::rebuild::Rebuilt {
    set_cs3(clock::CS3_LONG);
    let r = rp_drv_mbx_display::rebuild::run(regs, rt::delay_us);
    set_cs3(clock::CS3_WM);
    // our time-out, not Linux's longest the rebuild leaves (run 280: 8.9 s
    // an access to a wedged chip)
    let cfg = regs.read32(SYSCFG);
    regs.write32(SYSCFG, syscfg_timeout_on(cfg));
    let clk = regs.read32(MBXCLK);
    clock::restore(regs, with_3d(clk));
    rt::delay_us(1_000);
    let _ = underrun_take(regs);
    r
}

/// R344: a rebuild's line for the log.
fn rebuilt_line(
    log: fn(Arguments<'_>),
    why: &str,
    n: u32,
    ok: bool,
    us: u32,
    r: &rp_drv_mbx_display::rebuild::Rebuilt,
) {
    log(format_args!(
        "rp-gfx: R344: the 2700G reset and rebuilt ({n}, {why}) in {} ms: PLLSTAT 0x{:x} LMPWRSTAT 0x{:x}, {} writes read back differently, DINTRS 0x{:x}, the display {} (DSIG 0x{:x} then 0x{:x}); the TA {}; Windows Mobile's 2700G clock from here",
        us / 1000,
        r.pllstat,
        r.lmpwrstat,
        r.off_by,
        r.dintrs,
        if r.sending() {
            "sending"
        } else {
            "NOT sending"
        },
        r.dsig.0,
        r.dsig.1,
        if ok { "ready" } else { "NOT ready" }
    ));
}

/// CS3's bus timing for a clock step (`clock::step`, `clock::put_back`).
fn set_cs3(c: clock::Cs3) {
    let _ = rp_hal_pxa27x::diag::set_cs3_vlio(c.0, c.1, c.2);
}

/// The fastest system clock with the cores at divide 1 a game may run at
/// on this unit: the owner's game step, 136.50 / 1 (ADR-0035 amendment D,
/// "The game step", ruled 2026-11-09; 137.80 / 1 the proven ceiling).
pub const GAME_STEP_KHZ: u32 = 136_500;
/// ADR-0035's top rung for the system clock at divide 2 (149.50 / 2).
pub const TOP_KHZ: u32 = 149_500;

/// Whether `rung` is one a test run may ask for: divide 1 up to the game
/// step, divide 2 up to the ladder's top; never past either.
#[must_use]
pub const fn rung_allowed(rung: clock::Rung) -> bool {
    let khz = rung.speed.khz();
    match rung.div {
        1 => khz <= GAME_STEP_KHZ,
        2 => khz <= TOP_KHZ,
        _ => false,
    }
}

fn us_since(t0: u32) -> u32 {
    rt::ticks().wrapping_sub(t0) / 13 * 4
}

/// The background under every frame: a strip over the whole screen (the
/// GLES clear's colourless object draws nothing: audit C80), depth
/// written at 0 so every face (z = 1/w, compared "greater or equal")
/// passes over it.
/// ADR-0035 E's alpha-0 overlay as a TA strip: the background's form,
/// blended (source alpha, one less source alpha), depth always and not
/// written, every corner's colour 0 (alpha 0): it changes no pixel.
fn overlay() -> [u32; 26] {
    let mut s = background();
    s[1] = m3::blend(
        m3::depth_test(0x2260_0E80, m3::depth::ALWAYS, false),
        m3::factor::SRC_ALPHA,
        m3::factor::INV_SRC_ALPHA,
    );
    for k in 0..4 {
        s[6 + 5 * k + 4] = 0;
    }
    s
}

fn background() -> [u32; 26] {
    let mut s = [0u32; 26];
    s[..6].copy_from_slice(&[
        0x1000_0053,
        m3::depth_test(0x2260_0E80, m3::depth::ALWAYS, true),
        m3::ISP_ALWAYS,
        ta::clip(W, H),
        0,
        ta::STRIP,
    ]);
    let one = 1.0f32.to_bits();
    let f = |v: i32| fixed::f32_bits(v, 0);
    for (k, (x, y)) in [(0, 0), (W as i32, 0), (0, H as i32), (W as i32, H as i32)]
        .into_iter()
        .enumerate()
    {
        s[6 + 5 * k..11 + 5 * k].copy_from_slice(&[f(x), f(y), 0, one, BLACK]);
    }
    s
}

/// Counted since the last report.
#[derive(Clone, Copy, Default)]
struct Counts {
    frames: u32,
    words: u32,
    strips: u32,
    /// The CPU's feeding, the scene's end to the TA's done.
    feed_us: u32,
    ta_us: u32,
    /// The CPU waiting for the last render before a feed.
    waited_us: u32,
    /// Renders waited for at once ([`TIMED_EVERY`]) and their time.
    timed: u32,
    render_us: u32,
    refused: u32,
    ta_hangs: u32,
    render_hangs: u32,
    /// Step 4f: frames fed by the chain, their descriptors, the time from
    /// the arm to the TA's done (us), how many the pump found done (the
    /// rest waited for at the next frame), frames the chain could not
    /// hold (fed by the CPU), and time waiting at a frame's start for the
    /// last chain.
    chained: u32,
    descriptors: u32,
    chain_us: u32,
    pumped: u32,
    unfit: u32,
    settle_us: u32,
    /// `-rpchaincheck` (kit 63): frames fed both ways, pictures the same,
    /// different, and checks that could not finish (a hang).
    check_same: u32,
    check_differ: u32,
    check_failed: u32,
    /// Kit 113: renders a frame's start had to wait for, and their whole
    /// time from start to seen done (the chip's own render time); the
    /// chained frames' start-late bound summed ([`Chain::last_gap_us`]);
    /// the pump's looks at frames in flight.
    rwaits: u32,
    rwait_us: u32,
    /// R340: hangs of either kind since the screen opened (not reset by
    /// the reports).
    run_hangs: u32,
    /// R340: chains that did not finish in a row, and the CPU's feeds
    /// the TA never finished in a row (the escalation's counts; kept
    /// across the reports).
    chain_fails_row: u32,
    cpu_fails_row: u32,
    /// R340: events seen since the screen opened (bits 5, 7, 8, 9), and
    /// `0x804` writes made for bit 5.
    ev: [u32; 4],
    ev_answered: u32,
    /// R344: full resets and rebuilds since the screen opened, and those
    /// after which the TA's memory manager came up.
    rebuilds: u32,
    rebuilds_ok: u32,
    gap_us: u32,
    /// R340 (run 281): a chain failed in a way a core reset makes worse
    /// (the TA stopped for memory, bit 5; the DMA never finished; the
    /// chip not answering): the full rebuild at the next frame, the pump
    /// leaving the chip alone until then.
    rebuild_owed: bool,
    /// R340's fits check since the last report: the most pages a scene
    /// took, the most two scenes held at once, and frames held back (the
    /// next scene's TA after the render) for room.
    pages_most: u32,
    live_most: u32,
    held_for_room: u32,
}

/// The screen while the program has it.
struct Open {
    vram: &'static mut [u8],
    ready: ta::Ready,
    /// `MBXCLK` as found, put back at the close.
    clk0: u32,
    /// R340: `SYSCFG` as found, put back at the close.
    syscfg0: u32,
    gfx: Gfx,
    /// A render started and not yet seen done: when it started.
    rendering: Option<u32>,
    /// R340 after run 330: the texture writes counted when the render in
    /// flight began ([`Quiet`]'s `calls`).
    render_writes: u32,
    /// `ENABLE` before the render's done bit went on.
    en: u32,
    /// The panel's plane, where the program's picture moved it from.
    moved: Option<Plane>,
    /// The clock was stepped at the open: put back at the close.
    stepped: bool,
    /// The rung the 2700G runs now (`None`: as the loader left it).
    cur: Option<clock::Rung>,
    lay: &'static Lay,
    /// Render words in place of the start table's (R295's cells).
    words: &'static [(usize, u32)],
    /// Whether this frame's render takes [`Self::words`] (run 27: every
    /// other frame, the frames between them the control).
    words_on: bool,
    /// A render past this has hung.
    limit_us: u32,
    /// The render block as the last hang left it, read before the reset.
    hang_block: Option<[u32; BLOCK_WORDS]>,
    /// Marks written before each render (the ladder's cells).
    marking: bool,
    /// Which marks the core had drawn over at the last hang.
    hang_marks: u32,
    /// What of each frame goes to the chip (the ladder's cells).
    part: Part,
    /// A warm-up render after each hang (the cell's), and one owed.
    warm: bool,
    warm_owed: bool,
    /// The cell reorders the region array (run 28), and this frame does.
    checker: bool,
    checker_on: bool,
    /// The copy's time, summed (CPU us), and copies.
    checker_us: u32,
    checker_n: u32,
    /// The cell draws the alpha-0 overlay (run 31), and this frame does.
    overlay: bool,
    overlay_on: bool,
    /// The frame-rate bar: frames and hangs since it was last drawn, and
    /// when; the hangs since the screen opened.
    bar: bool,
    bar_t0: u32,
    bar_frames: u32,
    /// R343: the curve shown (`-rpcurves`), since when, and the table and
    /// `GSCTRL` as found (put back at the close).
    curve: Option<(usize, u32, [u32; 17], u32)>,
    /// R343: NOW shown while Record is held ([`CURVE_KEY`]).
    comparing: bool,
    /// R343: the Record presses already stepped ([`CURVE_KEY`]).
    curve_seen: u32,
    /// R340: bit 5 seen while a render ran: `0x804` owed at its end.
    mem_owed: bool,
    bar_hangs: u32,
    /// Kit 113 (ref-rp.md): the TA takes frame N + 1 while the 3D stage
    /// draws frame N, the two scenes in two tail buffers and region
    /// arrays ([`TAIL_B`]); `-rpnooverlap` the old way (each frame's TA
    /// after the last render).
    overlap: bool,
    /// The scene the next chain goes into (true: the second), the one the
    /// chain in flight went into, the one the render in flight (or the
    /// next started) draws, and a scene the TA finished waiting for the
    /// render before it to end.
    next_b: bool,
    armed_b: bool,
    render_b: bool,
    pending: Option<bool>,
    /// R340's measure: the frame and the open's time, for [`pm_snap`].
    pm_frame: u32,
    pm_log: fn(Arguments<'_>),
    pm_opened: u32,
    /// R340's fits check: the TA's page cursor when the chain in flight
    /// was armed, the pages each scene (A, B) took, and a recent scene's
    /// most (decaying).
    arm_cursor: Option<u32>,
    scene_pages: [u32; 2],
    /// R340 (run 310): the last render's time from its start to being
    /// seen done, and the last TA's from its arm to its done (the next
    /// frame's are taken to be alike).
    render_us: u32,
    ta_us: u32,
    /// R340's put-off arm: when the frame waiting for its arm was built
    /// (ticks), and, for the chain in flight, how long its arm was put
    /// off (the margin's measure).
    defer_t0: Option<u32>,
    defer_us: Option<u32>,
    pages_recent: u32,
    /// R340's measure (`-rpoverlapat S`): kit 113's overlap off until S
    /// seconds after the open, then on (one run, both ways, the manager
    /// watched in each); `None`: as [`Self::overlap`] says from the start.
    overlap_from: Option<u32>,
}

/// The 2700G as `rp_port`'s [`Screen`].
pub struct Chip {
    /// The 2700G's registers (none without a display).
    regs: Option<Mmio>,
    open: Option<Open>,
    log: fn(Arguments<'_>),
    counts: Counts,
    since: u32,
    /// R340: when the screen was opened (the hang lines' time).
    opened: u32,
    frame: u32,
    /// The 2700G's clock while the program draws (a test run's
    /// `-rpclock`), or the loader's as found.
    rung: Option<clock::Rung>,
    /// R295's diagnostic ladder asked for (`-rpladder`), and where it is.
    ladder_on: bool,
    /// R343: the curves stepped through (`-rpcurves`; the bar on with it).
    curves: bool,
    /// The frame-rate bar asked for (`-rpfpsbar`).
    bar: bool,
    ladder: Option<Ladder>,
    /// C2: the core's turbo multiplier while the program has the screen
    /// (`-rpcpu`), or Dell's 624 MHz.
    cpu: Option<u16>,
    /// Step 4f's probe (`-rp4f`).
    probe: Option<Probe>,
    /// Step 4f: the frames fed by one DMA chain each (`None`: by the CPU).
    chain: Option<Chain>,
    /// `-rpchaincheck`: frames left to feed both ways (the CPU's picture
    /// hashed, then the chain's, compared; kit 63's check that the chain's
    /// own clear and scene start draw what the CPU's do).
    chain_check: u32,
    /// Kit 113's overlap asked for (off with `-rpnooverlap`).
    overlap: bool,
    /// R340's measure (`-rpoverlapat S`): the overlap from S seconds on.
    overlap_at: Option<u32>,
    /// R341/R343: the backlight (the looks set it), put back at the close.
    light: Option<Light>,
    /// R346: the last frame's call (OS timer ticks), for the sampler's
    /// slow frames.
    last_frame: u32,
    /// R344's test (`-rprebuild S`): one full reset and rebuild forced S
    /// seconds after the screen opened, on a healthy chip, so the cure is
    /// proven (the emulator, the X51v) without waiting for a stall.
    rebuild_at: Option<u32>,
    /// How often the counts are said (5 s; free play 15 s: run 279's 15
    /// minutes filled the card log's 256 KiB at 11.5).
    report_us: u32,
    /// R344: when the last rebuild ran, and the gap before the next may
    /// ([`REBUILD_GAP_US`]); `None` once a chain finished after it.
    rebuilt_at: Option<(u32, u32)>,
}

impl Chip {
    /// No screen yet: [`Screen::open`] takes it from the panel. `log`
    /// writes a line to the run's log; `regs` are the 2700G's registers.
    pub const fn new(log: fn(Arguments<'_>), regs: Option<Mmio>) -> Self {
        Self {
            regs,
            open: None,
            log,
            counts: Counts {
                frames: 0,
                words: 0,
                strips: 0,
                feed_us: 0,
                ta_us: 0,
                waited_us: 0,
                timed: 0,
                render_us: 0,
                refused: 0,
                ta_hangs: 0,
                render_hangs: 0,
                chained: 0,
                descriptors: 0,
                chain_us: 0,
                pumped: 0,
                unfit: 0,
                settle_us: 0,
                check_same: 0,
                check_differ: 0,
                check_failed: 0,
                rwaits: 0,
                rwait_us: 0,
                run_hangs: 0,
                chain_fails_row: 0,
                cpu_fails_row: 0,
                ev: [0; 4],
                ev_answered: 0,
                rebuilds: 0,
                rebuilds_ok: 0,
                gap_us: 0,
                rebuild_owed: false,
                pages_most: 0,
                live_most: 0,
                held_for_room: 0,
            },
            since: 0,
            opened: 0,
            frame: 0,
            rung: None,
            ladder_on: false,
            curves: false,
            ladder: None,
            bar: false,
            cpu: None,
            probe: None,
            chain: None,
            chain_check: 0,
            overlap: true,
            overlap_at: None,
            light: None,
            last_frame: 0,
            rebuild_at: None,
            rebuilt_at: None,
            report_us: 5_000_000,
        }
    }

    /// The counts said every `us` (free play's long runs: 15 s).
    #[must_use]
    pub const fn with_report_every(mut self, us: u32) -> Self {
        self.report_us = us;
        self
    }

    /// R344's test: a rebuild forced `s` seconds after the open.
    #[must_use]
    pub const fn with_rebuild_at(mut self, s: Option<u32>) -> Self {
        self.rebuild_at = s;
        self
    }

    /// Kit 113: the TA and the 3D stage at work on two frames at once
    /// (`false`: `-rpnooverlap`).
    #[must_use]
    pub const fn with_overlap(mut self, on: bool) -> Self {
        self.overlap = on;
        self
    }

    /// R341/R343: the backlight the runner set for the game.
    #[must_use]
    pub fn with_light(mut self, light: Option<Light>) -> Self {
        self.light = light;
        self
    }

    /// R340's measure: the overlap off until `s` seconds after the open,
    /// then on (`-rpoverlapat S`).
    #[must_use]
    pub const fn with_overlap_at(mut self, s: Option<u32>) -> Self {
        self.overlap_at = s;
        self
    }

    /// `-rpchaincheck`: the next `frames` frames fed both ways, compared.
    #[must_use]
    pub const fn with_chain_check(mut self, frames: u32) -> Self {
        self.chain_check = frames;
        self
    }

    /// Step 4f: frames fed by a DMA chain ([`Chain`]; `None`: by the CPU).
    #[must_use]
    pub fn with_chain(mut self, chain: Option<Chain>) -> Self {
        self.chain = chain;
        self
    }

    /// Step 4f: the frame in flight waited for, its render started (or
    /// the core reset if it failed). The time waited is counted.
    fn settle(&mut self) {
        let (Some(o), Some(regs), Some(ch)) =
            (self.open.as_mut(), self.regs.as_mut(), self.chain.as_mut())
        else {
            return;
        };
        if !ch.busy() {
            return;
        }
        if self.counts.rebuild_owed {
            // the chip is silent: a frame never armed is not sent into it
            ch.forget();
            o.defer_t0 = None;
        }
        let t0 = rt::ticks();
        // R340: a frame still waiting for its arm is armed first, the
        // render in flight waited for here
        Self::arm_waiting(&mut self.counts, o, regs, ch, true);
        let s = ch.settle(regs);
        self.counts.settle_us += us_since(t0);
        Self::settled(
            &mut self.counts,
            self.log,
            o,
            regs,
            s,
            false,
            ch.last_gap_us,
            ch.last_fail.take(),
        );
    }

    /// R340's put-off arm: the frame built and waiting armed, once the
    /// render in flight has ended (`wait`: waited for here; else a look,
    /// and nothing done while it still runs).
    fn arm_waiting(c: &mut Counts, o: &mut Open, regs: &mut Mmio, ch: &mut Chain, wait: bool) {
        if !ch.waiting() {
            return;
        }
        if wait {
            let t0 = rt::ticks();
            // (a render that hung is reset there, as a frame's wait does)
            let _ = o.wait_render(regs);
            HOLD_END.with(|h| {
                h.in_call += 1;
                h.in_call_us = h.in_call_us.wrapping_add(us_since(t0));
            });
        } else if !o.render_done_now(regs) {
            return;
        }
        let d = o.defer_t0.take().map_or(0, us_since);
        HOLD_END.with(|h| {
            h.held += 1;
            h.held_us = h.held_us.wrapping_add(d);
        });
        o.defer_us = Some(d);
        o.arm_cursor = pm_cursor(o);
        frame_ev(
            FrameKind::Arm,
            o.pm_frame,
            ch.last_words,
            ch.last_descriptors | u32::from(o.rendering.is_some()) << 31,
        );
        if !ch.arm(regs) {
            o.defer_us = None;
            c.ta_hangs += 1;
            o.recover(regs);
        }
    }

    /// What a settle or a pump found, acted on.
    fn settled(
        c: &mut Counts,
        log: fn(Arguments<'_>),
        o: &mut Open,
        regs: &mut Mmio,
        s: Settled,
        pumped: bool,
        gap: u32,
        fail: Option<([u32; 4], u32, u32, u32)>,
    ) {
        match s {
            Settled::Idle => {}
            Settled::Done(us) => {
                BISECT.with(|b| {
                    b.ta_us = b.ta_us.wrapping_add(us);
                    b.ta_n += 1;
                });
                o.ta_us = us;
                frame_ev(
                    FrameKind::TaDone,
                    o.pm_frame,
                    us,
                    u32::from(o.rendering.is_some()),
                );
                if let Some(d) = o.defer_us.take() {
                    HOLD_END.with(|h| {
                        if d < us {
                            h.needed += 1;
                        } else if d < us + 1_000 {
                            h.unsure += 1;
                        } else {
                            h.needless += 1;
                            h.needless_us = h.needless_us.wrapping_add(d - us);
                        }
                    });
                }
                c.chain_fails_row = 0;
                c.chain_us += us;
                c.gap_us += gap;
                c.pumped += u32::from(pumped);
                if let Some(h0) = o.arm_cursor.take()
                    && let Some(h1) = pm_cursor(o)
                {
                    let pages = ring_dist(h0, h1);
                    o.scene_pages[usize::from(o.armed_b)] = pages;
                    o.pages_recent = pages.max(o.pages_recent - o.pages_recent / 64);
                    c.pages_most = c.pages_most.max(pages);
                }
                if o.pm_frame >= PM_SNAP_FROM {
                    let m = if !o.overlap {
                        Moment::TaDoneAlone
                    } else if o.rendering.is_some() {
                        Moment::TaDoneOverlap
                    } else {
                        Moment::Report
                    };
                    if m != Moment::Report {
                        pm_snap(m, regs, o, 0, log);
                    }
                }
                if o.overlap {
                    o.pending = Some(o.armed_b);
                    o.start_pending(regs);
                } else {
                    o.start_render(regs);
                }
            }
            Settled::Failed { dma } => {
                // run 281: the TA stopped asking for memory (bit 5, three
                // times while a render ran), the core reset, and the next
                // chain wedged the chip's host path; then every access
                // waited out the bus time-out (~10 ms), the 1 ms pump read
                // the status each tick, and the program starved until the
                // run limit (30 s). So such a failure goes straight to the
                // full rebuild (run 280: 445 ms), no core reset first.
                // (the word kept for the log even when no answer)
                let (st, mute) = status_word(regs);
                if dma || mute || st & EV_TA_MEM != 0 {
                    c.rebuild_owed = true;
                }
                if c.run_hangs < HANGS_SAID {
                    let en = if mute {
                        0
                    } else {
                        regs.read32(MBX + reg::ENABLE)
                    };
                    log(format_args!(
                        "rp-gfx: R340: at the failed chain the core's status 0x{st:08x}, enables 0x{en:08x}; bit 5 {}, owed {}",
                        if st & EV_TA_MEM != 0 { "set" } else { "clear" },
                        o.mem_owed
                    ));
                }
                c.ta_hangs += 1;
                c.run_hangs += 1;
                c.chain_fails_row += 1;
                if c.run_hangs <= HANGS_SAID {
                    let (slowest, timeouts) = reads();
                    log(format_args!(
                        "rp-gfx: R340: TA hang {} ({}): {timeouts} status reads ended by the bus time-out since the last report, the slowest {slowest} us; {}",
                        c.run_hangs,
                        if dma {
                            "the DMA did not finish"
                        } else {
                            "the TA never said done"
                        },
                        if c.rebuild_owed {
                            "the full rebuild next"
                        } else {
                            "the core reset"
                        },
                    ));
                    if let Some((r, after, desc, us)) = fail {
                        log(format_args!(
                            "rp-gfx: R340: the chain's DMA at {us} us: DCSR 0x{:08x} DDADR 0x{:08x} DSADR 0x{:08x} DTADR 0x{:08x} (descriptor {desc}); DCSR after the stop 0x{after:08x} ({})",
                            r[0],
                            r[1],
                            r[2],
                            r[3],
                            if after & (1 << 3) != 0 {
                                "stopped"
                            } else {
                                "NOT stopped"
                            }
                        ));
                    }
                }
                if c.ta_hangs == 1 {
                    log(format_args!(
                        "rp-gfx: step 4f: a frame's chain {}: the core reset",
                        if dma {
                            "did not finish (the DMA)"
                        } else {
                            "ran but the TA never said done"
                        }
                    ));
                }
                // (said for the first hangs only, as the lines above: the
                // emulator's chains all fail, and these filled its run log)
                let said = c.run_hangs <= HANGS_SAID;
                if said {
                    log(format_args!(
                        "rp-gfx: R340 fits at the failed chain: scene A {} pages, B {}, a recent scene's most {}, the cursor now 0x{:x}",
                        o.scene_pages[0],
                        o.scene_pages[1],
                        o.pages_recent,
                        if mute { 0 } else { pm_cursor(o).unwrap_or(0) }
                    ));
                }
                if c.rebuild_owed {
                    log(format_args!(
                        "rp-gfx: R340: {}: the full rebuild owed (no core reset)",
                        if dma {
                            "the DMA did not finish"
                        } else if mute {
                            "the chip does not answer"
                        } else {
                            "the TA stopped for memory (bit 5)"
                        }
                    ));
                } else {
                    if said {
                        pm_snap(Moment::Failed, regs, o, F_TA_BUSY, log);
                    }
                    o.recover(regs);
                }
            }
        }
    }

    /// Step 4f's probe (`-rp4f`, [`Probe`]).
    #[must_use]
    pub fn with_probe(mut self, probe: Option<Probe>) -> Self {
        self.probe = probe;
        self
    }

    /// C2 (`crate::cpu`): the core past 624 MHz while the program has the
    /// screen (`-rpcpu 7` or `8`).
    #[must_use]
    pub const fn with_cpu(mut self, n2: Option<u16>) -> Self {
        self.cpu = n2;
        self
    }

    /// The frame-rate bar along the bottom (`-rpfpsbar`).
    #[must_use]
    pub const fn with_bar(mut self, on: bool) -> Self {
        self.bar = on;
        self
    }

    /// R343: the tone curves stepped through, named in the bar.
    #[must_use]
    pub const fn with_curves(mut self, on: bool) -> Self {
        self.curves = on;
        if on {
            self.bar = true;
        }
        self
    }

    /// R295's diagnostic ladder (`-rpladder`): from frame [`LADDER_FROM`]
    /// the [`CELLS`] one after another, then the run's own clock again.
    #[must_use]
    pub const fn with_ladder(mut self, on: bool) -> Self {
        self.ladder_on = on;
        self
    }

    /// The 2700G's clock while the program has the screen (`None`: the
    /// loader's, as found); a rung past [`rung_allowed`] is not taken.
    #[must_use]
    pub const fn with_rung(mut self, rung: Option<clock::Rung>) -> Self {
        self.rung = rung;
        self
    }

    /// The counts as one line every five seconds, then counted again.
    fn report(&mut self) {
        if us_since(self.since) < self.report_us {
            return;
        }
        let c = self.counts;
        let f = c.frames.max(1);
        let tenths = |us: u32, n: u32| {
            let t = us / n.max(1) / 100;
            (t / 10, t % 10)
        };
        let (feed, feed_t) = tenths(c.feed_us, f);
        let (tad, tad_t) = tenths(c.ta_us, f);
        let (wait, wait_t) = tenths(c.waited_us, f);
        let (render, render_t) = tenths(c.render_us, c.timed);
        (self.log)(format_args!(
            "rp-gfx: {} frames: a frame {} words, {} strips; the CPU feeding {feed}.{feed_t} ms, the TA done {tad}.{tad_t} ms after its start; waiting for the last render {wait}.{wait_t} ms; a render {render}.{render_t} ms ({} timed); refused {}, TA hangs {}, render hangs {}",
            c.frames,
            c.words / f,
            c.strips / f,
            c.timed,
            c.refused,
            c.ta_hangs,
            c.render_hangs
        ));
        if c.chained > 0 || c.unfit > 0 {
            let n = c.chained.max(1);
            let (ch, ch_t) = tenths(c.chain_us, n);
            let (st, st_t) = tenths(c.settle_us, f);
            (self.log)(format_args!(
                "rp-gfx: step 4f: {} frames fed by the DMA chain ({} descriptors a frame), the TA done {ch}.{ch_t} ms after the arm, {} of them seen done by the pump; waiting at a frame's start for the last chain {st}.{st_t} ms; {} frames too large for the chain (the CPU's)",
                c.chained,
                c.descriptors / n,
                c.pumped,
                c.unfit
            ));
        }
        if c.chained > 0 {
            let looks = self
                .chain
                .as_mut()
                .map_or(0, |ch| core::mem::take(&mut ch.looks));
            let (rw, rw_t) = tenths(c.rwait_us, c.rwaits);
            let (gap, gap_t) = tenths(c.gap_us, c.chained);
            (self.log)(format_args!(
                "rp-gfx: kit 113: a render waited for {rw}.{rw_t} ms from its start ({} frames waited); the TA's done seen at most {gap}.{gap_t} ms late (the render's start); {} looks a frame",
                c.rwaits,
                looks / c.chained.max(1)
            ));
        }
        if c.check_same + c.check_differ + c.check_failed > 0 {
            (self.log)(format_args!(
                "rp-gfx: -rpchaincheck: {} frames fed by the CPU then by the chain: pictures the same {}, different {}, not finished {}",
                c.check_same + c.check_differ + c.check_failed,
                c.check_same,
                c.check_differ,
                c.check_failed
            ));
        }
        let q = QUIET.with(|q| *q);
        if q.hold_s.is_some() || q.skipped > 0 || q.cuts > 0 {
            (self.log)(format_args!(
                "rp-gfx: R340 texture writes since the open: {} looks with the hold on ({}), {} waited for the render ({} ms in all); {} skipped at a silent chip; {} chains cut (the last at descriptor {}, {} us after its arm)",
                q.looks,
                match q.hold_s {
                    None => "off",
                    Some(0) => "-rptexhold, the whole run",
                    Some(_) => "-rptexhold S, the first S seconds",
                },
                q.held,
                q.held_us / 1000,
                q.skipped,
                q.cuts,
                q.cut_at.0,
                q.cut_at.1
            ));
        }
        let he = HOLD_END.with(|h| *h);
        (self.log)(format_args!(
            "rp-gfx: R340 a render never ending under a TA ({}; the wait {}): {} frames' TA waited for the render in flight ({} ms in all; {} of them inside the next frame call, {} ms), {} armed beside one that would outlast it; of the waits, judged by the frame's own TA time: {} needed, {} within 1 ms, {} needless (by {} ms in all); the last render {} us, the last TA {} us",
            if he.on { "on" } else { "-rpnoholdend" },
            if he.defer {
                "put off to the pump"
            } else {
                "-rpnodefer: in the frame call"
            },
            he.held,
            he.held_us / 1000,
            he.in_call,
            he.in_call_us / 1000,
            he.beside,
            he.needed,
            he.unsure,
            he.needless,
            he.needless_us / 1000,
            self.open.as_ref().map_or(0, |o| o.render_us),
            self.open.as_ref().map_or(0, |o| o.ta_us)
        ));
        let e9 = EV9.with(|e| *e);
        if e9.idle + e9.early + e9.late > 0 || e9.answer {
            (self.log)(format_args!(
                "rp-gfx: R340 event bit 9 since the open: {} with no chain in flight, {} in a chain's first 128 descriptors, {} later in a chain; {} answered with 0x834 ({})",
                e9.idle,
                e9.early,
                e9.late,
                e9.answered,
                if e9.answer { "-rpev9" } else { "off" }
            ));
        }
        let (mb_on, mb) = MEM_BACK.with(|m| *m);
        if mb_on {
            (self.log)(format_args!(
                "rp-gfx: R340 -rpmemback: 0x804 = 1 after {mb} renders since the open"
            ));
        }
        if c.pages_most > 0 || c.held_for_room > 0 {
            (self.log)(format_args!(
                "rp-gfx: R340 fits: a scene took {} pages at most, the chip held {} at once at most (of {PM_PAGES}), {} frames held back for room",
                c.pages_most, c.live_most, c.held_for_room
            ));
        }
        // R340's measure: the manager's block every report
        if let (Some(o), Some(regs)) = (self.open.as_ref(), self.regs.as_mut()) {
            let busy = self.chain.as_ref().is_some_and(Chain::busy);
            pm_snap(
                Moment::Report,
                regs,
                o,
                if busy { F_TA_BUSY } else { 0 },
                self.log,
            );
        }
        // R340: the slowest status read and the bus's time-outs (said
        // every report, then counted again; the hangs so far kept)
        let (slowest, timeouts) = reads();
        READS.with(|r| *r = (0, 0));
        (self.log)(format_args!(
            "rp-gfx: R340: the slowest 2700G status read {slowest} us, {timeouts} ended by the bus time-out; {} hangs since the screen opened; R344: {} full resets ({} brought the TA back)",
            c.run_hangs, c.rebuilds, c.rebuilds_ok
        ));
        if c.ev.iter().any(|&e| e != 0) {
            (self.log)(format_args!(
                "rp-gfx: R340: the core's events since the open: bit 5 {} ({} answered with 0x804), bit 7 {}, bit 8 {}, bit 9 {}",
                c.ev[0], c.ev_answered, c.ev[1], c.ev[2], c.ev[3]
            ));
        }
        self.counts = Counts {
            run_hangs: c.run_hangs,
            chain_fails_row: c.chain_fails_row,
            cpu_fails_row: c.cpu_fails_row,
            rebuilds: c.rebuilds,
            rebuilds_ok: c.rebuilds_ok,
            ev: c.ev,
            ev_answered: c.ev_answered,
            rebuild_owed: c.rebuild_owed,
            ..Counts::default()
        };
        self.since = rt::ticks();
    }
}

impl Chip {
    /// R344: the 2700G's full reset and rebuild while a game has the
    /// screen (R340's escalation, `why`); the frame not drawn, and the
    /// error the program gets: its textures are gone (`Errno::Stale`).
    fn rebuild_now(&mut self, why: &str) -> Errno {
        // ADR-0035 rule 6 as the owner changed it (2026-11-20): back to the
        // game's clock after a rebuild, unless the last was within a minute
        let recent = self
            .rebuilt_at
            .is_some_and(|(t, _)| us_since(t) < 60_000_000);
        let gap = self
            .rebuilt_at
            .map_or(REBUILD_GAP_US, |(_, g)| (g * 2).min(REBUILD_GAP_MOST_US));
        self.rebuilt_at = Some((rt::ticks(), gap));
        let (Some(o), Some(regs)) = (self.open.as_mut(), self.regs.as_mut()) else {
            return Errno::BadHandle;
        };
        let n = self.counts.rebuilds + 1;
        if n <= REBUILDS_SAID {
            // said first: the card has it even if the rebuild froze
            (self.log)(format_args!(
                "rp-gfx: R344: {why} at frame {}: the 2700G's full reset and rebuild next",
                self.frame
            ));
        }
        let t0 = rt::ticks();
        let (ok, r) = o.rebuild(regs);
        let mut back = "kept at Windows Mobile's (a second rebuild within a minute)";
        if ok
            && !recent
            && let Some(rung) = self.rung.filter(|&g| rung_allowed(g))
        {
            back = match clock::step(regs, rung, rt::delay_us, set_cs3) {
                Some(a) if a.unwritten == 0 => {
                    o.cur = Some(rung);
                    o.stepped = true;
                    o.limit_us = render_limit_us(Some(rung));
                    "stepped back to the game's"
                }
                _ => {
                    let _ = clock::put_back(regs, rt::delay_us, set_cs3);
                    "NOT taken: Windows Mobile's kept"
                }
            };
            let _ = underrun_take(regs);
        }
        let us = us_since(t0);
        if n <= REBUILDS_SAID {
            (self.log)(format_args!(
                "rp-gfx: R344: after the rebuild the 2700G's clock {back}"
            ));
        }
        self.counts.rebuilds = n;
        self.counts.rebuilds_ok += u32::from(ok);
        if n <= REBUILDS_SAID {
            rebuilt_line(self.log, why, n, ok, us, &r);
        }
        self.frame = self.frame.wrapping_add(1);
        self.report();
        Errno::Stale
    }

    /// Before a frame's feed: the ladder's next cell when this one is
    /// full (its counts logged), the run's own clock and layout after the
    /// last.
    fn ladder_before(&mut self) {
        if !self.ladder_on || self.frame < LADDER_FROM {
            return;
        }
        let next = match &self.ladder {
            None => 0,
            Some(l) if l.n >= CELLS[l.cell].frames => {
                self.ladder_log();
                l.cell + 1
            }
            Some(_) => return,
        };
        let ok = self.ladder_switch(next);
        if next < CELLS.len() && ok {
            self.ladder = Some(Ladder::new(next));
        } else {
            if next < CELLS.len() {
                // A cell not taken ends the ladder; the run's own clock.
                let _ = self.ladder_switch(CELLS.len());
            }
            self.ladder = None;
            self.ladder_on = false;
            (self.log)(format_args!(
                "rp-gfx ladder: over; the run's own clock and layout again"
            ));
        }
    }

    /// The cell `k`'s clock and layout (past the last: the run's own),
    /// the TA set up afresh in it; false if the clock or the set-up was
    /// not taken.
    fn ladder_switch(&mut self, k: usize) -> bool {
        let log = self.log;
        let (Some(o), Some(regs)) = (self.open.as_mut(), self.regs.as_mut()) else {
            return false;
        };
        let _ = o.wait_render(regs);
        let (rung, lay, words, limit) = match CELLS.get(k) {
            Some(c) => (Some(c.rung()), c.lay, c.words, LADDER_LIMIT_US),
            None => (
                self.rung.filter(|r| rung_allowed(*r)),
                &LAY_SDRAM,
                NONE,
                RENDER_LIMIT_US,
            ),
        };
        if rung.is_some_and(|r| !rung_allowed(r)) {
            log(format_args!(
                "rp-gfx ladder: cell {} past the game step, not taken",
                k + 1
            ));
            return false;
        }
        let mut ok = true;
        if rung != o.cur {
            let a = match rung {
                Some(r) => clock::step(regs, r, rt::delay_us, set_cs3),
                None => clock::put_back(regs, rt::delay_us, set_cs3),
            };
            ok = matches!(a, Some(a) if a.unwritten == 0);
            if ok {
                o.cur = rung;
                o.stepped |= rung.is_some();
            } else {
                let _ = clock::put_back(regs, rt::delay_us, set_cs3);
                o.cur = None;
            }
            log(format_args!(
                "rp-gfx ladder: clock {} ({} kHz / {}): COREPLL 0x{:x} MBXCLK 0x{:x} LMTIM 0x{:x}; the underrun flag {}",
                if ok {
                    "stepped"
                } else {
                    "NOT taken, Windows Mobile's put back"
                },
                rung.map_or(0, |r| r.speed.khz()),
                rung.map_or(0, |r| r.div),
                regs.read32(clock::COREPLL),
                regs.read32(MBXCLK),
                regs.read32(clock::LMTIM),
                if underrun_take(regs) { "set" } else { "clear" }
            ));
        }
        let lay = if ok { lay } else { &LAY_SDRAM };
        match ta::set_up(
            regs,
            o.vram,
            lay.pm,
            &lay.keep,
            rt::delay_us,
            cpu::drain_write_buffer,
            CLOCK,
        ) {
            Ok(r) => {
                o.ready = r;
                o.lay = lay;
                o.words = if ok { words } else { NONE };
                o.limit_us = if ok { limit } else { RENDER_LIMIT_US };
                o.marking = ok && k < CELLS.len();
                o.part = match CELLS.get(k) {
                    Some(c) if ok => c.part,
                    _ => Part::All,
                };
                o.warm = ok && CELLS.get(k).is_some_and(|c| c.warm);
                o.checker = ok && CELLS.get(k).is_some_and(|c| c.checker);
                o.overlay = ok && CELLS.get(k).is_some_and(|c| c.overlay);
                o.warm_owed = false;
            }
            Err(e) => {
                log(format_args!(
                    "rp-gfx ladder: the TA's set-up in {} refused: {e:?}",
                    lay.name
                ));
                // Back to the layout the open proved.
                if let Ok(r) = ta::set_up(
                    regs,
                    o.vram,
                    LAY_SDRAM.pm,
                    &LAY_SDRAM.keep,
                    rt::delay_us,
                    cpu::drain_write_buffer,
                    CLOCK,
                ) {
                    o.ready = r;
                }
                o.lay = &LAY_SDRAM;
                o.words = NONE;
                o.limit_us = RENDER_LIMIT_US;
                return false;
            }
        }
        ok
    }

    /// One cell's line, and its render block at the first hang against
    /// the last good render's.
    fn ladder_log(&self) {
        let Some(l) = &self.ladder else {
            return;
        };
        let c = &CELLS[l.cell];
        let r = c.rung();
        let avg = l.render_us / l.ok.max(1);
        let shown = (l.n as usize).min(l.pattern.len());
        (self.log)(format_args!(
            "rp-gfx ladder {}/{}: {} kHz / 1, {}, {}: {} frames, render hangs {}, TA hangs {}; a good render {}.{} ms (most {}.{}) over {}; {}",
            l.cell + 1,
            CELLS.len(),
            r.speed.khz(),
            c.lay.name,
            c.what,
            l.n,
            l.hangs,
            l.ta_hangs,
            avg / 1000,
            avg % 1000 / 100,
            l.max_us / 1000,
            l.max_us % 1000 / 100,
            l.ok,
            core::str::from_utf8(&l.pattern[..shown]).unwrap_or("")
        ));
        if c.checker {
            if let Some(o) = &self.open {
                (self.log)(format_args!(
                    "rp-gfx ladder {}: the checker copy {} us a frame on the CPU ({} copies)",
                    l.cell + 1,
                    o.checker_us / o.checker_n.max(1),
                    o.checker_n
                ));
            }
        }
        if c.part != Part::All || !c.words.is_empty() || c.checker || c.overlay {
            (self.log)(format_args!(
                "rp-gfx ladder {}: with the change {} hangs of {} frames, without it {} of {} (`h`/`,` the frames with it); {} strips left out by it in all",
                l.cell + 1,
                l.hangs_b,
                l.n / 2,
                l.hangs - l.hangs_b,
                l.n - l.n / 2,
                l.left_out
            ));
        }
        for (mask, &n) in l.marks.iter().enumerate() {
            if n > 0 {
                (self.log)(format_args!(
                    "rp-gfx ladder {}: {n} hangs with the marks drawn over: {} of 5 (mask 0b{mask:05b}: bit 0 the top-left, bit 4 the bottom-right)",
                    l.cell + 1,
                    (mask as u32).count_ones()
                ));
            }
        }
        if l.cell == 0 {
            if let Some(b) = &l.ok_block {
                for (k, row) in b.chunks(8).enumerate() {
                    (self.log)(format_args!(
                        "rp-gfx ladder: the render block after a good render, 0x{:x}: {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
                        BLOCK_AT + 32 * k,
                        row[0],
                        row[1],
                        row[2],
                        row[3],
                        row[4],
                        row[5],
                        row[6],
                        row[7]
                    ));
                }
            }
        }
        if let (Some(h), Some(g)) = (&l.hang_block, &l.ok_block) {
            let mut any = false;
            for (k, (hv, gv)) in h.iter().zip(g.iter()).enumerate() {
                if hv != gv {
                    any = true;
                    (self.log)(format_args!(
                        "rp-gfx ladder {}: at the first hang 0x{:x} = 0x{hv:08x} (a good render left 0x{gv:08x})",
                        l.cell + 1,
                        BLOCK_AT + 4 * k
                    ));
                }
            }
            if !any {
                (self.log)(format_args!(
                    "rp-gfx ladder {}: at the first hang the render block read as a good render left it",
                    l.cell + 1
                ));
            }
        } else if let Some(h) = &l.hang_block {
            for (k, row) in h.chunks(8).enumerate() {
                (self.log)(format_args!(
                    "rp-gfx ladder {}: the render block at the first hang (no good render), 0x{:x}: {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
                    l.cell + 1,
                    BLOCK_AT + 32 * k,
                    row[0],
                    row[1],
                    row[2],
                    row[3],
                    row[4],
                    row[5],
                    row[6],
                    row[7]
                ));
            }
        }
    }
}

impl Open {
    /// The core reset and the TA's memory manager set up again after a
    /// hang (the driver's one way back).
    fn recover(&mut self, regs: &mut Mmio) -> bool {
        self.ready
            .recover(
                regs,
                self.vram,
                rt::delay_us,
                cpu::drain_write_buffer,
                CLOCK,
            )
            .is_ok()
    }

    /// R344/R340: the 2700G's full reset and rebuild (run Z,
    /// `rp_drv_mbx_display::rebuild`), its 3D clock on, the TA's manager
    /// set up again in the same layout, the scenes in flight forgotten and
    /// the R343 curve put back. Windows Mobile's 2700G clock from here on
    /// (ADR-0035 rule 6). The chip's memory is lost: the caller tells the
    /// program (`Errno::Stale`). Whether the TA came up, and what the
    /// rebuild found.
    fn rebuild(&mut self, regs: &mut Mmio) -> (bool, rp_drv_mbx_display::rebuild::Rebuilt) {
        let r = rebuild_chip(regs);
        self.cur = None;
        self.limit_us = render_limit_us(None);
        self.rendering = None;
        self.pending = None;
        self.render_b = false;
        self.armed_b = false;
        self.next_b = self.overlap;
        self.warm_owed = false;
        if let Some((k, _, table, _)) = self.curve {
            use rp_drv_mbx_display::reg::GSCTRL;
            let gs = regs.read32(GSCTRL);
            regs.write32(GSCTRL, gs | GSCTRL_GAMMA);
            curve_set(regs, k, &table);
        }
        let ok = match ta::set_up(
            regs,
            self.vram,
            self.lay.pm,
            &self.lay.keep,
            rt::delay_us,
            cpu::drain_write_buffer,
            CLOCK,
        ) {
            Ok(ready) => {
                self.ready = ready;
                true
            }
            Err(_) => false,
        };
        (ok, r)
    }

    /// R340: Dell's answer to the core's events ([`EVENTS`]): each enabled,
    /// acknowledged and counted; bit 5 answered with `0x804` = 1 at once
    /// if no render runs, else once it has ended. Called from the 1 ms
    /// pump and before each frame (one status read, ~1 us).
    fn chip_events(
        &mut self,
        regs: &mut Mmio,
        c: &mut Counts,
        log: fn(Arguments<'_>),
        at: Option<u32>,
    ) {
        let en = regs.read32(MBX + reg::ENABLE);
        if en & EVENTS != EVENTS {
            // a core reset clears the enables
            regs.write32(MBX + reg::ENABLE, en | EVENTS);
            self.en |= EVENTS;
        }
        let Some(st) = status(regs) else {
            return;
        };
        let ev = st & EVENTS;
        if ev != 0 {
            regs.write32(MBX + reg::CLEAR, ev);
            for (k, bit) in [EV_TA_MEM, EV_7, EV_8, EV_9].into_iter().enumerate() {
                if ev & bit != 0 {
                    c.ev[k] += 1;
                    if c.ev[k] <= 3 {
                        log(format_args!(
                            "rp-gfx: R340: the core's event bit {} (status 0x{st:08x}, a render {})",
                            bit.trailing_zeros(),
                            if self.rendering.is_some() {
                                "running"
                            } else {
                                "idle"
                            }
                        ));
                    }
                }
            }
            if ev & EV_TA_MEM != 0 {
                self.mem_owed = true;
            }
            if ev & EV_9 != 0 {
                frame_ev(
                    FrameKind::Ev9,
                    self.pm_frame,
                    self.rendering.map_or(u32::MAX, us_since),
                    st,
                );
                // R340 after run 330: each of the first 60 events whole
                // (the render's age, the program's texture writes during
                // it, the frame, the chain's place), so a play names what
                // comes with them
                let n9 = c.ev[3];
                if (4..=60).contains(&n9) {
                    let age = self.rendering.map_or(u32::MAX, us_since);
                    let writes = QUIET.with(|q| q.calls).wrapping_sub(self.render_writes);
                    log(format_args!(
                        "rp-gfx: R340 event {n9}: frame {}, status 0x{st:08x}, render {} us old ({}), {} texture writes during it, chain {}",
                        c.frames,
                        if age == u32::MAX { 0 } else { age },
                        if self.rendering.is_some() {
                            "running"
                        } else {
                            "none"
                        },
                        if self.rendering.is_some() { writes } else { 0 },
                        match at {
                            None => u32::MAX,
                            Some(d) => d,
                        }
                    ));
                }
                EV9.with(|e| {
                    match at {
                        None => e.idle += 1,
                        Some(d) if d < 128 => e.early += 1,
                        Some(_) => e.late += 1,
                    }
                    e.last = Some((rt::ticks(), at.unwrap_or(u32::MAX)));
                    e.owed = e.answer;
                });
            }
        }
        // the lever: bit 9 answered once no render is in flight
        if (self.rendering.is_none() || st & m3::RENDER_DONE != 0)
            && EV9.with(|e| core::mem::replace(&mut e.owed, false))
        {
            regs.write32(MBX + TA_EV9_ANSWER, 1);
            EV9.with(|e| e.answered += 1);
        }
        if self.mem_owed && (self.rendering.is_none() || st & m3::RENDER_DONE != 0) {
            self.mem_owed = false;
            regs.write32(MBX + TA_MEM_BACK, 1);
            c.ev_answered += 1;
        }
    }

    /// Waits for the render in flight, if any: how long it took to be seen
    /// done, or `Err` if it hung (the core then reset and set up again).
    fn wait_render(&mut self, regs: &mut Mmio) -> Result<u32, Errno> {
        let Some(t0) = self.rendering.take() else {
            return Ok(0);
        };
        let ok = loop {
            // R340: a read the bus timed out is no answer: the limit
            // below decides (QEMU's reads take up to ~0.7 ms of host
            // time, so a slow read alone is no hang)
            if status(regs).is_some_and(|s| s & m3::RENDER_DONE != 0) {
                break true;
            }
            if us_since(t0) > self.limit_us {
                break false;
            }
        };
        if !ok {
            self.hang_block = Some(read_block(regs));
            if self.marking {
                self.hang_marks = self.marks_drawn();
            }
        }
        let us = us_since(t0);
        regs.write32(MBX + reg::CLEAR, m3::RENDER_DONE);
        regs.write32(MBX + reg::ENABLE, self.en);
        if ok {
            self.render_us = us;
            frame_ev(FrameKind::RenderDone, self.pm_frame, us, 1);
            mem_back(regs, self.overlap);
            Ok(us)
        } else {
            self.recover(regs);
            Err(Errno::Io)
        }
    }

    fn mark_at(x: u32, y: u32) -> usize {
        (y * W + x) as usize * 2
    }

    /// The marks into the screen (the CPU's stores drained before the
    /// render starts).
    fn put_marks(&mut self) {
        for &(x, y) in &MARKS {
            let _ = store32_volatile(self.vram, Self::mark_at(x, y), MARK);
        }
        cpu::drain_write_buffer();
    }

    /// The bar, once a second: frames a second, the 2700G's clock, the
    /// hangs since the screen opened.
    fn draw_bar(&mut self) {
        let us = us_since(self.bar_t0);
        if us < 1_000_000 {
            return;
        }
        if self.curve.is_some() {
            self.draw_band(false);
            return;
        }
        let tenths = u64::from(self.bar_frames) * 10_000_000 / u64::from(us.max(1));
        let khz = self.cur.map_or(0, |r| r.speed.khz());
        let at = (DRAW_H * W * 2) as usize;
        let Some(rows) = self.vram.get_mut(at..at + (BAR_ROWS * W * 2) as usize) else {
            return;
        };
        let mut c = Console::new(
            rows,
            STRIDE,
            (6, 0),
            (W as usize - 12, BAR_ROWS as usize),
            2,
            (0xFFE0, 0x0000),
        );
        if let Some(k) = self.curve.map(|v| v.0) {
            let _ = write!(c, "{}  ", CURVES[k].0);
        }
        let _ = write!(
            c,
            "{}.{} fps  2700G {}.{:02} MHz  stalls {}",
            tenths / 10,
            tenths % 10,
            khz / 1000,
            khz % 1000 / 10,
            self.bar_hangs
        );
        self.bar_t0 = rt::ticks();
        self.bar_frames = 0;
    }

    /// R343's band: the curve's name upright and big (its letters 25 x 35
    /// pixels, stacked), the frame rate under it; `name` draws the whole
    /// band (a step), else the rate only (once a second).
    fn draw_band(&mut self, name: bool) {
        let us = us_since(self.bar_t0).max(1);
        let fps = u64::from(self.bar_frames) * 1_000_000 / u64::from(us);
        let stalls = self.bar_hangs > 0;
        let Some(k) = self.curve.map(|v| if self.comparing { 0 } else { v.0 }) else {
            return;
        };
        let at = (PICK_H * W * 2) as usize;
        let Some(rows) = self.vram.get_mut(at..at + (PICK_ROWS * W * 2) as usize) else {
            return;
        };
        if name {
            rows.fill(0);
            band_text(rows, (12, 8), 5, CURVES[k].0, 0xFFE0, true);
        } else {
            for ly in 380..W {
                for lx in 0..PICK_ROWS {
                    band_px(rows, lx, ly, 0);
                }
            }
            let f = fps.min(999) as u32;
            let digits = [
                b'0' + (f / 100) as u8,
                b'0' + (f / 10 % 10) as u8,
                b'0' + (f % 10) as u8,
            ];
            let skip = if f >= 100 {
                0
            } else if f >= 10 {
                1
            } else {
                2
            };
            let n = core::str::from_utf8(&digits[skip..]).unwrap_or("");
            band_text(rows, (3, 384), 3, n, 0xFFFF, false);
            band_text(rows, (6, 420), 2, "FPS", 0xFFFF, false);
            if stalls {
                band_text(rows, (6, 446), 2, "STL", 0xF800, false);
            }
            self.bar_t0 = rt::ticks();
            self.bar_frames = 0;
        }
        cpu::drain_write_buffer();
    }

    /// Bit k set: `MARKS[k]` no longer reads as the mark.
    fn marks_drawn(&self) -> u32 {
        let mut m = 0;
        for (k, &(x, y)) in MARKS.iter().enumerate() {
            if load32_volatile(self.vram, Self::mark_at(x, y)).is_some_and(|v| v != MARK) {
                m |= 1 << k;
            }
        }
        m
    }

    /// The TA's region array (raster order) copied to [`CHK_REGIONS`] in
    /// the checker order: the last entry's bit moved to the new last.
    fn copy_checker(&mut self) {
        let (cols, rows) = self.lay.ta.grid();
        let (cols, rows) = (cols as usize, rows as usize);
        let src = self.lay.ta.regions as usize;
        let dst = CHK_REGIONS as usize;
        let mut out = 0usize;
        for i in 0..2 * cols * rows {
            if let Some(t) = order_tile(TileOrder::Checker, i, cols, rows) {
                let h = load32_volatile(self.vram, src + 8 * t).unwrap_or(0) & !REGION_LAST;
                let a = load32_volatile(self.vram, src + 8 * t + 4).unwrap_or(0);
                let _ = store32_volatile(self.vram, dst + 8 * out, h);
                let _ = store32_volatile(self.vram, dst + 8 * out + 4, a);
                out += 1;
            }
        }
        if out > 0 {
            let last = dst + 8 * (out - 1);
            let h = load32_volatile(self.vram, last).unwrap_or(0);
            let _ = store32_volatile(self.vram, last, h | REGION_LAST);
        }
        cpu::drain_write_buffer();
    }

    /// Kit 113: the TA's layout for scene `b` (the second's own tail
    /// buffer and region array).
    fn ta_lay(&self, b: bool) -> ta::Layout {
        if b {
            ta::Layout {
                tail: TAIL_B,
                regions: REGIONS_B,
                ..self.lay.ta
            }
        } else {
            self.lay.ta
        }
    }

    /// Kit 113: a look, never a wait: the render in flight seen done (its
    /// bit cleared) or none in flight.
    fn render_done_now(&mut self, regs: &mut Mmio) -> bool {
        if self.rendering.is_none() {
            return true;
        }
        // R340: a read the bus timed out is not done (the next wait's
        // limit decides)
        if status(regs).is_none_or(|s| s & m3::RENDER_DONE == 0) {
            return false;
        }
        if let Some(t0) = self.rendering.take() {
            self.render_us = us_since(t0);
            frame_ev(FrameKind::RenderDone, self.pm_frame, self.render_us, 0);
        }
        regs.write32(MBX + reg::CLEAR, m3::RENDER_DONE);
        regs.write32(MBX + reg::ENABLE, self.en);
        mem_back(regs, self.overlap);
        true
    }

    /// Kit 113: the scene the TA finished drawn as soon as the render
    /// before it has ended (a look, never a wait).
    fn start_pending(&mut self, regs: &mut Mmio) {
        if let Some(b) = self.pending
            && self.render_done_now(regs)
        {
            if self.pm_frame >= PM_SNAP_FROM {
                pm_snap(Moment::RenderDoneOverlap, regs, self, 0, self.pm_log);
            }
            self.pending = None;
            self.render_b = b;
            self.start_render(regs);
        }
    }

    /// Starts the 3D stage on the TA's region array, into the screen.
    fn start_render(&mut self, regs: &mut Mmio) {
        if self.marking {
            self.put_marks();
        }
        self.en = regs.read32(MBX + reg::ENABLE);
        regs.write32(MBX + reg::CLEAR, m3::RENDER_DONE);
        regs.write32(MBX + reg::ENABLE, self.en | m3::RENDER_DONE);
        let mut from = self.lay.from;
        if self.render_b {
            from.regions = REGIONS_B;
        }
        if self.checker_on {
            let t0 = rt::ticks();
            self.copy_checker();
            self.checker_us += us_since(t0);
            self.checker_n += 1;
            from.regions = CHK_REGIONS;
        }
        self.rendering = Some(rt::ticks());
        frame_ev(FrameKind::RenderStart, self.pm_frame, 0, 0);
        self.render_writes = QUIET.with(|q| q.calls);
        let target = if self.lay.ta.height < H {
            SCREEN_BAR
        } else {
            SCREEN
        };
        for (off, v) in binner::start_table(&from, target) {
            let words = if self.words_on { self.words } else { NONE };
            let v = words.iter().find(|w| w.0 == off).map_or(v, |w| w.1);
            regs.write32(MBX + off, v);
        }
    }
}

impl Screen for Chip {
    fn open(&mut self) -> Result<Gfx, Errno> {
        if self.open.is_some() {
            return Err(Errno::Busy);
        }
        let regs = self.regs.as_mut().ok_or(Errno::NoSys)?;
        let (vram, offset, stride) = panel::take().ok_or(Errno::NoSys)?;
        if stride != STRIDE || vram.len() < (TEX_AT + TEX_BYTES) as usize {
            (self.log)(format_args!(
                "rp-gfx: not opened: the screen {stride} bytes a line (wanted {STRIDE}), {} KiB of 2700G memory",
                vram.len() / 1024
            ));
            panel::give_back(vram);
            return Err(Errno::Io);
        }
        // The picture at 0x0, inside the safe region (ADR-0034), where
        // Windows Mobile and the loader leave it on the X51v; the QEMU
        // fork starts its plane at 0x60000 (docs/hw/emulator.md, as
        // `rp-crash` does): the plane moved there for the program's time.
        let moved = (offset != 0).then(|| {
            let at = |o: usize| Plane {
                format: PlaneFormat::Rgb565,
                width: W as u16,
                height: H as u16,
                offset: o,
                stride: STRIDE,
            };
            graphics_set(regs, &at(0), Layer::SOLO, false);
            at(offset)
        });
        // R340: the bus time-out on before anything else (a wedged chip
        // then ends its cycle instead of holding the device).
        let syscfg0 = regs.read32(SYSCFG);
        regs.write32(SYSCFG, syscfg_timeout_on(syscfg0));
        (self.log)(format_args!(
            "rp-gfx: R340: the 2700G's bus time-out on: SYSCFG 0x{syscfg0:x} -> 0x{:x}",
            regs.read32(SYSCFG)
        ));
        let clk0 = regs.read32(MBXCLK);
        clock::restore(regs, with_3d(clk0));
        rt::delay_us(1_000);
        // The program's clock (ADR-0035's ladder, its own step code: the
        // long bus timing first, the PLL and memory timing read back and
        // written again, then the divider and the tier's bus timing).
        let mut stepped = false;
        if let Some(rung) = self.rung {
            if !rung_allowed(rung) {
                (self.log)(format_args!(
                    "rp-gfx: clock {} kHz / {} not taken: past the game step (ADR-0035 D)",
                    rung.speed.khz(),
                    rung.div
                ));
            } else {
                let before = underrun_take(regs);
                match clock::step(regs, rung, rt::delay_us, set_cs3) {
                    Some(a) if a.unwritten == 0 => {
                        stepped = true;
                        (self.log)(format_args!(
                            "rp-gfx: clock stepped to {} kHz / {} (the cores {} kHz): PLL locked in {} us, {} memory-controller writes written again; COREPLL 0x{:x} MBXCLK 0x{:x} LMTIM 0x{:x}; the underrun flag {} before, {} after (cleared)",
                            rung.speed.khz(),
                            rung.div,
                            rung.core_khz(),
                            a.lock_us,
                            a.rewrites,
                            regs.read32(clock::COREPLL),
                            regs.read32(MBXCLK),
                            regs.read32(clock::LMTIM),
                            if before { "set" } else { "clear" },
                            if underrun_take(regs) { "set" } else { "clear" }
                        ));
                    }
                    other => {
                        let _ = clock::put_back(regs, rt::delay_us, set_cs3);
                        clock::restore(regs, with_3d(clk0));
                        let _ = underrun_take(regs);
                        (self.log)(format_args!(
                            "rp-gfx: clock {} kHz / {} not taken ({}): Windows Mobile's put back",
                            rung.speed.khz(),
                            rung.div,
                            if other.is_none() {
                                "the PLL did not lock"
                            } else {
                                "a memory-controller write never held"
                            }
                        ));
                    }
                }
            }
        }
        if let Some(n2) = self.cpu {
            crate::cpu::step(n2, self.log);
        }
        // R340 (run 282: run 281's crash left the chip wedged, its set-up
        // "ready" from a read that timed out, then the pump starved the
        // program): a status read the bus time-out ended is a wedged chip,
        // rebuilt before the set-up writes 16 KiB into it at ~10 ms a word
        let _ = mute_take();
        for _ in 0..MUTE_ROW {
            let _ = status(regs);
        }
        let mute = mute_take();
        let mut ready = if mute {
            Err(ta::SetUpError::NotReady)
        } else {
            ta::set_up(
                regs,
                vram,
                TA_PM,
                &KEEP,
                rt::delay_us,
                cpu::drain_write_buffer,
                CLOCK,
            )
        };
        if mute || matches!(ready, Err(ta::SetUpError::NotReady)) {
            // R344 (run 277: a chip the last run left wedged played blind):
            // the full reset and rebuild, at Windows Mobile's clock from
            // then on (ADR-0035 rule 6), and the set-up again
            (self.log)(format_args!(
                "rp-gfx: R344: the TA's set-up {} at the open: the 2700G's full reset and rebuild next",
                if mute {
                    "on a chip that does not answer (a status read took the whole bus time-out)"
                } else {
                    "NotReady"
                }
            ));
            let t0 = rt::ticks();
            let r = rebuild_chip(regs);
            stepped = false;
            ready = ta::set_up(
                regs,
                vram,
                TA_PM,
                &KEEP,
                rt::delay_us,
                cpu::drain_write_buffer,
                CLOCK,
            );
            self.counts.rebuilds += 1;
            self.counts.rebuilds_ok += u32::from(ready.is_ok());
            rebuilt_line(self.log, "at the open", 1, ready.is_ok(), us_since(t0), &r);
        }
        let ready = match ready {
            Ok(r) => r,
            Err(e) => {
                (self.log)(format_args!("rp-gfx: not opened: the TA's set-up: {e:?}"));
                if stepped {
                    let _ = clock::put_back(regs, rt::delay_us, set_cs3);
                    let _ = underrun_take(regs);
                }
                clock::restore(regs, clk0);
                if let Some(p) = moved {
                    graphics_set(regs, &p, Layer::SOLO, false);
                }
                regs.write32(SYSCFG, syscfg0);
                panel::give_back(vram);
                // R344: never play with no picture unsaid
                panel::note(
                    b"2700G NOT READY, even after|a full reset: no picture.|Soft-reset the X51v.",
                );
                return Err(Errno::Io);
            }
        };
        // The texture memory as the program writes it: Rust never touches
        // this range while the screen is open.
        let tex = vram[TEX_AT as usize..].as_mut_ptr().cast();
        // The screen, for a test's eye: the program reads it only.
        let screen = vram.as_ptr().cast();
        let gfx = Gfx {
            width: W as u16,
            height: H as u16,
            tex_at: TEX_AT,
            tex_bytes: TEX_BYTES,
            tex,
            screen,
            // Step 4f: the words are read by the chain after the call.
            flags: if self.chain.is_some() {
                rp_hal_cabi::gfx_flags::KEEP
            } else {
                0
            },
        };
        (self.log)(format_args!(
            "rp-gfx: the screen is the program's: 480 x 640 drawn by the 3D core into the screen, MBXCLK 0x{clk0:x}, texture memory {} KiB at 0x{TEX_AT:x}",
            TEX_BYTES / 1024
        ));
        let overlap_ok = self.overlap
            && self.chain.is_some()
            && self.probe.is_none()
            && !self.ladder_on
            && self.chain_check == 0;
        self.open = Some(Open {
            vram,
            ready,
            clk0,
            syscfg0,
            gfx,
            rendering: None,
            render_writes: 0,
            en: 0,
            moved,
            stepped,
            cur: if stepped { self.rung } else { None },
            lay: &LAY_SDRAM,
            words: NONE,
            words_on: true,
            limit_us: render_limit_us(if stepped { self.rung } else { None }),
            hang_block: None,
            marking: false,
            hang_marks: 0,
            part: Part::All,
            warm: false,
            warm_owed: false,
            checker: false,
            checker_on: false,
            checker_us: 0,
            checker_n: 0,
            overlay: false,
            overlay_on: false,
            bar: self.bar,
            bar_t0: rt::ticks(),
            bar_frames: 0,
            curve: None,
            curve_seen: 0,
            comparing: false,
            mem_owed: false,
            bar_hangs: 0,
            overlap: overlap_ok && self.overlap_at.is_none(),
            next_b: false,
            armed_b: false,
            render_b: false,
            pending: None,
            pm_frame: 0,
            pm_log: self.log,
            pm_opened: rt::ticks(),
            overlap_from: self.overlap_at.filter(|_| overlap_ok),
            arm_cursor: None,
            scene_pages: [0; 2],
            render_us: 0,
            ta_us: 0,
            defer_t0: None,
            defer_us: None,
            pages_recent: 0,
        });
        if let (Some(o), Some(regs)) = (self.open.as_ref(), self.regs.as_mut()) {
            pm_snap(Moment::SetUp, regs, o, 0, self.log);
            if let Some(s) = o.overlap_from {
                (self.log)(format_args!(
                    "rp-gfx: R340: -rpoverlapat: kit 113's overlap off until {s} s after the open, then on"
                ));
            }
        }
        if self.curves
            && let (Some(o), Some(regs)) = (self.open.as_mut(), self.regs.as_mut())
        {
            use rp_drv_mbx_display::reg::{GFXGAM0, GSCTRL};
            let table = core::array::from_fn(|k| regs.read32(GFXGAM0 + 4 * k));
            let gs = regs.read32(GSCTRL);
            regs.write32(GSCTRL, gs | GSCTRL_GAMMA);
            curve_set(regs, 0, &table);
            o.curve = Some((0, rt::ticks(), table, gs));
            o.curve_seen = CURVE_KEY.with(|k| k.2);
            o.lay = &LAY_PICK;
            o.draw_band(true);
            (self.log)(format_args!(
                "rp-gfx: R343: the picker: the Record button steps the curves (found GFXGAM0 0x{:x} .. GFXGAM16 0x{:x}, GSCTRL 0x{gs:x})",
                table[0], table[16]
            ));
        } else if let (Some(o), Some(regs)) = (self.open.as_mut(), self.regs.as_mut()) {
            // R343: the owner's pick from run 289 ("bl60 looked the best"):
            // the game's look by default, the table as found put back at
            // the close; until the panel is measured (the next step: its
            // own curve and black at each backlight, then the backlight
            // following the scene)
            use rp_drv_mbx_display::reg::{GFXGAM0, GSCTRL};
            let table = core::array::from_fn(|k| regs.read32(GFXGAM0 + 4 * k));
            let gs = regs.read32(GSCTRL);
            regs.write32(GSCTRL, gs | GSCTRL_GAMMA);
            curve_set(regs, LOOK_DEFAULT, &table);
            o.curve = Some((LOOK_DEFAULT, rt::ticks(), table, gs));
            if let Some((b, _)) = self.light.as_mut() {
                b.set(CURVES[LOOK_DEFAULT].1);
            }
            (self.log)(format_args!(
                "rp-gfx: R343: the game's look {} (backlight {}%, its table)",
                CURVES[LOOK_DEFAULT].0, CURVES[LOOK_DEFAULT].1
            ));
        }
        if self.bar
            && !self.curves
            && let Some(o) = self.open.as_mut()
        {
            o.lay = &LAY_BAR;
        }
        self.opened = rt::ticks();
        self.since = rt::ticks();
        Ok(gfx)
    }

    fn frame(&mut self, parts: &[&[u32]]) -> Result<(), Errno> {
        if self.open.is_none() || self.regs.is_none() {
            return Err(Errno::BadHandle);
        }
        // R346: the sampler keeps a slow frame's samples whole, the rest
        // one in N (a no-op when not sampling)
        let dt = us_since(self.last_frame);
        self.last_frame = rt::ticks();
        rt::sample_frame(dt, self.frame > 0);
        // C2: the core's known answer every frame while past 624 MHz.
        crate::cpu::check(self.log);
        // Step 4f: the last frame's chain done and its render started.
        self.settle();
        // R340 (run 281): a rebuild owed now, before anything else reads
        // the chip
        if mute_take() {
            self.counts.rebuild_owed = true;
        }
        if self.counts.rebuild_owed {
            self.counts.rebuild_owed = false;
            return Err(self.rebuild_now("the TA stalled or the chip stopped answering"));
        }
        let (Some(o), true) = (self.open.as_mut(), self.regs.is_some()) else {
            return Err(Errno::BadHandle);
        };
        let c = &mut self.counts;
        // Every part checked before any is fed (a refusal draws nothing).
        let mut counts = rp_port::gfx::Counts::default();
        let mut before = 0;
        for words in parts {
            match check(words, &o.gfx) {
                Ok(n) => {
                    counts.states += n.states;
                    counts.strips += n.strips;
                    counts.corners += n.corners;
                }
                Err(r) => {
                    c.refused += 1;
                    if c.refused == 1 {
                        (self.log)(format_args!(
                            "rp-gfx: a frame refused at word {} of {} (part {} of {}): {:?} (0x{:08x})",
                            before + r.at,
                            parts.iter().map(|p| p.len()).sum::<usize>(),
                            before,
                            parts.len(),
                            r.why,
                            words.get(r.at).copied().unwrap_or(0)
                        ));
                    }
                    return Err(Errno::Invalid);
                }
            }
            before += words.len();
        }
        self.ladder_before();
        let (Some(o), Some(regs)) = (self.open.as_mut(), self.regs.as_mut()) else {
            return Err(Errno::BadHandle);
        };
        o.pm_frame = self.frame;
        if let Some(s) = o.overlap_from
            && us_since(o.pm_opened) / 1_000_000 >= s
        {
            // R340's measure: the overlap on from here, the next chain
            // into the second scene while this render draws the first
            o.overlap_from = None;
            o.overlap = true;
            o.next_b = true;
            (self.log)(format_args!(
                "rp-gfx: R340: -rpoverlapat: kit 113's overlap on at frame {} ({s} s)",
                self.frame
            ));
        }
        let c = &mut self.counts;
        let t0 = rt::ticks();
        // Kit 113: with the overlap the render waited for is the one two
        // frames back (its scene is the one this frame's TA reuses), and
        // only while the last frame's scene is still waiting behind it.
        let waited = if !o.overlap {
            let r = o.wait_render(regs);
            if r.is_ok() && o.pm_frame >= PM_SNAP_FROM {
                pm_snap(Moment::RenderDoneAlone, regs, o, 0, self.log);
            }
            r
        } else if o.pending.is_some() {
            let r = o.wait_render(regs);
            if r.is_ok() {
                o.start_pending(regs);
            } else {
                o.pending = None;
            }
            r
        } else {
            Ok(0)
        };
        let w = us_since(t0);
        c.waited_us += w;
        // Kit 113: a wait of more than a few looks means the render was
        // still going: its whole time is the chip's render time.
        if let Ok(r) = waited
            && w >= 50
        {
            c.rwaits += 1;
            c.rwait_us += r;
        }
        let (presses, compare) = CURVE_KEY.with(|k| (k.2, k.4));
        if let Some((k, _, table, _)) = o.curve
            && compare != o.comparing
        {
            // R343: hold to see NOW, let go for the look again
            o.comparing = compare;
            let show = if compare { 0 } else { k };
            curve_set(regs, show, &table);
            o.draw_band(true);
            if let Some((b, _)) = self.light.as_mut() {
                b.set(CURVES[show].1);
            }
            if compare {
                (self.log)(format_args!(
                    "rp-gfx: R343: the owner held Record: NOW against {} at frame {}",
                    CURVES[k].0, self.frame
                ));
            }
        }
        if let Some((k, _, table, gs)) = o.curve
            && presses != o.curve_seen
            && !o.comparing
        {
            // one step a frame, however many presses came (each shown)
            o.curve_seen = o.curve_seen.wrapping_add(1);
            let k = (k + 1) % CURVES.len();
            curve_set(regs, k, &table);
            o.curve = Some((k, rt::ticks(), table, gs));
            o.draw_band(true);
            (self.log)(format_args!(
                "rp-gfx: R343: the owner stepped to the curve {} at frame {}",
                CURVES[k].0, self.frame
            ));
            if let Some((b, _)) = self.light.as_mut() {
                b.set(CURVES[k].1);
            }
        }
        if o.bar {
            o.bar_frames += 1;
            o.bar_hangs += u32::from(waited.is_err());
            o.draw_bar();
        }
        if waited.is_err() {
            c.render_hangs += 1;
            c.run_hangs += 1;
            // R340: every hang said at once, so the card has it even if
            // the run never reaches its next report
            if c.run_hangs <= HANGS_SAID {
                let (slowest, timeouts) = reads();
                (self.log)(format_args!(
                    "rp-gfx: R340: render hang {} at frame {} ({} ms into the screen's time): {timeouts} status reads ended by the bus time-out since the last report, the slowest {slowest} us; the core reset",
                    c.run_hangs,
                    self.frame,
                    us_since(self.opened) / 1000,
                ));
            }
            // The core's recovery leaves it likely to hang again at
            // divide 1 above 133 MHz (R295 run 25: about half the next
            // renders); one background-only render after it ends that
            // (run 26: 16%).
            o.warm_owed = true;
        }
        let bg = background();
        if o.warm_owed {
            // The background alone through the TA and the core, its
            // result not counted.
            o.warm_owed = false;
            // kit 113: the first scene's memory, the next chain the second's
            o.render_b = false;
            o.next_b = o.overlap;
            let _ = ta::clear(o.vram, &o.lay.ta);
            cpu::drain_write_buffer();
            let mut f = ta::Feed::start(regs, &o.ready, &o.lay.ta, CLOCK, TA_LIMIT_US);
            let _ = ta::draws(&[&bg]).all(|i| f.put(i));
            if f.finish().done {
                o.start_render(regs);
                let _ = o.wait_render(regs);
            } else {
                o.recover(regs);
            }
        }
        // Step 4f's probe on its frames (not counted with the others).
        if self.ladder.is_none()
            && let Some(p) = self.probe.as_mut()
            && p.due()
        {
            let r = p.frame(o, regs, parts, &bg, self.log);
            self.frame = self.frame.wrapping_add(1);
            self.report();
            return r;
        }
        // R340's escalation, R344's cure (runs 276-278: after the core's
        // reset, and at Windows Mobile's clock too, every chain stuck
        // writing the 2700G's own memory; only a reset cleared it): chains
        // that keep failing take the full reset and rebuild (~0.3 s), the
        // frame not drawn, and the program told its textures are gone
        if c.chain_fails_row == 0 && c.cpu_fails_row == 0 {
            self.rebuilt_at = None;
        }
        if c.chain_fails_row >= CHAIN_FAILS_OFF && rebuild_due(self.rebuilt_at) {
            return Err(self.rebuild_now("chains in a row did not finish"));
        }
        if self
            .rebuild_at
            .is_some_and(|s| us_since(self.opened) / 1_000_000 >= s)
        {
            // R344's test: the render in flight finished first (a healthy
            // chip), then the same path as a stall's
            self.rebuild_at = None;
            if let (Some(o), Some(regs)) = (self.open.as_mut(), self.regs.as_mut()) {
                let _ = o.wait_render(regs);
                if o.pending.is_some() {
                    o.start_pending(regs);
                    let _ = o.wait_render(regs);
                }
            }
            return Err(self.rebuild_now("-rprebuild: forced"));
        }
        // Step 4f: the frame as one DMA chain, the CPU let go at once.
        if self.ladder.is_none()
            && let Some(ch) = self.chain.as_mut()
        {
            let t1 = rt::ticks();
            if self.chain_check > 0 {
                // the CPU's picture of this frame, then the chain's
                self.chain_check -= 1;
                let height = o.lay.ta.height;
                let fed = probe4f::feed_cpu(o, regs, parts, &bg);
                let cpu = if fed.done {
                    probe4f::render_hash(o, regs, height)
                } else {
                    None
                };
                if cpu.is_none() {
                    o.recover(regs);
                }
                let chain = match ch.start(regs, &o.lay.ta, &bg, parts, ta_eps_now(self.log)) {
                    Ok(true) => match ch.settle(regs) {
                        chain::Settled::Done(_) => probe4f::render_hash(o, regs, height),
                        _ => None,
                    },
                    _ => None,
                };
                match (cpu, chain) {
                    (Some(a), Some(b)) if a == b => c.check_same += 1,
                    (Some(_), Some(_)) => c.check_differ += 1,
                    _ => {
                        c.check_failed += 1;
                        o.recover(regs);
                    }
                }
                self.frame = self.frame.wrapping_add(1);
                self.report();
                return Ok(());
            }
            // R340 (run 310): a render must not end under a running TA.
            let mut defer = false;
            if o.overlap
                && HOLD_END.with(|h| h.on)
                && let Some(t0) = o.rendering
                && !o.render_done_now(regs)
            {
                let left = o.render_us.saturating_sub(us_since(t0));
                if !HOLD_END.with(|h| h.beside_ok) || left < o.ta_us + END_MARGIN_US {
                    if HOLD_END.with(|h| h.defer) && o.pending.is_none() {
                        // the arm put off to the pump, the program let go
                        defer = true;
                    } else {
                        let t1 = rt::ticks();
                        let _ = o.wait_render(regs);
                        if o.pending.is_some() {
                            o.start_pending(regs);
                        }
                        HOLD_END.with(|h| {
                            h.held += 1;
                            h.held_us = h.held_us.wrapping_add(us_since(t1));
                        });
                    }
                } else {
                    HOLD_END.with(|h| h.beside += 1);
                }
            }
            if defer {
                // (armed once the render has ended: the chip then holds
                // no other scene, so the fits check has nothing to weigh)
                let into = o.next_b;
                if ch.prepare(&o.ta_lay(into), &bg, parts, ta_eps_now(self.log)) {
                    o.defer_t0 = Some(rt::ticks());
                    frame_ev(
                        FrameKind::Prepare,
                        o.pm_frame,
                        ch.last_words,
                        u32::from(o.rendering.is_some()) << 31,
                    );
                    bisect_step(c.ev[3], o.scene_pages, self.log);
                    o.armed_b = into;
                    o.next_b = !into;
                    c.frames += 1;
                    c.chained += 1;
                    c.words += (bg.len() + parts.iter().map(|p| p.len()).sum::<usize>()) as u32;
                    c.strips += counts.strips;
                    c.descriptors += ch.last_descriptors;
                    c.feed_us += us_since(t1);
                    self.frame = self.frame.wrapping_add(1);
                    self.report();
                    return Ok(());
                }
                // too large for the chain: the CPU's, below (it waits for
                // every render first; the build below says so again and
                // counts it)
            }
            // R340's fits check: the next scene's TA starts beside the
            // scenes the chip still holds only when its pages provably fit
            if o.overlap && (o.rendering.is_some() || o.pending.is_some()) {
                let live = o
                    .rendering
                    .map_or(0, |_| o.scene_pages[usize::from(o.render_b)])
                    + o.pending.map_or(0, |b| o.scene_pages[usize::from(b)]);
                c.live_most = c.live_most.max(live);
                if live + o.pages_recent * 3 / 2 + PM_MARGIN > PM_PAGES {
                    c.held_for_room += 1;
                    let _ = o.wait_render(regs);
                    if o.pending.is_some() {
                        o.start_pending(regs);
                        let _ = o.wait_render(regs);
                    }
                }
            }
            o.arm_cursor = pm_cursor(o);
            // kit 63: the TA's memory cleared by the chain itself
            let into = o.overlap && o.next_b;
            frame_ev(
                FrameKind::Arm,
                o.pm_frame,
                (bg.len() + parts.iter().map(|p| p.len()).sum::<usize>()) as u32,
                u32::from(o.rendering.is_some()) << 31,
            );
            match ch.start(regs, &o.ta_lay(into), &bg, parts, ta_eps_now(self.log)) {
                Ok(true) => {
                    if o.rendering.is_some() || o.pending.is_some() {
                        BISECT.with(|b| b.over += 1);
                    }
                    bisect_step(c.ev[3], o.scene_pages, self.log);
                    o.armed_b = into;
                    o.next_b = o.overlap && !into;
                    c.frames += 1;
                    c.chained += 1;
                    c.words += (bg.len() + parts.iter().map(|p| p.len()).sum::<usize>()) as u32;
                    c.strips += counts.strips;
                    c.descriptors += ch.last_descriptors;
                    c.feed_us += us_since(t1);
                    self.frame = self.frame.wrapping_add(1);
                    self.report();
                    return Ok(());
                }
                Ok(false) => c.unfit += 1,
                Err(()) => {
                    c.ta_hangs += 1;
                    o.recover(regs);
                    self.report();
                    return Err(Errno::Io);
                }
            }
        }
        if o.overlap {
            // Kit 113: a frame the chain can't hold goes by the CPU into
            // the first scene once every render has ended; the next
            // chain into the second.
            let _ = o.wait_render(regs);
            o.render_b = false;
            o.next_b = true;
        }
        let _ = ta::clear(o.vram, &o.lay.ta);
        cpu::drain_write_buffer();
        let mut feed = ta::Feed::start(regs, &o.ready, &o.lay.ta, CLOCK, TA_LIMIT_US);
        let _ = ta::draws(&[&bg]).all(|i| feed.put(i));
        // Run 25: a cell's change on every other frame only, the frames
        // between it its control over the same view (the view moves).
        let variant = self.ladder.as_ref().is_some_and(|l| l.n % 2 == 1);
        o.words_on = variant || self.ladder.is_none();
        o.checker_on = o.checker && variant;
        o.overlay_on = o.overlay && variant;
        let part = if variant { o.part } else { Part::All };
        let put = |p| {
            feed.put(match p {
                Piece::Word(w) => ta::Item::Word(w),
                Piece::End => ta::Item::End,
            })
        };
        // The frame as made goes straight to the TA: the split (its 7 KiB
        // strip buffer, a branch a word) cost 3.8 ms a frame in run 29.
        let left_out = if part == Part::All {
            // The frame as runs of words, each in one tight loop.
            drop(put);
            let probe = TA_PROBE.with(|m| *m);
            if probe == 0 {
                for words in parts {
                    send_runs(
                        words,
                        &mut feed,
                        |f, ws| f.put_words(ws),
                        |f| f.put(ta::Item::End),
                    );
                }
            } else {
                // ADR-0040 step 7g6's probe (`-rpta17`, `-rpta21`,
                // `-rpta1721`): Dell's drivers read (2700g.md "Dell's
                // drivers read for unused features") suggest the strip
                // header's bit 17 takes 16.16 vertex words and bit 21 no z
                // word. Each strip goes with the bit set and its corners
                // rewritten to match: the picture tells whether the chip
                // reads them so.
                let mut corners_next = false;
                for words in parts {
                    send_runs(
                        words,
                        &mut feed,
                        |f, ws| {
                            if ws.len() == 1 && matches!(ws[0] >> 28, 4 | 5) {
                                corners_next = true;
                                let mut h = ws[0];
                                if probe & 1 != 0 {
                                    h |= 1 << 17;
                                }
                                if probe & 2 != 0 {
                                    h |= 1 << 21;
                                }
                                return f.put_words(&[h]);
                            }
                            if !core::mem::replace(&mut corners_next, false) {
                                return f.put_words(ws);
                            }
                            let mut buf = [0u32; 7];
                            for c in ws.chunks_exact(7) {
                                let mut n = 0;
                                for (k, &w) in c.iter().enumerate() {
                                    if k == 2 && probe & 2 != 0 {
                                        continue;
                                    }
                                    buf[n] = if k == 4 || probe & 1 == 0 {
                                        w
                                    } else {
                                        fix16(w)
                                    };
                                    n += 1;
                                }
                                if !f.put_words(&buf[..n]) {
                                    return false;
                                }
                            }
                            true
                        },
                        |f| f.put(ta::Item::End),
                    );
                }
            }
            0
        } else {
            let mut split = Split {
                part,
                put,
                state_left: 0,
                state_total: 7,
                in_prim: None,
                dropping: false,
                model: false,
                held: None,
                buf: [0; STRIP_MOST],
                left_out: 0,
            };
            for words in parts {
                send(words, |p| split.put(p));
            }
            split.left_out
        };
        if o.overlay_on {
            let ov = overlay();
            let _ = ta::draws(&[&ov]).all(|i| feed.put(i));
        }
        let fed = feed.finish();
        c.frames += 1;
        c.words += fed.words;
        c.strips += counts.strips;
        c.feed_us += fed.feed_us;
        c.ta_us += fed.ta_us;
        c.cpu_fails_row = if fed.done { 0 } else { c.cpu_fails_row + 1 };
        if !fed.done && c.cpu_fails_row >= CPU_FAILS_CLOCK && rebuild_due(self.rebuilt_at) {
            // R340's escalation, R344's cure: as the chain's above
            c.ta_hangs += 1;
            c.run_hangs += 1;
            return Err(self.rebuild_now("CPU feeds in a row the TA never finished"));
        }
        if !fed.done {
            c.ta_hangs += 1;
            c.run_hangs += 1;
            if c.ta_hangs == 1 {
                (self.log)(format_args!(
                    "rp-gfx: the TA never said done ({} words, stuck {:?}): the core reset",
                    fed.words, fed.stuck
                ));
            }
            o.recover(regs);
            if let Some(l) = self.ladder.as_mut() {
                l.ta_hangs += 1;
                l.mark(b'T');
            }
            self.report();
            return Err(Errno::Io);
        }
        o.start_render(regs);
        self.frame = self.frame.wrapping_add(1);
        if let Some(l) = self.ladder.as_mut() {
            l.left_out += left_out;
            match o.wait_render(regs) {
                Ok(us) => {
                    l.ok += 1;
                    l.render_us += us;
                    l.max_us = l.max_us.max(us);
                    l.ok_block = Some(read_block(regs));
                    l.mark(if variant { b',' } else { b'.' });
                }
                Err(_) => {
                    c.render_hangs += 1;
                    l.hangs += 1;
                    if variant {
                        l.hangs_b += 1;
                    }
                    if let Some(n) = l.marks.get_mut(o.hang_marks as usize & 31) {
                        *n += 1;
                    }
                    if l.hang_block.is_none() {
                        l.hang_block = o.hang_block;
                    }
                    l.mark(if variant { b'h' } else { b'H' });
                    o.warm_owed = o.warm;
                }
            }
        } else if self.frame.is_multiple_of(TIMED_EVERY) {
            match o.wait_render(regs) {
                Ok(us) => {
                    c.timed += 1;
                    c.render_us += us;
                }
                Err(_) => {
                    c.render_hangs += 1;
                    o.warm_owed = true;
                }
            }
        }
        self.report();
        Ok(())
    }

    fn pump(&mut self) {
        // R340 (run 281): the rebuild at once, from the tick (~0.5 s), so
        // neither the pump's reads nor the program's writes wait out the
        // bus time-out on a wedged chip in the meantime
        // R340: a rebuild owed is the next frame's (run 286: one run from
        // here, the tick's interrupt, never finished: a white screen and
        // the watchdog); meanwhile the pump leaves the chip alone, so its
        // reads can't starve the program on a wedged chip
        if mute_take() {
            self.counts.rebuild_owed = true;
        }
        if self.counts.rebuild_owed {
            // run 297: the chain in flight cut at once (its DMA would go
            // on into the silent chip a time-out a descriptor, holding
            // the bus the CPU's memory reads need: 7 to 11 s a stall)
            if let Some((desc, us)) = self.chain.as_mut().and_then(chain::Chain::cut) {
                let n = QUIET.with(|q| {
                    q.cuts += 1;
                    q.cut_at = (desc, us);
                    q.cuts
                });
                if n <= HANGS_SAID {
                    (self.log)(format_args!(
                        "rp-gfx: R340: the chip stopped answering: the frame's chain cut at descriptor {desc}, {us} us after its arm (cut {n})"
                    ));
                    match EV9.with(|e| e.last) {
                        Some((t, d)) => (self.log)(format_args!(
                            "rp-gfx: R340: the last event bit 9 before it: {} us earlier, {}",
                            us_since(t),
                            if d == u32::MAX {
                                "no chain in flight"
                            } else {
                                "in a chain"
                            }
                        )),
                        None => (self.log)(format_args!("rp-gfx: R340: no event bit 9 before it")),
                    }
                }
            }
            return;
        }
        let (Some(o), Some(regs), Some(ch)) =
            (self.open.as_mut(), self.regs.as_mut(), self.chain.as_mut())
        else {
            return;
        };
        o.chip_events(regs, &mut self.counts, self.log, ch.descriptor());
        // R340: the frame waiting for its arm, the moment its render ended
        Self::arm_waiting(&mut self.counts, o, regs, ch, false);
        if ch.busy() {
            let s = ch.pump(regs);
            Self::settled(
                &mut self.counts,
                self.log,
                o,
                regs,
                s,
                true,
                ch.last_gap_us,
                ch.last_fail.take(),
            );
        }
        if o.pending.is_some() {
            o.start_pending(regs);
        }
    }

    fn look(&mut self, level: u8) -> bool {
        if level > PICTURE_STEPS {
            return false;
        }
        PICTURE.with(|p| *p = Some(picture_table(level)));
        // on the screen now, unless the picker has the table (its looks
        // are the owner's to step) or the chip is lost (the rebuild sets
        // the look again, this step with it)
        if !self.curves
            && !self.counts.rebuild_owed
            && let (Some(o), Some(regs)) = (self.open.as_mut(), self.regs.as_mut())
            && let Some((k, _, table, _)) = o.curve
            && k == LOOK_DEFAULT
        {
            curve_set(regs, LOOK_DEFAULT, &table);
        }
        (self.log)(format_args!(
            "rp-gfx: R343: the picture's brightness at step {level} of {PICTURE_STEPS}"
        ));
        true
    }

    fn quiet(&mut self, hold: bool) -> bool {
        QUIET.with(|q| q.calls = q.calls.wrapping_add(1));
        if mute_take() {
            self.counts.rebuild_owed = true;
        }
        if self.counts.rebuild_owed {
            QUIET.with(|q| q.skipped += 1);
            return false;
        }
        let Some(s) = QUIET.with(|q| q.hold_s).filter(|_| hold) else {
            return true;
        };
        if s != 0 && us_since(self.opened) / 1_000_000 >= s {
            return true;
        }
        let (Some(o), Some(regs), Some(ch)) =
            (self.open.as_mut(), self.regs.as_mut(), self.chain.as_ref())
        else {
            return true;
        };
        QUIET.with(|q| q.looks += 1);
        // The lever: a write only while the TA and a render are not both
        // at work (either alone is the serial path's case, which never
        // stalled: run 279). The render is the one waited for: it ends
        // by itself, and the chain goes on feeding the TA meanwhile.
        if !o.overlap || !ch.busy() || o.render_done_now(regs) {
            return true;
        }
        let t0 = rt::ticks();
        // (a render that hung is reset there, as a frame's wait does: the
        // write goes on either way)
        let _ = o.wait_render(regs);
        QUIET.with(|q| {
            q.held += 1;
            q.held_us = q.held_us.wrapping_add(us_since(t0));
        });
        true
    }

    fn wait(&mut self) -> Result<(), Errno> {
        self.settle();
        let (Some(o), Some(regs)) = (self.open.as_mut(), self.regs.as_mut()) else {
            return Err(Errno::BadHandle);
        };
        // Kit 113: a scene waiting behind the render drawn too.
        o.wait_render(regs)?;
        if o.pending.is_some() {
            o.start_pending(regs);
            o.wait_render(regs)?;
        }
        Ok(())
    }

    fn close(&mut self) -> Result<(), Errno> {
        let (Some(regs), true) = (self.regs.as_mut(), self.open.is_some()) else {
            return Err(Errno::BadHandle);
        };
        let Some(mut o) = self.open.take() else {
            return Err(Errno::BadHandle);
        };
        crate::cpu::put_back(self.log);
        if let Some(ch) = self.chain.as_mut() {
            Self::arm_waiting(&mut self.counts, &mut o, regs, ch, true);
            let s = ch.settle(regs);
            Self::settled(
                &mut self.counts,
                self.log,
                &mut o,
                regs,
                s,
                false,
                ch.last_gap_us,
                ch.last_fail.take(),
            );
        }
        let _ = o.wait_render(regs);
        if o.pending.is_some() {
            o.start_pending(regs);
            let _ = o.wait_render(regs);
        }
        if o.stepped {
            // Windows Mobile's clock and bus timing back (ADR-0035 rule 2),
            // the underrun flag checked after (rule 8).
            let a = clock::put_back(regs, rt::delay_us, set_cs3);
            (self.log)(format_args!(
                "rp-gfx: clock put back to Windows Mobile's ({}); the underrun flag {} (cleared)",
                match a {
                    Some(a) if a.unwritten == 0 => "locked",
                    Some(_) => "a memory-controller write never held",
                    None => "the PLL did not lock",
                },
                if underrun_take(regs) { "set" } else { "clear" }
            ));
        }
        if let Some((_, _, table, gs)) = o.curve {
            for (k, v) in table.iter().enumerate() {
                regs.write32(rp_drv_mbx_display::reg::GFXGAM0 + 4 * k, *v);
            }
            regs.write32(rp_drv_mbx_display::reg::GSCTRL, gs);
        }
        clock::restore(regs, o.clk0);
        if let Some(p) = o.moved {
            graphics_set(regs, &p, Layer::SOLO, false);
        }
        regs.write32(SYSCFG, o.syscfg0);
        if let Some((b, found)) = self.light.take() {
            // R341: the backlight as the runner found it
            use rp_drv_pxa_pwm::reg as pwm;
            let mut p = b.into_inner();
            p.write32(pwm::CR, found[0]);
            p.write32(pwm::PCR, found[1]);
            p.write32(pwm::DCR, found[2]);
            (self.log)(format_args!(
                "runner: R341: the backlight put back as found"
            ));
        }
        (self.log)(format_args!("rp-gfx: the screen given back"));
        panel::give_back(o.vram);
        Ok(())
    }
}
