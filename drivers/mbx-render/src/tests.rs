//! `Mbx2dRenderer` on the engine model against the reference: the CPU
//! renderer's ARGB8888 layer mixed over the same wallpaper as the display
//! mixes planes (`scanout::mix`). Equal within 2 per channel: the CPU
//! rounds a ÷255, the engine weighs alpha out of 256 (measured).

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

/// The wallpaper: a pattern with every channel moving.
fn wall(x: u16, y: u16) -> u32 {
    let (x, y) = (u32::from(x), u32::from(y));
    0xFF00_0000 | ((x * 255 / 479) << 16) | ((y * 255 / 639) << 8) | ((x + y) & 0xFF)
}

/// The renderer on the model, the wallpaper stored, shown.
fn gpu() -> (Mbx2dRenderer<Bus, Mem>, model::Shared) {
    let mem = model::memory();
    let mut r = Mbx2dRenderer::new(
        Bus::new(mem.clone()),
        Mem(mem.clone()),
        Layout::X51V,
        now,
        1,
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

/// The picture the plane shows.
fn shown(r: &mut Mbx2dRenderer<Bus, Mem>, mem: &model::Shared) -> Vec<u32> {
    let scadr = r.parts().0.regs.reg(rp_drv_mbx_display::reg::GSCADR);
    let base = ((scadr & 0x7F_FFFF) * 16) as usize;
    let m = mem.borrow();
    m[base..base + SCREEN_BYTES]
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
            work: std::vec![0; SCREEN_BYTES],
            plane: std::vec![0; SCREEN_BYTES],
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

/// Every pixel within `tol` per channel; reports the first that isn't.
fn same(got: &[u32], want: &[u32], tol: u32) -> Result<(), std::string::String> {
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        if [0, 8, 16]
            .iter()
            .any(|&sh| ((g >> sh) & 0xFF).abs_diff((w >> sh) & 0xFF) > tol)
        {
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

#[test]
fn dither_adds_the_pattern_below_one_step() {
    let (mut r, mem) = gpu();
    let mut cpu = Reference::new();
    let mut f = Frame::new();
    scene(&mut f, 0);
    f.damage(Rect::new(0, 0, W, H));
    r.draw(&f);
    assert!(r.poll());
    let plain = shown(&mut r, &mem);
    r.dither = true;
    r.draw(&f);
    assert!(r.poll());
    let dithered = shown(&mut r, &mem);
    let _ = cpu.draw(&f);
    for (i, (p, d)) in plain.iter().zip(&dithered).enumerate() {
        let want = dither_at(i % 480, i / 480);
        for sh in [0, 8, 16] {
            let (p, d, t) = ((p >> sh) & 0xFF, (d >> sh) & 0xFF, (want >> sh) & 0xFF);
            assert_eq!(d, (p + t).min(255), "pixel {i}, shift {sh}");
        }
    }
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
