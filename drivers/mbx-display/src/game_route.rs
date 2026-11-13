//! The game path's route switch, way C' (ADR-0038 G2, ticked by the owner
//! 2026-10-29 on R101's and R108's runs; R90's driver).
//!
//! The video plane is never hidden in a game. For the native and lines
//! routes it sits under an opaque graphics plane, its base pointed at the
//! shown picture (so it reads inside the safe region); for the smooth route
//! the graphics plane goes see-through over it and its base points at the
//! UYVY picture. A switch is the graphics plane's registers and the video
//! plane's base, written on scan line 0 (the registers act at once; the
//! clean window is lines 643-3 of the 645-line frame): the video base
//! repointed first going into the smooth route, last coming out, so the
//! plane shown never reads a picture being left.
//!
//! **Rule 0.** Windows Mobile has no route switch (one picture, one size);
//! Dell's driver shows the video plane by its stream bit and today's
//! hide-and-show tear one frame entering the smooth route (R101 runs 1-3).
//! Consoles change render size behind a full-screen copy. Ours changes
//! only what the display reads, in the blanking, with no copy: measured
//! +0.25% of a native render (~50 us), 2,400 of 2,400 frames whole by
//! `DSIG` switching every frame and every 8 (gpubench-0211), and with the
//! 3D core drawing at 149.50 / 2, 3,748 of 3,748 (gpubench-0214).
//!
//! This is `rp-gpubench`'s way 3 (`gc2dc_way_show`), moved here so the
//! game path and the device runs share one copy.

use crate::layers::{Blend, Layer, Slot, reg as lreg};
use crate::{PixelFormat, Plane, graphics_set, reg};
use rp_hal_api::RegisterBus;

/// Native, 480 x 640.
pub const NATIVE: u8 = 0;
/// 480 x 320, each line shown twice.
pub const LINES: u8 = 1;
/// 240 x 320 UYVY, the video plane's scaler 2x.
pub const SMOOTH: u8 = 2;

/// The graphics plane over the video plane on the smooth route: the
/// middle slot, see-through (global alpha 0), as R101's P9c arrangement.
const OVER_SMOOTH: Layer = Layer {
    slot: Slot::Middle,
    blend: Blend::Global(0),
};

/// The graphics plane on the native and lines routes: the middle slot,
/// opaque over the video plane.
const OVER_VIDEO: Layer = Layer {
    slot: Slot::Middle,
    blend: Blend::Opaque,
};

/// The video plane's base pointed at `at` (its other `VSCADR` bits kept).
fn video_base<B: RegisterBus>(regs: &mut B, at: u32) {
    let v = regs.read32(lreg::VSCADR);
    regs.write32(lreg::VSCADR, (v & !0x7F_FFFF) | (at >> 4));
}

/// The graphics plane for `route` (`at`: each route's picture in 2700G
/// memory; the smooth route's is the UYVY the video plane reads).
fn graphics<B: RegisterBus>(regs: &mut B, route: u8, at: [u32; 3]) {
    if route == SMOOTH {
        let screen = Plane {
            format: PixelFormat::Rgb565,
            width: 480,
            height: 640,
            offset: 0,
            stride: 960,
        };
        graphics_set(regs, &screen, OVER_SMOOTH, false);
        return;
    }
    let plane = Plane {
        format: PixelFormat::Rgb565,
        width: 480,
        height: if route == LINES { 320 } else { 640 },
        offset: at[usize::from(route)] as usize,
        stride: 960,
    };
    graphics_set(regs, &plane, OVER_VIDEO, false);
    if route == LINES {
        let v = regs.read32(reg::GDRCTRL);
        regs.write32(reg::GDRCTRL, (v & !(1 << 31)) | 1 << 29);
    }
}

/// Shows `route` (way C'). Call on scan line 0 (the caller waits for it:
/// the registers act at once).
pub fn show<B: RegisterBus>(regs: &mut B, route: u8, at: [u32; 3]) {
    if route == SMOOTH {
        video_base(regs, at[2]);
        graphics(regs, route, at);
    } else {
        graphics(regs, route, at);
        video_base(regs, at[usize::from(route).min(2)]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A register file that records every write in order.
    struct Regs {
        v: std::collections::BTreeMap<usize, u32>,
        log: std::vec::Vec<(usize, u32)>,
    }

    impl RegisterBus for Regs {
        fn read32(&mut self, off: usize) -> u32 {
            self.v.get(&off).copied().unwrap_or(0)
        }
        fn write32(&mut self, off: usize, v: u32) {
            self.v.insert(off, v);
            self.log.push((off, v));
        }
    }

    fn regs() -> Regs {
        let mut v = std::collections::BTreeMap::new();
        v.insert(lreg::VSCADR, 0x8000_0000 | 0x12);
        Regs {
            v,
            log: std::vec::Vec::new(),
        }
    }

    const AT: [u32; 3] = [0x0, 0x4_B000, 0x9_6000];

    #[test]
    fn into_the_smooth_route_the_video_base_moves_first_and_out_of_it_last() {
        let mut r = regs();
        show(&mut r, SMOOTH, AT);
        let first_video = r.log.iter().position(|&(o, _)| o == lreg::VSCADR).unwrap();
        assert_eq!(first_video, 0, "the video base first going in");
        assert_eq!(r.v[&lreg::VSCADR] & 0x7F_FFFF, AT[2] >> 4);
        assert_eq!(
            r.v[&lreg::VSCADR] & 0x8000_0000,
            0x8000_0000,
            "the plane kept shown"
        );
        let mut r = regs();
        show(&mut r, NATIVE, AT);
        assert_eq!(
            r.log.last().unwrap().0,
            lreg::VSCADR,
            "the video base last coming out"
        );
        assert_eq!(r.v[&lreg::VSCADR] & 0x7F_FFFF, AT[0] >> 4);
    }

    #[test]
    fn the_lines_route_doubles_lines_and_never_pixels() {
        let mut r = regs();
        r.v.insert(reg::GDRCTRL, 1 << 31);
        show(&mut r, LINES, AT);
        let g = r.v[&reg::GDRCTRL];
        assert_eq!(g & (1 << 29), 1 << 29);
        assert_eq!(g & (1 << 31), 0);
        assert_eq!(r.v[&lreg::VSCADR] & 0x7F_FFFF, AT[1] >> 4);
    }
}
