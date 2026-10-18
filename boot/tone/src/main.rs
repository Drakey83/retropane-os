//! Audio check (docs/hw/audio.md): what the loader's `tone` command did,
//! as a test image of its own, so the loader doesn't carry the audio drivers
//! (the loader diet, 2026-10-16).
//!
//! 1. The WM8750 is looked for on I2C (at 0x1A, with its I2C gate,
//!    GPIO82, open for each write as Windows Mobile does: docs/hw/audio.md)
//!    and routed to the speaker or the headphones by the jack (GPIO96, low
//!    = plugged in).
//! 2. 440 Hz for 500 ms at 48 kHz, streamed through I2S by DMA from a ring
//!    the CPU refills by halves; the log says how long it really took (the
//!    codec's clock paces it) and how much of that the CPU was busy.
//! 3. Up to 15 s for the jack to change (plug the headphones in or out);
//!    on a change, 1000 Hz for 50 ms to the new output.
//!
//! The speaker amplifier (unknown: CPLD `0x14` bit 3 is a candidate) is
//! left alone.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed once everything was reported.
pub const MARKER: &str = "RETROPANE-TONE-OK";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_mbx_display::console::Console;
    use rp_drv_mbx_display::{Display, crash};
    use rp_drv_pxa_gpio::Gpio;
    use rp_drv_pxa_i2c::PxaI2c;
    use rp_drv_pxa_i2s::{PxaI2s, Rate, StreamError, Tone, stream};
    use rp_drv_wm8750::{Gate, Gated, Output, Wm8750};
    use rp_hal_pxa27x::dma::Dma;
    use rp_hal_pxa27x::{Mmio, dma_map, rt};
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices;

    rp_hal_pxa27x::entry!(main);

    /// Headphone jack (docs/hw/audio.md, Dell's driver): low = plugged in
    /// (likely; the level is logged).
    const HP_DETECT: u8 = 96;
    /// The codec's I2C gate: high for each transfer, low after.
    const I2C_GATE: u8 = 82;

    /// The GPIO block, holding the codec's I2C gate.
    struct Pins(Gpio<Mmio>);

    impl Gate for Pins {
        fn set(&mut self, open: bool) {
            self.0.write(I2C_GATE, open);
        }
    }

    type Codec = Wm8750<Gated<PxaI2c<Mmio>, Pins>>;
    /// OS timer ticks per second (3.25 MHz).
    const TICK_1S: u32 = 3_250_000;
    /// How long to wait for the jack to change.
    const JACK_WAIT_S: u32 = 15;
    /// The streaming ring (the CPU refills it by halves).
    const RING: usize = 8192;

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

    fn output(plugged: bool) -> Output {
        if plugged {
            Output::Headphones
        } else {
            Output::Speaker
        }
    }

    /// Plays `hz` for `ms` and logs how it went.
    fn play(
        out: &mut impl Write,
        codec: &Codec,
        i2s: &mut PxaI2s<Mmio>,
        dma: &mut Dma,
        ring: &mut [u8],
        hz: u32,
        ms: u32,
    ) {
        let rate = Rate::Khz48;
        let frames = rate.hz() / 1000 * ms;
        let mut source = Tone::new(hz, rate.hz(), 12_000);
        i2s.start(rate);
        let r = stream(
            dma,
            ring,
            &mut source,
            frames,
            &mut rt::ticks,
            TICK_1S / 5,
            &mut rt::check_in,
        );
        i2s.stop();
        match r {
            Ok(s) => {
                let _ = writeln!(
                    out,
                    "tone {hz} Hz {ms} ms ({frames} frames in {} ms, by DMA, CPU busy {}%), codec at {:#04x}, to {}",
                    s.total / u64::from(TICK_1S / 1000),
                    s.busy * 100 / s.total.max(1),
                    codec.address(),
                    if codec.output() == Output::Headphones {
                        "headphones"
                    } else {
                        "speaker"
                    }
                );
            }
            Err(e) => {
                let why = match e {
                    StreamError::Refused => "refused",
                    StreamError::Stalled => "stalled",
                };
                let _ = writeln!(out, "tone: DMA stream {why}");
            }
        }
    }

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let Some((regs, vram)) = devices::take_mbx() else {
            halt()
        };
        let (plane, _) = Display::new(regs).adopt(MBX_VRAM_SIZE as usize);
        let end = plane.offset + plane.stride * usize::from(plane.height);
        let mut con = Console::new(
            &mut vram[plane.offset..end],
            plane.stride,
            (0, 0),
            (480, 640),
            2,
            (0xFFFF, 0x0000),
        );
        let mut out = Tee(&mut con);
        let _ = writeln!(out, "RetroPane 26 tone");
        let (Some(gpio_bus), Some(scratch)) = (rt::take_gpio(), devices::take_scratch()) else {
            let _ = writeln!(out, "tone: GPIO or scratch unavailable");
            finish()
        };
        let mut gpio = Gpio::new(gpio_bus);
        gpio.make_input(HP_DETECT);
        // Closed until a write opens it.
        gpio.make_output(I2C_GATE, false);
        let level = gpio.read(HP_DETECT);
        let mut plugged = !level;
        let _ = writeln!(
            out,
            "headphones {} (GPIO96 {})",
            if plugged { "in" } else { "out" },
            u8::from(level)
        );
        let (Some(i2c), Some(i2s)) = (rt::take_i2c(), rt::take_i2s()) else {
            let _ = writeln!(out, "tone: I2C or I2S unavailable");
            finish()
        };
        let gated = Gated {
            bus: PxaI2c::new(i2c),
            gate: Pins(gpio),
        };
        let mut codec: Codec = match Wm8750::init(gated, 60) {
            Ok(c) => c,
            Err(_) => {
                let _ = writeln!(
                    out,
                    "tone: no codec answered on I2C (0x1a, 0x1b) with GPIO82 open"
                );
                finish()
            }
        };
        let _ = writeln!(out, "codec at {:#04x}", codec.address());
        let _ = codec.route(output(plugged));
        let Some(mut dma) = rt::take_dma(dma_map::AUDIO) else {
            let _ = writeln!(out, "tone: no DMA channel");
            finish()
        };
        let mut i2s = PxaI2s::new(i2s);
        let ring = &mut scratch[..RING];
        play(&mut out, &codec, &mut i2s, &mut dma, ring, 440, 500);

        // The jack: a change that holds over two polls 1 s apart.
        let _ = writeln!(out, "PLUG HEADPHONES IN OR OUT NOW ({JACK_WAIT_S} s)");
        let mut seen = plugged;
        let mut changed = false;
        for _ in 0..JACK_WAIT_S {
            rt::check_in();
            rt::delay_us(1_000_000);
            let now = !codec.bus_mut().gate.0.read(HP_DETECT);
            if now == seen && now != plugged {
                plugged = now;
                changed = true;
                break;
            }
            seen = now;
        }
        if changed {
            let _ = writeln!(
                out,
                "headphones {} (GPIO96 {})",
                if plugged { "in" } else { "out" },
                u8::from(!plugged)
            );
            let _ = codec.route(output(plugged));
            play(&mut out, &codec, &mut i2s, &mut dma, ring, 1000, 50);
        } else {
            let _ = writeln!(out, "jack: no change in {JACK_WAIT_S} s");
        }
        finish()
    }

    fn finish() -> ! {
        let _ = writeln!(devices::RunLog, "{}", super::MARKER);
        if devices::loader_present() {
            rt::delay_us(1_000_000);
            devices::return_to_loader();
        }
        halt()
    }

    /// Writes to the screen and the run log.
    struct Tee<'a, 'b>(&'a mut Console<'b>);

    impl Write for Tee<'_, '_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let _ = self.0.write_str(s);
            devices::log_write(s.as_bytes());
            Ok(())
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
