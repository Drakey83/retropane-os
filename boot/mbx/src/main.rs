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
    use rp_hal_api::RegisterBus;
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
