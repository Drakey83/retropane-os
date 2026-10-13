//! Test image (ADR-0021): the shell's Today screen on the X51v.
//!
//! The wallpaper goes in the 2700G's video plane (4:2:0, opaque, at the
//! bottom), `rp-shell` is drawn by the CPU renderer into the graphics plane
//! (ARGB4444, dithered, per-pixel alpha) and the display mixes the two as
//! it scans out: the plane set-up `rp-blend` proved as stage F. Touch and
//! the D-pad drive the shell; Home goes to Today; the Start menu's "Back
//! to loader" returns.
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

/// Screen width and height.
pub const W: usize = 480;
/// Screen height.
pub const H: usize = 640;
/// The wallpaper's video plane in 2700G memory (above the loader's and
/// WM's planes at 0).
pub const VIDEO_AT: usize = 0x60_0000;
/// The interface's graphics plane.
pub const GRAPHICS_AT: usize = 0xA0_0000;

/// Puts `wallpaper` (planar 4:2:0, `W * H * 3 / 2` bytes) in the video
/// plane, clears the graphics plane and shows both, mixed at scan-out:
/// stage F's set-up from `rp-blend`. Returns the graphics plane. Shared by
/// the image and the emulator check's reference, so they can't drift.
pub fn show<B: rp_hal_api::RegisterBus>(
    d: &mut rp_drv_mbx_display::Display<B>,
    vram: &mut [u8],
    wallpaper: &[u8],
) -> rp_drv_mbx_display::Plane {
    use rp_drv_mbx_display::blendtest::{GRAPHICS, VIDEO};
    use rp_drv_mbx_display::layers::{VideoFormat, VideoPlane};
    use rp_drv_mbx_display::{PixelFormat, Plane};

    let video = VideoPlane::new(VideoFormat::Yuv420, W as u16, H as u16, VIDEO_AT);
    let (luma, chroma) = wallpaper.split_at((W * H).min(wallpaper.len()));
    let (u, v) = chroma.split_at((W * H / 4).min(chroma.len()));
    for (y, row) in luma.chunks(W).enumerate() {
        let at = video.offset + y * video.stride;
        vram[at..at + row.len()].copy_from_slice(row);
    }
    for (plane, at0) in [(u, video.u_offset), (v, video.v_offset)] {
        for (y, row) in plane.chunks(W / 2).enumerate() {
            let at = at0 + y * (video.stride / 2);
            vram[at..at + row.len()].copy_from_slice(row);
        }
    }
    let gfx = Plane {
        format: PixelFormat::Argb4444,
        width: W as u16,
        height: H as u16,
        offset: GRAPHICS_AT,
        stride: W * 2,
    };
    // Clear before it shows: nothing but the wallpaper for a moment.
    vram[gfx.offset..gfx.offset + gfx.stride * H].fill(0);
    d.show_video(&video, VIDEO);
    d.set_graphics(&gfx, GRAPHICS);
    gfx
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

    #[test]
    fn the_real_fonts_fit_the_shell() {
        // Every frame of the real Today screen fits, menu open included.
        let mut s = rp_shell::Shell::new(shell_fonts(), true);
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
