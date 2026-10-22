//! Test image (ADR-0025 S0-S2): the 2700G's MVED1 video decoder, under the
//! loader. Version 1.
//!
//! Everything it knows is static (docs/hw/2700g.md, "The MVED1 video
//! decoder"); each stage checks one finding, logs its evidence and stops
//! the run at the first surprise:
//!
//! - **S0, clock and identity:** `M24CLK` read (WM idles at `0x2`) and set
//!   on (`0x3`); only then the three registers Dell's library reads:
//!   capabilities `0xA0`, FIFO levels `0x50`, status `0x80`;
//! - **S1, reset:** `0x00` = 1, `0x00` = 0, `0x88` = 3; `0x50` and `0x80`
//!   again (expected: empty, 0);
//! - **S2, conversion (registers only):** a 64 x 64 planar 4:2:0 test frame
//!   (stride 256, the two chroma components side by side) at 4 MiB in
//!   2700G memory, far from the screen, converted to YUY2 (`0x30`-`0x3C`,
//!   `0x48` = 1); `0x80` bit 0 polled for 100 ms and timed; acknowledged
//!   with `0x88` = 1; the result compared with the host's YUY2 for both
//!   chroma orders, and 4 KiB guards either side checked (the unit is a
//!   bus master: did it write only where told?). Then mode 3 (3 bytes a
//!   pixel), compared with BT.601 RGB and BGR.
//!
//! The screen then shows the host's picture, the chip's YUY2 and its
//! mode-3 output (3x), and a square per stage (green passed, amber ran
//! with a surprise, red failed, grey not run), until a button.
//!
//! Safety: only `0x50`, `0x80` and `0xA0` are ever read (a wrong read may
//! hang the bus; [`rp_drv_mved::may_read`] is checked before each read);
//! the command windows are never touched; every wait has a timeout; on
//! the way out the unit is reset and `M24CLK` put back as found, as are
//! the display registers changed.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed with the verdict.
pub const MARKER: &str = "RETROPANE-MVED";

/// How a stage ended, for the log and its square on the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Not reached (an earlier stage stopped the run).
    NotRun,
    /// Everything as the survey predicts.
    Passed,
    /// Ran to the end, but something differed from the prediction (still
    /// a finding: the run goes on only where it is safe).
    Surprise,
    /// Didn't finish (a timeout, a refused job, a guard hit).
    Failed,
}

impl Outcome {
    /// The square's colour (ARGB).
    #[must_use]
    pub const fn colour(self) -> u32 {
        match self {
            Self::NotRun => 0xFF60_6060,
            Self::Passed => 0xFF20_C040,
            Self::Surprise => 0xFFE0_A000,
            Self::Failed => 0xFFD0_2020,
        }
    }

    /// For the log.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::NotRun => "not run",
            Self::Passed => "passed",
            Self::Surprise => "surprise",
            Self::Failed => "FAILED",
        }
    }

    /// Whether the next stage may run after this one.
    #[must_use]
    pub const fn go_on(self) -> bool {
        matches!(self, Self::Passed)
    }
}

/// S0's or S1's verdict from the FIFO and status reads: both idle passes;
/// anything else is a surprise (the reset may clear it).
#[must_use]
pub fn idle(fifo: u32, status: u32) -> Outcome {
    let f = rp_drv_mved::Fifo::from_reg(fifo);
    if f.empty() && status & (rp_drv_mved::CONVERT_DONE | rp_drv_mved::FRAME_DONE) == 0 {
        Outcome::Passed
    } else {
        Outcome::Surprise
    }
}

/// A conversion's verdict: it must finish and stay inside its guards;
/// then a match with one hypothesis passes, none is a surprise.
#[must_use]
pub fn conversion(done: bool, guards_intact: bool, matched: bool) -> Outcome {
    match (done, guards_intact, matched) {
        (true, true, true) => Outcome::Passed,
        (true, true, false) => Outcome::Surprise,
        _ => Outcome::Failed,
    }
}

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_blit2d::{self as cmd, Engine, Format};
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, layers::Layer, reg as dreg};
    use rp_drv_mved::{
        self as mv, CONVERT_DONE, ChromaOrder, Convert, Diff, Frame420, M24CLK, MVED,
        RESET_SEQUENCE, Rgb3, reg,
    };
    use rp_drv_pxa_keypad::{Keypad, Pressed};
    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_api::RegisterBus;
    use rp_hal_mem::{load32_volatile, store32_volatile};
    use rp_hal_pxa27x::{Mmio, cpu, rt};
    use rp_hal_x51v::devices::RunLog;

    use super::{MARKER, Outcome, conversion, idle};

    use rp_image::crash;

    rp_image::entry!(main);

    macro_rules! log {
        ($($t:tt)*) => {{ let _ = writeln!(RunLog, $($t)*); }};
    }

    /// This image's version, in its first log line.
    const VERSION: u32 = 1;

    /// Microseconds since `t0` (OSCR0 at 3.25 MHz).
    fn us_since(t0: u32) -> u32 {
        rt::ticks().wrapping_sub(t0) / 13 * 4
    }

    // 2700G local memory: the 32 bpp screen (2 MiB in, clear of the
    // loader's plane below 1 MiB); the test frame at 4 MiB and the two
    // destinations after it, each with a 4 KiB guard either side; the 2D
    // engine's fence.
    const TARGET: usize = 0x0020_0000;
    const W: u32 = 480;
    const H: u32 = 640;
    const STRIDE: usize = 480 * 4;
    const SRC: usize = 0x0040_0000;
    const DST_YUY2: usize = 0x0041_0000;
    const DST_RGB3: usize = 0x0042_0000;
    const GUARD: usize = 4096;
    const GUARD_WORD: u32 = 0xA5A5_A5A5;
    const FENCE: usize = 0x00FF_F000;
    /// The conversion's timeout (ADR-0025).
    const LIMIT_US: u32 = 100_000;
    /// Tolerance for the 3-byte format against BT.601 (the chip's exact
    /// arithmetic is unknown); YUY2 is a copy and must be exact.
    const RGB_TOLERANCE: u8 = 3;

    const GREY: u32 = 0xFF20_2020;
    const DARK: u32 = 0xFF40_4040;

    /// Reads a decoder register, only one of the three Dell's library
    /// reads: anything else stops the image before the read happens.
    fn read(regs: &mut Mmio, off: usize) -> u32 {
        if !mv::may_read(off) {
            crash(format_args!(
                "refused to read MVED 0x{off:02x} (bus-hang rule)"
            ))
        }
        regs.read32(MVED + off)
    }

    fn write(regs: &mut Mmio, off: usize, v: u32) {
        regs.write32(MVED + off, v);
    }

    fn byte(vram: &[u8], at: usize) -> u8 {
        let w = load32_volatile(vram, at & !3).unwrap_or(0);
        (w >> (8 * (at & 3))) as u8
    }

    fn put(vram: &mut [u8], x: u32, y: u32, c: u32) {
        let _ = store32_volatile(vram, TARGET + y as usize * STRIDE + 4 * x as usize, c);
    }

    /// Fills `[dst - GUARD, dst + bytes + GUARD)` with the guard word.
    fn arm_guards(vram: &mut [u8], dst: usize, bytes: usize) {
        for at in (dst - GUARD..dst + bytes + GUARD).step_by(4) {
            let _ = store32_volatile(vram, at, GUARD_WORD);
        }
        cpu::drain_write_buffer();
    }

    /// Whether both guards are untouched; how many destination words
    /// still hold the guard word (all of them: the chip wrote nothing).
    fn guards(vram: &[u8], dst: usize, bytes: usize) -> (bool, usize) {
        let intact = (dst - GUARD..dst)
            .chain(dst + bytes..dst + bytes + GUARD)
            .step_by(4)
            .all(|at| load32_volatile(vram, at) == Some(GUARD_WORD));
        let untouched = (dst..dst + bytes)
            .step_by(4)
            .filter(|&at| load32_volatile(vram, at) == Some(GUARD_WORD))
            .count();
        (intact, untouched)
    }

    /// Logs `n` bytes of row `y` of a destination.
    fn log_row(vram: &[u8], dst: usize, stride: u32, y: u32, n: usize) {
        let _ = write!(RunLog, "    row {y:2}:");
        for k in 0..n {
            let _ = write!(
                RunLog,
                " {:02x}",
                byte(vram, dst + (y * stride) as usize + k)
            );
        }
        log!();
    }

    /// Logs a comparison with one hypothesis.
    fn log_diff(name: &str, d: &Diff) {
        let _ = write!(
            RunLog,
            "    vs {name}: {} of {} bytes off, max diff {}",
            d.wrong, d.bytes, d.max
        );
        match d.first {
            Some((xb, y, got, want)) => {
                log!(", first byte {xb} of row {y}: 0x{got:02x}, wanted 0x{want:02x}")
            }
            None => log!(),
        }
    }

    /// Starts a job, polls `0x80` bit 0 for [`LIMIT_US`], acknowledges.
    /// Whether it finished.
    fn run_job(regs: &mut Mmio, job: &[(usize, u32)]) -> bool {
        let before = read(regs, reg::STATUS);
        let _ = write!(RunLog, "    status 0x{before:08x}; job");
        for &(off, v) in job {
            let _ = write!(RunLog, " 0x{off:02x}=0x{v:x}");
        }
        log!();
        let t0 = rt::ticks();
        for &(off, v) in job {
            write(regs, off, v);
        }
        let mut done = false;
        let mut last = before;
        let mut changes = 0;
        while us_since(t0) < LIMIT_US {
            let s = read(regs, reg::STATUS);
            if s != last && changes < 8 {
                changes += 1;
                log!("    {} us: status 0x{s:08x}", us_since(t0));
                last = s;
            }
            if s & CONVERT_DONE != 0 {
                done = true;
                break;
            }
        }
        let fifo = read(regs, reg::FIFO);
        write(regs, reg::ACK, CONVERT_DONE);
        log!(
            "    {} after {} us; FIFO 0x{fifo:08x}; after the ack 0x{:08x}",
            if done { "done" } else { "NOT DONE" },
            us_since(t0),
            read(regs, reg::STATUS)
        );
        done
    }

    /// Runs one conversion with the destination guarded, logs three rows;
    /// whether it finished and whether the guards held.
    fn run_conversion(
        regs: &mut Mmio,
        vram: &mut [u8],
        job: &Convert,
        words: &[(usize, u32)],
        bpp: u32,
    ) -> (bool, bool) {
        let bytes = job.dst_bytes(bpp);
        arm_guards(vram, job.dst, bytes);
        let done = run_job(regs, words);
        let (intact, untouched) = guards(vram, job.dst, bytes);
        log!(
            "    guards intact: {intact}; destination words unwritten: {untouched} of {}",
            bytes / 4
        );
        for y in [0, 1, 63] {
            log_row(vram, job.dst, job.dst_stride, y, 12);
        }
        (done, intact)
    }

    /// The YUY2 conversion; its verdict and the chroma order it matched.
    fn s2_yuy2(regs: &mut Mmio, vram: &mut [u8], job: &Convert) -> (Outcome, Option<ChromaOrder>) {
        log!(
            "S2: 64 x 64 4:2:0 at 0x{:x} to YUY2 at 0x{:x}",
            job.src,
            job.dst
        );
        let words = match mv::convert_yuy2(job, vram.len()) {
            Ok(w) => w,
            Err(e) => {
                log!("    the job was refused: {}", e.name());
                return (Outcome::Failed, None);
            }
        };
        let (done, intact) = run_conversion(regs, vram, job, &words, 2);
        let mut d = [Diff::default(); 2];
        for y in 0..job.frame.height {
            for xb in 0..job.frame.width * 2 {
                let got = byte(vram, job.dst + (y * job.dst_stride + xb) as usize);
                for (k, o) in ChromaOrder::ALL.iter().enumerate() {
                    d[k].add(xb, y, got, mv::yuy2_byte(*o, xb, y), 0);
                }
            }
        }
        let mut order = None;
        for (k, o) in ChromaOrder::ALL.iter().enumerate() {
            log_diff(o.name(), &d[k]);
            if d[k].matches() {
                order = Some(*o);
            }
        }
        (conversion(done, intact, order.is_some()), order)
    }

    /// The mode-3 conversion; its verdict and the closest hypothesis.
    fn s2_rgb3(regs: &mut Mmio, vram: &mut [u8], job: &Convert) -> (Outcome, Rgb3) {
        log!(
            "S2 mode 3: to 3 bytes a pixel at 0x{:x}, BT.601 within {RGB_TOLERANCE}",
            job.dst
        );
        let words = match mv::convert_rgb3(job, vram.len()) {
            Ok(w) => w,
            Err(e) => {
                log!("    the job was refused: {}", e.name());
                return (Outcome::Failed, Rgb3::ALL[0]);
            }
        };
        let (done, intact) = run_conversion(regs, vram, job, &words, 3);
        let mut d = [Diff::default(); 4];
        for y in 0..job.frame.height {
            for xb in 0..job.frame.width * 3 {
                let got = byte(vram, job.dst + (y * job.dst_stride + xb) as usize);
                for (k, h) in Rgb3::ALL.iter().enumerate() {
                    d[k].add(xb, y, got, mv::rgb3_byte(*h, xb, y), RGB_TOLERANCE);
                }
            }
        }
        let mut best = 0;
        for (k, h) in Rgb3::ALL.iter().enumerate() {
            log_diff(h.name(), &d[k]);
            if d[k].wrong < d[best].wrong {
                best = k;
            }
        }
        (conversion(done, intact, d[best].matches()), Rgb3::ALL[best])
    }

    /// A 64 x 64 picture at 3x, each colour from `f` (which may read the
    /// 2700G memory: no copy on the stack).
    fn panel(vram: &mut [u8], x0: u32, y0: u32, f: impl Fn(&[u8], u32, u32) -> u32) {
        for y in 0..64 {
            for x in 0..64 {
                let c = f(vram, x, y);
                for dy in 0..3 {
                    for dx in 0..3 {
                        put(vram, x0 + 3 * x + dx, y0 + 3 * y + dy, c);
                    }
                }
            }
        }
    }

    fn argb([r, g, b]: [u8; 3]) -> u32 {
        0xFF00_0000 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
    }

    /// The chip's YUY2 pixel, read as U first.
    fn yuy2_px(vram: &[u8], x: u32, y: u32) -> u32 {
        let at = |k: u32| byte(vram, DST_YUY2 + (y * 128 + (x & !1) * 2 + k) as usize);
        argb(mv::rgb601(at(2 * (x & 1)), at(1), at(3)))
    }

    /// The results for the owner's photo: the host's picture (top left),
    /// the chip's YUY2 read as U first (top right), its mode-3 output in
    /// the closest hypothesis' byte order (bottom left), the four stages'
    /// squares (bottom right).
    fn draw(
        e: &mut Engine<Mmio>,
        vram: &mut [u8],
        stages: &[Outcome; 4],
        yuy2: bool,
        rgb3: Option<Rgb3>,
    ) {
        let w = rp_image::need(
            cmd::fill(
                Format::BPP32,
                TARGET,
                STRIDE,
                (0, 0, W as u16, H as u16),
                GREY,
            ),
            "a 2D rectangle",
        );
        if e.submit(&w).and_then(|()| e.finish(vram, 200_000)).is_err() {
            log!("the 2D clear didn't finish: the screen may show old pixels");
        }
        panel(vram, 36, 60, |_, x, y| {
            let (u, v) = ChromaOrder::UFirst.uv(x / 2);
            argb(mv::rgb601(mv::luma(x, y), u, v))
        });
        if yuy2 {
            panel(vram, 252, 60, yuy2_px);
        } else {
            panel(vram, 252, 60, |_, _, _| DARK);
        }
        match rgb3 {
            Some(h) => panel(vram, 36, 300, |m, x, y| {
                let at = DST_RGB3 + (y * 192 + x * 3) as usize;
                let b = [byte(m, at), byte(m, at + 1), byte(m, at + 2)];
                argb(if h.bgr { [b[2], b[1], b[0]] } else { b })
            }),
            None => panel(vram, 36, 300, |_, _, _| DARK),
        }
        for (k, s) in stages.iter().enumerate() {
            let x0 = 252 + 52 * k as u32;
            for y in 300..344 {
                for x in x0..x0 + 44 {
                    put(vram, x, y, s.colour());
                }
            }
        }
        cpu::drain_write_buffer();
    }

    fn main() -> ! {
        rt::init_mmu();
        let Some((regs, vram)) = rp_image::take_mbx() else {
            crash(format_args!("no 2700G"))
        };
        let mut display = Display::new(regs);
        let _ = display.adopt(vram.len());
        let regs = display.into_inner();
        let mut e = Engine::new(regs, FENCE, rt::ticks, 3_250);
        let _ = store32_volatile(vram, FENCE, 0);
        cpu::drain_write_buffer();
        log!("{MARKER}: the MVED1 video decoder, S0-S2 (ADR-0025, version {VERSION})");
        let mut stages = [Outcome::NotRun; 4];

        // S0: the clock on, then the three readable registers.
        rt::check_in();
        let clk0 = e.regs.read32(M24CLK);
        let clk = mv::m24clk_on(clk0);
        e.regs.write32(M24CLK, clk);
        rt::delay_us(1_000);
        let caps = read(&mut e.regs, reg::CAPS);
        let fifo = read(&mut e.regs, reg::FIFO);
        let status = read(&mut e.regs, reg::STATUS);
        stages[0] = idle(fifo, status);
        log!(
            "S0: M24CLK 0x{clk0:x} (WM idles at 0x2){}, set to 0x{clk:x} (reads 0x{:x}); caps 0xA0 = 0x{caps:08x}; FIFO 0x50 = 0x{fifo:08x}; status 0x80 = 0x{status:08x}: {}",
            if clk0 == 0x2 {
                ""
            } else {
                " - not as expected"
            },
            e.regs.read32(M24CLK),
            stages[0].name()
        );

        // S1: the reset; S0's surprise, if any, may clear here.
        rt::check_in();
        for (off, v) in RESET_SEQUENCE {
            write(&mut e.regs, off, v);
            rt::delay_us(10);
        }
        let fifo = read(&mut e.regs, reg::FIFO);
        let status = read(&mut e.regs, reg::STATUS);
        stages[1] = idle(fifo, status);
        log!(
            "S1: reset; FIFO 0x{fifo:08x}; status 0x{status:08x}: {}",
            stages[1].name()
        );

        // S2: the conversions, if the unit is idle after its reset.
        let mut rgb3 = None;
        if stages[1].go_on() {
            rt::check_in();
            let frame = Frame420::new(64, 64).unwrap_or(Frame420 {
                width: 64,
                height: 64,
                stride: 256,
            });
            for off in (0..frame.bytes()).step_by(4) {
                let _ = store32_volatile(vram, SRC + off, mv::test_frame_word(&frame, off));
            }
            cpu::drain_write_buffer();
            let job = Convert {
                frame,
                src: SRC,
                dst: DST_YUY2,
                dst_stride: 128,
            };
            let order;
            (stages[2], order) = s2_yuy2(&mut e.regs, vram, &job);
            log!(
                "S2 YUY2: {} ({})",
                stages[2].name(),
                order.map_or("no chroma order matched", |o| o.name())
            );
            // Mode 3 only after a conversion that finished inside its guards.
            if stages[2] != Outcome::Failed {
                rt::check_in();
                let job = Convert {
                    dst: DST_RGB3,
                    dst_stride: 192,
                    ..job
                };
                let (s, h) = s2_rgb3(&mut e.regs, vram, &job);
                stages[3] = s;
                rgb3 = (s != Outcome::Failed).then_some(h);
                log!("S2 mode 3: {}", s.name());
            }
        } else {
            log!("S1 didn't leave the unit idle: no conversion tried");
        }

        draw(
            &mut e,
            vram,
            &stages,
            stages[2] != Outcome::Failed && stages[2] != Outcome::NotRun,
            rgb3,
        );
        let saved = show_target(&mut e.regs);
        let keypad = arm_keypad();
        let verdict = match (stages[2], stages[3]) {
            (Outcome::NotRun, _) => "not idle after the reset (see S0/S1)",
            (Outcome::Failed, _) => "the YUY2 conversion didn't finish cleanly",
            (_, Outcome::Failed) => "YUY2 converted, mode 3 didn't finish cleanly",
            (Outcome::Passed, Outcome::Passed) => "both conversions match a host reference",
            _ => "both conversions ran, with a surprise (see the log)",
        };
        log!(
            "{MARKER}: S0 {}, S1 {}, S2 YUY2 {}, mode 3 {}: {verdict} - shown until a button",
            stages[0].name(),
            stages[1].name(),
            stages[2].name(),
            stages[3].name()
        );
        if let Some(bus) = rt::take_ffuart() {
            let mut uart = PxaUart::new(bus);
            uart.init(115_200);
            let _ = writeln!(uart, "{MARKER}: {verdict}");
        }
        hold_until_button(keypad);
        for (off, v) in saved {
            e.regs.write32(off, v);
        }
        for (off, v) in RESET_SEQUENCE {
            write(&mut e.regs, off, v);
            rt::delay_us(10);
        }
        e.regs.write32(M24CLK, clk0);
        log!("put back: the unit reset, M24CLK 0x{clk0:x}");
        rp_image::finish()
    }

    /// Points the graphics plane at the 32 bpp target (opaque); returns the
    /// registers as found.
    fn show_target(regs: &mut Mmio) -> [(usize, u32); 5] {
        let plane = Plane {
            format: PixelFormat::Argb8888,
            width: W as u16,
            height: H as u16,
            offset: TARGET,
            stride: STRIDE,
        };
        let new = plane.registers();
        let saved = new.map(|(off, _)| (off, regs.read32(off)));
        let gamma = regs.read32(dreg::GSCTRL) & (1 << 26);
        for (off, v) in new {
            let v = match off {
                dreg::GSCTRL => v | gamma,
                dreg::GSCADR => Layer::SOLO.scadr(v),
                dreg::GBBASE => Layer::SOLO.bbase(v),
                _ => v,
            };
            regs.write32(off, v);
        }
        saved
    }

    /// The keypad, enabled with earlier activity discarded, before the
    /// verdict goes out (a press right after it must count).
    fn arm_keypad() -> Option<Keypad<Mmio>> {
        let kp = rt::take_keypad()?;
        let mut keypad = Keypad::new(kp);
        keypad.enable();
        keypad.acknowledge();
        Some(keypad)
    }

    /// Holds until a button is pressed and released (5 minutes at most),
    /// checking in with the loader.
    fn hold_until_button(keypad: Option<Keypad<Mmio>>) {
        let Some(mut keypad) = keypad else {
            return;
        };
        let t0 = rt::ticks();
        let mut seen = false;
        while us_since(t0) < 300_000_000 {
            rt::check_in();
            seen |= keypad.acknowledge();
            if seen && keypad.pressed() == Pressed::NONE {
                return;
            }
            rt::delay_us(20_000);
        }
        log!("no button in 5 minutes: returning");
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}

#[cfg(test)]
mod tests {
    use super::{Outcome, conversion, idle};

    #[test]
    fn idle_needs_empty_fifos_and_no_status() {
        assert_eq!(idle(0, 0), Outcome::Passed);
        assert_eq!(idle(0x3F, 0x4), Outcome::Passed, "outside the fields");
        assert_eq!(idle(1 << 6, 0), Outcome::Surprise);
        assert_eq!(idle(0, 1), Outcome::Surprise);
        assert_eq!(idle(0, 2), Outcome::Surprise);
    }

    #[test]
    fn a_conversion_must_finish_inside_its_guards() {
        assert_eq!(conversion(true, true, true), Outcome::Passed);
        assert_eq!(conversion(true, true, false), Outcome::Surprise);
        assert_eq!(
            conversion(true, false, true),
            Outcome::Failed,
            "wrote outside"
        );
        assert_eq!(conversion(false, true, false), Outcome::Failed, "timed out");
    }

    #[test]
    fn only_a_pass_goes_on() {
        assert!(Outcome::Passed.go_on());
        for o in [Outcome::NotRun, Outcome::Surprise, Outcome::Failed] {
            assert!(!o.go_on());
        }
    }
}
