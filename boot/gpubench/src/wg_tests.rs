//! Run WG's shared part on the host: every window state inside its
//! picture and on a 16-byte start, the scripts' ends, the strip's pixel
//! steps, the engine's "over" at its edges, and the L2 layouts composed
//! through the scan-out model (a clear window reads as screen 0 alone).

extern crate std;

use std::vec;
use std::vec::Vec;

use rp_drv_mbx_display::layers::{Blend, Layer, Scale, Slot, VideoFormat, VideoPlane};
use rp_drv_mbx_display::{PixelFormat, Plane, reg, scanout};

use super::*;

/// Every state each test of the run shows, with its picture's size.
fn states() -> Vec<(&'static str, Win, (u32, u32))> {
    let mut v = Vec::new();
    for f in 0..FLING_FRAMES {
        v.push(("fling", page_win(fling(f)), (W, PIC_H)));
    }
    for k in 0..=ENTRANCE_FRAMES {
        v.push(("reveal", reveal(k), (W, PIC_H)));
        v.push(("cascade", cascade(k), (W, PIC_H)));
    }
    for p in (0..=W).step_by(4) {
        v.push(("strip4", strip4(p), (STRIP_W, WIN_H)));
    }
    for p in 0..=W {
        v.push(("strip1", strip1(p), (STRIP_W, WIN_H)));
    }
    v
}

#[test]
fn every_state_reads_inside_its_picture_on_a_16_byte_start() {
    for (name, s, (pw, ph)) in states() {
        assert!(
            s.w >= 2 && s.h >= 2 && s.w.is_multiple_of(2) && s.h.is_multiple_of(2),
            "{name} {s:?}"
        );
        assert!(s.x + s.w <= W && s.y + s.h <= H, "{name} on screen {s:?}");
        assert!(
            s.y >= WIN_Y && s.y + s.h <= SOFTKEYS_Y,
            "{name} over the chrome {s:?}"
        );
        assert!(
            s.col + s.w <= pw && s.row + s.h <= ph,
            "{name} past the picture {s:?}"
        );
        let stride = pw * 4;
        assert_eq!(s.start(0x7A_0000, stride, 4) % 16, 0, "{name} {s:?}");
    }
    // The fling's ARGB4444 picture (V3): the page's window at any scroll.
    for f in 0..FLING_FRAMES {
        assert_eq!(page_win(fling(f)).start(0x7A_0000, W * 2, 2) % 16, 0);
    }
    // V3 is ARGB4444: Reveal on 8-pixel steps and the turn to the pixel on
    // 8-pixel starts, 16-byte aligned at 2 bytes a pixel.
    for k in 0..=ENTRANCE_FRAMES {
        let r = reveal_on(k, 8);
        assert_eq!(r.start(0x7A_0000, W * 2, 2) % 16, 0, "{r:?}");
        assert!(r.x + r.w <= W && r.row + r.h <= PIC_H);
    }
    assert_eq!(reveal_on(ENTRANCE_FRAMES, 8), page_win(0));
    for p in 0..=W {
        let s = strip1_on(p, 8);
        assert_eq!(s.start(0, STRIP_W * 2, 2) % 16, 0);
        assert_eq!(s.col, s.x + p);
        assert!(s.x <= 7 && s.w >= W - 8 && s.w.is_multiple_of(2) && s.col + s.w <= STRIP_W);
    }
}

#[test]
fn the_fling_runs_into_both_margins_and_home() {
    let s: Vec<i32> = (0..FLING_FRAMES).map(fling).collect();
    assert_eq!(
        (s[0], s[FLING_FRAMES - 1]),
        (0, ease_in_out(-110, 0, 49, 50))
    );
    assert!(*s.iter().max().unwrap() >= SCROLL_MAX + 100);
    assert!(*s.iter().min().unwrap() <= -100);
    assert!(
        s.iter()
            .all(|&v| v >= -(MARGIN as i32) && v <= SCROLL_MAX + MARGIN as i32)
    );
    // Each frame moves at most a little more than the fling's start
    // speed: no jump from one piece to the next.
    assert!(
        s.windows(2)
            .all(|p| (p[1] - p[0]).abs() <= 2 * 740 / 100 + 1)
    );
    let mut distinct = s.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert!(distinct.len() > 150, "{}", distinct.len());
}

#[test]
fn the_entrances_end_on_the_page_and_start_on_the_card() {
    assert_eq!(reveal(ENTRANCE_FRAMES), page_win(0));
    assert_eq!(cascade(ENTRANCE_FRAMES), page_win(0));
    let r = reveal(0);
    assert_eq!((r.x, r.y, r.w, r.h), (16, 210, 448, 140));
    assert_eq!(cascade(0).h, 2);
    // Each grows step by step.
    for k in 1..=ENTRANCE_FRAMES {
        assert!(reveal(k).h >= reveal(k - 1).h && reveal(k).w >= reveal(k - 1).w);
        assert!(cascade(k).h >= cascade(k - 1).h);
    }
}

#[test]
fn a_strip_step_of_one_pixel_moves_the_picture_one_pixel() {
    // Screen column X shows strip column X + p, through the start and the
    // window's place.
    for p in 0..=W {
        let s = strip1(p);
        for xs in [s.x, s.x + 1, s.x + s.w - 1] {
            assert_eq!(s.col + (xs - s.x), xs + p, "p {p} column {xs}");
        }
        // What the window leaves at either side is clear in the page.
        assert!(s.x <= 3 && W - (s.x + s.w) <= 4);
    }
    for x in (0..8).chain(472..480) {
        assert_eq!(strip(x, 10) >> 24, 0);
        assert_eq!(strip(W + x, 10) >> 24, 0);
    }
}

#[test]
fn over_keeps_the_ends_and_widen_is_exact_on_565() {
    assert_eq!(over(0x00_FF_00_00, 0xFF_12_34_56), 0xFF_12_34_56);
    assert_eq!(over(0xFF_AB_CD_EF, 0xFF_12_34_56), 0xFF_AB_CD_EF);
    for p in [0u16, 0xFFFF, 0xF800, 0x07E0, 0x001F, 0x1234] {
        assert_eq!(to565(widen(p)), p);
    }
    assert_ne!(page(100, MARGIN + 20, false) >> 24, 0);
    assert_eq!(page(100, MARGIN - 1, false), 0);
}

/// 4 MiB of local memory: screen 0 at 0, the page's picture at 1 MiB in
/// `fmt`.
fn memory(fmt: VideoFormat) -> Vec<u8> {
    let mut m = vec![0u8; 4 << 20];
    for y in 0..H {
        for x in 0..W {
            let o = ((y * W + x) * 2) as usize;
            m[o..o + 2].copy_from_slice(&screen0(x, y, "WG", "TEST").to_le_bytes());
        }
    }
    for y in 0..PIC_H {
        for x in 0..W {
            let c = page(x, y, false);
            match fmt {
                VideoFormat::Argb8888 => {
                    let o = (PAGE_AT + (y * W + x) * 4) as usize;
                    m[o..o + 4].copy_from_slice(&c.to_le_bytes());
                }
                _ => {
                    let o = (PAGE_AT + (y * W + x) * 2) as usize;
                    let d = rp_gfx::layer::to_4444_dithered(c, x as usize, y as usize);
                    m[o..o + 2].copy_from_slice(&d.to_le_bytes());
                }
            }
        }
    }
    m
}

const PAGE_AT: u32 = 1 << 20;

/// The registers for screen 0 alone, or with the page layer in `win` as
/// `fmt` and `blend`.
fn regs(layer: Option<(Win, VideoFormat, Blend)>) -> Vec<u32> {
    let mut r = vec![0u32; 0x4000 / 4];
    let screen = Plane {
        format: PixelFormat::Rgb565,
        width: W as u16,
        height: H as u16,
        offset: 0,
        stride: (W * 2) as usize,
    };
    for (o, v) in screen.registers() {
        r[o / 4] = v;
    }
    r[reg::GSCADR / 4] = Layer::SOLO.scadr(r[reg::GSCADR / 4]);
    r[reg::DSCTRL / 4] = 1 << 31;
    if let Some((win, fmt, blend)) = layer {
        let bpp = if fmt == VideoFormat::Argb8888 { 4 } else { 2 };
        let mut v = VideoPlane::new(fmt, win.w as u16, win.h as u16, 0);
        v.offset = win.start(PAGE_AT, W * bpp, bpp) as usize;
        v.stride = (W * bpp) as usize;
        let scale = Scale {
            x: win.x as u16,
            y: win.y as u16,
            ..Scale::NONE
        };
        let l = Layer {
            slot: Slot::Middle,
            blend,
        };
        for (o, val) in v.registers_scaled(l, false, scale) {
            r[o / 4] = val;
        }
        r[rp_drv_mbx_display::layers::reg::VOVRCLK / 4] = 1;
    }
    r
}

fn shot(m: &[u8], r: &[u32]) -> Vec<u8> {
    let mut out = vec![0u8; (W * H * 3) as usize];
    scanout::compose(|o| r[o / 4], m, &mut out);
    out
}

#[test]
fn the_l2_layouts_through_the_scan_out_model() {
    for fmt in [VideoFormat::Argb8888, VideoFormat::Argb4444] {
        let m = memory(fmt);
        let alone = shot(&m, &regs(None));
        // The window over the top margin's clear lines: screen 0 alone
        // (M1's per-pixel alpha check).
        let clear = Win {
            h: MARGIN,
            ..page_win(-(MARGIN as i32))
        };
        assert_eq!(
            shot(&m, &regs(Some((clear, fmt, Blend::PixelAlpha)))),
            alone,
            "{fmt:?}"
        );
        // The page differs, and differs again if its alpha is ignored.
        let page_shot = shot(&m, &regs(Some((page_win(0), fmt, Blend::PixelAlpha))));
        let opaque = shot(&m, &regs(Some((page_win(0), fmt, Blend::Opaque))));
        assert_ne!(page_shot, alone);
        assert_ne!(page_shot, opaque);
        // Header and softkeys are never covered.
        let n = (WIN_Y * W * 3) as usize;
        assert_eq!(page_shot[..n], alone[..n]);
        assert_eq!(
            page_shot[(SOFTKEYS_Y * W * 3) as usize..],
            alone[(SOFTKEYS_Y * W * 3) as usize..]
        );
        // A pixel between two rows' cards shows the wallpaper.
        let (x, y) = (200, WIN_Y + ROW_PITCH);
        assert_eq!(page(x, MARGIN + ROW_PITCH, false), 0);
        let o = ((y * W + x) * 3) as usize;
        assert_eq!(page_shot[o..o + 3], alone[o..o + 3]);
    }
}
