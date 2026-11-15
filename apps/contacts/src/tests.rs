//! Contacts driven as the stylus drives it, against a real store.

extern crate std;

use super::*;
use rp_app::{Argb, Fonts, Frame, Look, Now, Shapes};
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

fn draw(c: &mut Contacts, cx: &Ctx) {
    let mut f = Box::new(Frame::new());
    c.draw(&mut f, cx);
}

fn people(pim: &core::cell::RefCell<Option<Store<'static>>>) {
    let mut g = pim.borrow_mut();
    let s = g.as_mut().unwrap();
    for (first, last, mobile, addr) in [
        (
            &b"Grace"[..],
            &b"Hopper"[..],
            &b"+17035550107"[..],
            &b"2100 Clarendon Blvd
Arlington, VA 22201"[..],
        ),
        (b"Margaret", b"Hamilton", b"+16175550111", b""),
        (b"Barbara", b"Liskov", b"+16175550170", b""),
        (b"Katherine", b"Johnson", b"+17575550116", b""),
    ] {
        s.put(
            Kind::Contact,
            None,
            &[
                (tag::FIRST, first),
                (tag::LAST, last),
                (tag::MOBILE, mobile),
                (tag::ADDRESS, addr),
            ],
        )
        .unwrap();
    }
}

fn search(c: &mut Contacts, cx: &Ctx, q: &[u8]) -> usize {
    c.event(
        Event::PenUp {
            x: SEARCH.x + 20,
            y: SEARCH.y + 10,
        },
        cx,
    );
    for &ch in q {
        c.event(Event::Char(ch), cx);
    }
    draw(c, cx);
    let n = c.n_rows;
    for _ in q {
        c.event(Event::Delete, cx);
    }
    n
}

#[test]
fn search_by_name_initials_digits_and_city() {
    let pim = store();
    people(pim);
    let cx = ctx(pim);
    let mut c = Contacts::new();
    draw(&mut c, &cx);
    assert_eq!(c.n_rows, 4);
    assert_eq!(search(&mut c, &cx, b"lisk"), 1);
    assert_eq!(search(&mut c, &cx, b"gh"), 1);
    // The digits of numbers kept as +1617...
    assert_eq!(search(&mut c, &cx, b"617 555"), 2);
    assert_eq!(search(&mut c, &cx, b"arlington"), 1);
    assert_eq!(search(&mut c, &cx, b"zz"), 0);
}

#[test]
fn the_letter_strip_starts_the_list_at_its_letter_and_a_row_opens_its_card() {
    let pim = store();
    people(pim);
    let cx = ctx(pim);
    let mut c = Contacts::new();
    draw(&mut c, &cx);
    // K is the 11th letter.
    let y = STRIP.y + 10 * 18 + 4;
    c.event(Event::PenDown { x: STRIP.x + 6, y }, &cx);
    assert_eq!(c.bubble, Some(b'K'));
    c.event(Event::PenUp { x: STRIP.x + 6, y }, &cx);
    draw(&mut c, &cx);
    // Liskov only (from K on).
    assert_eq!(c.n_rows, 1);
    let (r, _) = c.rows_at[0];
    c.event(
        Event::PenUp {
            x: r.x + 100,
            y: r.y + 10,
        },
        &cx,
    );
    assert!(matches!(c.view, View::Card(_)));
}

#[test]
fn the_dpad_walks_the_people_and_its_centre_opens_one() {
    // R203: the D-pad works in every app.
    let pim = store();
    people(pim);
    let cx = ctx(pim);
    let mut c = Contacts::new();
    draw(&mut c, &cx);
    assert!(c.n_rows > 1);
    assert_eq!(c.event(Event::Key(Key::Down), &cx), Reply::Redraw);
    assert_eq!(c.event(Event::Key(Key::Down), &cx), Reply::Redraw);
    draw(&mut c, &cx);
    assert_eq!(c.event(Event::Key(Key::Enter), &cx), Reply::Redraw);
    assert!(matches!(c.view, View::Card(_)), "{:?}", c.view);
}
