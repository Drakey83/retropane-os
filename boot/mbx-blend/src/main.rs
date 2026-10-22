//! Test image (renderer step 1, ADR-0022): what the MBX 2D engine's blend
//! state does, and what the display's interrupt events mean, before
//! `Mbx2dRenderer` is designed around either.
//!
//! **Events first** (no engine work; register names from the Linux mbxfb
//! header: `DINTRS` bits 0/1 HBLNK0/1, 2/3 VEVENT0/1, 4-6 underruns, 16-18
//! overruns; `DVECTRL` (`0x2310`) holds the VEVENT line, 643 here):
//!
//! - E1: the registers as found, and the frame (lines, period) from `DVLNUM`;
//! - E2: each status bit clears by read-modify-write (`DINTRS` is plain
//!   read/write: a 1 written sets a bit, measured on the first run);
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
//! - B6 (ADR-0023): source stride 0 (one row repeated down a rectangle);
//!   blending onto a 32 bpp destination against the reference; an
//!   ARGB4444 destination (option A); a 32 bpp screen shown while the
//!   engine fills and copies 32 bpp, scan-out's under/overrun bits read;
//!   the screen left showing four smooth 32 bpp ramps until a button.
//!
//! First run (2026-10-14, docs/hw/2700g.md): all of it answered except E5
//! and B5, which cleared bits by writing 1s and so set them; fixed.
//!
//! Every wait has a timeout. A stuck engine is reset and the probe goes on
//! (logged); three in a row end the blend stages. Every state touched
//! (`DVECTRL`, `DINTRCNT`, `DINTRE`, `MINTENABLE`, the blend word, the key)
//! is put back: the 2700G isn't reset between images.

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

/// Whether this build runs B7 alone (`RP_MBX_BLEND_B7`): ADR-0023's
/// 16-bit screen measurement, unattended, nothing shown, no button.
pub const B7_ONLY: bool = option_env!("RP_MBX_BLEND_B7").is_some();

/// How an 8-bit channel becomes `bits` bits: the rules B7 tells apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Narrow {
    /// The top bits (`v >> (8 - bits)`).
    Truncate,
    /// The nearest step of the top bits, half-way up (`(v + half) >> ..`,
    /// held at the top).
    Round,
    /// Scaled to the nearest of `2^bits - 1` levels (`(v * max + 127) / 255`).
    Scale,
}

impl Narrow {
    /// Every rule, in the order B7 logs them.
    pub const ALL: [Self; 3] = [Self::Truncate, Self::Round, Self::Scale];

    /// The rule's name in the run log.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Truncate => "truncates",
            Self::Round => "rounds",
            Self::Scale => "scales",
        }
    }

    /// `v` (0-255) narrowed to `bits` bits (5 or 6).
    #[must_use]
    pub const fn apply(self, v: u32, bits: u32) -> u32 {
        let max = (1 << bits) - 1;
        match self {
            Self::Truncate => v >> (8 - bits),
            Self::Round => {
                let r = (v + (1 << (7 - bits))) >> (8 - bits);
                if r > max { max } else { r }
            }
            Self::Scale => (v * max + 127) / 255,
        }
    }
}

/// Of the levels 0-255, how many each [`Narrow`] rule gives as `got`
/// gave them: `got(v)` is the channel the engine wrote for level `v`,
/// `level(v)` the 8-bit value an exact engine would narrow.
#[must_use]
pub fn rule_counts(bits: u32, level: impl Fn(u32) -> u32, got: impl Fn(u32) -> u32) -> [u32; 3] {
    Narrow::ALL.map(|rule| {
        (0..256)
            .filter(|&v| got(v) == rule.apply(level(v), bits))
            .count() as u32
    })
}

/// The side of B7's grey ramp, pixels.
pub const RAMP: usize = 64;

/// B7's grey ramp (run P2b's): level `64 + x / 8 + 8 * (y / 8)`, so every
/// level 64-127 fills one 8 x 8 cell and meets every place of a 4 x 4
/// pattern.
#[must_use]
pub const fn ramp_level(x: usize, y: usize) -> u32 {
    64 + (x / 8) as u32 + 8 * (y / 8) as u32
}

/// The marker B7 leaves in a 16-bit destination before each try: red 27,
/// which no ramp level (64-127: red 8-15) converts to.
pub const MARK16: u16 = 0xDEAD;

/// What one converted ramp came out as, against truncation of each
/// pixel's level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RampStats {
    /// Pixels still holding [`MARK16`] (not written).
    pub unwritten: u32,
    /// Pixels whose channels are all their level truncated.
    pub trunc: u32,
    /// Pixels with every channel truncated or one step up, one at least up.
    pub up: u32,
    /// Anything else.
    pub other: u32,
    /// Levels seen truncated somewhere (bit `level - 64`).
    seen_trunc: u64,
    /// Levels seen one step up somewhere.
    seen_up: u64,
}

impl RampStats {
    /// Adds pixel `p` written for ramp `level`.
    pub fn add(&mut self, level: u32, p: u16) {
        if p == MARK16 {
            self.unwritten += 1;
            return;
        }
        let got = channels(p);
        let want = [
            Narrow::Truncate.apply(level, 5),
            Narrow::Truncate.apply(level, 6),
            Narrow::Truncate.apply(level, 5),
        ];
        let bit = 1u64 << (level - 64).min(63);
        if got == want {
            self.trunc += 1;
            self.seen_trunc |= bit;
        } else if (0..3).all(|k| got[k] == want[k] || got[k] == want[k] + 1) {
            self.up += 1;
            self.seen_up |= bit;
        } else {
            self.other += 1;
        }
    }

    /// Levels that came out truncated at some places and one step up at
    /// others: a position-dependent result, which is what a dither does
    /// (a plain rule gives one answer a level).
    #[must_use]
    pub const fn mixed_levels(&self) -> u32 {
        (self.seen_trunc & self.seen_up).count_ones()
    }

    /// The reading, measured not judged: what the counts look like.
    #[must_use]
    pub const fn reading(&self) -> &'static str {
        if self.unwritten > 0 || self.other > 0 {
            "something else (see the counts)"
        } else if self.mixed_levels() > 0 {
            "position-dependent: dither-like"
        } else if self.up > 0 {
            "one answer a level, some up: rounding-like"
        } else {
            "every pixel truncated"
        }
    }
}

/// B7's tries for a 2D dither: `(header flags, blend word)`. First the
/// plain conversion, then each header bit nobody has tried (19, 21, 22,
/// 27) alone; then the blend path (bit 17 with Fs 1, Fd 0: a plain copy,
/// `0x02000000`) and that word with each of the bits B2 only tried on one
/// 16-pixel row (`[31:28]`, `[19:0]`) alone.
pub fn dither_tries() -> impl Iterator<Item = (u32, Option<u32>)> {
    const COPY: u32 = 0x0200_0000;
    let plain = core::iter::once((0, None));
    let header = [19u32, 21, 22, 27].into_iter().map(|b| (1 << b, None));
    let path = core::iter::once((rp_drv_blit2d::OP_BLEND, Some(COPY)));
    let words = (28..32)
        .chain(0..20)
        .map(|b| (rp_drv_blit2d::OP_BLEND, Some(COPY | 1 << b)));
    plain.chain(header).chain(path).chain(words)
}

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_blit2d::{
        self as cmd, BLEND_DELL_TEXT, Engine, Format, OP_BLEND, ROP_PATCOPY, ROP_SRCCOPY,
    };
    use rp_drv_mbx_display::Display;
    use rp_drv_pxa_keypad::{Keypad, Pressed};
    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_api::RegisterBus;
    use rp_hal_mem::{load32_volatile, store32_volatile};
    use rp_hal_pxa27x::{Mmio, cpu, rt};
    use rp_hal_x51v::devices::RunLog;

    use super::{
        ARGB_CODES, B7_ONLY, FENCE, MARK16, MARKER, Narrow, RAMP, RampStats, SRC16, SRC32,
        SetStats, channels, coverage_row, dither_tries, lerp565, near, ramp_level, rule_counts,
        sweep,
    };

    use rp_image::crash;

    rp_image::entry!(main);

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
    const OFFSCREEN: usize = 0x00D0_0000;

    const BLUE: u16 = 0x001F;
    const RED: u16 = 0xF800;
    const GREEN: u16 = 0x07E0;

    fn main() -> ! {
        rt::init_mmu();
        let Some((regs, vram)) = rp_image::take_mbx() else {
            crash(format_args!("no 2700G"))
        };
        let mut display = Display::new(regs);
        if B7_ONLY {
            b7_only(display.into_inner(), vram);
        }
        let (plane, _) = display.adopt(vram.len());
        let mut regs = display.into_inner();
        let screen = (plane.offset, plane.stride);

        log!("{MARKER}: events (E1-E5), then blending (B1-B5)");
        let events_ok = events(&mut regs);
        rt::check_in();

        let mut e = Engine::new(regs, FENCE, rt::ticks, TICKS_PER_MS);
        let (blend_ok, saved) = blending(&mut e, vram, screen);
        rt::check_in();

        let verdict = if events_ok && blend_ok {
            "PASS"
        } else {
            "FAIL (see the log)"
        };
        log!("{MARKER}: {verdict} - 32 bpp ramps on screen; held until a button");
        let keypad = arm_keypad();
        if let Some(bus) = rt::take_ffuart() {
            let mut uart = PxaUart::new(bus);
            uart.init(115_200);
            let _ = writeln!(uart, "{MARKER}: {verdict}");
        }
        hold_until_button(keypad);
        // The plane as found (the 2700G keeps it across images).
        for (off, v) in saved.into_iter().flatten() {
            e.regs.write32(off, v);
        }
        rp_image::finish()
    }

    // ---------------------------------------------------------------- events

    /// Clears one status bit: `DINTRS` is plain read/write (a 1 written
    /// sets a bit; measured 2026-10-14), so only read-modify-write is safe.
    fn clear(regs: &mut Mmio, bit: u32) {
        rp_drv_mbx_display::ack_events(regs, 1 << bit);
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

        // E2: does each bit clear by read-modify-write?
        let mut how = [false; 4];
        for bit in 0..4u32 {
            if !wait(40_000, || regs.read32(DINTRS) & 1 << bit != 0) {
                log!("E2 {}: never set in 40 ms", BIT_NAMES[bit as usize]);
                continue;
            }
            clear(regs, bit);
            let r1 = regs.read32(DINTRS);
            how[bit as usize] = r1 & 1 << bit == 0;
            log!(
                "E2 {} cleared by read-modify-write: {} (read 0x{r1:08x})",
                BIT_NAMES[bit as usize],
                if how[bit as usize] { "yes" } else { "no" }
            );
        }

        // E3: when each bit sets again, cleared at 16 lines over the frame.
        let set_after = |regs: &mut Mmio, bit: u32, at: u32| -> Option<(u32, u32)> {
            if !wait(2 * frame_us, || regs.read32(DVLNUM) & 0xFFF == at) {
                return None;
            }
            clear(regs, bit);
            let t0 = rt::ticks();
            while us_since(t0) < 2 * frame_us {
                if regs.read32(DINTRS) & 1 << bit != 0 {
                    return Some((regs.read32(DVLNUM) & 0xFFF, us_since(t0)));
                }
            }
            None
        };
        let measure = |regs: &mut Mmio, bit: u32, n: u32| {
            let mut s = SetStats::default();
            for k in 0..n {
                s.add(set_after(regs, bit, k * lines / n));
                rt::check_in();
            }
            s
        };
        for bit in 0..4u32 {
            if !how[bit as usize] {
                log!("E3 {}: doesn't clear: skipped", BIT_NAMES[bit as usize]);
                continue;
            }
            let s = measure(regs, bit, 16);
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
                if !how[bit as usize] {
                    continue;
                }
                let s = measure(regs, bit, 8);
                log!(
                    "E4 VEVENT at {line}: {} set {}/{}, lines {:?}: {}",
                    BIT_NAMES[bit as usize],
                    s.sets,
                    s.tries,
                    s.lines,
                    s.kind()
                );
            }
            let (Some(line_irq), true) = (irq.as_mut(), how[2]) else {
                continue;
            };
            // VEVENT0 enabled: each set should edge GPIO85; it is cleared
            // at once so the line can edge again next frame.
            regs.write32(MINTENABLE, 0x800F);
            clear(regs, 2);
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
                        clear(regs, 2);
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
        let count_sets = |regs: &mut Mmio, how: &[bool; 4]| -> [u32; 4] {
            let mut n = [0u32; 4];
            let t0 = rt::ticks();
            while us_since(t0) < 20_000 {
                let s = regs.read32(DINTRS);
                for bit in 0..4 {
                    if s & 1 << bit != 0 && how[bit] {
                        n[bit] += 1;
                        clear(regs, bit as u32);
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
        rp_image::need(
            cmd::blit(
                (Format::RGB565, DST, DST_STRIDE),
                (Format::COVERAGE4, COV, COV_STRIDE),
                (0, 0),
                (0, 0, 16, 1),
                ROP_SRCCOPY,
                flags,
                u32::from(RED),
            ),
            "a 2D rectangle",
        )
    }

    /// `blend` state words then the blit: one batch.
    fn with_blend(blend: u32, blit: ([u32; 10], usize)) -> ([u32; 12], usize) {
        let mut w = [0u32; 12];
        w[..2].copy_from_slice(&cmd::blend(blend));
        w[2..2 + blit.1].copy_from_slice(&blit.0[..blit.1]);
        (w, 2 + blit.1)
    }

    fn blending(e: &mut Engine<Mmio>, vram: &mut [u8], (_, s): (usize, usize)) -> (bool, Saved) {
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
            return (false, None);
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
                    let blit = rp_image::need(
                        cmd::blit(
                            (Format::RGB565, DST, DST_STRIDE),
                            (Format::code(code), src, stride),
                            (0, 0),
                            (0, 0, 4, 1),
                            ROP_SRCCOPY,
                            flags,
                            u32::from(RED),
                        ),
                        "a 2D rectangle",
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
                let mut fill = rp_image::need(
                    cmd::fill(Format::RGB565, DST, 64, (0, 0, 4, 1), colour),
                    "a 2D rectangle",
                );
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
        let flags = rp_drv_mbx_display::event::UNDER_OVERRUN;
        let found = rp_drv_mbx_display::ack_events(&mut e.regs, flags) & flags;
        let before = e.regs.read32(DINTRS) & flags;
        let t0 = rt::ticks();
        for i in 0..20u32 {
            let _ = e.submit(&rp_image::need(
                cmd::fill(Format::RGB565, OFFSCREEN, s, (0, 0, 480, 640), i),
                "a 2D rectangle",
            ));
        }
        let fine = e.finish(vram, 1_000_000).is_ok();
        let after = e.regs.read32(DINTRS) & flags;
        log!(
            "B5 under/overrun bits as found 0x{found:08x}, cleared 0x{before:08x}, after 20 fills 0x{after:08x} ({} us{})",
            us_since(t0),
            if fine { "" } else { ", fence never written" }
        );

        // B6: what ADR-0023 needs; ends with a 32 bpp screen on show.
        let (b6_ok, saved) = adr23(e, vram);
        (state_ok && fine && stalls < 3 && b6_ok, saved)
    }

    /// The plane's registers as found, to put back after the 32 bpp screen.
    type Saved = Option<[(usize, u32); 5]>;

    // B6's places in local memory: two 32 bpp screens (1.2 MiB each, at 2
    // and 4 MiB, clear of the 16 bpp plane below 1 MiB), a row for
    // stride-0 blits, small 32 bpp and ARGB4444 test rows.
    const SCREEN32: usize = 0x0020_0000;
    const SCREEN32B: usize = 0x0040_0000;
    const STRIDE32: usize = 480 * 4;
    const ROW: usize = 0x00C4_0000;
    const DST32: usize = 0x00C5_0000;
    const DST4444: usize = 0x00C6_0000;

    /// `N` words of local memory from `at`.
    fn words<const N: usize>(vram: &[u8], at: usize) -> [u32; N] {
        core::array::from_fn(|k| load32_volatile(vram, at + 4 * k).unwrap_or(0xDEAD_BEEF))
    }

    /// Stores `ws` at `at` (CPU writes).
    fn set(vram: &mut [u8], at: usize, ws: &[u32]) {
        for (k, &w) in ws.iter().enumerate() {
            let _ = store32_volatile(vram, at + 4 * k, w);
        }
        cpu::drain_write_buffer();
    }

    /// Runs `w` and waits for it; false (logged, engine reset) if stuck.
    fn go(e: &mut Engine<Mmio>, vram: &[u8], w: &[u32], what: &str) -> bool {
        match e.submit(w).and_then(|()| e.finish(vram, 1_000_000)) {
            Ok(_) => true,
            Err(why) => {
                log!("{what}: {why}: engine reset");
                e.reset(rt::delay_us);
                false
            }
        }
    }

    /// Straight-alpha `over` onto an opaque pixel, 8-bit channels, rounded:
    /// the reference's arithmetic for B6's check.
    fn over_opaque(src: u32, dst: u32) -> u32 {
        let a = src >> 24;
        let ch = |sh: u32| {
            let (s, d) = ((src >> sh) & 0xFF, (dst >> sh) & 0xFF);
            ((s * a + d * (255 - a) + 127) / 255) << sh
        };
        0xFF00_0000 | ch(16) | ch(8) | ch(0)
    }

    /// A pixel of the 16 bpp test row's line `l`.
    fn px16(vram: &[u8], l: usize, k: usize) -> u16 {
        let off = DST + l * DST_STRIDE as usize + 2 * k;
        load32_volatile(vram, off & !3).map_or(0xDEAD, |w| (w >> (8 * (off & 2))) as u16)
    }

    /// B6 (ADR-0023): stride 0; blending onto a 32 bpp destination, checked
    /// against the reference; an ARGB4444 destination (option A's record);
    /// 32 bpp fills timed; a 32 bpp screen shown with the engine busy,
    /// scan-out's under/overrun bits read. Leaves the 32 bpp screen up and
    /// returns the plane's registers as found.
    fn adr23(e: &mut Engine<Mmio>, vram: &mut [u8]) -> (bool, Saved) {
        use rp_drv_mbx_display::{PixelFormat, Plane, event, layers::Layer, reg as dreg};
        let mut ok = true;

        // B6a: stride 0. A 16-pixel ARGB8888 row copied over 4 lines of
        // RGB565, then red at alpha 0x80 blended over green.
        for (name, flags, blend, base) in [
            ("copy", 0, 0, 0x0000u16),
            ("blend", OP_BLEND, BLEND_DELL_TEXT, GREEN),
        ] {
            let row: [u32; 16] = if flags == 0 {
                core::array::from_fn(|k| 0xFF00_0000 | (k as u32 * 0x11) << 16 | 0x40)
            } else {
                [0x80FF_0000; 16]
            };
            set(vram, ROW, &row);
            for line in 0..4 {
                let at = DST + line * DST_STRIDE as usize;
                set(vram, at, &[u32::from(base) * 0x0001_0001; 8]);
            }
            let (w, n) = with_blend(
                blend,
                rp_image::need(
                    cmd::blit(
                        (Format::RGB565, DST, DST_STRIDE),
                        (Format::BPP32, ROW, 0),
                        (0, 0),
                        (0, 0, 16, 4),
                        ROP_SRCCOPY,
                        flags,
                        0,
                    ),
                    "a 2D rectangle",
                ),
            );
            if !go(e, vram, &w[..n], "B6 stride 0") {
                ok = false;
                continue;
            }
            let lines: [[u16; 4]; 4] =
                core::array::from_fn(|l| core::array::from_fn(|k| px16(vram, l, k)));
            let same = lines.iter().all(|l| *l == lines[0]);
            log!(
                "B6a stride 0 {name}: line 0{} line 3{} -> {}",
                Px(&lines[0]),
                Px(&lines[3]),
                if same && lines[0][1] != base {
                    "every line reads the row"
                } else {
                    "NOT repeated"
                }
            );
        }

        // B6b: blending onto 32 bpp (opaque green), against the reference.
        set(vram, ARGB32, &SRC32);
        set(vram, DST32, &[0xFF00_FF00; 4]);
        let (w, n) = with_blend(
            BLEND_DELL_TEXT,
            rp_image::need(
                cmd::blit(
                    (Format::BPP32, DST32, 16),
                    (Format::BPP32, ARGB32, 16),
                    (0, 0),
                    (0, 0, 4, 1),
                    ROP_SRCCOPY,
                    OP_BLEND,
                    0,
                ),
                "a 2D rectangle",
            ),
        );
        if go(e, vram, &w[..n], "B6 32 bpp blend") {
            let got: [u32; 4] = words(vram, DST32);
            let want = SRC32.map(|s| over_opaque(s, 0xFF00_FF00));
            let exact = got
                .iter()
                .zip(&want)
                .all(|(g, w)| (g ^ w) & 0x00FF_FFFF == 0);
            let close = got.iter().zip(&want).all(|(g, w)| {
                [0, 8, 16]
                    .iter()
                    .all(|&sh| ((g >> sh) & 0xFF).abs_diff((w >> sh) & 0xFF) <= 1)
            });
            let verdict = if exact {
                "exact (colour)"
            } else if close {
                "within 1"
            } else {
                "DIFFERENT"
            };
            log!("B6b 32 bpp over: {got:08x?}, reference {want:08x?} -> {verdict}");
        } else {
            ok = false;
        }

        // B6c: ARGB4444 destination (option A): half-clear green, then
        // clear, under the same sources.
        for (name, d) in [("half-clear green", 0x80F0u32), ("clear", 0)] {
            set(vram, DST4444, &[d | d << 16; 2]);
            let (w, n) = with_blend(
                BLEND_DELL_TEXT,
                rp_image::need(
                    cmd::blit(
                        (Format::code(7), DST4444, 8),
                        (Format::BPP32, ARGB32, 16),
                        (0, 0),
                        (0, 0, 4, 1),
                        ROP_SRCCOPY,
                        OP_BLEND,
                        0,
                    ),
                    "a 2D rectangle",
                ),
            );
            if go(e, vram, &w[..n], "B6 ARGB4444 destination") {
                let [w0, w1]: [u32; 2] = words(vram, DST4444);
                let px = [w0 as u16, (w0 >> 16) as u16, w1 as u16, (w1 >> 16) as u16];
                log!(
                    "B6c ARGB4444 dest {name} ({d:04x}), red at alpha 0, 1/4, 1/2, 1:{}",
                    Px(&px)
                );
            } else {
                ok = false;
            }
        }

        // B6d: a 32 bpp screen of four smooth ramps (grey, red, green,
        // blue; 256 levels over 480 px), each one CPU row and one stride-0
        // copy; 32 bpp full-screen fills timed.
        for (k, c) in [0x00FF_FFFFu32, 0x00FF_0000, 0x0000_FF00, 0x0000_00FF]
            .into_iter()
            .enumerate()
        {
            for x in 0..480u32 {
                let level = x * 255 / 479;
                let ch = |sh: u32| (((c >> sh) & 0xFF) * level / 255) << sh;
                let _ = store32_volatile(
                    vram,
                    ROW + 4 * x as usize,
                    0xFF00_0000 | ch(16) | ch(8) | ch(0),
                );
            }
            cpu::drain_write_buffer();
            let y = 160 * k as u16;
            let (w, n) = rp_image::need(
                cmd::blit(
                    (Format::BPP32, SCREEN32, STRIDE32 as i32),
                    (Format::BPP32, ROW, 0),
                    (0, 0),
                    (0, y, 480, y + 160),
                    ROP_SRCCOPY,
                    0,
                    0,
                ),
                "a 2D rectangle",
            );
            ok &= go(e, vram, &w[..n], "B6 ramp");
        }
        let fill32 = |c| {
            rp_image::need(
                cmd::fill(Format::BPP32, SCREEN32B, STRIDE32, (0, 0, 480, 640), c),
                "a 2D rectangle",
            )
        };
        let t0 = rt::ticks();
        for i in 0..10 {
            let _ = e.submit(&fill32(0xFF00_0000 | i));
        }
        let fills = e.finish(vram, 1_000_000).map(|_| us_since(t0) / 10);
        log!("B6d 32 bpp full-screen fill: {fills:?} us each");

        // Show it (opaque, 32 bpp), keeping the registers as found.
        let plane = Plane {
            format: PixelFormat::Argb8888,
            width: 480,
            height: 640,
            offset: SCREEN32,
            stride: STRIDE32,
        };
        let regs32 = plane.registers();
        let saved = regs32.map(|(off, _)| (off, e.regs.read32(off)));
        let gamma = e.regs.read32(dreg::GSCTRL) & (1 << 26);
        for (off, v) in regs32 {
            let v = match off {
                dreg::GSCTRL => v | gamma,
                dreg::GSCADR => Layer::SOLO.scadr(v),
                dreg::GBBASE => Layer::SOLO.bbase(v),
                _ => v,
            };
            e.regs.write32(off, v);
        }
        // Scan-out of 32 bpp with the engine busy: 10 fills and 10 copies of
        // the off-screen 32 bpp screen, the flags cleared first.
        rt::delay_us(40_000);
        let found = rp_drv_mbx_display::ack_events(&mut e.regs, event::UNDER_OVERRUN)
            & event::UNDER_OVERRUN;
        let t0 = rt::ticks();
        for i in 0..10u32 {
            let _ = e.submit(&fill32(0xFF10_1010 * (i & 1)));
            let (w, n) = rp_image::need(
                cmd::copy(
                    Format::BPP32,
                    (SCREEN32B, STRIDE32),
                    (SCREEN32B + STRIDE32 * 320, STRIDE32),
                    (0, 0),
                    (0, 0, 480, 320),
                    0,
                ),
                "a 2D rectangle",
            );
            let _ = e.submit(&w[..n]);
        }
        let busy = e.finish(vram, 2_000_000).map(|_| us_since(t0));
        rt::delay_us(40_000);
        let after = e.regs.read32(DINTRS) & event::UNDER_OVERRUN;
        log!(
            "B6d 32 bpp screen on; 10 full fills + 10 half copies at 32 bpp: {busy:?} us; under/overrun bits 0x{found:08x} as found, 0x{after:08x} after"
        );
        ok &= busy.is_ok() && fills.is_ok();
        log!("B6d the screen shows four 32 bpp ramps: grey, red, green, blue");
        (ok, Some(saved))
    }

    // ------------------------------------------------------------------ B7

    // B7's local memory, clear of the loader's plane (below 1 MiB) and the
    // fence. The ramp's destination sits between two guard bands.
    const B7_RAMP: usize = 0x00C8_0000;
    const B7_RAMP_STRIDE: usize = RAMP * 4;
    const B7_GUARD: usize = 0x4000;
    const B7_DST: usize = 0x00C9_0000 + B7_GUARD;
    const B7_DST_STRIDE: usize = RAMP * 2;
    const B7_GUARD_WORD: u32 = 0x5A5A_5A5A;
    /// Three 256-pixel rows (red, green, blue ramps), 32 and 16 bits.
    const B7_ROWS: usize = 0x00CA_0000;
    const B7_ROWS16: usize = 0x00CA_1000;
    /// Full-screen pictures for the timings.
    const B7_SRC32: usize = 0x0040_0000;
    const B7_SCREEN16: [usize; 2] = [0x0060_0000, 0x006A_0000];
    const B7_PATTERN: usize = 0x00CB_0000;

    /// B7 alone (`RP_MBX_BLEND_B7`): no events, no B1-B6, nothing shown,
    /// no button; back to the loader.
    fn b7_only(regs: Mmio, vram: &mut [u8]) -> ! {
        log!(
            "{MARKER}: B7 only (ADR-0023's 16-bit screen: the 2D engine's 32 to 16-bit conversion)"
        );
        let mut e = Engine::new(regs, FENCE, rt::ticks, TICKS_PER_MS);
        log!(
            "B0 engine: ID 0x{:08x}",
            e.regs.read32(cmd::MBX + cmd::reg::ID)
        );
        e.reset(rt::delay_us);
        let _ = store32_volatile(vram, FENCE, 0);
        cpu::drain_write_buffer();
        let ok = b7(&mut e, vram);
        let verdict = if ok { "PASS" } else { "FAIL (see the log)" };
        log!("{MARKER}: {verdict} - B7 done");
        if let Some(bus) = rt::take_ffuart() {
            let mut uart = PxaUart::new(bus);
            uart.init(115_200);
            let _ = writeln!(uart, "{MARKER}: {verdict}");
        }
        rp_image::finish()
    }

    /// A 16-bit pixel at `at`.
    fn px16_at(vram: &[u8], at: usize) -> u16 {
        load32_volatile(vram, at & !3).map_or(0xDEAD, |w| (w >> (8 * (at & 2))) as u16)
    }

    /// Fills `words` words from `at` with `v` (CPU writes).
    fn fill_words(vram: &mut [u8], at: usize, words: usize, v: u32) {
        for k in 0..words {
            let _ = store32_volatile(vram, at + 4 * k, v);
        }
        cpu::drain_write_buffer();
    }

    /// Guard words around B7's ramp destination that changed.
    fn guards_changed(vram: &[u8]) -> usize {
        let before = B7_DST - B7_GUARD;
        let after = B7_DST + RAMP * B7_DST_STRIDE;
        [before, after]
            .iter()
            .map(|&at| {
                (0..B7_GUARD / 4)
                    .filter(|k| load32_volatile(vram, at + 4 * k) != Some(B7_GUARD_WORD))
                    .count()
            })
            .sum()
    }

    /// Up to four levels, for the log.
    struct Levels([u32; 4], usize);
    impl core::fmt::Display for Levels {
        fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
            if self.1 == 0 {
                return f.write_str("none");
            }
            for (i, v) in self.0[..self.1].iter().enumerate() {
                write!(f, "{}{v}", if i == 0 { "" } else { " " })?;
            }
            Ok(())
        }
    }

    /// B7a/b: three 256-pixel rows (a ramp of red, green, blue; `alpha` in
    /// the top byte) copied into RGB565 with `flags` (and `word` as the
    /// blend state first), each channel's results against the [`Narrow`]
    /// rules applied to `level(v)` (what an exact engine would narrow).
    fn b7_rows(
        e: &mut Engine<Mmio>,
        vram: &mut [u8],
        label: &str,
        (alpha, flags, word): (u32, u32, Option<u32>),
        level: fn(u32) -> u32,
    ) -> bool {
        const NAMES: [&str; 3] = ["red", "green", "blue"];
        const BITS: [u32; 3] = [5, 6, 5];
        for (k, sh) in [16u32, 8, 0].into_iter().enumerate() {
            for v in 0..256u32 {
                let _ = store32_volatile(
                    vram,
                    B7_ROWS + k * 1024 + 4 * v as usize,
                    alpha << 24 | v << sh,
                );
            }
        }
        fill_words(vram, B7_ROWS16, 3 * 128, 0);
        let mut words = [0u32; 12];
        let mut n = 0;
        if let Some(b) = word {
            words[..2].copy_from_slice(&cmd::blend(b));
            n = 2;
        }
        let (bw, bn) = rp_image::need(
            cmd::blit(
                (Format::RGB565, B7_ROWS16, 512),
                (Format::BPP32, B7_ROWS, 1024),
                (0, 0),
                (0, 0, 256, 3),
                ROP_SRCCOPY,
                flags,
                0,
            ),
            "a 2D rectangle",
        );
        words[n..n + bn].copy_from_slice(&bw[..bn]);
        if !go(e, vram, &words[..n + bn], label) {
            return false;
        }
        for k in 0..3 {
            let px = |v: u32| channels(px16_at(vram, B7_ROWS16 + k * 512 + 2 * v as usize));
            let got = |v: u32| px(v)[k];
            let others = (0..256u32)
                .filter(|&v| (0..3).any(|j| j != k && px(v)[j] != 0))
                .count();
            let c = rule_counts(BITS[k], level, got);
            let mut odd = Levels([0; 4], 0);
            for v in 0..256u32 {
                if odd.1 < 4
                    && Narrow::ALL
                        .iter()
                        .all(|r| got(v) != r.apply(level(v), BITS[k]))
                {
                    odd.0[odd.1] = v;
                    odd.1 += 1;
                }
            }
            log!(
                "{label} {}: of 256 levels {} {}, {} {}, {} {}; no rule's: {odd}; other channels written: {others}; levels 4 8 12 128 252 255 gave {} {} {} {} {} {}",
                NAMES[k],
                Narrow::ALL[0].name(),
                c[0],
                Narrow::ALL[1].name(),
                c[1],
                Narrow::ALL[2].name(),
                c[2],
                got(4),
                got(8),
                got(12),
                got(128),
                got(252),
                got(255),
            );
        }
        true
    }

    /// B7c: one try at a 2D dither: the 64 x 64 grey ramp converted to
    /// RGB565 with header `flags` (and blend `word`), into a destination
    /// marked [`MARK16`] between guard bands: what came out and how many
    /// guard words changed, or `None` if the engine stuck (reset, logged).
    fn b7_ramp_try(
        e: &mut Engine<Mmio>,
        vram: &mut [u8],
        (flags, word): (u32, Option<u32>),
    ) -> Option<(RampStats, usize)> {
        fill_words(vram, B7_DST - B7_GUARD, B7_GUARD / 4, B7_GUARD_WORD);
        fill_words(
            vram,
            B7_DST,
            RAMP * B7_DST_STRIDE / 4,
            u32::from(MARK16) * 0x0001_0001,
        );
        fill_words(
            vram,
            B7_DST + RAMP * B7_DST_STRIDE,
            B7_GUARD / 4,
            B7_GUARD_WORD,
        );
        let mut words = [0u32; 12];
        let mut n = 0;
        if let Some(b) = word {
            words[..2].copy_from_slice(&cmd::blend(b));
            n = 2;
        }
        let (bw, bn) = rp_image::need(
            cmd::blit(
                (Format::RGB565, B7_DST, B7_DST_STRIDE as i32),
                (Format::BPP32, B7_RAMP, B7_RAMP_STRIDE as i32),
                (0, 0),
                (0, 0, RAMP as u16, RAMP as u16),
                ROP_SRCCOPY,
                flags,
                0,
            ),
            "a 2D rectangle",
        );
        words[n..n + bn].copy_from_slice(&bw[..bn]);
        if !go(e, vram, &words[..n + bn], "B7c try") {
            return None;
        }
        let mut st = RampStats::default();
        for y in 0..RAMP {
            for x in 0..RAMP {
                st.add(
                    ramp_level(x, y),
                    px16_at(vram, B7_DST + y * B7_DST_STRIDE + 2 * x),
                );
            }
        }
        Some((st, guards_changed(vram)))
    }

    /// The 4 x 4 places (bit `4 * (y % 4) + x % 4`) where ramp level
    /// `level` came out one step up in red, from the destination as left.
    fn up_places(vram: &[u8], level: u32) -> u16 {
        let mut m = 0u16;
        for y in 0..RAMP {
            for x in 0..RAMP {
                let r = channels(px16_at(vram, B7_DST + y * B7_DST_STRIDE + 2 * x))[0];
                if ramp_level(x, y) == level && r == Narrow::Truncate.apply(level, 5) + 1 {
                    m |= 1 << (4 * (y % 4) + x % 4);
                }
            }
        }
        m
    }

    /// Times `count` repeats of `words` queued back to back: microseconds
    /// each, or `None` if the engine stuck (reset, logged).
    fn b7_time(
        e: &mut Engine<Mmio>,
        vram: &[u8],
        words: &[u32],
        count: u32,
        what: &str,
    ) -> Option<u32> {
        let t0 = rt::ticks();
        let done = (0..count)
            .try_for_each(|_| e.submit(words))
            .and_then(|()| e.finish(vram, 2_000_000));
        match done {
            Ok(_) => Some(us_since(t0) / count),
            Err(why) => {
                log!("B7d {what}: {why}: engine reset");
                e.reset(rt::delay_us);
                None
            }
        }
    }

    /// B7 (ADR-0023's 16-bit screen; measured, not judged): B7a the
    /// 32 to 16-bit conversion copy per channel; B7b blends onto RGB565;
    /// B7c the untried bits on a converted ramp (a 2D dither?); B7d timings.
    /// False only if a part couldn't run or the blend state wasn't put back.
    fn b7(e: &mut Engine<Mmio>, vram: &mut [u8]) -> bool {
        let mut base = [0u32; 6];
        base[0] = cmd::SCALE_1_TO_1;
        base[1..4].copy_from_slice(&cmd::key(0, 0));
        base[4..].copy_from_slice(&cmd::blend(0));
        let mut ok = go(e, vram, &base, "B7 baseline");

        // B7a: the plain conversion copy (no bit 17).
        ok &= b7_rows(e, vram, "B7a convert", (0xFF, 0, None), |v| v);
        rt::check_in();
        // B7b: blends onto RGB565 with Dell's "over": opaque, then alpha
        // 0x80 over black, against the exact result rounded to 8 bits.
        let over = (OP_BLEND, Some(BLEND_DELL_TEXT));
        ok &= b7_rows(
            e,
            vram,
            "B7b over alpha 0xff",
            (0xFF, over.0, over.1),
            |v| v,
        );
        ok &= b7_rows(
            e,
            vram,
            "B7b over alpha 0x80",
            (0x80, over.0, over.1),
            |v| (v * 0x80 + 127) / 255,
        );
        ok &= go(e, vram, &cmd::blend(0), "B7b blend back");
        rt::check_in();

        // B7c: the ramp, then each try.
        for y in 0..RAMP {
            for x in 0..RAMP {
                let l = ramp_level(x, y);
                let _ = store32_volatile(
                    vram,
                    B7_RAMP + y * B7_RAMP_STRIDE + 4 * x,
                    0xFF00_0000 | l * 0x0001_0101,
                );
            }
        }
        cpu::drain_write_buffer();
        let mut in_a_row = 0;
        let mut plain: Option<RampStats> = None;
        for (flags, word) in dither_tries() {
            let (what, val) = match word {
                None => ("header bit", flags),
                Some(w) => ("blend word", w),
            };
            log!(
                "B7c try: {what} 0x{val:08x} (header 0x{:08x})",
                0x8000_0000 | ROP_SRCCOPY | flags
            );
            let Some((st, outside)) = b7_ramp_try(e, vram, (flags, word)) else {
                in_a_row += 1;
                let back = go(e, vram, &base, "B7c baseline after a reset");
                log!(
                    "B7c {what} 0x{val:08x}: STUCK; after the reset: {}",
                    if back { "runs" } else { "STILL STUCK" }
                );
                if in_a_row >= 3 || !back {
                    log!("B7c: three stuck in a row (or not back): the tries end here");
                    ok = false;
                    break;
                }
                continue;
            };
            in_a_row = 0;
            let same = plain.is_some_and(|p| p == st);
            plain.get_or_insert(st);
            // Level 68 is half-way between two 5-bit steps (64, 72): a
            // dither raises it at about half the places, which are listed.
            let places = up_places(vram, 68);
            log!(
                "B7c {what} 0x{val:08x}: trunc {} up {} other {} unwritten {}; mixed levels {}; level 68 red up at places 0x{places:04x}; guard words changed {outside}; {}{}",
                st.trunc,
                st.up,
                st.other,
                st.unwritten,
                st.mixed_levels(),
                st.reading(),
                if same {
                    " (as the plain conversion)"
                } else {
                    ""
                },
            );
            if word.is_some() {
                ok &= go(e, vram, &cmd::blend(0), "B7c blend back");
            }
            rt::check_in();
        }

        // B7d: timings, 5 each. The 32-bit picture is made first (a
        // CPU-written row repeated with stride 0).
        let row: [u32; 16] = core::array::from_fn(|k| 0xFF00_0000 | k as u32 * 0x000F_0F0F);
        set(vram, B7_PATTERN, &row);
        let blit = |dst: (Format, usize, i32), src: (Format, usize, i32), r, flags| {
            rp_image::need(
                cmd::blit(dst, src, (0, 0), r, ROP_SRCCOPY, flags, 0),
                "a 2D rectangle",
            )
        };
        let full = (0, 0, 480, 640);
        let band = (0, 0, 480, 80);
        let s32 = (Format::BPP32, B7_SRC32, 1920);
        let half32 = (Format::BPP32, B7_SRC32 + 1920 * 320, 1920);
        let d16 = (Format::RGB565, B7_SCREEN16[0], 960);
        let d16b = (Format::RGB565, B7_SCREEN16[1], 960);
        let pattern = (Format::BPP32, B7_PATTERN, 0);
        for (what, (w, n)) in [
            (
                "make the 32-bit picture (stride-0 copy), full screen",
                blit(s32, pattern, full, 0),
            ),
            (
                "convert 32 to 16 bits, full screen",
                blit(d16, s32, full, 0),
            ),
            ("convert 32 to 16 bits, 480 x 80", blit(d16, s32, band, 0)),
            (
                "copy 32 to 32 bits, half screen (B6's control)",
                blit(half32, s32, (0, 0, 480, 320), 0),
            ),
            (
                "copy 16 to 16 bits (a forward copy), full screen",
                blit(d16b, d16, full, 0),
            ),
            ("copy 16 to 16 bits, 480 x 80", blit(d16b, d16, band, 0)),
        ] {
            let us = b7_time(e, vram, &w[..n], 5, what);
            log!("B7d {what}: {us:?} us each");
            ok &= us.is_some();
            rt::check_in();
        }
        // The pattern add as the renderer does it: four blits (stride x 4,
        // the pattern row with stride 0), blend word 0 (add, saturating).
        let mut add = [0u32; 2 + 4 * 10];
        add[..2].copy_from_slice(&cmd::blend(0));
        let mut n = 2;
        for ph in 0..4usize {
            let (w, k) = blit(
                (Format::BPP32, B7_SRC32 + 1920 * ph, 1920 * 4),
                pattern,
                (0, 0, 480, 160),
                OP_BLEND,
            );
            add[n..n + k].copy_from_slice(&w[..k]);
            n += k;
        }
        let us = b7_time(e, vram, &add[..n], 5, "dither add");
        log!("B7d dither add at 32 bits (4 blits, blend 0), full screen: {us:?} us each");
        ok &= us.is_some();

        let back = go(e, vram, &base, "B7 blend back");
        log!(
            "B7 blend state put back to 0: {}",
            if back { "yes" } else { "NO" }
        );
        ok && back
    }

    /// The keypad, enabled with earlier activity discarded: taken before
    /// the image says it is ready (the verdict on the serial port), so a
    /// press that follows the verdict at once can't fall between the two
    /// (the emulator check presses at once; CI lost the press, 2026-10-14).
    fn arm_keypad() -> Option<Keypad<Mmio>> {
        let Some(kp) = rt::take_keypad() else {
            log!("no keypad: returning");
            return None;
        };
        let mut keypad = Keypad::new(kp);
        keypad.enable();
        keypad.acknowledge();
        Some(keypad)
    }

    /// Keeps the picture up until a button is pressed and released (the
    /// release too, so the press doesn't reach the loader), checking in with
    /// the loader's watchdog; gives up after 5 minutes.
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
    fn narrow_rules_at_their_edges() {
        // 5 bits: a step is 8. 7 -> 0 truncated, 1 rounded, 1 scaled
        // (7 * 31 / 255 = 0.85); 255 stays the top under every rule.
        assert_eq!(Narrow::Truncate.apply(7, 5), 0);
        assert_eq!(Narrow::Round.apply(7, 5), 1);
        assert_eq!(Narrow::Round.apply(3, 5), 0);
        assert_eq!(Narrow::Round.apply(4, 5), 1);
        assert_eq!(Narrow::Scale.apply(7, 5), 1);
        for rule in Narrow::ALL {
            assert_eq!(rule.apply(0, 5), 0);
            assert_eq!(rule.apply(255, 5), 31);
            assert_eq!(rule.apply(255, 6), 63);
        }
        // Round never passes the top (252 + 4 = 256 -> 32 held at 31).
        assert_eq!(Narrow::Round.apply(252, 5), 31);
    }

    #[test]
    fn rule_counts_tell_the_rules_apart() {
        for (k, rule) in Narrow::ALL.into_iter().enumerate() {
            for bits in [5, 6] {
                let c = rule_counts(bits, |v| v, |v| rule.apply(v, bits));
                assert_eq!(c[k], 256, "{rule:?} {bits}");
                // The others differ somewhere, so a count names one rule.
                for (j, &n) in c.iter().enumerate() {
                    if j != k {
                        assert!(n < 256, "{rule:?} vs {j} at {bits} bits");
                    }
                }
            }
        }
    }

    #[test]
    fn ramp_covers_every_level_at_every_place() {
        let mut seen = [[0u16; 64]; 16];
        for y in 0..RAMP {
            for x in 0..RAMP {
                let l = ramp_level(x, y);
                assert!((64..128).contains(&l));
                seen[(y % 4) * 4 + x % 4][(l - 64) as usize] += 1;
            }
        }
        assert!(seen.iter().flatten().all(|&n| n == 4));
        // The marker isn't any level's conversion (red 27 > 15).
        assert!(channels(MARK16)[0] > Narrow::Truncate.apply(127, 5));
    }

    fn rgb(r: u32, g: u32, b: u32) -> u16 {
        (r << 11 | g << 5 | b) as u16
    }

    #[test]
    fn ramp_stats_readings() {
        let t = |l: u32| rgb(l >> 3, l >> 2, l >> 3);
        // Plain truncation.
        let mut s = RampStats::default();
        (64..128).for_each(|l| s.add(l, t(l)));
        assert_eq!((s.trunc, s.up, s.mixed_levels()), (64, 0, 0));
        assert_eq!(s.reading(), "every pixel truncated");
        // Rounding: one answer a level.
        let mut s = RampStats::default();
        for l in 64..128 {
            let p = rgb(
                Narrow::Round.apply(l, 5),
                Narrow::Round.apply(l, 6),
                Narrow::Round.apply(l, 5),
            );
            s.add(l, p);
            s.add(l, p);
        }
        assert_eq!(s.mixed_levels(), 0);
        assert_eq!(s.reading(), "one answer a level, some up: rounding-like");
        // A dither: level 65 truncated at one place, up at another.
        let mut s = RampStats::default();
        s.add(65, t(65));
        s.add(65, rgb(9, 17, 9));
        assert_eq!((s.mixed_levels(), s.up), (1, 1));
        assert_eq!(s.reading(), "position-dependent: dither-like");
        // Unwritten and wild pixels.
        let mut s = RampStats::default();
        s.add(70, MARK16);
        s.add(70, rgb(0, 0, 0));
        assert_eq!((s.unwritten, s.other), (1, 1));
        assert_eq!(s.reading(), "something else (see the counts)");
    }

    #[test]
    fn dither_tries_cover_the_untried_bits_once() {
        let t: std::vec::Vec<(u32, Option<u32>)> = dither_tries().collect();
        assert_eq!(t.len(), 1 + 4 + 1 + 4 + 20);
        assert_eq!(t[0], (0, None));
        for b in [19, 21, 22, 27] {
            assert!(t.contains(&(1 << b, None)));
        }
        assert_eq!(t[5], (rp_drv_blit2d::OP_BLEND, Some(0x0200_0000)));
        for b in (28..32).chain(0..20) {
            let w = 0x0200_0000 | 1 << b;
            assert_eq!(t.iter().filter(|&&x| x.1 == Some(w)).count(), 1, "bit {b}");
        }
        // None touches a bit already known (16-18, 20, 23-26) or the
        // blend word's factor nibbles.
        let known = 0x07 << 16 | 1 << 20 | 0xF << 23;
        assert!(
            t.iter()
                .all(|&(h, _)| h & known & !rp_drv_blit2d::OP_BLEND == 0)
        );
        assert!(
            t.iter()
                .filter_map(|&(_, w)| w)
                .all(|w| w & 0x0FF0_0000 == 0x0200_0000)
        );
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
