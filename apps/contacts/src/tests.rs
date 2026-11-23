//! Contacts driven as the stylus drives it, against a real store.

extern crate std;

use super::*;
use rp_app::{Argb, Fonts, Frame, Look, Now, Shapes};
use rp_draw::{Font, Image};
use rp_pim::store::Store;
use std::boxed::Box;
use std::vec;
use std::vec::Vec;

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

fn type_in(c: &mut Contacts, cx: &Ctx, s: &[u8]) {
    for &ch in s {
        c.event(Event::Char(ch), cx);
    }
}

/// A record's fields by tag.
type Fields = std::vec::Vec<(u8, std::vec::Vec<u8>)>;

/// The fields of the person whose first name is `first`, and its id.
fn kept(pim: &core::cell::RefCell<Option<Store<'static>>>, first: &[u8]) -> Option<(u32, Fields)> {
    let g = pim.borrow();
    let s = g.as_ref().unwrap();
    let r = s.all(Kind::Contact).find(|r| r.text(tag::FIRST) == first)?;
    let tags = [
        tag::FIRST,
        tag::LAST,
        tag::COMPANY,
        tag::MOBILE,
        tag::HOME,
        tag::WORK,
        tag::EMAIL,
        tag::ADDRESS,
        tag::FAVOURITE,
        tag::BIRTHDAY,
    ];
    Some((
        r.id,
        tags.iter().map(|&t| (t, r.text(t).to_vec())).collect(),
    ))
}

/// The D-pad down the form until field `to` is the one typed.
fn down_to(c: &mut Contacts, cx: &Ctx, to: EditField) {
    for _ in 0..40 {
        if c.view == View::Edit(to) {
            return;
        }
        c.event(Event::Key(Key::Down), cx);
    }
    panic!("{to:?} not reached: {:?}", c.view);
}

#[test]
fn every_field_is_typed_and_a_kept_person_is_changed_and_deleted() {
    use EditField as E;
    // The first editor typed a first name, a mobile and a street; version 2
    // nine fields; version 3 every field of the sheet: the address in its
    // parts, a job title, the days, a website, and extra numbers and
    // e-mails with labels of their own.
    let pim = store();
    let cx = ctx(pim);
    let mut c = Contacts::new();
    draw(&mut c, &cx);
    // Done with no name says so and keeps nothing.
    c.event(Event::Key(Key::SoftRight), &cx);
    c.event(Event::Key(Key::Down), &cx);
    c.event(Event::Key(Key::SoftRight), &cx);
    assert!(c.need_name && c.view == View::Edit(E::First));
    assert_eq!(pim.borrow().as_ref().unwrap().all(Kind::Contact).count(), 0);
    for (f, text) in [
        (E::First, &b"Ada"[..]),
        (E::Last, b"Lovelace"),
        (E::Company, b"Analytical Engines"),
        (E::Job, b"Programmer"),
        (E::Mobile, b"2065550142"),
        (E::Home, b"2065550143"),
        (E::Work, b"2065550144"),
        (E::Email, b"ada@example.com"),
        (E::Street, b"12 St James Sq"),
        (E::City, b"Seattle"),
        (E::Region, b"WA"),
        (E::Postal, b"98101"),
        (E::Birthday, b"02/30"),
        (E::Anniversary, b"7/8/1835"),
        (E::Website, b"ada.example"),
    ] {
        down_to(&mut c, &cx, f);
        draw(&mut c, &cx);
        assert!(c.keyboard().is_some(), "{f:?}");
        type_in(&mut c, &cx, text);
    }
    assert_eq!(c.keyboard(), Some(Field::Link));
    // A number of its own: + Add, the label tapped and typed, the number.
    let row = |c: &Contacts, f: E| c.form_at[..c.n_form].iter().find(|t| t.1 == f).map(|t| t.0);
    c.view = View::Edit(E::Work);
    draw(&mut c, &cx);
    let add = row(&c, E::AddPhone).expect("+ Add a phone shown");
    // (A key's row is pressed by its first tap.)
    c.event(
        Event::PenUp {
            x: add.x + 20,
            y: add.y + 20,
        },
        &cx,
    );
    assert_eq!(c.view, View::Edit(E::Extra(0)));
    assert_eq!(c.keyboard(), Some(Field::Phone));
    type_in(&mut c, &cx, b"3605550199");
    draw(&mut c, &cx);
    let extra = row(&c, E::Extra(0)).unwrap();
    c.event(
        Event::PenUp {
            x: extra.x + 20,
            y: extra.y + 20,
        },
        &cx,
    );
    assert_eq!(c.view, View::Edit(E::Label(0)));
    type_in(&mut c, &cx, b"Cabin");
    c.event(Event::Done, &cx);
    assert_eq!(c.view, View::Edit(E::Extra(0)));
    // An e-mail of its own, its label from the strip.
    down_to(&mut c, &cx, E::AddEmail);
    c.event(Event::Key(Key::Enter), &cx);
    assert_eq!(c.view, View::Edit(E::Extra(1)));
    assert_eq!(c.keyboard(), Some(Field::Email));
    type_in(&mut c, &cx, b"ada@work.example");
    c.view = View::Edit(E::Label(1));
    assert_eq!(c.suggestions()[1], b"Work");
    c.event(Event::Suggestion(1), &cx);
    // 30 February: refused on its row, kept once it is a day.
    down_to(&mut c, &cx, E::Favourite);
    c.event(Event::Key(Key::Enter), &cx);
    c.event(Event::Key(Key::SoftRight), &cx);
    assert_eq!(c.view, View::Edit(E::Birthday));
    assert_eq!(c.bad_day, Some(E::Birthday));
    assert_eq!(pim.borrow().as_ref().unwrap().all(Kind::Contact).count(), 0);
    for _ in 0..5 {
        c.event(Event::Delete, &cx);
    }
    assert_eq!(c.bad_day, None);
    type_in(&mut c, &cx, b"12/10");
    c.event(Event::Key(Key::SoftRight), &cx);
    assert_eq!(c.view, View::List);
    let (id, f) = kept(pim, b"Ada").expect("kept");
    let get = |f: &Fields, t: u8| f.iter().find(|x| x.0 == t).unwrap().1.clone();
    let rec = |t: u8| {
        let g = pim.borrow();
        g.as_ref().unwrap().get(id).unwrap().text(t).to_vec()
    };
    assert_eq!(get(&f, tag::LAST), b"Lovelace");
    assert_eq!(get(&f, tag::COMPANY), b"Analytical Engines");
    assert_eq!(rec(tag::JOB), b"Programmer");
    assert_eq!(get(&f, tag::MOBILE), b"+12065550142");
    assert_eq!(get(&f, tag::HOME), b"+12065550143");
    assert_eq!(get(&f, tag::WORK), b"+12065550144");
    assert_eq!(get(&f, tag::EMAIL), b"ada@example.com");
    assert_eq!(
        get(&f, tag::ADDRESS),
        b"12 St James Sq\nSeattle\nWA\n98101\n"
    );
    assert_eq!(get(&f, tag::FAVOURITE), 1u32.to_le_bytes());
    let day = |b: Vec<u8>| rp_pim::contact::Day::of_field(&b);
    let d = |y, m, dd| {
        Some(rp_pim::contact::Day {
            year: y,
            month: m,
            day: dd,
        })
    };
    assert_eq!(day(get(&f, tag::BIRTHDAY)), d(None, 12, 10));
    assert_eq!(day(rec(tag::ANNIVERSARY)), d(Some(1835), 7, 8));
    assert_eq!(rec(tag::WEBSITE), b"ada.example");
    let more: Vec<(Vec<u8>, Vec<u8>)> = rp_pim::contact::entries(&rec(tag::MORE))
        .map(|e| (e.label.to_vec(), e.value.to_vec()))
        .collect();
    assert_eq!(
        more,
        [
            (b"Cabin".to_vec(), b"+13605550199".to_vec()),
            (b"Work".to_vec(), b"ada@work.example".to_vec()),
        ]
    );
    // A field the form does not show is kept through a change.
    pim.borrow_mut()
        .as_mut()
        .unwrap()
        .change(id, &[(tag::NOTE, b"Met at the Royal Society")])
        .unwrap();
    // Changed from the card: every field comes back as kept; the last
    // name retyped, the work number emptied, the rest left alone.
    let card_of = |c: &Contacts| {
        View::Card(
            c.people
                .iter()
                .position(|p| p.is_some_and(|p| p.id == id))
                .unwrap() as u8,
        )
    };
    draw(&mut c, &cx);
    c.view = card_of(&c);
    draw(&mut c, &cx);
    c.event(Event::Key(Key::SoftRight), &cx);
    assert_eq!(c.view, View::Edit(E::First));
    assert_eq!(c.val(E::Mobile), b"+12065550142");
    assert_eq!(c.val(E::City), b"Seattle");
    assert_eq!(c.val(E::Birthday), b"12/10");
    assert_eq!(c.val(E::Extra(0)), b"+13605550199");
    assert_eq!(c.val(E::Label(0)), b"Cabin");
    assert!(c.fav);
    c.event(Event::Key(Key::Down), &cx);
    for _ in 0..8 {
        c.event(Event::Delete, &cx);
    }
    type_in(&mut c, &cx, b"King");
    down_to(&mut c, &cx, E::Work);
    for _ in 0..12 {
        c.event(Event::Delete, &cx);
    }
    // A tap on another row types that one from then on.
    draw(&mut c, &cx);
    let (r, fld) = c.form_at[c.n_form - 1];
    assert_ne!(fld, E::Work);
    c.event(
        Event::PenDown {
            x: r.x + 109,
            y: r.y + 9,
        },
        &cx,
    );
    c.event(
        Event::PenUp {
            x: r.x + 109,
            y: r.y + 9,
        },
        &cx,
    );
    assert_eq!(c.view, View::Edit(fld));
    c.event(Event::Key(Key::SoftRight), &cx);
    let (id2, f) = kept(pim, b"Ada").expect("still kept");
    assert_eq!(id2, id);
    assert_eq!(get(&f, tag::LAST), b"King");
    assert_eq!(get(&f, tag::WORK), b"");
    assert_eq!(get(&f, tag::MOBILE), b"+12065550142");
    assert_eq!(get(&f, tag::HOME), b"+12065550143");
    assert_eq!(
        get(&f, tag::ADDRESS),
        b"12 St James Sq\nSeattle\nWA\n98101\n"
    );
    assert_eq!(get(&f, tag::FAVOURITE), 1u32.to_le_bytes());
    assert_eq!(rec(tag::NOTE), b"Met at the Royal Society");
    assert_eq!(rp_pim::contact::entries(&rec(tag::MORE)).count(), 2);
    // Deleted: asked once, then gone; a move away takes the question back.
    draw(&mut c, &cx);
    c.view = card_of(&c);
    c.event(Event::Key(Key::SoftRight), &cx);
    down_to(&mut c, &cx, E::Delete);
    c.event(Event::Key(Key::Enter), &cx);
    assert!(c.confirm_delete && kept(pim, b"Ada").is_some());
    c.event(Event::Key(Key::Up), &cx);
    c.event(Event::Key(Key::Down), &cx);
    assert!(!c.confirm_delete);
    c.event(Event::Key(Key::Enter), &cx);
    c.event(Event::Key(Key::Enter), &cx);
    assert_eq!(c.view, View::List);
    assert!(kept(pim, b"Ada").is_none());
}

#[test]
fn a_card_shows_every_line_and_each_copies_by_tap_or_dpad() {
    let pim = store();
    pim.borrow_mut()
        .as_mut()
        .unwrap()
        .put(
            Kind::Contact,
            None,
            &[
                (tag::FIRST, b"Grace"),
                (tag::LAST, b"Hopper"),
                (tag::MOBILE, b"+17035550107"),
                (tag::HOME, b"+17035550108"),
                (tag::WORK, b"+17035550109"),
                (tag::EMAIL, b"grace@example.com"),
                (tag::ADDRESS, b"2100 Clarendon Blvd\nArlington, VA 22201"),
            ],
        )
        .unwrap();
    let cx = ctx(pim);
    let mut c = Contacts::new();
    draw(&mut c, &cx);
    c.view = View::Card(0);
    draw(&mut c, &cx);
    // Three numbers, the e-mail, the address and the Copy key, all on the
    // card (the address was dropped when the rest filled it).
    let kinds: std::vec::Vec<u8> = c.card_at[..c.n_card].iter().map(|t| t.1).collect();
    assert_eq!(kinds, [PICTURE, 5, 10, 11, 12, 13, 14]);
    // (Past the avatar, which opens the picture chooser.)
    c.event(Event::Key(Key::Down), &cx);
    assert!(c.card_at[..c.n_card].iter().all(|(r, _)| r.bottom() <= 596));
    // The D-pad reaches each, the centre copies it; the mark shows, then
    // goes.
    let mut seen = std::vec::Vec::new();
    for _ in 0..8 {
        c.event(Event::Key(Key::Down), &cx);
        assert_eq!(c.event(Event::Key(Key::Enter), &cx), Reply::Animate);
        seen.push(std::string::String::from_utf8_lossy(cx.clip.borrow().get()).into_owned());
    }
    for want in [
        "(703) 555-0107",
        "(703) 555-0108",
        "(703) 555-0109",
        "grace@example.com",
        "2100 Clarendon Blvd, Arlington, VA 22201",
    ] {
        assert!(
            seen.iter().any(|s| s == want),
            "{want} not copied: {seen:?}"
        );
    }
    assert!(c.copied.is_some());
    for _ in 0..40 {
        c.event(Event::Tick, &cx);
    }
    assert!(c.copied.is_none());
}

#[test]
fn a_letter_with_more_people_than_fit_goes_on_over_the_page() {
    // Thirty people under S: the list cut the letter off at the screen's
    // foot, and the rest were out of the stylus's reach and the D-pad's.
    let pim = store();
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        for k in 0..30u8 {
            let first = [b'A' + k % 26, b'a' + k / 26];
            s.put(
                Kind::Contact,
                None,
                &[(tag::FIRST, &first), (tag::LAST, b"Smith")],
            )
            .unwrap();
        }
    }
    let cx = ctx(pim);
    let mut c = Contacts::new();
    draw(&mut c, &cx);
    assert!(c.n_rows < 30);
    // The D-pad alone: down through every page, each person focused once.
    let mut met = std::vec::Vec::new();
    for _ in 0..200 {
        if c.event(Event::Key(Key::Down), &cx) == Reply::Nothing {
            break;
        }
        draw(&mut c, &cx);
        if let Some(&(_, i)) = c.rows_at[..c.n_rows]
            .iter()
            .find(|(r, _)| Some(*r) == c.focus.rect())
            && !met.contains(&i)
        {
            met.push(i);
        }
    }
    assert_eq!(met.len(), 30, "people reached by the D-pad");
    // And back up to the first page; a drag turns the page too.
    for _ in 0..200 {
        if c.event(Event::Key(Key::Up), &cx) == Reply::Nothing {
            break;
        }
        draw(&mut c, &cx);
    }
    assert_eq!(c.first_row, 0);
    c.event(Event::PenDown { x: 200, y: 400 }, &cx);
    c.event(Event::PenUp { x: 200, y: 300 }, &cx);
    assert!(c.first_row > 0);
    c.event(Event::PenDown { x: 200, y: 300 }, &cx);
    c.event(Event::PenUp { x: 200, y: 400 }, &cx);
    assert_eq!(c.first_row, 0);
}

#[test]
fn the_keyboard_s_done_keeps_the_person() {
    // The owner on the X51v, 2026-11-15: "Keyboard done doesnt do
    // anything when trying to say done when editing contact".
    let pim = store();
    let cx = ctx(pim);
    let mut c = Contacts::new();
    draw(&mut c, &cx);
    c.event(Event::Key(Key::SoftRight), &cx);
    // With no name it says so and keeps nothing.
    c.event(Event::Done, &cx);
    assert!(c.need_name && matches!(c.view, View::Edit(_)));
    type_in(&mut c, &cx, b"Ada");
    c.event(Event::Key(Key::Down), &cx);
    type_in(&mut c, &cx, b"Lovelace");
    assert_eq!(c.event(Event::Done, &cx), Reply::Redraw);
    assert_eq!(c.view, View::List);
    assert!(kept(pim, b"Ada").is_some());
}

#[test]
fn a_suggested_address_fills_the_street_and_the_city_line() {
    // The owner on the X51v, 2026-11-15: "the address doent auto fill in
    // the city, state, zipcode when selecting a pre populated address".
    let pim = store();
    let cx = ctx(pim);
    let answer = br#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"osm_type":"W","osm_id":242245378,"osm_key":"building","osm_value":"house","type":"house","housenumber":"410","street":"Pine Street","city":"Boulder","county":"Boulder","state":"Colorado","country":"United States","postcode":"80302","countrycode":"US","extent":[-105.29,40.01,-105.29,40.01]},"geometry":{"type":"Point","coordinates":[-105.29,40.01]}}]}"#;
    for by_strip in [false, true] {
        let mut c = Contacts::new();
        draw(&mut c, &cx);
        c.event(Event::Key(Key::SoftRight), &cx);
        type_in(&mut c, &cx, b"Ada");
        // First, Last, Company, Job, three phones, + Add, E-mail, + Add.
        for _ in 0..10 {
            c.event(Event::Key(Key::Down), &cx);
        }
        assert_eq!(c.view, View::Edit(EditField::Street));
        type_in(&mut c, &cx, b"410 Pi");
        assert!(c.net_ask().is_some(), "the street asked about");
        c.net_answer(true, answer, &cx);
        draw(&mut c, &cx);
        if by_strip {
            // The keyboard's suggestion strip.
            assert_eq!(c.suggestions()[0], b"410 Pine Street");
            c.event(Event::Suggestion(0), &cx);
        } else {
            // A tap on the sheet under the field.
            let r = suggestion_row(0);
            c.event(
                Event::PenDown {
                    x: r.x + 20,
                    y: r.y + 20,
                },
                &cx,
            );
            c.event(
                Event::PenUp {
                    x: r.x + 20,
                    y: r.y + 20,
                },
                &cx,
            );
        }
        assert_eq!(
            c.val(EditField::Street),
            b"410 Pine Street",
            "strip {by_strip}"
        );
        // The service's city line in its parts.
        assert_eq!(c.val(EditField::City), b"Boulder", "strip {by_strip}");
        assert_eq!(c.val(EditField::Region), b"Colorado");
        assert_eq!(c.val(EditField::Postal), b"80302");
        // And kept so.
        draw(&mut c, &cx);
        c.event(Event::Done, &cx);
        let (_, f) = kept(pim, b"Ada").expect("kept");
        let addr = &f.iter().find(|x| x.0 == tag::ADDRESS).unwrap().1;
        assert_eq!(addr, b"410 Pine Street\nBoulder\nColorado\n80302\n");
        let id = kept(pim, b"Ada").unwrap().0;
        pim.borrow_mut().as_mut().unwrap().remove(id).unwrap();
    }
}

#[test]
fn a_person_s_picture_is_chosen_from_the_avatar_kept_shown_and_removed() {
    // The owner on the X51v, 2026-11-15: "tapping on the name icon in
    // contacts doenst give you the option to assign a picture".
    let pim = store();
    {
        let mut g = pim.borrow_mut();
        g.as_mut()
            .unwrap()
            .put(
                Kind::Contact,
                None,
                &[(tag::FIRST, b"Grace"), (tag::LAST, b"Hopper")],
            )
            .unwrap();
    }
    let mut cx = ctx(pim);
    let classes = rp_app::kit::PIC_CLASSES;
    let pics: &'static rp_draw::pics::Pics<'static> =
        Box::leak(Box::new(rp_draw::pics::Pics::new(
            vec![0u32; rp_draw::pics::Pics::words(&classes)].leak(),
            &classes,
        )));
    cx.pics = Some(pics);
    let mut c = Contacts::new();
    draw(&mut c, &cx);
    c.view = View::Card(0);
    draw(&mut c, &cx);
    let id = c.people[0].unwrap().id;
    // The avatar is the card's first target; a touch opens the chooser,
    // which asks the PC for its pictures.
    let (face, kind) = c.card_at[0];
    assert_eq!(kind, PICTURE);
    c.event(
        Event::PenDown {
            x: face.x + 40,
            y: face.y + 40,
        },
        &cx,
    );
    c.event(
        Event::PenUp {
            x: face.x + 40,
            y: face.y + 40,
        },
        &cx,
    );
    assert_eq!(c.view, View::Photo(0));
    let ask = c.net_ask().expect("the list asked for");
    assert_eq!((ask.what, ask.arg.get()), (&b"PHOTOS"[..], &b"0"[..]));
    assert!(c.net_ask().is_none(), "asked once");
    // Nine names: eight shown and MORE.
    let mut list = std::string::String::new();
    for k in 0..9 {
        list += &std::format!(
            "{k}|Picture {k}
"
        );
    }
    c.net_answer(true, list.as_bytes(), &cx);
    draw(&mut c, &cx);
    assert_eq!((c.n_picks, c.more), (PICKS, true));
    assert_eq!(c.picks[3].1.get(), b"Picture 3");
    let kinds: std::vec::Vec<u8> = c.pick_at[..c.n_pick_at].iter().map(|t| t.1).collect();
    assert_eq!(
        kinds,
        [0, 1, 2, 3, 4, 5, 6, 7, MORE],
        "no REMOVE with none kept"
    );
    // In the D-pad's reach.
    assert_eq!(c.targets().count(), 9);
    // MORE asks from the ninth on.
    let more = c.pick_at[8].0;
    c.event(
        Event::PenUp {
            x: more.x + 5,
            y: more.y + 5,
        },
        &cx,
    );
    let ask = c.net_ask().unwrap();
    assert_eq!((ask.what, ask.arg.get()), (&b"PHOTOS"[..], &b"8"[..]));
    c.net_answer(
        true,
        b"8|Picture 8
",
        &cx,
    );
    draw(&mut c, &cx);
    assert_eq!((c.n_picks, c.more, c.picks[0].0), (1, false, 8));
    // Its row asks for the picture; the answer is kept and shown.
    let row = c.pick_at[0].0;
    c.event(
        Event::PenUp {
            x: row.x + 5,
            y: row.y + 5,
        },
        &cx,
    );
    let ask = c.net_ask().unwrap();
    assert_eq!((ask.what, ask.arg.get()), (&b"PHOTO"[..], &b"8"[..]));
    // No picture sent: said, nothing kept.
    c.net_answer(true, b"short", &cx);
    assert_eq!(c.view, View::Photo(0));
    assert_eq!(c.photo_note, b"The PC sent no picture");
    c.event(
        Event::PenUp {
            x: row.x + 5,
            y: row.y + 5,
        },
        &cx,
    );
    c.net_ask().unwrap();
    let mut raw = vec![0u8; photo::RAW];
    for (i, p) in raw.chunks_mut(2).enumerate() {
        let v: u16 = if (i % photo::SIDE) < 48 {
            0xF800
        } else {
            0x001F
        };
        p.copy_from_slice(&v.to_le_bytes());
    }
    c.net_answer(true, &raw, &cx);
    assert_eq!(c.view, View::Card(0), "back on the card");
    let pages = |pim: &core::cell::RefCell<Option<Store<'static>>>| {
        let g = pim.borrow();
        let s = g.as_ref().unwrap();
        s.all(Kind::Ink)
            .filter(|r| r.num(tag::PARENT) == Some(id))
            .count()
    };
    assert!(
        (3..=PHOTO_PAGES).contains(&pages(pim)),
        "{} pages",
        pages(pim)
    );
    let mut f = Box::new(Frame::new());
    c.draw(&mut f, &cx);
    assert!(c.has_photo[0]);
    let shown = c
        .faces
        .iter()
        .find(|x| x.0 == id && x.1 == 96)
        .and_then(|x| x.2)
        .expect("drawn");
    // Red on its left, blue on its right, clear at the corner.
    pics.read(shown, |px| {
        let at = |x: usize, y: usize| px[y * 96 + x];
        assert_eq!(at(0, 0), 0);
        let (l, r) = (at(24, 48), at(72, 48));
        assert!((l >> 16) & 255 > 200 && l & 255 < 60, "{l:08X}");
        assert!(r & 255 > 200 && (r >> 16) & 255 < 60, "{r:08X}");
    })
    .unwrap();
    // The list shows it too, at its own size.
    c.view = View::List;
    c.draw(&mut f, &cx);
    assert!(
        c.faces
            .iter()
            .any(|x| x.0 == id && x.1 == 40 && x.2.is_some())
    );
    // A second picture replaces the first (no pages left behind).
    let before = pages(pim);
    c.view = View::Card(0);
    c.draw(&mut f, &cx);
    c.open_photo(0);
    c.net_ask().unwrap();
    c.net_answer(
        true, b"0|One
", &cx,
    );
    draw(&mut c, &cx);
    let row = c.pick_at[0].0;
    c.event(
        Event::PenUp {
            x: row.x + 5,
            y: row.y + 5,
        },
        &cx,
    );
    c.net_ask().unwrap();
    c.net_answer(true, &raw, &cx);
    assert_eq!(pages(pim), before);
    // REMOVE takes it; the initials are back.
    c.draw(&mut f, &cx);
    c.open_photo(0);
    draw(&mut c, &cx);
    let remove = c.pick_at[..c.n_pick_at]
        .iter()
        .find(|t| t.1 == REMOVE)
        .expect("REMOVE")
        .0;
    c.event(
        Event::PenUp {
            x: remove.x + 5,
            y: remove.y + 5,
        },
        &cx,
    );
    assert_eq!(c.view, View::Card(0));
    assert_eq!(pages(pim), 0);
    c.draw(&mut f, &cx);
    assert!(!c.has_photo[0]);
    // A person deleted takes their picture along.
    c.open_photo(0);
    c.net_ask().unwrap();
    c.net_answer(
        true, b"0|One
", &cx,
    );
    draw(&mut c, &cx);
    let row = c.pick_at[0].0;
    c.event(
        Event::PenUp {
            x: row.x + 5,
            y: row.y + 5,
        },
        &cx,
    );
    c.net_ask().unwrap();
    c.net_answer(true, &raw, &cx);
    assert!(pages(pim) > 0);
    c.draw(&mut f, &cx);
    c.open_edit(0, &cx);
    assert!(c.delete(&cx));
    assert_eq!(pages(pim), 0);
    // No connection: said.
    let mut c = Contacts::new();
    {
        let mut g = pim.borrow_mut();
        g.as_mut()
            .unwrap()
            .put(Kind::Contact, None, &[(tag::FIRST, b"Ada")])
            .unwrap();
    }
    draw(&mut c, &cx);
    c.open_photo(0);
    c.net_ask().unwrap();
    c.net_answer(false, b"", &cx);
    assert_eq!(c.photo_note, b"No connection (Wi-Fi or the dock)");
}

// ---- Import, export, doubles and card pictures (io.rs) ----------------

fn with_pics(cx: &mut Ctx<'static>) {
    let classes = rp_app::kit::PIC_CLASSES;
    let pics: &'static rp_draw::pics::Pics<'static> =
        Box::leak(Box::new(rp_draw::pics::Pics::new(
            vec![0u32; rp_draw::pics::Pics::words(&classes)].leak(),
            &classes,
        )));
    cx.pics = Some(pics);
}

fn tap_rect(c: &mut Contacts, cx: &Ctx, r: Rect) -> Reply {
    c.event(
        Event::PenUp {
            x: r.x + r.w / 2,
            y: r.y + r.h / 2,
        },
        cx,
    )
}

fn base64(b: &[u8]) -> std::string::String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = std::string::String::new();
    for c in b.chunks(3) {
        let n = u32::from(c[0]) << 16
            | u32::from(*c.get(1).unwrap_or(&0)) << 8
            | u32::from(*c.get(2).unwrap_or(&0));
        for k in 0..4 {
            if k <= c.len() {
                s.push(A[(n >> (18 - 6 * k)) as usize & 63] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// A vCard file of three people, one with a picture, one the double of a
/// person already kept.
fn vcf() -> Vec<u8> {
    let jpeg = include_bytes!("../../../shell/pim/src/jpeg_fixtures/base420.jpg");
    let photo = base64(jpeg);
    let mut folded = std::string::String::new();
    for (k, ch) in photo.chars().enumerate() {
        if k > 0 && k % 72 == 0 {
            folded.push_str("\r\n ");
        }
        folded.push(ch);
    }
    std::format!(
        "BEGIN:VCARD\r\nVERSION:3.0\r\nN:Lovelace;Ada;;;\r\nFN:Ada Lovelace\r\nTEL;TYPE=CELL:(206) 555-0142\r\nitem1.TEL:(360) 555-0199\r\nitem1.X-ABLabel:Cabin\r\nBDAY:1815-12-10\r\nPHOTO;ENCODING=b;TYPE=JPEG:{folded}\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:4.0\r\nFN:Grace Hopper\r\nEMAIL:grace@example.org\r\nADR;TYPE=work:;;2100 Clarendon Blvd;Arlington;VA;22201;\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:2.1\r\nN:Babbage;Charles\r\nTEL;WORK:+44 20 7946 0958\r\nEND:VCARD\r\n"
    )
    .into_bytes()
}

#[test]
fn a_vcard_file_is_imported_with_its_picture_and_its_double_offered_as_one() {
    let pim = store();
    // Grace is already kept, with the same e-mail.
    pim.borrow_mut()
        .as_mut()
        .unwrap()
        .put(
            Kind::Contact,
            None,
            &[
                (tag::FIRST, b"Grace"),
                (tag::LAST, b"Hopper"),
                (tag::MOBILE, b"+17035550107"),
                (tag::EMAIL, b"Grace@Example.org"),
                (tag::FAVOURITE, &1u32.to_le_bytes()),
            ],
        )
        .unwrap();
    let mut cx = ctx(pim);
    with_pics(&mut cx);
    let mut c = Contacts::new();
    draw(&mut c, &cx);
    // The tools, from the key beside the search.
    tap_rect(&mut c, &cx, io::TOOLS_KEY);
    assert_eq!(c.view, View::Tools);
    draw(&mut c, &cx);
    assert_eq!(c.io.n_pairs, 0);
    let import = c.io_tool_for_test(0);
    tap_rect(&mut c, &cx, import);
    assert_eq!(c.view, View::Files);
    let ask = c.card_ask().expect("the card asked");
    assert_eq!((ask.what, ask.arg.get()), (&b"CARDFILES"[..], &b"vcf"[..]));
    c.net_answer(true, b"0|people.vcf\n", &cx);
    let ask = c.net_ask().expect("the PC asked");
    assert_eq!((ask.what, ask.arg.get()), (&b"FILES"[..], &b"vcf"[..]));
    c.net_answer(true, b"0|From Outlook.vcf\n", &cx);
    draw(&mut c, &cx);
    // The PC's file chosen.
    let row = c.io_row_for_test(1).expect("two rows");
    tap_rect(&mut c, &cx, row);
    let ask = c.net_ask().expect("the PC's file asked");
    assert_eq!((ask.what, ask.arg.get()), (&b"FILE"[..], &b"0 vcf"[..]));
    let file = vcf();
    c.net_answer(true, &file, &cx);
    // Three people kept, Ada's picture with her; the double offered.
    assert_eq!(c.view, View::Merge);
    assert_eq!(c.io.status.get(), b"Imported 3 people, 1 pictures");
    {
        let g = pim.borrow();
        let s = g.as_ref().unwrap();
        assert_eq!(s.all(Kind::Contact).count(), 4);
        let ada = s
            .all(Kind::Contact)
            .find(|r| r.text(tag::FIRST) == b"Ada")
            .unwrap();
        assert_eq!(ada.text(tag::MOBILE), b"+12065550142");
        assert!(
            s.all(Kind::Ink).any(|r| r.num(tag::PARENT) == Some(ada.id)),
            "Ada's picture kept"
        );
        let cb = s
            .all(Kind::Contact)
            .find(|r| r.text(tag::LAST) == b"Babbage")
            .unwrap();
        assert_eq!(cb.text(tag::WORK), b"+442079460958");
    }
    draw(&mut c, &cx);
    // Merged: one Grace, with the kept one's number and favourite and the
    // file's address.
    let key = c.io_merge_key_for_test(true);
    tap_rect(&mut c, &cx, key);
    assert_eq!(c.view, View::Tools, "no double left");
    let g = pim.borrow();
    let s = g.as_ref().unwrap();
    let graces: Vec<_> = s
        .all(Kind::Contact)
        .filter(|r| r.text(tag::FIRST) == b"Grace")
        .collect();
    assert_eq!(graces.len(), 1);
    let gr = graces[0];
    assert_eq!(gr.text(tag::MOBILE), b"+17035550107");
    assert_eq!(gr.num(tag::FAVOURITE), Some(1));
    assert_eq!(
        rp_pim::contact::Address::of(gr.text(tag::ADDRESS)).city,
        b"Arlington"
    );
}

#[test]
fn keep_both_is_remembered_and_never_asked_again() {
    let pim = store();
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        for first in [&b"Sam"[..], b"Samantha"] {
            s.put(
                Kind::Contact,
                None,
                &[(tag::FIRST, first), (tag::HOME, b"+12065550100")],
            )
            .unwrap();
        }
    }
    let cx = ctx(pim);
    let mut c = Contacts::new();
    draw(&mut c, &cx);
    tap_rect(&mut c, &cx, io::TOOLS_KEY);
    draw(&mut c, &cx);
    assert_eq!(c.io.n_pairs, 1, "a shared home number");
    let doubles = c.io_tool_for_test(5);
    tap_rect(&mut c, &cx, doubles);
    assert_eq!(c.view, View::Merge);
    draw(&mut c, &cx);
    let key = c.io_merge_key_for_test(false);
    tap_rect(&mut c, &cx, key);
    assert_eq!(c.view, View::Tools);
    assert_eq!(c.io.n_pairs, 0);
    // Opened again: still none to ask about; both kept.
    c.go(View::List);
    tap_rect(&mut c, &cx, io::TOOLS_KEY);
    assert_eq!(c.io.n_pairs, 0);
    assert_eq!(pim.borrow().as_ref().unwrap().all(Kind::Contact).count(), 2);
}

#[test]
fn everyone_is_exported_a_piece_at_a_time_and_reads_back() {
    let pim = store();
    let cx = ctx(pim);
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        for k in 0..30u32 {
            let first = std::format!("Person{k}");
            let more = {
                let mut b = [0u8; 64];
                let n = rp_pim::contact::push_entry(
                    &mut b,
                    0,
                    rp_pim::contact::Entry {
                        kind: rp_pim::contact::Kind::Phone,
                        label: b"Cabin",
                        value: b"+13605550199",
                    },
                )
                .unwrap();
                b[..n].to_vec()
            };
            s.put(
                Kind::Contact,
                None,
                &[
                    (tag::FIRST, first.as_bytes()),
                    (tag::LAST, b"Example; with, escapes"),
                    (tag::MOBILE, b"+12065550142"),
                    (tag::MORE, &more),
                    (
                        tag::BIRTHDAY,
                        &rp_pim::contact::Day {
                            year: None,
                            month: 2,
                            day: 29,
                        }
                        .field()
                        .unwrap(),
                    ),
                ],
            )
            .unwrap();
        }
    }
    let mut c = Contacts::new();
    draw(&mut c, &cx);
    tap_rect(&mut c, &cx, io::TOOLS_KEY);
    draw(&mut c, &cx);
    for (k, v4) in [(4usize, true), (3, false)] {
        let fmt = c.io_tool_for_test(k);
        tap_rect(&mut c, &cx, fmt);
        assert_eq!(c.io.v4, v4);
    }
    draw(&mut c, &cx);
    let to_pc = c.io_tool_for_test(2);
    tap_rect(&mut c, &cx, to_pc);
    let ask = c.net_ask().expect("the PC asked to keep it");
    assert_eq!(ask.what, b"PUT");
    let arg = ask.arg.get();
    let sp = arg.iter().position(|&b| b == b' ').unwrap();
    let len: usize = std::str::from_utf8(&arg[..sp]).unwrap().parse().unwrap();
    assert_eq!(&arg[sp + 1..], b"Contacts 2024-01-01.vcf");
    let mut file = Vec::new();
    let mut piece = [0u8; 333];
    loop {
        let n = c.net_body(file.len(), &mut piece, &cx);
        if n == 0 {
            break;
        }
        file.extend_from_slice(&piece[..n]);
    }
    assert_eq!(file.len(), len);
    let mut card = rp_pim::vcard::Card::new();
    let mut back = Vec::new();
    rp_pim::vcard::each(&file, &mut card, |c| {
        back.push(*c);
        true
    });
    assert_eq!(back.len(), 30);
    assert!(
        back.iter()
            .all(|c| c.last.get() == b"Example; with, escapes")
    );
    assert_eq!(back[7].phones[1].label.get(), b"Cabin");
    assert_eq!(
        back[7].birthday.map(|d| (d.month, d.day, d.year)),
        Some((2, 29, None))
    );
    c.net_answer(true, b"", &cx);
    assert_eq!(
        c.io.status.get(),
        b"Saved in Documents\\RetroPane on your PC"
    );
}

#[test]
fn a_card_picture_is_read_in_pieces_into_a_person_s_picture() {
    let pim = store();
    pim.borrow_mut()
        .as_mut()
        .unwrap()
        .put(Kind::Contact, None, &[(tag::FIRST, b"Ada")])
        .unwrap();
    let mut cx = ctx(pim);
    with_pics(&mut cx);
    let mut c = Contacts::new();
    draw(&mut c, &cx);
    c.view = View::Card(0);
    draw(&mut c, &cx);
    let id = c.people[0].unwrap().id;
    // The avatar opens the chooser; the card is asked for its pictures.
    let face = c.card_at[0].0;
    tap_rect(&mut c, &cx, face);
    assert_eq!(c.view, View::Photo(0));
    let ask = c.card_ask().expect("the card's pictures asked");
    assert_eq!((ask.what, ask.arg.get()), (&b"CARDFILES"[..], &b"pic"[..]));
    c.net_answer(true, b"0|CAMERA.JPG\n", &cx);
    draw(&mut c, &cx);
    let row = c.pick_at[..c.n_pick_at]
        .iter()
        .find(|t| t.1 == CARD_PIC)
        .unwrap()
        .0;
    tap_rect(&mut c, &cx, row);
    // The camera's file in pieces of 4 KB, as the device would answer.
    let file = include_bytes!("../../../shell/pim/src/jpeg_fixtures/restart.jpg");
    let mut asks = 0;
    while let Some(ask) = c.card_ask() {
        asks += 1;
        let arg = std::string::String::from(std::str::from_utf8(ask.arg.get()).unwrap());
        let at: usize = arg.split(' ').nth(2).unwrap().parse().unwrap();
        let end = (at + 4096).min(file.len());
        let mut body = (file.len() as u32).to_le_bytes().to_vec();
        body.extend_from_slice(&file[at..end]);
        c.net_answer(true, &body, &cx);
        assert!(asks < 100);
    }
    assert!(asks > 1, "read in pieces");
    assert_eq!(c.view, View::Card(0), "kept: back on the card");
    let g = pim.borrow();
    assert!(
        g.as_ref()
            .unwrap()
            .all(Kind::Ink)
            .any(|r| r.num(tag::PARENT) == Some(id))
    );
}

#[test]
fn a_card_shows_extras_days_and_website_and_turns_its_page() {
    let pim = store();
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        let mut more = [0u8; 400];
        let mut n = 0;
        for (kind, label, value) in [
            (
                rp_pim::contact::Kind::Phone,
                &b"Cabin"[..],
                &b"+13605550199"[..],
            ),
            (rp_pim::contact::Kind::Phone, b"", b"+13605550100"),
            (
                rp_pim::contact::Kind::Email,
                b"School",
                b"ada@school.example",
            ),
        ] {
            n = rp_pim::contact::push_entry(
                &mut more,
                n,
                rp_pim::contact::Entry { kind, label, value },
            )
            .unwrap();
        }
        let bday = rp_pim::contact::Day {
            year: Some(1815),
            month: 12,
            day: 10,
        }
        .field()
        .unwrap();
        s.put(
            Kind::Contact,
            None,
            &[
                (tag::FIRST, b"Ada"),
                (tag::LAST, b"Lovelace"),
                (tag::JOB, b"Programmer"),
                (tag::COMPANY, b"Analytical Engines"),
                (tag::MOBILE, b"+12065550142"),
                (tag::HOME, b"+12065550143"),
                (tag::WORK, b"+12065550144"),
                (tag::EMAIL, b"ada@example.com"),
                (tag::MORE, &more[..n]),
                (tag::ADDRESS, b"12 St James Sq\nSeattle\nWA\n98101\n"),
                (tag::BIRTHDAY, &bday),
                (tag::WEBSITE, b"ada.example"),
                (tag::NOTE, b"Met at the Royal Society\nmore"),
            ],
        )
        .unwrap();
    }
    let cx = ctx(pim);
    let mut c = Contacts::new();
    draw(&mut c, &cx);
    c.view = View::Card(0);
    draw(&mut c, &cx);
    let lines: Vec<(Vec<u8>, Vec<u8>, Vec<u8>)> = c.details[..c.n_details]
        .iter()
        .map(|d| {
            (
                d.label.get().to_vec(),
                d.value.get().to_vec(),
                d.more.get().to_vec(),
            )
        })
        .collect();
    let want: [(&[u8], &[u8], &[u8]); 11] = [
        (b"MOBILE", b"(206) 555-0142", b""),
        (b"HOME", b"(206) 555-0143", b""),
        (b"WORK", b"(206) 555-0144", b""),
        (b"CABIN", b"(360) 555-0199", b""),
        (b"OTHER", b"(360) 555-0100", b""),
        (b"E-MAIL", b"ada@example.com", b""),
        (b"SCHOOL", b"ada@school.example", b""),
        (b"HOME", b"12 St James Sq", b"Seattle, WA 98101"),
        // On 1 January 2024: 209 at the next one.
        (b"BIRTHDAY", b"December 10, 1815", b"turns 209 next"),
        (b"WEBSITE", b"ada.example", b""),
        (b"NOTE", b"Met at the Royal Society", b""),
    ];
    for (k, w) in want.iter().enumerate() {
        assert_eq!(
            (&lines[k].0[..], &lines[k].1[..], &lines[k].2[..]),
            *w,
            "line {k}"
        );
    }
    // More than fit: the rest on the next page, by the D-pad past the
    // last or a drag.
    assert!(c.card_more);
    let first_page = c.card_at[..c.n_card].iter().filter(|t| t.1 >= 10).count();
    assert!(first_page < 11);
    for _ in 0..20 {
        c.event(Event::Key(Key::Down), &cx);
        draw(&mut c, &cx);
        if c.card_first > 0 {
            break;
        }
    }
    assert_eq!(usize::from(c.card_first), first_page);
    // On to the page with the note.
    for _ in 0..20 {
        if c.card_at[..c.n_card].iter().any(|t| t.1 == 10 + 10) {
            break;
        }
        c.event(Event::Key(Key::Down), &cx);
        draw(&mut c, &cx);
    }
    assert!(
        c.card_at[..c.n_card].iter().any(|t| t.1 == 10 + 10),
        "the note reached"
    );
    // A drag down: the first page again.
    c.event(Event::PenDown { x: 200, y: 400 }, &cx);
    c.event(Event::PenUp { x: 200, y: 520 }, &cx);
    assert_eq!(c.card_first, 0);
}

#[test]
fn made_in_its_slot_it_is_the_same_app() {
    let pim = store();
    people(pim);
    let cx = ctx(pim);
    let mut slot = Box::new(core::mem::MaybeUninit::<Contacts>::uninit());
    let c = opened_in(&mut slot, 0);
    draw(c, &cx);
    let mut d = Contacts::new();
    draw(&mut d, &cx);
    assert_eq!(c.view, d.view);
    assert_eq!(c.n_rows, d.n_rows);
    assert!(c.n_rows > 0);
}
