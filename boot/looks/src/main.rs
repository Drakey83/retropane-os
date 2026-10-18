//! RetroPane looks test (display calibration C6, docs/design/display-calibration.md):
//! photos, a colour chart and ramps under a contrast-strength dial, against
//! the original, judged by eye.
//!
//! Version 4: the photos come from the SD card's `rp-looks` folder (every
//! 480 x 640 4:2:0 `.yuv` file there, in name order; read only, one page at
//! a time), or the two built in when there is no card or folder. Each look
//! is applied in the picture and dithered to the panel's 64 levels a
//! channel ([`rp_looks::shown`]); the hardware table stays straight. A
//! change redraws into the other of two screens, then shows it.
//!
//! | Input | Does |
//! |---|---|
//! | D-pad right / left | stronger / gentler: strength 1.00 (no curve) to 4.00, from 2.00 |
//! | D-pad down / up | next / previous page (the photos, the chart, the ramps) |
//! | D-pad centre | flips between this look and the original |
//! | Calendar | backlight full or as Windows Mobile left it (the original keeps WM's) |
//! | Home | done: everything put back, back to the loader |
//!
//! A caption across the top shows the strength, the light and the page for
//! 3 seconds after each change (red while the original is shown). The run
//! log keeps every change and the seconds spent at each strength. After 10
//! minutes with no input it puts everything back and returns. Display
//! registers, the gamma table and the backlight are restored as found.

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
    use rp_drv_pxa_gpio::Gpio;
    use rp_drv_pxa_keypad::keys::{Keys, Kind, Timing};
    use rp_drv_pxa_keypad::{Button, Keypad};
    use rp_drv_pxa_mmc::PxaMmc;
    use rp_drv_sdcard::{BLOCK, SdCard, mbr_partitions};
    use rp_fat::Volume;
    use rp_hal_api::RegisterBus;
    use rp_hal_pxa27x::{Mmio, rt};
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::{self, RunLog};
    use rp_looks::{
        BUILT_IN, CAPTION_H, H, Light, PHOTOS, Page, START, STEPS, W, YUV_BYTES, dial_table,
        page_pixel, shown, straight, strength, unpack,
    };
    use rp_vram_map::{CACHE, SCREENS};

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
    /// SD card detect: low with a card in (docs/hw/sd.md).
    const SD_DETECT: u8 = 12;
    /// The card's photo folder.
    const DIR: &str = "rp-looks";
    /// Photos read from the card at most.
    const MAX_PHOTOS: usize = 40;
    /// A name's longest form kept.
    const NAME: usize = 24;

    type Card = SdCard<PxaMmc<Mmio>>;

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

    /// Rows `rows` of a page through table `t`, dithered.
    fn draw(
        fb: &mut [u8],
        page: Page,
        photo: &[u8],
        t: &[u32; GAMMA_ENTRIES],
        rows: core::ops::Range<u32>,
    ) {
        for y in rows {
            for x in 0..W {
                put(fb, x, y, shown(t, page_pixel(page, photo, x, y), x, y));
            }
            rt::check_in();
        }
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

    /// The photos: names read from the card's folder, or the built-in two.
    struct Photos {
        names: [[u8; NAME]; MAX_PHOTOS],
        lens: [usize; MAX_PHOTOS],
        count: usize,
        card: bool,
    }

    impl Photos {
        fn name(&self, i: usize) -> &str {
            if self.card {
                core::str::from_utf8(&self.names[i][..self.lens[i]]).unwrap_or("?")
            } else {
                BUILT_IN[i]
            }
        }
    }

    /// The pages: every photo, then the chart and the ramps.
    fn page_kind(photos: &Photos, page: usize) -> Page {
        if page < photos.count {
            Page::Photo
        } else if page == photos.count {
            Page::Chart
        } else {
            Page::Ramps
        }
    }

    fn page_name<'a>(photos: &'a Photos, page: usize) -> &'a str {
        match page_kind(photos, page) {
            Page::Photo => photos.name(page),
            Page::Chart => "Colour chart",
            Page::Ramps => "Ramps",
        }
    }

    /// The caption over the top of `fb`.
    fn caption(
        fb: &mut [u8],
        s: usize,
        light: Light,
        original: bool,
        photos: &Photos,
        page: usize,
    ) {
        for y in 0..CAPTION_H {
            for x in 0..W {
                put(fb, x, y, (0, 0, 0));
            }
        }
        let mut top = Line::new();
        let mut low = Line::new();
        if original {
            let _ = write!(top, "ORIGINAL");
        } else {
            let st = strength(s);
            let _ = write!(
                top,
                "{}.{:02} {}",
                st / 100,
                st % 100,
                if light == Light::Full {
                    "full light"
                } else {
                    "WM light"
                }
            );
        }
        let pages = photos.count + 2;
        let _ = write!(low, "{}/{pages} {}", page + 1, page_name(photos, page));
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

    fn plane(screen: usize) -> Plane {
        Plane {
            format: PixelFormat::Argb8888,
            width: W as u16,
            height: H as u16,
            offset: SCREENS[screen].at as usize,
            stride: STRIDE,
        }
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

    /// The card, its partition mounted, if one is in and starts (read only:
    /// this test never writes the card).
    fn mount_card(gpio: &mut Gpio<Mmio>) -> Option<(Card, u32)> {
        if gpio.read(SD_DETECT) {
            let _ = writeln!(RunLog, "no SD card: the built-in photos");
            return None;
        }
        let mmc = rt::take_mmc()?;
        let mut card = match SdCard::try_init(PxaMmc::new(mmc)) {
            Ok(c) => c,
            Err((e, _)) => {
                let _ = writeln!(RunLog, "card init failed: {}", e.name());
                return None;
            }
        };
        let mut block = [0u8; BLOCK];
        if card.read(0, &mut block).is_err() {
            let _ = writeln!(RunLog, "card block 0 unreadable");
            return None;
        }
        let start = mbr_partitions(&block)
            .and_then(|p| p[0])
            .map_or(0, |p| p.start);
        Some((card, start))
    }

    /// The folder's photos, in name order (exactly one 4:2:0 screen each).
    fn list(vol: &mut Volume<&mut Card>) -> Photos {
        let mut p = Photos {
            names: [[0; NAME]; MAX_PHOTOS],
            lens: [0; MAX_PHOTOS],
            count: 0,
            card: true,
        };
        let seen = vol.for_each_in(DIR, |e| {
            let yuv = e.name.len() > 4 && e.name[e.name.len() - 4..].eq_ignore_ascii_case(b".yuv");
            if !e.is_dir
                && yuv
                && e.size as usize == YUV_BYTES
                && e.name.len() <= NAME
                && p.count < MAX_PHOTOS
            {
                p.names[p.count][..e.name.len()].copy_from_slice(e.name);
                p.lens[p.count] = e.name.len();
                p.count += 1;
            }
        });
        if let Err(e) = seen {
            let _ = writeln!(
                RunLog,
                "no {DIR} folder ({}): the built-in photos",
                e.name()
            );
            p.count = 0;
        }
        // Name order (insertion sort: a few dozen at most).
        for i in 1..p.count {
            let mut j = i;
            while j > 0 && p.names[j - 1][..p.lens[j - 1]] > p.names[j][..p.lens[j]] {
                p.names.swap(j - 1, j);
                p.lens.swap(j - 1, j);
                j -= 1;
            }
        }
        p
    }

    /// Reads photo `i` from the card into `out`.
    fn read_photo(vol: &mut Volume<&mut Card>, photos: &Photos, i: usize, out: &mut [u8]) -> bool {
        let mut path = Line::new();
        let _ = write!(path, "{DIR}/{}", photos.name(i));
        let Ok(file) = vol.find(path.as_str()) else {
            let _ = writeln!(RunLog, "{} not found", path.as_str());
            return false;
        };
        let mut scratch = [0u8; 8 * BLOCK];
        let mut at = 0usize;
        let read = vol.read_file(&file, &mut scratch, |chunk| {
            let n = chunk.len().min(out.len() - at);
            out[at..at + n].copy_from_slice(&chunk[..n]);
            at += n;
        });
        if read.is_err() || at != YUV_BYTES {
            let _ = writeln!(RunLog, "{}: read failed ({at} bytes)", path.as_str());
            return false;
        }
        true
    }

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let mut clock = rt::Elapsed::start();
        let (Some((regs, vram)), Some(mut pwm), Some(mut gpio)) = (
            devices::take_mbx(),
            rt::take_pwm0(),
            rt::take_gpio().map(Gpio::new),
        ) else {
            halt()
        };
        let mut display = Display::new(regs);
        let _ = display.adopt(MBX_VRAM_SIZE as usize);
        let before = Snapshot::take(display.regs());
        let gsctrl = display.regs().read32(GSCTRL);
        let found: [u32; GAMMA_ENTRIES] =
            core::array::from_fn(|i| display.regs().read32(GFXGAM0 + 4 * i));
        let light_found = (pwm.read32(CR), pwm.read32(DCR), pwm.read32(PCR));
        let _ = writeln!(
            RunLog,
            "GSCTRL {gsctrl:08X}; PWM0 found: CR {:02X} DCR {:03X} PCR {:03X}",
            light_found.0, light_found.1, light_found.2
        );
        log_table("gamma found", &found);

        // The photo scratch above the two screens: the built-in photos
        // unpacked there, or each card photo read into its first slot when
        // its page comes up.
        let (screens, high) = vram.split_at_mut(CACHE.at as usize);
        let (slot0, rest) = high.split_at_mut(YUV_BYTES);
        let slot1 = &mut rest[..YUV_BYTES];
        let mut mounted = mount_card(&mut gpio);
        let mut vol = mounted
            .as_mut()
            .and_then(|(card, start)| Volume::mount(card, *start).ok());
        let mut photos = match vol.as_mut() {
            Some(v) => list(v),
            None => Photos {
                names: [[0; NAME]; MAX_PHOTOS],
                lens: [0; MAX_PHOTOS],
                count: 0,
                card: false,
            },
        };
        if photos.count == 0 {
            photos.card = false;
            photos.count = BUILT_IN.len();
            for (photo, out) in PHOTOS.iter().zip([&mut *slot0, &mut *slot1]) {
                if let Err(e) = unpack(photo, out) {
                    crash_out(format_args!("a built-in photo is damaged: {e:?}"))
                }
            }
        }
        let _ = writeln!(
            RunLog,
            "{} photos from {}",
            photos.count,
            if photos.card { "the card" } else { "the image" }
        );
        // Which photo slot 0 holds (card photos), so a page shows its own.
        let mut in_slot: Option<usize> = None;
        let screen_at = |s: usize| SCREENS[s].at as usize;

        let (mut s, mut page, mut original, mut light) = (START, 0usize, false, Light::Full);
        let mut screen = 0usize;

        let t0 = clock.ms();
        // Shows the current state: reads the page's photo if it is a card
        // photo not yet in the slot, draws into the hidden screen, shows it.
        macro_rules! show {
            () => {{
                let kind = page_kind(&photos, page);
                if kind == Page::Photo && photos.card && in_slot != Some(page) {
                    let ok = vol
                        .as_mut()
                        .is_some_and(|v| read_photo(v, &photos, page, &mut *slot0));
                    in_slot = ok.then_some(page);
                    if !ok {
                        slot0.fill(0);
                    }
                }
                let photo: &[u8] = if photos.card || page == 0 {
                    &*slot0
                } else {
                    &*slot1
                };
                let t = if original {
                    found
                } else {
                    dial_table(s, &found)
                };
                screen ^= 1;
                let fb = &mut screens[screen_at(screen)..screen_at(screen) + PAGE_BYTES];
                draw(fb, kind, photo, &t, 0..H);
                caption(fb, s, light, original, &photos, page);
                display.set_graphics(&plane(screen), Layer::SOLO);
                set_light(
                    &mut pwm,
                    if original { Light::Found } else { light },
                    light_found,
                );
            }};
        }
        show!();
        write_gamma(display.regs(), &straight());
        let on = display.regs().read32(GSCTRL) | GSCTRL_GAMMA;
        display.regs().write32(GSCTRL, on);
        let _ = writeln!(RunLog, "page drawn in {} ms", clock.ms() - t0);

        let Some(kp) = rt::take_keypad() else {
            crash_out(format_args!("no keypad"))
        };
        let mut keypad = Keypad::new(kp);
        keypad.enable();
        keypad.acknowledge();
        let mut keys = Keys::new(Timing::DEFAULT);
        let _ = writeln!(RunLog, "ready: strength 2.00, {}", page_name(&photos, page));

        let mut last_input = clock.ms();
        let mut caption_until = Some(last_input + CAPTION_MS);
        let mut changes = 0u32;
        // Milliseconds at each strength (full light), at WM light (any
        // strength), and on the original; since when the current one.
        let mut seen = [0u64; STEPS];
        let (mut seen_wm, mut seen_original) = (0u64, 0u64);
        let mut since = last_input;
        let reason = loop {
            rt::check_in();
            rt::delay_us(10_000);
            let now = clock.ms();
            if now - last_input > IDLE_MS {
                break "no input for 10 minutes";
            }
            let (mut by, mut go, mut flip, mut toggle, mut done) =
                (0i32, 0i32, false, false, false);
            keypad.acknowledge();
            keys.update(now as u32, keypad.pressed(), &mut |e| {
                if matches!(e.kind, Kind::Down) {
                    match e.button {
                        Button::Right => by += 1,
                        Button::Left => by -= 1,
                        Button::Down => go += 1,
                        Button::Up => go -= 1,
                        Button::Enter => flip = !flip,
                        Button::Calendar => toggle = !toggle,
                        Button::Home => done = true,
                        _ => {}
                    }
                }
            });
            if done {
                break "Home";
            }
            if by != 0 || go != 0 || flip || toggle {
                let spent = now - since;
                if original {
                    seen_original += spent;
                } else if light == Light::Found {
                    seen_wm += spent;
                } else {
                    seen[s] += spent;
                }
                changes += 1;
                if by != 0 {
                    s = (s as i32 + by).clamp(0, STEPS as i32 - 1) as usize;
                    original = false;
                }
                if go != 0 {
                    page = (page as i32 + go).rem_euclid(photos.count as i32 + 2) as usize;
                }
                if toggle {
                    light = if light == Light::Full {
                        Light::Found
                    } else {
                        Light::Full
                    };
                    original = false;
                }
                original ^= flip;
                show!();
                let after = clock.ms();
                since = after;
                last_input = after;
                caption_until = Some(after + CAPTION_MS);
                let st = strength(s);
                let _ = writeln!(
                    RunLog,
                    "at {now} ms ({} ms): {}.{:02} {}{}, {}",
                    after - now,
                    st / 100,
                    st % 100,
                    if light == Light::Full {
                        "full"
                    } else {
                        "WM light"
                    },
                    if original {
                        " (showing the original)"
                    } else {
                        ""
                    },
                    page_name(&photos, page)
                );
            }
            if caption_until.is_some_and(|t| now >= t) {
                caption_until = None;
                let t = if original {
                    found
                } else {
                    dial_table(s, &found)
                };
                let photo: &[u8] = if photos.card || page == 0 {
                    &*slot0
                } else {
                    &*slot1
                };
                let fb = &mut screens[screen_at(screen)..screen_at(screen) + PAGE_BYTES];
                draw(fb, page_kind(&photos, page), photo, &t, 0..CAPTION_H);
            }
        };
        let now = clock.ms();
        let spent = now - since;
        if original {
            seen_original += spent;
        } else if light == Light::Found {
            seen_wm += spent;
        } else {
            seen[s] += spent;
        }

        let _ = writeln!(
            RunLog,
            "done ({reason}) after {changes} changes; seconds at each strength (full light):"
        );
        for (i, ms) in seen.iter().enumerate() {
            let st = strength(i);
            let _ = writeln!(RunLog, "  {}.{:02}: {} s", st / 100, st % 100, ms / 1000);
        }
        let _ = writeln!(
            RunLog,
            "  WM light: {} s; original: {} s",
            seen_wm / 1000,
            seen_original / 1000
        );
        let st = strength(s);
        let _ = writeln!(
            RunLog,
            "last: {}.{:02}, {}",
            st / 100,
            st % 100,
            page_name(&photos, page)
        );
        drop(vol);
        drop(mounted);
        write_gamma(display.regs(), &found);
        pwm.write32(CR, light_found.0);
        pwm.write32(PCR, light_found.2);
        pwm.write32(DCR, light_found.1);
        // The planes as found (GSCTRL's gamma bit with them).
        before.restore(display.regs());
        display.regs().write32(GSCTRL, gsctrl);
        let _ = writeln!(RunLog, "{}", super::MARKER);
        if devices::loader_present() {
            devices::return_to_loader();
        }
        halt()
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
