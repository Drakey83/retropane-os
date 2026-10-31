//! Run WG (`RP_GPUBENCH_WEATHER`): the Weather app's movement on the
//! planes, the device tests of docs/research/sheets/weather-gpu-movement.md
//! as the owner cut them (2026-10-30: the ~5 minute run, the entrances
//! Reveal and Cascade; M5's Stagger part and M6 dropped). ADR-0034 C4 as
//! changed ("L2"): screen 0 (RGB565, the graphics plane, the bottom slot)
//! holds the wallpaper and chrome, still; the page's glass is a
//! per-pixel-alpha picture on the video plane above it, moved by repointing
//! in the blanking.
//!
//! - **Unattended, judged by `DSIG`** (the display's output signature, one
//!   a refresh: R101, R110, R115). Every state a test shows has its
//!   reference: shown, held a whole refresh with nothing written, its
//!   signature read. A refresh is then whole as due, the state before
//!   (late), another state, or torn. The emulator doesn't model `DSIG`
//!   (reads 0): there the run checks its wiring, the pictures through the
//!   scan-out model, and the 2D engine's pictures against the CPU's.
//!   - M1: ARGB8888 and ARGB4444 + dither on the video plane over screen 0,
//!     per-pixel alpha (a clear window must read as screen 0 alone), the
//!     alpha ignored (opaque), Fade's probe (mode 3 with the global alpha
//!     set), the underrun flag over a hold.
//!   - M2: the 480 x 462 window at y 94 over the 480 x 1,332 picture:
//!     the scripted fling and rebound (300 frames, twice), then Reveal and
//!     Cascade (the window's place, size and start changed together).
//!   - M3: the turn's 960-wide strip (stride 3,840) in 4-pixel steps, and
//!     to the pixel by the window's place for the last 0-3.
//!   - M5: the 2D engine building ARGB pictures (an alpha-0 fill, row
//!     copies, radar cells, the neighbour page, the hand-back composed into
//!     a screen), timed, each checked word for word.
//!   - M4: hand-offs between the page composed into a screen (M5's) alone
//!     and screen 0 + the page layer, 50 each way, written in the blanking.
//! - **Watched** (started by a button): the two depths in turn on one
//!   place (a plane holds one format, so in place rather than side by
//!   side), the turn dragged slowly in 4- and 1-pixel steps, hand-offs at
//!   speed, Reveal, Cascade and the fling at speed.
//! - **M7 last** (R133): with the page showing, the 2D clock off (`MBXCLK`
//!   bits 1-0 = 0; the 3D clock is already off at Windows Mobile's `0x5`),
//!   then `MBXCLK` 0 (Dell's off path, REA #39), then the on-die buffer to
//!   active low power (`ODFBPWR` 1), one per step, each preceded by its
//!   marker in the kept log (ADR-0035 rule 5); the owner says if the
//!   screen held; the 2D clock's wake timed.
//!
//! No 3D render anywhere (ADR-0034 amendment P rules 2 and 4: every frame
//! here is 3D-idle, so the planes may read outside the safe region), no
//! clock change (ADR-0035 rules 2 and 8: the loader's Windows Mobile clock,
//! put back first if found otherwise), every picture a pool block
//! (ADR-0027).

use super::*;
use crate::wg::{self, H, MARGIN, PIC_H, ROW_PITCH, STRIP_W, W, WIN_H, WIN_Y, Win};
use rp_drv_mbx_display::layers::Scale;

/// The on-die frame buffer's power control and state (mbxfb; 2700g.md).
const ODFBPWR: usize = 0x064;
const ODFBSTAT: usize = 0x068;

/// The emulator's check: fewer repeats in the watched block and M7.
const QUICK: bool = option_env!("RP_GPUBENCH_WG_QUICK").is_some();
/// How long the watched block waits for a button, in seconds (then it
/// starts anyway).
const WAIT_SECS: u32 = match option_env!("RP_GPUBENCH_WG_WAIT_SECS") {
    Some(s) => parse_decimal(s, 1),
    None => 120,
};

/// Labelled steps (the label rule).
const STEPS: u32 = 13;
/// Run WG-R (`RP_GPUBENCH_WG_R`, with `RP_GPUBENCH_WEATHER`): R134's ways
/// for Reveal's tears alone (sheet `docs/research/sheets/r134-reveal-tears.md`).
const R134: bool = option_env!("RP_GPUBENCH_WG_R").is_some();
const R134_STEPS: u32 = 11;
/// Screen 0: local memory 0 (Windows Mobile's place).
const S0: u32 = 0;
const S0_STRIDE: u32 = W * 2;
const P8_STRIDE: u32 = W * 4;
const P4_STRIDE: u32 = W * 2;
const STRIP_STRIDE: u32 = STRIP_W * 4;
/// A 480 x 462 32-bit work picture.
const WORK_BYTES: u32 = W * WIN_H * 4;
/// A 2D batch done within this, or stuck.
const LIMIT_US: u32 = 200_000;

/// The run's pictures in 2700G memory.
struct Pics {
    p8: u32,
    p4: u32,
    strip: u32,
    back: u32,
    wall: u32,
    s0c: u32,
}

/// The three tall pictures, pool blocks (5.6 of the bench pool's 6.75
/// MiB).
const SIZES: [u32; 3] = [P8_STRIDE * PIC_H, P4_STRIDE * PIC_H, STRIP_STRIDE * WIN_H];
/// The work pictures and the composed screen in the bench map's fixed
/// regions of the 2D renderer, which this run never starts (the pool
/// can't hold all six: the emulator's check, 2026-10-30).
const BACK: u32 = map::WALLPAPER.at;
const WALL: u32 = map::BACKGROUND.at;
const S0C: u32 = map::CACHE.at;
const _: () = assert!(
    WORK_BYTES <= map::WALLPAPER.size
        && WORK_BYTES <= map::BACKGROUND.size
        && S0_STRIDE * H <= map::CACHE.size
);

/// The run's per-frame times: every sample kept (a pan is at most 640
/// frames), median and worst.
struct Samples {
    v: [u16; 640],
    n: usize,
}

impl Samples {
    const fn new() -> Self {
        Self { v: [0; 640], n: 0 }
    }

    fn add(&mut self, us: u32) {
        if self.n < self.v.len() {
            self.v[self.n] = us.min(u32::from(u16::MAX)) as u16;
            self.n += 1;
        }
    }

    fn median_max(&self) -> (u32, u32) {
        let mut s = self.v;
        s[..self.n].sort_unstable();
        if self.n == 0 {
            (0, 0)
        } else {
            (u32::from(s[self.n / 2]), u32::from(s[self.n - 1]))
        }
    }
}

// ---- drawing ----------------------------------------------------------

/// Screen 0's rows `y0..y1` at `at` with the label `l1`, `l2`.
fn draw_screen(g: &mut Rig, at: u32, (y0, y1): (u32, u32), l1: &str, l2: &str) {
    let mut row = [0u32; 240];
    for y in y0..y1 {
        for (k, w) in row.iter_mut().enumerate() {
            let x = 2 * k as u32;
            *w = u32::from(wg::screen0(x, y, l1, l2))
                | u32::from(wg::screen0(x + 1, y, l1, l2)) << 16;
        }
        g.mem.store((at + y * S0_STRIDE) as usize, &row);
        if y.is_multiple_of(64) {
            rt::check_in();
        }
    }
    g.mem.sync();
}

/// The label (the step and what it is) in screen 0's header, and in the
/// composed screen's when there is one, so the two stay the same picture.
fn label(g: &mut Rig, p: &Pics, step: u32, what: &str, l2: &str) {
    label_of(g, p, (step, STEPS), what, l2);
}

/// [`label`] with its step count given.
fn label_of(g: &mut Rig, p: &Pics, (step, of): (u32, u32), what: &str, l2: &str) {
    let mut l1 = heapless_name();
    let _ = write!(l1, "STEP {step} OF {of}: {what}");
    draw_screen(g, S0, (0, WIN_Y), l1.as_str(), l2);
    if p.s0c != 0 {
        draw_screen(g, p.s0c, (0, WIN_Y), l1.as_str(), l2);
    }
}

/// A picture `w` x `h` at `at` from `f` (ARGB8888), as ARGB8888 or, with
/// `four`, ARGB4444 with the shipped 4x4 ordered dither.
fn draw_pic(g: &mut Rig, at: u32, (w, h): (u32, u32), four: bool, f: &dyn Fn(u32, u32) -> u32) {
    let mut row = [0u32; 480];
    let bpp = if four { 2 } else { 4 };
    for y in 0..h {
        let mut x0 = 0;
        while x0 < w {
            let n = if four {
                960.min(w - x0)
            } else {
                480.min(w - x0)
            };
            if four {
                for (k, word) in row[..(n / 2) as usize].iter_mut().enumerate() {
                    let x = x0 + 2 * k as u32;
                    let d = |x: u32| {
                        u32::from(rp_gfx::layer::to_4444_dithered(
                            f(x, y),
                            x as usize,
                            y as usize,
                        ))
                    };
                    *word = d(x) | d(x + 1) << 16;
                }
                g.mem.store(
                    (at + y * w * bpp + x0 * bpp) as usize,
                    &row[..(n / 2) as usize],
                );
            } else {
                for (k, px) in row[..n as usize].iter_mut().enumerate() {
                    *px = f(x0 + k as u32, y);
                }
                g.mem
                    .store((at + y * w * bpp + x0 * bpp) as usize, &row[..n as usize]);
            }
            x0 += n;
        }
        if y.is_multiple_of(32) {
            rt::check_in();
        }
    }
    g.mem.sync();
}

// ---- the planes -------------------------------------------------------

/// Screen 0 (or the composed screen) alone on the graphics plane: RGB565,
/// the bottom slot, opaque, the gamma bit as found.
fn graphics_screen(g: &mut Rig, at: u32) {
    let plane = Plane {
        format: PlaneFormat::Rgb565,
        width: W as u16,
        height: H as u16,
        offset: at as usize,
        stride: S0_STRIDE as usize,
    };
    let gamma = g.e.regs.read32(dreg::GSCTRL) & (1 << 26);
    for (off, v) in plane.registers() {
        let v = match off {
            dreg::GSCTRL => v | gamma,
            dreg::GSCADR => Layer::SOLO.scadr(v),
            dreg::GBBASE => Layer::SOLO.bbase(v),
            _ => v,
        };
        g.e.regs.write32(off, v);
    }
}

/// `GSCADR` for a screen at `at` as [`graphics_screen`] sets it.
fn gscadr(g: &mut Rig, at: u32) -> u32 {
    let v = g.e.regs.read32(dreg::GSCADR);
    (v & !0x7F_FFFF) | (at / 16)
}

/// Which picture a window reads, and how.
#[derive(Clone, Copy)]
struct Src {
    base: u32,
    stride: u32,
    fmt: VideoFormat,
    blend: Blend,
}

impl Src {
    const fn bpp(self) -> u32 {
        match self.fmt {
            VideoFormat::Argb8888 => 4,
            _ => 2,
        }
    }
}

/// The page layer's registers for window `w` over `s`: the video plane,
/// 1:1 at the window's place, the middle slot.
fn layer_regs(w: Win, s: Src) -> [(usize, u32); 9] {
    let mut v = VideoPlane::new(s.fmt, w.w as u16, w.h as u16, 0);
    v.offset = w.start(s.base, s.stride, s.bpp()) as usize;
    v.stride = s.stride as usize;
    let scale = Scale {
        x: w.x as u16,
        y: w.y as u16,
        ..Scale::NONE
    };
    v.registers_scaled(
        Layer {
            slot: Slot::Middle,
            blend: s.blend,
        },
        false,
        scale,
    )
}

/// The page layer on: the video clock, every register (the stream last).
fn layer_on(g: &mut Rig, r: &[(usize, u32); 9]) {
    g.e.regs.modify(lreg::VOVRCLK, rp_hal_api::Field::bit(0), 1);
    for &(off, v) in r {
        g.e.regs.write32(off, v);
    }
}

/// A moved window: only the registers that changed since `last`, in the
/// driver's order (`VSCADR`, the start and the stream, last). Registers
/// written.
fn layer_move(g: &mut Rig, r: &[(usize, u32); 9], last: &mut [(usize, u32); 9]) -> u32 {
    let mut n = 0;
    for (k, &(off, v)) in r.iter().enumerate() {
        if last[k] != (off, v) {
            g.e.regs.write32(off, v);
            last[k] = (off, v);
            n += 1;
        }
    }
    n
}

/// The page layer off (its stream bit; kept out of the graphics slot).
fn layer_off(g: &mut Rig) {
    rp_drv_mbx_display::video_hide(&mut g.e.regs);
}

/// Waits for the blanking (lines 643-647 wrapping to 2: GC2DC run 2's
/// clean window for plane writes).
fn at_blank(g: &mut Rig) -> bool {
    g.wait_line(643, 5)
}

/// Whether the scan is still in that window (a write landed in time).
fn in_blank(g: &mut Rig) -> bool {
    let l = g.line();
    l >= 643 || l < sig::PUBLISHED_LINE
}

/// The last refresh's signature, once published (line 3 of the next).
fn sig_prev(g: &mut Rig) -> u32 {
    let _ = g.wait_line(sig::PUBLISHED_LINE, 600);
    g.e.regs.read32(DSIG)
}

/// The next refresh's signature: the blanking waited for first, so a
/// refresh is never read twice.
fn sig_next(g: &mut Rig) -> u32 {
    let _ = at_blank(g);
    sig_prev(g)
}

/// A state's reference: `apply` written in the blanking, a whole refresh
/// with nothing written after it, its signature.
fn reference(g: &mut Rig, apply: &mut dyn FnMut(&mut Rig)) -> u32 {
    let _ = at_blank(g);
    apply(g);
    let _ = g.wait_line(sig::PUBLISHED_LINE, 600);
    let _ = at_blank(g);
    let _ = g.wait_line(sig::PUBLISHED_LINE, 600);
    let _ = at_blank(g);
    sig_prev(g)
}

/// A picture's signature over three refreshes, and whether they agreed.
fn steady(g: &mut Rig) -> (u32, bool) {
    let f = FRAME_US * 13 / 4;
    let a = sig::settled(&mut g.e.regs, rt::ticks, f).unwrap_or(0);
    let b = sig::next(&mut g.e.regs, rt::ticks, f).unwrap_or(0);
    (a, a == b)
}

/// FNV-1a over bytes.
fn fnv(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811C_9DC5u32, |h, &b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

/// The registers the scan-out model reads.
const MODEL_REGS: [usize; 15] = [
    dreg::GSCTRL,
    dreg::GSCADR,
    dreg::GSADR,
    dreg::GBBASE,
    dreg::DSCTRL,
    dreg::DHDET,
    dreg::DVDET,
    lreg::VSCTRL,
    lreg::VSCADR,
    lreg::VSADR,
    lreg::VBBASE,
    lreg::VUBASE,
    lreg::VVBASE,
    lreg::VOVRCLK,
    lreg::BGCOLOR,
];

/// The screen as the scan-out model composes it from the registers and
/// memory now, into `out` (RGB888), and its hash.
fn model(g: &mut Rig, out: &mut [u8]) -> u32 {
    let mut r = [(0usize, 0u32); MODEL_REGS.len() + scanout::GAMMA_ENTRIES];
    for (k, off) in MODEL_REGS
        .iter()
        .copied()
        .chain((0..scanout::GAMMA_ENTRIES).map(|i| dreg::GFXGAM0 + 4 * i))
        .enumerate()
    {
        r[k] = (off, g.e.regs.read32(off));
    }
    let n = (W * H * 3) as usize;
    scanout::compose(
        |o| r.iter().find(|&&(k, _)| k == o).map_or(0, |&(_, v)| v),
        &*g.mem.0,
        &mut out[..n],
    );
    fnv(&out[..n])
}

/// The 2D engine's batch submitted and waited for: (done, the CPU's time
/// writing it, the whole time).
fn run2d(g: &mut Rig, words: &[u32]) -> (bool, u32, u32) {
    let t0 = rt::ticks();
    let sent = g.e.submit(words).is_ok();
    let write = us_since(t0);
    let done = sent && g.e.finish(g.mem.0, LIMIT_US).is_ok();
    (done, write, us_since(t0))
}

/// Words of 2700G memory at `a` and `b` (`w` 4-byte words a line, `h`
/// lines, strides in bytes) that differ.
fn differ(g: &mut Rig, (a, sa): (u32, u32), (b, sb): (u32, u32), (w, h): (u32, u32)) -> u32 {
    let mut bad = 0;
    for y in 0..h {
        for x in 0..w {
            let p = g.mem.load((a + y * sa + 4 * x) as usize);
            let q = g.mem.load((b + y * sb + 4 * x) as usize);
            bad += u32::from(p != q || p.is_none());
        }
        if y.is_multiple_of(64) {
            rt::check_in();
        }
    }
    bad
}

// ---- the run ----------------------------------------------------------

#[inline(never)]
pub(super) fn run(g: &mut Rig) {
    log!(
        "WG: the Weather movement on the planes (docs/research/sheets/weather-gpu-movement.md; ADR-0034 C4 as changed, L2): M1 per-pixel alpha on the video plane, M2 the window panned and its entrances, M3 the turn's strip, M5 the 2D engine's ARGB pictures, M4 the hand-offs, judged by DSIG every refresh; then a watched block; M7 (R133) last. No 3D, no clock change"
    );
    wm_clock_first(g);
    if R134 {
        r134(g);
        log!("WG: done");
        return;
    }
    let Some(scratch) = rp_hal_x51v::devices::take_scratch() else {
        log!("  WG: no scratch SDRAM: nothing run");
        return;
    };
    let mut at = [0u32; 3];
    for k in 0..SIZES.len() {
        match g.pool.alloc(SIZES[k], 0x1000) {
            Some(a) => at[k] = a,
            None => {
                log!(
                    "  WG: the pool can't hold the pictures ({} bytes free, the largest block {}, {} wanted): nothing run",
                    g.pool.free_bytes(),
                    g.pool.largest(),
                    SIZES[k]
                );
                for j in 0..k {
                    let _ = g.pool.free(at[j], SIZES[j]);
                }
                return;
            }
        }
    }
    let mut p = Pics {
        p8: at[0],
        p4: at[1],
        strip: at[2],
        back: BACK,
        wall: WALL,
        s0c: 0,
    };
    let t0 = rt::ticks();
    draw_screen(
        g,
        S0,
        (0, H),
        "STEP 1 OF 13: DRAWING",
        "THE SCREEN CHANGES; NOTHING TO DO",
    );
    graphics_screen(g, S0);
    layer_off(g);
    draw_pic(g, p.p8, (W, PIC_H), false, &|x, y| wg::page(x, y, false));
    draw_pic(g, p.p4, (W, PIC_H), true, &|x, y| wg::page(x, y, false));
    draw_pic(g, p.strip, (STRIP_W, WIN_H), false, &wg::strip);
    log!(
        "  WG pictures drawn by the CPU (screen 0 RGB565 at 0x{S0:x}; the page 480 x {PIC_H} ARGB8888 at 0x{:x} and ARGB4444 + dither at 0x{:x}; the strip 960 x {WIN_H} at 0x{:x}): {} ms; MBXCLK 0x{:x}",
        p.p8,
        p.p4,
        p.strip,
        us_since(t0) / 1000,
        g.e.regs.read32(MBXCLK)
    );
    let out = &mut scratch[..(W * H * 3) as usize];
    m1(g, &p, out);
    m2(g, &p);
    m3(g, &p);
    p.s0c = S0C;
    m5(g, &p);
    m4(g, &p, out);
    watched(g, &p);
    m7(g, &p);
    layer_off(g);
    graphics_screen(g, S0);
    for k in 0..SIZES.len() {
        let _ = g.pool.free(at[k], SIZES[k]);
    }
    log!("WG: done");
}

/// M1's layouts: (name, the window and its source, the global alpha over
/// mode 3 when set).
type M1Layout = (&'static str, Option<(Win, Src)>, Option<u8>);

#[inline(never)]
fn m1(g: &mut Rig, p: &Pics, out: &mut [u8]) {
    label(g, p, 1, "PER-PIXEL ALPHA", "UNATTENDED: NOTHING TO DO");
    let s8 = |blend| Src {
        base: p.p8,
        stride: P8_STRIDE,
        fmt: VideoFormat::Argb8888,
        blend,
    };
    let s4 = |blend| Src {
        base: p.p4,
        stride: P4_STRIDE,
        fmt: VideoFormat::Argb4444,
        blend,
    };
    // The window over the top margin's clear lines.
    let clear = Win {
        h: MARGIN,
        ..wg::page_win(-(MARGIN as i32))
    };
    let page = wg::page_win(0);
    let layouts: [M1Layout; 9] = [
        ("screen 0 alone", None, None),
        ("ARGB8888 page", Some((page, s8(Blend::PixelAlpha))), None),
        (
            "ARGB8888 clear lines",
            Some((clear, s8(Blend::PixelAlpha))),
            None,
        ),
        (
            "ARGB8888 page, alpha ignored",
            Some((page, s8(Blend::Opaque))),
            None,
        ),
        ("ARGB4444 page", Some((page, s4(Blend::PixelAlpha))), None),
        (
            "ARGB4444 clear lines",
            Some((clear, s4(Blend::PixelAlpha))),
            None,
        ),
        (
            "ARGB4444 page, alpha ignored",
            Some((page, s4(Blend::Opaque))),
            None,
        ),
        (
            "ARGB8888 page, mode 3 + global 0x80 (Fade)",
            Some((page, s8(Blend::PixelAlpha))),
            Some(0x80),
        ),
        (
            "ARGB8888 page, global 0x80 alone (mode 2)",
            Some((page, s8(Blend::Global(0x80)))),
            None,
        ),
    ];
    let mut sig = [0u32; 9];
    let mut hash = [0u32; 9];
    let mut steady_all = true;
    for (k, &(name, lay, glob)) in layouts.iter().enumerate() {
        let _ = underrun_take(&mut g.e.regs);
        let _ = at_blank(g);
        match lay {
            None => layer_off(g),
            Some((w, s)) => {
                let mut r = layer_regs(w, s);
                if let Some(a) = glob {
                    for e in &mut r {
                        if e.0 == lreg::VBBASE {
                            e.1 = (e.1 & 0x00FF_FFFF) | u32::from(a) << 24;
                        }
                    }
                }
                layer_on(g, &r);
            }
        }
        let (s, same) = steady(g);
        // Held a second for the underrun flag (the scan's bandwidth).
        rt::delay_us(1_000_000);
        let under = underrun_take(&mut g.e.regs);
        sig[k] = s;
        steady_all &= same;
        hash[k] = model(g, out);
        log!(
            "  WG M1 {name}: DSIG 0x{s:08x} ({}), the underrun flag over 1 s {}; the scan-out model 0x{:08x}",
            if same { "steady" } else { "NOT STEADY" },
            if under { "SET" } else { "clear" },
            hash[k]
        );
        rt::check_in();
    }
    layer_off(g);
    let live = sig[0] != 0 && steady_all;
    let judge = |a: bool| {
        if live {
            if a { "yes" } else { "NO" }
        } else {
            "not judged"
        }
    };
    for (fmt, b) in [("ARGB8888", 1), ("ARGB4444", 4)] {
        log!(
            "  WG M1 {fmt} per-pixel alpha on the video plane: the clear lines read as screen 0 alone {}, the page differs from screen 0 {}, the page differs from its alpha ignored {}; the model: {} {} {}",
            judge(sig[b + 1] == sig[0]),
            judge(sig[b] != sig[0]),
            judge(sig[b] != sig[b + 2]),
            hash[b + 1] == hash[0],
            hash[b] != hash[0],
            hash[b] != hash[b + 2]
        );
    }
    log!(
        "  WG M1 Fade (mode 3 with the global alpha 0x80): differs from the page at full {}, from screen 0 {}; mode 2 alone differs from mode 3 + global {}{}",
        judge(sig[7] != sig[1]),
        judge(sig[7] != sig[0]),
        judge(sig[8] != sig[7]),
        if live {
            ""
        } else {
            " (DSIG not live: 0 or unsteady)"
        }
    );
}

/// One sequence of `n` window states, a state a refresh: every state's
/// reference first (`key` names the state of frame `f`, below 1024), then
/// `passes` runs through the sequence, each refresh's `DSIG` judged
/// against its due state's reference.
fn pan(
    g: &mut Rig,
    name: &str,
    n: usize,
    passes: u32,
    key: &dyn Fn(usize) -> usize,
    state: &dyn Fn(usize) -> [(usize, u32); 9],
) {
    pan_with(g, name, (n, passes), key, state, Way::Blank);
}

/// How a run writes each frame's window (R134's ways).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Way {
    /// The changed registers in the driver's order, in lines 643-2 (A).
    Blank,
    /// `VSADR` after `VSCADR` (B).
    VsadrLast,
    /// `VSADR` before the others (C).
    VsadrFirst,
    /// In lines 640-642, the blanking's start (D).
    Early,
    /// `DSCTRL` bit 24 set and the registers written mid-refresh (line
    /// 300), then released in the blanking (G): 0 bit 24 cleared,
    /// 1 `DUCTRL` = 1 twice, 2 `DUCTRL` = 0 then 1.
    Hold(u8),
}

/// The display's update control (R113: Dell writes 1 after a flip).
const DUCTRL: usize = 0x230C;
/// `DSCTRL` bit 24 (`UPDCNT`): holds plane writes (R113).
const UPDCNT: u32 = 1 << 24;

/// `r`'s changed registers written in `order` (1 `VSADR` last, 2 `VSADR`
/// first, the others in the driver's order). Registers written.
fn layer_write(g: &mut Rig, r: &[(usize, u32); 9], last: &mut [(usize, u32); 9], order: u8) -> u32 {
    let vsadr = r.iter().position(|e| e.0 == lreg::VSADR).unwrap_or(3);
    let mut idx = [0usize; 9];
    let mut n = 0;
    if order == 2 {
        idx[0] = vsadr;
        n = 1;
    }
    for k in (0..9).filter(|&k| k != vsadr) {
        idx[n] = k;
        n += 1;
    }
    if order != 2 {
        idx[8] = vsadr;
    }
    let mut w = 0;
    for &k in &idx {
        if last[k] != r[k] {
            g.e.regs.write32(r[k].0, r[k].1);
            last[k] = r[k];
            w += 1;
        }
    }
    w
}

/// One frame's write in `way` (see [`Way`]): (registers written, the line
/// the write ended on, whether it ended inside the way's lines, the CPU's
/// time writing, waits left out).
fn way_write(
    g: &mut Rig,
    way: Way,
    r: Option<&[(usize, u32); 9]>,
    last: &mut [(usize, u32); 9],
) -> (u32, u32, bool, u32) {
    let mut n = 0;
    match way {
        Way::Blank | Way::VsadrLast | Way::VsadrFirst => {
            let _ = at_blank(g);
            let t = rt::ticks();
            let order = match way {
                Way::VsadrLast => 1,
                Way::VsadrFirst => 2,
                _ => 0,
            };
            if let Some(r) = r {
                n = if order == 0 {
                    layer_move(g, r, last)
                } else {
                    layer_write(g, r, last, order)
                };
            }
            let us = us_since(t);
            let l = g.line();
            (n, l, in_blank(g), us)
        }
        Way::Early => {
            let _ = g.wait_line(640, 3);
            let t = rt::ticks();
            if let Some(r) = r {
                n = layer_move(g, r, last);
            }
            let us = us_since(t);
            let l = g.line();
            (n, l, (640..643).contains(&l), us)
        }
        Way::Hold(release) => {
            let _ = g.wait_line(300, 200);
            let t = rt::ticks();
            let d = g.e.regs.read32(dreg::DSCTRL);
            g.e.regs.write32(dreg::DSCTRL, d | UPDCNT);
            if let Some(r) = r {
                n = layer_move(g, r, last);
            }
            let mut us = us_since(t);
            let _ = at_blank(g);
            let t = rt::ticks();
            match release {
                0 => g.e.regs.write32(dreg::DSCTRL, d & !UPDCNT),
                1 => {
                    g.e.regs.write32(DUCTRL, 1);
                    g.e.regs.write32(DUCTRL, 1);
                }
                _ => {
                    g.e.regs.write32(DUCTRL, 0);
                    g.e.regs.write32(DUCTRL, 1);
                }
            }
            us += us_since(t);
            let l = g.line();
            (n, l, in_blank(g), us)
        }
    }
}

/// [`pan`] with each frame written `way`, the first torn refreshes logged
/// (the frame, its state and the one before, the registers and line of
/// its write).
#[inline(never)]
fn pan_with(
    g: &mut Rig,
    name: &str,
    (n, passes): (usize, u32),
    key: &dyn Fn(usize) -> usize,
    state: &dyn Fn(usize) -> [(usize, u32); 9],
    way: Way,
) {
    let mut refs = [0u32; 1024];
    let mut have = [false; 1024];
    let mut last = state(0);
    layer_on(g, &last);
    let t0 = rt::ticks();
    let mut taken = 0;
    for f in 0..n {
        let k = key(f);
        if have[k] {
            continue;
        }
        let r = state(f);
        refs[k] = reference(g, &mut |g| {
            let _ = layer_move(g, &r, &mut last);
        });
        have[k] = true;
        taken += 1;
        if taken % 32 == 0 {
            rt::check_in();
        }
    }
    // The first reference taken again: DSIG repeatable?
    let r0 = state(0);
    let again = reference(g, &mut |g| {
        let _ = layer_move(g, &r0, &mut last);
    });
    let mut distinct = 0;
    for k in 0..refs.len() {
        if have[k] && !(0..k).any(|j| have[j] && refs[j] == refs[k]) {
            distinct += 1;
        }
    }
    let live = refs[key(0)] != 0 && again == refs[key(0)] && distinct > 1;
    let ref_ms = us_since(t0) / 1000;
    let _ = underrun_take(&mut g.e.regs);
    let d0 = g.e.regs.read32(dreg::DSCTRL);
    let (mut due, mut late, mut other, mut torn, mut slow) = (0u32, 0u32, 0u32, 0u32, 0u32);
    let mut cpu = Samples::new();
    let mut shown = 0u32;
    for _ in 0..passes {
        let mut prev = (key(0), key(0));
        // The write that put the due state up: its registers and line.
        let mut wrote = (0u32, 0u32);
        for f in 0..=n {
            let r = (f < n).then(|| state(f));
            let (w, line, ok, us) = way_write(g, way, r.as_ref(), &mut last);
            cpu.add(us);
            slow += u32::from(!ok);
            // The refresh just ended showed the state written a blanking
            // before this one.
            let s = sig_prev(g);
            if f > 0 && live {
                let (want, before) = prev;
                if s == refs[want] {
                    due += 1;
                } else if s == refs[before] {
                    late += 1;
                } else if (0..refs.len()).any(|k| have[k] && refs[k] == s) {
                    other += 1;
                } else {
                    torn += 1;
                    if shown < 16 {
                        shown += 1;
                        log!(
                            "    WG {name}: torn at frame {}: state {want} after {before}, {} registers written, the write ended on line {}",
                            f - 1,
                            wrote.0,
                            wrote.1
                        );
                    }
                }
            }
            wrote = (w, line);
            if f < n {
                prev = (key(f), prev.0);
            }
            if f % 64 == 0 {
                rt::check_in();
            }
        }
    }
    if g.e.regs.read32(dreg::DSCTRL) != d0 {
        g.e.regs.write32(dreg::DSCTRL, d0);
    }
    let under = underrun_take(&mut g.e.regs);
    let (med, worst) = cpu.median_max();
    log!(
        "  WG {name}: {} states, {distinct} distinct signatures, references {ref_ms} ms; {passes} x {n} refreshes: {due} whole as due, {late} the state before (late), {other} another state, {torn} TORN{}; writes outside the way's lines {slow}; the CPU a frame median {med} us worst {worst} us; the underrun flag {}",
        taken,
        if live {
            ""
        } else {
            " (not judged: DSIG 0 or not repeatable)"
        },
        if under { "SET" } else { "clear" }
    );
}

#[inline(never)]
fn m2(g: &mut Rig, p: &Pics) {
    let s8 = Src {
        base: p.p8,
        stride: P8_STRIDE,
        fmt: VideoFormat::Argb8888,
        blend: Blend::PixelAlpha,
    };
    label(g, p, 2, "FLING", "UNATTENDED: NOTHING TO DO");
    let band = MARGIN as i32;
    pan(
        g,
        "M2 fling and rebound (VSCADR, ARGB8888)",
        wg::FLING_FRAMES,
        2,
        &|f| (wg::fling(f) + band) as usize,
        &|f| layer_regs(wg::page_win(wg::fling(f)), s8),
    );
    label(g, p, 3, "REVEAL, CASCADE", "UNATTENDED: NOTHING TO DO");
    // Each cycle grows, then shrinks back.
    let span = 2 * (wg::ENTRANCE_FRAMES as usize + 1);
    let k_of = move |f: usize| {
        let c = f % span;
        if c <= wg::ENTRANCE_FRAMES as usize {
            c
        } else {
            span - 1 - c
        }
    };
    let cycles = if QUICK { 2 } else { 8 };
    pan(
        g,
        "M2 Reveal (VSADR, VSCTRL, SSSIZE and VSCADR a frame)",
        cycles * span,
        1,
        &k_of,
        &|f| layer_regs(wg::reveal(k_of(f) as u32), s8),
    );
    pan(
        g,
        "M2 Cascade (VSCTRL, SSSIZE and VSCADR a frame)",
        cycles * span,
        1,
        &k_of,
        &|f| layer_regs(wg::cascade(k_of(f) as u32), s8),
    );
    layer_off(g);
}

/// The strip's source.
fn strip_src(p: &Pics) -> Src {
    Src {
        base: p.strip,
        stride: STRIP_STRIDE,
        fmt: VideoFormat::Argb8888,
        blend: Blend::PixelAlpha,
    }
}

#[inline(never)]
fn m3(g: &mut Rig, p: &Pics) {
    label(g, p, 4, "THE TURN", "UNATTENDED: NOTHING TO DO");
    let s = strip_src(p);
    // 4-pixel steps across and back.
    let there = |f: usize| if f <= 120 { f } else { 240 - f };
    pan(
        g,
        "M3 turn, 4-pixel steps (VSCADR, stride 3840)",
        241,
        1,
        &there,
        &|f| layer_regs(wg::strip4(4 * there(f) as u32), s),
    );
    // 1-pixel steps over 64 pixels and back: the start's next 4-pixel step
    // and the window's place.
    let one = |f: usize| if f < 64 { f } else { 127 - f };
    pan(
        g,
        "M3 turn, 1-pixel steps (VSCADR, VSADR x and VSCTRL width)",
        128,
        2,
        &one,
        &|f| layer_regs(wg::strip1(one(f) as u32), s),
    );
    layer_off(g);
}

#[inline(never)]
fn m5(g: &mut Rig, p: &Pics) {
    use cmd::{BLEND_DELL_TEXT, OP_BLEND, ROP_SRCCOPY};
    label(g, p, 5, "2D PICTURES", "UNATTENDED: NOTHING TO DO");
    graphics_screen(g, S0);
    // The work pictures: the back filled with a marker (a fill that never
    // happens can't pass), the wallpaper under the window widened to 32
    // bits as the engine reads screen 0.
    draw_pic(g, p.back, (W, WIN_H), false, &|_, _| 0x5A5A_5A5A);
    draw_pic(g, p.wall, (W, WIN_H), false, &|x, y| {
        wg::widen(wg::screen0(x, y + WIN_Y, "", ""))
    });
    let report = |what: &str, (ok, write, all): (bool, u32, u32), bad: u32, of: u32| {
        log!(
            "  WG M5 {what}: {all} us (the CPU writing {write} us), done {ok}; {bad} of {of} words wrong"
        );
    };
    // (a) The back cleared to alpha 0.
    let w = rp_image::need(
        cmd::fill(
            Format::BPP32,
            p.back as usize,
            P8_STRIDE as usize,
            (0, 0, W as u16, WIN_H as u16),
            0,
        ),
        "a 2D rectangle",
    );
    let t = run2d(g, &w);
    let mut bad = 0;
    for k in 0..W * WIN_H {
        bad += u32::from(g.mem.load((p.back + 4 * k) as usize) != Some(0));
    }
    report("an alpha-0 fill 480 x 462 (32 bits)", t, bad, W * WIN_H);
    // (b) Ten rows' cards copied into place (plain copies, no blending).
    let mut words = [0u32; 10 * 10];
    let mut n = 0;
    for k in 0..10u32 {
        let y = k * ROW_PITCH + 2;
        let (c, m) = rp_image::need(
            cmd::copy(
                Format::BPP32,
                (p.back as usize, P8_STRIDE as usize),
                (p.p8 as usize, P8_STRIDE as usize),
                (8, (MARGIN + y) as u16),
                (8, y as u16, 472, (y + 41) as u16),
                0,
            ),
            "a 2D rectangle",
        );
        words[n..n + m].copy_from_slice(&c[..m]);
        n += m;
    }
    let t = run2d(g, &words[..n]);
    let bad = differ(
        g,
        (p.back, P8_STRIDE),
        (p.p8 + MARGIN * P8_STRIDE, P8_STRIDE),
        (W, 10 * ROW_PITCH),
    );
    report(
        "ten rows' cards 464 x 41 copied (32 bits)",
        t,
        bad,
        W * 10 * ROW_PITCH,
    );
    // (c) Radar cells: 24 copies of 32 x 32 (the sun's square as the cell).
    let mut words = [0u32; 24 * 10];
    let mut n = 0;
    for k in 0..24u32 {
        let (x, y) = (16 + (k % 8) * 56, 300 + (k / 8) * 48);
        let (c, m) = rp_image::need(
            cmd::copy(
                Format::BPP32,
                (p.back as usize, P8_STRIDE as usize),
                (p.p8 as usize, P8_STRIDE as usize),
                (290, (MARGIN + 6) as u16),
                (x as u16, y as u16, (x + 30) as u16, (y + 32) as u16),
                0,
            ),
            "a 2D rectangle",
        );
        words[n..n + m].copy_from_slice(&c[..m]);
        n += m;
    }
    let t = run2d(g, &words[..n]);
    let mut bad = 0;
    for k in 0..24u32 {
        let (x, y) = (16 + (k % 8) * 56, 300 + (k / 8) * 48);
        bad += differ(
            g,
            (p.back + y * P8_STRIDE + 4 * x, P8_STRIDE),
            (p.p8 + (MARGIN + 6) * P8_STRIDE + 4 * 290, P8_STRIDE),
            (30, 32),
        );
    }
    report(
        "24 radar cells 30 x 32 copied (32 bits)",
        t,
        bad,
        24 * 30 * 32,
    );
    // (d) The neighbour page for a turn: the page copied into the strip's
    // right half.
    let (c, m) = rp_image::need(
        cmd::copy(
            Format::BPP32,
            ((p.strip + W * 4) as usize, STRIP_STRIDE as usize),
            ((p.p8 + MARGIN * P8_STRIDE) as usize, P8_STRIDE as usize),
            (0, 0),
            (0, 0, W as u16, WIN_H as u16),
            0,
        ),
        "a 2D rectangle",
    );
    let t = run2d(g, &c[..m]);
    let bad = differ(
        g,
        (p.strip + W * 4, STRIP_STRIDE),
        (p.p8 + MARGIN * P8_STRIDE, P8_STRIDE),
        (W, WIN_H),
    );
    report(
        "the neighbour page 480 x 462 into the strip (32 bits)",
        t,
        bad,
        W * WIN_H,
    );
    // (e) The hand-back: screen 0 copied (16 bits), the page blended over
    // the widened wallpaper ("over", the renderer's word), converted into
    // the composed screen's window.
    let (c, m) = rp_image::need(
        cmd::copy(
            Format::RGB565,
            (p.s0c as usize, S0_STRIDE as usize),
            (S0 as usize, S0_STRIDE as usize),
            (0, 0),
            (0, 0, W as u16, H as u16),
            0,
        ),
        "a 2D rectangle",
    );
    let t1 = run2d(g, &c[..m]);
    let bad = differ(g, (p.s0c, S0_STRIDE), (S0, S0_STRIDE), (W / 2, H));
    report("screen 0 copied 480 x 640 (16 bits)", t1, bad, W / 2 * H);
    let mut words = [0u32; 2 + 10 + 10];
    words[..2].copy_from_slice(&cmd::blend(BLEND_DELL_TEXT));
    let (b, nb) = rp_image::need(
        cmd::blit(
            (Format::BPP32, p.wall as usize, P8_STRIDE as i32),
            (
                Format::BPP32,
                (p.p8 + MARGIN * P8_STRIDE) as usize,
                P8_STRIDE as i32,
            ),
            (0, 0),
            (0, 0, W as u16, WIN_H as u16),
            ROP_SRCCOPY,
            OP_BLEND,
            0,
        ),
        "a 2D rectangle",
    );
    words[2..2 + nb].copy_from_slice(&b[..nb]);
    let (v, nv) = rp_image::need(
        cmd::blit(
            (
                Format::RGB565,
                (p.s0c + WIN_Y * S0_STRIDE) as usize,
                S0_STRIDE as i32,
            ),
            (Format::BPP32, p.wall as usize, P8_STRIDE as i32),
            (0, 0),
            (0, 0, W as u16, WIN_H as u16),
            ROP_SRCCOPY,
            0,
            0,
        ),
        "a 2D rectangle",
    );
    words[2 + nb..2 + nb + nv].copy_from_slice(&v[..nv]);
    let t2 = run2d(g, &words[..2 + nb + nv]);
    // Held to the CPU's "over" (as measured) and the engine's narrowing.
    let mut bad = 0;
    for y in 0..WIN_H {
        for x in (0..W).step_by(2) {
            let got = g
                .mem
                .load((p.s0c + (WIN_Y + y) * S0_STRIDE + 2 * x) as usize);
            let px = |x: u32| {
                let s = g
                    .mem
                    .load((p.p8 + (MARGIN + y) * P8_STRIDE + 4 * x) as usize)
                    .unwrap_or(0);
                let d = wg::widen(wg::screen0(x, WIN_Y + y, "", ""));
                u32::from(wg::to565(wg::over(s, d)))
            };
            let want = px(x) | px(x + 1) << 16;
            bad += u32::from(got != Some(want));
        }
        if y.is_multiple_of(32) {
            rt::check_in();
        }
    }
    log!(
        "  WG M5 the hand-back (the page over the wallpaper, 32 bits, converted into the screen): {} us (the CPU writing {} us), done {}; {bad} of {} words differ from the CPU's \"over\"; with the copy of screen 0, {} us in all",
        t2.2,
        t2.1,
        t2.0,
        W / 2 * WIN_H,
        t1.2 + t2.2
    );
    // (f) 16 bits: a row of the ARGB4444 page copied as plain 16-bit words.
    let y = MARGIN + 2;
    let (c, m) = rp_image::need(
        cmd::copy(
            Format::RGB565,
            (p.back as usize, P4_STRIDE as usize),
            (p.p4 as usize, P4_STRIDE as usize),
            (0, y as u16),
            (0, 0, W as u16, 41),
            0,
        ),
        "a 2D rectangle",
    );
    let t = run2d(g, &c[..m]);
    let bad = differ(
        g,
        (p.back, P4_STRIDE),
        (p.p4 + y * P4_STRIDE, P4_STRIDE),
        (W / 2, 41),
    );
    report(
        "an ARGB4444 row card 480 x 41 copied (16 bits)",
        t,
        bad,
        W / 2 * 41,
    );
}

/// M4's two ways of showing the same page: the composed screen alone (X),
/// screen 0 with the page layer (Y): (`GSCADR`, `VSCADR`) of each.
fn handoff_regs(g: &mut Rig, p: &Pics) -> ([u32; 2], [u32; 2], [(usize, u32); 9]) {
    let r = layer_regs(
        wg::page_win(0),
        Src {
            base: p.p8,
            stride: P8_STRIDE,
            fmt: VideoFormat::Argb8888,
            blend: Blend::PixelAlpha,
        },
    );
    let v = r[8].1;
    (
        [gscadr(g, p.s0c), v & !(1 << 31)],
        [gscadr(g, S0), v | 1 << 31],
        r,
    )
}

/// Shows one of M4's two (`GSCADR` and `VSCADR`, the stream bit last).
fn handoff(g: &mut Rig, [gs, vs]: [u32; 2]) {
    g.e.regs.write32(dreg::GSCADR, gs);
    g.e.regs.write32(lreg::VSCADR, vs);
}

#[inline(never)]
fn m4(g: &mut Rig, p: &Pics, out: &mut [u8]) {
    label(g, p, 6, "HAND-OFFS", "UNATTENDED: NOTHING TO DO");
    graphics_screen(g, S0);
    let (x, y, r) = handoff_regs(g, p);
    layer_on(g, &r);
    // The model's mix against the 2D engine's compose (M5's).
    let _ = model(g, out);
    let (mut px, mut most) = (0u32, 0u32);
    for yy in WIN_Y..wg::SOFTKEYS_Y {
        for xx in 0..W {
            let o = ((yy * W + xx) * 3) as usize;
            let m = u32::from(out[o]) << 16 | u32::from(out[o + 1]) << 8 | u32::from(out[o + 2]);
            let word = g
                .mem
                .load((p.s0c + yy * S0_STRIDE + 2 * (xx & !1)) as usize)
                .unwrap_or(0);
            let c = wg::widen((word >> (16 * (xx & 1))) as u16);
            if wg::to565(m) != wg::to565(c) {
                px += 1;
            }
            for sh in [0, 8, 16] {
                most = most.max(((m >> sh) & 0xFF).abs_diff((c >> sh) & 0xFF));
            }
        }
        if yy.is_multiple_of(32) {
            rt::check_in();
        }
    }
    log!(
        "  WG M4 the scan-out model's mix against the 2D engine's compose: {px} of {} pixels differ at 16 bits, the most a channel {most} of 255",
        W * WIN_H
    );
    let sx = reference(g, &mut |g| handoff(g, x));
    let sy = reference(g, &mut |g| handoff(g, y));
    let live = sx != 0 && sy != 0;
    let _ = underrun_take(&mut g.e.regs);
    // Each way: [settled in the refresh due, a refresh late, never in three,
    // a refresh of neither].
    let mut tally = [[0u32; 4]; 2];
    let mut slow = 0;
    for rep in 0..100u32 {
        let (from, to, sto) = if rep % 2 == 0 { (x, y, sy) } else { (y, x, sx) };
        let _ = reference(g, &mut |g| handoff(g, from));
        let _ = at_blank(g);
        handoff(g, to);
        slow += u32::from(!in_blank(g));
        let _old = sig_prev(g);
        let seen = [sig_next(g), sig_next(g), sig_next(g)];
        let t = &mut tally[(rep % 2) as usize];
        if !live {
            // Two signatures of 0 can't tell the pictures apart.
        } else if seen.iter().any(|&s| s != sx && s != sy) {
            t[3] += 1;
        } else if seen[0] == sto {
            t[0] += 1;
        } else if seen[1] == sto {
            t[1] += 1;
        } else {
            t[2] += 1;
        }
        rt::check_in();
    }
    let under = underrun_take(&mut g.e.regs);
    log!(
        "  WG M4 the composed screen alone 0x{sx:08x}, screen 0 + the page layer 0x{sy:08x}: the same picture {}",
        if live {
            if sx == sy {
                "yes"
            } else {
                "NO (the display's mix differs from the 2D engine's)"
            }
        } else {
            "not judged (DSIG 0)"
        }
    );
    for (k, way) in ["composed -> layer", "layer -> composed"]
        .iter()
        .enumerate()
    {
        let [a, b, c, d] = tally[k];
        log!(
            "  WG M4 {way}: 50 hand-offs: {a} in the refresh due, {b} a refresh late, {c} not in three, {d} with a refresh of NEITHER{}",
            if live { "" } else { " (not judged)" }
        );
    }
    log!(
        "  WG M4 writes outside the blanking {slow}; the underrun flag {}",
        if under { "SET" } else { "clear" }
    );
    layer_off(g);
    graphics_screen(g, S0);
}

/// Shows `r` (the layer) at a blanking and holds it `frames` refreshes.
fn hold(g: &mut Rig, r: &[(usize, u32); 9], last: &mut [(usize, u32); 9], frames: u32) {
    for _ in 0..frames {
        let _ = at_blank(g);
        let _ = layer_move(g, r, last);
        let _ = g.wait_line(sig::PUBLISHED_LINE, 600);
    }
}

#[inline(never)]
fn watched(g: &mut Rig, p: &Pics) {
    label(g, p, 7, "WATCH NOW", "PRESS ANY BUTTON TO START (2 MIN)");
    graphics_screen(g, S0);
    g.arm_button();
    let t0 = rt::ticks();
    let mut pressed = false;
    while us_since(t0) < WAIT_SECS.saturating_mul(1_000_000) {
        if g.button() {
            pressed = true;
            break;
        }
        rt::check_in();
    }
    log!(
        "  WG watched block: {} after {} s",
        if pressed {
            "a button"
        } else {
            "no button: started anyway"
        },
        us_since(t0) / 1_000_000
    );
    let s8 = Src {
        base: p.p8,
        stride: P8_STRIDE,
        fmt: VideoFormat::Argb8888,
        blend: Blend::PixelAlpha,
    };
    let s4 = Src {
        base: p.p4,
        stride: P4_STRIDE,
        fmt: VideoFormat::Argb4444,
        blend: Blend::PixelAlpha,
    };
    // The two depths in turn, 2 s each, on the same place.
    let r8 = layer_regs(wg::page_win(0), s8);
    let r4 = layer_regs(wg::page_win(0), s4);
    let mut last = r8;
    layer_on(g, &r8);
    for _ in 0..if QUICK { 1 } else { 8 } {
        for (r, d) in [(&r8, "GLASS 8888 (32 BIT)"), (&r4, "GLASS 4444 + DITHER")] {
            label(g, p, 7, d, "WHICH TEXT LOOKS BETTER? SAY 8888 OR 4444");
            hold(g, r, &mut last, 120);
        }
        rt::check_in();
    }
    layer_off(g);
    // The turn dragged slowly: 4-pixel steps, then to the pixel.
    draw_pic(g, p.strip, (STRIP_W, WIN_H), false, &wg::strip);
    let s = strip_src(p);
    let step = if QUICK { 32 } else { 1 };
    for (one, d) in [
        (false, "TURN: 4-PIXEL STEPS"),
        (true, "TURN: 1-PIXEL STEPS"),
    ] {
        label(g, p, 8, d, "SLOW DRAG: DOES IT JUMP OR JITTER?");
        let r = layer_regs(wg::strip4(0), s);
        let mut last = r;
        layer_on(g, &r);
        for pos in (0..=W).step_by(step).chain((0..=W).rev().step_by(step)) {
            let w = if one {
                wg::strip1(pos)
            } else {
                wg::strip4(pos)
            };
            hold(g, &layer_regs(w, s), &mut last, 1);
            if pos % 64 == 0 {
                rt::check_in();
            }
        }
        layer_off(g);
    }
    // The hand-off at speed: every half second.
    label(g, p, 9, "HAND-OFF", "A FLASH OR A COLOUR JUMP? (10 S)");
    graphics_screen(g, S0);
    let (x, y, r) = handoff_regs(g, p);
    layer_on(g, &r);
    for k in 0..if QUICK { 2 } else { 20 } {
        let _ = at_blank(g);
        handoff(g, if k % 2 == 0 { x } else { y });
        rt::delay_us(30 * FRAME_US);
        rt::check_in();
    }
    let _ = at_blank(g);
    handoff(g, y);
    layer_off(g);
    graphics_screen(g, S0);
    // The chosen entrances and the fling at speed.
    let times = if QUICK { 1 } else { 3 };
    for (d, f) in [
        ("REVEAL", wg::reveal as fn(u32) -> Win),
        ("CASCADE", wg::cascade as fn(u32) -> Win),
    ] {
        label(g, p, 10, d, "THE PAGE ARRIVING AT SPEED");
        for _ in 0..times {
            let r = layer_regs(f(0), s8);
            let mut last = r;
            let _ = at_blank(g);
            layer_on(g, &r);
            for k in 1..=wg::ENTRANCE_FRAMES {
                hold(g, &layer_regs(f(k), s8), &mut last, 1);
            }
            rt::delay_us(1_000_000);
            layer_off(g);
            rt::delay_us(500_000);
            rt::check_in();
        }
    }
    label(g, p, 10, "FLING", "THE LIST FLUNG AND REBOUNDING");
    let r = layer_regs(wg::page_win(0), s8);
    let mut last = r;
    layer_on(g, &r);
    for _ in 0..times {
        for f in 0..wg::FLING_FRAMES {
            hold(g, &layer_regs(wg::page_win(wg::fling(f)), s8), &mut last, 1);
        }
        rt::check_in();
    }
    layer_off(g);
}

/// M7's settings: (name, written as).
const M7: [&str; 3] = [
    "MBXCLK clocks off (bits 1-0 = 0, the divider kept)",
    "MBXCLK 0 (Dell's off path)",
    "ODFBPWR 1 (the on-die buffer active low power)",
];

#[inline(never)]
fn m7(g: &mut Rig, p: &Pics) {
    let s8 = Src {
        base: p.p8,
        stride: P8_STRIDE,
        fmt: VideoFormat::Argb8888,
        blend: Blend::PixelAlpha,
    };
    let r = layer_regs(wg::page_win(0), s8);
    let clk0 = g.e.regs.read32(MBXCLK);
    let odfb0 = g.e.regs.read32(ODFBPWR);
    log!(
        "  WG M7 (R133, last): the page showing; MBXCLK 0x{clk0:x}, ODFBPWR 0x{odfb0:x} ODFBSTAT 0x{:x} as found; the 3D clock is off at Windows Mobile's 0x5, so the 2D clock is what gates",
        g.e.regs.read32(ODFBSTAT)
    );
    let hold_frames = if QUICK { 30 } else { 300 };
    for (k, name) in M7.iter().enumerate() {
        let step = 11 + k as u32;
        let mut l2 = heapless_name();
        let _ = write!(l2, "DOES THE SCREEN HOLD? ({} S)", hold_frames / 60);
        label(
            g,
            p,
            step,
            ["2D CLOCK OFF", "MBXCLK 0", "ODFB LOW POWER"][k],
            l2.as_str(),
        );
        graphics_screen(g, S0);
        layer_on(g, &r);
        let want = reference(g, &mut |_| {});
        let _ = underrun_take(&mut g.e.regs);
        // The marker (ADR-0035 rule 5): in the kept log before the write,
        // so a hang leaves this line last.
        log!(
            "  WG M7 step {} of 3, {name}: about to write (a hang from here is this step's)",
            k + 1
        );
        let t0 = rt::ticks();
        match k {
            0 => g.e.regs.write32(MBXCLK, clk0 & !3),
            1 => g.e.regs.write32(MBXCLK, 0),
            _ => g.e.regs.write32(ODFBPWR, (odfb0 & !3) | 1),
        }
        let stat = g.e.regs.read32(ODFBSTAT);
        let (mut same, mut zero, mut other) = (0u32, 0u32, 0u32);
        for _ in 0..hold_frames {
            let s = sig_prev(g);
            if s == want {
                same += 1;
            } else if s == 0 {
                zero += 1;
            } else {
                other += 1;
            }
            let _ = at_blank(g);
        }
        let under = underrun_take(&mut g.e.regs);
        let held = us_since(t0) / 1000;
        match k {
            0 | 1 => g.e.regs.write32(MBXCLK, clk0),
            _ => g.e.regs.write32(ODFBPWR, odfb0),
        }
        // The 2D clock's wake: a 64 x 64 fill submitted and finished.
        let w = rp_image::need(
            cmd::fill(
                Format::BPP32,
                p.back as usize,
                P8_STRIDE as usize,
                (0, 0, 64, 64),
                0,
            ),
            "a 2D rectangle",
        );
        let (ok, _, wake) = run2d(g, &w);
        log!(
            "  WG M7 step {} of 3, {name}: held {held} ms, ODFBSTAT 0x{stat:x} meanwhile; {hold_frames} refreshes: {same} the page as before, {zero} DSIG 0, {other} other{}; the underrun flag {}; put back, the first 2D fill after it {wake} us (done {ok}); MBXCLK 0x{:x} ODFBPWR 0x{:x} ODFBSTAT 0x{:x}",
            k + 1,
            if want == 0 {
                " (not judged: DSIG 0)"
            } else {
                ""
            },
            if under { "SET" } else { "clear" },
            g.e.regs.read32(MBXCLK),
            g.e.regs.read32(ODFBPWR),
            g.e.regs.read32(ODFBSTAT)
        );
        rt::check_in();
    }
    layer_off(g);
}

/// Run WG-R (R134): Reveal's 8 grow-and-shrink cycles written ways A-G,
/// then way H (V3's ARGB4444: Reveal and the 1-pixel turn), each refresh
/// judged and the first torn ones placed.
#[inline(never)]
fn r134(g: &mut Rig) {
    log!(
        "  WG-R (R134, sheet r134-reveal-tears.md): Reveal written ways A-G (the order, the line, y alone, the height alone, DSCTRL bit 24 with three releases), then H at ARGB4444; DSCTRL 0x{:x} as found",
        g.e.regs.read32(dreg::DSCTRL)
    );
    let strip16 = P4_STRIDE * 2 * WIN_H;
    let sizes = [P8_STRIDE * PIC_H, P4_STRIDE * PIC_H, strip16];
    let mut at = [0u32; 3];
    for k in 0..3 {
        match g.pool.alloc(sizes[k], 0x1000) {
            Some(a) => at[k] = a,
            None => {
                log!("  WG-R: no pool block of {} bytes: nothing run", sizes[k]);
                for j in 0..k {
                    let _ = g.pool.free(at[j], sizes[j]);
                }
                return;
            }
        }
    }
    let p = Pics {
        p8: at[0],
        p4: at[1],
        strip: at[2],
        back: BACK,
        wall: WALL,
        s0c: 0,
    };
    draw_screen(
        g,
        S0,
        (0, H),
        "STEP 1 OF 11: DRAWING",
        "UNATTENDED: NOTHING TO DO",
    );
    graphics_screen(g, S0);
    layer_off(g);
    draw_pic(g, p.p8, (W, PIC_H), false, &|x, y| wg::page(x, y, false));
    draw_pic(g, p.p4, (W, PIC_H), true, &|x, y| wg::page(x, y, false));
    draw_pic(g, p.strip, (STRIP_W, WIN_H), true, &wg::strip);
    let s8 = Src {
        base: p.p8,
        stride: P8_STRIDE,
        fmt: VideoFormat::Argb8888,
        blend: Blend::PixelAlpha,
    };
    let s4 = Src {
        base: p.p4,
        stride: P4_STRIDE,
        fmt: VideoFormat::Argb4444,
        blend: Blend::PixelAlpha,
    };
    let span = 2 * (wg::ENTRANCE_FRAMES as usize + 1);
    let k_of = move |f: usize| {
        let c = f % span;
        if c <= wg::ENTRANCE_FRAMES as usize {
            c
        } else {
            span - 1 - c
        }
    };
    let n = if QUICK { 2 } else { 8 } * span;
    let reveal = move |f: usize| wg::reveal(k_of(f) as u32);
    // E: the top alone moves (the size and the start fixed).
    let y_alone = move |f: usize| Win {
        x: 0,
        y: reveal(f).y,
        w: W,
        h: wg::SOFTKEYS_Y - 210,
        row: MARGIN,
        col: 0,
    };
    // F: the height alone, at Reveal's steps (the top fixed).
    let h_alone = move |f: usize| Win {
        h: reveal(f).h,
        ..wg::page_win(0)
    };
    let ways: [(&str, Way, &dyn Fn(usize) -> Win); 9] = [
        ("A REVEAL AS RUN WG", Way::Blank, &reveal),
        ("B REVEAL, VSADR LAST", Way::VsadrLast, &reveal),
        ("C REVEAL, VSADR FIRST", Way::VsadrFirst, &reveal),
        ("D REVEAL, LINES 640-642", Way::Early, &reveal),
        ("E THE TOP ALONE MOVING", Way::Blank, &y_alone),
        ("F THE HEIGHT ALONE MOVING", Way::Blank, &h_alone),
        ("G1 HELD, BIT 24 CLEARED", Way::Hold(0), &reveal),
        ("G2 HELD, DUCTRL 1 TWICE", Way::Hold(1), &reveal),
        ("G3 HELD, DUCTRL 0 THEN 1", Way::Hold(2), &reveal),
    ];
    for (k, &(name, way, win)) in ways.iter().enumerate() {
        label_of(g, &p, (k as u32 + 2, R134_STEPS), &name[..2], name);
        pan_with(g, name, (n, 1), &k_of, &|f| layer_regs(win(f), s8), way);
        layer_off(g);
        rt::check_in();
    }
    // H: V3's ARGB4444: Reveal on 8-pixel steps, the turn to the pixel.
    label_of(
        g,
        &p,
        (11, R134_STEPS),
        "H 4444",
        "REVEAL AND THE TURN AT ARGB4444",
    );
    pan_with(
        g,
        "H REVEAL AT ARGB4444",
        (n, 1),
        &k_of,
        &|f| layer_regs(wg::reveal_on(k_of(f) as u32, 8), s4),
        Way::Blank,
    );
    let st = Src {
        base: p.strip,
        stride: P4_STRIDE * 2,
        fmt: VideoFormat::Argb4444,
        blend: Blend::PixelAlpha,
    };
    let one = |f: usize| if f < 64 { f } else { 127 - f };
    pan_with(
        g,
        "H TURN AT ARGB4444, 1-PIXEL STEPS (8-PIXEL STARTS, THE WINDOW 0-7)",
        (128, 2),
        &one,
        &|f| layer_regs(wg::strip1_on(one(f) as u32, 8), st),
        Way::Blank,
    );
    layer_off(g);
    graphics_screen(g, S0);
    for k in 0..3 {
        let _ = g.pool.free(at[k], sizes[k]);
    }
}
