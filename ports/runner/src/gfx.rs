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
use rp_drv_mbx_display::{PixelFormat as PlaneFormat, Plane, graphics_set};
use rp_drv_mbx3d::{self as m3, ta};
use rp_hal_api::RegisterBus;
use rp_hal_cabi::{Errno, Gfx};
use rp_hal_pxa27x::{Mmio, cpu, rt};
use rp_mbx3d_render::{Picture, PixelFormat, Target, binner, fixed};
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
/// The TA's time from its start to done, the feeding included (a frame
/// of c1a0 is ~70,000 words: ~8 ms of bus).
const TA_LIMIT_US: u32 = 500_000;
/// A render past this has hung: the core is reset.
const RENDER_LIMIT_US: u32 = 200_000;
/// Every this many frames the render is waited for at once and timed.
const TIMED_EVERY: u32 = 16;
/// The background: opaque black.
const BLACK: u32 = 0xFF00_0000;

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
        }
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
            if us_since(t0) > RENDER_LIMIT_US {
                break false;
            }
        };
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

    /// Starts the 3D stage on the TA's region array, into the screen.
    fn start_render(&mut self, regs: &mut Mmio) {
        self.en = regs.read32(MBX + reg::ENABLE);
        regs.write32(MBX + reg::CLEAR, m3::RENDER_DONE);
        regs.write32(MBX + reg::ENABLE, self.en | m3::RENDER_DONE);
        self.rendering = Some(rt::ticks());
        for (off, v) in binner::start_table(&FROM_TA, SCREEN) {
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
        });
        self.since = rt::ticks();
        Ok(gfx)
    }

    fn frame(&mut self, words: &[u32]) -> Result<(), Errno> {
        let (Some(o), Some(regs)) = (self.open.as_mut(), self.regs.as_mut()) else {
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
        let t0 = rt::ticks();
        let waited = o.wait_render(regs);
        c.waited_us += us_since(t0);
        if waited.is_err() {
            c.render_hangs += 1;
        }
        let _ = ta::clear(o.vram, &TA);
        cpu::drain_write_buffer();
        let bg = background();
        let mut feed = ta::Feed::start(regs, &o.ready, &TA, CLOCK, TA_LIMIT_US);
        let _ = ta::draws(&[&bg]).all(|i| feed.put(i));
        send(words, |p| {
            feed.put(match p {
                Piece::Word(w) => ta::Item::Word(w),
                Piece::End => ta::Item::End,
            })
        });
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
            self.report();
            return Err(Errno::Io);
        }
        o.start_render(regs);
        self.frame = self.frame.wrapping_add(1);
        if self.frame.is_multiple_of(TIMED_EVERY) {
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
        clock::restore(regs, o.clk0);
        if let Some(p) = o.moved {
            graphics_set(regs, &p, Layer::SOLO, false);
        }
        (self.log)(format_args!("rp-gfx: the screen given back"));
        panel::give_back(o.vram);
        Ok(())
    }
}
