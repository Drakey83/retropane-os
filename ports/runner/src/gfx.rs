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
//! | `0x200000` | the TA's parameter memory (5 MiB, below the manager's 8 MiB reach) |
//! | `0x800000` | the program's texture memory (7 MiB) |

use core::fmt::Arguments;

use rp_drv_blit2d::{MBX, reg};
use rp_drv_mbx_display::clock::{self, MBXCLK, with_3d};
use rp_drv_mbx_display::layers::Layer;
use rp_drv_mbx_display::{PixelFormat as PlaneFormat, Plane, ack_events, graphics_set};
use rp_drv_mbx3d::{self as m3, ta};
use rp_hal_api::RegisterBus;
use rp_hal_cabi::{Errno, Gfx};
use rp_hal_mem::{load32_volatile, store32_volatile};
use rp_hal_pxa27x::{Mmio, cpu, rt};
use rp_mbx3d_render::{Picture, PixelFormat, Target, TileOrder, binner, fixed, order_tile};
use rp_port::gfx::{Piece, Screen, check, send};

use super::panel;

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
#[allow(dead_code, reason = "run 18's cell 5 (no cure); kept for R295's next cells")]
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
                    let k = 8 - self.state_left;
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
                        (2, Part::ModelsNoAlphaTest) if self.model => (w & 0x001F_FFFF) | 0xE000_0000,
                        (3, Part::TinyTextures) => w & !(0xFF << 20),
                        _ => w,
                    };
                    return (self.put)(Piece::Word(w));
                }
                if w == rp_port::gfx::STATE {
                    self.state_left = 7;
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
#[allow(dead_code, reason = "run 18's cell 2 (clean)")]
const P117: (u32, u32, u32) = (27, 3, 0);
#[allow(dead_code, reason = "run 19's cells")]
const P133: (u32, u32, u32) = (41, 4, 0);
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
    }
}

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
const CELLS: [Cell; 5] = [
    cell(P108, &LAY_SDRAM, NONE, "control", 40),
    warmed(checkered(cell(P136, &LAY_SDRAM, NONE, "the region array in the checker order every other frame, a warm-up after each hang", 160))),
    warmed(checkered(cell(P136, &LAY_SDRAM, NONE, "the checker order again, a warm-up after each hang", 160))),
    checkered(cell(P108, &LAY_SDRAM, NONE, "the checker order at 108.33 (does it draw the same?)", 40)),
    cell(P108, &LAY_SDRAM, NONE, "control again", 40),
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
/// A render past this has hung: the core is reset.
const RENDER_LIMIT_US: u32 = 200_000;
/// Every this many frames the render is waited for at once and timed.
const TIMED_EVERY: u32 = 16;
/// The background: opaque black.
const BLACK: u32 = 0xFF00_0000;

/// The display's underrun flag (`DINTRS` bit 4, the graphics stream's
/// fetch run short), cleared Dell's way; whether it was set (ADR-0035
/// rule 8: checked after every clock change).
const GFX_UNDERRUN: u32 = 1 << 4;

fn underrun_take(regs: &mut Mmio) -> bool {
    ack_events(regs, GFX_UNDERRUN) & GFX_UNDERRUN != 0
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
}

/// The screen while the program has it.
struct Open {
    vram: &'static mut [u8],
    ready: ta::Ready,
    /// `MBXCLK` as found, put back at the close.
    clk0: u32,
    gfx: Gfx,
    /// A render started and not yet seen done: when it started.
    rendering: Option<u32>,
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
}

/// The 2700G as `rp_port`'s [`Screen`].
pub struct Chip {
    /// The 2700G's registers (none without a display).
    regs: Option<Mmio>,
    open: Option<Open>,
    log: fn(Arguments<'_>),
    counts: Counts,
    since: u32,
    frame: u32,
    /// The 2700G's clock while the program draws (a test run's
    /// `-rpclock`), or the loader's as found.
    rung: Option<clock::Rung>,
    /// R295's diagnostic ladder asked for (`-rpladder`), and where it is.
    ladder_on: bool,
    ladder: Option<Ladder>,
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
            },
            since: 0,
            frame: 0,
            rung: None,
            ladder_on: false,
            ladder: None,
        }
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
        if us_since(self.since) < 5_000_000 {
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
        self.counts = Counts::default();
        self.since = rt::ticks();
    }
}

impl Chip {
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
            (self.log)(format_args!("rp-gfx ladder: over; the run's own clock and layout again"));
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
            None => (self.rung.filter(|r| rung_allowed(*r)), &LAY_SDRAM, NONE, RENDER_LIMIT_US),
        };
        if rung.is_some_and(|r| !rung_allowed(r)) {
            log(format_args!("rp-gfx ladder: cell {} past the game step, not taken", k + 1));
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
                if ok { "stepped" } else { "NOT taken, Windows Mobile's put back" },
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
                o.warm_owed = false;
            }
            Err(e) => {
                log(format_args!("rp-gfx ladder: the TA's set-up in {} refused: {e:?}", lay.name));
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
        if c.part != Part::All || !c.words.is_empty() || c.checker {
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
                        row[0], row[1], row[2], row[3], row[4], row[5], row[6], row[7]
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
                    row[0], row[1], row[2], row[3], row[4], row[5], row[6], row[7]
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

    /// Waits for the render in flight, if any: how long it took to be seen
    /// done, or `Err` if it hung (the core then reset and set up again).
    fn wait_render(&mut self, regs: &mut Mmio) -> Result<u32, Errno> {
        let Some(t0) = self.rendering.take() else {
            return Ok(0);
        };
        let ok = loop {
            if regs.read32(MBX + reg::STATUS) & m3::RENDER_DONE != 0 {
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

    /// Starts the 3D stage on the TA's region array, into the screen.
    fn start_render(&mut self, regs: &mut Mmio) {
        if self.marking {
            self.put_marks();
        }
        self.en = regs.read32(MBX + reg::ENABLE);
        regs.write32(MBX + reg::CLEAR, m3::RENDER_DONE);
        regs.write32(MBX + reg::ENABLE, self.en | m3::RENDER_DONE);
        let mut from = self.lay.from;
        if self.checker_on {
            let t0 = rt::ticks();
            self.copy_checker();
            self.checker_us += us_since(t0);
            self.checker_n += 1;
            from.regions = CHK_REGIONS;
        }
        self.rendering = Some(rt::ticks());
        for (off, v) in binner::start_table(&from, SCREEN) {
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
                            if other.is_none() { "the PLL did not lock" } else { "a memory-controller write never held" }
                        ));
                    }
                }
            }
        }
        let ready = match ta::set_up(
            regs,
            vram,
            TA_PM,
            &KEEP,
            rt::delay_us,
            cpu::drain_write_buffer,
            CLOCK,
        ) {
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
                panel::give_back(vram);
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
        };
        (self.log)(format_args!(
            "rp-gfx: the screen is the program's: 480 x 640 drawn by the 3D core into the screen, MBXCLK 0x{clk0:x}, texture memory {} KiB at 0x{TEX_AT:x}",
            TEX_BYTES / 1024
        ));
        self.open = Some(Open {
            vram,
            ready,
            clk0,
            gfx,
            rendering: None,
            en: 0,
            moved,
            stepped,
            cur: if stepped { self.rung } else { None },
            lay: &LAY_SDRAM,
            words: NONE,
            words_on: true,
            limit_us: RENDER_LIMIT_US,
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
        });
        self.since = rt::ticks();
        Ok(gfx)
    }

    fn frame(&mut self, words: &[u32]) -> Result<(), Errno> {
        let (Some(o), true) = (self.open.as_mut(), self.regs.is_some()) else {
            return Err(Errno::BadHandle);
        };
        let c = &mut self.counts;
        let counts = match check(words, &o.gfx) {
            Ok(n) => n,
            Err(r) => {
                c.refused += 1;
                if c.refused == 1 {
                    (self.log)(format_args!(
                        "rp-gfx: a frame refused at word {} of {}: {:?} (0x{:08x})",
                        r.at,
                        words.len(),
                        r.why,
                        words.get(r.at).copied().unwrap_or(0)
                    ));
                }
                return Err(Errno::Invalid);
            }
        };
        self.ladder_before();
        let (Some(o), Some(regs)) = (self.open.as_mut(), self.regs.as_mut()) else {
            return Err(Errno::BadHandle);
        };
        let c = &mut self.counts;
        let t0 = rt::ticks();
        let waited = o.wait_render(regs);
        c.waited_us += us_since(t0);
        if waited.is_err() {
            c.render_hangs += 1;
        }
        let bg = background();
        if o.warm_owed {
            // The background alone through the TA and the core, its
            // result not counted.
            o.warm_owed = false;
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
        let _ = ta::clear(o.vram, &o.lay.ta);
        cpu::drain_write_buffer();
        let mut feed = ta::Feed::start(regs, &o.ready, &o.lay.ta, CLOCK, TA_LIMIT_US);
        let _ = ta::draws(&[&bg]).all(|i| feed.put(i));
        // Run 25: a cell's change on every other frame only, the frames
        // between it its control over the same view (the view moves).
        let variant = self.ladder.as_ref().is_some_and(|l| l.n % 2 == 1);
        o.words_on = variant || self.ladder.is_none();
        o.checker_on = o.checker && variant;
        let mut split = Split {
            part: if variant { o.part } else { Part::All },
            put: |p| {
                feed.put(match p {
                    Piece::Word(w) => ta::Item::Word(w),
                    Piece::End => ta::Item::End,
                })
            },
            state_left: 0,
            in_prim: None,
            dropping: false,
            model: false,
            held: None,
            buf: [0; STRIP_MOST],
            left_out: 0,
        };
        send(words, |p| split.put(p));
        let left_out = split.left_out;
        let fed = feed.finish();
        c.frames += 1;
        c.words += fed.words;
        c.strips += counts.strips;
        c.feed_us += fed.feed_us;
        c.ta_us += fed.ta_us;
        if !fed.done {
            c.ta_hangs += 1;
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
                Err(_) => c.render_hangs += 1,
            }
        }
        self.report();
        Ok(())
    }

    fn wait(&mut self) -> Result<(), Errno> {
        let (Some(o), Some(regs)) = (self.open.as_mut(), self.regs.as_mut()) else {
            return Err(Errno::BadHandle);
        };
        o.wait_render(regs).map(|_| ())
    }

    fn close(&mut self) -> Result<(), Errno> {
        let (Some(regs), true) = (self.regs.as_mut(), self.open.is_some()) else {
            return Err(Errno::BadHandle);
        };
        let Some(mut o) = self.open.take() else {
            return Err(Errno::BadHandle);
        };
        let _ = o.wait_render(regs);
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
        clock::restore(regs, o.clk0);
        if let Some(p) = o.moved {
            graphics_set(regs, &p, Layer::SOLO, false);
        }
        (self.log)(format_args!("rp-gfx: the screen given back"));
        panel::give_back(o.vram);
        Ok(())
    }
}
