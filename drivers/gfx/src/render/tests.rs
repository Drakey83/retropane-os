use rp_draw::{Font, Glyph, Image};

use super::*;

const W: u32 = 48;
const H: u32 = 32;

static DOT: Glyph = Glyph {
    w: 2,
    h: 2,
    left: -1,
    top: 2,
    advance: 3,
    cover: &[255, 128, 64, 0],
};
static FONT: Font = Font {
    line: 6,
    ascent: 4,
    first: b'.',
    glyphs: &[DOT],
};
static ICON: Image = Image {
    w: 2,
    h: 1,
    px: &[0x80FF_0000, 0xFF00_FF00],
};

/// A scene with every command kind, glyphs at the left edge included.
fn scene(f: &mut Frame, tick: u8) {
    use rp_draw::Rect as R;
    f.push(Cmd::HGradient {
        r: R::new(2, 2, 40, 20),
        left: 0x4000_0000,
        right: 0xC0A0_A0A0,
    });
    f.push(Cmd::Outline {
        r: R::new(2, 2, 40, 20),
        color: 0x80FF_FFFF,
    });
    f.push(Cmd::Fill {
        r: R::new(30, 25, 10, 5),
        color: 0xFF00_00FF,
    });
    f.push(Cmd::Image {
        x: 47,
        y: 31,
        img: &ICON,
    });
    f.text(0, 10, &FONT, 0xFFFF_FFFF, b"...");
    // What changes between frames: a small "clock".
    f.push(Cmd::Fill {
        r: R::new(10, 24, 4, 4),
        color: 0xFF00_0000 | u32::from(tick),
    });
}

fn renderer<'a>(px: &'a mut [Argb], plane: &'a mut [u8]) -> CpuRenderer<'a> {
    let layer = Layer::new(px, W, H, W as usize).unwrap();
    CpuRenderer::new(layer, plane, W as usize * 4, PlaneFormat::Argb8888)
}

#[test]
fn gradient_ends_exactly_and_rounds_between() {
    assert_eq!(gradient_at(0x0000_0000, 0xFFFF_FFFF, 0, 5), 0);
    assert_eq!(gradient_at(0x0000_0000, 0xFFFF_FFFF, 4, 5), 0xFFFF_FFFF);
    assert_eq!(gradient_at(0x0000_0000, 0xFFFF_FFFF, 2, 5), 0x8080_8080);
    assert_eq!(gradient_at(0xFF00_0000, 0x0000_0000, 1, 3), 0x8000_0000);
    // One column: the left colour.
    assert_eq!(gradient_at(0x1234_5678, 0xFFFF_FFFF, 0, 1), 0x1234_5678);
}

#[test]
fn a_partial_redraw_matches_a_full_one() {
    // Frame 1: everything. Frame 2: the clock changed, only its rectangle
    // is dirty. The result must equal drawing frame 2's scene in full.
    let (mut a_px, mut a_plane) = (vec![0; (W * H) as usize], vec![0; (W * H * 4) as usize]);
    let mut a = renderer(&mut a_px, &mut a_plane);
    let mut f = Frame::new();
    scene(&mut f, 1);
    f.damage(rp_draw::Rect::new(0, 0, W as u16, H as u16));
    a.draw(&f);
    f.clear();
    scene(&mut f, 2);
    f.damage(rp_draw::Rect::new(10, 24, 4, 4));
    let drawn = a.draw(&f);
    assert!(
        drawn.cmds < 6,
        "commands away from the dirty rectangle skipped: {drawn:?}"
    );

    let (mut b_px, mut b_plane) = (vec![0; (W * H) as usize], vec![0; (W * H * 4) as usize]);
    let mut b = renderer(&mut b_px, &mut b_plane);
    f.clear();
    scene(&mut f, 2);
    f.damage(rp_draw::Rect::new(0, 0, W as u16, H as u16));
    b.draw(&f);
    assert_eq!(a.plane(), b.plane());
}

#[test]
fn nothing_outside_the_dirty_rectangles_changes() {
    let (mut px, mut plane) = (
        vec![0x1111_1111; (W * H) as usize],
        vec![0xEE; (W * H * 4) as usize],
    );
    let mut r = renderer(&mut px, &mut plane);
    let mut f = Frame::new();
    scene(&mut f, 3);
    let d = rp_draw::Rect::new(20, 4, 6, 6);
    f.damage(d);
    r.draw(&f);
    for y in 0..H {
        for x in 0..W {
            let inside = d.contains_point(x as u16, y as u16);
            let p = r.layer().get(x, y).unwrap();
            if !inside {
                assert_eq!(p, 0x1111_1111, "layer ({x}, {y})");
                let at = (y * W + x) as usize * 4;
                assert_eq!(&r.plane()[at..at + 4], &[0xEE; 4], "plane ({x}, {y})");
            }
        }
    }
}

#[test]
fn commands_draw_what_they_say() {
    let (mut px, mut plane) = (vec![0; (W * H) as usize], vec![0; (W * H * 4) as usize]);
    let mut r = renderer(&mut px, &mut plane);
    let mut f = Frame::new();
    scene(&mut f, 9);
    f.damage(rp_draw::Rect::new(0, 0, W as u16, H as u16));
    r.draw(&f);
    let l = r.layer();
    // The fill, solid.
    assert_eq!(l.get(35, 27), Some(0xFF00_00FF));
    // The outline over the gradient's first column at its top left.
    let under = gradient_at(0x4000_0000, 0xC0A0_A0A0, 0, 40);
    assert_eq!(l.get(2, 2), Some(layer::over(0x80FF_FFFF, under)));
    // Inside the panel: the gradient alone.
    assert_eq!(
        l.get(3, 12),
        Some(gradient_at(0x4000_0000, 0xC0A0_A0A0, 1, 40))
    );
    // The image clipped at the corner: only its first pixel lands.
    assert_eq!(l.get(47, 31), Some(0x80FF_0000));
    assert_eq!(l.get(46, 31), Some(0));
    // The first glyph overhangs the left edge (left -1): its second
    // column (cover 128) lands at x 0, row baseline - top = 8.
    // Colour alpha 255: the coverage is the alpha.
    let ink = |c: u32| layer::over(c << 24 | 0x00FF_FFFF, 0);
    assert_eq!(l.get(0, 8), Some(ink(128)));
    // The second glyph at pen 3, left -1: x 2..4.
    assert_eq!(l.get(2, 8), Some(ink(255)));
    // Out to the plane as ARGB8888, little-endian.
    let at = (27 * W + 35) as usize * 4;
    assert_eq!(&r.plane()[at..at + 4], &0xFF00_00FFu32.to_le_bytes());
}

#[test]
fn hostile_frames_draw_nothing_wrong() {
    let (mut px, mut plane) = (vec![0; (W * H) as usize], vec![0; (W * H * 4) as usize]);
    let mut r = renderer(&mut px, &mut plane);
    let mut f = Frame::new();
    use rp_draw::Rect as R;
    f.push(Cmd::Fill {
        r: R::new(u16::MAX - 2, u16::MAX - 2, 10, 10),
        color: 0xFFFF_FFFF,
    });
    f.push(Cmd::HGradient {
        r: R::new(0, 0, u16::MAX, u16::MAX),
        left: 0,
        right: 0xFFFF_FFFF,
    });
    f.push(Cmd::Outline {
        r: R::new(40, 30, 1, 1),
        color: 0xFFFF_FFFF,
    });
    static SHORT: Image = Image {
        w: 100,
        h: 100,
        px: &[0xFFFF_FFFF; 3],
    };
    f.push(Cmd::Image {
        x: 0,
        y: 0,
        img: &SHORT,
    });
    f.text(u16::MAX, u16::MAX, &FONT, 0xFFFF_FFFF, b"....");
    f.damage(R::new(0, 0, u16::MAX, u16::MAX));
    r.draw(&f);
    // The single-pixel outline is one pixel.
    assert_eq!(r.layer().get(40, 30), Some(0xFFFF_FFFF));
}
