use super::*;

static A: Glyph = Glyph {
    w: 2,
    h: 3,
    left: 0,
    top: 3,
    advance: 4,
    cover: &[255; 6],
};
static FONT: Font = Font {
    line: 10,
    ascent: 8,
    first: b'A',
    glyphs: &[A],
};

#[test]
fn rect_intersect_and_union() {
    let a = Rect::new(10, 10, 20, 20);
    let b = Rect::new(25, 0, 20, 15);
    assert_eq!(a.intersect(&b), Rect::new(25, 10, 5, 5));
    assert!(a.intersect(&Rect::new(40, 40, 5, 5)).is_empty());
    assert_eq!(a.union(&b), Rect::new(10, 0, 35, 30));
    assert_eq!(Rect::default().union(&a), a);
    assert!(a.contains_point(10, 29) && !a.contains_point(30, 10));
}

#[test]
fn rect_edges_saturate() {
    let r = Rect::new(u16::MAX - 1, 0, 10, 10);
    assert_eq!(r.right(), u16::MAX);
    assert_eq!(r.intersect(&Rect::new(0, 0, 100, 100)), Rect::default());
}

#[test]
fn font_widths_skip_missing_glyphs() {
    assert_eq!(FONT.width(b"AA"), 8);
    assert_eq!(FONT.width(b"A?A\xff"), 8);
    assert!(FONT.glyph(b' ').is_none());
}

#[test]
fn a_full_frame_drops_and_says_so() {
    let mut f = Frame::new();
    for _ in 0..MAX_CMDS {
        f.push(Cmd::Fill {
            r: Rect::new(0, 0, 1, 1),
            color: 0,
        });
    }
    assert!(!f.overflowed());
    f.push(Cmd::Fill {
        r: Rect::new(0, 0, 1, 1),
        color: 0,
    });
    assert!(f.overflowed());
    assert_eq!(f.cmds().count(), MAX_CMDS);
    f.clear();
    assert!(!f.overflowed() && f.cmds().count() == 0);
}

#[test]
fn text_is_stored_and_bounded() {
    let mut f = Frame::new();
    f.text(100, 50, &FONT, 0xFFFF_FFFF, b"AA");
    let c = *f.cmds().next().unwrap();
    let Cmd::Text { text, .. } = c else { panic!() };
    assert_eq!(f.text_of(text), b"AA");
    let b = c.bounds(&f);
    // The line box (8 wide, ascent 8 above the baseline, 10 high) plus 4.
    assert_eq!(b, Rect::new(96, 38, 16, 18));
    // Too much text: dropped, flagged, nothing half-stored.
    let long = [b'A'; MAX_TEXT];
    f.text(0, 20, &FONT, 0, &long);
    assert!(f.overflowed());
    assert_eq!(f.cmds().count(), 1);
}

#[test]
fn dirty_rectangles_merge_when_full() {
    let mut f = Frame::new();
    f.damage(Rect::default());
    assert!(f.dirty().is_empty());
    for i in 0..MAX_DIRTY as u16 {
        f.damage(Rect::new(i * 50, 0, 10, 10));
    }
    // Inside one already: nothing new.
    f.damage(Rect::new(1, 1, 2, 2));
    assert_eq!(f.dirty().len(), MAX_DIRTY);
    // One more: merged into the rectangle it grows least (its neighbour).
    f.damage(Rect::new(352, 0, 10, 10));
    assert_eq!(f.dirty().len(), MAX_DIRTY);
    assert!(f.dirty().contains(&Rect::new(350, 0, 12, 10)));
    // Everything damaged is still covered.
    let all = f.dirty().iter().fold(Rect::default(), |a, r| a.union(r));
    assert_eq!(all, Rect::new(0, 0, 362, 10));
}

/// R172: a rectangle that covers ones already listed replaces them, and
/// overlapping ones merge when the union is no bigger than the two: the
/// look switch's row, panel and whole screen are one whole screen, not
/// 1.84 screens drawn.
#[test]
fn dirty_rectangles_never_list_a_pixel_twice_where_avoidable() {
    let area = |rs: &[Rect]| {
        rs.iter()
            .map(|r| u32::from(r.w) * u32::from(r.h))
            .sum::<u32>()
    };
    let mut f = Frame::new();
    f.damage(Rect::new(16, 383, 448, 36));
    f.damage(Rect::new(0, 98, 480, 502));
    f.damage(Rect::new(0, 0, 480, 640));
    assert_eq!(f.dirty(), &[Rect::new(0, 0, 480, 640)]);
    // Two that overlap mostly: one union.
    let mut f = Frame::new();
    f.damage(Rect::new(0, 0, 100, 100));
    f.damage(Rect::new(10, 0, 100, 100));
    assert_eq!(f.dirty(), &[Rect::new(0, 0, 110, 100)]);
    // Two that barely touch at a corner: kept apart (the union would draw
    // far more).
    let mut f = Frame::new();
    f.damage(Rect::new(0, 0, 100, 100));
    f.damage(Rect::new(99, 99, 100, 100));
    assert_eq!(f.dirty().len(), 2);
    // A merge that grows into a third: that one folds in too.
    let mut f = Frame::new();
    f.damage(Rect::new(0, 0, 50, 100));
    f.damage(Rect::new(100, 0, 50, 100));
    f.damage(Rect::new(40, 0, 70, 100));
    assert_eq!(f.dirty(), &[Rect::new(0, 0, 150, 100)]);
    // Every pixel damaged stays covered, and no more pixels listed than
    // the rectangles given, in every order.
    let given = [
        Rect::new(16, 383, 448, 36),
        Rect::new(0, 98, 480, 502),
        Rect::new(0, 0, 480, 640),
        Rect::new(240, 556, 120, 44),
    ];
    for start in 0..given.len() {
        let mut f = Frame::new();
        for k in 0..given.len() {
            f.damage(given[(start + k) % given.len()]);
        }
        for g in &given {
            assert!(
                f.dirty().iter().any(|d| d.intersect(g) == *g) || {
                    // Covered by several together: check pixel by pixel.
                    (g.y..g.y + g.h).all(|y| {
                        (g.x..g.x + g.w).all(|x| {
                            f.dirty().iter().any(|d| {
                                d.intersect(&Rect::new(x, y, 1, 1)) == Rect::new(x, y, 1, 1)
                            })
                        })
                    })
                }
            );
        }
        assert!(area(f.dirty()) <= area(&given));
    }
}

/// A glyph lying wholly left of its pen (the living sky's clouds) is
/// within its text's bounds: bounded by the line box alone, a cloud whose
/// pen passed the sky's right edge was skipped while still on screen (the
/// owner's run, 2026-11-01).
#[test]
fn text_bounds_hold_a_glyph_left_of_its_pen() {
    static COVER: [u8; 40 * 10] = [255; 40 * 10];
    static GLYPHS: [Glyph; 1] = [Glyph {
        w: 40,
        h: 10,
        left: -40,
        top: 0,
        advance: 0,
        cover: &COVER,
    }];
    static CLOUD: Font = Font {
        line: 10,
        ascent: 0,
        first: b'A',
        glyphs: &GLYPHS,
    };
    let mut f = Frame::new();
    f.text(300, 100, &CLOUD, 0xFFFF_FFFF, b"A");
    let b = f.cmds().next().unwrap().bounds(&f);
    assert!(b.x <= 260 && b.x + b.w >= 300, "{b:?}");
    assert!(b.y <= 100 && b.y + b.h >= 110, "{b:?}");
    // A dirty area left of the pen still meets it.
    assert!(!Rect::new(250, 90, 30, 30).intersect(&b).is_empty());
}
