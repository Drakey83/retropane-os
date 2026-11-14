//! Calendar driven as the stylus drives it, against a real store.

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

fn draw(c: &mut Calendar, cx: &Ctx) {
    let mut f = Box::new(Frame::new());
    c.draw(&mut f, cx);
}

#[test]
fn the_us_week_starts_on_sunday_and_others_on_monday() {
    // 7 January 2024 was a Sunday, 8 a Monday.
    assert_eq!(in_week(19_729, true), 0);
    assert_eq!(in_week(19_730, true), 1);
    assert_eq!(in_week(19_730, false), 0);
    assert_eq!(column_name(0, true), b"SUN");
    assert_eq!(column_name(0, false), b"MON");
}

#[test]
fn tasks_due_show_on_their_days_and_hidden_calendars_stay_out() {
    let pim = store();
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        let one = 1u32.to_le_bytes();
        let hid = s
            .put(
                Kind::Calendar,
                None,
                &[(tag::NAME, b"Old work"), (tag::HIDDEN, &one)],
            )
            .unwrap();
        let shown = s
            .put(Kind::Calendar, None, &[(tag::NAME, b"Home")])
            .unwrap();
        let at = (date::start_of(19_723) + 600).to_le_bytes();
        s.put(
            Kind::Appointment,
            None,
            &[
                (tag::TITLE, b"Secret"),
                (tag::START, &at),
                (tag::CAL, &hid.to_le_bytes()),
            ],
        )
        .unwrap();
        s.put(
            Kind::Appointment,
            None,
            &[
                (tag::TITLE, b"Dentist"),
                (tag::START, &at),
                (tag::CAL, &shown.to_le_bytes()),
            ],
        )
        .unwrap();
        let due = 19_723u32.to_le_bytes();
        s.put(
            Kind::Task,
            None,
            &[(tag::TITLE, b"Pay rent"), (tag::DUE, &due)],
        )
        .unwrap();
    }
    let cx = ctx(pim);
    let mut c = opened(19_723);
    draw(&mut c, &cx);
    let (list, n) = c.on(19_723);
    let titles: std::vec::Vec<&[u8]> = list[..n].iter().map(|a| a.title.get()).collect();
    assert!(titles.contains(&&b"Dentist"[..]));
    assert!(!titles.contains(&&b"Secret"[..]));
    assert!(
        list[..n]
            .iter()
            .any(|a| a.task && a.title.get() == b"Pay rent")
    );
}

#[test]
fn the_editor_changes_start_repeat_and_alert_then_deletes_on_the_second_tap() {
    let pim = store();
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        let at = (date::start_of(19_723) + 15 * 60).to_le_bytes();
        s.put(
            Kind::Appointment,
            None,
            &[(tag::TITLE, b"Dentist"), (tag::START, &at)],
        )
        .unwrap();
    }
    let cx = ctx(pim);
    let mut c = opened(19_723);
    draw(&mut c, &cx);
    let (r, _, _) = c.rows_at[0];
    c.event(
        Event::PenUp {
            x: r.x + 100,
            y: r.y + 20,
        },
        &cx,
    );
    assert!(matches!(c.view, View::Edit));
    // Start a quarter later (the right key), weekly, an hour's alert.
    c.event(
        Event::PenUp {
            x: W - 40,
            y: E_START + 20,
        },
        &cx,
    );
    c.event(
        Event::PenUp {
            x: MARGIN + WIDE * 2 / 5 + 10,
            y: E_REPEAT + 24,
        },
        &cx,
    );
    c.event(
        Event::PenUp {
            x: MARGIN + WIDE * 3 / 5 + 10,
            y: E_ALERT + 24,
        },
        &cx,
    );
    c.event(Event::Key(Key::SoftRight), &cx);
    assert!(matches!(c.view, View::Agenda));
    {
        let g = pim.borrow();
        let s = g.as_ref().unwrap();
        let r = s.all(Kind::Appointment).next().unwrap();
        assert_eq!(
            r.num(tag::START),
            Some(date::start_of(19_723) + 15 * 60 + 15)
        );
        let rule = rp_pim::repeat::Rule::unpack(r.num(tag::REPEAT).unwrap(), 19_723).unwrap();
        assert_eq!(rule.every, rp_pim::repeat::Every::Week);
        assert_eq!(r.num(tag::REMIND), Some(60));
    }
    // Delete asks in place: the first tap only asks.
    draw(&mut c, &cx);
    let (r, _, _) = c.rows_at[0];
    c.event(
        Event::PenUp {
            x: r.x + 100,
            y: r.y + 20,
        },
        &cx,
    );
    c.event(
        Event::PenUp {
            x: 200,
            y: E_DELETE + 20,
        },
        &cx,
    );
    assert_eq!(
        pim.borrow()
            .as_ref()
            .unwrap()
            .all(Kind::Appointment)
            .count(),
        1
    );
    c.event(
        Event::PenUp {
            x: 200,
            y: E_DELETE + 20,
        },
        &cx,
    );
    assert_eq!(
        pim.borrow()
            .as_ref()
            .unwrap()
            .all(Kind::Appointment)
            .count(),
        0
    );
}
