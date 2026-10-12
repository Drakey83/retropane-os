//! RetroPane's resident loader (ADR-0012 step U3).
//!
//! Started once by HaRET, it moves itself to its reserved home at
//! `0xA8400000` and waits for an image from the PC, over the USB serial port
//! (the device) or the FFUART (the emulator, and an optional cable). A
//! complete, CRC-checked image goes to `0xA8008000` and starts exactly as if
//! HaRET had started it. So a new build reaches the device in about a second,
//! with nobody copying files or tapping HaRET.
//!
//! A file frame instead replaces a file on the SD card (ADR-0013): the
//! loader itself, HaRET, its script. Copy-then-swap in `rp-fat`, then the
//! contents are read back and checked against the frame's CRC before the
//! loader says OK, so what the PC sent is what the card holds.
//!
//! Protocol: `rp-loadproto`. PC side: `cargo xtask load <image>`,
//! `cargo xtask put <file>`.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed on serial once the loader is waiting (also the READY line).
pub const MARKER: &str = "RPLOAD1 READY";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_cpld::Cpld;
    use rp_drv_mbx_display::console::Console;
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, X51V_VGA, reg};
    use rp_drv_pxa_gpio::Gpio;
    use rp_drv_pxa_mmc::PxaMmc;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_pxa_udc::Udc;
    use rp_drv_sdcard::{SdCard, mbr_partitions};
    use rp_fat::Volume;
    use rp_hal_api::RegisterBus;
    use rp_hal_pxa27x::{Mmio, handoff, rt};
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::{self, IMAGE_BASE};
    use rp_loadproto::{Frame, Kind, LoadError, OK, READY, Receiver, crc32_update, feed_slice};

    rp_hal_pxa27x::entry!(main);

    /// D+ pull-up and cable detect (docs/hw/udc.md).
    const USB_PULLUP: u8 = 2;
    const USB_DETECT: u8 = 14;
    /// OS timer ticks (3.25 MHz) between READY lines and status redraws.
    const TICK_1S: u32 = 3_250_000;
    const STATUS_H: usize = 6 * 16;
    /// How long a loaded image may run without returning or feeding the
    /// watchdog (the probe, the slowest, takes about 15 s on the device).
    const RUN_LIMIT: u32 = 30 * TICK_1S;
    /// The end of the image area, kept for searching the FAT for free
    /// clusters and for reading a written file back (the bigger it is, the
    /// fewer card commands each takes).
    const SCRATCH: usize = 64 * 1024;

    /// The SD card's FAT32 volume, opened on the first file frame.
    type Card = Volume<SdCard<PxaMmc<Mmio>>>;

    /// A reply line built on the stack.
    struct Line {
        buf: [u8; 96],
        len: usize,
    }

    impl Line {
        const fn new() -> Self {
            Self {
                buf: [0; 96],
                len: 0,
            }
        }
        fn as_str(&self) -> &str {
            core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
        }
    }

    impl Write for Line {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let n = s.len().min(self.buf.len() - self.len);
            self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
            self.len += n;
            Ok(())
        }
    }

    /// Powers the card, identifies it and mounts its first partition.
    #[inline(never)]
    fn open_card() -> Result<Card, &'static str> {
        let cpld = devices::take_cpld().ok_or("no CPLD")?;
        let mmc = rt::take_mmc().ok_or("no MMC controller")?;
        let mut cpld = Cpld::new(cpld);
        cpld.set_sd_power(true);
        rt::delay_us(10_000);
        let mut card = SdCard::init(PxaMmc::new(mmc)).map_err(|_| "no card")?;
        let mut block = [0u8; rp_drv_sdcard::BLOCK];
        card.read(0, &mut block).map_err(|_| "card read")?;
        let start = mbr_partitions(&block)
            .and_then(|p| p[0])
            .map_or(0, |p| p.start);
        Volume::mount(card, start).map_err(|_| "not FAT32")
    }

    /// Writes a file frame to the card and checks it back; the reply line.
    #[inline(never)]
    fn put_file(card: &mut Option<Card>, frame: Frame, dest: &mut [u8], reply: &mut Line) {
        let (payload, scratch) = dest.split_at_mut(dest.len() - SCRATCH);
        if frame.len > payload.len() {
            let _ = reply.write_str(LoadError::TooBig.reply());
            return;
        }
        let (name, contents) = match frame.file(payload) {
            Ok(x) => x,
            Err(e) => {
                let _ = reply.write_str(e.reply());
                return;
            }
        };
        if card.is_none() {
            match open_card() {
                Ok(c) => *card = Some(c),
                Err(why) => {
                    let _ = write!(reply, "RPL ERR card: {why}\r\n");
                    return;
                }
            }
        }
        let Some(vol) = card.as_mut() else { return };
        let t0 = rt::ticks();
        let reads0 = vol.device().reads();
        let written = match vol.replace(name, contents, scratch) {
            Ok(w) => w,
            Err(e) => {
                let _ = write!(reply, "RPL ERR {}{}\r\n", e.name(), card_note(vol));
                return;
            }
        };
        let t_write = rt::ticks().wrapping_sub(t0);
        let reads = vol.device().reads().wrapping_sub(reads0);
        // Read back through the FAT code and compare with what was sent.
        let mut crc = !0u32;
        let mut got = 0usize;
        let check = vol.find(name).and_then(|f| {
            vol.read_file(&f, scratch, |chunk| {
                crc = crc32_update(crc, chunk);
                got += chunk.len();
            })
        });
        let want = crc32_update(!0, contents);
        match check {
            Ok(()) if crc == want && got == contents.len() => {
                let _ = write!(
                    reply,
                    "RPL OK {} bytes in {} ms, {} cluster{} {} ({} reads), read back in {} ms\r\n",
                    contents.len(),
                    t_write / (TICK_1S / 1000),
                    written.clusters,
                    if written.clusters == 1 { "" } else { "s" },
                    if written.contiguous {
                        "contiguous"
                    } else {
                        "scattered"
                    },
                    reads,
                    rt::ticks().wrapping_sub(t0 + t_write) / (TICK_1S / 1000),
                );
            }
            Ok(()) => {
                let _ = write!(
                    reply,
                    "RPL ERR verify: {got} bytes read back, crc differs\r\n"
                );
            }
            Err(e) => {
                let _ = write!(reply, "RPL ERR verify: {}{}\r\n", e.name(), card_note(vol));
            }
        }
    }

    /// The card's own error behind a device error, for the reply.
    fn card_note(vol: &Card) -> Note {
        let what = match vol.device().last_error() {
            Some(rp_drv_sdcard::Error::Host(rp_drv_sdcard::HostError::Timeout)) => "timeout",
            Some(rp_drv_sdcard::Error::Host(rp_drv_sdcard::HostError::Crc)) => "CRC",
            Some(rp_drv_sdcard::Error::Host(rp_drv_sdcard::HostError::Data)) => "data",
            Some(rp_drv_sdcard::Error::CardStatus(_)) => "status",
            Some(_) => "error",
            None => "",
        };
        Note {
            what,
            failure: vol.device().host().last_failure(),
        }
    }

    /// `card_note`'s text: the error and, for a timeout, which command and
    /// the status register it left.
    struct Note {
        what: &'static str,
        failure: Option<(u8, u32)>,
    }

    impl core::fmt::Display for Note {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            if self.what.is_empty() {
                return Ok(());
            }
            write!(f, " (card: {}", self.what)?;
            if let Some((cmd, stat)) = self.failure {
                write!(f, " on CMD{cmd} stat {stat:#x}")?;
            }
            f.write_str(")")
        }
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    #[inline(never)]
    fn crash(msg: core::fmt::Arguments) -> ! {
        let mut uart = PxaUart::new(rt::emergency_uart());
        let _ = writeln!(uart, "\n{msg}");
        if let Some(vram) = devices::emergency_vram() {
            let plane = Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565);
            let mut c = Console::new(
                &mut vram[plane.offset..],
                plane.stride,
                (0, 576),
                (480, 64),
                2,
                (0xFFFF, 0xA800),
            );
            let _ = write!(c, "{msg}");
        }
        halt()
    }

    /// Start-up colours (as in rp-light): a hang leaves the last one on
    /// screen. Red: main entered (the relocation worked); yellow: FFUART
    /// set up; green: MMU on; cyan: 2700G answered.
    #[inline(never)]
    fn step(i: usize) {
        const COLOURS: [u16; 4] = [0xF800, 0xFFE0, 0x07E0, 0x07FF];
        devices::beacon(COLOURS[i], COLOURS[i]);
    }

    fn main() -> ! {
        step(0);
        let Some(bus) = rt::take_ffuart() else { halt() };
        let mut uart = PxaUart::new(bus);
        uart.init(115_200);
        step(1);
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        step(2);

        let Some((mut regs, vram)) = devices::take_mbx() else {
            halt()
        };
        step(3);
        let plane = Plane::decode(
            regs.read32(reg::GSCTRL),
            regs.read32(reg::GSCADR),
            regs.read32(reg::GSADR),
            MBX_VRAM_SIZE as usize,
        )
        .filter(|p| p.format == PixelFormat::Rgb565 && p.width == 480 && p.height == 640)
        .unwrap_or_else(|| {
            let p = Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565);
            Display::new(regs).enable(&X51V_VGA, &p);
            p
        });
        let end = plane.offset + plane.stride * usize::from(plane.height);
        let (top, rest) = vram[plane.offset..end].split_at_mut(plane.stride * STATUS_H);
        let mut log = Console::new(
            rest,
            plane.stride,
            (0, 0),
            (480, 640 - STATUS_H),
            2,
            (0xFFFF, 0x0000),
        );

        let Some(dest) = devices::take_image_area() else {
            let _ = writeln!(log, "loader not at its home address");
            halt()
        };

        let Some(gpio_bus) = rt::take_gpio() else {
            halt()
        };
        let mut gpio = Gpio::new(gpio_bus);
        gpio.make_input(USB_DETECT);
        gpio.make_output(USB_PULLUP, false);
        rt::delay_us(200_000);
        let Some(udc_bus) = rt::take_udc() else {
            halt()
        };
        let mut udc = Udc::new(udc_bus);
        let usb = udc.enable().is_ok();
        if usb {
            gpio.write(USB_PULLUP, true);
        }
        let _ = writeln!(log, "RetroPane loader at {:#x}", devices::LOADER_BASE);
        let _ = writeln!(log, "USB {}", if usb { "connected" } else { "unavailable" });
        let _ = writeln!(log, "waiting for an image: cargo xtask load");

        // The previous run's log (an image this loader started, now back):
        // to the FFUART at once, and over USB once the PC has the port open
        // again (the device reconnected when it came back).
        let mut pending = devices::log_take();
        if let Some(run) = &pending {
            let how = if run.returned {
                "returned"
            } else {
                "did NOT return"
            };
            let _ = writeln!(log, "previous run {how}: {} bytes of log", run.log.len());
            send_log(&mut |b| uart_bytes(&mut uart, b), run);
        }

        let mut rx = Receiver::new();
        let mut last = rt::ticks().wrapping_sub(TICK_1S);
        let mut buf = [0u8; 64];
        let mut watchdog = rt::watchdog();
        let mut card: Option<Card> = None;
        loop {
            // Armed by an earlier run (it can't be disarmed): keep it fed
            // while waiting.
            if watchdog.armed() {
                watchdog.feed(RUN_LIMIT);
            }
            udc.poll();
            if udc.host_open() {
                if let Some(run) = pending.take() {
                    send_log(&mut |b| usb_bytes(&mut udc, b), &run);
                }
            }
            let mut outcome = None;
            while outcome.is_none() {
                let Some(b) = uart.read_byte() else { break };
                outcome = feed_slice(&mut rx, &[b], dest).1;
            }
            if outcome.is_none() {
                let n = udc.read(&mut buf);
                let mut used = 0;
                while outcome.is_none() && used < n {
                    let (u, r) = feed_slice(&mut rx, &buf[used..n], dest);
                    used += u;
                    outcome = r;
                }
            }
            match outcome {
                Some(Ok(Frame {
                    kind: Kind::File, ..
                })) => {
                    let frame = outcome.and_then(Result::ok).unwrap_or(Frame {
                        kind: Kind::File,
                        len: 0,
                    });
                    // A card write can take seconds: keep the watchdog fed.
                    if watchdog.armed() {
                        watchdog.feed(RUN_LIMIT);
                    }
                    let mut reply = Line::new();
                    put_file(&mut card, frame, dest, &mut reply);
                    send(&mut uart, &mut udc, reply.as_str());
                    let _ = writeln!(log, "{}", reply.as_str().trim_ascii_end());
                }
                Some(Ok(Frame { len, .. })) => {
                    // Bring-up check: does the image survive the cache flush
                    // the launch does? Flush, drop the cache, re-read from
                    // memory and compare with the CRC the sender gave.
                    rp_hal_pxa27x::cpu::clean_invalidate_all();
                    let intact = rp_loadproto::crc32(&dest[..len]) == rx.expected_crc();
                    send(&mut uart, &mut udc, OK);
                    let _ = writeln!(log, "image OK, {len} bytes: starting");
                    // Let the reply leave over USB before disconnecting: on
                    // the device, 50 ms was not enough for the PC's program to
                    // read it before the port went away.
                    let t = rt::ticks();
                    while rt::ticks().wrapping_sub(t) < TICK_1S / 2 {
                        udc.poll();
                    }
                    gpio.write(USB_PULLUP, false);
                    udc.disable();
                    devices::log_start();
                    // The verdict goes at the top of the image's run log, so
                    // it reaches the PC whatever the image then does.
                    devices::log_write(if intact {
                        b"[loader: image intact after the cache flush]
"
                    } else {
                        b"[loader: image CHANGED after the cache flush]
"
                    });
                    // A run limit: an image that hangs (never returns, never
                    // feeds the watchdog) resets the device, and the loader
                    // comes back (started again by Windows Mobile's StartUp
                    // folder on the device; the emulator restarts it).
                    watchdog.feed(RUN_LIMIT);
                    handoff::launch(IMAGE_BASE as u32);
                }
                Some(Err(e)) => {
                    send(&mut uart, &mut udc, e.reply());
                    let _ = writeln!(log, "refused: {}", what(e));
                }
                None => {}
            }
            if rt::ticks().wrapping_sub(last) >= TICK_1S {
                last = rt::ticks();
                if rx.progress().is_none() {
                    // Over USB only once the previous run's log has gone,
                    // so the PC can tell "back with a log" from "restarted
                    // without one" (a hang whose log the reset wiped).
                    let _ = uart.write_str(READY);
                    // At most one READY waits in the USB ring: while no
                    // program on the PC reads the port nothing drains, and
                    // a ring full of stale READYs would cut the reply to the
                    // next frame short (seen on the device after an hour
                    // idle: 36 old lines, one torn, and the reply lost).
                    if pending.is_none() && udc.tx_pending() == 0 {
                        udc.write(READY.as_bytes());
                    }
                }
                let s = udc.stats();
                let mut st = Console::new(
                    &mut *top,
                    plane.stride,
                    (0, 0),
                    (480, STATUS_H),
                    2,
                    (0x0000, 0x07FF),
                );
                let _ = writeln!(st, "RetroPane 26 loader");
                let _ = writeln!(
                    st,
                    "cable {} configured {} open {}",
                    u8::from(gpio.read(USB_DETECT)),
                    yes(udc.configured()),
                    yes(udc.host_open())
                );
                let _ = writeln!(st, "usb resets {} setups {}", s.resets, s.setups);
                match rx.progress() {
                    Some((got, len)) => {
                        let _ = writeln!(st, "receiving {got}/{len}");
                    }
                    None => {
                        let _ = writeln!(st, "idle");
                    }
                }
            }
        }
    }

    /// A line to the host on both transports; over USB it goes whole, the
    /// ring draining as the PC reads (never cut short to fit).
    fn send(uart: &mut PxaUart<rp_hal_pxa27x::Mmio>, udc: &mut Udc<rp_hal_pxa27x::Mmio>, s: &str) {
        let _ = uart.write_str(s);
        usb_bytes(udc, s.as_bytes());
    }

    /// The run log, framed for `cargo xtask load`; a run that didn't
    /// return (a hang the watchdog reset) says so before the end marker.
    fn send_log(out: &mut dyn FnMut(&[u8]), run: &devices::PreviousRun) {
        out(b"RPL LOG\r\n");
        out(run.log);
        if run.overflow {
            out(b"\r\n[run log full: later output dropped]");
        }
        if !run.returned {
            out(b"\r\n[the image did not return: it hung and the device was reset]");
        }
        out(b"\r\nRPL END\r\n");
    }

    fn uart_bytes(uart: &mut PxaUart<rp_hal_pxa27x::Mmio>, data: &[u8]) {
        for &b in data {
            uart.write_byte(b);
        }
    }

    /// All of `data` over USB, polling as the transmit ring drains (gives
    /// up after 5 s or if the PC goes away).
    fn usb_bytes(udc: &mut Udc<rp_hal_pxa27x::Mmio>, mut data: &[u8]) {
        let start = rt::ticks();
        while !data.is_empty() && udc.configured() && rt::ticks().wrapping_sub(start) < 5 * TICK_1S
        {
            let n = udc.write(data);
            data = &data[n..];
            udc.poll();
        }
    }

    fn what(e: LoadError) -> &'static str {
        match e {
            LoadError::TooBig => "too big",
            LoadError::BadCrc => "bad CRC",
            LoadError::BadName => "bad file name",
        }
    }

    fn yes(b: bool) -> &'static str {
        if b { "yes" } else { "no" }
    }

    fn halt() -> ! {
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        match info.location() {
            Some(l) => crash(format_args!("PANIC {}:{}", l.file(), l.line())),
            None => crash(format_args!("PANIC")),
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
