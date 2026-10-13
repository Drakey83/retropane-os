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
