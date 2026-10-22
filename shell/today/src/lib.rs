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
/// check's reference. `scratch` is the caller's (the image's is in `.bss`).
pub fn show_gpu<R: rp_hal_api::RegisterBus, M: rp_mbx_render::LocalMem>(
    r: &mut rp_mbx_render::Mbx2dRenderer<R, M>,
    wallpaper: &[u8],
    delay_us: fn(u32),
    scratch: &mut WallScratch,
) {
    load_wallpaper(r, wallpaper, Depth::Rgb565, scratch);
    r.show(delay_us);
}

/// How the wallpaper's rows are made. The renderer keeps the wallpaper as
/// RGB565 (ADR-0023 Q2 L1, 2026-10-21), taking each channel's top bits, so
/// [`Depth::Rgb565`] is the wallpaper; [`Depth::Full`] hands it 8 bits a
/// channel, which it truncates: plain 16 bits, no dither (kept for the
/// side-by-side compare, whose other side was 32-bit before L1; kept by
/// the owner as dithered against plain, quality audit C48).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    /// 8 bits a channel, as converted from the picture.
    Full,
    /// RGB565, dithered once on the CPU (Floyd-Steinberg), each channel
    /// widened back by repeating its top bits (as the engine reads a
    /// 16-bit wallpaper, measured 2026-10-17): the look the owner approved
    /// side by side on 2026-10-17.
    Rgb565,
}

/// Room for [`load_wallpaper`]'s work: a row, and the error the 16-bit
/// dither carries to the next row (~8 KiB: the image keeps it in `.bss`).
pub struct WallScratch {
    row: [u32; W],
    /// Error per channel for this row and the next, one column of margin
    /// each side.
    err: [[[i16; 3]; W + 2]; 2],
}

impl Default for WallScratch {
    fn default() -> Self {
        Self::new()
    }
}

impl WallScratch {
    /// Zeroed.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            row: [0; W],
            err: [[[0; 3]; W + 2]; 2],
        }
    }
}

/// A channel `v` (with error) to `bits`, then widened back to 8 bits by
/// repeating its top bits: (the shown value, the error left).
fn quantise(v: i16, bits: u32) -> (u8, i16) {
    let v = v.clamp(0, 255);
    let max = (1i16 << bits) - 1;
    let q = (v * max + 127) / 255;
    let shown = (q << (8 - bits)) | (q >> (2 * bits - 8));
    (shown as u8, v - shown)
}

/// `wallpaper` (planar 4:2:0) into the renderer's wallpaper at `depth`.
/// The caller then calls `wallpaper_changed` if a frame was drawn before.
pub fn load_wallpaper<R: rp_hal_api::RegisterBus, M: rp_mbx_render::LocalMem>(
    r: &mut rp_mbx_render::Mbx2dRenderer<R, M>,
    wallpaper: &[u8],
    depth: Depth,
    s: &mut WallScratch,
) {
    wallpaper_rows(wallpaper, depth, s, |y, row| r.wallpaper_row(y, row));
}

/// `wallpaper` (planar 4:2:0) as XRGB8888 rows at `depth`, each handed to
/// `f` with its line number.
pub fn wallpaper_rows(
    wallpaper: &[u8],
    depth: Depth,
    s: &mut WallScratch,
    mut f: impl FnMut(u16, &[u32]),
) {
    use rp_drv_mbx_display::scanout::yuv_to_rgb;
    let (luma, chroma) = wallpaper.split_at((W * H).min(wallpaper.len()));
    let (u, v) = chroma.split_at((W * H / 4).min(chroma.len()));
    s.err = [[[0; 3]; W + 2]; 2];
    for y in 0..H {
        let (cur, next) = (y % 2, (y + 1) % 2);
        s.err[next] = [[0; 3]; W + 2];
        for x in 0..W {
            let c = (y / 2) * (W / 2) + x / 2;
            let (r8, g8, b8) = yuv_to_rgb(
                luma.get(y * W + x).copied().unwrap_or(16),
                u.get(c).copied().unwrap_or(128),
                v.get(c).copied().unwrap_or(128),
            );
            // The default picture (the owner, 2026-10-22), before the dither.
            let (r8, g8, b8) = if rp_draw::look::ON {
                rp_draw::look::apply(r8, g8, b8)
            } else {
                (r8, g8, b8)
            };
            let px = match depth {
                Depth::Full => [r8, g8, b8],
                Depth::Rgb565 => {
                    let mut out = [0u8; 3];
                    for (k, (v8, bits)) in [(r8, 5), (g8, 6), (b8, 5)].into_iter().enumerate() {
                        let (shown, e) = quantise(i16::from(v8) + s.err[cur][x + 1][k] / 16, bits);
                        out[k] = shown;
                        // Floyd-Steinberg: 7 right, 3 below left, 5 below,
                        // 1 below right (sixteenths, divided on use).
                        s.err[cur][x + 2][k] += e * 7;
                        s.err[next][x][k] += e * 3;
                        s.err[next][x + 1][k] += e * 5;
                        s.err[next][x + 2][k] += e;
                    }
                    out
                }
            };
            s.row[x] =
                0xFF00_0000 | u32::from(px[0]) << 16 | u32::from(px[1]) << 8 | u32::from(px[2]);
        }
        f(y as u16, &s.row);
    }
}

/// The probe's test pixels: every red and blue level, and every green one.
pub const PROBE_PIXELS: usize = 64;

/// Test pixel `i` of the probe, RGB565.
#[must_use]
pub const fn probe_pixel(i: usize) -> u16 {
    let (r, g, b) = ((i & 31) as u16, (i & 63) as u16, (31 - (i & 31)) as u16);
    r << 11 | g << 5 | b
}

/// `p` (RGB565) widened to XRGB8888 two ways: (repeating the top bits,
/// filling with zeros).
#[must_use]
pub fn widened(p: u16) -> (u32, u32) {
    let (r, g, b) = (
        u32::from(p >> 11),
        u32::from((p >> 5) & 63),
        u32::from(p & 31),
    );
    let rep = (r << 3 | r >> 2) << 16 | (g << 2 | g >> 4) << 8 | (b << 3 | b >> 2);
    let zero = (r << 3) << 16 | (g << 2) << 8 | (b << 3);
    (rep, zero)
}

/// What the engine made of the probe's pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Widen {
    /// Widened by repeating the top bits (and not by zeros).
    pub replicated: u32,
    /// Widened by zeros (and not by repeating).
    pub zero_filled: u32,
    /// The same both ways (the low levels).
    pub either: u32,
    /// Neither: (the source pixel, what came out), the first one.
    pub other: u32,
    /// The first pixel that was neither.
    pub first_other: Option<(u16, u32)>,
}

/// Sorts what came out for each probe pixel (`got(i)`, colour bits only).
pub fn classify(mut got: impl FnMut(usize) -> u32) -> Widen {
    let mut w = Widen::default();
    for i in 0..PROBE_PIXELS {
        let p = probe_pixel(i);
        let g = got(i) & 0xFF_FFFF;
        let (rep, zero) = widened(p);
        match (g == rep, g == zero) {
            (true, true) => w.either += 1,
            (true, false) => w.replicated += 1,
            (false, true) => w.zero_filled += 1,
            (false, false) => {
                w.other += 1;
                w.first_other.get_or_insert((p, g));
            }
        }
    }
    w
}

/// Asks the 2D engine how it widens a 16-bit source copied into 32 bits
/// (read-only for the screen: it uses `at`, 2700G memory nothing else
/// holds, and 64 words after it). `None` if the engine didn't finish.
pub fn probe_widen<R: rp_hal_api::RegisterBus, M: rp_mbx_render::LocalMem>(
    r: &mut rp_mbx_render::Mbx2dRenderer<R, M>,
    at: usize,
) -> Option<Widen> {
    use rp_drv_blit2d::{Format, ROP_SRCCOPY, blit};
    let (e, mem) = r.parts();
    let mut words = [0u32; PROBE_PIXELS / 2];
    for (k, w) in words.iter_mut().enumerate() {
        *w = u32::from(probe_pixel(2 * k)) | u32::from(probe_pixel(2 * k + 1)) << 16;
    }
    mem.store(at, &words);
    let out = at + 256;
    mem.store(out, &[0x5A5A_5A5A; PROBE_PIXELS]);
    mem.sync();
    let (w, n) = blit(
        (Format::BPP32, out, (PROBE_PIXELS * 4) as i32),
        (Format::RGB565, at, (PROBE_PIXELS * 2) as i32),
        (0, 0),
        (0, 0, PROBE_PIXELS as u16, 1),
        ROP_SRCCOPY,
        0,
        0,
    )?;
    e.submit(&w[..n]).ok()?;
    e.finish_with(20_000, |a| mem.load(a)).ok()?;
    Some(classify(|i| mem.load(out + 4 * i).unwrap_or(0)))
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
    fn the_probe_tells_repeating_from_zeros() {
        // Repeating the top bits: white stays white; zeros: it doesn't.
        assert_eq!(widened(0xFFFF), (0xFF_FFFF, 0xF8_FCF8));
        assert_eq!(widened(0), (0, 0));
        let rep = classify(|i| widened(probe_pixel(i)).0);
        let zero = classify(|i| widened(probe_pixel(i)).1);
        assert_eq!(rep.replicated + rep.either, PROBE_PIXELS as u32);
        assert_eq!(zero.zero_filled + zero.either, PROBE_PIXELS as u32);
        assert!(
            rep.replicated > 50 && zero.zero_filled > 50,
            "{rep:?} {zero:?}"
        );
        let odd = classify(|_| 0x12_3456);
        assert_eq!(odd.other, PROBE_PIXELS as u32);
        assert_eq!(odd.first_other, Some((probe_pixel(0), 0x12_3456)));
    }

    /// The 16-bit wallpaper holds only colours RGB565 can hold (as the
    /// engine would show them, top bits repeated) and, dithered, keeps
    /// the picture's average: a smooth gradient's 16 x 16 blocks within a
    /// level of the full-depth picture's (a block's error partly leaves it
    /// across its edges, so smaller blocks drift more).
    #[test]
    fn the_16_bit_wallpaper_is_rgb565_and_keeps_the_average() {
        use rp_mbx_render::model::{self, Bus, Mem};
        use rp_mbx_render::{Layout, Mbx2dRenderer, STRIDE16, to565};

        // A slow diagonal gradient in every channel (luma, then U and V).
        let mut wall = vec![0u8; W * H * 3 / 2];
        for y in 0..H {
            for x in 0..W {
                wall[y * W + x] = (40 + (x + y) * 160 / (W + H)) as u8;
            }
        }
        for (i, b) in wall[W * H..].iter_mut().enumerate() {
            *b = (110 + (i % (W / 2)) * 40 / (W / 2)) as u8;
        }
        let read = |depth| {
            let mut px = Vec::with_capacity(W * H);
            wallpaper_rows(&wall, depth, &mut WallScratch::new(), |_, row| {
                px.extend_from_slice(row);
            });
            px
        };
        let full = read(Depth::Full);
        let low = read(Depth::Rgb565);
        // The renderer keeps the 16-bit rows exactly (RGB565, the top bits).
        let mem = model::memory();
        let mut r = Mbx2dRenderer::new(
            Bus::new(mem.clone()),
            Mem(mem.clone()),
            Layout::X51V,
            || 0,
            1,
        );
        load_wallpaper(&mut r, &wall, Depth::Rgb565, &mut WallScratch::new());
        let at = Layout::X51V.wallpaper;
        let m = mem.borrow();
        for (i, &p) in low.iter().enumerate() {
            let a = at + (i / W) * STRIDE16 + (i % W) * 2;
            let kept = u16::from_le_bytes([m[a], m[a + 1]]);
            assert_eq!(kept, to565(p), "pixel {i}");
            assert_eq!(
                model::widen(kept),
                p | 0xFF00_0000,
                "pixel {i} widened back"
            );
        }
        for &p in &low {
            let r5 = (p >> 19) & 31;
            let g6 = (p >> 10) & 63;
            let b5 = (p >> 3) & 31;
            let as565 = (r5 << 11 | g6 << 5 | b5) as u16;
            assert_eq!(
                p & 0xFF_FFFF,
                widened(as565).0,
                "{p:08x} isn't a 16-bit colour"
            );
        }
        for by in (0..H).step_by(16) {
            for bx in (0..W).step_by(16) {
                for sh in [16, 8, 0] {
                    let sum = |v: &[u32]| -> i64 {
                        (0..16)
                            .flat_map(|dy| (0..16).map(move |dx| (by + dy) * W + bx + dx))
                            .map(|i| i64::from((v[i] >> sh) & 0xFF))
                            .sum()
                    };
                    let d = (sum(&low) - sum(&full)).abs();
                    assert!(
                        d <= 256,
                        "block ({bx}, {by}) channel {sh}: off by {d} / 256"
                    );
                }
            }
        }
    }

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
        use rp_drv_mbx_display::scanout::mix;
        use rp_gfx::layer::{Layer, PlaneFormat};
        use rp_gfx::render::CpuRenderer;
        use rp_mbx_render::model::{self, Bus, Mem};
        use rp_mbx_render::{Layout, Mbx2dRenderer, PICTURE_BYTES, STRIDE, STRIDE16};
        use rp_shell::{Input, Shell};

        // A wallpaper with every channel moving: luma, then U and V.
        let mut wall = vec![0u8; W * H * 3 / 2];
        for (i, b) in wall.iter_mut().enumerate() {
            *b = (16 + (i * 7 + i / W * 3) % 220) as u8;
        }
        // The wallpaper as the renderer keeps it (16 bits, dithered once).
        let mut wall16 = Vec::with_capacity(W * H);
        wallpaper_rows(&wall, Depth::Rgb565, &mut WallScratch::new(), |_, row| {
            wall16.extend_from_slice(row);
        });
        let rgb = |x: usize, y: usize| {
            let p = wall16[y * W + x];
            ((p >> 16) as u8, (p >> 8) as u8, p as u8)
        };
        let mem = model::memory();
        let mut gpu = Mbx2dRenderer::new(
            Bus::new(mem.clone()),
            Mem(mem.clone()),
            Layout::X51V,
            || 0,
            1,
        );
        show_gpu(&mut gpu, &wall, |_| {}, &mut WallScratch::new());
        gpu.dither = false;
        let mut work = vec![0u8; PICTURE_BYTES];
        let mut plane = vec![0u8; PICTURE_BYTES];
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
                    let s = base + y * STRIDE16 + x * 2;
                    let g = model::widen(u16::from_le_bytes([m[s], m[s + 1]]));
                    let (wr, wg, wb) = rgb(x, y);
                    let a = (l >> 24) as u8;
                    for (sh, w, bits) in [(16, wr, 5), (8, wg, 6), (0, wb, 5)] {
                        // The screen is the composition truncated: how far
                        // the reference lies outside the truncated step.
                        let want = mix((l >> sh) as u8, w, a);
                        let lost = 8 - bits;
                        let lo = ((g >> sh) as u8 >> lost) << lost;
                        let hi = lo | ((1u8 << lost) - 1);
                        let d = if want < lo {
                            lo - want
                        } else {
                            want.saturating_sub(hi)
                        };
                        worst = worst.max(d);
                        over2 += usize::from(d > 2);
                    }
                }
            }
            // Measured at 32 bits: 3 at worst, in about a dozen of 921,600
            // channel values a frame, where glass is stacked on glass (each
            // engine blend truncates; the CPU composes the layer once).
            // With the weather icons baked in (local builds; CI builds
            // without them), 114 or 152 a frame, still 3 at worst: their
            // see-through edges over the glass are one more truncating blend
            // (2026-10-17). Now the screen is RGB565: the reference must lie
            // in (or within 3 of) the step the screen shows.
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
        show_gpu(&mut gpu, &wall, |_| {}, &mut WallScratch::new());
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
