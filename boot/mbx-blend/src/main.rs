//! Test image (renderer step 1, ADR-0022): what the MBX 2D engine's blend
//! state does, and what the display's interrupt events mean, before
//! `Mbx2dRenderer` is designed around either.
//!
//! **Events first** (no engine work; register names from the Linux mbxfb
//! header: `DINTRS` bits 0/1 HBLNK0/1, 2/3 VEVENT0/1, 4-6 underruns, 16-18
//! overruns; `DVECTRL` (`0x2310`) holds the VEVENT line, 643 here):
//!
//! - E1: the registers as found, and the frame (lines, period) from `DVLNUM`;
//! - E2: how a status bit clears: writing 0 to it, or writing 1;
//! - E3: when each of bits 0-3 sets again after a clear, at 16 lines spread
//!   over the frame: the scan line it sets on and the delay;
//! - E4: VEVENT moved to lines 100 and 400: do bits 2/3 follow, and does
//!   GPIO85 (the 2700G's interrupt line) edge at the chosen line? (A flip
//!   interrupt at any line, no polling.)
//! - E5: `DINTRCNT` (`0x2180`, never touched by Dell) set to 1 and 4: does
//!   it change how often the events set?
//!
//! **Blending** (Dell's anti-aliased text sends blend state `0x20000004,
//! 0x09100000`, a 4 bpp coverage source and header `0x8002CCCC`: bit 17 is
//! the new bit):
//!
//! - B1: Dell's text blit reproduced: 16 coverage levels over blue in red,
//!   with and without bit 17 and the blend word, against the CPU's lerp;
//! - B2: the blend word swept: each of its top three nibbles 0-15, and each
//!   low bit, 4 coverage levels read back per word;
//! - B3: per-pixel alpha sources: format codes 4, 7-9 and 12-15 as the
//!   source (ARGB4444-, ARGB8888-shaped data), blended or not;
//! - B4: translucent solid fills (a fill with bit 17 and alpha in the colour);
//! - B5: does heavy 2D work starve scan-out (`DINTRS` underrun bits)?
//!
//! Every wait has a timeout. A stuck engine is reset and the probe goes on
//! (logged); three in a row end the blend stages. Every state touched
//! (`DVECTRL`, `DINTRCNT`, `DINTRE`, `MINTENABLE`, the blend word, the key)
//! is put back: the 2700G isn't reset between images. The picture left up
//! until a button: a GPU-blended ramp above the CPU's expected ramp.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed with the verdict.
pub const MARKER: &str = "RETROPANE-MBX-BLEND";

/// Where the fence word lives: the last 4 KiB of the 2700G's 16 MiB.
pub const FENCE: usize = 0x00FF_F000;

/// The coverage test row: 16 pixels of 4 bpp coverage 0..15, the lower
/// nibble of each byte first (Dell's origin puts `x & 1` in the origin and
/// `x / 2` in the address, so a byte holds two pixels; which comes first is
/// read off the result).
#[must_use]
pub fn coverage_row() -> [u8; 8] {
    core::array::from_fn(|k| (2 * k as u8) | (2 * k as u8 + 1) << 4)
}

/// RGB565 split into 5/6/5-bit channels.
#[must_use]
pub fn channels(p: u16) -> [u32; 3] {
    let p = u32::from(p);
    [p >> 11, (p >> 5) & 0x3F, p & 0x1F]
}

/// `d` moved toward `c` by `a / max`, per channel, rounded: the blend Dell's
/// text expects, as the CPU computes it.
#[must_use]
pub fn lerp565(d: u16, c: u16, a: u32, max: u32) -> u16 {
    let (d, c) = (channels(d), channels(c));
    let ch = |k: usize| (d[k] * (max - a) + c[k] * a + max / 2) / max;
    (ch(0) << 11 | ch(1) << 5 | ch(2)) as u16
}

/// Whether two RGB565 pixels are within one step per channel.
#[must_use]
pub fn near(a: u16, b: u16) -> bool {
    let (a, b) = (channels(a), channels(b));
    (0..3).all(|k| a[k].abs_diff(b[k]) <= 1)
}

/// The blend words B2 tries: Dell's word with each of its top three nibbles
/// (bits 20-31) set to 0-15 in turn, then Dell's word with each low bit
/// (0-19) set. Dell's own word appears once, first.
pub fn sweep() -> impl Iterator<Item = u32> {
    const DELL: u32 = rp_drv_blit2d::BLEND_DELL_TEXT;
    let nibbles = [20u32, 24, 28].into_iter().flat_map(|sh| {
        (0..16u32)
            .map(move |v| (DELL & !(0xF << sh)) | v << sh)
            .filter(|&w| w != DELL)
    });
    let bits = (0..20).map(|b| DELL | 1 << b);
    core::iter::once(DELL).chain(nibbles).chain(bits)
}

/// Format codes B3 tries as a per-pixel-alpha source: 12 (32 bpp, known)
/// first, then the unused codes next to the known 16- and 32-bit ones.
pub const ARGB_CODES: [u32; 8] = [0xC, 0x8, 0x9, 0x7, 0x4, 0xD, 0xE, 0xF];

/// B3's source pixels, as 16-bit words: red at alpha 0, 1/4, 1/2, 1 if
/// read as ARGB4444.
pub const SRC16: [u16; 4] = [0x0F00, 0x4F00, 0x8F00, 0xFF00];
/// B3's source pixels, as 32-bit words: red at alpha 0, 1/4, 1/2, 1 if read
/// as ARGB8888.
pub const SRC32: [u32; 4] = [0x00FF_0000, 0x40FF_0000, 0x80FF_0000, 0xFFFF_0000];

/// What a status bit's set times say: the scan lines it set on and the
/// delays after the clear.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetStats {
    /// Clears tried.
    pub tries: u32,
    /// Sets seen within the time limit.
    pub sets: u32,
    /// Lowest and highest scan line at the set.
    pub lines: Option<(u32, u32)>,
    /// Shortest and longest delay after the clear, microseconds.
    pub delay: Option<(u32, u32)>,
}

impl SetStats {
    /// Adds one clear: the line and delay of the set that followed, if any.
    pub fn add(&mut self, set: Option<(u32, u32)>) {
        self.tries += 1;
        let Some((line, us)) = set else { return };
        self.sets += 1;
        let widen =
            |r: Option<(u32, u32)>, v| Some(r.map_or((v, v), |(a, b)| (a.min(v), b.max(v))));
        self.lines = widen(self.lines, line);
        self.delay = widen(self.delay, us);
    }

    /// The likely kind: a per-line event sets within a line or two (~27 µs
    /// each), a frame event on one line, anything else is listed raw.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match (self.lines, self.delay) {
            (None, _) => "never set again",
            (_, Some((_, max))) if max <= 100 => "sets within a few lines (per-line event)",
            (Some((a, b)), _) if b - a <= 1 => "sets on one line (frame event)",
            _ => "sets on varying lines",
        }
    }
}

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_blit2d::{
        self as cmd, BLEND_DELL_TEXT, Engine, Format, OP_BLEND, ROP_PATCOPY, ROP_SRCCOPY,
    };
    use rp_drv_mbx_display::{Display, fill_rect_rgb565};
    use rp_drv_pxa_keypad::{Keypad, Pressed};
    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_api::RegisterBus;
    use rp_hal_mem::{load32_volatile, store32_volatile};
    use rp_hal_pxa27x::{Mmio, cpu, rt};
    use rp_hal_x51v::devices::RunLog;

    use super::{
        ARGB_CODES, FENCE, MARKER, SRC16, SRC32, SetStats, coverage_row, lerp565, near, sweep,
    };

    rp_hal_pxa27x::entry!(main);

    /// OSCR0 ticks a millisecond (3.25 MHz).
    const TICKS_PER_MS: u32 = 3_250;

    /// Microseconds since `t0`.
    fn us_since(t0: u32) -> u32 {
        rt::ticks().wrapping_sub(t0) / 13 * 4
    }

    macro_rules! log {
        ($($t:tt)*) => {{ let _ = writeln!(RunLog, $($t)*); }};
    }

    // Display registers (all in Dell's driver or our display driver's set).
    const DINTRS: usize = 0x2178;
    const DINTRE: usize = 0x217C;
    const DINTRCNT: usize = 0x2180;
    const DVLNUM: usize = 0x2308;
    const DVECTRL: usize = 0x2310;
    /// The 2700G's interrupt enable (system block, write-only: never read).
    const MINTENABLE: usize = 0x018;
    /// `DINTRS` bit names (mbxfb).
    const BIT_NAMES: [&str; 4] = ["HBLNK0", "HBLNK1", "VEVENT0", "VEVENT1"];

    /// GPIO85 (the 2700G's interrupt line): GPDR2 bit 21.
    const PIN85: u32 = 1 << 21;
    const GPDR2: usize = 0x014;

    // Local-memory layout for the blend stages: 12 MiB in, clear of the
    // plane and the fence.
    const COV: usize = 0x00C0_0000;
    const COV_STRIDE: i32 = 8;
    const DST: usize = 0x00C1_0000;
    const DST_STRIDE: i32 = 64;
    const ARGB16: usize = 0x00C2_0000;
    const ARGB32: usize = 0x00C2_1000;
    const RAMP: usize = 0x00C3_0000;
    const RAMP_STRIDE: usize = 240;
    const OFFSCREEN: usize = 0x00D0_0000;

    const BLUE: u16 = 0x001F;
    const RED: u16 = 0xF800;
    const GREEN: u16 = 0x07E0;

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let Some((regs, vram)) = rp_hal_x51v::devices::take_mbx() else {
            crash(format_args!("no 2700G"))
        };
        let mut display = Display::new(regs);
        let (plane, _) = display.adopt(vram.len());
        let mut regs = display.into_inner();
        let screen = (plane.offset, plane.stride);

        log!("{MARKER}: events (E1-E5), then blending (B1-B5)");
        let events_ok = events(&mut regs);
        rt::check_in();

        let mut e = Engine::new(regs, FENCE, rt::ticks, TICKS_PER_MS);
        let blend_ok = blending(&mut e, vram, screen);
        rt::check_in();

        let verdict = if events_ok && blend_ok {
            "PASS"
        } else {
            "FAIL (see the log)"
        };
        log!(
            "{MARKER}: {verdict} - top band GPU-blended, bottom band the CPU's; held until a button"
        );
        if let Some(bus) = rt::take_ffuart() {
            let mut uart = PxaUart::new(bus);
            uart.init(115_200);
            let _ = writeln!(uart, "{MARKER}: {verdict}");
        }
        hold_until_button();
        rp_hal_x51v::devices::return_to_loader();
        loop {
            core::hint::spin_loop();
        }
    }

    // ---------------------------------------------------------------- events

    /// How a status bit clears.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Clear {
        /// Write 0 to it, 1s elsewhere (Dell's way).
        Zero,
        /// Write 1 to it, 0s elsewhere.
        One,
    }

    fn clear(regs: &mut Mmio, bit: u32, how: Clear) {
        match how {
            Clear::Zero => regs.write32(DINTRS, !(1 << bit)),
            Clear::One => regs.write32(DINTRS, 1 << bit),
        }
    }

    /// Waits (up to `limit_us`) for `f`; true if it held.
    fn wait(limit_us: u32, mut f: impl FnMut() -> bool) -> bool {
        let t0 = rt::ticks();
        while us_since(t0) < limit_us {
            if f() {
                return true;
            }
        }
        false
    }

    /// E1-E5. Returns false only if a register couldn't be put back.
    fn events(regs: &mut Mmio) -> bool {
        let found = [DINTRS, DINTRE, DINTRCNT, DVECTRL].map(|r| regs.read32(r));
        log!(
            "E1 as found: DINTRS 0x{:08x} DINTRE 0x{:08x} DINTRCNT 0x{:08x} DVECTRL 0x{:08x} (VEVENT line {}, VFETCH {})",
            found[0],
            found[1],
            found[2],
            found[3],
            (found[3] >> 16) & 0xFFF,
            found[3] & 0xFFF
        );
        regs.write32(DINTRE, 0);

        // The frame: highest line seen and the time between wraps.
        let (mut top, mut prev, mut wraps, mut first, mut last) = (0, 0, 0u32, 0, 0);
        let t0 = rt::ticks();
        while us_since(t0) < 60_000 {
            let l = regs.read32(DVLNUM) & 0xFFF;
            top = top.max(l);
            if l < prev {
                let now = us_since(t0);
                if wraps == 0 {
                    first = now;
                }
                last = now;
                wraps += 1;
            }
            prev = l;
        }
        if wraps < 2 {
            log!("E1 DVLNUM never wrapped twice in 60 ms (top {top}): line-timed stages skipped");
            return restore_events(regs, found);
        }
        let frame_us = (last - first) / (wraps - 1);
        let lines = top + 1;
        log!(
            "E1 frame: lines 0..{top} ({lines}), {frame_us} us a frame ({} mHz), {} ns a line",
            1_000_000_000 / frame_us.max(1),
            frame_us * 1000 / lines.max(1)
        );

        // E2: how each bit clears.
        let mut how = [None; 4];
        for bit in 0..4u32 {
            for method in [Clear::Zero, Clear::One] {
                if !wait(40_000, || regs.read32(DINTRS) & 1 << bit != 0) {
                    log!("E2 {}: never set in 40 ms", BIT_NAMES[bit as usize]);
                    break;
                }
                clear(regs, bit, method);
                let r1 = regs.read32(DINTRS);
                let line = regs.read32(DVLNUM) & 0xFFF;
                let cleared = r1 & 1 << bit == 0;
                log!(
                    "E2 {} cleared by writing {}: {} (read 0x{r1:08x} at line {line})",
                    BIT_NAMES[bit as usize],
                    if method == Clear::Zero { 0 } else { 1 },
                    if cleared { "yes" } else { "no" }
                );
                if cleared && how[bit as usize].is_none() {
                    how[bit as usize] = Some(method);
                }
            }
        }

        // E3: when each bit sets again, cleared at 16 lines over the frame.
        let set_after = |regs: &mut Mmio, bit: u32, how: Clear, at: u32| -> Option<(u32, u32)> {
            if !wait(2 * frame_us, || regs.read32(DVLNUM) & 0xFFF == at) {
                return None;
            }
            clear(regs, bit, how);
            let t0 = rt::ticks();
            while us_since(t0) < 2 * frame_us {
                if regs.read32(DINTRS) & 1 << bit != 0 {
                    return Some((regs.read32(DVLNUM) & 0xFFF, us_since(t0)));
                }
            }
            None
        };
        let measure = |regs: &mut Mmio, bit: u32, how: Clear, n: u32| {
            let mut s = SetStats::default();
            for k in 0..n {
                s.add(set_after(regs, bit, how, k * lines / n));
                rt::check_in();
            }
            s
        };
        for bit in 0..4u32 {
            let Some(h) = how[bit as usize] else {
                log!(
                    "E3 {}: no way to clear it found: skipped",
                    BIT_NAMES[bit as usize]
                );
                continue;
            };
            let s = measure(regs, bit, h, 16);
            log!(
                "E3 {}: set again {}/{} times, lines {:?}, after {:?} us: {}",
                BIT_NAMES[bit as usize],
                s.sets,
                s.tries,
                s.lines,
                s.delay,
                s.kind()
            );
        }

        // E4: the VEVENT line moved; bits 2/3 timed again; GPIO85 at it.
        let gpio = rt::take_gpio();
        let irq = gpio.map(|mut g| {
            let d = g.read32(GPDR2);
            g.write32(GPDR2, d & !PIN85);
            rt::take_gpio_irq(85, rt::Edge::Both)
        });
        let mut irq = irq.flatten();
        if irq.is_none() {
            log!("E4 GPIO85's interrupt unavailable: edges not counted");
        }
        for line in [100u32, 400] {
            regs.write32(DVECTRL, (found[3] & !(0xFFF << 16)) | line << 16);
            for bit in [2u32, 3] {
                let Some(h) = how[bit as usize] else { continue };
                let s = measure(regs, bit, h, 8);
                log!(
                    "E4 VEVENT at {line}: {} set {}/{}, lines {:?}: {}",
                    BIT_NAMES[bit as usize],
                    s.sets,
                    s.tries,
                    s.lines,
                    s.kind()
                );
            }
            let (Some(line_irq), Some(h)) = (irq.as_mut(), how[2]) else {
                continue;
            };
            // VEVENT0 enabled: each set should edge GPIO85; it is cleared
            // at once so the line can edge again next frame.
            regs.write32(MINTENABLE, 0x800F);
            clear(regs, 2, h);
            let _ = line_irq.take();
            line_irq.done();
            regs.write32(DINTRE, 1 << 2);
            let (mut edges, mut at) = (0u32, [0u32; 3]);
            let t0 = rt::ticks();
            while us_since(t0) < 100_000 {
                if line_irq.take() {
                    let l = regs.read32(DVLNUM) & 0xFFF;
                    if (edges as usize) < at.len() {
                        at[edges as usize] = l;
                    }
                    edges += 1;
                    if regs.read32(DINTRS) & 1 << 2 != 0 {
                        clear(regs, 2, h);
                    }
                    line_irq.done();
                }
            }
            regs.write32(DINTRE, 0);
            log!(
                "E4 VEVENT at {line}, VEVENT0 enabled: GPIO85 edges in 100 ms {edges} (both edges: 2 a set), first at lines {:?}",
                &at[..(edges as usize).min(3)]
            );
        }
        regs.write32(DVECTRL, found[3]);

        // E5: DINTRCNT's effect on how often each bit sets in 20 ms.
        let count_sets = |regs: &mut Mmio, how: &[Option<Clear>; 4]| -> [u32; 4] {
            let mut n = [0u32; 4];
            let t0 = rt::ticks();
            while us_since(t0) < 20_000 {
                let s = regs.read32(DINTRS);
                for bit in 0..4 {
                    if let (true, Some(h)) = (s & 1 << bit != 0, how[bit]) {
                        n[bit] += 1;
                        clear(regs, bit as u32, h);
                    }
                }
            }
            n
        };
        log!(
            "E5 DINTRCNT 0x{:08x}: sets in 20 ms (bits 0-3) {:?}",
            found[2],
            count_sets(regs, &how)
        );
        for v in [1u32, 4] {
            regs.write32(DINTRCNT, v);
            let back = regs.read32(DINTRCNT);
            let n = count_sets(regs, &how);
            log!("E5 DINTRCNT {v} (reads 0x{back:08x}): sets in 20 ms {n:?}");
        }
        restore_events(regs, found)
    }

    /// Puts `DINTRCNT`, `DVECTRL` and `DINTRE` back as found (the 2700G
    /// keeps them across images); `MINTENABLE` back to WM's `0x800F`.
    fn restore_events(regs: &mut Mmio, found: [u32; 4]) -> bool {
        regs.write32(DINTRCNT, found[2]);
        regs.write32(DVECTRL, found[3]);
        regs.write32(DINTRE, found[1]);
        regs.write32(MINTENABLE, 0x800F);
        let ok = regs.read32(DVECTRL) == found[3] && regs.read32(DINTRE) == found[1];
        log!(
            "E restore: DVECTRL, DINTRE, DINTRCNT as found: {}",
            if ok { "yes" } else { "NO" }
        );
        ok
    }

    // -------------------------------------------------------------- blending

    /// Engine stalls; three in a row end the blend stages.
    struct Probe<'a> {
        e: &'a mut Engine<Mmio>,
        vram: &'a mut [u8],
        stalls: u32,
        in_a_row: u32,
    }

    impl Probe<'_> {
        /// The state the probes start from: no scaling, key off, blend 0.
        fn baseline(&mut self) -> bool {
            let mut w = [0u32; 6];
            w[0] = cmd::SCALE_1_TO_1;
            w[1..4].copy_from_slice(&cmd::key(0, 0));
            w[4..].copy_from_slice(&cmd::blend(0));
            self.e.submit(&w).is_ok() && self.e.finish(self.vram, 50_000).is_ok()
        }

        /// Fills `n` pixels of the test row with `c` (CPU writes, so the
        /// destination doesn't depend on the engine).
        fn prefill(&mut self, n: usize, c: u16) {
            let v = u32::from(c) | u32::from(c) << 16;
            for k in (0..n).step_by(2) {
                let _ = store32_volatile(self.vram, DST + 2 * k, v);
            }
            cpu::drain_write_buffer();
        }

        fn px(&self, k: usize) -> u16 {
            let off = DST + 2 * k;
            load32_volatile(self.vram, off & !3).map_or(0xDEAD, |w| (w >> (8 * (off & 2))) as u16)
        }

        /// Runs `words` over a test row prefilled with `base`, returns the
        /// first `N` pixels, or `None` after a stall (engine reset).
        fn run<const N: usize>(&mut self, base: u16, words: &[u32]) -> Option<[u16; N]> {
            self.prefill(N.max(16), base);
            let done = self
                .e
                .submit(words)
                .and_then(|()| self.e.finish(self.vram, 50_000));
            if let Err(why) = done {
                self.stalls += 1;
                self.in_a_row += 1;
                log!("  stuck ({why}): engine reset");
                self.e.reset(rt::delay_us);
                let back = self.baseline();
                log!(
                    "  after the reset: {}",
                    if back { "runs" } else { "STILL STUCK" }
                );
                if !back {
                    self.in_a_row = 3;
                }
                return None;
            }
            self.in_a_row = 0;
            Some(core::array::from_fn(|k| self.px(k)))
        }

        fn gave_up(&self) -> bool {
            self.in_a_row >= 3
        }
    }

    /// Up to 16 pixels as hex, on one line.
    struct Px<'a>(&'a [u16]);
    impl core::fmt::Display for Px<'_> {
        fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
            for p in self.0 {
                write!(f, " {p:04x}")?;
            }
            Ok(())
        }
    }

    /// The coverage blit: 16 pixels of the test row, red through coverage,
    /// with `flags` on the header (Dell's: [`OP_BLEND`]).
    fn coverage_blit(flags: u32) -> ([u32; 10], usize) {
        cmd::blit(
            (Format::RGB565, DST, DST_STRIDE),
            (Format::COVERAGE4, COV, COV_STRIDE),
            (0, 0),
            (0, 0, 16, 1),
            ROP_SRCCOPY,
            flags,
            u32::from(RED),
        )
    }

    /// `blend` state words then the blit: one batch.
    fn with_blend(blend: u32, blit: ([u32; 10], usize)) -> ([u32; 12], usize) {
        let mut w = [0u32; 12];
        w[..2].copy_from_slice(&cmd::blend(blend));
        w[2..2 + blit.1].copy_from_slice(&blit.0[..blit.1]);
        (w, 2 + blit.1)
    }

    fn blending(e: &mut Engine<Mmio>, vram: &mut [u8], (a, s): (usize, usize)) -> bool {
        log!(
            "B0 engine: ID 0x{:08x}",
            e.regs.read32(cmd::MBX + cmd::reg::ID)
        );
        e.reset(rt::delay_us);
        let _ = store32_volatile(vram, FENCE, 0);
        cpu::drain_write_buffer();
        // Sources, written once by the CPU.
        for (k, b) in coverage_row().chunks(4).enumerate() {
            let w = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            let _ = store32_volatile(vram, COV + 4 * k, w);
        }
        for (k, p) in SRC16.chunks(2).enumerate() {
            let _ = store32_volatile(
                vram,
                ARGB16 + 4 * k,
                u32::from(p[0]) | u32::from(p[1]) << 16,
            );
        }
        for (k, &w) in SRC32.iter().enumerate() {
            let _ = store32_volatile(vram, ARGB32 + 4 * k, w);
        }
        cpu::drain_write_buffer();
        let mut p = Probe {
            e,
            vram,
            stalls: 0,
            in_a_row: 0,
        };
        if !p.baseline() {
            log!("B0 the engine doesn't run: blend stages skipped");
            return false;
        }

        // B1: Dell's text blit and its variants, against the CPU's lerp.
        let lerp: [u16; 16] = core::array::from_fn(|c| lerp565(BLUE, RED, c as u32, 15));
        let swapped: [u16; 16] = core::array::from_fn(|c| lerp[c ^ 1]);
        log!("B1 coverage 0..15, red over blue; CPU lerp:{}", Px(&lerp));
        let mut dell: Option<[u16; 16]> = None;
        for (name, blend, flags) in [
            ("Dell (blend 0x09100000, bit 17)", BLEND_DELL_TEXT, OP_BLEND),
            ("blend 0x09100000, no bit 17", BLEND_DELL_TEXT, 0),
            ("blend 0, bit 17", 0, OP_BLEND),
            ("blend 0, no bit 17", 0, 0),
        ] {
            let (w, n) = with_blend(blend, coverage_blit(flags));
            let Some(got) = p.run::<16>(BLUE, &w[..n]) else {
                log!("B1 {name}: stuck");
                continue;
            };
            let verdict = if got.iter().zip(&lerp).all(|(g, l)| near(*g, *l)) {
                "= lerp (low nibble first)"
            } else if got.iter().zip(&swapped).all(|(g, l)| near(*g, *l)) {
                "= lerp (high nibble first)"
            } else if got.iter().all(|&g| g == BLUE) {
                "untouched"
            } else {
                "other"
            };
            log!("B1 {name}:{} -> {verdict}", Px(&got));
            if blend == BLEND_DELL_TEXT && flags == OP_BLEND {
                dell = Some(got);
            }
        }

        // B2: the blend word swept; coverage 0, 5, 10, 15 read back.
        let idx = [0usize, 5, 10, 15];
        for word in sweep() {
            if p.gave_up() {
                break;
            }
            rt::check_in();
            let (w, n) = with_blend(word, coverage_blit(OP_BLEND));
            let Some(got) = p.run::<16>(BLUE, &w[..n]) else {
                log!("B2 0x{word:08x}: stuck");
                continue;
            };
            let four = idx.map(|k| got[k]);
            let mark = if got.iter().all(|&g| g == BLUE) {
                " untouched"
            } else if Some(got) == dell {
                " =Dell"
            } else {
                ""
            };
            log!("B2 0x{word:08x}:{}{mark}", Px(&four));
        }

        // B3: per-pixel alpha sources, 4 pixels each, over green.
        for code in ARGB_CODES {
            for (src, name) in [(ARGB16, "16"), (ARGB32, "32")] {
                for (vname, blend, flags) in [
                    ("Dell+17", BLEND_DELL_TEXT, OP_BLEND),
                    ("0+17", 0, OP_BLEND),
                    ("plain", 0, 0),
                ] {
                    if p.gave_up() {
                        break;
                    }
                    let stride = if name == "16" { 8 } else { 16 };
                    let blit = cmd::blit(
                        (Format::RGB565, DST, DST_STRIDE),
                        (Format::code(code), src, stride),
                        (0, 0),
                        (0, 0, 4, 1),
                        ROP_SRCCOPY,
                        flags,
                        u32::from(RED),
                    );
                    let (w, n) = with_blend(blend, blit);
                    match p.run::<4>(GREEN, &w[..n]) {
                        Some(got) => log!("B3 code {code} src{name} {vname}:{}", Px(&got)),
                        None => log!("B3 code {code} src{name} {vname}: stuck"),
                    }
                }
            }
        }

        // B4: translucent solid fills over green.
        for colour in [0x80FF_0000u32, 0x8000_F800] {
            for (vname, blend) in [("Dell", BLEND_DELL_TEXT), ("0", 0)] {
                if p.gave_up() {
                    break;
                }
                let mut fill = cmd::fill(Format::RGB565, DST, 64, (0, 0, 4, 1), colour);
                fill[2] = 0x8000_0000 | ROP_PATCOPY | OP_BLEND;
                let mut w = [0u32; 9];
                w[..2].copy_from_slice(&cmd::blend(blend));
                w[2..].copy_from_slice(&fill);
                if let Some(got) = p.run::<4>(GREEN, &w) {
                    log!("B4 fill 0x{colour:08x} bit 17, blend {vname}:{}", Px(&got));
                }
            }
        }

        // Back to the state every image expects: blend 0, key off.
        let state_ok = p.baseline();
        let stalls = p.stalls;
        let e = p.e;
        let vram = p.vram;
        log!(
            "B blend state put back to 0: {}; engine stalls {stalls}",
            if state_ok { "yes" } else { "NO" }
        );

        // B5: 20 off-screen full-screen fills; do scan-out's underrun or
        // overrun bits (4-6, 16-18) set? Cleared both ways first.
        e.regs.write32(DINTRS, !0x0007_0070);
        e.regs.write32(DINTRS, 0x0007_0070);
        let before = e.regs.read32(DINTRS) & 0x0007_0070;
        let t0 = rt::ticks();
        for i in 0..20u32 {
            let _ = e.submit(&cmd::fill(
                Format::RGB565,
                OFFSCREEN,
                s,
                (0, 0, 480, 640),
                i,
            ));
        }
        let fine = e.finish(vram, 1_000_000).is_ok();
        let after = e.regs.read32(DINTRS) & 0x0007_0070;
        log!(
            "B5 under/overrun bits 0x{before:08x} before, 0x{after:08x} after 20 fills ({} us{})",
            us_since(t0),
            if fine { "" } else { ", fence never written" }
        );

        // The picture: a GPU-blended ramp (Dell's state, 16 steps of 30 px)
        // above the CPU's lerp of the same.
        picture(e, vram, (a, s));
        state_ok && fine && stalls < 3
    }

    fn picture(e: &mut Engine<Mmio>, vram: &mut [u8], (a, s): (usize, usize)) {
        for row in 0..64 {
            for k in 0..RAMP_STRIDE / 4 {
                // 4 bytes = 8 pixels; pixel x has coverage x / 30.
                let mut w = 0u32;
                for n in 0..8 {
                    let x = 8 * k + n;
                    w |= ((x / 30) as u32 & 0xF) << (4 * n);
                }
                let _ = store32_volatile(vram, RAMP + row * RAMP_STRIDE + 4 * k, w);
            }
        }
        fill_rect_rgb565(vram, s, 0, 0, 480, 640, 0);
        fill_rect_rgb565(vram, s, 0, 200, 480, 64, BLUE);
        for c in 0..16u32 {
            let x = (c * 30) as usize;
            fill_rect_rgb565(vram, s, x, 300, 30, 64, lerp565(BLUE, RED, c, 15));
        }
        cpu::drain_write_buffer();
        let blit = cmd::blit(
            (Format::RGB565, a + 200 * s, s as i32),
            (Format::COVERAGE4, RAMP, RAMP_STRIDE as i32),
            (0, 0),
            (0, 0, 480, 64),
            ROP_SRCCOPY,
            OP_BLEND,
            u32::from(RED),
        );
        let (w, n) = with_blend(BLEND_DELL_TEXT, blit);
        let mut all = [0u32; 14];
        all[..n].copy_from_slice(&w[..n]);
        all[n..n + 2].copy_from_slice(&cmd::blend(0));
        let ok = e
            .submit(&all[..n + 2])
            .and_then(|()| e.finish(vram, 50_000))
            .is_ok();
        log!(
            "picture: GPU ramp at y 200-263, CPU ramp at y 300-363{}",
            if ok { "" } else { " (GPU ramp: engine stuck)" }
        );
    }

    /// Keeps the picture up until a button is pressed and released,
    /// checking in with the loader's watchdog; gives up after 5 minutes.
    fn hold_until_button() {
        let Some(kp) = rt::take_keypad() else {
            log!("no keypad: returning");
            return;
        };
        let mut keypad = Keypad::new(kp);
        keypad.enable();
        keypad.acknowledge();
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

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    #[inline(never)]
    fn crash(msg: core::fmt::Arguments) -> ! {
        rp_hal_x51v::devices::crash_to_loader(msg);
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        crash(format_args!("PANIC {}", info.message()))
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_row_counts_up_low_nibble_first() {
        let r = coverage_row();
        assert_eq!(r[0], 0x10);
        assert_eq!(r[7], 0xFE);
    }

    #[test]
    fn lerp_ends_and_middle() {
        assert_eq!(lerp565(0x001F, 0xF800, 0, 15), 0x001F);
        assert_eq!(lerp565(0x001F, 0xF800, 15, 15), 0xF800);
        // Half way: red 31 * 7.5/15 = 15.5 -> 16 (rounded), blue the same.
        let [r, g, b] = channels(lerp565(0x001F, 0xF800, 15, 30));
        assert_eq!((r, g, b), (16, 0, 16));
    }

    #[test]
    fn near_allows_one_step() {
        assert!(near(0x0821, 0x0000));
        assert!(!near(0x1000, 0x0000));
    }

    #[test]
    fn sweep_is_bounded_and_starts_with_dell() {
        let words: std::vec::Vec<u32> = sweep().collect();
        assert_eq!(words[0], 0x0910_0000);
        assert_eq!(words.len(), 1 + 3 * 15 + 20);
        assert_eq!(words.iter().filter(|&&w| w == 0x0910_0000).count(), 1);
        assert!(words.contains(&0x0010_0000), "nibble 24 at 0");
        assert!(words.contains(&0x0910_0001), "bit 0");
    }

    #[test]
    fn set_stats_kinds() {
        let mut s = SetStats::default();
        assert_eq!(s.kind(), "never set again");
        s.add(Some((643, 9_000)));
        s.add(Some((643, 15_000)));
        s.add(None);
        assert_eq!((s.tries, s.sets), (3, 2));
        assert_eq!(s.kind(), "sets on one line (frame event)");
        let mut h = SetStats::default();
        h.add(Some((10, 20)));
        h.add(Some((300, 30)));
        assert_eq!(h.kind(), "sets within a few lines (per-line event)");
    }
}
