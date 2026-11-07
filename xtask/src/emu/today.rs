//! The Today screen's emulator checks and previews: its screens drawn on the PC, the replays, the steps and the watch (emu-today, ui-preview's helpers).

#[allow(clippy::wildcard_imports)]
use super::*;

/// The wallpaper rp-today was built with: its build script's output, the
/// newest `wallpaper.yuv` under the release build folder in an `rp-today`
/// folder (Cargo's layout is `build/rp-today-<hash>/out` or, newer,
/// `build/rp-today/<hash>/out`: both are searched).
pub(crate) fn today_wallpaper(release: &Path) -> Result<Vec<u8>, String> {
    today_output(release, "wallpaper.yuv")
}

/// The radar map's layers rp-today was built with (`radar.zz`: the canned
/// relay's map), unpacked as the image unpacks them, or `None` if it isn't
/// there.
pub(crate) fn today_map(release: &Path) -> Option<Vec<u8>> {
    let packed = today_output(release, "radar.zz").ok()?;
    let mut map = vec![0u8; rp_weather::maplayers::BYTES];
    rp_weather::maplayers::unpack(&packed, &mut map).ok()?;
    Some(map)
}

/// The newest `name` among rp-today's build outputs.
pub(crate) fn today_output(release: &Path, name: &str) -> Result<Vec<u8>, String> {
    fn find(dir: &Path, depth: u32, in_today: bool, name: &str, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.filter_map(Result::ok) {
            let p = e.path();
            let today = in_today || e.file_name().to_string_lossy().starts_with("rp-today");
            if p.is_dir() && depth > 0 {
                find(&p, depth - 1, today, name, out);
            } else if today && e.file_name() == name {
                out.push(p);
            }
        }
    }
    let mut found = Vec::new();
    find(&release.join("build"), 4, false, name, &mut found);
    let newest = found
        .into_iter()
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
        .ok_or_else(|| format!("today: no {name} from rp-today's build"))?;
    std::fs::read(&newest).map_err(|e| format!("{}: {e}", newest.display()))
}

/// What rp-today should show: the same set-up (`rp_today::show_gpu`),
/// shell and GPU renderer (ADR-0023) as the image, on the Rust model of
/// the 2D engine, frame after frame; the emulator's own model of the
/// engine must draw the same pictures. `steps` are applied between
/// screens; the first screen is before any. The clock's time is
/// arbitrary: the check masks it. `settle` ends any movement before each
/// picture (the preview's pictures show pages in place; the emulator check
/// keeps the image's own timing). `alerts` is an alerts answer as the
/// relay brings it (the previews'; the emulator's image has no relay).
pub(crate) fn today_screens(
    wall: &[u8],
    map: Option<&[u8]>,
    steps: &[rp_shell::Input],
    settle: bool,
    alerts: Option<&[u8]>,
) -> Vec<Vec<u8>> {
    use rp_draw::{Frame, Renderer};
    use rp_mbx_render::model::{self, Bus, Mem};
    use rp_mbx_render::{Layout, Mbx2dRenderer, SCREEN_BYTES};

    let mem = model::memory();
    let mut gpu = Mbx2dRenderer::new(
        Bus::new(mem.clone()),
        Mem(mem.clone()),
        Layout::X51V,
        || 0,
        1,
    );
    gpu.set_pics(preview_pics());
    rp_today::show_gpu(&mut gpu, wall, |_| {}, &mut rp_today::WallScratch::new());
    let mut shell = rp_shell::Shell::new(rp_today::shell_fonts(), today_icons(), true);
    rp_today::keep_graph(&mut shell, preview_pics(), &mut None);
    shell.set_dials(today_dials());
    shell.set_time(preview_clock(rp_shell::Time {
        hour: 12,
        minute: 0,
        weekday: 0,
        day: 1,
        month: 1,
    }));
    if let Some(a) = alerts {
        shell.set_alerts_from_nws(a);
        // The previews' station too (the relay's canned Tempest shape).
        shell.set_station_from_tempest(PREVIEW_STATION);
        shell.set_history_from_tempest(preview_history().as_bytes());
    }
    let mut frame = Frame::new();
    let mut screens = Vec::new();
    let mut shown = None;
    let mut painted = None;
    let mut chart_painted = None;
    for step in std::iter::once(None).chain(steps.iter().map(Some)) {
        if let Some(&e) = step {
            shell.input(e);
        }
        if settle {
            shell.settle();
        }
        preview_net(&mut shell);
        // A chart opened: its line painted into its own picture, as the
        // image does.
        rp_today::keep_graph(&mut shell, preview_pics(), &mut painted);
        rp_today::keep_chart(&mut shell, preview_pics(), &mut chart_painted);
        backdrop(&mut gpu, &shell, wall, map, &mut shown);
        shell.frame(&mut frame);
        gpu.draw(&frame);
        while !gpu.poll() {}
        let scadr = gpu.parts().0.regs.reg(rp_drv_mbx_display::reg::GSCADR);
        let base = ((scadr & 0x7F_FFFF) * 16) as usize;
        let m = mem.borrow();
        let out = m[base..base + SCREEN_BYTES]
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| shown_565(u16::from_le_bytes(*p)))
            .collect();
        screens.push(out);
    }
    screens
}

/// Hourly as the X51v shows it on the plane path (the preview's device
/// shot): screen 0 the app's chrome through the 2D engine model, the page
/// the CPU's picture (bands of the page, Hourly's graph its one picture,
/// out to ARGB4444 dithered as the image writes it) blended over it in the
/// page's window, at rest (ARGB8888, as the image writes its pages).
pub(crate) fn today_hourly_on_planes(wall: &[u8], icons: Option<rp_shell::Icons>) -> Vec<u8> {
    today_page_on_planes(wall, icons, 1, Some(NWS_ALERTS))
}

/// Weather's page `tab` as the X51v shows it on the plane path: the app's
/// chrome on screen 0 and the page's picture blended over it, Hourly's at
/// ARGB8888 with its graph picture, the others at ARGB4444
/// dithered (the image's `build_page`). `alerts`: the NWS answer the
/// header shows (the previews' canned one), or none (the image with no
/// link, as emu-today runs it).
pub(crate) fn today_page_on_planes(
    wall: &[u8],
    icons: Option<rp_shell::Icons>,
    tab: u8,
    alerts: Option<&[u8]>,
) -> Vec<u8> {
    use rp_draw::{Frame, Renderer};
    use rp_gfx::layer::{Layer, PlaneFormat};
    use rp_gfx::render::CpuRenderer;
    use rp_mbx_render::model::{self, Bus, Mem};
    use rp_mbx_render::{Layout, Mbx2dRenderer, SCREEN_BYTES};

    let mem = model::memory();
    let mut gpu = Mbx2dRenderer::new(
        Bus::new(mem.clone()),
        Mem(mem.clone()),
        Layout::X51V,
        || 0,
        1,
    );
    gpu.set_pics(preview_pics());
    rp_today::show_gpu(&mut gpu, wall, |_| {}, &mut rp_today::WallScratch::new());
    // The caller's set, else the image's (the previews' store holds one
    // set at a time: making the image's would give the caller's back).
    let icons = icons.unwrap_or_else(today_icons);
    let mut shell = rp_shell::Shell::new(rp_today::shell_fonts(), icons, true);
    rp_today::keep_graph(&mut shell, preview_pics(), &mut None);
    shell.set_dials(today_dials());
    shell.set_time(preview_clock(rp_shell::Time {
        hour: 12,
        minute: 0,
        weekday: 0,
        day: 1,
        month: 1,
    }));
    shell.use_planes(true);
    if let Some(a) = alerts {
        shell.set_alerts_from_nws(a);
    }
    let right = core::iter::repeat_n(rp_shell::Input::Key(rp_shell::Key::Right), usize::from(tab));
    for e in [
        rp_shell::Input::PenDown { x: 240, y: 150 },
        rp_shell::Input::PenUp { x: 240, y: 150 },
    ]
    .into_iter()
    .chain(right)
    {
        shell.input(e);
        shell.settle();
    }
    let mut frame = Frame::new();
    shell.frame(&mut frame);
    gpu.draw(&frame);
    while !gpu.poll() {}
    let scadr = gpu.parts().0.regs.reg(rp_drv_mbx_display::reg::GSCADR);
    let base = ((scadr & 0x7F_FFFF) * 16) as usize;
    let mut out: Vec<u8> = {
        let m = mem.borrow();
        m[base..base + SCREEN_BYTES]
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| shown_565(u16::from_le_bytes(*p)))
            .collect()
    };
    // The page's picture, as the image builds it.
    let rows = u32::from(rp_shell::picture_rows(tab));
    let mut px = vec![0u32; 480 * rows as usize];
    let mut plane = vec![0u8; 480 * 4 * rows as usize];
    let mut converted = None;
    {
        let mut cpu = CpuRenderer::new(
            Layer::new(&mut px, 480, rows, 480).expect("the layer"),
            &mut plane,
            480 * 4,
            PlaneFormat::Argb8888,
        );
        cpu.set_pics(preview_pics());
        let mut f = Frame::new();
        let mut top = 0u16;
        while u32::from(top) < rows {
            let n = rp_today::PAGE_BAND.min(rows as u16 - top);
            shell.page_band(tab, &mut f, top, n);
            let _ = cpu.draw(&f);
            top += n;
        }
        if tab != 1 {
            // To the panel's ARGB4444, dithered, as the image sends it;
            // back to 8888 (each nibble repeated) for the blend below.
            let mut wide = vec![0u8; 480 * 4 * rows as usize];
            let mut half = [0u8; 480 * 2];
            for y in 0..rows {
                let r = rp_gfx::Rect::new(0, y, 480, 1);
                let _ = rp_gfx::layer::flush(
                    &cpu.layer().view(),
                    PlaneFormat::Argb4444Dithered,
                    &mut half,
                    0,
                    &[r],
                );
                for (x, q) in half.as_chunks::<2>().0.iter().enumerate() {
                    let v = u16::from_le_bytes(*q);
                    let n4 = |s: u16| u32::from((v >> s) & 15) * 17;
                    let w = n4(12) << 24 | n4(8) << 16 | n4(4) << 8 | n4(0);
                    let at = (y as usize * 480 + x) * 4;
                    wide[at..at + 4].copy_from_slice(&w.to_le_bytes());
                }
            }
            converted = Some(wide);
        }
    }
    if let Some(wide) = converted {
        plane = wide;
    }
    // Over screen 0 in the page's window (at rest: row LAYER_MARGIN at its
    // top), each 4444 pixel by its alpha.
    let v = shell.page_layer().expect("the page on the layer");
    for y in 0..usize::from(v.win.h) {
        for x in 0..usize::from(v.win.w) {
            let at = (usize::from(v.row) + y) * 480 * 4 + (usize::from(v.win.x) + x) * 4;
            let p = u32::from_le_bytes([plane[at], plane[at + 1], plane[at + 2], plane[at + 3]]);
            let a = p >> 24;
            let c = [(p >> 16) & 255, (p >> 8) & 255, p & 255];
            let o = ((usize::from(v.win.y) + y) * 480 + usize::from(v.win.x) + x) * 3;
            for k in 0..3 {
                let d = u32::from(out[o + k]);
                out[o + k] = ((c[k] * a + d * (255 - a)) / 255) as u8;
            }
        }
    }
    out
}

/// The movement after the last of `steps` (the others applied and settled):
/// a screen every `every_ms` from the moment of that input, `n` of them,
/// through the same set-up, shell and 2D engine model as [`today_screens`].
/// For the preview's moving pictures; the device runs the same frames.
pub(crate) fn today_motion(
    wall: &[u8],
    map: Option<&[u8]>,
    steps: &[rp_shell::Input],
    every_ms: u32,
    n: u32,
) -> Vec<Vec<u8>> {
    today_live(
        wall,
        map,
        rp_shell::Motion::Stagger,
        steps,
        &[],
        every_ms,
        n,
    )
}

/// The movement of a live stroke: `setup` applied and settled, then each
/// of `live` one `every_ms` after the other (the pen's samples, a frame
/// after each), then `n` more frames as it moves by itself (a fling, a
/// rebound, an entrance); pages arrive as `motion`. With `live` empty,
/// the last of `setup` starts the movement.
/// What the display shows while Reveal's video window grows: `today`
/// (RGB, 480 x 640) around the window, `app` inside it read `shift` rows
/// lower (the shell's `reveal_window`; on the plane path, an offset in
/// the picture's start address). Unchanged without both.
pub(crate) fn reveal_over(
    today: Option<&[u8]>,
    app: Vec<u8>,
    window: Option<(rp_draw::Rect, u16)>,
) -> Vec<u8> {
    let (Some(today), Some((r, shift))) = (today, window) else {
        return app;
    };
    let mut out = today.to_vec();
    for y in r.y..r.bottom().min(640) {
        let from = (usize::from(y) + usize::from(shift)).min(639);
        let (x0, x1) = (usize::from(r.x), usize::from(r.right().min(480)));
        let (d, s) = ((usize::from(y) * 480 + x0) * 3, (from * 480 + x0) * 3);
        out[d..d + (x1 - x0) * 3].copy_from_slice(&app[s..s + (x1 - x0) * 3]);
    }
    out
}

/// The light look in `style` at `hour` (noon the light look, night the
/// dark; the owner's pick, 2026-11-01): the
/// Today screen with a rainy living sky, the Weather app's Now page with a
/// station live, and its settings, as the image draws them.
pub(crate) fn today_light(wall: &[u8], style: rp_shell::LightStyle, hour: u8) -> [Vec<u8>; 3] {
    use rp_draw::{Frame, Renderer};
    use rp_mbx_render::model::{self, Bus, Mem};
    use rp_mbx_render::{Layout, Mbx2dRenderer, SCREEN_BYTES};

    let mem = model::memory();
    let mut gpu = Mbx2dRenderer::new(
        Bus::new(mem.clone()),
        Mem(mem.clone()),
        Layout::X51V,
        || 0,
        1,
    );
    gpu.set_pics(preview_pics());
    rp_today::show_gpu(&mut gpu, wall, |_| {}, &mut rp_today::WallScratch::new());
    let mut shell = rp_shell::Shell::new(rp_today::shell_fonts(), today_icons(), true);
    shell.set_dials(today_dials());
    shell.set_time(rp_shell::Time {
        hour,
        minute: 0,
        weekday: 6,
        day: 1,
        month: 11,
    });
    shell.set_light_style(style);
    shell.set_station_from_tempest(PREVIEW_STATION);
    let mut wx = *shell.forecast();
    wx.now.sky.wmo = 63;
    wx.now.sky.cover = 90;
    shell.set_forecast(wx);
    shell.preview_sky(true);
    let mut frame = Frame::new();
    let mut shot = |shell: &mut rp_shell::Shell, gpu: &mut Mbx2dRenderer<Bus, Mem>| {
        shell.frame(&mut frame);
        gpu.draw(&frame);
        while !gpu.poll() {}
        let scadr = gpu.parts().0.regs.reg(rp_drv_mbx_display::reg::GSCADR);
        let base = ((scadr & 0x7F_FFFF) * 16) as usize;
        let m = mem.borrow();
        m[base..base + SCREEN_BYTES]
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| shown_565(u16::from_le_bytes(*p)))
            .collect::<Vec<u8>>()
    };
    for k in 0..90 {
        shell.tick(10_000 + k * rp_shell::sky::STEP_MS);
    }
    let today = shot(&mut shell, &mut gpu);
    shell.input(rp_shell::Input::PenDown { x: 240, y: 150 });
    shell.input(rp_shell::Input::PenUp { x: 240, y: 150 });
    shell.settle();
    let now = shot(&mut shell, &mut gpu);
    shell.input(rp_shell::Input::PenDown { x: 400, y: 620 });
    shell.input(rp_shell::Input::PenUp { x: 400, y: 620 });
    shell.settle();
    let settings = shot(&mut shell, &mut gpu);
    [today, now, settings]
}

/// For panel mock-ups: the Today screen's and the Now page's words and
/// pictures alone (no panels, no wallpaper) as an RGBA layer, by the CPU
/// renderer the device's 2D path is held to, in the light look or the dark;
/// and the wallpaper alone. Each [480 x 640 x 4] (RGBA), the wallpaper RGB.
pub(crate) fn panel_mock_layers(wall: &[u8], light: bool) -> (Vec<u8>, [Vec<u8>; 2]) {
    use rp_draw::{Frame, Renderer};
    use rp_gfx::layer::{Layer, PlaneFormat};
    use rp_mbx_render::model::{self, Bus, Mem};
    use rp_mbx_render::{Layout, Mbx2dRenderer, SCREEN_BYTES};

    // The wallpaper alone, through the 2D model.
    let mem = model::memory();
    let mut gpu = Mbx2dRenderer::new(
        Bus::new(mem.clone()),
        Mem(mem.clone()),
        Layout::X51V,
        || 0,
        1,
    );
    gpu.set_pics(preview_pics());
    rp_today::show_gpu(&mut gpu, wall, |_| {}, &mut rp_today::WallScratch::new());
    let mut frame = Frame::new();
    frame.damage(rp_draw::Rect::new(0, 0, 480, 640));
    gpu.draw(&frame);
    while !gpu.poll() {}
    let scadr = gpu.parts().0.regs.reg(rp_drv_mbx_display::reg::GSCADR);
    let base = ((scadr & 0x7F_FFFF) * 16) as usize;
    let wall_rgb: Vec<u8> = mem.borrow()[base..base + SCREEN_BYTES]
        .as_chunks::<2>()
        .0
        .iter()
        .flat_map(|p| shown_565(u16::from_le_bytes(*p)))
        .collect();
    let mut shell = rp_shell::Shell::new(rp_today::shell_fonts(), today_icons(), true);
    shell.set_time(rp_shell::Time {
        hour: if light { 12 } else { 22 },
        minute: 0,
        weekday: 6,
        day: 1,
        month: 11,
    });
    shell.set_station_from_tempest(PREVIEW_STATION);
    shell.preview_panels_off(true);
    let layer = |shell: &mut rp_shell::Shell| -> Vec<u8> {
        let mut px = vec![0u32; 480 * 640];
        let mut plane = vec![0u8; 480 * 4 * 640];
        {
            let mut cpu = rp_gfx::render::CpuRenderer::new(
                Layer::new(&mut px, 480, 640, 480).expect("the layer"),
                &mut plane,
                480 * 4,
                PlaneFormat::Argb8888,
            );
            cpu.set_pics(preview_pics());
            let mut f = Frame::new();
            shell.redraw_all();
            shell.frame(&mut f);
            let _ = cpu.draw(&f);
        }
        // BGRA to RGBA.
        plane
            .chunks(4)
            .flat_map(|c| [c[2], c[1], c[0], c[3]])
            .collect()
    };
    let today = layer(&mut shell);
    shell.input(rp_shell::Input::PenDown { x: 240, y: 150 });
    shell.input(rp_shell::Input::PenUp { x: 240, y: 150 });
    shell.settle();
    let now = layer(&mut shell);
    (wall_rgb, [today, now])
}

/// The living sky on the Today screen (W6, `rp_shell::sky`): `n` frames
/// `every_ms` apart with the sample forecast's sky set to `wmo` (cloud
/// cover `cover` %, wind `kmh10` from `from` degrees), as the image draws
/// it with a real forecast.
pub(crate) fn today_sky(
    wall: &[u8],
    (wmo, cover, kmh10, from): (u8, u8, u16, u16),
    hour: u8,
    every_ms: u32,
    n: u32,
) -> Vec<Vec<u8>> {
    use rp_draw::{Frame, Renderer};
    use rp_mbx_render::model::{self, Bus, Mem};
    use rp_mbx_render::{Layout, Mbx2dRenderer, SCREEN_BYTES};

    let mem = model::memory();
    let mut gpu = Mbx2dRenderer::new(
        Bus::new(mem.clone()),
        Mem(mem.clone()),
        Layout::X51V,
        || 0,
        1,
    );
    gpu.set_pics(preview_pics());
    rp_today::show_gpu(&mut gpu, wall, |_| {}, &mut rp_today::WallScratch::new());
    let mut shell = rp_shell::Shell::new(rp_today::shell_fonts(), today_icons(), true);
    shell.set_time(rp_shell::Time {
        hour,
        minute: 0,
        weekday: 3,
        day: 30,
        month: 10,
    });
    let mut wx = *shell.forecast();
    wx.now.sky.wmo = wmo;
    wx.now.sky.cover = cover;
    wx.now.sky.wind = kmh10;
    wx.now.wind_from = from;
    shell.set_forecast(wx);
    shell.preview_sky(true);
    let mut frame = Frame::new();
    let mut frames = Vec::new();
    for k in 0..n {
        shell.tick(10_000 + k * every_ms);
        shell.frame(&mut frame);
        gpu.draw(&frame);
        while !gpu.poll() {}
        let scadr = gpu.parts().0.regs.reg(rp_drv_mbx_display::reg::GSCADR);
        let base = ((scadr & 0x7F_FFFF) * 16) as usize;
        let m = mem.borrow();
        frames.push(
            m[base..base + SCREEN_BYTES]
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|p| shown_565(u16::from_le_bytes(*p)))
                .collect::<Vec<u8>>(),
        );
    }
    frames
}

pub(crate) fn today_live(
    wall: &[u8],
    map: Option<&[u8]>,
    motion: rp_shell::Motion,
    setup: &[rp_shell::Input],
    live: &[rp_shell::Input],
    every_ms: u32,
    n: u32,
) -> Vec<Vec<u8>> {
    let steps = setup;
    use rp_draw::{Frame, Renderer};
    use rp_mbx_render::model::{self, Bus, Mem};
    use rp_mbx_render::{Layout, Mbx2dRenderer, SCREEN_BYTES};

    let mem = model::memory();
    let mut gpu = Mbx2dRenderer::new(
        Bus::new(mem.clone()),
        Mem(mem.clone()),
        Layout::X51V,
        || 0,
        1,
    );
    gpu.set_pics(preview_pics());
    rp_today::show_gpu(&mut gpu, wall, |_| {}, &mut rp_today::WallScratch::new());
    let mut shell = rp_shell::Shell::new(rp_today::shell_fonts(), today_icons(), true);
    rp_today::keep_graph(&mut shell, preview_pics(), &mut None);
    shell.set_dials(today_dials());
    shell.set_time(preview_clock(rp_shell::Time {
        hour: 12,
        minute: 0,
        weekday: 3,
        day: 30,
        month: 10,
    }));
    let mut frame = Frame::new();
    let mut shown = None;
    let mut show = |shell: &mut rp_shell::Shell| {
        backdrop(&mut gpu, shell, wall, map, &mut shown);
        shell.frame(&mut frame);
        gpu.draw(&frame);
        while !gpu.poll() {}
        let scadr = gpu.parts().0.regs.reg(rp_drv_mbx_display::reg::GSCADR);
        let base = ((scadr & 0x7F_FFFF) * 16) as usize;
        let m = mem.borrow();
        m[base..base + SCREEN_BYTES]
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| shown_565(u16::from_le_bytes(*p)))
            .collect::<Vec<u8>>()
    };
    shell.set_motion(motion);
    let t0 = 10_000;
    shell.tick(t0);
    let (before, last) = if live.is_empty() {
        let Some((last, before)) = steps.split_last() else {
            return vec![show(&mut shell)];
        };
        (before, Some(*last))
    } else {
        (steps, None)
    };
    for &e in before {
        shell.input(e);
        shell.settle();
        show(&mut shell);
    }
    let mut frames = Vec::new();
    let mut t = t0;
    // Reveal's look on the plane path: the app's picture shown through a
    // growing window over the Today screen (sheet weather-gpu-movement.md),
    // composed here from the Today frame and the app's frames.
    let today = (motion == rp_shell::Motion::Reveal).then(|| show(&mut shell));
    if let Some(e) = last {
        shell.input(e);
    }
    for &e in live {
        t += every_ms;
        shell.tick(t);
        shell.input(e);
        let rgb = show(&mut shell);
        frames.push(reveal_over(today.as_deref(), rgb, shell.reveal_window()));
    }
    for k in 0..n {
        shell.tick(t + k * every_ms);
        let rgb = show(&mut shell);
        frames.push(reveal_over(today.as_deref(), rgb, shell.reveal_window()));
    }
    frames
}

thread_local! {
    /// The previews' picture store (`rp_draw::pics`): a slot for every
    /// condition at every size and Hourly's graph, made once a thread and kept (the renderers
    /// hold it for good, as the image's do). It holds one set at a time:
    /// [`preview_set`] gives the last one back.
    pub(crate) static PREVIEW_PICS: &'static rp_draw::pics::Pics<'static> = {
        // The big pictures' slots are the device's shape (250 x 176): square,
        // they were roomier than the graph's, and a short big picture took
        // the graph's slot (the store gives the least room that fits), so
        // the previews drew Hourly and the charts without their line.
        let mut classes: Vec<(u16, u16, u8)> = rp_shell::Size::ALL
            .iter()
            .map(|&z| {
                let h = if z == rp_shell::Size::Hero { rp_shell::HERO_PIC_H } else { z.side() };
                (z.side(), h, 22)
            })
            .collect();
        let (gw, gh) = rp_shell::weather_app::GRAPH_PIC;
        classes.push((gw, gh, 1));
        let (cw, ch) = rp_shell::chart::CHART_PIC;
        classes.push((cw, ch, 1));
        Box::leak(Box::new(rp_draw::pics::Pics::new(
            Box::leak(vec![0; rp_draw::pics::Pics::words(&classes)].into_boxed_slice()),
            &classes,
        )))
    };
}

/// The previews' picture store (every renderer here draws from it).
pub(crate) fn preview_pics() -> &'static rp_draw::pics::Pics<'static> {
    PREVIEW_PICS.with(|p| *p)
}

/// Pack `file`'s pictures made at every size as the image makes them
/// (its big pictures decoded and shrunk by `rp_today::Maker`) into the
/// previews' store, the set before given back, with the radar's rain cells
/// drawn as the image draws them. No pictures if it isn't a pack.
pub(crate) fn preview_set(file: &[u8]) -> rp_shell::Icons {
    let pics = preview_pics();
    pics.clear();
    let mut big = vec![0; rp_today::BIG_WORDS];
    let mut shrink = vec![0; rp_today::Maker::shrink_words()];
    let mut icons =
        rp_today::make_all(file, pics, &mut rp_today::Maker::new(&mut big, &mut shrink));
    icons.radar = rp_today::radar_into(
        Box::leak(vec![0; rp_today::RADAR_WORDS].into_boxed_slice()),
        Box::leak(Box::new([rp_today::NO_IMAGE; 3])),
    );
    icons
}

/// The pictures the Today image shows with no other pack chosen: the
/// default pack (pack 0, VClouds) as it makes them, if the build made it
/// (the image carries none of its own: option C).
pub(crate) fn today_icons() -> rp_shell::Icons {
    let release = crate::arm_release(&crate::workspace_root());
    preview_set(&today_output(&release, "rp-icons0.pak").unwrap_or_default())
}

/// An SD card image holding the default icon pack in its parts
/// (`RPIC0A.PAK`, ...) as `cargo xtask load` puts it there, if the build
/// made the pack (none without the art in local/: no card, no pictures).
pub(crate) fn today_card(root: &Path, release: &Path) -> Result<Option<PathBuf>, String> {
    let Ok(pack) = today_output(release, "rp-icons0.pak") else {
        return Ok(None);
    };
    let mut parts: Vec<(String, &[u8])> = pack
        .chunks(rp_today::PACK_PART)
        .enumerate()
        .map(|(p, c)| (format!("RPIC0{}.PAK", (b'A' + p as u8) as char), c))
        .collect();
    parts.push((
        format!("RPIC0{}.PAK", (b'A' + parts.len() as u8) as char),
        &[],
    ));
    // The PC's time at the load, as `cargo xtask load` puts it (R182):
    // Monday 2026-11-02 23:11 in UTC-6.
    let time = rp_today::clock::card_time(20_760 * 24 * 60 + 5 * 60 + 11, -360);
    parts.push(("RPTIMA.PAK".to_string(), &time[..]));
    parts.push(("RPTIMB.PAK".to_string(), &[]));
    let files: Vec<rp_fat::format::Entry> = parts
        .iter()
        .map(|(name, data)| rp_fat::format::Entry { name, data })
        .collect();
    let img = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: (SD_IMAGE_BYTES / 512) as u32,
        sectors_per_cluster: 8,
        files: &files,
        blocked_clusters: &[],
        ballast: 0,
    });
    let path = scratch(root, "today-card.img")?;
    std::fs::write(&path, img).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(path))
}

thread_local! {
    /// The clock the references are drawn at instead of the previews'
    /// noon: the image's own start (its build's local time) while a check
    /// compares the image's screens, whose look (Auto: light by day, dark
    /// by night) follows its clock.
    pub(crate) static PREVIEW_CLOCK: std::cell::Cell<Option<rp_shell::Time>> =
        const { std::cell::Cell::new(None) };
}

/// `noon`, or the image's clock while a check compares against it.
/// The previews' clock from now on (`None`: noon again).
pub(crate) fn set_preview_clock(t: Option<rp_shell::Time>) {
    PREVIEW_CLOCK.with(|c| c.set(t));
}

pub(crate) fn preview_clock(noon: rp_shell::Time) -> rp_shell::Time {
    PREVIEW_CLOCK.with(std::cell::Cell::get).unwrap_or(noon)
}

/// The relay's canned alerts answer (`ALERTS`), for the previews.
pub(crate) const NWS_ALERTS: &[u8] =
    include_bytes!("../../../shell/weather/tests/data/nws-alerts-two.json");

/// Icon pack `n` as the image unpacks it from the card, if the build made it.
pub(crate) fn today_pack(release: &Path, n: usize) -> Option<rp_shell::Icons> {
    let pack = today_output(release, &format!("rp-icons{n}.pak")).ok()?;
    pack_icons(&pack)
}

/// A pack file's set, made as the image makes it.
pub(crate) fn pack_icons(pack: &[u8]) -> Option<rp_shell::Icons> {
    rp_today::PackIndex::read(pack)?;
    Some(preview_set(pack))
}

/// A station's reading for the previews (Tempest's shape).
pub(crate) const PREVIEW_STATION: &[u8] = br#"{"latitude": 37.7, "longitude": -89.86,
  "location_name": "Perryville Loop Station",
  "current_conditions": {"time": 1761900000, "conditions": "Clear", "icon": "clear-day",
    "air_temperature": 19.9, "feels_like": 19.6, "relative_humidity": 61, "dew_point": 12.2,
    "wind_avg": 12.9, "wind_gust": 33.8, "wind_direction": 315, "uv": 3,
    "sea_level_pressure": 1016.4, "pressure_trend": "rising", "precip_accum_local_day": 1.0,
    "solar_radiation": 420, "lightning_strike_count_last_3hr": 4,
    "lightning_strike_last_distance": 10}}"#;

/// The previews' station day: a day of Tempest's one-minute rows (every
/// fifth), cool at dawn and warm mid-afternoon, a breeze building, ending
/// at [`PREVIEW_STATION`]'s reading.
pub(crate) fn preview_history() -> String {
    let end = 1_761_900_000i64;
    let rows: Vec<String> = (0..288)
        .map(|k| {
            let t = end - (287 - k) * 300;
            // Hours before noon: the day's shape as a smooth wave.
            let h = (k as f64 - 287.0) / 12.0;
            let temp = 15.5 + 4.5 * ((h + 9.0) * std::f64::consts::PI / 12.0).sin();
            let wind = 2.5 + 1.5 * ((h + 4.0) * std::f64::consts::PI / 10.0).sin().abs();
            let rh = 72.0 - 12.0 * ((h + 9.0) * std::f64::consts::PI / 12.0).sin();
            let p = 985.0 + 0.08 * (k as f64 / 12.0);
            // Daylight from 7 AM (hour -5) to 6 PM; a shower mid-morning
            // with a few strikes.
            let sun = (((h + 5.0) / 11.0 * std::f64::consts::PI).sin() * 780.0).max(0.0);
            let uv = sun / 85.0;
            let rain = if (-3.5..-2.0).contains(&h) { 0.06 } else { 0.0 };
            let strikes = u32::from((-3.0..-2.5).contains(&h)) * (k as u32 % 3);
            format!(
                "[{t},{:.2},{wind:.2},{:.2},300,60,{p:.1},{temp:.1},{rh:.0},{:.0},{uv:.2},{sun:.0},{rain:.3},1,9,{strikes}]",
                wind * 0.6,
                wind * 1.8,
                sun * 120.0
            )
        })
        .collect();
    format!("{{\"obs\":[{}]}}", rows.join(","))
}

/// The Station screen's dials as the image paints them.
pub(crate) fn today_dials() -> rp_shell::station::Dials {
    rp_today::dials_into(
        Box::leak(vec![0; rp_shell::station::DIAL_WORDS].into_boxed_slice()),
        Box::leak(Box::new(
            [rp_today::NO_IMAGE; rp_shell::station::DIALS.len()],
        )),
    )
}

thread_local! {
    /// The look the previews colour the radar's map in instead of the
    /// shell's (`RP_PREVIEW_MAPS` draws each look).
    pub(crate) static PREVIEW_LOOK: std::cell::Cell<Option<rp_weather::maplayers::Look>> =
        const { std::cell::Cell::new(None) };
}

/// The previews' radar look from now on (`None`: the settings' again).
pub(crate) fn set_preview_look(l: Option<rp_weather::maplayers::Look>) {
    PREVIEW_LOOK.with(|c| c.set(l));
}

thread_local! {
    /// A RainViewer tile (PNG) the previews' real radar shows in every
    /// tile of its view, every frame (R184 step 2: the rain over each
    /// look), with the relay's answers given to the shell.
    pub(crate) static PREVIEW_RAIN: std::cell::RefCell<Option<Vec<u8>>> =
        const { std::cell::RefCell::new(None) };
}

/// The previews' real rain from now on (`None`: the sample's again).
pub(crate) fn set_preview_rain(png: Option<Vec<u8>>) {
    PREVIEW_RAIN.with(|c| *c.borrow_mut() = png);
}

/// With the previews' rain set, the shell's radar asks answered as the
/// relay would: 13 frames ten minutes apart, the map (the build's own,
/// already shown), every tile (kept by the preview, not the shell).
pub(crate) fn preview_net(shell: &mut rp_shell::Shell) {
    if PREVIEW_RAIN.with(|c| c.borrow().is_none()) {
        return;
    }
    // The shell's network turn: the radar's asks once it is open.
    shell.tick(1);
    let times: Vec<String> = (0..13u32)
        .map(|k| (1_793_450_000 + 600 * k).to_string())
        .collect();
    while let Some(q) = shell.next_ask() {
        match q.ask {
            rp_shell::Ask::Frames => shell.net_answer(true, times.join(" ").as_bytes()),
            rp_shell::Ask::Tile => shell.net_answer(true, b""),
            _ => shell.net_answer(false, b""),
        }
    }
    // The preview composes every frame alike (`preview_bands`): all ready.
    shell.set_radar_ready((1 << 13) - 1);
}

/// The previews' rain composed over the coloured map as the image does
/// (`rp_today::rain`: the rainy bands into a 16-bit picture in 2700G
/// memory, the wallpaper's bands pointed at it).
pub(crate) fn preview_bands(
    gpu: &mut rp_mbx_render::Mbx2dRenderer<rp_mbx_render::model::Bus, rp_mbx_render::model::Mem>,
    shell: &rp_shell::Shell,
    map: &[u32],
    look: rp_weather::maplayers::Look,
) {
    use rp_mbx_render::LocalMem;
    use rp_today::rain::{BAND_H, BANDS};
    use rp_weather::tiles::{VIEW_H, VIEW_W};
    let Some(png) = PREVIEW_RAIN.with(|c| c.borrow().clone()) else {
        return;
    };
    if !matches!(shell.radar_real(), Some(Some(_))) {
        return;
    }
    let mut tile = vec![0u32; 256 * 256];
    let mut data = vec![0u8; 256 * 1024];
    let mut raw = vec![0u8; 256 * (1 + 256 * 4)];
    if rp_weather::png::decode(&png, &mut tile, &mut data, &mut raw) != Ok((256, 256)) {
        return;
    }
    let v = shell.radar_view();
    let mut rain = vec![0u32; VIEW_W * VIEW_H];
    for k in 0..9 {
        v.place(k, &tile, &mut rain);
    }
    let spans = rp_today::rain::bands_of(&rain);
    let at = rp_vram_map::POOL.at as usize;
    let mut bands = [None; BANDS];
    let mut row = 0usize;
    let mut words = [0u32; VIEW_W / 2];
    for b in 0..BANDS {
        if spans[b].is_none() {
            continue;
        }
        bands[b] = Some(at + row * VIEW_W * 2);
        for y in 0..usize::from(BAND_H) {
            let i = (b * usize::from(BAND_H) + y) * VIEW_W;
            rp_today::rain::row565(&map[i..i + VIEW_W], &rain[i..i + VIEW_W], look, &mut words);
            gpu.parts().1.store(at + (row + y) * VIEW_W * 2, &words);
        }
        row += usize::from(BAND_H);
    }
    gpu.wallpaper_bands(
        rp_today::MAP_TOP,
        BAND_H,
        &bands,
        &[rp_draw::Rect::new(0, 0, 480, 640)],
    );
}

/// The radar's map under the Weather page while Radar is open, its layers
/// coloured in the look the settings name, the wallpaper after, as the
/// image does (`rp_today::show_backdrop`).
pub(crate) fn backdrop(
    gpu: &mut rp_mbx_render::Mbx2dRenderer<rp_mbx_render::model::Bus, rp_mbx_render::model::Mem>,
    shell: &rp_shell::Shell,
    wall: &[u8],
    map: Option<&[u8]>,
    shown: &mut Option<rp_weather::maplayers::Look>,
) {
    let want = shell
        .backdrop()
        .map(|l| PREVIEW_LOOK.with(std::cell::Cell::get).unwrap_or(l))
        .filter(|_| map.is_some());
    if want != *shown {
        let coloured = want.zip(map).map(|(look, layers)| {
            let mut view = vec![0u32; rp_weather::maplayers::W * rp_weather::maplayers::H];
            rp_weather::maplayers::colour(layers, look, &mut view);
            view
        });
        gpu.wallpaper_bands(0, 1, &[], &[]);
        rp_today::show_backdrop(
            gpu,
            wall,
            coloured.as_deref(),
            &mut rp_today::WallScratch::new(),
        );
        if let (Some(look), Some(view)) = (want, coloured.as_deref()) {
            preview_bands(gpu, shell, view, look);
        }
        *shown = want;
    }
}

/// A screen pixel (RGB565) as the emulator's screendump shows it: each
/// channel shifted up, its low bits 0 (the fork's scan-out, docs/hw/2700g.md
/// "Plane composition": "RGB565 shifted up without bit replication").
pub(crate) fn shown_565(p: u16) -> [u8; 3] {
    let p = u32::from(p);
    [
        ((p >> 11) << 3) as u8,
        (((p >> 5) & 0x3F) << 2) as u8,
        ((p & 0x1F) << 3) as u8,
    ]
}

/// The wallpaper comparison's screens (the test image's "Wallpaper
/// compare", the owner's 16-bit check; since ADR-0023's L1 dithered
/// against plain 16 bits, quality audit C48): the wallpaper alone as the
/// OS keeps it (dithered), then plain, each once its notice has gone,
/// through the same shell and renderer code as the image.
pub(crate) fn compare_screens(wall: &[u8]) -> [Vec<u8>; 2] {
    use rp_draw::{Frame, Renderer};
    use rp_mbx_render::model::{self, Bus, Mem};
    use rp_mbx_render::{Layout, Mbx2dRenderer, SCREEN_BYTES};
    use rp_shell::{Action, Input, Shell};

    let mem = model::memory();
    let mut gpu = Mbx2dRenderer::new(
        Bus::new(mem.clone()),
        Mem(mem.clone()),
        Layout::X51V,
        || 0,
        1,
    );
    gpu.set_pics(preview_pics());
    let mut scratch = rp_today::WallScratch::new();
    rp_today::show_gpu(&mut gpu, wall, |_| {}, &mut scratch);
    let mut shell = Shell::new(rp_today::shell_fonts(), today_icons(), true);
    let mut frame = Frame::new();
    let mut step = |shell: &mut Shell, gpu: &mut Mbx2dRenderer<Bus, Mem>| -> Vec<u8> {
        shell.frame(&mut frame);
        gpu.draw(&frame);
        while !gpu.poll() {}
        let scadr = gpu.parts().0.regs.reg(rp_drv_mbx_display::reg::GSCADR);
        let base = ((scadr & 0x7F_FFFF) * 16) as usize;
        let m = mem.borrow();
        m[base..base + SCREEN_BYTES]
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| shown_565(u16::from_le_bytes(*p)))
            .collect()
    };
    let tap = |shell: &mut Shell, r: rp_draw::Rect| {
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        shell.input(Input::PenDown { x, y });
        shell.input(Input::PenUp { x, y })
    };
    step(&mut shell, &mut gpu);
    tap(&mut shell, rp_shell::START);
    step(&mut shell, &mut gpu);
    tap(&mut shell, Shell::menu_entry(Shell::menu_entries(true) - 4));
    assert!(
        shell.wallpaper_only(),
        "the compare entry is fourth from the end (then Sky test, Align, Back)"
    );
    step(&mut shell, &mut gpu);
    shell.tick(10_000);
    let dithered = step(&mut shell, &mut gpu);
    let act = tap(&mut shell, rp_draw::Rect::new(200, 300, 80, 40));
    assert_eq!(act, Action::WallpaperDepth { dithered: false });
    rp_today::load_wallpaper(&mut gpu, wall, rp_today::Depth::Full, &mut scratch);
    gpu.wallpaper_changed();
    step(&mut shell, &mut gpu);
    shell.tick(20_000);
    let plain = step(&mut shell, &mut gpu);
    [dithered, plain]
}

/// Pixels that differ outside the clock's areas (their text depends on
/// when the check runs).
pub(crate) fn today_differ(got: &[u8], want: &[u8]) -> usize {
    let masked = [
        rp_shell::DATE_BOX,
        rp_shell::FORECAST_ROW,
        rp_shell::TITLE_CLOCK,
    ];
    got.chunks(3)
        .zip(want.chunks(3))
        .enumerate()
        .filter(|(i, (a, b))| {
            let (x, y) = ((i % rp_today::W) as u16, (i / rp_today::W) as u16);
            a != b && !masked.iter().any(|r| r.contains_point(x, y))
        })
        .count()
}

/// A drag at `x` through `ys`, each sample held 60 ms of the image's own
/// time (`hold`: never the wall clock, so a starved guest still sees every
/// sample), then the lift: emu-today's one stroke that isn't a press
/// waiting on the screen (the page moves after it, checked by the caller).
pub(crate) fn today_drag(
    qmp: &mut crate::screen::Qmp,
    x: u16,
    ys: &[u16],
    hold: &dyn Fn(u32) -> Result<(), String>,
) -> Result<(), String> {
    for (k, &y) in ys.iter().enumerate() {
        if k > 0 {
            hold(60)?;
        }
        touch(qmp, &Action::TouchDown(x, y))?;
    }
    hold(60)?;
    touch(qmp, &Action::TouchUp).map(drop)
}

/// The Weather page's area (`weather_app::PAGE`'s rows): pixels further than an ARGB4444
/// step plus its dither from `want` in any channel (48 of 255), and pixels
/// that differ at all.
/// The Now page's Sunset bar (how much of the day has gone: the clock's,
/// like the masked clocks) is left out.
pub(crate) fn page_differ(got: &[u8], want: &[u8]) -> (usize, usize) {
    let w = rp_today::W * 3;
    let page = rp_shell::weather_app::PAGE;
    let rows = usize::from(page.y) * w..usize::from(page.bottom()) * w;
    let (a, b) = (&got[rows.clone()], &want[rows]);
    let sunset = rp_shell::weather_app::SUNSET_BAR;
    a.chunks(3)
        .zip(b.chunks(3))
        .enumerate()
        .filter(|(i, _)| {
            let (x, y) = ((i % rp_today::W) as u16, page.y + (i / rp_today::W) as u16);
            !sunset.contains_point(x, y)
        })
        .fold((0, 0), |(far, any), (_, (p, q))| {
            let d = p
                .iter()
                .zip(q)
                .map(|(x, y)| x.abs_diff(*y))
                .max()
                .unwrap_or(0);
            (far + usize::from(d > 48), any + usize::from(d > 0))
        })
}

/// Pixels that differ within 32 px of a press at (`x`, `y`), unmasked.
pub(crate) fn press_answer(got: &[u8], want: &[u8], x: u16, y: u16) -> usize {
    let near = rp_draw::Rect::new(x.saturating_sub(32), y.saturating_sub(32), 64, 64);
    got.chunks(3)
        .zip(want.chunks(3))
        .enumerate()
        .filter(|(i, (a, b))| {
            let (px, py) = ((i % rp_today::W) as u16, (i / rp_today::W) as u16);
            a != b && near.contains_point(px, py)
        })
        .count()
}

/// How long emu-today waits for the image to answer a press on screen: a
/// backstop only, never a timing (the press waits for the answer).
pub(crate) const PRESS_ANSWER: Duration = Duration::from_secs(60);
/// A tap the image answers on the lift, held this long in its own time
/// (its pen filter lands a press in ~30 ms of samples).
pub(crate) const TAP_GUEST_MS: u32 = 150;

/// The shell's Today screen (ADR-0021) under the loader: pixel-exact
/// against the reference (clocks masked), still there after 40 s untouched
/// (past the run limit: idle sleeps must check in), a stylus tap on Start opens the
/// menu (pixel-exact again), and the menu's "Back to loader" returns with
/// the touch-to-flush latencies in the run log.
pub fn boot_loader_today(root: &Path) -> Result<(), String> {
    use rp_shell::Input;
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-today.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-today"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let wall = today_wallpaper(&release)?;
    let map = today_map(&release);
    // The references at the image's own clock: its look (Auto) follows it.
    PREVIEW_CLOCK.with(|c| c.set(Some(rp_today::build_time())));
    // The card the X51v's has: the default icon pack (option C: the image
    // carries no pictures), read and decoded as on the device; the
    // references draw it as the image makes it (`today_icons`).
    let sd = today_card(root, &release)?;
    let start = rp_shell::START;
    let (sx, sy) = (start.x + start.w / 2, start.y + start.h / 2);
    let want = today_screens(
        &wall,
        map.as_deref(),
        &[
            Input::PenDown { x: sx, y: sy },
            Input::PenUp { x: sx, y: sy },
        ],
        false,
        None,
    );
    let bare = compare_screens(&wall);
    // The Weather app's pages as the plane path shows them (screen 0's
    // chrome through the 2D engine model, the page's picture over it), the
    // model the image's own path is held to: the 2D path's drawing differs
    // where the two paths draw differently by design (Hourly's line: pieces
    // on screen 0, painted per pixel in the picture), so it was the wrong
    // reference (919 pixels "far" on the line alone, 2026-10-31).
    let weather = [0u8, 1].map(|tab| today_page_on_planes(&wall, None, tab, None));
    let wsl = matches!(emulator(root)?, Emulator::Wsl);
    let dump = |name: &str| -> Result<Vec<u8>, String> {
        let shot = scratch(root, &format!("today-{name}.ppm"))?;
        let guest = if wsl {
            to_wsl_path(&shot)
        } else {
            shot.display().to_string()
        };
        let _ = std::fs::remove_file(&shot);
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.screendump(&guest, &shot)?;
        let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
        Ok(img.rgb)
    };
    // Polls the screen until it matches (emulated time runs slower than
    // the wall clock on a busy PC: never a fixed sleep).
    let until = |name: &str, want: &[u8]| -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            poll_pause(Duration::from_millis(500));
            let n = today_differ(&dump(name)?, want);
            if n == 0 {
                return Ok(());
            }
            if Instant::now() > deadline {
                // The expected picture beside the screendump, to compare.
                let mut ppm = format!(
                    "P6
{} {}
255
",
                    rp_today::W,
                    rp_today::H
                )
                .into_bytes();
                ppm.extend_from_slice(want);
                let _ = std::fs::write(scratch(root, &format!("today-{name}-want.ppm"))?, ppm);
                return Err(format!(
                    "today: the {name} screen differs from the reference in {n} pixels (target/emulator/today-{name}.ppm, expected today-{name}-want.ppm)"
                ));
            }
        }
    };
    let check = || -> Result<(), String> {
        until("first", &want[0])?;
        // Left alone past the loader's 30 s run limit: an idle image must
        // keep checking in (it slept through it once and the watchdog reset
        // the device). Then it must still be there, unchanged.
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            40000,
            "today",
        )?;
        until("idle", &want[0])?;
        let mut qmp = crate::screen::Qmp::connect(monitor_port() + 1)?;
        // Every press is held until the image answers it on screen, never
        // for a fixed time: the image samples the pen in its own time, and
        // a busy host gives it few samples per wall-clock second (with
        // every core busy a 700 ms hold fell short of the align screen's
        // HOLD samples and the menu-aligned check failed by 234960 px).
        let changed = |name: &str, from: &[u8]| -> Result<Vec<u8>, String> {
            let deadline = Instant::now() + PRESS_ANSWER;
            loop {
                let now = dump(name)?;
                if today_differ(&now, from) > 0 {
                    return Ok(now);
                }
                if Instant::now() > deadline {
                    return Err(format!("today: no answer on screen to the {name}"));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        };
        // A press's answer shows where the pen is (a pressed target, a
        // target turning green): only pixels within 32 px of it count, so
        // the clock's areas needn't be masked (the align screen's centre
        // target lies inside them, and a masked answer was never seen).
        let answered = |name: &str, from: &[u8], x: u16, y: u16| -> Result<Vec<u8>, String> {
            let deadline = Instant::now() + PRESS_ANSWER;
            loop {
                let now = dump(name)?;
                if press_answer(&now, from, x, y) > 0 {
                    return Ok(now);
                }
                if Instant::now() > deadline {
                    return Err(format!("today: no answer on screen to the {name}"));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        };
        // Guest time, for presses the image answers only on the lift (the
        // bare wallpaper's depth flip).
        let guest_hold = |ms: u32| {
            guest_hold(
                &mut crate::screen::Monitor::connect(monitor_port())?,
                ms,
                "today",
            )
        };
        // Down until the screen shows the press (a pressed target, the
        // align target turning green), or for `guest_ms` of the image's
        // time when it answers only on the lift; then up; returns the
        // screen while held.
        let press =
            |qmp: &mut crate::screen::Qmp, name: &str, x: u16, y: u16, guest_ms: Option<u32>| {
                let before = dump(name)?;
                touch(qmp, &Action::TouchDown(x, y))?;
                let held = match guest_ms {
                    None => answered(&format!("{name}-down"), &before, x, y),
                    Some(ms) => guest_hold(ms).map(|()| before),
                };
                touch(qmp, &Action::TouchUp)?;
                held
            };
        press(&mut qmp, "start", sx, sy, None)?;
        until("menu", &want[2])?;
        // The align screen (the test image's second-to-last entry): hold
        // each target, then the check dot, as the owner would.
        let entries = rp_shell::Shell::menu_entries(true);
        let align = rp_shell::Shell::menu_entry(entries - 2);
        let held = press(
            &mut qmp,
            "align",
            align.x + align.w / 2,
            align.y + align.h / 2,
            None,
        )?;
        changed("align-up", &held)?;
        for (i, &(x, y)) in rp_shell::align::TARGETS
            .iter()
            .chain([&rp_shell::align::CHECK])
            .enumerate()
        {
            // The lift moves on (the next target, or the Today screen).
            let held = press(&mut qmp, &format!("target{i}"), x, y, None)?;
            changed(&format!("target{i}-up"), &held)?;
        }
        // Calibrated: Start must open the same menu, pixel for pixel (the
        // taps land where they should through the new calibration).
        press(&mut qmp, "start-aligned", sx, sy, None)?;
        until("menu-aligned", &want[2])?;
        // The wallpaper comparison (the owner's 16-bit check, the fourth
        // entry from the end): the wallpaper alone, pixel-exact dithered
        // and plain once each notice has gone; a tap flips it, Home goes
        // back. Ends dithered, so the menu below matches as before.
        let compare = rp_shell::Shell::menu_entry(entries - 4);
        press(
            &mut qmp,
            "compare",
            compare.x + compare.w / 2,
            compare.y + compare.h / 2,
            None,
        )?;
        until("bare-dithered", &bare[0])?;
        press(&mut qmp, "flip-plain", 240, 320, Some(TAP_GUEST_MS))?;
        until("bare-plain", &bare[1])?;
        press(&mut qmp, "flip-dithered", 240, 320, Some(TAP_GUEST_MS))?;
        until("bare-dithered-again", &bare[0])?;
        // Home, held in the image's own time (a `sendkey` hold is wall
        // clock, and under load the image never sampled it).
        qmp.key("f4", true)?;
        guest_hold(TAP_GUEST_MS)?;
        qmp.key("f4", false)?;
        // Home leaves the wallpaper alone: wait for the Today screen (its
        // depth notice gone), not for a time (under load 500 ms wasn't
        // enough and Start was pressed on the bare wallpaper).
        until("today-after-compare", &want[0])?;
        // The plane path (sheet weather-gpu-movement.md; ADR-0034 C4 as
        // changed, L2): the hero card opens Weather by Reveal; each page
        // shows on the video plane over screen 0 as the 2D path draws it
        // (ARGB4444 + dither against 8 bits: near, not exact); a drag moves
        // the page and leaves the header and tabs byte for byte; Home hands
        // the screen back pixel-exact.
        let page_near = |name: &str, want: &[u8]| -> Result<Vec<u8>, String> {
            let deadline = Instant::now() + Duration::from_secs(60);
            loop {
                poll_pause(Duration::from_millis(500));
                let got = dump(name)?;
                let (far, _) = page_differ(&got, want);
                if far == 0 {
                    return Ok(got);
                }
                if Instant::now() > deadline {
                    let mut ppm = format!(
                        "P6
{} {}
255
",
                        rp_today::W,
                        rp_today::H
                    )
                    .into_bytes();
                    ppm.extend_from_slice(want);
                    let _ = std::fs::write(scratch(root, &format!("today-{name}-want.ppm"))?, ppm);
                    return Err(format!(
                        "today: the {name} page is far from the 2D path's in {far} pixels (target/emulator/today-{name}.ppm, expected today-{name}-want.ppm)"
                    ));
                }
            }
        };
        press(&mut qmp, "hero", 240, 150, Some(TAP_GUEST_MS))?;
        // Diagnostics on request (`RETROPANE_TODAY_OPEN_FRAMES=n`): the
        // opening's screens, as fast as the monitor dumps them
        // (target/emulator/today-open-NN.ppm), to see each step.
        let frames: u32 = std::env::var("RETROPANE_TODAY_OPEN_FRAMES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        for k in 0..frames {
            dump(&format!("open-{k:02}"))?;
        }
        page_near("weather-now", &weather[0])?;
        press(&mut qmp, "hourly", 180, 578, Some(TAP_GUEST_MS))?;
        let still = page_near("weather-hourly", &weather[1])?;
        today_drag(&mut qmp, 240, &[500, 470, 430, 390, 350, 310], &guest_hold)?;
        guest_hold(2000)?;
        let moved = dump("weather-dragged")?;
        let (_, changed) = page_differ(&moved, &still);
        if changed == 0 {
            return Err("today: the Hourly page didn't move under a drag".into());
        }
        let chrome = |s: &[u8]| {
            let w = rp_today::W * 3;
            let (head, tabs) = (rp_shell::weather_app::HEAD, rp_shell::weather_app::TABS);
            (
                s[usize::from(head.y) * w..usize::from(head.bottom()) * w].to_vec(),
                s[usize::from(tabs.y) * w..rp_today::H * w].to_vec(),
            )
        };
        if chrome(&moved) != chrome(&still) {
            return Err(
                "today: a drag changed the header or tabs (screen 0 must stay still)".into(),
            );
        }
        qmp.key("f4", true)?;
        guest_hold(TAP_GUEST_MS)?;
        qmp.key("f4", false)?;
        until("today-after-weather", &want[0])?;
        press(&mut qmp, "start-after-compare", sx, sy, None)?;
        until("menu-after-compare", &want[2])?;
        // The last menu entry is "Back to loader" (test images only).
        let back = rp_shell::Shell::menu_entry(entries - 1);
        press(
            &mut qmp,
            "back",
            back.x + back.w / 2,
            back.y + back.h / 2,
            None,
        )?;
        // Back at the loader: the wallpaper between the clock and weather
        // panels (rows 210-217, nothing drawn over it) is gone. Polled, so
        // the run log has been sent when the emulator stops.
        let strip = |s: &[u8]| s[210 * rp_today::W * 3..218 * rp_today::W * 3].to_vec();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            poll_pause(Duration::from_millis(500));
            if strip(&dump("after")?) != strip(&want[2]) {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err("today: still showing after Back to loader".into());
            }
        }
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: sd.as_deref(),
            steps: &[],
            // Guest time from instructions, not the host clock: with every
            // core busy the starved guest saw its time jump past the
            // loader's 30 s run limit mid-hold ("FAULT RunLimit", checking
            // in every 5 ms of its own time; 2026-10-15). The idle wait
            // stays in real time, which a mostly sleeping guest follows.
            extra: &["-icount", "shift=0"],
            input: Some((&rp_loadproto::frame(&payload), rp_today::MARKER)),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("today: never came back to the loader")?;
    // The whole run log on request (a slow answer's line, a counter).
    if std::env::var_os("RETROPANE_TODAY_LOG").is_some() {
        eprintln!("{log}");
    }
    if crashed(log) || log.contains("did not return") {
        return Err(format!("today: didn't return cleanly:{log}"));
    }
    let line = log
        .lines()
        .find(|l| l.contains("touch to flush:"))
        .ok_or_else(|| format!("today: no latency line in the run log:{log}"))?;
    let answered: u32 = line
        .split("flush: ")
        .nth(1)
        .and_then(|t| t.split(' ').next())
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| format!("today: unreadable latency line {line:?}"))?;
    if answered < 2 {
        return Err(format!("today: the taps weren't answered: {line:?}"));
    }
    // The frame budget (R171, the owner, 2026-11-01: "I also want the
    // entirety of the OS to run smoothly"): no frame of the replay may take
    // longer to prepare than a refresh at 30 fps, in the emulator's own
    // time (instructions, `-icount`): a change that makes a screen's frame
    // heavier fails here before it reaches the X51v.
    const FRAME_BUDGET_US: u32 = 33_000;
    let worst: u32 = line
        .split("worst draw ")
        .nth(1)
        .and_then(|t| t.split(' ').next())
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| format!("today: no worst draw in {line:?}"))?;
    if worst > FRAME_BUDGET_US {
        return Err(format!(
            "today: a frame took {worst} us to prepare, over the {FRAME_BUDGET_US} us budget (30 fps, R171): {line:?}"
        ));
    }
    let calibrated = log
        .lines()
        .find(|l| l.contains("calibrated:"))
        .ok_or_else(|| format!("today: the align screen never finished:{log}"))?;
    let stack =
        check_stack(log)?.ok_or_else(|| format!("today: no stack line in the run log:{log}"))?;
    eprintln!(
        "    today OK: Today screen pixel-exact (clocks masked) and still up after 40 s idle, Start opened the menu pixel-exact, the align screen calibrated ({}) and the menu opened pixel-exact again, the wallpaper alone pixel-exact dithered and plain (16 bits), Weather on the plane path (Now and Hourly near the plane path model's, a drag moved the page and not the header or tabs, Home pixel-exact; {}), Back to loader returned; {}; stack {} of {} bytes",
        calibrated.trim(),
        log.lines()
            .find(|l| l.contains("plane path:") && l.contains("commits"))
            .map_or("no commit line", str::trim),
        line.trim(),
        stack.0,
        stack.1
    );
    Ok(())
}

/// The touch replay (`RP_TODAY_REPLAY`, the ratchet's touch row,
/// docs/design/perf-ratchet.md): the owner's recorded taps through the
/// image's filter, shell and drawing, no stylus. It must take all 28
/// presses, answer them, write its `TR` line and return to the loader by
/// itself. Its times are the emulator's, not the device's: not compared.
pub fn today_replay(root: &Path) -> Result<(), String> {
    today_replay_with(root, false)?;
    // The profile (`RP_TODAY_PROFILE`, R81 item 1): the same replay with
    // the performance monitor around four zones, a `PROFILE` line each
    // (the fork's event counters read 0: the plumbing only).
    today_replay_with(root, true)
}

/// `cargo xtask emu-today-steps`: the owner's report of the option C run
/// (2026-10-31) as taps on the image in the emulator, the SD card holding
/// the default pack, the screen saved after each (`target/emulator/
/// steps-NN-<name>.ppm`): open Weather, the place menu, a place, Home,
/// Weather again, Hourly, Now, Home.
pub fn today_steps(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-today.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-today"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let sd = today_card(root, &release)?;
    let wsl = matches!(emulator(root)?, Emulator::Wsl);
    let steps = || -> Result<(), String> {
        let hold = |ms: u32| {
            guest_hold(
                &mut crate::screen::Monitor::connect(monitor_port())?,
                ms,
                "steps",
            )
        };
        let dump = |k: usize, name: &str| -> Result<(), String> {
            let shot = scratch(root, &format!("steps-{k:02}-{name}.ppm"))?;
            let guest = if wsl {
                to_wsl_path(&shot)
            } else {
                shot.display().to_string()
            };
            crate::screen::Monitor::connect(monitor_port())?.screendump(&guest, &shot)
        };
        let mut qmp = crate::screen::Qmp::connect(monitor_port() + 1)?;
        // The default pack read and its pictures made.
        hold(8000)?;
        dump(0, "today")?;
        let list: [(&str, Option<(u16, u16)>); 10] = [
            ("weather", Some((240, 150))),
            ("place-menu", Some((100, 53))),
            ("place-picked", Some((100, 91))),
            ("home", None),
            ("weather-again", Some((240, 150))),
            ("hourly", Some((180, 580))),
            ("now", Some((60, 580))),
            ("place-menu-2", Some((100, 53))),
            ("place-closed", Some((400, 400))),
            ("home-2", None),
        ];
        for (k, (name, at)) in list.into_iter().enumerate() {
            match at {
                Some((x, y)) => {
                    touch(&mut qmp, &Action::TouchDown(x, y))?;
                    hold(150)?;
                    touch(&mut qmp, &Action::TouchUp)?;
                }
                None => {
                    qmp.key("f4", true)?;
                    hold(150)?;
                    qmp.key("f4", false)?;
                }
            }
            hold(2500)?;
            dump(k + 1, name)?;
        }
        // Back to the loader: Start, its last entry.
        Ok(())
    };
    let _ = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: sd.as_deref(),
            steps: &[],
            extra: &["-icount", "shift=0"],
            input: Some((&rp_loadproto::frame(&payload), rp_today::MARKER)),
            during: Some(&steps),
            then: None,
            timeout: Duration::from_secs(600),
        },
    );
    eprintln!("    steps: screens in target/emulator/steps-*.ppm");
    Ok(())
}

/// `cargo xtask emu-today-watch <recording>`: the owner's taps from a
/// device run (its run log's `R` lines, a file under the repo) replayed
/// on the image in the emulator, with the SD card holding the default
/// icon pack, the screen saved every second (`target/emulator/watch-NNN.ppm`)
/// to see what the panel showed (the run log says only what the image
/// believed it did).
pub fn today_watch(root: &Path, recording: &str) -> Result<(), String> {
    let release = crate::arm_release(root);
    let envs = [
        ("RP_TODAY_REPLAY", "1"),
        ("RP_TODAY_REPLAY_FILE", recording),
        ("RP_TODAY_REPLAY_GAP_MS", "3000"),
    ];
    let bin = variant_image_envs(root, "rp-today", &envs)?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let sd = today_card(root, &release)?;
    let wsl = matches!(emulator(root)?, Emulator::Wsl);
    let shots = || -> Result<(), String> {
        for k in 0..240 {
            poll_pause(Duration::from_millis(1000));
            let shot = scratch(root, &format!("watch-{k:03}.ppm"))?;
            let guest = if wsl {
                to_wsl_path(&shot)
            } else {
                shot.display().to_string()
            };
            let Ok(mut mon) = crate::screen::Monitor::connect(monitor_port()) else {
                return Ok(());
            };
            if mon.screendump(&guest, &shot).is_err() {
                return Ok(());
            }
        }
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: sd.as_deref(),
            steps: &[],
            extra: &["-icount", "shift=0"],
            input: Some((&rp_loadproto::frame(&payload), rp_today::MARKER)),
            during: Some(&shots),
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    if let Some((_, log)) = output.rsplit_once("RPL LOG") {
        let path = scratch(root, "watch-log.txt")?;
        std::fs::write(&path, log).map_err(|e| e.to_string())?;
        eprintln!("    watch: run log in {}", path.display());
    }
    Ok(())
}

pub(crate) fn today_replay_with(root: &Path, profile: bool) -> Result<(), String> {
    let mut envs = vec![("RP_TODAY_REPLAY", "1")];
    if profile {
        envs.push(("RP_TODAY_PROFILE", "1"));
    }
    let bin = variant_image_envs(root, "rp-today", &envs)?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), rp_today::MARKER)),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("today replay: never came back to the loader")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("today replay: didn't return cleanly:{log}"));
    }
    let Some(tr) = log.lines().find(|l| l.starts_with("TR touch replay: ")) else {
        return Err(format!("today replay: no `TR touch replay:` line:{log}"));
    };
    if !tr.contains(" 28 presses, ") {
        return Err(format!(
            "today replay: not the recording's 28 presses: {tr}"
        ));
    }
    if log.lines().any(|l| l.starts_with("R ")) {
        return Err(format!(
            "today replay: the recorder logged samples (it is off in the replay):{log}"
        ));
    }
    if profile {
        let zones = [
            "PROFILE touch sample (SPI): ",
            "PROFILE touch filter: ",
            "PROFILE shell frame (layout and draw list): ",
            "PROFILE compositor (2D queueing): ",
        ];
        for z in zones {
            let Some(line) = log.lines().find(|l| l.starts_with(z)) else {
                return Err(format!("today profile: no `{z}` line:{log}"));
            };
            if line.starts_with(&format!("{z}0 spans")) {
                return Err(format!("today profile: a zone never ran: {line}"));
            }
            eprintln!("      {}", line.trim_end());
        }
        eprintln!("    today profile OK: four zones counted over the replay");
        return Ok(());
    }
    if log.lines().any(|l| l.starts_with("PROFILE ")) {
        return Err(format!(
            "today replay: PROFILE lines without the profile:{log}"
        ));
    }
    eprintln!("      {}", tr.trim_end());
    eprintln!(
        "    today replay OK: 28 presses replayed and answered, back at the loader by itself (times are the emulator's)"
    );
    Ok(())
}
