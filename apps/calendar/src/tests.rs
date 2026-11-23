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

/// A motion's frames drawn to its landing (the shell's ticks), then the
/// still frame after: how many frames moved.
fn land(c: &mut Calendar, cx: &Ctx) -> u8 {
    let mut n = 0;
    draw(c, cx);
    while c.event(Event::Tick, cx) == Reply::Animate {
        n += 1;
        draw(c, cx);
        assert!(n < 20, "a motion that never lands");
    }
    draw(c, cx);
    n
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
    c.show(View::Agenda);
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
            Some(date::start_of(19_723) + 15 * 60 + 5)
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

#[test]
fn every_tab_day_and_column_answers_the_stylus() {
    // The owner, 2026-11-14: "i should be able to click on everything
    // (agenda, day, week, month) including individual days".
    let pim = store();
    let cx = ctx(pim);
    let mut c = opened(19_723);
    draw(&mut c, &cx);
    let tap = |c: &mut Calendar, h: Hit| {
        let r = c.hits.find(h).expect("drawn");
        c.event(
            Event::PenUp {
                x: r.x + r.w / 2,
                y: r.y + r.h / 2,
            },
            &cx,
        )
    };
    // (Between day, week and month the change is a zoom: its frames run
    // to the end before the next tap.)
    for (v, want) in [
        (View::Day, Reply::Redraw),
        (View::Week, Reply::Animate),
        (View::Month, Reply::Animate),
        (View::Agenda, Reply::Redraw),
    ] {
        assert_eq!(tap(&mut c, Hit::View(v)), want);
        assert_eq!(c.view, v);
        land(&mut c, &cx);
    }
    // A day in the week strip.
    tap(&mut c, Hit::Day(19_725));
    assert_eq!(c.day, 19_725);
    // A slim column in Week moves the lens to its day; the wide one
    // opens its day.
    c.view = View::Week;
    draw(&mut c, &cx);
    tap(&mut c, Hit::Day(19_724));
    assert_eq!((c.view, c.day), (View::Week, 19_724));
    draw(&mut c, &cx);
    let lens = c.hits.find(Hit::OpenDay(19_724)).expect("the lens");
    assert!(lens.w > 150, "{lens:?}");
    assert_eq!(tap(&mut c, Hit::OpenDay(19_724)), Reply::Animate);
    assert_eq!((c.view, c.day), (View::Day, 19_724));
    land(&mut c, &cx);
    // A day in Month.
    c.view = View::Month;
    draw(&mut c, &cx);
    tap(&mut c, Hit::Day(19_740));
    assert_eq!((c.view, c.day), (View::Month, 19_740));
    // Room for the busiest screen.
    assert!(c.hits.len() < 96);
}

#[test]
fn the_board_shows_what_is_next_and_everything_on_it_opens() {
    // Design language version 3: Calendar opens on the day's board.
    let pim = store();
    {
        let mut g = pim.borrow_mut();
        let st = g.as_mut().unwrap();
        // 3:00 PM on the day the context calls today.
        let at = (date::start_of(19_723) + 15 * 60).to_le_bytes();
        st.put(
            Kind::Appointment,
            None,
            &[(tag::TITLE, b"Dentist"), (tag::START, &at)],
        )
        .unwrap();
    }
    let cx = ctx(pim);
    let mut c = opened(19_723);
    assert_eq!(c.view, View::Home);
    draw(&mut c, &cx);
    // The ribbon's block, the card and its key all open the appointment.
    let id = match c.hits.find(Hit::View(View::Day)) {
        Some(_) => c.on(19_723).0[0].id,
        None => panic!("the board's DAY VIEW key isn't drawn"),
    };
    let r = c
        .hits
        .find(Hit::Appt(id))
        .expect("the next appointment is a target");
    c.event(
        Event::PenUp {
            x: r.x + r.w / 2,
            y: r.y + r.h / 2,
        },
        &cx,
    );
    assert_eq!(c.view, View::Edit);
    // The D-pad: with nothing focused the centre opens what is next, and
    // left and right change the day.
    let mut c = opened(19_723);
    draw(&mut c, &cx);
    assert_eq!(c.event(Event::Key(Key::Enter), &cx), Reply::Redraw);
    assert_eq!(c.view, View::Edit);
    let mut c = opened(19_723);
    draw(&mut c, &cx);
    c.event(Event::Key(Key::Right), &cx);
    assert_eq!(c.day, 19_724);
    c.event(Event::Key(Key::Left), &cx);
    // Up goes to the views' tabs; there left and right choose the view.
    c.event(Event::Key(Key::Up), &cx);
    assert_eq!(
        c.focus.point(),
        kit::Focus::at(kit::foot_tab(TABS.len(), 0))
    );
    for want in [View::Agenda, View::Day, View::Week, View::Month] {
        c.event(Event::Key(Key::Right), &cx);
        draw(&mut c, &cx);
        assert_eq!(c.view, want);
    }
    assert_eq!(c.day, 19_723);
    // In Month the arrows walk the days: up from the tabs (past the
    // chosen day's list) to a day of the last week, up again a week
    // earlier, left the day before, each chosen as it is reached.
    for _ in 0..6 {
        c.event(Event::Key(Key::Up), &cx);
        draw(&mut c, &cx);
        if c.day != 19_723 {
            break;
        }
    }
    let last = c.day;
    assert_ne!(last, 19_723, "no day reached");
    c.event(Event::Key(Key::Up), &cx);
    draw(&mut c, &cx);
    assert_eq!(c.day, last - 7);
    c.event(Event::Key(Key::Left), &cx);
    draw(&mut c, &cx);
    assert_eq!(c.day, last - 8);
    // Every target of every view is within the arrows' reach of its tab
    // (the class: nothing a stylus can tap is out of the D-pad's).
    for (i, v) in TABS.into_iter().enumerate() {
        let mut c = opened(19_723);
        c.view = v;
        draw(&mut c, &cx);
        let all: std::vec::Vec<Rect> = c.targets().collect();
        let mut seen = std::vec![kit::foot_tab(TABS.len(), i)];
        let mut k0 = 0;
        while k0 < seen.len() {
            for k in [Key::Up, Key::Down, Key::Left, Key::Right] {
                let mut f = kit::Focus::default();
                f.set(seen[k0]);
                if f.step(k, all.iter().copied())
                    && let Some(r) = f.rect()
                    && !seen.contains(&r)
                {
                    seen.push(r);
                }
            }
            k0 += 1;
        }
        for r in &all {
            assert!(seen.contains(r), "{v:?}: no way to {r:?}");
        }
    }
    // The centre taps the focused appointment (Agenda's first, down from
    // whatever leads the screen).
    let mut c = opened(19_723);
    c.view = View::Agenda;
    draw(&mut c, &cx);
    for _ in 0..12 {
        c.event(Event::Key(Key::Down), &cx);
        draw(&mut c, &cx);
        let on = c.focus.point().and_then(|(x, y)| c.hits.at(x, y));
        if matches!(on, Some(Hit::Appt(_))) {
            break;
        }
    }
    assert_eq!(c.view, View::Agenda);
    c.event(Event::Key(Key::Enter), &cx);
    assert_eq!(c.view, View::Edit);
    // A day of the week strip shows that day's board; New rises over it.
    let mut c = opened(19_723);
    draw(&mut c, &cx);
    let r = c
        .hits
        .find(Hit::Day(19_725))
        .expect("the week's days are targets");
    c.event(
        Event::PenUp {
            x: r.x + 4,
            y: r.y + 4,
        },
        &cx,
    );
    assert_eq!((c.view, c.day), (View::Home, 19_725));
    draw(&mut c, &cx);
    assert!(c.hits.len() < 96);
}

#[test]
fn the_dpad_works_the_editor_the_calendars_and_the_quick_add_sheet() {
    // R203, the screens the views' walk left out.
    let pim = store();
    {
        let mut g = pim.borrow_mut();
        let st = g.as_mut().unwrap();
        // 3:00 PM on the day the context calls today, an hour long.
        let at = (date::start_of(19_723) + 15 * 60).to_le_bytes();
        st.put(
            Kind::Appointment,
            None,
            &[
                (tag::TITLE, b"Dentist"),
                (tag::START, &at),
                (tag::MINUTES, &60u32.to_le_bytes()),
            ],
        )
        .unwrap();
        st.put(Kind::Calendar, None, &[(tag::NAME, b"Home")])
            .unwrap();
    }
    let cx = ctx(pim);
    let mut c = opened(19_723);
    draw(&mut c, &cx);
    // The editor on what is next: down to the day, right a day later;
    // the start a step later; the repeat to the chip beside; up to the
    // title, the centre types in it.
    assert_eq!(c.event(Event::Key(Key::Enter), &cx), Reply::Redraw);
    assert_eq!(c.view, View::Edit);
    let was = c.edit.unwrap();
    for _ in 0..3 {
        c.event(Event::Key(Key::Down), &cx);
    }
    c.event(Event::Key(Key::Right), &cx);
    c.event(Event::Key(Key::Down), &cx);
    c.event(Event::Key(Key::Right), &cx);
    c.event(Event::Key(Key::Down), &cx);
    c.event(Event::Key(Key::Left), &cx);
    c.event(Event::Key(Key::Down), &cx);
    c.event(Event::Key(Key::Right), &cx);
    c.event(Event::Key(Key::Right), &cx);
    c.event(Event::Key(Key::Left), &cx);
    let now = c.edit.unwrap();
    assert_eq!(now.day, was.day + 1);
    assert_eq!(now.start, was.start + 5);
    assert!(now.len < was.len);
    assert_eq!(now.repeat, was.repeat + 1);
    for _ in 0..9 {
        c.event(Event::Key(Key::Up), &cx);
    }
    assert_eq!(c.edit.unwrap().row, Some(0));
    c.event(Event::Key(Key::Enter), &cx);
    assert_eq!(c.edit.unwrap().typing, Some(EditText::Title));
    draw(&mut c, &cx);
    // Down to Delete: asked once (a move away takes it back), then gone.
    let id = now.id.expect("a kept appointment");
    for _ in 0..9 {
        c.event(Event::Key(Key::Down), &cx);
    }
    assert_eq!(c.edit.unwrap().row, Some(8));
    c.event(Event::Key(Key::Enter), &cx);
    assert!(c.edit.unwrap().confirm_delete);
    c.event(Event::Key(Key::Up), &cx);
    c.event(Event::Key(Key::Down), &cx);
    assert!(!c.edit.unwrap().confirm_delete);
    c.event(Event::Key(Key::Enter), &cx);
    c.event(Event::Key(Key::Enter), &cx);
    assert!(pim.borrow().as_ref().unwrap().get(id).is_none());
    // The calendars' list: its rows and the key that asks the PC are in
    // reach; the centre hides a calendar as a tap does.
    let mut c = opened(19_723);
    c.view = View::Calendars;
    draw(&mut c, &cx);
    assert!(c.n_cal_rows > 0);
    let hidden = |c: &Calendar| c.cals[..c.n_cals].iter().filter(|k| k.hidden).count();
    c.event(Event::Key(Key::Down), &cx);
    assert_eq!(c.focus.rect(), Some(c.cal_rows[0].0));
    let before = hidden(&c);
    c.event(Event::Key(Key::Enter), &cx);
    draw(&mut c, &cx);
    assert_eq!(hidden(&c), before + 1);
    for _ in 0..8 {
        c.event(Event::Key(Key::Down), &cx);
    }
    assert_eq!(c.focus.rect(), Some(GET_KEY));
    let day = c.day;
    c.event(Event::Key(Key::Left), &cx);
    assert_eq!(c.day, day, "no days to step in the calendars' list");
    // The quick-add sheet: down to its way into the full editor.
    let mut c = opened(19_723);
    c.view = View::New;
    draw(&mut c, &cx);
    c.event(Event::Key(Key::Down), &cx);
    c.event(Event::Key(Key::Enter), &cx);
    assert_eq!(c.view, View::Edit);
}

#[test]
fn the_calendar_button_steps_through_the_views() {
    let cx = ctx(store());
    let mut c = opened(19_723);
    for (want, reply) in [
        (View::Agenda, Reply::Redraw),
        (View::Day, Reply::Redraw),
        (View::Week, Reply::Animate),
        (View::Month, Reply::Animate),
        (View::Home, Reply::Redraw),
    ] {
        assert_eq!(c.event(Event::Key(Key::App), &cx), reply);
        assert_eq!(c.view, want);
        land(&mut c, &cx);
    }
    // From a screen that is no view: the first.
    c.view = View::Calendars;
    c.event(Event::Key(Key::App), &cx);
    assert_eq!(c.view, View::Home);
}

#[test]
fn search_finds_appointments_of_any_day_by_title_or_place() {
    let pim = store();
    {
        let mut g = pim.borrow_mut();
        let st = g.as_mut().unwrap();
        // Far from the days the views hold: a year back, a year on.
        for (title, place, day) in [
            (&b"Dentist"[..], &b"Dr. Lee, 410 Pine St"[..], 19_723 - 365),
            (b"Lunch with Sam", b"Cafe Vita", 19_723),
            (b"Book club", b"Pine Street Library", 19_723 + 365),
            (b"Stand-up", b"Zoom", 19_724),
        ] {
            let at = (date::start_of(day) + 9 * 60).to_le_bytes();
            st.put(
                Kind::Appointment,
                None,
                &[(tag::TITLE, title), (tag::PLACE, place), (tag::START, &at)],
            )
            .unwrap();
        }
    }
    let cx = ctx(pim);
    let mut c = opened(19_723);
    c.view = View::Agenda;
    draw(&mut c, &cx);
    // Agenda's search field opens the search, the keyboard up; the D-pad
    // reaches it as it does every target.
    let field = c
        .hits
        .rects()
        .find(|r| c.hits.at(r.x + 5, r.y + 5) == Some(Hit::Find))
        .expect("a search field");
    c.event(
        Event::PenDown {
            x: field.x + 5,
            y: field.y + 5,
        },
        &cx,
    );
    c.event(
        Event::PenUp {
            x: field.x + 5,
            y: field.y + 5,
        },
        &cx,
    );
    assert_eq!(c.view, View::Find);
    assert_eq!(c.keyboard(), Some(Field::Line));
    for &ch in b"pine" {
        c.event(Event::Char(ch), &cx);
    }
    draw(&mut c, &cx);
    // A year back and a year on, the sooner first; by the place.
    assert_eq!(c.n_find, 2);
    let title = |id: u32| {
        let g = pim.borrow();
        g.as_ref()
            .unwrap()
            .get(id)
            .unwrap()
            .text(tag::TITLE)
            .to_vec()
    };
    assert_eq!(title(c.find_rows[0].1), b"Dentist");
    assert_eq!(title(c.find_rows[1].1), b"Book club");
    // The D-pad: down to the field, then the rows; the centre opens one.
    c.event(Event::Done, &cx);
    assert_eq!(c.keyboard(), None);
    c.event(Event::Key(Key::Down), &cx);
    c.event(Event::Key(Key::Down), &cx);
    c.event(Event::Key(Key::Down), &cx);
    assert_eq!(c.focus.rect(), Some(c.find_rows[1].0));
    c.event(Event::Key(Key::Enter), &cx);
    assert_eq!(c.view, View::Edit);
    assert_eq!(c.edit.unwrap().id, Some(c.find_rows[1].1));
    // Nothing typed, or nothing found: no rows.
    let mut c = opened(19_723);
    c.view = View::Find;
    draw(&mut c, &cx);
    assert_eq!(c.n_find, 0);
    for &ch in b"zzz" {
        c.event(Event::Char(ch), &cx);
    }
    draw(&mut c, &cx);
    assert_eq!((c.n_find, c.find_total), (0, 0));
    c.event(Event::Key(Key::SoftLeft), &cx);
    assert_eq!(c.view, View::Agenda);
}

#[test]
fn a_start_and_an_alert_are_set_to_the_minute() {
    // The owner on the X51v, 2026-11-15: "setting tasks and calendar
    // alerts by time should be able to be selected by the min not by
    // every 15 mins". A touch on the start's time brings the pad; the
    // sixth alert chip brings it for a lead of any length.
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
    c.show(View::Agenda);
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
    draw(&mut c, &cx);
    c.event(
        Event::PenUp {
            x: W / 2 + 40,
            y: E_START + 20,
        },
        &cx,
    );
    assert!(c.pad.is_some(), "the start's pad");
    assert_eq!(c.softkeys(), ("Cancel", "Set"));
    for ch in *b"307" {
        c.event(Event::Char(ch), &cx);
    }
    c.event(Event::Key(Key::SoftRight), &cx);
    assert!(c.pad.is_none());
    let e = c.edit.unwrap();
    assert_eq!(e.start % 60, 7, "to the minute");
    assert_eq!(e.start % 720, 3 * 60 + 7);
    // The alert: 7 minutes before, by the sixth chip.
    draw(&mut c, &cx);
    c.event(
        Event::PenUp {
            x: W - MARGIN - 20,
            y: E_ALERT + 24,
        },
        &cx,
    );
    assert!(c.pad.is_some(), "the alert's pad");
    c.event(Event::Char(b'7'), &cx);
    c.event(Event::Key(Key::SoftRight), &cx);
    let e = c.edit.unwrap();
    assert_eq!((e.alert, e.alert_minutes()), (5, Some(7)));
    draw(&mut c, &cx);
    // A lead that is a quick one lights its own chip.
    c.event(
        Event::PenUp {
            x: W - MARGIN - 20,
            y: E_ALERT + 24,
        },
        &cx,
    );
    c.event(Event::Char(b'1'), &cx);
    c.event(Event::Char(b'5'), &cx);
    c.event(Event::Key(Key::SoftRight), &cx);
    assert_eq!(c.edit.unwrap().alert, 2);
    c.event(
        Event::PenUp {
            x: W - MARGIN - 20,
            y: E_ALERT + 24,
        },
        &cx,
    );
    c.event(Event::Char(b'7'), &cx);
    c.event(Event::Key(Key::SoftRight), &cx);
    // Kept, and read back as it was set.
    c.event(Event::Key(Key::SoftRight), &cx);
    {
        let g = pim.borrow();
        let s = g.as_ref().unwrap();
        let r = s.all(Kind::Appointment).next().unwrap();
        assert_eq!(r.num(tag::REMIND), Some(7));
        assert_eq!(r.num(tag::START).unwrap() % 60, 7);
    }
    draw(&mut c, &cx);
    let (r, _, _) = c.rows_at[0];
    c.event(
        Event::PenUp {
            x: r.x + 100,
            y: r.y + 20,
        },
        &cx,
    );
    let e = c.edit.unwrap();
    assert_eq!((e.alert, e.alert_minutes()), (5, Some(7)));
}

#[test]
fn a_zoom_grows_out_of_the_very_cell_drawn() {
    // The cell each zoom starts from is where the view drew that day (one
    // geometry: a moved grid would zoom out of the wrong place).
    let cx = ctx(store());
    let mut c = opened(19_723);
    for day in [19_723, 19_740, 19_752, 19_754] {
        c.day = day;
        c.view = View::Month;
        draw(&mut c, &cx);
        let cell = c.hits.find(Hit::Day(day)).expect("the day drawn");
        assert_eq!(c.zoom_cell(View::Month, View::Day, true), cell, "{day}");
        let row = c.zoom_cell(View::Week, View::Month, true);
        assert_eq!((row.y, row.h, row.w), (cell.y, cell.h, WIDE));
        c.view = View::Week;
        draw(&mut c, &cx);
        let lens = c.hits.find(Hit::OpenDay(day)).expect("the lens");
        assert_eq!(c.zoom_cell(View::Day, View::Week, true), lens, "{day}");
    }
}

#[test]
fn zooms_run_their_frames_and_land_on_the_view() {
    let cx = ctx(store());
    let mut c = opened(19_723);
    c.view = View::Month;
    draw(&mut c, &cx);
    // The chosen day tapped again zooms into Day: six frames, the card
    // growing each frame, clipped, the page cleared under it.
    let cell = c.hits.find(Hit::Day(19_723)).unwrap();
    let tap = Event::PenUp {
        x: cell.x + 5,
        y: cell.y + 5,
    };
    assert_eq!(c.event(tap, &cx), Reply::Animate);
    assert_eq!(c.view, View::Day);
    let mut last = cell;
    for k in 1..=motion::FRAMES {
        let mut f = Box::new(Frame::new());
        c.draw(&mut f, &cx);
        let clip = f
            .cmds()
            .find_map(|m| match m {
                rp_app::Cmd::Clip { r } if r.w < u16::MAX => Some(*r),
                _ => None,
            })
            .expect("a zoom frame clips its card");
        assert!(clip.intersect(&last) == last, "frame {k} shrank");
        last = clip;
        let want = if k < motion::FRAMES {
            Reply::Animate
        } else {
            Reply::Redraw
        };
        assert_eq!(c.event(Event::Tick, &cx), want, "frame {k}");
    }
    assert_eq!(last, rp_app::CONTENT, "the last frame is the whole page");
    assert_eq!(c.motion, None);
    // And out again: Month's tab shrinks the day back into its cell.
    draw(&mut c, &cx);
    let r = c.hits.find(Hit::View(View::Month)).unwrap();
    assert_eq!(
        c.event(
            Event::PenUp {
                x: r.x + 4,
                y: r.y + 4
            },
            &cx
        ),
        Reply::Animate
    );
    assert_eq!(land(&mut c, &cx), motion::FRAMES - 1);
    assert_eq!(c.view, View::Month);
    // A tap mid-zoom lands it at once (the pen never waits).
    c.view = View::Day;
    c.event(Event::Key(Key::App), &cx);
    assert!(c.motion.is_some());
    draw(&mut c, &cx);
    c.event(Event::PenDown { x: 10, y: 300 }, &cx);
    assert_eq!(c.motion, None);
}

#[test]
fn the_strip_follows_the_pen_and_pages_a_week() {
    let cx = ctx(store());
    let mut c = opened(19_723);
    c.view = View::Day;
    draw(&mut c, &cx);
    let band = c.strip_at.expect("the strip drawn");
    let y = band.y + 30;
    // A drag left past a quarter: the strip follows by the pixel, then
    // settles on the next week, the chosen weekday kept.
    c.event(Event::PenDown { x: 300, y }, &cx);
    assert_eq!(c.event(Event::PenMove { x: 296, y }, &cx), Reply::Nothing);
    assert_eq!(c.event(Event::PenMove { x: 250, y }, &cx), Reply::Redraw);
    assert_eq!(c.slide, -50);
    draw(&mut c, &cx);
    c.event(Event::PenMove { x: 160, y }, &cx);
    assert_eq!(c.event(Event::PenUp { x: 160, y }, &cx), Reply::Animate);
    assert_eq!(land(&mut c, &cx), motion::FRAMES - 1);
    assert_eq!(c.day, 19_730);
    // A short slow drag goes back.
    c.event(Event::PenDown { x: 100, y }, &cx);
    c.event(Event::PenMove { x: 120, y }, &cx);
    c.event(Event::PenMove { x: 140, y }, &cx);
    c.event(Event::PenUp { x: 141, y }, &cx);
    land(&mut c, &cx);
    assert_eq!(c.day, 19_730);
    // A flick right, short as it is, goes back a week.
    c.event(Event::PenDown { x: 100, y }, &cx);
    c.event(Event::PenMove { x: 110, y }, &cx);
    c.event(Event::PenMove { x: 130, y }, &cx);
    c.event(Event::PenUp { x: 150, y }, &cx);
    land(&mut c, &cx);
    assert_eq!(c.day, 19_723);
    // A tap is still a tap: the day under it chosen.
    draw(&mut c, &cx);
    let cell = c.hits.find(Hit::Day(19_725)).unwrap();
    c.event(Event::PenDown { x: cell.x + 30, y }, &cx);
    c.event(Event::PenMove { x: cell.x + 33, y }, &cx);
    c.event(Event::PenUp { x: cell.x + 33, y }, &cx);
    assert_eq!((c.day, c.motion), (19_725, None));
}

#[test]
fn a_sliding_strip_draws_nothing_off_screen_and_nothing_wrong() {
    // Every frame of a slide: each command's rectangle inside the screen
    // (u16 can't hold a negative edge: a part left of the screen is cut
    // or left out, never wrapped to the right), the band clipped and the
    // clip ended.
    let cx = ctx(store());
    let mut c = opened(19_723);
    c.view = View::Agenda;
    draw(&mut c, &cx);
    let band = c.strip_at.unwrap();
    for dx in [-448i16, -301, -40, -1, 1, 13, 200, 447, 448] {
        c.slide = dx;
        // (As the shell draws it: inside its window's clip, which the strip
        // must put back, never end.)
        let window = Rect::new(0, 36, W, 564);
        let mut f = Box::new(Frame::new());
        f.clip(Some(window));
        c.draw(&mut f, &cx);
        assert_eq!(f.clip_now(), window, "{dx}: the window's clip put back");
        let mut clipped = false;
        for m in f.cmds() {
            match m {
                rp_app::Cmd::Clip { r } if *r == band => clipped = true,
                rp_app::Cmd::Clip { .. } => clipped = false,
                rp_app::Cmd::Fill { r, .. } | rp_app::Cmd::Panel { r, .. } if clipped => {
                    assert!(r.x < W && r.right() <= W + 64, "{dx}: {r:?}");
                }
                _ => {}
            }
        }
        assert!(!clipped, "{dx}: the band's clip left on");
    }
}

#[test]
fn the_dpad_across_a_week_slides_the_strip_in_from_its_side() {
    let cx = ctx(store());
    let mut c = opened(19_723);
    c.view = View::Day;
    // 19 723 is a Monday: Saturday is five days on, then Sunday starts
    // the next (US) week.
    for _ in 0..5 {
        assert_eq!(c.event(Event::Key(Key::Right), &cx), Reply::Redraw);
    }
    assert_eq!(c.event(Event::Key(Key::Right), &cx), Reply::Animate);
    assert_eq!(c.day, 19_729);
    assert_eq!(c.strip_offset(), {
        let k = 1;
        motion::lerp(i32::from(motion::STRIP_W), 0, k) as i16
    });
    land(&mut c, &cx);
    assert_eq!(c.day, 19_729);
    assert_eq!(c.event(Event::Key(Key::Left), &cx), Reply::Animate);
    assert!(c.strip_offset() < 0);
}
