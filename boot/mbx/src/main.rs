//! Test image (item 69): the first GPU-drawn rectangle, through the MBX
//! core's 2D engine, under the loader.
//!
//! The static analysis of Dell's driver (docs/hw/2700g.md, "The 2D engine")
//! found that Windows Mobile's fills are 2D commands: words stored one after
//! another into a 4 KiB command window at `0x0FFF5000` (inside the register
//! window RetroPane already maps), the free space read from the MBX's status
//! register `0x12c`, and completion learnt by a "fence": a 1 × 1 fill of a
//! counter into a word of local memory, polled by the CPU. This image checks
//! all of it, step by step, each step named in the run log before it runs
//! (a wrong address in the 2700G's register block hangs the bus):
//!
//! 1. the core's ID (`0xf00`, known), the status register, the driver's reset;
//! 2. a fence alone: does the engine run commands at all?
//! 3. a red rectangle on a black screen, its edges read back (is the end
//!    exclusive, is the colour the low 16 bits);
//! 4. 20 full-screen fills by the GPU, then 5 by the CPU, both timed (the
//!    CPU's 14.5 ms is in docs/perf.md);
//! 5. three bars and a frame, all GPU-drawn, kept on screen until a button
//!    is pressed (for a photo).
//!
//! Every wait has a timeout; on a failure the image logs it and returns.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed with the verdict.
pub const MARKER: &str = "RETROPANE-MBX-2D";

/// The MBX core's registers in the 2700G register window (`0x0FFE0000`).
pub const MBX: usize = 0x4000;
/// MBX register offsets used here (all ones the driver itself touches).
pub mod reg {
    /// Core ID (`0x01020000` on the X51v).
    pub const ID: usize = 0xf00;
    /// Reset control: `0x111` asserts, 0 releases.
    pub const RESET: usize = 0x080;
    /// Interrupt status; bits 16-23 and 30 are also the command FIFO's level.
    pub const STATUS: usize = 0x12c;
    /// Interrupt enables.
    pub const ENABLE: usize = 0x130;
    /// Interrupt clear (write 1s).
    pub const CLEAR: usize = 0x134;
}
/// The 2D command window (`0x0FFF5000`) in the register window: 4 KiB,
/// written sequentially, wrapping.
pub const PORT: usize = 0x1_5000;
/// The command window's size.
pub const PORT_SIZE: usize = 0x1000;

/// Where the fence word lives: the last 4 KiB of the 2700G's 16 MiB, far
/// from every plane RetroPane uses.
pub const FENCE: usize = 0x00FF_F000;

/// The MBX 2D engine's command words (docs/hw/2700g.md, "The 2D engine").
pub mod cmd {
    /// Destination formats (the driver's table).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Format {
        /// RGB 5:6:5.
        Rgb565 = 0x5_0000,
        /// 32 bits per pixel.
        Bpp32 = 0x6_0000,
    }
    /// Pattern copy: fill with the colour.
    pub const ROP_PATCOPY: u32 = 0xF0F0;
    /// Ends a batch of commands (not a completion signal).
    pub const FLUSH: u32 = 0x7000_0000;
    /// The FIFO's capacity in words.
    pub const FIFO_WORDS: u32 = 0x1FE;

    /// A solid fill of `[x0, x1) × [y0, y1)` with `color` into the surface
    /// at local-memory offset `addr`, `stride` bytes per line: seven words.
    #[must_use]
    pub fn fill(
        format: Format,
        addr: usize,
        stride: usize,
        (x0, y0, x1, y1): (u16, u16, u16, u16),
        color: u32,
    ) -> [u32; 7] {
        [
            0xA000_0000 | format as u32 | (stride as u32 & 0x7FFF),
            addr as u32 & 0x03FF_FFFC,
            0x8000_0000 | ROP_PATCOPY,
            color,
            xy(x0, y0),
            xy(x1, y1),
            FLUSH,
        ]
    }

    /// Source copy.
    pub const ROP_SRCCOPY: u32 = 0xCCCC;
    /// Operation header bits: no colour word follows; the colour key applies.
    pub const OP_NO_COLOUR: u32 = 1 << 16;
    /// The colour key applies (with [`OP_NO_COLOUR`], as the driver sends it).
    pub const OP_KEY: u32 = 1 << 20;
    // Bit 23, the driver's "y not positive", is deliberately absent: it
    // doesn't make the chip copy bottom-up (measured 2026-10-14). A copy
    // down over itself is one copy with a negative stride on both surfaces
    // (`Down::NegativeStride`), or strips as the fallback (see `copies`).
    /// Copy right to left (for a destination right of an overlapping source).
    pub const OP_RIGHT_LEFT: u32 = 1 << 24;
    /// No scaling: the driver sends it once at start-up.
    pub const SCALE_1_TO_1: u32 = 0x6080_0200;

    /// The colour key: source pixels whose bits under `mask` equal `key`
    /// leave the destination as it was (with [`OP_KEY`]). `key(0, 0)` with no
    /// `OP_KEY` is the driver's "key off".
    #[must_use]
    pub fn key(key: u32, mask: u32) -> [u32; 3] {
        [0x2000_0001, key, mask]
    }

    /// A copy of the `[x0, x1) × [y0, y1)` destination rectangle from the
    /// source surface at `src` (`src_stride`), starting at `(sx, sy)` there.
    /// `flags`: [`OP_RIGHT_LEFT`] for a rightward overlap, or
    /// [`OP_KEY`]. Ten words (nine keyed: no colour word).
    #[must_use]
    pub fn copy(
        format: Format,
        (dst, dst_stride): (usize, usize),
        (src, src_stride): (usize, usize),
        (sx, sy): (u16, u16),
        (x0, y0, x1, y1): (u16, u16, u16, u16),
        flags: u32,
    ) -> ([u32; 10], usize) {
        let keyed = flags & OP_KEY != 0;
        let op = 0x8000_0000 | ROP_SRCCOPY | flags | if keyed { OP_NO_COLOUR } else { 0 };
        let mut w = [
            0xA000_0000 | format as u32 | (dst_stride as u32 & 0x7FFF),
            dst as u32 & 0x03FF_FFFC,
            0x9400_0000 | format as u32 | (src_stride as u32 & 0x7FFF),
            src as u32 & 0x03FF_FFFC,
            0x3000_0000 | u32::from(sx & 0x1FFF) << 14 | u32::from(sy & 0x1FFF),
            op,
            0, // the colour word the driver always sends (ignored for a copy)
            xy(x0, y0),
            xy(x1, y1),
            FLUSH,
        ];
        if keyed {
            w.copy_within(7.., 6);
            return (w, 9);
        }
        (w, 10)
    }

    /// A coordinate word: 13-bit x and y.
    #[must_use]
    pub fn xy(x: u16, y: u16) -> u32 {
        u32::from(x & 0x1FFF) << 16 | u32::from(y & 0x1FFF)
    }

    /// The fence: writes `n` into the 32-bit word at local-memory `addr`.
    #[must_use]
    pub fn fence(addr: usize, n: u32) -> [u32; 7] {
        fill(Format::Bpp32, addr, 4, (0, 0, 1, 1), n)
    }

    /// Free bytes in the command FIFO, from the status register.
    #[must_use]
    pub fn free_bytes(status: u32) -> u32 {
        let used = ((status & 0x00FF_0000) | ((status >> 6) & 0x0100_0000)) >> 16;
        FIFO_WORDS.saturating_sub(used) * 4
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn fill_words() {
            let w = fill(Format::Rgb565, 0x6_0000, 960, (40, 120, 440, 320), 0xF800);
            assert_eq!(
                w,
                [
                    0xA005_03C0,
                    0x0006_0000,
                    0x8000_F0F0,
                    0xF800,
                    0x0028_0078,
                    0x01B8_0140,
                    0x7000_0000
                ]
            );
        }

        #[test]
        fn fence_is_one_pixel_of_32_bits() {
            let w = fence(super::super::FENCE, 7);
            assert_eq!(w[0], 0xA006_0004);
            assert_eq!(w[1], 0x00FF_F000);
            assert_eq!((w[3], w[4], w[5]), (7, 0, 0x0001_0001));
        }

        #[test]
        fn fifo_level() {
            assert_eq!(free_bytes(0), 0x1FE * 4);
            // WM's fills showed 0x00950000: 0x95 words queued.
            assert_eq!(free_bytes(0x0095_0000), (0x1FE - 0x95) * 4);
            // Bit 30 is the level's ninth bit; other bits are ignored.
            assert_eq!(free_bytes(0x4000_0004), (0x1FE - 0x100) * 4);
            assert_eq!(free_bytes(0x40FF_0000), 0);
        }

        #[test]
        fn copy_words() {
            let (w, n) = copy(
                Format::Rgb565,
                (0, 960),
                (0x80_0000, 128),
                (3, 5),
                (40, 360, 104, 424),
                OP_RIGHT_LEFT,
            );
            assert_eq!(
                w[..n],
                [
                    0xA005_03C0,
                    0,
                    0x9405_0080,
                    0x0080_0000,
                    0x3000_C005,
                    0x8100_CCCC,
                    0,
                    0x0028_0168,
                    0x0068_01A8,
                    0x7000_0000
                ]
            );
        }

        #[test]
        fn keyed_copy_has_no_colour_word() {
            let (w, n) = copy(
                Format::Rgb565,
                (0, 960),
                (0, 960),
                (0, 0),
                (0, 0, 1, 1),
                OP_KEY,
            );
            assert_eq!(n, 9);
            assert_eq!(w[5], 0x8011_CCCC, "the driver's keyed header");
            assert_eq!(w[6..9], [0, 0x0001_0001, 0x7000_0000]);
        }

        #[test]
        fn coordinates_are_13_bits() {
            assert_eq!(xy(0x2001, 0xFFFF), 0x0001_1FFF);
        }
    }
}

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_mbx_display::{Display, fill_rect_rgb565};
    use rp_drv_pxa_keypad::{Keypad, Pressed};
    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_api::{DmaEngine, RegisterBus};
    use rp_hal_mem::{load32_volatile, store32_volatile};
    use rp_hal_pxa27x::{Mmio, cpu, rt};
    use rp_hal_x51v::devices::RunLog;

    use super::{FENCE, MARKER, MBX, PORT, PORT_SIZE, cmd, reg};

    rp_hal_pxa27x::entry!(main);

    /// Microseconds since `t0` (OSCR0 runs at 3.25 MHz).
    fn us_since(t0: u32) -> u32 {
        rt::ticks().wrapping_sub(t0) / 13 * 4
    }

    macro_rules! log {
        ($($t:tt)*) => {{ let _ = writeln!(RunLog, $($t)*); }};
    }

    /// The 2D engine: the register window and the command window's write
    /// offset.
    struct Engine {
        regs: Mmio,
        at: usize,
        fence: u32,
    }

    impl Engine {
        fn status(&mut self) -> u32 {
            self.regs.read32(MBX + reg::STATUS)
        }

        /// Queues `words`, waiting (up to 100 ms) for room in the FIFO.
        fn submit(&mut self, words: &[u32]) -> Result<(), u32> {
            let need = 4 * words.len() as u32;
            let t0 = rt::ticks();
            loop {
                let s = self.status();
                if cmd::free_bytes(s) >= need {
                    break;
                }
                if us_since(t0) > 100_000 {
                    return Err(s);
                }
            }
            for &w in words {
                self.regs.write32(PORT + self.at, w);
                self.at = (self.at + 4) % PORT_SIZE;
            }
            Ok(())
        }

        /// Queues a fence and waits (up to `limit_us`) for the engine to
        /// write it: everything queued before it is done. Returns the wait.
        fn finish(&mut self, vram: &mut [u8], limit_us: u32) -> Result<u32, &'static str> {
            self.fence += 1;
            let n = self.fence;
            self.submit(&cmd::fence(FENCE, n))
                .map_err(|_| "FIFO full")?;
            let t0 = rt::ticks();
            loop {
                if load32_volatile(vram, FENCE) == Some(n) {
                    return Ok(us_since(t0));
                }
                if us_since(t0) > limit_us {
                    return Err("fence never written");
                }
            }
        }
    }

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let Some((regs, vram)) = rp_hal_x51v::devices::take_mbx() else {
            crash(format_args!("no 2700G"))
        };
        let mut display = Display::new(regs);
        let (plane, _) = display.adopt(vram.len());
        let regs = display.into_inner();
        let (a, s) = (plane.offset, plane.stride);
        let mut e = Engine {
            regs,
            at: 0,
            fence: 0,
        };

        log!("MBX ID 0xf00");
        log!("  = 0x{:08x}", e.regs.read32(MBX + reg::ID));
        log!("status 0x12c");
        let st = e.status();
        log!("  = 0x{st:08x} ({} bytes free)", cmd::free_bytes(st));

        // The driver's reset (variant 2), interrupts left masked: this image
        // polls.
        log!("reset: 0x080 = 0x111, 0x130 masked, 0x080 = 0, 0x134 = 0xffff");
        e.regs.write32(MBX + reg::RESET, 0x111);
        rt::delay_us(2_000);
        let en = e.regs.read32(MBX + reg::ENABLE);
        e.regs.write32(MBX + reg::ENABLE, en & !0xFFFF);
        e.regs.write32(MBX + reg::RESET, 0);
        rt::delay_us(2_000);
        e.regs.write32(MBX + reg::CLEAR, 0xFFFF);
        let st = e.status();
        log!("  status 0x{st:08x} ({} bytes free)", cmd::free_bytes(st));

        // 2. A fence alone.
        let _ = store32_volatile(vram, FENCE, 0);
        cpu::drain_write_buffer();
        log!("fence: first command into 0x0FFF5000");
        match e.finish(vram, 50_000) {
            Ok(us) => log!("  fence written after {us} us: the 2D engine runs"),
            Err(why) => {
                let st = e.status();
                fail(format_args!("{why} (status 0x{st:08x})"));
            }
        }

        // 3. A red rectangle on black, edges read back.
        fill_rect_rgb565(vram, s, 0, 0, 480, 640, 0x0000);
        cpu::drain_write_buffer();
        let rect = (40, 120, 440, 320);
        log!("rectangle {rect:?} red 0xf800 at 0x{a:x}, stride {s}");
        let t0 = rt::ticks();
        if e.submit(&cmd::fill(cmd::Format::Rgb565, a, s, rect, 0xF800))
            .is_err()
        {
            fail(format_args!("FIFO full"));
        }
        match e.finish(vram, 50_000) {
            Ok(_) => log!("  drawn in {} us", us_since(t0)),
            Err(why) => fail(format_args!("{why}")),
        }
        let px = |vram: &[u8], x: usize, y: usize| {
            let off = a + y * s + x * 2;
            load32_volatile(vram, off & !3).map(|w| (w >> (8 * (off & 2))) as u16)
        };
        for (x, y, want) in [
            (40, 120, "red"),
            (439, 319, "red"),
            (240, 220, "red"),
            (39, 120, "black"),
            (440, 200, "black (end exclusive?)"),
            (100, 320, "black (end exclusive?)"),
        ] {
            log!("  pixel ({x},{y}) = {:04x?}, expect {want}", px(vram, x, y));
        }
        let ok = px(vram, 240, 220) == Some(0xF800)
            && px(vram, 439, 319) == Some(0xF800)
            && px(vram, 440, 200) == Some(0)
            && px(vram, 100, 320) == Some(0);

        // 3b. Copies: from off-screen memory, overlapping in both
        // directions, colour-keyed; each checked by reading pixels back.
        let copy_ok = copies(&mut e, vram, (a, s), &px);
        let ok = ok && copy_ok;

        // 4. Timing: GPU full-screen fills, then the CPU's.
        let colors: [u16; 4] = [0x001F, 0x07E0, 0xF81F, 0xFFE0];
        let t0 = rt::ticks();
        for i in 0..20 {
            let w = cmd::fill(
                cmd::Format::Rgb565,
                a,
                s,
                (0, 0, 480, 640),
                u32::from(colors[i % 4]),
            );
            if e.submit(&w).is_err() {
                fail(format_args!("FIFO full at fill {i}"));
            }
        }
        let queued = us_since(t0);
        let gpu = match e.finish(vram, 1_000_000) {
            Ok(_) => us_since(t0),
            Err(why) => fail(format_args!("{why}")),
        };
        let t0 = rt::ticks();
        for i in 0..5 {
            fill_rect_rgb565(vram, s, 0, 0, 480, 640, colors[i % 4]);
        }
        cpu::drain_write_buffer();
        let cpu_us = us_since(t0);
        log!(
            "20 GPU full-screen fills: {gpu} us ({} us each; queued in {queued} us)",
            gpu / 20
        );
        log!(
            "5 CPU full-screen fills: {cpu_us} us ({} us each)",
            cpu_us / 5
        );

        // 4b. DMA from SDRAM into 2700G memory, against the CPU (ADR-0022
        // open question 2); then 4c: can the engine read past its own
        // 16 MiB (open question 1)? Last, as it may hang the engine.
        let ok = dma_vs_cpu(vram) && ok;
        probe_beyond_local_memory(&mut e, vram);
        // 4d. Every way to copy down over itself, checked and timed (the
        // owner: the most efficient that works, not the easiest).
        let ok = ways_down(&mut e, vram, (a, s)) && ok;
        // 4e. Does the engine say it's done, and does that reach the CPU
        // (ADR-0022 open question 5)? Informational.
        if let Some(pins) = completion(&mut e, vram, (a, s)) {
            // 4f. The 2700G's interrupt line to the PXA, through the
            // display's frame event. Informational.
            interrupt_line(&mut e, pins);
        }

        // 5. A picture for the photo: black, three bars, a white frame.
        let scene: [((u16, u16, u16, u16), u32); 8] = [
            ((0, 0, 480, 640), 0x0000),
            ((60, 100, 160, 540), 0xF800),
            ((190, 100, 290, 540), 0x07E0),
            ((320, 100, 420, 540), 0x001F),
            ((20, 20, 460, 24), 0xFFFF),
            ((20, 616, 460, 620), 0xFFFF),
            ((20, 20, 24, 620), 0xFFFF),
            ((456, 20, 460, 620), 0xFFFF),
        ];
        for (r, c) in scene {
            if e.submit(&cmd::fill(cmd::Format::Rgb565, a, s, r, c))
                .is_err()
            {
                fail(format_args!("FIFO full"));
            }
        }
        if let Err(why) = e.finish(vram, 50_000) {
            fail(format_args!("{why}"));
        }
        let verdict = if ok {
            "PASS"
        } else {
            "edges differ, see pixels"
        };
        log!("{MARKER}: {verdict} - the picture stays until a button is pressed");
        // The emulator's test waits for this on the serial port (the X51v
        // has none wired: harmless there).
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

    /// Off-screen test tile: 64 × 64 RGB565 at 8 MiB into local memory.
    const TILE: usize = 0x0080_0000;
    const TILE_STRIDE: usize = 128;
    /// The same tile with every other pixel the key colour.
    const KEYED: usize = TILE + 0x2000;
    const KEY: u16 = 0x1234;
    /// A full-screen source for the copy timing.
    const SCREEN_COPY: usize = 0x0090_0000;

    /// The tile's pixel at (x, y): distinct, every bit used, never `KEY`.
    fn tile(x: usize, y: usize) -> u16 {
        ((x * 37 + y * 101 + 1) as u16) | 0x8000
    }

    /// Stage 3b: copies, each checked by reading pixels back.
    fn copies(
        e: &mut Engine,
        vram: &mut [u8],
        (a, s): (usize, usize),
        px: &dyn Fn(&[u8], usize, usize) -> Option<u16>,
    ) -> bool {
        use cmd::{Format::Rgb565, OP_KEY, OP_RIGHT_LEFT};
        for y in 0..64 {
            for x in 0..64 {
                let at = y * TILE_STRIDE + x * 2;
                vram[TILE + at..TILE + at + 2].copy_from_slice(&tile(x, y).to_le_bytes());
                let k = if (x + y) % 2 == 0 { KEY } else { tile(x, y) };
                vram[KEYED + at..KEYED + at + 2].copy_from_slice(&k.to_le_bytes());
            }
        }
        cpu::drain_write_buffer();
        let mut ok = true;
        let mut run =
            |e: &mut Engine, what: &str, words: &[u32], checks: &[(usize, usize, u16)]| {
                log!("{what}");
                if e.submit(words).is_err() {
                    fail(format_args!("FIFO full"));
                }
                if let Err(why) = e.finish(vram, 50_000) {
                    fail(format_args!("{why}"));
                }
                for &(x, y, want) in checks {
                    let got = px(vram, x, y);
                    let fine = got == Some(want);
                    ok &= fine;
                    log!(
                        "  pixel ({x},{y}) = {got:04x?}, expect {want:04x}{}",
                        if fine { "" } else { "  <-- WRONG" }
                    );
                }
            };
        // The driver's start-up state: no scaling, no key.
        let mut init = [0u32; 4];
        init[0] = cmd::SCALE_1_TO_1;
        init[1..].copy_from_slice(&cmd::key(0, 0));
        run(e, "copy state: scale 1:1, key off", &init, &[]);
        let checks = |dx: usize, dy: usize| {
            [(0, 0), (63, 63), (17, 40), (63, 0)].map(|(x, y)| (dx + x, dy + y, tile(x, y)))
        };
        let (w, n) = cmd::copy(
            Rgb565,
            (a, s),
            (TILE, TILE_STRIDE),
            (0, 0),
            (40, 360, 104, 424),
            0,
        );
        run(
            e,
            "copy: tile from off-screen to (40,360)",
            &w[..n],
            &checks(40, 360),
        );
        let (w, n) = cmd::copy(
            Rgb565,
            (a, s),
            (a, s),
            (40, 360),
            (48, 360, 112, 424),
            OP_RIGHT_LEFT,
        );
        run(
            e,
            "copy: 8 px right over itself (right to left)",
            &w[..n],
            &checks(48, 360),
        );
        // Down over itself: bit 23 doesn't walk bottom-up on the chip (an
        // exact top-down smear, 2026-10-14) and Dell's driver never asks it
        // to. Strips no taller than the shift, the bottom one first, each a
        // plain copy that doesn't overlap its own source.
        const DY: u16 = 8;
        let mut words = [0u32; 10 * 8];
        let mut len = 0;
        for k in (0..64 / DY).rev() {
            let (w, n) = cmd::copy(
                Rgb565,
                (a, s),
                (a, s),
                (48, 360 + k * DY),
                (48, 368 + k * DY, 112, 368 + (k + 1) * DY),
                0,
            );
            words[len..len + n].copy_from_slice(&w[..n]);
            len += n;
        }
        run(
            e,
            "copy: 8 px down over itself (8 px strips, bottom first)",
            &words[..len],
            &checks(48, 368),
        );
        let green = cmd::fill(Rgb565, a, s, (200, 360, 264, 424), 0x07E0);
        let (w, n) = cmd::copy(
            Rgb565,
            (a, s),
            (KEYED, TILE_STRIDE),
            (0, 0),
            (200, 360, 264, 424),
            OP_KEY,
        );
        let mut words = [0u32; 3 + 7 + 9];
        words[..3].copy_from_slice(&cmd::key(u32::from(KEY), 0xFFFF));
        words[3..10].copy_from_slice(&green);
        words[10..10 + n].copy_from_slice(&w[..n]);
        run(
            e,
            "keyed copy over green: key pixels stay green",
            &words[..10 + n],
            &[
                (200, 360, 0x07E0),
                (201, 360, tile(1, 0)),
                (263, 423, 0x07E0),
                (262, 423, tile(62, 63)),
            ],
        );
        run(e, "key off", &cmd::key(0, 0), &[]);

        // Copy speed: ten full screens from off-screen memory.
        let t0 = rt::ticks();
        for _ in 0..10 {
            let (w, n) = cmd::copy(
                Rgb565,
                (a, s),
                (SCREEN_COPY, s),
                (0, 0),
                (0, 0, 480, 640),
                0,
            );
            if e.submit(&w[..n]).is_err() {
                fail(format_args!("FIFO full"));
            }
        }
        let queued = us_since(t0);
        match e.finish(vram, 1_000_000) {
            Ok(_) => {
                let us = us_since(t0);
                log!(
                    "10 GPU full-screen copies: {us} us ({} us each; queued in {queued} us)",
                    us / 10
                );
            }
            Err(why) => fail(format_args!("{why}")),
        }
        log!(
            "copies: {}",
            if ok { "all as expected" } else { "SOME WRONG" }
        );
        ok
    }

    /// Where stage 4b's copies land: off screen, 10 MiB in.
    const DMA_DST: usize = 0x00A0_0000;
    /// A full screen of RGB565: 600 KiB.
    const SCREEN_BYTES: usize = 480 * 640 * 2;
    /// One DMA transfer: 120 KiB (under the controller's 128 KiB chain).
    const DMA_PIECE: usize = 120 * 1024;
    /// The 2700G's local memory as the CPU sees it (CS3).
    const CS3: u32 = 0x0C00_0000;

    /// Spins `n` times; what the CPU gets done while a copy runs.
    fn busy(n: &mut u32) {
        *n = core::hint::black_box(*n + 1);
    }

    /// Stage 4b: a full screen from SDRAM into 2700G memory, by the CPU and
    /// by DMA, both timed; how much the CPU gets done meanwhile; the copy
    /// checked. Returns whether the DMA copy arrived intact (a refused or
    /// failed transfer is logged, not a failure: the question is speed).
    fn dma_vs_cpu(vram: &mut [u8]) -> bool {
        let Some(scratch) = rp_hal_x51v::devices::take_scratch() else {
            log!("dma: no scratch RAM");
            return true;
        };
        let src = &mut scratch[..SCREEN_BYTES];
        for (i, w) in src.chunks_exact_mut(4).enumerate() {
            w.copy_from_slice(&(i as u32).wrapping_mul(0x9E37_79B9).to_le_bytes());
        }
        // The CPU's rate at spinning, for comparison.
        let mut idle = 0u32;
        let t0 = rt::ticks();
        while us_since(t0) < 10_000 {
            busy(&mut idle);
        }
        let t0 = rt::ticks();
        vram[DMA_DST..DMA_DST + SCREEN_BYTES].copy_from_slice(src);
        cpu::drain_write_buffer();
        let cpu_us = us_since(t0);
        log!(
            "dma: CPU copies a full screen from SDRAM in {cpu_us} us ({} MB/s)",
            SCREEN_BYTES as u32 / cpu_us.max(1)
        );
        // Clear the target so the check below sees what DMA wrote.
        rp_hal_mem::fill32(&mut vram[DMA_DST..DMA_DST + SCREEN_BYTES], 0);
        cpu::drain_write_buffer();
        let Some(mut dma) = rt::take_dma(0) else {
            log!("dma: no channel");
            return true;
        };
        log!(
            "dma: memory to memory, SDRAM 0x{:08x} to 0x{:08x}",
            src.as_ptr() as u32,
            CS3 + DMA_DST as u32
        );
        let mut spins = 0u32;
        let t0 = rt::ticks();
        for (k, piece) in src.chunks(DMA_PIECE).enumerate() {
            let dst = CS3 + (DMA_DST + k * DMA_PIECE) as u32;
            if !dma.arm_copy_to_device_memory(piece, dst) {
                log!("  refused at piece {k}");
                return true;
            }
            let p0 = rt::ticks();
            loop {
                match dma.done() {
                    Some(true) => break,
                    Some(false) => {
                        log!("  bus error at piece {k}");
                        return true;
                    }
                    None if us_since(p0) > 200_000 => {
                        dma.abort();
                        log!("  piece {k} not done after 200 ms: stopped");
                        return true;
                    }
                    None => busy(&mut spins),
                }
            }
        }
        let dma_us = us_since(t0);
        // Spins per microsecond, idle vs during DMA: how free the CPU was.
        let free =
            u64::from(spins) * 10_000 * 100 / (u64::from(idle).max(1) * u64::from(dma_us.max(1)));
        log!(
            "dma: a full screen in {dma_us} us ({} MB/s); the CPU kept {free}% of its idle pace meanwhile",
            SCREEN_BYTES as u32 / dma_us.max(1)
        );
        let mut intact = true;
        for at in [0, 4, SCREEN_BYTES / 2, SCREEN_BYTES - 4] {
            let want = load32_volatile(src, at);
            let got = load32_volatile(vram, DMA_DST + at);
            if got != want {
                intact = false;
                log!("  word at +{at}: {got:08x?}, expected {want:08x?}  <-- WRONG");
            }
        }
        log!("dma: copy {}", if intact { "intact" } else { "WRONG" });
        intact
    }

    /// Where stage 4c's copies land, prefilled with a pattern the engine
    /// won't produce by accident.
    const PROBE_DST: usize = 0x00B0_0000;
    const UNTOUCHED: u32 = 0xDEAD_BEEF;

    /// Stage 4c: copies 64 bytes from addresses past the 2700G's 16 MiB
    /// (its address field reaches 64 MiB) and says what arrived:
    /// nothing, a mirror of local memory (the screen at offset 0, or the
    /// test tile at 8 MiB), or something else (logged, to compare with
    /// SDRAM). Informational: the engine may hang; the log names the
    /// address before each try.
    fn probe_beyond_local_memory(e: &mut Engine, vram: &mut [u8]) {
        let screen0 = load32_volatile(vram, 0);
        let tile0 = load32_volatile(vram, TILE);
        for mib in [16usize, 24, 32, 40, 48, 56] {
            let src = mib << 20;
            for k in 0..16 {
                let _ = store32_volatile(vram, PROBE_DST + 4 * k, UNTOUCHED);
            }
            cpu::drain_write_buffer();
            log!("probe: engine reads local address 0x{src:08x} ({mib} MiB)");
            let (w, n) = cmd::copy(
                cmd::Format::Rgb565,
                (PROBE_DST, 64),
                (src, 64),
                (0, 0),
                (0, 0, 32, 1),
                0,
            );
            if e.submit(&w[..n]).is_err() {
                log!("  FIFO full: stopping the probe");
                return;
            }
            if let Err(why) = e.finish(vram, 50_000) {
                log!("  {why}: the engine is stuck; stopping the probe");
                return;
            }
            let got: [Option<u32>; 4] =
                core::array::from_fn(|k| load32_volatile(vram, PROBE_DST + 4 * k));
            let what = if got[0] == Some(UNTOUCHED) {
                "nothing written"
            } else if got[0] == screen0 && mib % 16 == 0 {
                "same as local 0x0 (a mirror?)"
            } else if got[0] == tile0 && mib % 16 == 8 {
                "same as local 8 MiB (a mirror?)"
            } else {
                "SOMETHING ELSE: compare with SDRAM"
            };
            log!("  {got:08x?}: {what}");
        }
    }

    /// The ways tried to copy rows `[0, h - dy)` of a surface onto
    /// `[dy, h)`, the copy a scroll needs.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Down {
        /// Bit 23, coordinates as for a plain copy (smeared, 2026-10-14).
        Bit23,
        /// Bit 23, the source origin on the source's bottom row.
        Bit23OriginBottom,
        /// Bit 23, origin on the bottom row, destination given bottom to top.
        Bit23Flipped,
        /// Bit 23, origin on the top row, destination given bottom to top.
        Bit23RectFlipped,
        /// Both surfaces start at their bottom row with a negative stride
        /// (15-bit two's complement): a plain top-down copy in mirrored
        /// memory, bottom first in reality, if the engine takes it.
        NegativeStride,
        /// Strips no taller than `dy`, the bottom one first.
        Strips,
        /// Out to scratch memory, then back `dy` lower: two copies.
        Scratch,
    }
    const DOWN: [Down; 7] = [
        Down::Bit23,
        Down::Bit23OriginBottom,
        Down::Bit23Flipped,
        Down::Bit23RectFlipped,
        Down::NegativeStride,
        Down::Strips,
        Down::Scratch,
    ];
    /// Bit 23 of the operation header (Dell's "y not positive").
    const BIT23: u32 = 1 << 23;
    /// The small test surface: a guard row, 72 rows of 64 pixels, a guard
    /// row; 12 MiB into local memory, clear of everything else here (a
    /// negative stride read as unsigned stays below 15 MiB).
    const WAYS: usize = 0x00C0_0000;
    const WAYS_STRIDE: usize = 128;
    const WAYS_H: u16 = 72;
    /// Scratch for [`Down::Scratch`] (a full screen fits).
    const SCRATCH: usize = 0x00D0_0000;

    /// Queues one way's commands; returns how many copies it took.
    fn queue_down(
        e: &mut Engine,
        m: Down,
        (at, st): (usize, usize),
        (w, h): (u16, u16),
        dy: u16,
    ) -> Result<u32, u32> {
        use cmd::Format::Rgb565;
        let surf = (at, st);
        let mut send = |dst, src, origin, rect, flags| {
            let (words, n) = cmd::copy(Rgb565, dst, src, origin, rect, flags);
            e.submit(&words[..n])
        };
        let bottom = h - dy - 1;
        match m {
            Down::Bit23 => send(surf, surf, (0, 0), (0, dy, w, h), BIT23)?,
            Down::Bit23OriginBottom => send(surf, surf, (0, bottom), (0, dy, w, h), BIT23)?,
            Down::Bit23Flipped => send(surf, surf, (0, bottom), (0, h - 1, w, dy - 1), BIT23)?,
            Down::Bit23RectFlipped => send(surf, surf, (0, 0), (0, h - 1, w, dy - 1), BIT23)?,
            Down::NegativeStride => {
                let neg = 0x8000 - st;
                let last = |row: u16| at + usize::from(row) * st;
                send(
                    (last(h - 1), neg),
                    (last(bottom), neg),
                    (0, 0),
                    (0, 0, w, h - dy),
                    0,
                )?;
            }
            Down::Strips => {
                let n = (h - dy).div_ceil(dy);
                for i in (0..n).rev() {
                    let y0 = dy + i * dy;
                    send(surf, surf, (0, i * dy), (0, y0, w, (y0 + dy).min(h)), 0)?;
                }
                return Ok(u32::from(n));
            }
            Down::Scratch => {
                send((SCRATCH, st), surf, (0, 0), (0, 0, w, h - dy), 0)?;
                send(surf, (SCRATCH, st), (0, 0), (0, dy, w, h), 0)?;
                return Ok(2);
            }
        }
        Ok(1)
    }

    /// The small surface's pixel at row `r` (0 and `WAYS_H + 1` are guards).
    fn ways_px(x: usize, r: usize) -> u16 {
        tile(x, r)
    }

    /// Fills the small surface, guards included.
    fn ways_fill(vram: &mut [u8]) {
        for r in 0..usize::from(WAYS_H) + 2 {
            for x in (0..64).step_by(2) {
                let v = u32::from(ways_px(x, r)) | u32::from(ways_px(x + 1, r)) << 16;
                let _ = store32_volatile(vram, WAYS + r * WAYS_STRIDE + x * 2, v);
            }
        }
        cpu::drain_write_buffer();
    }

    /// Pixels of the small surface that differ from a correct shift by `dy`.
    fn ways_wrong(vram: &[u8], dy: usize) -> u32 {
        let h = usize::from(WAYS_H);
        let mut wrong = 0;
        for r in 0..h + 2 {
            let from = if r > dy && r <= h { r - dy } else { r };
            for x in (0..64).step_by(2) {
                let want = u32::from(ways_px(x, from)) | u32::from(ways_px(x + 1, from)) << 16;
                let got = load32_volatile(vram, WAYS + r * WAYS_STRIDE + x * 2).unwrap_or(!want);
                wrong += ((got ^ want) & 0xFFFF != 0) as u32 + ((got ^ want) >> 16 != 0) as u32;
            }
        }
        wrong
    }

    /// Stage 4d: each way on the small surface, shifted 1 and 8 rows, every
    /// pixel and both guard rows checked; then the ways that worked, timed
    /// on the whole screen. The negative stride (the winner on the device,
    /// 2026-10-14: one copy, 4.2 ms full screen), strips and scratch must
    /// work; the bit 23 ways are informational (all wrong on the device);
    /// stops at the first stuck engine.
    fn ways_down(e: &mut Engine, vram: &mut [u8], screen: (usize, usize)) -> bool {
        let surf = (WAYS + WAYS_STRIDE, WAYS_STRIDE);
        let mut works = [true; DOWN.len()];
        let mut ok = true;
        for (k, &m) in DOWN.iter().enumerate() {
            for dy in [1u16, 8] {
                ways_fill(vram);
                if let Err(st) = queue_down(e, m, surf, (64, WAYS_H), dy) {
                    log!("down {m:?} dy {dy}: FIFO stuck (0x{st:08x}): stopping");
                    return false;
                }
                if let Err(why) = e.finish(vram, 50_000) {
                    log!("down {m:?} dy {dy}: {why}: stopping");
                    return false;
                }
                let wrong = ways_wrong(vram, usize::from(dy));
                works[k] &= wrong == 0;
                log!(
                    "down {m:?} dy {dy}: {}",
                    if wrong == 0 { "exact" } else { "WRONG" }
                );
                if wrong != 0 {
                    log!("  {wrong} of 4736 pixels differ");
                }
            }
            if matches!(m, Down::NegativeStride | Down::Strips | Down::Scratch) {
                ok &= works[k];
            }
        }
        for (k, &m) in DOWN.iter().enumerate() {
            if !works[k] {
                continue;
            }
            for dy in [1u16, 8, 32] {
                let t0 = rt::ticks();
                let n = match queue_down(e, m, screen, (480, 640), dy) {
                    Ok(n) => n,
                    Err(st) => {
                        log!("time {m:?} dy {dy}: FIFO stuck (0x{st:08x}): stopping");
                        return false;
                    }
                };
                let queued = us_since(t0);
                if let Err(why) = e.finish(vram, 2_000_000) {
                    log!("time {m:?} dy {dy}: {why}: stopping");
                    return false;
                }
                log!(
                    "time {m:?} full screen dy {dy}: {} us, {n} copies (CPU busy queueing {queued} us)",
                    us_since(t0)
                );
            }
        }
        ok
    }

    /// `0x12c`'s bits that aren't the FIFO level (16-23 and 30).
    const STATUS_BITS: u32 = !(0x00FF_0000 | 1 << 30);
    /// GPLR0-3: every GPIO's level.
    const GPLR: [usize; 4] = [0x000, 0x004, 0x008, 0x100];

    /// Stage 4e: with the MBX interrupt enables (`0x130`) off, as Windows
    /// Mobile sets them (`0x87F5`), and bit 10 alone, queues ~60 ms of fills
    /// and a fence, and samples `0x12c` and every GPIO level as fast as it
    /// can until 5 ms after the fence lands, logging each change with its
    /// time from the queueing. A status bit rising at the fence says the
    /// engine reports completion; a GPIO following it says the 2700G's
    /// interrupt reaches the PXA (and on which pin). The 2700G's own
    /// system-level interrupt registers are left as Windows Mobile set them
    /// and never read (their layout is unknown; a wrong read hangs the bus).
    fn completion(
        e: &mut Engine,
        vram: &mut [u8],
        (a, s): (usize, usize),
    ) -> Option<(Mmio, [u32; 4])> {
        let Some(mut gpio) = rt::take_gpio() else {
            log!("completion: no GPIO controller");
            return None;
        };
        let levels = |g: &mut Mmio| GPLR.map(|r| g.read32(r));
        // Pins that change with the engine idle are other peripherals'
        // clocks and PWM outputs (they filled the first run's log in 0.3 ms):
        // learnt over 5 ms, then ignored.
        let mut noisy = [0u32; 4];
        let mut prev = levels(&mut gpio);
        let t = rt::ticks();
        while us_since(t) < 5_000 {
            let g1 = levels(&mut gpio);
            for k in 0..4 {
                noisy[k] |= g1[k] ^ prev[k];
            }
            prev = g1;
        }
        log!("completion: GPIO bits busy while idle (ignored): {noisy:08x?}");
        for enables in [0u32, 0x87F5, 0x0400] {
            e.regs.write32(MBX + reg::ENABLE, enables);
            e.regs.write32(MBX + reg::CLEAR, 0xFFFF);
            let st0 = e.status() & STATUS_BITS;
            let g0 = levels(&mut gpio);
            log!("completion: 0x130 = 0x{enables:04x}, status 0x{st0:08x} before");
            let t0 = rt::ticks();
            for i in 0..20 {
                let w = cmd::fill(cmd::Format::Rgb565, a, s, (0, 0, 480, 640), 0x0841 * i);
                if e.submit(&w).is_err() {
                    log!("  FIFO full at fill {i}: stopping");
                    return None;
                }
            }
            e.fence += 1;
            let n = e.fence;
            if e.submit(&cmd::fence(FENCE, n)).is_err() {
                log!("  FIFO full at the fence: stopping");
                return None;
            }
            let (mut st, mut g) = (st0, g0);
            let mut landed: Option<u32> = None;
            let (mut status_changes, mut pin_changes) = (0, 0);
            let mut seen = st0;
            loop {
                let now = us_since(t0);
                let st1 = e.status() & STATUS_BITS;
                let g1 = levels(&mut gpio);
                if landed.is_none() && load32_volatile(vram, FENCE) == Some(n) {
                    landed = Some(now);
                    log!("  {now} us: fence landed, status 0x{st1:08x}");
                }
                // Status and pins on separate budgets: neither crowds out
                // the other.
                if st1 != st && status_changes < 16 {
                    status_changes += 1;
                    log!("  {now} us: status 0x{st1:08x}");
                }
                let flip: [u32; 4] = core::array::from_fn(|k| (g1[k] ^ g[k]) & !noisy[k]);
                if flip != [0; 4] && pin_changes < 16 {
                    pin_changes += 1;
                    log!("  {now} us: GPIO bits flipped {flip:08x?} (now {g1:08x?})");
                }
                seen |= st1;
                (st, g) = (st1, g1);
                match landed {
                    Some(t) if now > t + 5_000 => break,
                    None if now > 500_000 => {
                        log!("  fence never landed: stopping");
                        return None;
                    }
                    _ => {}
                }
            }
            log!("  every status bit seen: 0x{seen:08x}; status now 0x{st:08x}");
            e.regs.write32(MBX + reg::CLEAR, 0xFFFF);
            let st2 = e.status() & STATUS_BITS;
            let g2 = levels(&mut gpio);
            let flip: [u32; 4] = core::array::from_fn(|k| (g2[k] ^ g0[k]) & !noisy[k]);
            log!(
                "  after clearing: status 0x{st2:08x}, GPIO bits differing from before {flip:08x?}"
            );
        }
        // As the image found them: masked.
        e.regs.write32(MBX + reg::ENABLE, 0);
        e.regs.write32(MBX + reg::CLEAR, 0xFFFF);
        Some((gpio, noisy))
    }

    /// The display's interrupt status and enable (`DINTRS`, `DINTRE`: read
    /// and written by Dell's display ISR, so present; status bits clear by
    /// writing 1), and the 2700G's interrupt enable (`MINTENABLE`, system
    /// block: write-only for us, never read; bit 15 the master enable, bits
    /// 0-3 the sources, as WM writes `0x800F`).
    const DINTRS: usize = 0x2178;
    const DINTRE: usize = 0x217C;
    const MINTENABLE: usize = 0x018;
    /// The display's three status bits. Device, 2026-10-14: bit 0 was set
    /// and cleared by writing 1; bit 2 read set at every one of 26,237 polls
    /// in 100 ms, even right after writing 1 (a level: "the last address
    /// change was taken", as Dell enables it only while a flip is pending).
    const DISPLAY_BITS: u32 = 0b111;

    /// Stage 4f: the 2700G's interrupt line and the display's events.
    /// GPIO levels are taken before anything is enabled. With `DINTRE` at
    /// 0, then each display bit alone (`MINTENABLE` `0x800F`), for 40 ms
    /// each: every display event logged with its time (first 3 per bit),
    /// held 0.5 ms (so a level line shows) and then cleared; every GPIO
    /// outside the busy pins that differs from before or changes is logged.
    /// The pin that follows an enabled event is the 2700G's interrupt
    /// (SYSINTR 27 under WM; candidates 11, 12, 83-87, 94, 96). `DINTRE` is
    /// put back after. Informational.
    fn interrupt_line(e: &mut Engine, (mut gpio, noisy): (Mmio, [u32; 4])) {
        let levels = |g: &mut Mmio| GPLR.map(|r| g.read32(r));
        let quiet = |a: [u32; 4], b: [u32; 4]| -> [u32; 4] {
            core::array::from_fn(|k| (a[k] ^ b[k]) & !noisy[k])
        };
        let g_before = levels(&mut gpio);
        let re0 = e.regs.read32(DINTRE);
        let rs0 = e.regs.read32(DINTRS);
        log!("irq: DINTRE 0x{re0:08x}, DINTRS 0x{rs0:08x} as found");
        e.regs.write32(DINTRE, 0);
        e.regs.write32(DINTRS, DISPLAY_BITS);
        e.regs.write32(MINTENABLE, 0x800F);
        for enable in [0u32, 1, 2, 4] {
            e.regs.write32(DINTRS, DISPLAY_BITS);
            e.regs.write32(DINTRE, enable);
            let g0 = levels(&mut gpio);
            log!(
                "irq: DINTRE = {enable}: status 0x{:08x}, GPIO differing from before {:08x?}",
                e.regs.read32(DINTRS),
                quiet(g0, g_before)
            );
            let mut g = g0;
            let mut events = [0u32; 3];
            let mut held: Option<(u32, u32)> = None;
            let mut pin_changes = 0;
            let t0 = rt::ticks();
            while us_since(t0) < 40_000 {
                let now = us_since(t0);
                let st = e.regs.read32(DINTRS) & DISPLAY_BITS;
                let g1 = levels(&mut gpio);
                let flip = quiet(g1, g);
                if flip != [0; 4] && pin_changes < 12 {
                    pin_changes += 1;
                    log!("  {now} us: GPIO bits flipped {flip:08x?} (status 0x{st:x})");
                }
                g = g1;
                match held {
                    None if st & 0b011 != 0 => {
                        for b in 0..2 {
                            if st & 1 << b != 0 {
                                events[b] += 1;
                                if events[b] <= 3 {
                                    log!("  {now} us: display bit {b} set");
                                }
                            }
                        }
                        held = Some((now, st & 0b011));
                    }
                    Some((t, bits)) if now > t + 500 => {
                        e.regs.write32(DINTRS, bits);
                        held = None;
                    }
                    _ => {}
                }
                if st & 0b100 != 0 {
                    events[2] += 1;
                }
            }
            e.regs.write32(DINTRE, 0);
            e.regs.write32(DINTRS, DISPLAY_BITS);
            let g2 = levels(&mut gpio);
            log!(
                "  in 40 ms: bit 0 set {} times, bit 1 {}, bit 2 read set {} times; GPIO after disabling differs from before {:08x?}",
                events[0],
                events[1],
                events[2],
                quiet(g2, g_before)
            );
        }
        e.regs.write32(DINTRE, re0);
    }

    /// Keeps the picture up until a button is pressed and released (the
    /// release too, so the press doesn't reach the loader), checking in with
    /// the loader's watchdog; gives up after 5 minutes.
    fn hold_until_button() {
        let Some(kp) = rt::take_keypad() else {
            log!("no keypad: returning");
            return;
        };
        let mut keypad = Keypad::new(kp);
        keypad.enable();
        // Activity left over from before (the loader's own keys) doesn't count.
        keypad.acknowledge();
        let t0 = rt::ticks();
        let mut seen = false;
        while us_since(t0) < 300_000_000 {
            rt::check_in();
            // The controller latches activity until it's read, so a press
            // and release that both fall between two polls still count;
            // polling only the buttons held now missed them in the emulator
            // (a 100 ms press while the host stalled the guest).
            seen |= keypad.acknowledge();
            if seen && keypad.pressed() == Pressed::NONE {
                return;
            }
            rt::delay_us(20_000);
        }
        log!("no button in 5 minutes: returning");
    }

    /// Logs a failed step and returns to the loader.
    fn fail(msg: core::fmt::Arguments) -> ! {
        log!("{MARKER}: FAIL {msg}");
        rp_hal_x51v::devices::return_to_loader();
        loop {
            core::hint::spin_loop();
        }
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
