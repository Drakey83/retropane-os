//! `cargo xtask motion-cost`: Calendar's motion (calendar-rule0.md, group E)
//! drawn frame by frame through the image's own shell, the app and the 2D
//! renderer on the engine's model, each frame priced from what the engine
//! is asked to do, against the owner's frame rule (33 ms a moving frame,
//! one whole-screen change up to 45 ms: lookcheck.rs).
//!
//! The emulator models the engine's pixels, not its time (docs/hw/
//! emulator.md: "no 2D timing is modelled"), so each frame's work (blits,
//! and pixels copied, blended, dithered, converted) is priced at the rates
//! measured on the X51v: ~6 us a command (run F), copies at 32 Mpx/s and
//! blends at 16 Mpx/s (a full-screen 2x scale 18.7 ms at half a copy's
//! speed, P6; coverage ~16 Mpx/s; the 4 x 4 dither 18.9 ms a screen,
//! ADR-0034 C2). Fitted against run today-0140/0141's frames (Today's
//! whole screen, 650 blits and ~278k blended pixels: 55.5 ms measured, 59
//! priced; the Start menu, 191 blits: 18.2 ms measured) the price runs a
//! little high: a frame priced under 33 ms is under it on the device. The
//! device's own times come with the next device run (reminder R369).

use rp_draw::{Frame, Renderer};
use rp_mbx_render::model::{self, Bus, Mem};
use rp_mbx_render::{Layout, Mbx2dRenderer};
use rp_shell::Input;

/// A frame's price, microseconds, from the engine's work.
fn price(s: &rp_mbx_render::FrameStats) -> u32 {
    let px = |n: u32, mpx: u32| n / mpx;
    s.blits * 6
        + px(s.px_copy, 32)
        + px(s.px_blend, 16)
        + px(s.px_text, 16)
        + px(s.px_dither, 16)
        + px(s.px_convert, 32)
}

/// One motion: its name (and its frames' file names), the view it starts
/// on, and what starts it.
struct Scene {
    name: &'static str,
    file: &'static str,
    view: rp_app_calendar::View,
    start: Vec<Input>,
}

pub fn run(root: &std::path::Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let wall = crate::emu::today_wallpaper(&release)?;
    use rp_app_calendar::View;
    let strip_y = 36 + 76 + 6 + 30;
    let drag: Vec<Input> = std::iter::once(Input::PenDown { x: 400, y: strip_y })
        .chain((1..=10).map(|k| Input::PenMove {
            x: 400 - k * 24,
            y: strip_y,
        }))
        .chain(std::iter::once(Input::PenUp { x: 160, y: strip_y }))
        .collect();
    // Monday 1 January 2024's cell in the month (the app's tests hold it).
    let tap = |x, y| vec![Input::PenDown { x, y }, Input::PenUp { x, y }];
    let scenes = [
        Scene {
            name: "strip: drag a week left, release, settle (Day)",
            file: "strip",
            view: View::Day,
            start: drag,
        },
        Scene {
            name: "zoom D: Month into Day (the chosen day tapped)",
            file: "month-day",
            view: View::Month,
            start: tap(110, 189),
        },
        Scene {
            name: "zoom D: Day out to Month (the Month tab)",
            file: "day-month",
            view: View::Day,
            start: tap(416, 578),
        },
        Scene {
            name: "zoom D: Week into Day (the day's wide column)",
            file: "week-day",
            view: View::Week,
            start: tap(150, 140),
        },
    ];
    let out = root.join("target").join("motion-cost");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut worst_moving = 0;
    for sc in &scenes {
        let mem = model::memory();
        let mut gpu = Mbx2dRenderer::new(
            Bus::new(mem.clone()),
            Mem(mem.clone()),
            Layout::X51V,
            || 0,
            1,
        );
        gpu.set_pics(crate::emu::preview_pics());
        rp_today::show_gpu(&mut gpu, &wall, |_| {}, &mut rp_today::WallScratch::new());
        let mut shell =
            rp_shell::Shell::new(rp_today::shell_fonts(), crate::emu::today_icons(), true);
        shell.use_backdrops(true);
        // The two lists the image keeps (the draw-list difference).
        shell.keep_lists(vec![0u32; rp_shell::SHOWN_WORDS].leak());
        crate::app_mockups::sample_week(&mut shell);
        shell.give_pics(crate::emu::preview_pics());
        let mut c = rp_app_calendar::Calendar::new(&crate::app_mockups::CALS, 19_723);
        c.show(sc.view);
        shell.open_program(Box::leak(Box::new(c)));
        let mut frame = Frame::new();
        let mut shown = None;
        let mut now = 1000u32;
        let mut shot = 0;
        let mut draw = |shell: &mut rp_shell::Shell, gpu: &mut Mbx2dRenderer<Bus, Mem>| {
            crate::emu::backdrop(gpu, shell, &wall, None, &mut shown);
            shell.frame(&mut frame);
            gpu.draw(&frame);
            while !gpu.poll() {}
            // The screen as shown, kept (the frames the owner's pick names).
            let scadr = gpu.parts().0.regs.reg(rp_drv_mbx_display::reg::GSCADR);
            let base = ((scadr & 0x7F_FFFF) * 16) as usize;
            let rgb: Vec<u8> = mem.borrow()[base..base + rp_mbx_render::SCREEN_BYTES]
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|p| crate::emu::shown_565(u16::from_le_bytes(*p)))
                .collect();
            let _ = std::fs::write(
                out.join(format!("{}-{shot:02}.png", sc.file)),
                crate::ui_preview::png_rgb(480, 640, &rgb),
            );
            shot += 1;
            (price(&gpu.last), gpu.last.blits)
        };
        // Settled first (the whole screen once: not the motion's).
        for _ in 0..3 {
            shell.settle();
            draw(&mut shell, &mut gpu);
        }
        println!("{}:", sc.name);
        let mut moving = Vec::new();
        for &e in &sc.start {
            let _ = shell.input(e);
            now += 16;
            let _ = shell.tick(now);
            let (us, blits) = draw(&mut shell, &mut gpu);
            moving.push((us, blits));
        }
        // The motion's frames: a tick every 33 ms until nothing moves.
        for _ in 0..12 {
            now += 33;
            let _ = shell.tick(now);
            let (us, blits) = draw(&mut shell, &mut gpu);
            moving.push((us, blits));
        }
        let line: Vec<String> = moving
            .iter()
            .filter(|(us, _)| *us > 0)
            .map(|(us, b)| format!("{:.1} ms/{b}", f64::from(*us) / 1000.0))
            .collect();
        let worst = moving.iter().map(|m| m.0).max().unwrap_or(0);
        worst_moving = worst_moving.max(worst);
        println!("    frames (priced ms / blits): {}", line.join(", "));
        println!(
            "    worst {:.1} ms: {}",
            f64::from(worst) / 1000.0,
            if worst <= crate::lookcheck::BUDGET_US {
                "inside the 33 ms rule"
            } else if worst <= crate::lookcheck::WHOLE_SCREEN_US {
                "over 33 ms (a whole-screen change may take 45 once)"
            } else {
                "OVER the frame rule"
            }
        );
    }
    println!(
        "motion-cost: worst frame priced {:.1} ms (the price runs high: device times owed)",
        f64::from(worst_moving) / 1000.0
    );
    Ok(())
}
