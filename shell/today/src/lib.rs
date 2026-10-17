//! Test image (ADR-0021): the shell's Today screen on the X51v.
//!
//! Drawn by the 2700G's 2D engine (ADR-0023, option B: `rp-mbx-render`):
//! the wallpaper, converted once to XRGB8888 in 2700G memory, is copied
//! under each changed rectangle and `rp-shell`'s frame drawn over it into
//! one of two opaque 32 bpp pictures, then the screen flips to it. The CPU
//! only describes frames. ([`show`], the earlier set-up: wallpaper in the
//! video plane under the CPU renderer's ARGB4444 layer, stays as the
//! reference the GPU path is tested against.) Touch and the D-pad drive
//! the shell; Home goes to Today; the Start menu's "Back to loader"
//! returns.
//!
//! Tickless: between events the CPU sleeps until the pen or a button
//! interrupts, or an alarm (the pen sampled every 5 ms while it is down, a
//! held key's repeat, the next minute for the clock, a notice's end).
//!
//! **The 50 ms promise, measured** (design brief): every pen-down and
//! pen-up that changes the screen is timed from the moment it is read to
//! the end of the flush that shows it; the run log gets the count, median
//! and worst. On the glass it shows at the next refresh after the flush
//! (at most one more frame; the vsync wait isn't known yet, docs/hw/2700g.md).
//!
//! The clock starts from the build machine's local time (no RTC driver
//! yet). Uses 2700G memory from 6 MiB up and the scratch region; every
//! display register it changes is put back before it returns.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod clock;
pub mod fonts;

/// Printed in the run log once the Today screen is up.
pub const MARKER: &str = "RETROPANE-TODAY-UP";

/// The shell's fonts, as generated.
#[must_use]
pub fn shell_fonts() -> rp_shell::Fonts {
    rp_shell::Fonts {
        ui: &fonts::UI,
        label: &fonts::LABEL,
        big: &fonts::BIG,
    }
}

/// The weather pictures the Today screen shows, from `local/icons/icons.rs`
/// (made by `tools/shell-assets/icons.py` from the default pack; without it
/// the pictures are empty and the screen draws outlines).
mod icon_data {
    use rp_draw::Image;
    include!(concat!(env!("OUT_DIR"), "/icons.rs"));
}

/// The icons for the shell: none when the pictures weren't made.
#[must_use]
pub fn icons() -> rp_shell::Icons {
    if icon_data::HERO.w == 0 {
        return rp_shell::Icons::NONE;
    }
    let s = &icon_data::SMALL;
    rp_shell::Icons {
        hero: Some(&icon_data::HERO),
        forecast: [
            Some(&s[0]),
            Some(&s[1]),
            Some(&s[2]),
            Some(&s[3]),
            Some(&s[4]),
        ],
    }
}

/// Screen width and height.
pub const W: usize = 480;
/// Screen height.
pub const H: usize = 640;
/// ADR-0023's set-up: `wallpaper` (planar 4:2:0) converted to XRGB8888
/// into the renderer's wallpaper, the engine reset and the first picture
/// shown as an opaque 32 bpp plane. Shared by the image and the emulator
/// check's reference.
pub fn show_gpu<R: rp_hal_api::RegisterBus, M: rp_mbx_render::LocalMem>(
    r: &mut rp_mbx_render::Mbx2dRenderer<R, M>,
    wallpaper: &[u8],
    delay_us: fn(u32),
) {
    use rp_drv_mbx_display::scanout::yuv_to_rgb;
    let (luma, chroma) = wallpaper.split_at((W * H).min(wallpaper.len()));
    let (u, v) = chroma.split_at((W * H / 4).min(chroma.len()));
    let mut row = [0u32; W];
    for y in 0..H {
        for (x, p) in row.iter_mut().enumerate() {
            let c = (y / 2) * (W / 2) + x / 2;
            let (r, g, b) = yuv_to_rgb(
                luma.get(y * W + x).copied().unwrap_or(16),
                u.get(c).copied().unwrap_or(128),
                v.get(c).copied().unwrap_or(128),
            );
            *p = 0xFF00_0000 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b);
        }
        r.wallpaper_row(y as u16, &row);
    }
    r.show(delay_us);
}

/// Latencies kept for the median (the rest only count towards the worst).
pub const KEPT: usize = 256;

/// Touch-to-flush times in microseconds.
pub struct Latency {
    kept: [u32; KEPT],
    n: usize,
    count: u32,
    worst: u32,
}

impl Default for Latency {
    fn default() -> Self {
        Self::new()
    }
}

impl Latency {
    /// None yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            kept: [0; KEPT],
            n: 0,
            count: 0,
            worst: 0,
        }
    }

    /// One more.
    pub fn add(&mut self, us: u32) {
        if let Some(slot) = self.kept.get_mut(self.n) {
            *slot = us;
            self.n += 1;
        }
        self.count += 1;
        self.worst = self.worst.max(us);
    }

    /// (count, median of the kept ones, worst); median 0 with none.
    #[must_use]
    pub fn summary(&mut self) -> (u32, u32, u32) {
        let kept = &mut self.kept[..self.n];
        kept.sort_unstable();
        let median = kept.get(kept.len() / 2).copied().unwrap_or(0);
        (self.count, median, self.worst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_summary() {
        let mut l = Latency::new();
        assert_eq!(l.summary(), (0, 0, 0));
        for us in [30_000, 10_000, 20_000] {
            l.add(us);
        }
        assert_eq!(l.summary(), (3, 20_000, 30_000));
        for _ in 0..KEPT * 2 {
            l.add(1);
        }
        let (n, _, worst) = l.summary();
        assert_eq!((n, worst), (3 + 2 * KEPT as u32, 30_000));
    }

    /// The Today screen through the GPU renderer (on the engine model)
    /// against the CPU renderer's layer over the same wallpaper, frame
    /// after frame through a tap on Start, the menu and back: within 3
    /// per channel (the engine weighs alpha out of 256 and truncates each
    /// blend; the CPU rounds out of 255 and mixes once).
    #[test]
    fn gpu_today_matches_the_cpu_renderer() {
        use rp_draw::{Frame, Renderer};
        use rp_drv_mbx_display::scanout::{mix, yuv_to_rgb};
        use rp_gfx::layer::{Layer, PlaneFormat};
        use rp_gfx::render::CpuRenderer;
        use rp_mbx_render::model::{self, Bus, Mem};
        use rp_mbx_render::{Layout, Mbx2dRenderer, SCREEN_BYTES, STRIDE};
        use rp_shell::{Input, Shell};

        // A wallpaper with every channel moving: luma, then U and V.
        let mut wall = vec![0u8; W * H * 3 / 2];
        for (i, b) in wall.iter_mut().enumerate() {
            *b = (16 + (i * 7 + i / W * 3) % 220) as u8;
        }
        let rgb = |x: usize, y: usize| {
            let c = W * H + (y / 2) * (W / 2) + x / 2;
            yuv_to_rgb(wall[y * W + x], wall[c], wall[c + W * H / 4])
        };
        let mem = model::memory();
        let mut gpu = Mbx2dRenderer::new(
            Bus::new(mem.clone()),
            Mem(mem.clone()),
            Layout::X51V,
            || 0,
            1,
        );
        show_gpu(&mut gpu, &wall, |_| {});
        gpu.dither = false;
        let mut work = vec![0u8; SCREEN_BYTES];
        let mut plane = vec![0u8; SCREEN_BYTES];
        let mut shell = Shell::new(shell_fonts(), icons(), true);
        // The emulator's run showed this; frames differ by their text.
        shell.set_time(rp_shell::Time {
            hour: 12,
            minute: 43,
            weekday: 2,
            day: 13,
            month: 10,
        });
        let mut f = Frame::new();
        let s = rp_shell::START;
        let (sx, sy) = (s.x + s.w / 2, s.y + s.h / 2);
        let mut steps = vec![
            None,
            Some(Input::PenDown { x: sx, y: sy }),
            Some(Input::PenUp { x: sx, y: sy }),
            Some(Input::Key(rp_shell::Key::Down)),
            Some(Input::Key(rp_shell::Key::Home)),
        ];
        // The menu opened and closed again and again: from the third time
        // its areas come from the renderer's cache, and must still match.
        for _ in 0..3 {
            for _ in 0..2 {
                steps.push(Some(Input::PenDown { x: sx, y: sy }));
                steps.push(Some(Input::PenUp { x: sx, y: sy }));
            }
        }
        let mut hits = 0;
        for (k, step) in steps.into_iter().enumerate() {
            if let Some(e) = step {
                shell.input(e);
            }
            shell.frame(&mut f);
            gpu.draw(&f);
            assert!(gpu.poll());
            hits += gpu.last.cache_hits;
            let layer = Layer::in_bytes(&mut work, W as u32, H as u32).unwrap();
            CpuRenderer::new(layer, &mut plane, STRIDE, PlaneFormat::Argb8888).draw(&f);
            let scadr = gpu.parts().0.regs.reg(rp_drv_mbx_display::reg::GSCADR);
            let base = ((scadr & 0x7F_FFFF) * 16) as usize;
            let m = mem.borrow();
            let (mut worst, mut over2) = (0u8, 0usize);
            for y in 0..H {
                for x in 0..W {
                    let at = (y * W + x) * 4;
                    let l = u32::from_le_bytes(plane[at..at + 4].try_into().unwrap());
                    let g = u32::from_le_bytes(m[base + at..base + at + 4].try_into().unwrap());
                    let (wr, wg, wb) = rgb(x, y);
                    let a = (l >> 24) as u8;
                    for (sh, w) in [(16, wr), (8, wg), (0, wb)] {
                        let want = mix((l >> sh) as u8, w, a);
                        let got = (g >> sh) as u8;
                        let d = got.abs_diff(want);
                        worst = worst.max(d);
                        over2 += usize::from(d > 2);
                    }
                }
            }
            // Measured: 3 at worst, in about a dozen of 921,600 channel
            // values a frame, where glass is stacked on glass (each engine
            // blend truncates; the CPU composes the layer once). With the
            // weather icons baked in (local builds; CI builds without them),
            // 114 or 152 a frame, still 3 at worst: their see-through edges
            // over the glass are one more truncating blend (2026-10-17).
            assert!(
                worst <= 3 && over2 < 256,
                "step {k}: worst {worst}, {over2} channels off by more than 2"
            );
        }
        assert_eq!(gpu.stalls, 0);
        assert!(hits >= 4, "cache hits {hits}");
    }

    /// What each frame of opening and closing the Start menu costs the
    /// CPU: atlases built, pictures uploaded, mid-frame waits, words queued.
    #[test]
    fn menu_frames_cost() {
        use rp_draw::{Frame, Renderer};
        use rp_mbx_render::model::{self, Bus, Mem};
        use rp_mbx_render::{Layout, Mbx2dRenderer};
        use rp_shell::{Input, Shell};

        let wall = vec![128u8; W * H * 3 / 2];
        let mem = model::memory();
        let mut gpu = Mbx2dRenderer::new(
            Bus::new(mem.clone()),
            Mem(mem.clone()),
            Layout::X51V,
            || 0,
            1,
        );
        show_gpu(&mut gpu, &wall, |_| {});
        let mut shell = Shell::new(shell_fonts(), icons(), true);
        let mut f = Frame::new();
        let s = rp_shell::START;
        let (sx, sy) = (s.x + s.w / 2, s.y + s.h / 2);
        let mut steps = vec![("first", None)];
        for _ in 0..3 {
            steps.push(("start down", Some(Input::PenDown { x: sx, y: sy })));
            steps.push(("start up (menu)", Some(Input::PenUp { x: sx, y: sy })));
            steps.push(("start down", Some(Input::PenDown { x: sx, y: sy })));
            steps.push(("start up (close)", Some(Input::PenUp { x: sx, y: sy })));
        }
        for (name, step) in steps {
            if let Some(e) = step {
                shell.input(e);
            }
            if !shell.needs_frame() {
                eprintln!("{name}: no frame");
                continue;
            }
            shell.frame(&mut f);
            gpu.draw(&f);
            assert!(gpu.poll());
            let t = gpu.last;
            eprintln!(
                "{name}: rects {}, forward {}, hits {}, stores {}, bg {}, atlases {}, images {}, waits {}, words {}, blits {}, px copy {} blend {} text {} dither {}",
                t.rects,
                t.forward,
                t.cache_hits,
                t.cache_stores,
                t.background,
                t.atlases,
                t.images,
                t.waits,
                t.words,
                t.blits,
                t.px_copy,
                t.px_blend,
                t.px_text,
                t.px_dither
            );
        }
    }

    #[test]
    fn the_real_fonts_fit_the_shell() {
        // Every frame of the real Today screen fits, menu open included.
        let mut s = rp_shell::Shell::new(shell_fonts(), icons(), true);
        let mut f = rp_draw::Frame::new();
        s.frame(&mut f);
        assert!(!f.overflowed());
        s.input(rp_shell::Input::Key(rp_shell::Key::Enter));
        s.input(rp_shell::Input::Key(rp_shell::Key::Enter));
        assert!(s.menu_open());
        s.frame(&mut f);
        assert!(!f.overflowed());
        // The big font has the clock's digits and the degree sign.
        for c in b"0123456789:\xb0" {
            assert!(
                fonts::BIG.glyph(*c).is_some_and(|g| g.w > 0),
                "{}",
                *c as char
            );
        }
    }
}
