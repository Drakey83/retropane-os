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
    // Make a task (tasks-rule0.md, the tie-in): one task in Tasks with
    // the appointment's title, due on its day, its time and place in the
    // note; a second tap makes no second one.
    for _ in 0..2 {
        c.event(
            Event::PenUp {
                x: 100,
                y: E_DELETE + 20,
            },
            &cx,
        );
    }
    assert!(c.edit.unwrap().task_made);
    {
        let g = pim.borrow();
        let s = g.as_ref().unwrap();
        let tasks: std::vec::Vec<_> = s.all(Kind::Task).collect();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].text(tag::TITLE), b"Dentist");
        // Weekly from Monday 1 January 2024: the next one from today is today.
        assert_eq!(tasks[0].num(tag::DUE), Some(19_723));
        assert_eq!(
            tasks[0].text(tag::NOTE),
            b"From Calendar: Mon Jan 1, 3:05 PM"
        );
    }
    c.event(
        Event::PenUp {
            x: 360,
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
            x: 360,
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
    for v in [View::Day, View::Week, View::Month, View::Agenda] {
        assert_eq!(tap(&mut c, Hit::View(v)), Reply::Redraw);
        assert_eq!(c.view, v);
        draw(&mut c, &cx);
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
    tap(&mut c, Hit::OpenDay(19_724));
    assert_eq!((c.view, c.day), (View::Day, 19_724));
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
    // Right is Delete (the centre is Make a task, beside it).
    c.event(Event::Key(Key::Right), &cx);
    assert!(c.edit.unwrap().confirm_delete);
    c.event(Event::Key(Key::Up), &cx);
    c.event(Event::Key(Key::Down), &cx);
    assert!(!c.edit.unwrap().confirm_delete);
    c.event(Event::Key(Key::Enter), &cx);
    assert!(c.edit.unwrap().task_made && !c.edit.unwrap().confirm_delete);
    c.event(Event::Key(Key::Right), &cx);
    c.event(Event::Key(Key::Right), &cx);
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
    for want in [View::Agenda, View::Day, View::Week, View::Month, View::Home] {
        assert_eq!(c.event(Event::Key(Key::App), &cx), Reply::Redraw);
        assert_eq!(c.view, want);
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

/// A repeating task shows on each of its days to come in the agenda; an
/// after-done one on its due day only.
#[test]
fn repeating_tasks_show_on_each_of_their_days() {
    use rp_pim::repeat::{Every, Rule};
    let pim = store();
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        let rule = |after| {
            Rule {
                every: Every::Day,
                n: 2,
                days: 0,
                until: None,
                after,
            }
            .pack(19_723)
            .to_le_bytes()
        };
        let due = 19_723u32.to_le_bytes();
        let (fixed, after) = (rule(false), rule(true));
        s.put(
            Kind::Task,
            None,
            &[
                (tag::TITLE, b"Run"),
                (tag::DUE, &due),
                (tag::REPEAT, &fixed),
            ],
        )
        .unwrap();
        s.put(
            Kind::Task,
            None,
            &[
                (tag::TITLE, b"Water"),
                (tag::DUE, &due),
                (tag::REPEAT, &after),
            ],
        )
        .unwrap();
    }
    let cx = ctx(pim);
    let mut c = opened(19_723);
    draw(&mut c, &cx);
    let titles = |c: &Calendar, day| {
        let (list, n) = c.on(day);
        list[..n]
            .iter()
            .filter(|a| a.task)
            .map(|a| a.title.get().to_vec())
            .collect::<std::vec::Vec<_>>()
    };
    assert_eq!(titles(&c, 19_723), [b"Run".to_vec(), b"Water".to_vec()]);
    assert!(titles(&c, 19_724).is_empty());
    assert_eq!(titles(&c, 19_725), [b"Run".to_vec()]);
    assert_eq!(titles(&c, 19_727), [b"Run".to_vec()]);
}

/// The Birthdays calendar (calendar-rule0.md, contacts-rule0.md): made the
/// first time a contact has a birthday, after this device's own calendar;
/// the contacts' days on it, gone when hidden or when the contact goes;
/// its rows open no editor, and no appointment can be put in it.
#[test]
fn contacts_birthdays_make_the_birthdays_calendar() {
    use rp_pim::contact::Day;
    let pim = store();
    let born = Day {
        year: Some(1983),
        month: 1,
        day: 3,
    }
    .field()
    .unwrap();
    let sam = pim
        .borrow_mut()
        .as_mut()
        .unwrap()
        .put(
            Kind::Contact,
            None,
            &[
                (tag::FIRST, b"Sam"),
                (tag::LAST, b"Lee"),
                (tag::BIRTHDAY, &born),
            ],
        )
        .unwrap();
    let cx = ctx(pim);
    let mut c = opened(19_723);
    c.show(View::Agenda);
    draw(&mut c, &cx);
    {
        let g = pim.borrow();
        let s = g.as_ref().unwrap();
        let cals: std::vec::Vec<_> = s
            .all(Kind::Calendar)
            .map(|r| r.text(tag::NAME).to_vec())
            .collect();
        assert_eq!(cals, [b"Calendar".to_vec(), b"Birthdays".to_vec()]);
        assert!(rp_pim::birthdays::calendar(s).is_some());
    }
    draw(&mut c, &cx);
    // Wednesday 3 January 2024.
    let (list, n) = c.on(19_725);
    let b = list[..n]
        .iter()
        .find(|a| a.id == sam)
        .expect("the birthday");
    assert_eq!(b.title.get(), b"Sam Lee's 41st birthday");
    assert_eq!(b.len, 0);
    assert_eq!(c.cals[usize::from(b.cal)].name.get(), b"Birthdays");
    // Its row opens no editor.
    c.open_edit(&cx, Some(sam));
    assert!(c.edit.is_none());
    assert_ne!(c.view, View::Edit);
    // The editor's calendars leave it out.
    let (ix, k) = c.chip_cals();
    assert_eq!(k, 1);
    assert_eq!(c.cals[ix[0]].name.get(), b"Calendar");
    // Hidden, its days go; shown again, back.
    let bid = c.cals[usize::from(b.cal)].id;
    c.toggle_hidden(&cx, bid);
    draw(&mut c, &cx);
    assert!(!c.on(19_725).0[..c.on(19_725).1].iter().any(|a| a.id == sam));
    c.toggle_hidden(&cx, bid);
    draw(&mut c, &cx);
    assert!(c.on(19_725).0[..c.on(19_725).1].iter().any(|a| a.id == sam));
    // The contact gone: its birthday too, no record left behind.
    pim.borrow_mut().as_mut().unwrap().remove(sam).unwrap();
    draw(&mut c, &cx);
    assert!(!c.on(19_725).0[..c.on(19_725).1].iter().any(|a| a.id == sam));
    assert_eq!(
        pim.borrow()
            .as_ref()
            .unwrap()
            .all(Kind::Appointment)
            .count(),
        0
    );
}

/// R370: the editor's chips say only "WEEKLY" or "MONTHLY"; a rule they
/// say in part (an iCal import, every 2 weeks on Mon and Wed to an end
/// day) is kept whole through an edit, and through a chip tapped away and
/// back; another chip chosen and kept is the plain rule it names.
#[test]
fn an_edit_keeps_a_repeat_the_chips_show_only_in_part() {
    use rp_pim::repeat::{Every, Rule};
    let pim = store();
    // Monday 1 January 2024.
    let day = 19_723;
    let rule = Rule {
        every: Every::Week,
        n: 2,
        days: 1 | 1 << 2,
        until: Some(day + 60),
        after: false,
    };
    let id = {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        let at = (date::start_of(day) + 15 * 60).to_le_bytes();
        s.put(
            Kind::Appointment,
            None,
            &[
                (tag::TITLE, b"Standup"),
                (tag::START, &at),
                (tag::REPEAT, &rule.pack(day).to_le_bytes()),
            ],
        )
        .unwrap()
    };
    let cx = ctx(pim);
    let kept = |want: Rule| {
        let g = pim.borrow();
        let r = g.as_ref().unwrap().get(id).unwrap();
        let got = Rule::unpack(r.num(tag::REPEAT).unwrap(), day).unwrap();
        assert_eq!(got, want);
    };
    let tap = |c: &mut Calendar, chip: u16, y: u16| {
        c.event(
            Event::PenUp {
                x: MARGIN + WIDE * chip / 5 + 10,
                y: y + 24,
            },
            &cx,
        );
    };
    let mut c = opened(day);
    // A change elsewhere (a quarter later), saved: the rule is the same.
    c.open_edit(&cx, Some(id));
    assert_eq!(c.edit.unwrap().repeat, 2);
    draw(&mut c, &cx);
    c.event(
        Event::PenUp {
            x: W - 40,
            y: E_START + 20,
        },
        &cx,
    );
    c.event(Event::Key(Key::SoftRight), &cx);
    kept(rule);
    // DAILY and back to WEEKLY: still its own.
    c.open_edit(&cx, Some(id));
    tap(&mut c, 1, E_REPEAT);
    tap(&mut c, 2, E_REPEAT);
    c.event(Event::Key(Key::SoftRight), &cx);
    kept(rule);
    // DAILY kept: plain daily.
    c.open_edit(&cx, Some(id));
    tap(&mut c, 1, E_REPEAT);
    c.event(Event::Key(Key::SoftRight), &cx);
    kept(Rule {
        every: Every::Day,
        n: 1,
        days: 0,
        until: None,
        after: false,
    });
    // The 2nd Tuesday of the month shows as MONTHLY and stays the 2nd
    // Tuesday (it became the 9th of each month before).
    let tue = day + 8;
    let nth = Rule {
        every: Every::MonthWeekday,
        n: 1,
        days: 0,
        until: None,
        after: false,
    };
    let id = {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        let at = date::start_of(tue).to_le_bytes();
        s.put(
            Kind::Appointment,
            None,
            &[
                (tag::TITLE, b"Club"),
                (tag::START, &at),
                (tag::REPEAT, &nth.pack(tue).to_le_bytes()),
            ],
        )
        .unwrap()
    };
    c.open_edit(&cx, Some(id));
    assert_eq!(c.edit.unwrap().repeat, 3);
    c.event(Event::Key(Key::SoftRight), &cx);
    let g = pim.borrow();
    let r = g.as_ref().unwrap().get(id).unwrap();
    assert_eq!(Rule::unpack(r.num(tag::REPEAT).unwrap(), tue), Some(nth));
}

/// R372: every date the calendar writes is in the region's order, "Jan 1"
/// in the US and "1 Jan" elsewhere (the editor's DAY row was "MON 1 Jan"
/// on a US device; the title and search were "Jan 1" everywhere).
#[test]
fn dates_are_written_in_the_regions_order() {
    // Monday 1 January 2024; Sunday 31 December 2023 the day before.
    for (day, us, want) in [
        (19_723, true, "Jan 1"),
        (19_723, false, "1 Jan"),
        (19_722, true, "Dec 31"),
        (19_722, false, "31 Dec"),
    ] {
        let mut b = Buf::<16>::new();
        b.date(day, us);
        assert_eq!(b.get(), want.as_bytes());
    }
    // The task made from an appointment, outside the US.
    let pim = store();
    let at = (date::start_of(19_723) + 9 * 60).to_le_bytes();
    let id = pim
        .borrow_mut()
        .as_mut()
        .unwrap()
        .put(
            Kind::Appointment,
            None,
            &[(tag::TITLE, b"Dentist"), (tag::START, &at)],
        )
        .unwrap();
    let cx = Ctx {
        us: false,
        ..ctx(pim)
    };
    let mut c = opened(19_723);
    c.open_edit(&cx, Some(id));
    draw(&mut c, &cx);
    c.event(
        Event::PenUp {
            x: 100,
            y: E_DELETE + 20,
        },
        &cx,
    );
    let g = pim.borrow();
    let t = g.as_ref().unwrap().all(Kind::Task).next().unwrap();
    assert_eq!(t.text(tag::NOTE), b"From Calendar: Mon 1 Jan, 9:00 AM");
}
