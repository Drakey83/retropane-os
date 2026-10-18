//! RetroPane looks test (display calibration C6, docs/design/display-calibration.md):
//! the owner's photos, a colour chart and ramps under the measured
//! calibration tables, against the original, judged by eye.
//!
//! Every page is drawn once at start into its own 32 bpp screen in 2700G
//! memory ([`rp_looks::page_pixel`]); a look changes only the gamma table
//! and the backlight, a page only the plane's address, so every flip is
//! instant.
//!
//! | Input | Does |
//! |---|---|
//! | D-pad right / left | next / previous look ([`rp_looks::LOOKS`]) |
//! | D-pad down / up | next / previous page (Pier, Milky Way, chart, ramps) |
//! | D-pad centre | flips between this look and the original |
//! | Home | done: everything put back, back to the loader |
//!
//! A caption across the top names the look and page for 3 seconds after
//! each change (red while the original is shown), then the picture comes
//! back. The run log keeps every change and the time each look was on
//! screen. After 10 minutes with no input it puts everything back and
//! returns. Display registers, the gamma table and the backlight are
//! restored as found.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed when the image puts everything back.
pub const MARKER: &str = "RETROPANE-LOOKS-OK";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_mbx_display::layers::{Layer, Snapshot};
    use rp_drv_mbx_display::scanout::GAMMA_ENTRIES;
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, crash, font};
    use rp_drv_pxa_keypad::keys::{Keys, Kind, Timing};
    use rp_drv_pxa_keypad::{Button, Keypad};
    use rp_hal_api::RegisterBus;
    use rp_hal_pxa27x::rt;
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::{self, RunLog};
    use rp_looks::{
        CAPTION_H, H, LOOKS, Light, PAGES, PHOTOS, W, YUV_BYTES, look_table, page_pixel, unpack,
    };
    use rp_vram_map::{BACKGROUND, CACHE, POOL, SCREENS, WALLPAPER};

    rp_hal_pxa27x::entry!(main);

    const GFXGAM0: usize = 0x2250;
    const GSCTRL: usize = 0x2000;
    const GSCTRL_GAMMA: u32 = 1 << 26;
    /// PWM registers (rp-drv-pxa-pwm's `reg`).
    const CR: usize = 0x0;
    const DCR: usize = 0x4;
    const PCR: usize = 0x8;
    /// RetroPane's backlight cycle: prescale 0, 432 counts (30 kHz).
    const COUNTS: u32 = 432;
    /// No input for this long: put back and return.
    const IDLE_MS: u64 = 600_000;
    /// The caption stays this long after a change.
    const CAPTION_MS: u64 = 3_000;
    const STRIDE: usize = W as usize * 4;
    const PAGE_BYTES: usize = STRIDE * H as usize;
    const STRIP_BYTES: usize = STRIDE * CAPTION_H as usize;

    /// Where each page lives in 2700G memory (all below the photo scratch).
    fn page_at(page: usize) -> usize {
        [SCREENS[0].at, SCREENS[1].at, WALLPAPER.at, BACKGROUND.at][page] as usize
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash_out(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    #[inline(never)]
    fn crash_out(msg: core::fmt::Arguments) -> ! {
        if let Some((regs, vram)) = devices::emergency_mbx() {
            let mut c =
                crash::emergency_console(regs, vram, (0, 0), (480, 64), 2, (0xFFFF, 0x0000));
            let _ = write!(c, "{msg}");
        }
        devices::crash_to_loader(msg);
        halt()
    }

    /// Writes one pixel (XRGB8888).
    fn put(fb: &mut [u8], x: u32, y: u32, (r, g, b): (u8, u8, u8)) {
        let o = y as usize * STRIDE + x as usize * 4;
        fb[o..o + 4].copy_from_slice(&[b, g, r, 0xFF]);
    }

    /// Text at `(x, y)`, `scale` times the font's size, on black.
    fn text(fb: &mut [u8], x: u32, y: u32, scale: u32, colour: (u8, u8, u8), s: &str) {
        let cell = (font::WIDTH as u32 + 1) * scale;
        let mut px = x;
        for c in s.chars() {
            if px + cell > W {
                break;
            }
            let g = font::glyph(c);
            for gy in 0..(font::HEIGHT as u32 + 1) * scale {
                for gx in 0..cell {
                    let (fx, fy) = (gx / scale, gy / scale);
                    let lit = fx < font::WIDTH as u32
                        && (fy as usize) < font::HEIGHT
                        && g[fy as usize] & (1 << (font::WIDTH as u32 - 1 - fx)) != 0;
                    put(fb, px + gx, y + gy, if lit { colour } else { (0, 0, 0) });
                }
            }
            px += cell;
        }
    }

    /// A short string on the stack.
    struct Line {
        buf: [u8; 64],
        len: usize,
    }

    impl Write for Line {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let n = s.len().min(self.buf.len() - self.len);
            self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
            self.len += n;
            Ok(())
        }
    }

    impl Line {
        fn new() -> Self {
            Self {
                buf: [0; 64],
                len: 0,
            }
        }
        fn as_str(&self) -> &str {
            core::str::from_utf8(&self.buf[..self.len]).unwrap_or("?")
        }
    }

    /// The caption over the top of page `fb`.
    fn caption(fb: &mut [u8], look: usize, page: usize, original: bool) {
        fb[..STRIP_BYTES].fill(0);
        for px in fb[..STRIP_BYTES].chunks_exact_mut(4) {
            px[3] = 0xFF;
        }
        let mut top = Line::new();
        let mut low = Line::new();
        if original {
            let _ = write!(top, "ORIGINAL ({})", LOOKS[0].name);
        } else {
            let _ = write!(top, "{}/{} {}", look + 1, LOOKS.len(), LOOKS[look].name);
        }
        let _ = write!(low, "{}  <> look  ^v picture  OK original", PAGES[page]);
        let colour = if original {
            (255, 60, 60)
        } else {
            (255, 220, 0)
        };
        text(fb, 6, 2, 3, colour, top.as_str());
        text(fb, 6, 27, 2, (200, 200, 200), low.as_str());
    }

    fn write_gamma<B: RegisterBus>(regs: &mut B, t: &[u32; GAMMA_ENTRIES]) {
        for (i, w) in t.iter().enumerate() {
            regs.write32(GFXGAM0 + 4 * i, *w);
        }
    }

    fn log_table(what: &str, t: &[u32; GAMMA_ENTRIES]) {
        let _ = write!(RunLog, "{what}:");
        for w in t {
            let _ = write!(RunLog, " {w:06X}");
        }
        let _ = writeln!(RunLog);
    }

    fn plane(page: usize) -> Plane {
        Plane {
            format: PixelFormat::Argb8888,
            width: W as u16,
            height: H as u16,
            offset: page_at(page),
            stride: STRIDE,
        }
    }

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let mut clock = rt::Elapsed::start();
        let (Some((regs, vram)), Some(mut pwm)) = (devices::take_mbx(), rt::take_pwm0()) else {
            halt()
        };
        let mut display = Display::new(regs);
        let _ = display.adopt(MBX_VRAM_SIZE as usize);
        let before = Snapshot::take(display.regs());
        let gsctrl = display.regs().read32(GSCTRL);
        let found: [u32; GAMMA_ENTRIES] =
            core::array::from_fn(|i| display.regs().read32(GFXGAM0 + 4 * i));
        let light = (pwm.read32(CR), pwm.read32(DCR), pwm.read32(PCR));
        let _ = writeln!(
            RunLog,
            "GSCTRL {gsctrl:08X}; PWM0 found: CR {:02X} DCR {:03X} PCR {:03X}",
            light.0, light.1, light.2
        );
        log_table("gamma found", &found);

        // Every page once: the photos unpacked into the scratch above the
        // pages, then converted; the caption strip of each kept in the pool.
        let (pages, high) = vram.split_at_mut(CACHE.at as usize);
        let (scratch, rest) = high.split_at_mut(POOL.at as usize - CACHE.at as usize);
        let strips = &mut rest[..4 * STRIP_BYTES];
        for page in 0..PAGES.len() {
            let photo = page < PHOTOS.len();
            if photo && let Err(e) = unpack(PHOTOS[page], &mut scratch[..YUV_BYTES]) {
                crash_out(format_args!("photo {} damaged: {e:?}", PAGES[page]))
            }
            let at = page_at(page);
            let fb = &mut pages[at..at + PAGE_BYTES];
            let yuv: &[u8] = &scratch[..YUV_BYTES];
            for y in 0..H {
                for x in 0..W {
                    put(fb, x, y, page_pixel(page, [yuv, yuv], x, y));
                }
                rt::check_in();
            }
            strips[page * STRIP_BYTES..(page + 1) * STRIP_BYTES]
                .copy_from_slice(&fb[..STRIP_BYTES]);
        }
        let _ = writeln!(RunLog, "pages drawn in {} ms", clock.ms());

        let (mut look, mut page, mut original) = (0usize, 0usize, false);
        let show = |display: &mut Display<_>, pwm: &mut _, look: usize, original: bool| {
            let l = if original { 0 } else { look };
            write_gamma(display.regs(), &look_table(l, &found));
            set_light(pwm, LOOKS[l].light, light);
        };
        caption(
            &mut pages[page_at(page)..page_at(page) + PAGE_BYTES],
            look,
            page,
            original,
        );
        display.set_graphics(&plane(page), Layer::SOLO);
        let on = display.regs().read32(GSCTRL) | GSCTRL_GAMMA;
        display.regs().write32(GSCTRL, on);
        show(&mut display, &mut pwm, look, original);

        let Some(kp) = rt::take_keypad() else {
            crash_out(format_args!("no keypad"))
        };
        let mut keypad = Keypad::new(kp);
        keypad.enable();
        keypad.acknowledge();
        let mut keys = Keys::new(Timing::DEFAULT);
        let _ = writeln!(RunLog, "ready: {}, {}", LOOKS[look].name, PAGES[page]);

        let mut last_input = clock.ms();
        let mut caption_until = Some(last_input + CAPTION_MS);
        let mut changes = 0u32;
        // Milliseconds each look was on screen (the original while comparing
        // counts as look 0), and since when the current one has been.
        let mut seen = [0u64; LOOKS.len()];
        let mut since = last_input;
        let reason = loop {
            rt::check_in();
            rt::delay_us(10_000);
            let now = clock.ms();
            if now - last_input > IDLE_MS {
                break "no input for 10 minutes";
            }
            let (mut by, mut go, mut flip, mut done) = (0i32, 0i32, false, false);
            keypad.acknowledge();
            keys.update(now as u32, keypad.pressed(), &mut |e| {
                if matches!(e.kind, Kind::Down) {
                    match e.button {
                        Button::Right => by += 1,
                        Button::Left => by -= 1,
                        Button::Down => go += 1,
                        Button::Up => go -= 1,
                        Button::Enter => flip = !flip,
                        Button::Home => done = true,
                        _ => {}
                    }
                }
            });
            if done {
                break "Home";
            }
            if by != 0 || go != 0 || flip {
                seen[if original { 0 } else { look }] += now - since;
                since = now;
                last_input = now;
                changes += 1;
                // The strip of the page leaving goes back first.
                let at = page_at(page);
                pages[at..at + STRIP_BYTES]
                    .copy_from_slice(&strips[page * STRIP_BYTES..(page + 1) * STRIP_BYTES]);
                if by != 0 {
                    look = (look as i32 + by).rem_euclid(LOOKS.len() as i32) as usize;
                    original = false;
                }
                if go != 0 {
                    page = (page as i32 + go).rem_euclid(PAGES.len() as i32) as usize;
                }
                original ^= flip;
                let at = page_at(page);
                caption(&mut pages[at..at + PAGE_BYTES], look, page, original);
                display.set_graphics(&plane(page), Layer::SOLO);
                show(&mut display, &mut pwm, look, original);
                caption_until = Some(now + CAPTION_MS);
                let _ = writeln!(
                    RunLog,
                    "at {now} ms: {}{}, {}",
                    LOOKS[look].name,
                    if original {
                        " (showing the original)"
                    } else {
                        ""
                    },
                    PAGES[page]
                );
            }
            if caption_until.is_some_and(|t| now >= t) {
                caption_until = None;
                let at = page_at(page);
                pages[at..at + STRIP_BYTES]
                    .copy_from_slice(&strips[page * STRIP_BYTES..(page + 1) * STRIP_BYTES]);
            }
        };
        let now = clock.ms();
        seen[if original { 0 } else { look }] += now - since;

        let _ = writeln!(
            RunLog,
            "done ({reason}) after {changes} changes; time on each look:"
        );
        for (l, ms) in LOOKS.iter().zip(seen) {
            let _ = writeln!(RunLog, "  {}: {} s", l.name, ms / 1000);
        }
        let _ = writeln!(RunLog, "last look: {}, {}", LOOKS[look].name, PAGES[page]);
        write_gamma(display.regs(), &found);
        pwm.write32(CR, light.0);
        pwm.write32(PCR, light.2);
        pwm.write32(DCR, light.1);
        // The planes as found (GSCTRL's gamma bit with them).
        before.restore(display.regs());
        display.regs().write32(GSCTRL, gsctrl);
        let _ = writeln!(RunLog, "{}", super::MARKER);
        if devices::loader_present() {
            devices::return_to_loader();
        }
        halt()
    }

    fn set_light<B: RegisterBus>(pwm: &mut B, l: Light, found: (u32, u32, u32)) {
        match l {
            Light::Found => {
                pwm.write32(CR, found.0);
                pwm.write32(PCR, found.2);
                pwm.write32(DCR, found.1);
            }
            Light::Full => {
                pwm.write32(CR, 0);
                pwm.write32(PCR, COUNTS - 1);
                pwm.write32(DCR, COUNTS);
            }
        }
    }

    fn halt() -> ! {
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        match info.location() {
            Some(l) => crash_out(format_args!("PANIC {}:{}", l.file(), l.line())),
            None => crash_out(format_args!("PANIC")),
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
