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

#[test]
fn every_field_is_typed_and_a_kept_person_is_changed_and_deleted() {
    // The first editor typed a first name, a mobile and a street; its Last
    // and City rows took no typing, and a kept person could not be changed
    // or deleted (the card's Edit softkey did nothing).
    let pim = store();
    let cx = ctx(pim);
    let mut c = Contacts::new();
    draw(&mut c, &cx);
    // New: every field in turn (the keyboard's Done moves on), then the
    // favourite switch by the D-pad, then the softkey keeps it. Done with
    // no name says so and keeps nothing.
    c.event(Event::Key(Key::SoftRight), &cx);
    c.event(Event::Key(Key::Down), &cx);
    c.event(Event::Key(Key::SoftRight), &cx);
    assert!(c.need_name && c.view == View::Edit(EditField::First));
    assert_eq!(pim.borrow().as_ref().unwrap().all(Kind::Contact).count(), 0);
    for (k, text) in [
        &b"Ada"[..],
        b"Lovelace",
        b"Analytical Engines",
        b"2065550142",
        b"2065550143",
        b"2065550144",
        b"ada@example.com",
        b"12 St James Sq",
        b"Seattle, WA 98101",
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(c.view, View::Edit(c.fields().0[k]));
        draw(&mut c, &cx);
        assert!(c.keyboard().is_some());
        type_in(&mut c, &cx, text);
        c.event(Event::Key(Key::Down), &cx);
    }
    assert_eq!(c.view, View::Edit(EditField::Favourite));
    assert!(c.keyboard().is_none());
    c.event(Event::Key(Key::Enter), &cx);
    // No Delete row for a person not kept yet.
    assert_eq!(c.event(Event::Key(Key::Down), &cx), Reply::Nothing);
    c.event(Event::Key(Key::SoftRight), &cx);
    assert_eq!(c.view, View::List);
    let (id, f) = kept(pim, b"Ada").expect("kept");
    let get = |f: &Fields, t: u8| f.iter().find(|x| x.0 == t).unwrap().1.clone();
    assert_eq!(get(&f, tag::LAST), b"Lovelace");
    assert_eq!(get(&f, tag::COMPANY), b"Analytical Engines");
    assert_eq!(get(&f, tag::MOBILE), b"+12065550142");
    assert_eq!(get(&f, tag::HOME), b"+12065550143");
    assert_eq!(get(&f, tag::WORK), b"+12065550144");
    assert_eq!(get(&f, tag::EMAIL), b"ada@example.com");
    assert_eq!(get(&f, tag::ADDRESS), b"12 St James Sq\nSeattle, WA 98101");
    assert_eq!(get(&f, tag::FAVOURITE), 1u32.to_le_bytes());
    // A field the form does not show is kept through a change.
    pim.borrow_mut()
        .as_mut()
        .unwrap()
        .change(id, &[(tag::BIRTHDAY, &7u32.to_le_bytes())])
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
    assert_eq!(c.view, View::Edit(EditField::First));
    assert_eq!(c.val(EditField::Mobile), b"+12065550142");
    assert_eq!(c.val(EditField::City), b"Seattle, WA 98101");
    assert!(c.fav);
    c.event(Event::Key(Key::Down), &cx);
    for _ in 0..8 {
        c.event(Event::Delete, &cx);
    }
    type_in(&mut c, &cx, b"King");
    for _ in 0..4 {
        c.event(Event::Key(Key::Down), &cx);
    }
    assert_eq!(c.view, View::Edit(EditField::Work));
    for _ in 0..12 {
        c.event(Event::Delete, &cx);
    }
    // A tap on another row types that one from then on.
    draw(&mut c, &cx);
    let (r, fld) = c.form_at[c.n_form - 1];
    assert_ne!(fld, EditField::Work);
    c.event(
        Event::PenDown {
            x: r.x + 9,
            y: r.y + 9,
        },
        &cx,
    );
    c.event(
        Event::PenUp {
            x: r.x + 9,
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
    assert_eq!(get(&f, tag::ADDRESS), b"12 St James Sq\nSeattle, WA 98101");
    assert_eq!(get(&f, tag::FAVOURITE), 1u32.to_le_bytes());
    assert_eq!(get(&f, tag::BIRTHDAY), 7u32.to_le_bytes());
    // Deleted: asked once, then gone; a move away takes the question back.
    draw(&mut c, &cx);
    c.view = card_of(&c);
    c.event(Event::Key(Key::SoftRight), &cx);
    for _ in 0..10 {
        c.event(Event::Key(Key::Down), &cx);
    }
    assert_eq!(c.view, View::Edit(EditField::Delete));
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
    assert_eq!(kinds, [PICTURE, MAP_KEY, 5, 0, 1, 2, 3, 4]);
    // (Past the avatar, which opens the picture chooser.)
    c.event(Event::Key(Key::Down), &cx);
    assert!(c.card_at[..c.n_card].iter().all(|(r, _)| r.bottom() <= 596));
    // The D-pad reaches each, the centre copies it; the mark shows, then
    // goes.
    let mut seen = std::vec::Vec::new();
    let mut mapped = false;
    for _ in 0..9 {
        c.event(Event::Key(Key::Down), &cx);
        let focus = c.focus;
        let r = c.event(Event::Key(Key::Enter), &cx);
        if let View::Map(i) = c.view {
            // The Map key opens the address's map; Back returns to the card
            // with the D-pad where it was.
            mapped = true;
            c.event(Event::Key(Key::SoftLeft), &cx);
            assert_eq!(c.view, View::Card(i));
            draw(&mut c, &cx);
            c.focus = focus;
            continue;
        }
        assert_eq!(r, Reply::Animate, "{:?} {seen:?}", c.focus.rect());
        seen.push(std::string::String::from_utf8_lossy(cx.clip.borrow().get()).into_owned());
    }
    assert!(mapped, "the Map key reached by the D-pad");
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
        for _ in 0..7 {
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
        assert_eq!(
            c.val(EditField::City),
            b"Boulder, Colorado 80302",
            "strip {by_strip}"
        );
        // And kept so.
        draw(&mut c, &cx);
        c.event(Event::Done, &cx);
        let (_, f) = kept(pim, b"Ada").expect("kept");
        let addr = &f.iter().find(|x| x.0 == tag::ADDRESS).unwrap().1;
        assert_eq!(addr, b"410 Pine Street\nBoulder, Colorado 80302");
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
    c.open_edit(0);
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
