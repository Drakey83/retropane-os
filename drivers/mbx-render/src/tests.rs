//! `Mbx2dRenderer` on the engine model against the reference: the CPU
//! renderer's ARGB8888 layer mixed over the same wallpaper as the display
//! mixes planes (`scanout::mix`). The composition (at 32 bits) is equal
//! within 2 per channel: the CPU rounds a ÷255, the engine weighs alpha
//! out of 256 (measured); the screen is that truncated to RGB565, so each
//! channel lies between the reference's ±2 truncated.

extern crate std;

use std::sync::atomic::{AtomicU32, Ordering};
use std::vec::Vec;

use rp_draw::{Cmd, Font, Frame, Glyph, Image, Rect, Renderer};
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
