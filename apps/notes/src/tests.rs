//! Notes driven as the stylus drives it, against a real store.

extern crate std;

use super::*;
use rp_app::{Fonts, Frame, Look, Now, Shapes};
use rp_draw::{Font, Image};
use rp_pim::store::Store;
use std::boxed::Box;
use std::vec;

static FONT: Font = Font {
    line: 20,
    ascent: 16,
    first: 32,
    glyphs: &[],
};
static CORNER: Image = Image {
    w: 0,
    h: 0,
    px: &[],
};
static ROWS: [Argb; 64] = [0xFF20_2020; 64];
static ART: PanelArt = PanelArt {
    radius: 0,
    rows: &ROWS,
    top: [0, 0],
    bottom: [0, 0],
    side: 0,
    corners: [&CORNER, &CORNER, &CORNER, &CORNER],
};
static FONTS: Fonts = Fonts {
    ui: &FONT,
    label: &FONT,
    big: &FONT,
    num: &FONT,
    title: &FONT,
};
static LOOK: Look = Look {
    light: false,
    panel: &ART,
    pressed: &ART,
    key: &ART,
    key_down: &ART,
    on: &ART,
};

use rp_app::PanelArt;

fn store() -> &'static core::cell::RefCell<Option<Store<'static>>> {
    let mem = vec![0u8; rp_pim::store::MEMORY].leak();
    let mut s = Store::new(mem);
    s.load_start(None);
    Box::leak(Box::new(core::cell::RefCell::new(Some(s))))
}

fn ctx(pim: &'static core::cell::RefCell<Option<Store<'static>>>) -> Ctx<'static> {
    Ctx {
        fonts: &FONTS,
        look: &LOOK,
        now: Now {
            day: 19_723,
            minute: 720,
        },
        pics: None,
        forecast: &[],
        us: true,
        utc_offset: 0,
        shapes: Box::leak(Box::new(Shapes::new())),
        clip: Box::leak(Box::new(core::cell::RefCell::new(rp_app::Text::EMPTY))),
        pim: Some(pim),
    }
}

fn draw(n: &mut Notes, cx: &Ctx) {
    let mut f = Box::new(Frame::new());
    n.draw(&mut f, cx);
}

fn note_text(pim: &core::cell::RefCell<Option<Store<'static>>>) -> (std::vec::Vec<u8>, usize) {
    let g = pim.borrow();
    let s = g.as_ref().unwrap();
    let r = s.all(Kind::Note).next().unwrap();
    (r.text(tag::TEXT).to_vec(), r.text(tag::INK).len())
}

#[test]
fn new_then_write_a_stroke_then_undo_it() {
    let pim = store();
    let cx = ctx(pim);
    let mut n = opened(19_723);
    draw(&mut n, &cx);
    // New: a note opens, its ink panel ready.
    assert_eq!(n.event(Event::Key(Key::SoftRight), &cx), Reply::Redraw);
    assert!(matches!(n.view, View::Note(0)));
    draw(&mut n, &cx);
    let (ix, iy) = n.ink_at.expect("an ink panel on screen");
    // A stroke across the panel.
    n.event(
        Event::PenDown {
            x: ix + 20,
            y: iy + 40,
        },
        &cx,
    );
    for k in 1..20u16 {
        n.event(
            Event::PenMove {
                x: ix + 20 + k * 6,
                y: iy + 40 + k % 5,
            },
            &cx,
        );
    }
    n.event(
        Event::PenUp {
            x: ix + 140,
            y: iy + 44,
        },
        &cx,
    );
    let (_, ink) = note_text(pim);
    assert!(ink > 6, "the stroke kept ({ink} bytes)");
    // UNDO: the stroke goes.
    draw(&mut n, &cx);
    n.event(
        Event::PenUp {
            x: STRIP.x + 6 + 5 * 76 + 10,
            y: STRIP.y + 20,
        },
        &cx,
    );
    let (_, ink) = note_text(pim);
    assert_eq!(ink, 0);
}

#[test]
fn a_checklist_item_ticks_and_unticks() {
    let pim = store();
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        s.put(
            Kind::Note,
            None,
            &[
                (tag::TITLE, b"Shopping"),
                (tag::TEXT, b"[ ] Milk\n[ ] Stamps"),
            ],
        )
        .unwrap();
    }
    let cx = ctx(pim);
    let mut n = opened(19_723);
    n.show(View::Note(0));
    draw(&mut n, &cx);
    let (r, _) = n.checks[1];
    n.event(
        Event::PenUp {
            x: r.x + 10,
            y: r.y + 10,
        },
        &cx,
    );
    assert_eq!(note_text(pim).0, b"[ ] Milk\n[x] Stamps");
    draw(&mut n, &cx);
    let (r, _) = n.checks[1];
    n.event(
        Event::PenUp {
            x: r.x + 10,
            y: r.y + 10,
        },
        &cx,
    );
    assert_eq!(note_text(pim).0, b"[ ] Milk\n[ ] Stamps");
}

#[test]
fn typing_a_list_adds_items_and_return_starts_the_next() {
    let pim = store();
    let cx = ctx(pim);
    let mut n = opened(19_723);
    n.event(Event::Key(Key::SoftRight), &cx);
    draw(&mut n, &cx);
    // LIST, then "Eggs", Return, "Tea".
    n.event(
        Event::PenUp {
            x: STRIP.x + 6 + 4 * 76 + 10,
            y: STRIP.y + 20,
        },
        &cx,
    );
    assert_eq!(n.keyboard(), Some(Field::Sentence));
    for &c in b"Eggs" {
        n.event(Event::Char(c), &cx);
    }
    n.event(Event::Done, &cx);
    for &c in b"Tea" {
        n.event(Event::Char(c), &cx);
    }
    assert_eq!(note_text(pim).0, b"\x01\n[ ] Eggs\n[ ] Tea");
}

#[test]
fn ink_round_trips_with_each_stroke_s_style() {
    let a: &[(i16, i16)] = &[(0, 0), (10, 5), (300, 5), (300, 190)];
    let b: &[(i16, i16)] = &[(40, 40)];
    let mut out = [0u8; 256];
    let n = encode_styled(&[a, b], &[3, MARKER], &mut out).unwrap();
    let mut pts = [(0i16, 0i16); 64];
    let mut ends = [0u16; 8];
    let mut styles = [0u8; 8];
    let ns = decode_ink(&out[..n], &mut pts, &mut ends, &mut styles);
    assert_eq!(ns, 2);
    assert_eq!(&styles[..2], &[3, MARKER]);
    // The long steps were split but every point given is there.
    let first = &pts[..usize::from(ends[0])];
    for p in a {
        assert!(first.contains(p), "{p:?}");
    }
    assert_eq!(&pts[usize::from(ends[0])..usize::from(ends[1])], b);
    // Every cut of the field reads without a panic.
    for cut in 0..n {
        let _ = decode_ink(&out[..cut], &mut pts, &mut ends, &mut styles);
    }
}

#[test]
fn search_finds_by_title_and_text_and_the_menu_pins_papers_and_deletes() {
    let pim = store();
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        s.put(
            Kind::Note,
            None,
            &[(tag::TITLE, b"Garden"), (tag::TEXT, b"Plant tomatoes")],
        )
        .unwrap();
        s.put(
            Kind::Note,
            None,
            &[(tag::TITLE, b"Shopping"), (tag::TEXT, b"[ ] Milk")],
        )
        .unwrap();
    }
    let cx = ctx(pim);
    let mut n = opened(19_723);
    draw(&mut n, &cx);
    assert_eq!(n.n_cards, 2);
    n.event(
        Event::PenUp {
            x: SEARCH.x + 10,
            y: SEARCH.y + 10,
        },
        &cx,
    );
    for &c in b"toma" {
        n.event(Event::Char(c), &cx);
    }
    draw(&mut n, &cx);
    assert_eq!(n.n_cards, 1);
    // Open it: Menu, then PIN and PAPER (ruled to plain).
    let (r, _) = n.cards[0];
    n.event(
        Event::PenUp {
            x: r.x + 20,
            y: r.y + 20,
        },
        &cx,
    );
    draw(&mut n, &cx);
    n.event(Event::Key(Key::SoftRight), &cx);
    n.event(
        Event::PenUp {
            x: STRIP.x + 20,
            y: STRIP.y + 20,
        },
        &cx,
    );
    n.event(
        Event::PenUp {
            x: STRIP.x + 6 + 2 * 114 + 20,
            y: STRIP.y + 20,
        },
        &cx,
    );
    {
        let g = pim.borrow();
        let r = g.as_ref().unwrap().all(Kind::Note).next().unwrap();
        assert_eq!(r.num(tag::PINNED), Some(1));
        assert_eq!(r.num(tag::PAPER), Some(1));
        assert_eq!(r.text(tag::TEXT), b"Plant tomatoes");
    }
    // DELETE asks once, then deletes.
    let del = STRIP.x + 6 + 3 * 114 + 20;
    draw(&mut n, &cx);
    n.event(
        Event::PenUp {
            x: del,
            y: STRIP.y + 20,
        },
        &cx,
    );
    assert_eq!(pim.borrow().as_ref().unwrap().all(Kind::Note).count(), 2);
    n.event(
        Event::PenUp {
            x: del,
            y: STRIP.y + 20,
        },
        &cx,
    );
    assert_eq!(pim.borrow().as_ref().unwrap().all(Kind::Note).count(), 1);
}
