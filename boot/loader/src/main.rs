//! RetroPane's resident loader (ADR-0012 step U3).
//!
//! Started once by HaRET, it moves itself to its reserved home at
//! `0xA8400000` and waits for an image from the PC, over the USB serial port
//! (the device) or the FFUART (the emulator, and an optional cable). A
//! complete, CRC-checked image goes to `0xA8008000` and starts exactly as if
//! HaRET had started it. So a new build reaches the device in about a second,
//! with nobody copying files or tapping HaRET.
//!
//! Protocol: `rp-loadproto`. PC side: `cargo xtask load <image>`.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed on serial once the loader is waiting (also the READY line).
pub const MARKER: &str = "RPLOAD1 READY";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_mbx_display::console::Console;
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, X51V_VGA, reg};
    use rp_drv_pxa_gpio::Gpio;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_pxa_udc::Udc;
    use rp_hal_api::RegisterBus;
    use rp_hal_pxa27x::{handoff, rt};
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::{self, IMAGE_BASE};
    use rp_loadproto::{LoadError, OK, READY, Receiver, feed_slice};

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
                Some(Ok(len)) => {
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
                    if pending.is_none() {
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

    /// A line to the host on both transports.
    fn send(uart: &mut PxaUart<rp_hal_pxa27x::Mmio>, udc: &mut Udc<rp_hal_pxa27x::Mmio>, s: &str) {
        let _ = uart.write_str(s);
        udc.write(s.as_bytes());
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
