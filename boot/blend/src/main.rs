//! Test image: plane blending at scan-out (overnight item 60). The
//! wallpaper goes in the 2700G's video plane (YUV), a glass panel with text
//! in the graphics plane with per-pixel alpha, and the display controller
//! mixes them as it scans out. Six stages ([`blendtest`]), each labelled
//! on screen and held [`STAGE_SECONDS`] so the owner can photograph it:
//! A video 4:2:2 + UI ARGB4444, B 4:2:0 + ARGB4444, C 4:2:0 + ARGB8888,
//! D as C with the video gamma bit off, E 4:2:0 + dithered ARGB4444, F as
//! E but drawn through `rp-gfx` (a RAM layer in the scratch region, dirty
//! flushes): the OS's own path, which should look identical to E. It
//! checks in with the hardware watchdog, since all six outlast the
//! deadline the loader sets at launch.
//!
//! The run log gets the display registers as found and, per stage, read
//! back after programming (do they hold what was written?). Every register
//! it changes is put back before it returns, and the loader takes the
//! screen with `Display::adopt` anyway, so even a crash here leaves the
//! loader's screen visible. Uses 2700G memory from 1 MiB up only; never
//! touches flash or the cards.
//!
//! [`blendtest`]: rp_drv_mbx_display::blendtest

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed (serial and run log) with the stage letter once a stage is up.
pub const MARKER: &str = "RETROPANE-BLEND-";
/// How long each stage stays up, unless the build sets `RP_BLEND_SECONDS`.
pub const STAGE_SECONDS: u32 = 15;
/// Which stages to show (letters), unless the build sets `RP_BLEND_STAGES`
/// (e.g. `RP_BLEND_STAGES=CDE RP_BLEND_SECONDS=20 cargo xtask load
/// rp-blend` to look at a few for longer).
pub const STAGES: &str = match option_env!("RP_BLEND_STAGES") {
    Some(s) => s,
    None => "ABCDEF",
};

/// The seconds per stage this build uses.
#[must_use]
pub fn stage_seconds() -> u32 {
    option_env!("RP_BLEND_SECONDS")
        .and_then(|s| s.parse().ok())
        .filter(|&n| (1..=60).contains(&n))
        .unwrap_or(STAGE_SECONDS)
}

/// The gate's P9/V5 run (`RP_BLEND_SCALE`, 2026-10-24): stage A's 4:2:2
/// wallpaper shown through the video plane's scaler and placed, each step
/// labelled with what should be seen and held [`stage_seconds`], the
/// display's under/overrun bits counted over it (the unattended part);
/// whether the picture is right is the owner's eye (the watched part).
pub const SCALE: bool = option_env!("RP_BLEND_SCALE").is_some();

/// One scaler step: its name, what to expect, the source (x, y, w, h)
/// inside the 480 x 640 wallpaper, and the output (w, h, x, y, taps).
pub struct ScaleStep {
    /// Line 2 on screen.
    pub name: &'static str,
    /// Line 3 on screen: what the owner should see.
    pub expect: &'static str,
    /// The source rectangle in the wallpaper.
    pub src: (u16, u16, u16, u16),
    /// The output size (0: the source's), place and vertical taps.
    pub out: (u16, u16, u16, u16, u8),
    /// `SPOCTRL` bit 27 as Windows Mobile sets it (`layers::Scale::ur_c`).
    pub ur_c: bool,
}

/// The steps, in order.
pub const SCALE_STEPS: [ScaleStep; 9] = [
    ScaleStep {
        name: "1:1 FULL (CONTROL)",
        expect: "THE WALLPAPER, NORMAL",
        src: (0, 0, 480, 640),
        out: (0, 0, 0, 0, 1),
        ur_c: false,
    },
    ScaleStep {
        name: "QUARTER UP 2X, 4 TAP",
        expect: "TOP LEFT SKY FILLS ALL",
        src: (0, 0, 240, 320),
        out: (480, 640, 0, 0, 4),
        ur_c: false,
    },
    ScaleStep {
        name: "QUARTER UP 2X, 1 TAP",
        expect: "SAME, ROWS BLOCKIER?",
        src: (0, 0, 240, 320),
        out: (480, 640, 0, 0, 1),
        ur_c: false,
    },
    ScaleStep {
        name: "HALF SIZE AT 120,160",
        expect: "SMALL, CENTRED, GREEN AROUND",
        src: (0, 0, 480, 640),
        out: (240, 320, 120, 160, 4),
        ur_c: false,
    },
    ScaleStep {
        name: "3/4 SIZE AT 60,80",
        expect: "3/4 SIZE, GREEN BORDER",
        src: (0, 0, 480, 640),
        out: (360, 480, 60, 80, 4),
        ur_c: false,
    },
    ScaleStep {
        name: "BARS 1:1 AT 240,320",
        expect: "BARS, BOTTOM RIGHT QUARTER",
        src: (0, 400, 240, 240),
        out: (0, 0, 240, 320, 1),
        ur_c: false,
    },
    // Run 1 (2026-10-24): the video stream underran (DINTRS bit 5) on
    // steps 3-5 and never on 2 or 6; Windows Mobile's bit 27 tried on each.
    ScaleStep {
        name: "UP 2X 1 TAP + WM BIT 27",
        expect: "AS STEP 3",
        src: (0, 0, 240, 320),
        out: (480, 640, 0, 0, 1),
        ur_c: true,
    },
    ScaleStep {
        name: "HALF + WM BIT 27",
        expect: "AS STEP 4",
        src: (0, 0, 480, 640),
        out: (240, 320, 120, 160, 4),
        ur_c: true,
    },
    ScaleStep {
        name: "3/4 + WM BIT 27",
        expect: "AS STEP 5",
        src: (0, 0, 480, 640),
        out: (360, 480, 60, 80, 4),
        ur_c: true,
    },
];

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_mbx_display::blendtest::{self, Stage};
    use rp_drv_mbx_display::layers::{self, Snapshot};
    use rp_drv_mbx_display::{Display, reg};
    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_api::RegisterBus;
    use rp_hal_pxa27x::rt;
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::RunLog;

    use rp_image::crash;

    rp_image::entry!(main);

    /// Registers worth recording, as found and per stage.
    const SHOWN: [(&str, usize); 6] = [
        ("VOVRCLK", layers::reg::VOVRCLK),
        ("VSCTRL", layers::reg::VSCTRL),
        ("VSCADR", layers::reg::VSCADR),
        ("VUBASE", layers::reg::VUBASE),
        ("GSCTRL", reg::GSCTRL),
        ("GSCADR", reg::GSCADR),
    ];

    fn record<B: RegisterBus>(regs: &mut B, what: &str) {
        let _ = write!(RunLog, "{what}:");
        for (name, off) in SHOWN {
            let _ = write!(RunLog, " {name} {:08X}", regs.read32(off));
        }
        let _ = writeln!(RunLog);
    }

    /// The label plane (ARGB4444, past stage F's): clear but a dark band
    /// at the top with three lines of big text.
    const LABEL_AT: usize = 0xB0_0000;
    const LABEL_BAND: usize = 96;

    fn label(vram: &mut [u8], lines: [&str; 3]) {
        use rp_drv_mbx_display::font;
        const W: usize = blendtest::W;
        let mut row = [0u8; W * 2];
        for y in 0..blendtest::H {
            for x in 0..W {
                let mut c: u16 = if y < LABEL_BAND { 0xF112 } else { 0 };
                for (k, text) in lines.iter().enumerate() {
                    let (scale, top) = if k == 0 {
                        (3, 6)
                    } else {
                        (2, 36 + (k - 1) * 26)
                    };
                    let (cw, ch) = ((font::WIDTH + 1) * scale, font::HEIGHT * scale);
                    if y < top || y >= top + ch || x < 8 {
                        continue;
                    }
                    if let Some(g) = text.chars().nth((x - 8) / cw) {
                        let (gx, gy) = ((x - 8) % cw / scale, (y - top) / scale);
                        if gx < font::WIDTH
                            && font::glyph(g)[gy] & (1 << (font::WIDTH - 1 - gx)) != 0
                        {
                            c = 0xFFFF;
                        }
                    }
                }
                row[x * 2..x * 2 + 2].copy_from_slice(&c.to_le_bytes());
            }
            let at = LABEL_AT + y * W * 2;
            vram[at..at + W * 2].copy_from_slice(&row);
        }
    }

    /// The P9 steps: the label plane over the scaled wallpaper, the
    /// background dark green so the uncovered screen shows.
    fn scale_steps<B: RegisterBus>(d: &mut Display<B>, vram: &mut [u8]) {
        use rp_drv_mbx_display::layers::{Scale, VideoFormat, VideoPlane};
        use rp_drv_mbx_display::{PixelFormat, Plane, ack_events, event};
        let (_, wall) = Stage::A.planes();
        d.regs().write32(layers::reg::BGCOLOR, 0x0000_6020);
        let n = super::SCALE_STEPS.len();
        for (i, step) in super::SCALE_STEPS.iter().enumerate() {
            let mut head = [0u8; 32];
            let mut l = Buf(&mut head, 0);
            let _ = write!(l, "STEP {} OF {} (P9)", i + 1, n);
            let len = l.1;
            let head = core::str::from_utf8(&head[..len]).unwrap_or("STEP");
            label(vram, [head, step.name, step.expect]);
            // The source rectangle inside the wallpaper: UYVY, 2 bytes a
            // pixel, every x here 0 so the start stays 16-byte aligned.
            let (sx, sy, sw, sh) = step.src;
            let mut src = VideoPlane::new(VideoFormat::Uyvy, sw, sh, wall.offset);
            src.stride = wall.stride;
            src.offset = wall.offset + usize::from(sy) * wall.stride + usize::from(sx) * 2;
            let (out_w, out_h, x, y, taps) = step.out;
            d.show_video_scaled(
                &src,
                blendtest::VIDEO,
                Scale {
                    out_w,
                    out_h,
                    x,
                    y,
                    taps,
                    ur_c: step.ur_c,
                },
            );
            d.set_graphics(
                &Plane {
                    format: PixelFormat::Argb4444,
                    width: blendtest::W as u16,
                    height: blendtest::H as u16,
                    offset: LABEL_AT,
                    stride: blendtest::W * 2,
                },
                blendtest::GRAPHICS,
            );
            record(d.regs(), step.name);
            let _ = writeln!(
                RunLog,
                "P9 scaler: VSADR {:08X} SSSIZE {:08X} SPOCTRL {:08X} SHCTRL {:08X}",
                d.regs().read32(layers::reg::VSADR),
                d.regs().read32(layers::reg::SSSIZE),
                d.regs().read32(layers::reg::SPOCTRL),
                d.regs().read32(layers::reg::SHCTRL)
            );
            let _ = ack_events(d.regs(), event::UNDER_OVERRUN | event::VEVENT0);
            let (mut frames, mut faults, mut fault_polls) = (0u32, 0u32, 0u32);
            for _ in 0..super::stage_seconds() * 100 {
                rt::delay_us(10_000);
                let s = ack_events(d.regs(), event::UNDER_OVERRUN | event::VEVENT0);
                frames += u32::from(s & event::VEVENT0 != 0);
                if s & event::UNDER_OVERRUN != 0 {
                    faults |= s & event::UNDER_OVERRUN;
                    fault_polls += 1;
                }
                rt::check_in();
            }
            let _ = writeln!(
                RunLog,
                "P9 step {}/{n} {}: frame events in {frame_polls} polls of 10 ms: {frames}, under/overrun in {fault_polls} (bits {faults:08X})",
                i + 1,
                step.name,
                frame_polls = super::stage_seconds() * 100,
            );
        }
        d.hide_video();
    }

    /// A formatting buffer on the stack.
    struct Buf<'a>(&'a mut [u8], usize);

    impl Write for Buf<'_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let n = s.len().min(self.0.len() - self.1);
            self.0[self.1..self.1 + n].copy_from_slice(&s.as_bytes()[..n]);
            self.1 += n;
            Ok(())
        }
    }

    fn main() -> ! {
        rt::init_mmu();
        let Some((regs, vram)) = rp_image::take_mbx() else {
            crash(format_args!("no display"))
        };
        let mut uart = rt::take_ffuart().map(|bus| {
            let mut u = PxaUart::new(bus);
            u.init(115_200);
            u
        });
        let mut d = Display::new(regs);
        record(d.regs(), "found");
        // Panel timing and an opaque graphics plane, as every image does;
        // that state is what goes back at the end.
        let _ = d.adopt(MBX_VRAM_SIZE as usize);
        let before = Snapshot::take(d.regs());

        // Stage F's interface layer (1.2 MB) lives in the scratch region.
        let Some(work) = rp_hal_x51v::devices::take_scratch() else {
            crash(format_args!("no scratch RAM"))
        };
        for stage in Stage::ALL {
            let t = rt::ticks();
            blendtest::draw(stage, vram, work);
            // F is the OS's drawing path: its time (compose in RAM, a full
            // clear flush, the dirty flush, and the shared wallpaper) is the
            // device number for docs/perf.md. OS timer: 3.25 ticks a microsecond.
            let us = rt::ticks().wrapping_sub(t) / 13 * 4;
            let _ = writeln!(RunLog, "draw {}: {us} us", stage.letter());
        }
        if super::SCALE {
            scale_steps(&mut d, vram);
            before.restore(d.regs());
            record(d.regs(), "restored");
            rp_image::finish()
        }
        for stage in Stage::ALL
            .into_iter()
            .filter(|s| super::STAGES.contains(s.letter()))
        {
            blendtest::program(&mut d, stage);
            record(d.regs(), stage.label()[0]);
            let c = stage.letter();
            if let Some(u) = uart.as_mut() {
                let _ = writeln!(u, "{}{c}", super::MARKER);
            }
            let _ = writeln!(RunLog, "{}{c}", super::MARKER);
            for _ in 0..super::stage_seconds() * 10 {
                rt::delay_us(100_000);
                rt::check_in();
            }
        }
        before.restore(d.regs());
        record(d.regs(), "restored");
        rp_image::finish()
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
