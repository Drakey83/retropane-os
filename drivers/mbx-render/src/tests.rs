//! `Mbx2dRenderer` on the engine model against the reference: the CPU
//! renderer's ARGB8888 layer mixed over the same wallpaper as the display
//! mixes planes (`scanout::mix`). The composition (at 32 bits) is equal
//! within 2 per channel: the CPU rounds a ÷255, the engine weighs alpha
//! out of 256 (measured); the screen is that truncated to RGB565, so each
//! channel lies between the reference's ±2 truncated.

extern crate std;

use std::sync::atomic::{AtomicU32, Ordering};
use std::vec::Vec;

use rp_draw::{Cmd, Font, Frame, Glyph, Image, PanelArt, Rect, Renderer};
use rp_drv_mbx_display::scanout::mix;
use rp_gfx::layer::{Layer, PlaneFormat};
use rp_gfx::render::CpuRenderer;

use super::model::{self, Bus, Mem};
use super::*;

static NOW: AtomicU32 = AtomicU32::new(0);
fn now() -> u32 {
    NOW.fetch_add(1, Ordering::Relaxed)
}

/// The wallpaper: a pattern with every channel moving, already quantised
/// to RGB565 (as `rp-today` hands it over), so the 16-bit wallpaper holds
/// it exactly.
fn wall(x: u16, y: u16) -> u32 {
    let (x, y) = (u32::from(x), u32::from(y));
    let raw = 0xFF00_0000 | ((x * 255 / 479) << 16) | ((y * 255 / 639) << 8) | ((x + y) & 0xFF);
    model::widen(to565(raw))
}

/// The renderer on the model, the wallpaper stored, shown.
fn gpu() -> (Mbx2dRenderer<Bus, Mem>, model::Shared) {
    gpu_ticking(1)
}

/// [`gpu`] with its clock at `ticks_per_ms` (each look at the clock one
/// tick).
fn gpu_ticking(ticks_per_ms: u32) -> (Mbx2dRenderer<Bus, Mem>, model::Shared) {
    let mem = model::memory();
    let mut r = Mbx2dRenderer::new(
        Bus::new(mem.clone()),
        Mem(mem.clone()),
        Layout::X51V,
        now,
        ticks_per_ms,
    );
    let mut row = [0u32; W as usize];
    for y in 0..H {
        for (x, p) in row.iter_mut().enumerate() {
            *p = wall(x as u16, y);
        }
        r.wallpaper_row(y, &row);
    }
    r.show(|_| {});
    r.dither = false;
    (r, mem)
}

/// The picture the plane shows (RGB565, widened as the engine reads it).
fn shown(r: &mut Mbx2dRenderer<Bus, Mem>, mem: &model::Shared) -> Vec<u32> {
    let scadr = r.parts().0.regs.reg(rp_drv_mbx_display::reg::GSCADR);
    let base = ((scadr & 0x7F_FFFF) * 16) as usize;
    let m = mem.borrow();
    m[base..base + SCREEN_BYTES]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| model::widen(u16::from_le_bytes(*b)) & 0x00FF_FFFF)
        .collect()
}

/// The work picture (32 bpp), as the last rectangle composed it.
fn work(mem: &model::Shared) -> Vec<u32> {
    let base = Layout::X51V.work;
    let m = mem.borrow();
    m[base..base + PICTURE_BYTES]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| u32::from_le_bytes(*b) & 0x00FF_FFFF)
        .collect()
}

/// The CPU renderer's layer over the wallpaper, as scan-out mixes it.
struct Reference {
    work: Vec<u8>,
    plane: Vec<u8>,
}

impl Reference {
    fn new() -> Self {
        Self {
            work: std::vec![0; PICTURE_BYTES],
            plane: std::vec![0; PICTURE_BYTES],
        }
    }

    /// Draws `f` (the layer persists between calls, as on the device).
    fn draw(&mut self, f: &Frame) -> Vec<u32> {
        let layer = Layer::in_bytes(&mut self.work, u32::from(W), u32::from(H)).expect("layer");
        let mut cpu = CpuRenderer::new(layer, &mut self.plane, STRIDE, PlaneFormat::Argb8888);
        cpu.draw(f);
        self.plane
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let l = u32::from_le_bytes(*b);
                let w = wall((i % 480) as u16, (i / 480) as u16);
                let k = (l >> 24) as u8;
                let ch = |sh: u32| u32::from(mix((l >> sh) as u8, (w >> sh) as u8, k)) << sh;
                ch(16) | ch(8) | ch(0)
            })
            .collect()
    }
}

/// Every pixel of the 16-bit screen `got` (widened) a truncation of the
/// 32-bit reference within `tol` per channel: between `want - tol` and
/// `want + tol`, each truncated to the channel's bits. Reports the first
/// that isn't.
fn same(got: &[u32], want: &[u32], tol: u32) -> Result<(), std::string::String> {
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        if [(0, 5), (8, 6), (16, 5)].iter().any(|&(sh, bits)| {
            let lost = 8 - bits;
            let (g, w) = ((g >> sh) & 0xFF, (w >> sh) & 0xFF);
            let lo = w.saturating_sub(tol) >> lost;
            let hi = (w + tol).min(255) >> lost;
            !(lo..=hi).contains(&(g >> lost))
        }) {
            return Err(std::format!(
                "({}, {}): engine {g:06x}, reference {w:06x}",
                i % 480,
                i / 480
            ));
        }
    }
    Ok(())
}

static ICON_PX: [u32; 12] = [
    0xFFFF_0000,
    0x80FF_0000,
    0x00FF_0000,
    0x40FF_FFFF, //
    0xFF00_FF00,
    0xC000_FF00,
    0x2000_00FF,
    0xFF00_00FF, //
    0x0000_0000,
    0x7F12_3456,
    0xFFFF_FFFF,
    0x0100_0000,
];
static ICON: Image = Image {
    w: 4,
    h: 3,
    px: &ICON_PX,
};

static COVER_A: [u8; 12] = [0, 64, 128, 255, 255, 128, 64, 0, 10, 200, 255, 1];
static GLYPHS: [Glyph; 2] = [
    Glyph {
        w: 4,
        h: 3,
        left: -1,
        top: 3,
        advance: 4,
        cover: &COVER_A,
    },
    Glyph {
        w: 0,
        h: 0,
        left: 0,
        top: 0,
        advance: 3,
        cover: &[],
    },
];
static FONT: Font = Font {
    line: 5,
    ascent: 4,
    first: b'A',
    glyphs: &GLYPHS,
};

/// A glass panel: rows darkening down, a lit rim, corners of their own.
static PANEL_ROWS: [u32; 64] = {
    let mut r = [0u32; 64];
    let mut i = 0;
    while i < 64 {
        r[i] = 0xF0B0_B2B8 - (i as u32) * 0x0001_0101;
        i += 1;
    }
    r
};
static CORNER_PX: [[u32; 16]; 4] = [
    [
        0,
        0x40FF_FFFF,
        0xC0E0_E0E0,
        0xF0D0_D0D0,
        0x40FF_FFFF,
        0xF0F0_F0F0,
        0xF0C0_C0C0,
        0xF0B0_B2B8,
        0xC0E0_E0E0,
        0xF0C0_C0C0,
        0xF0B0_B2B8,
        0xF0B0_B2B8,
        0xF0D0_D0D0,
        0xF0B0_B2B8,
        0xF0B0_B2B8,
        0xF0B0_B2B8,
    ],
    [
        0xF0D0_D0D0,
        0xC0E0_E0E0,
        0x40FF_FFFF,
        0,
        0xF0B0_B2B8,
        0xF0C0_C0C0,
        0xF0F0_F0F0,
        0x40FF_FFFF,
        0xF0B0_B2B8,
        0xF0B0_B2B8,
        0xF0C0_C0C0,
        0xC0E0_E0E0,
        0xF0B0_B2B8,
        0xF0B0_B2B8,
        0xF0B0_B2B8,
        0xF0D0_D0D0,
    ],
    [
        0xF080_8288,
        0xF070_7278,
        0xF070_7278,
        0xF070_7278,
        0xC0A0_A0A0,
        0xF080_8288,
        0xF070_7278,
        0xF070_7278,
        0x40C0_C0C0,
        0xF0A0_A0A0,
        0xF080_8288,
        0xF070_7278,
        0,
        0x40C0_C0C0,
        0xC0A0_A0A0,
        0xF080_8288,
    ],
    [
        0xF070_7278,
        0xF070_7278,
        0xF070_7278,
        0xF080_8288,
        0xF070_7278,
        0xF070_7278,
        0xF080_8288,
        0xC0A0_A0A0,
        0xF070_7278,
        0xF080_8288,
        0xF0A0_A0A0,
        0x40C0_C0C0,
        0xF080_8288,
        0xC0A0_A0A0,
        0x40C0_C0C0,
        0,
    ],
];
static CORNERS: [Image; 4] = [
    Image {
        w: 4,
        h: 4,
        px: &CORNER_PX[0],
    },
    Image {
        w: 4,
        h: 4,
        px: &CORNER_PX[1],
    },
    Image {
        w: 4,
        h: 4,
        px: &CORNER_PX[2],
    },
    Image {
        w: 4,
        h: 4,
        px: &CORNER_PX[3],
    },
];
static PANEL: PanelArt = PanelArt {
    radius: 4,
    rows: &PANEL_ROWS,
    top: [0xF0F8_F8F8, 0xF0D8_D8DC],
    bottom: [0xF090_9298, 0xF0A0_A2A8],
    side: 0xF0C8_C8CC,
    corners: [&CORNERS[0], &CORNERS[1], &CORNERS[2], &CORNERS[3]],
};

/// A scene with every command, glass under text and icons.
fn scene(f: &mut Frame, shift: u16) {
    f.clear();
    f.push(Cmd::Fill {
        r: Rect::new(0, 0, 480, 40),
        color: 0xFF20_3040,
    });
    f.push(Cmd::Fill {
        r: Rect::new(10 + shift, 60, 200, 120),
        color: 0x8040_8080,
    });
    f.push(Cmd::HGradient {
        r: Rect::new(20, 200, 300, 50),
        left: 0x10FF_FFFF,
        right: 0xC000_0040,
    });
    f.push(Cmd::HGradient {
        r: Rect::new(20, 260, 300, 10),
        left: 0xFF00_0000,
        right: 0xFFFF_FFFF,
    });
    f.push(Cmd::Outline {
        r: Rect::new(5 + shift, 55, 210, 130),
        color: 0x60FF_FFFF,
    });
    f.push(Cmd::Image {
        x: 30 + shift,
        y: 80,
        img: &ICON,
    });
    f.text(12 + shift, 100, &FONT, 0xFFFF_FFFF, b"ABAA");
    f.text(40, 230, &FONT, 0x80FF_FF00, b"AAB A");
    // Glass panels: one tall (every row its own), one short (rows
    // joined), one too small for its corners (left out).
    f.push(Cmd::Panel {
        r: Rect::new(330, 60 + shift, 140, 200),
        art: &PANEL,
    });
    f.push(Cmd::Panel {
        r: Rect::new(330, 300, 90, 30),
        art: &PANEL,
    });
    f.push(Cmd::Panel {
        r: Rect::new(430, 300, 8, 8),
        art: &PANEL,
    });
    f.text(340, 100 + shift, &FONT, 0xFF10_1010, b"AB");
}

#[test]
fn every_command_matches_the_reference() {
    let (mut r, mem) = gpu();
    let mut cpu = Reference::new();
    let mut f = Frame::new();
    scene(&mut f, 0);
    f.damage(Rect::new(0, 0, W, H));
    r.draw(&f);
    assert!(r.poll());
    same(&shown(&mut r, &mem), &cpu.draw(&f), 2).unwrap();
    assert_eq!(r.stalls, 0);
}

/// A slow engine (the FIFO full for longer than the 100 ms a command may
/// wait): the frame is dropped, counted apart from stalls, never shown, and
/// the next frame is drawn whole and right, with no reset.
#[test]
fn a_full_fifo_drops_the_frame_without_calling_it_a_hang() {
    let (mut r, mem) = gpu();
    let mut cpu = Reference::new();
    let mut f = Frame::new();
    scene(&mut f, 0);
    f.damage(Rect::new(0, 0, W, H));
    r.draw(&f);
    assert!(r.poll());
    same(&shown(&mut r, &mem), &cpu.draw(&f), 2).unwrap();
    let before = shown(&mut r, &mem);
    // The fake clock moves 1 ms a reading: ~100 readings time a command
    // out; 150 leave the FIFO room again by the frame's fence.
    scene(&mut f, 40);
    f.damage(Rect::new(0, 0, W, H));
    r.parts().0.regs.full_reads = 150;
    r.draw(&f);
    assert_eq!((r.fifo_full, r.last.fifo_full, r.stalls), (1, 1, 0));
    assert!(r.poll(), "the fence landed");
    assert_eq!(shown(&mut r, &mem), before, "the dropped frame isn't shown");
    // The next frame: whole, right.
    r.draw(&f);
    assert!(r.poll());
    assert_eq!((r.fifo_full, r.last.fifo_full, r.stalls), (1, 0, 0));
    same(&shown(&mut r, &mem), &cpu.draw(&f), 2).unwrap();
}

#[test]
fn small_changes_redraw_both_pictures_right() {
    // Buffer age 2: each picture is two frames old when drawn into.
    let (mut r, mem) = gpu();
    let mut cpu = Reference::new();
    let mut f = Frame::new();
    for (k, shift) in [0u16, 0, 7, 3, 3, 40, 0].into_iter().enumerate() {
        scene(&mut f, shift);
        if k == 0 {
            f.damage(Rect::new(0, 0, W, H));
        } else {
            // What moved: the glass panel's area, old and new.
            f.damage(Rect::new(0, 50, 300, 140));
        }
        r.draw(&f);
        assert!(r.poll());
        same(&shown(&mut r, &mem), &cpu.draw(&f), 2).unwrap_or_else(|e| panic!("frame {k}: {e}"));
    }
}

#[test]
fn many_translucent_fills_wrap_the_row_ring() {
    let (mut r, mem) = gpu();
    let mut cpu = Reference::new();
    let mut f = Frame::new();
    for k in 0..100u16 {
        f.push(Cmd::Fill {
            r: Rect::new(k * 4, k * 6, 30, 30),
            color: 0x3000_0000 | (u32::from(k) * 0x0201),
        });
    }
    f.damage(Rect::new(0, 0, W, H));
    r.draw(&f);
    assert!(r.poll());
    same(&shown(&mut r, &mem), &cpu.draw(&f), 2).unwrap();
}

#[test]
fn nothing_changed_draws_nothing() {
    let (mut r, mem) = gpu();
    let mut f = Frame::new();
    scene(&mut f, 0);
    f.damage(Rect::new(0, 0, W, H));
    r.draw(&f);
    assert!(r.poll());
    r.draw(&f);
    assert!(r.poll());
    // No damage: the back picture still lacks the last frame's, once.
    f.clear();
    scene(&mut f, 0);
    assert_ne!(r.draw(&f), Drawn::default());
    assert!(r.poll());
    let before = shown(&mut r, &mem);
    let ops = r.parts().0.regs.ops;
    assert_eq!(r.draw(&f), Drawn::default());
    assert_eq!(r.parts().0.regs.ops, ops);
    assert_eq!(shown(&mut r, &mem), before);
}

#[test]
fn place_clips_like_the_cpu_renderer() {
    let clip = Rect::new(10, 10, 20, 20);
    assert_eq!(
        place(4, 4, 8, 8, clip),
        Some((Rect::new(10, 10, 2, 2), 2, 2))
    );
    assert_eq!(
        place(4, 4, -2, 12, Rect::new(0, 0, 50, 50)),
        Some((Rect::new(0, 12, 2, 4), 2, 0))
    );
    assert_eq!(place(4, 4, 40, 40, clip), None);
}

/// The 3D core's rule (`rp-mbx3d-render`'s model of run P2): channel `v`
/// to `bits`, one step up where the fraction lost reaches `t` sixteenths
/// of a step, never past the top.
fn core_dither(v: u32, bits: u32, t: u32) -> u32 {
    let lost = 8 - bits;
    let (q, frac) = (v >> lost, v & ((1 << lost) - 1));
    if frac * 16 >= t << lost && q < (1 << bits) - 1 {
        q + 1
    } else {
        q
    }
}

#[test]
fn the_pattern_then_truncation_is_the_3d_cores_dither() {
    // Every level at every place, each channel: adding `dither_at`
    // (saturating) then truncating gives what the 3D core writes.
    for (y, row) in DITHER_3D.iter().enumerate() {
        for (x, &t) in row.iter().enumerate() {
            let a = dither_at(x, y);
            for (sh, bits) in [(16, 5), (8, 6), (0, 5)] {
                let add = (a >> sh) & 0xFF;
                assert!(add < 1 << (8 - bits), "below one step");
                for v in 0..256 {
                    assert_eq!(
                        (v + add).min(255) >> (8 - bits),
                        core_dither(v, bits, t),
                        "v {v}, place ({x}, {y}), {bits} bits"
                    );
                }
            }
        }
    }
    // The pattern repeats every 4 pixels both ways.
    assert_eq!(dither_at(5, 6), dither_at(1, 2));
}

#[test]
fn the_screen_is_the_work_picture_dithered_as_the_3d_core_would() {
    let (mut r, mem) = gpu();
    let mut f = Frame::new();
    scene(&mut f, 0);
    f.damage(Rect::new(0, 0, W, H));
    r.draw(&f);
    assert!(r.poll());
    // Whole-screen frames: the work picture holds the whole composition.
    let composed = work(&mem);
    r.dither = true;
    r.draw(&f);
    assert!(r.poll());
    let screen = shown(&mut r, &mem);
    let mut raised = 0;
    for (i, (c, s)) in composed.iter().zip(&screen).enumerate() {
        let (x, y) = (i % 480, i / 480);
        for (sh, bits) in [(16, 5), (8, 6), (0, 5)] {
            let want = core_dither((c >> sh) & 0xFF, bits, DITHER_3D[y % 4][x % 4]);
            let got = ((s >> sh) & 0xFF) >> (8 - bits);
            assert_eq!(got, want, "pixel ({x}, {y}), {bits} bits at {sh}");
            raised += u32::from(got != ((c >> sh) & 0xFF) >> (8 - bits));
        }
    }
    assert!(raised > 10_000, "the dither changed something: {raised}");
}

#[test]
fn glass_goes_in_the_background_once() {
    let (mut r, _mem) = gpu();
    let mut f = Frame::new();
    scene(&mut f, 0);
    f.damage(Rect::new(0, 0, W, H));
    r.draw(&f);
    assert!(r.poll());
    assert!(r.last.background, "first frame builds it");
    // Same glass, new text: no rebuild.
    scene(&mut f, 0);
    f.text(300, 400, &FONT, 0xFFFF_FFFF, b"AB");
    f.damage(Rect::new(290, 390, 40, 20));
    r.draw(&f);
    assert!(r.poll());
    assert!(!r.last.background);
    // The glass moves: rebuilt.
    scene(&mut f, 9);
    f.damage(Rect::new(0, 50, 300, 140));
    r.draw(&f);
    assert!(r.poll());
    assert!(r.last.background);
}

#[test]
fn a_new_wallpaper_redraws_everything_and_no_cached_area_shows_the_old_one() {
    let (mut r, mem) = gpu();
    let mut f = Frame::new();
    // The same frame, again and again: its areas are kept, then reused.
    let mut hits = 0;
    for _ in 0..4 {
        scene(&mut f, 0);
        f.damage(Rect::new(0, 0, W, H));
        r.draw(&f);
        assert!(r.poll());
        hits += r.last.cache_hits;
    }
    assert!(hits > 0, "the cache was used before the change");
    let before = shown(&mut r, &mem);
    // Where the wallpaper shows through untouched.
    let bare: std::vec::Vec<usize> = (0..before.len())
        .filter(|&i| before[i] == wall((i % 480) as u16, (i / 480) as u16) & 0xFF_FFFF)
        .collect();
    assert!(bare.len() > 10_000);
    // A new wallpaper: one colour.
    let row = [0xFF10_2031u32; W as usize];
    for y in 0..H {
        r.wallpaper_row(y, &row);
    }
    r.wallpaper_changed();
    for k in 0..2 {
        scene(&mut f, 0);
        f.damage(Rect::new(0, 0, W, H));
        r.draw(&f);
        assert!(r.poll());
        if k == 0 {
            assert!(r.last.background, "the background is made again");
        }
        let now = shown(&mut r, &mem);
        let stale = bare.iter().filter(|&&i| now[i] != 0x10_2031).count();
        assert_eq!(
            stale, 0,
            "frame {k}: pixels still showing the old wallpaper"
        );
    }
}

/// A frame filling `r` with `colour`, damaged.
fn fill_frame(r: Rect, colour: u32) -> Frame {
    let mut f = Frame::new();
    f.damage(r);
    f.push(Cmd::Fill { r, color: colour });
    f
}

#[test]
fn every_flip_is_written_in_the_blanking() {
    // R113 on the X51v: a GSCADR write shows at once, so one written
    // mid-refresh tears it. The scan moves 7 lines a look: the flip waits
    // for lines 640-644 (each look at the clock 1 us).
    let (mut r, _) = gpu_ticking(1000);
    r.parts().0.regs.scan_step = 7;
    r.parts().0.regs.flip_lines.clear();
    for (i, c) in [0xFFFF_0000u32, 0xFF00_FF00, 0xFF00_00FF]
        .into_iter()
        .enumerate()
    {
        r.draw(&fill_frame(Rect::new(0, 600, 480, 40), c));
        while !r.poll() {}
        r.flip();
        assert!(i < 3);
    }
    let lines = r.parts().0.regs.flip_lines.clone();
    assert!(lines.len() >= 3, "{lines:?}");
    for l in &lines {
        // The panel's 640 shown lines end at 639 (`X51V_60HZ`): not the
        // constant under test.
        assert!(
            (640..=644).contains(l),
            "a flip written at line {l}: {lines:?}"
        );
    }
    assert_eq!(r.late_flips, 0);
}

#[test]
fn a_scan_that_never_reaches_the_blanking_flips_late_and_counts_it() {
    // The scan stuck at line 0 (a stopped display): each flip waits a
    // refresh at most, then is written anyway, counted.
    let (mut r, _) = gpu();
    r.parts().0.regs.scan_step = 645;
    r.parts().0.regs.flip_lines.clear();
    r.draw(&fill_frame(Rect::new(0, 600, 480, 40), 0xFFFF_0000));
    while !r.poll() {}
    r.flip();
    assert!(r.late_flips >= 1);
    assert!(!r.parts().0.regs.flip_lines.is_empty(), "written anyway");
}

/// Glass sliding sideways (the Weather page's entrance, a dragged list):
/// the background is made again only where the moved panel was and is,
/// the frame after isn't drawn whole, and every frame matches the
/// reference (2026-10-30: each such frame rebuilt the whole screen's
/// background and forced two whole frames, 53-58 ms of CPU on the X51v).
#[test]
fn moving_glass_rebuilds_only_its_old_and_new_places() {
    let (mut r, mem) = gpu();
    let mut cpu = Reference::new();
    let mut f = Frame::new();
    scene(&mut f, 0);
    f.damage(Rect::new(0, 0, W, H));
    r.draw(&f);
    assert!(r.poll());
    assert_eq!(r.last.background_px, u32::from(W) * u32::from(H));
    same(&shown(&mut r, &mem), &cpu.draw(&f), 2).unwrap();
    for (k, shift) in [6u16, 12, 18, 18, 30, 0].into_iter().enumerate() {
        scene(&mut f, shift);
        f.damage(Rect::new(0, 50, 300, 140));
        r.draw(&f);
        assert!(r.poll());
        let moved = k != 3;
        assert_eq!(r.last.background, moved, "frame {k}");
        if moved {
            // The fill and the outline, old and new: well under a screen.
            assert!(
                r.last.background_px > 0 && r.last.background_px < 80_000,
                "frame {k}: {} px rebuilt",
                r.last.background_px
            );
        }
        assert!(r.last.rects <= 2, "frame {k}: drawn whole");
        same(&shown(&mut r, &mem), &cpu.draw(&f), 2).unwrap_or_else(|e| panic!("frame {k}: {e}"));
    }
}

/// Two hoisted panels swapping order where they overlap: the overlap is
/// made again (each one's key carries the keys it lies over).
#[test]
fn glass_changing_order_is_rebuilt_where_it_overlaps() {
    let (mut r, mem) = gpu();
    let mut cpu = Reference::new();
    let a = Cmd::Fill {
        r: Rect::new(50, 50, 100, 100),
        color: 0x80FF_0000,
    };
    let b = Cmd::Fill {
        r: Rect::new(100, 100, 100, 100),
        color: 0x8000_00FF,
    };
    for (k, order) in [[a, b], [b, a]].into_iter().enumerate() {
        let mut f = Frame::new();
        for c in order {
            f.push(c);
        }
        f.damage(Rect::new(0, 0, W, H));
        r.draw(&f);
        assert!(r.poll());
        assert!(r.last.background, "frame {k}");
        same(&shown(&mut r, &mem), &cpu.draw(&f), 2).unwrap_or_else(|e| panic!("frame {k}: {e}"));
    }
}

/// The same gradient on many rows and frames is written into the ring
/// once; a different one never reuses it.
#[test]
fn a_gradient_row_is_written_once_and_reused_only_for_the_same_colours() {
    let (mut r, mem) = gpu();
    let mut cpu = Reference::new();
    let rows = |f: &mut Frame, right: u32, dy: u16| {
        f.clear();
        // Text first, so the glass after it isn't hoisted: drawn each frame.
        f.text(5, 8, &FONT, 0xFFFF_FFFF, b"A");
        f.push(Cmd::Fill {
            r: Rect::new(0, 0, 40, 640),
            color: 0x40FF_FFFF,
        });
        for k in 0..10u16 {
            f.push(Cmd::HGradient {
                r: Rect::new(16, 20 + dy + k * 45, 448, 42),
                left: 0xB810_1418,
                right,
            });
        }
    };
    let mut f = Frame::new();
    rows(&mut f, 0x8C4C_545C, 0);
    f.damage(Rect::new(0, 0, W, H));
    r.draw(&f);
    assert!(r.poll());
    assert_eq!(r.last.rows_written, 2, "one fill row, one gradient row");
    same(&shown(&mut r, &mem), &cpu.draw(&f), 2).unwrap();
    rows(&mut f, 0x8C4C_545C, 3);
    f.damage(Rect::new(0, 0, W, H));
    r.draw(&f);
    assert!(r.poll());
    assert_eq!(r.last.rows_written, 0, "the next frame reuses them");
    same(&shown(&mut r, &mem), &cpu.draw(&f), 2).unwrap();
    rows(&mut f, 0x8C4C_5460, 3);
    f.damage(Rect::new(0, 0, W, H));
    r.draw(&f);
    assert!(r.poll());
    assert_eq!(r.last.rows_written, 1, "another colour: a row of its own");
    same(&shown(&mut r, &mem), &cpu.draw(&f), 2).unwrap();
}

/// More distinct rows than the ring holds in one frame: rows still read
/// by the engine are never overwritten (the frame waits first).
#[test]
fn a_full_ring_waits_before_overwriting_a_row_in_use() {
    let (mut r, mem) = gpu();
    let mut cpu = Reference::new();
    let mut f = Frame::new();
    f.text(5, 8, &FONT, 0xFFFF_FFFF, b"A");
    for k in 0..150u16 {
        f.push(Cmd::Fill {
            r: Rect::new(k * 3, k * 4, 30, 8),
            color: 0x2000_0000 | (u32::from(k) * 0x0103),
        });
    }
    f.damage(Rect::new(0, 0, W, H));
    r.draw(&f);
    assert!(r.poll());
    assert!(r.last.waits > 0);
    same(&shown(&mut r, &mem), &cpu.draw(&f), 2).unwrap();
}

/// A clip limits everything after it, glass, pictures and text, and the
/// background honours it too.
#[test]
fn a_clip_limits_the_commands_after_it() {
    let (mut r, mem) = gpu();
    let mut cpu = Reference::new();
    for dy in [0u16, 17, 33] {
        let mut f = Frame::new();
        f.clip(Some(Rect::new(0, 60, W, 150)));
        for k in 0..6u16 {
            let y = 40 + dy + k * 40;
            f.push(Cmd::HGradient {
                r: Rect::new(16, y, 448, 36),
                left: 0xB810_1418,
                right: 0x8C4C_545C,
            });
            f.push(Cmd::Image {
                x: 30,
                y: y + 2,
                img: &ICON,
            });
            f.text(60, y + 6, &FONT, 0xFFFF_FFFF, b"ABBA");
        }
        f.clip(None);
        f.push(Cmd::Fill {
            r: Rect::new(0, 300, W, 20),
            color: 0x8000_0000,
        });
        f.damage(Rect::new(0, 0, W, H));
        r.draw(&f);
        assert!(r.poll());
        let got = shown(&mut r, &mem);
        same(&got, &cpu.draw(&f), 2).unwrap_or_else(|e| panic!("dy {dy}: {e}"));
        // Nothing of the rows outside the clip: the wallpaper there.
        for y in (40..60).chain(210..300) {
            for x in [20usize, 200, 400] {
                assert_eq!(
                    got[y * 480 + x],
                    wall(x as u16, y as u16) & 0xFF_FFFF,
                    "dy {dy}: ({x}, {y})"
                );
            }
        }
    }
}

/// A store picture draws exactly as the same pixels as a static picture,
/// and a slot freed and reused for another picture is uploaded again (its
/// copy is kept by slot and era), never shown stale.
#[test]
fn a_store_picture_draws_as_an_image_and_a_reused_slot_is_uploaded_again() {
    use rp_draw::pics::Pics;
    let pics: &'static Pics<'static> = std::boxed::Box::leak(std::boxed::Box::new(Pics::new(
        std::boxed::Box::leak(std::vec![0; 16].into_boxed_slice()),
        &[(4, 4, 1)],
    )));
    let pic = pics.alloc(ICON.w, ICON.h).unwrap();
    assert!(pics.write(pic, |px| px.copy_from_slice(&ICON_PX)));
    let draw = |r: &mut Mbx2dRenderer<Bus, Mem>, mem: &model::Shared, c: Cmd| {
        let mut f = Frame::new();
        f.push(c);
        f.damage(Rect::new(0, 0, W, H));
        r.draw(&f);
        assert!(r.poll());
        shown(r, mem)
    };
    let (mut a, mem_a) = gpu();
    let want = draw(
        &mut a,
        &mem_a,
        Cmd::Image {
            x: 30,
            y: 80,
            img: &ICON,
        },
    );
    let (mut b, mem_b) = gpu();
    b.set_pics(pics);
    let got = draw(&mut b, &mem_b, Cmd::Pic { x: 30, y: 80, pic });
    assert_eq!(got, want);
    assert_eq!(b.last.images, 1);
    // The same handle again: no upload.
    draw(&mut b, &mem_b, Cmd::Pic { x: 30, y: 80, pic });
    assert_eq!(b.last.images, 0);
    // Freed, the slot reused for other pixels: uploaded again and shown.
    pics.free(pic);
    let other = pics.alloc(ICON.w, ICON.h).unwrap();
    assert!(pics.write(other, |px| px.fill(0xFF00_00FF)));
    let again = draw(
        &mut b,
        &mem_b,
        Cmd::Pic {
            x: 30,
            y: 80,
            pic: other,
        },
    );
    assert_eq!(b.last.images, 1);
    let blue = again[80 * usize::from(W) + 30];
    assert_eq!(blue & 0x00FF_FFFF, 0x0000_00FF, "{blue:08X}");
    // The old handle draws nothing at all.
    let (mut c, mem_c) = gpu();
    c.set_pics(pics);
    let none = draw(&mut c, &mem_c, Cmd::Pic { x: 30, y: 80, pic });
    // ... and says so (R181).
    assert_eq!(c.last.pics_missed.n, 1);
    assert_eq!(
        c.last.pics_missed.last,
        Some((pic, rp_draw::pics::Miss::Stale))
    );
    assert_eq!(b.last.pics_missed.n, 0, "a picture drawn isn't missed");
    let (mut d, mem_d) = gpu();
    let bare = draw(
        &mut d,
        &mem_d,
        Cmd::Fill {
            r: Rect::new(0, 0, 0, 0),
            color: 0,
        },
    );
    assert_eq!(none, bare);
}

/// Band `k`'s pixel at (x, y) in a test's band pictures: unlike the
/// wallpaper anywhere, already RGB565.
fn band_px(k: usize, x: usize, y: usize) -> u32 {
    let raw =
        0xFF00_0000 | (((k * 70 + x) & 0xFF) << 16) as u32 | (((y * 5) & 0xFF) << 8) as u32 | 0x40;
    model::widen(to565(raw))
}

/// Writes band pictures (480 x `h`, RGB565) at `at[k]`.
fn write_bands(r: &mut Mbx2dRenderer<Bus, Mem>, at: &[usize], h: usize) {
    for (k, &a) in at.iter().enumerate() {
        for y in 0..h {
            let words: Vec<u32> = (0..240)
                .map(|i| {
                    u32::from(to565(band_px(k, 2 * i, y)))
                        | u32::from(to565(band_px(k, 2 * i + 1, y))) << 16
                })
                .collect();
            r.parts().1.store(a + y * STRIDE16, &words);
        }
    }
}

/// R184 step 2: a radar frame's bands switched in draws exactly the
/// screen a rewritten wallpaper would, but makes the background and the
/// screen again only in the changed box (not whole, not two whole frames);
/// switching back shows the wallpaper again; a cached area over the box is
/// never shown stale.
#[test]
fn a_band_switch_redraws_only_its_box_and_matches_a_rewritten_wallpaper() {
    let (top, h) = (100u16, 40u16);
    let pool = rp_vram_map::POOL.at as usize;
    let at = [pool, pool + usize::from(h) * STRIDE16];
    let mut f = Frame::new();
    let (mut r, mem) = gpu();
    // The reference: the same bands written into its wallpaper, changed
    // whole.
    let (mut want, wmem) = gpu();
    for y in top + h..top + 3 * h {
        let k = usize::from((y - top) / h) - 1;
        let row: Vec<u32> = (0..480)
            .map(|x| band_px(k, x, usize::from((y - top) % h)))
            .collect();
        want.wallpaper_row(y, &row);
    }
    want.wallpaper_changed();
    for _ in 0..4 {
        scene(&mut f, 0);
        f.damage(Rect::new(0, 0, W, H));
        r.draw(&f);
        assert!(r.poll());
    }
    let before = shown(&mut r, &mem);
    write_bands(&mut r, &at, usize::from(h));
    let changed = Rect::new(0, top + h, W, 2 * h);
    r.wallpaper_bands(top, h, &[None, Some(at[0]), Some(at[1])], &[changed]);
    for k in 0..3 {
        scene(&mut f, 0);
        if k == 0 {
            f.damage(Rect::new(0, 0, W, H));
        }
        want.draw(&f);
        assert!(want.poll());
        scene(&mut f, 0);
        r.draw(&f);
        assert!(r.poll());
        if k == 0 {
            assert!(r.last.background, "the box's background made again");
            assert_eq!(r.last.background_px, u32::from(W) * u32::from(2 * h));
            assert!(r.last.rects <= 1, "the box only, not the screen");
        }
        assert_eq!(
            shown(&mut r, &mem),
            shown(&mut want, &wmem),
            "frame {k} after the switch"
        );
    }
    // Back to the wallpaper alone.
    r.wallpaper_bands(0, 1, &[], &[changed]);
    for k in 0..2 {
        scene(&mut f, 0);
        r.draw(&f);
        assert!(r.poll());
        assert_eq!(
            shown(&mut r, &mem),
            before,
            "frame {k} after switching back"
        );
    }
}

/// The bands' rows are read from their pictures and every other row from
/// the wallpaper, in as few copies as the sources allow.
#[test]
fn bands_read_their_rows_and_the_wallpaper_the_rest() {
    let (top, h) = (70u16, 33u16);
    let pool = rp_vram_map::POOL.at as usize;
    let (mut r, mem) = gpu();
    write_bands(&mut r, &[pool], usize::from(h));
    let mut f = Frame::new();
    r.wallpaper_bands(top, h, &[None, None, Some(pool)], &[Rect::new(0, 0, W, H)]);
    f.damage(Rect::new(0, 0, W, H));
    r.draw(&f);
    assert!(r.poll());
    let got = shown(&mut r, &mem);
    for y in [0u16, 69, 70, 135, 136, 168, 169, 639] {
        for x in [0usize, 239, 479] {
            let want = if (136..169).contains(&y) {
                band_px(0, x, usize::from(y - 136))
            } else {
                wall(x as u16, y)
            };
            assert_eq!(
                got[usize::from(y) * 480 + x],
                want & 0xFF_FFFF,
                "({x}, {y})"
            );
        }
    }
}

/// A band switch's areas are redrawn each on its own (rain at the left of
/// one band and the right of another: two areas, not their bounding box),
/// and the screen still equals a rewritten wallpaper's.
#[test]
fn a_band_switch_redraws_each_changed_area_not_their_bounding_box() {
    let (top, h) = (100u16, 40u16);
    let pool = rp_vram_map::POOL.at as usize;
    let at = [pool, pool + usize::from(h) * STRIDE16];
    let (mut r, mem) = gpu();
    let (mut want, wmem) = gpu();
    for y in top..top + 2 * h {
        let k = usize::from((y - top) / h);
        let row: Vec<u32> = (0..480)
            .map(|x| band_px(k, x, usize::from((y - top) % h)))
            .collect();
        want.wallpaper_row(y, &row);
    }
    want.wallpaper_changed();
    let mut f = Frame::new();
    f.damage(Rect::new(0, 0, W, H));
    r.draw(&f);
    assert!(r.poll());
    write_bands(&mut r, &at, usize::from(h));
    // The bands differ everywhere, so the areas name all of them, apart.
    let areas = [Rect::new(0, top, 200, h), Rect::new(200, top + h, 280, h)];
    r.wallpaper_bands(top, h, &[Some(at[0]), Some(at[1])], &[areas[0], areas[1]]);
    // Nothing else changed: the areas are the band switch's alone.
    let f = Frame::new();
    r.draw(&f);
    assert!(r.poll());
    assert_eq!(
        r.last.background_px,
        200 * 40 + 280 * 40,
        "the two areas only"
    );
    let mut w = Frame::new();
    w.damage(Rect::new(0, 0, W, H));
    want.draw(&w);
    assert!(want.poll());
    let (got, exp) = (shown(&mut r, &mem), shown(&mut want, &wmem));
    for a in areas {
        for y in a.y..a.y + a.h {
            for x in a.x..a.x + a.w {
                let i = usize::from(y) * 480 + usize::from(x);
                assert_eq!(got[i], exp[i], "({x}, {y})");
            }
        }
    }
}

/// A font of `n` 32 x 32 glyphs from 'A', its coverage a ramp (leaked: a
/// test's fonts live as long as the renderer).
fn big_font(n: usize, seed: u8) -> &'static Font {
    let glyphs: Vec<Glyph> = (0..n)
        .map(|k| {
            let cover: Vec<u8> = (0..32 * 32)
                .map(|i| {
                    (i as u8)
                        .wrapping_mul(7)
                        .wrapping_add(seed)
                        .wrapping_add(k as u8)
                })
                .collect();
            Glyph {
                w: 32,
                h: 32,
                left: 0,
                top: 32,
                advance: 34,
                cover: std::boxed::Box::leak(cover.into_boxed_slice()),
            }
        })
        .collect();
    std::boxed::Box::leak(std::boxed::Box::new(Font {
        line: 36,
        ascent: 32,
        first: b'A',
        glyphs: std::boxed::Box::leak(glyphs.into_boxed_slice()),
    }))
}

/// Each font in its colour, every one of its glyphs drawn (in lines of
/// 13: the glyph memory takes only the glyphs drawn), the whole screen
/// damaged.
fn all_glyphs_frame(fonts: &[(&'static Font, u32)]) -> Frame {
    let mut f = Frame::new();
    let mut line = 0u16;
    for &(font, colour) in fonts {
        let chars: Vec<u8> = (0..font.glyphs.len())
            .map(|k| font.first + k as u8)
            .collect();
        for part in chars.chunks(13) {
            f.text(8, 40 + 38 * line, font, colour, part);
            line += 1;
        }
    }
    f.damage(Rect::new(0, 0, W, H));
    f
}

/// R172: a look switch that keeps some (font, colour) pairs and adds new
/// ones, with the glyph memory full, uploads only the new pairs: the old
/// look's unused atlases are evicted one at a time, the shared ones stay
/// (flushing the whole memory uploaded every atlas still in use again).
/// The text stays right through the evictions.
#[test]
fn a_look_switch_uploads_only_its_new_atlases() {
    let (mut r, mem) = gpu();
    let mut cpu = Reference::new();
    // Five fonts that fill the glyph memory between them.
    let n = (Layout::X51V.glyphs.1 / 5 / 4096).min(super::GLYPHS);
    let fonts: Vec<&'static Font> = (0..5).map(|k| big_font(n, k as u8 * 31)).collect();
    let (dark, light) = (0xFFE0_E0E0, 0xFF20_2020);
    // The second look draws its new pairs first, as a screen may: a flush
    // then would drop the shared ones in the same frame.
    let look = |second: bool| -> Vec<(&'static Font, u32)> {
        let mut v: Vec<_> = fonts
            .iter()
            .enumerate()
            .map(|(k, &f)| (f, if k >= 3 && second { light } else { dark }))
            .collect();
        if second {
            v.reverse();
        }
        v
    };
    let a = all_glyphs_frame(&look(false));
    r.draw(&a);
    assert!(r.poll());
    assert_eq!(r.last.atlases, 5, "the first look uploads its five");
    same(&shown(&mut r, &mem), &cpu.draw(&a), 2).unwrap();
    // The switch: three pairs kept, two new, no room for them.
    let b = all_glyphs_frame(&look(true));
    r.draw(&b);
    assert!(r.poll());
    assert_eq!(r.last.atlases, 2, "only the two new pairs are uploaded");
    assert_eq!(
        r.last.atlases_evicted, 2,
        "the old look's two unused ones go"
    );
    same(&shown(&mut r, &mem), &cpu.draw(&b), 2).unwrap();
    // And back: the same again the other way.
    r.draw(&a);
    assert!(r.poll());
    assert_eq!(r.last.atlases, 2);
    same(&shown(&mut r, &mem), &cpu.draw(&a), 2).unwrap();
    assert_eq!(r.stalls, 0);
}

/// The owner's ruling of 2026-11-10 (ADR-0023 rule 4 amended): the image
/// cache evicts one picture at a time, the oldest not drawn this frame,
/// where it used to empty whole. Two big pictures fit, a third doesn't:
/// the one not drawn goes, the one drawn every frame stays uploaded.
#[test]
fn a_full_picture_cache_evicts_the_oldest_picture_not_the_whole_cache() {
    let (mut r, mem) = gpu();
    let mut cpu = Reference::new();
    // Three pictures of ~150 KB: two fit the 384 KiB cache, three don't.
    let big = |k: u32| -> &'static Image {
        let (w, h) = (250u16, 150u16);
        let px: &'static [u32] = Box::leak(
            (0..u32::from(w) * u32::from(h))
                .map(|i| 0xFF00_0000 | (i % 251) << 16 | k * 60 << 8 | i % 7)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        Box::leak(Box::new(Image { w, h, px }))
    };
    assert!(2 * 250 * 150 * 4 <= Layout::X51V.images.1);
    assert!(3 * 250 * 150 * 4 > Layout::X51V.images.1);
    let (a, b, c) = (big(1), big(2), big(3));
    let frame = |imgs: &[&'static Image]| {
        let mut f = Frame::new();
        f.clear();
        f.push(Cmd::Fill {
            r: Rect::new(0, 0, 480, 640),
            color: 0xFF10_2030,
        });
        for (k, &img) in imgs.iter().enumerate() {
            f.push(Cmd::Image {
                x: 20 + k as u16 * 200,
                y: 20 + k as u16 * 160,
                img,
            });
        }
        f.damage(Rect::new(0, 0, W, H));
        f
    };
    for (imgs, uploaded, evicted) in [
        (vec![a], 1, 0),
        (vec![b], 1, 0),
        // A drawn again, C new: B (not drawn this frame) goes, A stays.
        (vec![a, c], 1, 1),
        // Both kept: nothing uploaded.
        (vec![a, c], 0, 0),
    ] {
        let f = frame(&imgs);
        r.draw(&f);
        assert!(r.poll());
        assert_eq!(
            (r.last.images, r.last.images_evicted),
            (uploaded, evicted),
            "pictures uploaded, evicted"
        );
        same(&shown(&mut r, &mem), &cpu.draw(&f), 2).unwrap();
    }
}
