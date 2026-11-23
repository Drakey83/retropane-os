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
    assert_eq!((e.alerts, e.alert_other), (1 << 5, 7));
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
    assert_eq!(c.edit.unwrap().alerts, 1 << 2);
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
        // Both alerts kept (several per appointment), latest first.
        let leads: std::vec::Vec<u32> = rp_pim::remind::leads(&r).collect();
        assert_eq!(leads, [15, 7]);
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
    assert_eq!((e.alerts, e.alert_other), (1 << 2 | 1 << 5, 7));
}

#[test]
fn several_alerts_are_chips_turned_on_and_off_and_kept() {
    // calendar-rule0.md: "Reminders as a banner over any screen, several
    // per appointment". The alert row's chips turn on and off (NONE takes
    // all off); the D-pad walks the chips and the centre presses one; an
    // imported appointment's leads the chips can't show are kept when
    // its alerts aren't touched.
    let pim = store();
    let leads_of = |pim: &core::cell::RefCell<Option<Store<'static>>>| {
        let g = pim.borrow();
        let r = g.as_ref().unwrap().all(Kind::Appointment).next().unwrap();
        rp_pim::remind::leads(&r).collect::<std::vec::Vec<u32>>()
    };
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        let at = (date::start_of(19_723) + 15 * 60).to_le_bytes();
        let mut b = [0u8; 4 * rp_pim::remind::LEADS_MAX];
        let n = rp_pim::remind::pack(&[7, 2880], &mut b);
        s.put(
            Kind::Appointment,
            None,
            &[
                (tag::TITLE, b"Dentist"),
                (tag::START, &at),
                (tag::REMIND, &b[..n]),
            ],
        )
        .unwrap();
    }
    let cx = ctx(pim);
    let mut c = opened(19_723);
    c.show(View::Agenda);
    draw(&mut c, &cx);
    let open = |c: &mut Calendar| {
        let (r, _, _) = c.rows_at[0];
        c.event(
            Event::PenUp {
                x: r.x + 100,
                y: r.y + 20,
            },
            &cx,
        );
        draw(c, &cx);
    };
    // Untouched: both own leads kept (the chips show one).
    open(&mut c);
    c.event(Event::Key(Key::SoftRight), &cx);
    assert_eq!(leads_of(pim), [2880, 7]);
    draw(&mut c, &cx);
    open(&mut c);
    let row = Rect::new(MARGIN, E_ALERT + 14, WIDE, 30);
    let tap = |c: &mut Calendar, k: usize| {
        let r = kit::chip(row, 6, k);
        c.event(
            Event::PenUp {
                x: r.x + r.w / 2,
                y: r.y + 10,
            },
            &cx,
        );
        draw(c, &cx);
    };
    // NONE, then AT START, 1 HOUR and 1 DAY on, 1 HOUR off again.
    tap(&mut c, 0);
    assert_eq!(c.edit.unwrap().alerts, 0);
    for k in [1, 3, 4, 3] {
        tap(&mut c, k);
    }
    assert_eq!(c.edit.unwrap().alerts, 1 << 1 | 1 << 4);
    // The D-pad: down to the alert row, right to 15 MIN, the centre.
    for _ in 0..7 {
        c.event(Event::Key(Key::Down), &cx);
        if c.edit.unwrap().row == Some(6) {
            break;
        }
    }
    assert_eq!(c.edit.unwrap().row, Some(6));
    c.event(Event::Key(Key::Left), &cx);
    c.event(Event::Key(Key::Left), &cx);
    c.event(Event::Key(Key::Left), &cx);
    c.event(Event::Key(Key::Left), &cx);
    c.event(Event::Key(Key::Right), &cx);
    c.event(Event::Key(Key::Right), &cx);
    assert_eq!(c.edit.unwrap().alert_at, 2);
    c.event(Event::Key(Key::Enter), &cx);
    assert_eq!(c.edit.unwrap().alerts, 1 << 1 | 1 << 2 | 1 << 4);
    c.event(Event::Key(Key::SoftRight), &cx);
    assert_eq!(leads_of(pim), [1440, 15, 0]);
}

const FILE: &[u8] = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nDTSTART;TZID=America/New_York:20240101T150000\r\nDTEND;TZID=America/New_York:20240101T160000\r\nSUMMARY:Call New York\r\nDESCRIPTION:Bring the numbers\r\nBEGIN:VALARM\r\nTRIGGER:-PT10M\r\nEND:VALARM\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nDTSTART;VALUE=DATE:20240102\r\nSUMMARY:Holiday\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

fn tap(c: &mut Calendar, cx: &Ctx, r: Rect) -> Reply {
    c.event(
        Event::PenUp {
            x: r.x + r.w / 2,
            y: r.y + r.h / 2,
        },
        cx,
    )
}

/// The device in Los Angeles (its zone chosen).
fn in_los_angeles(c: &mut Calendar, cx: &Ctx) {
    c.keep_zone_for_test(cx, rp_pim::zone::find(b"America/Los_Angeles"));
    draw(c, cx);
}

#[test]
fn a_card_file_is_imported_once_into_its_own_calendar_and_shown_in_the_device_zone() {
    let pim = store();
    let cx = ctx(pim);
    let mut c = opened(19_723);
    in_los_angeles(&mut c, &cx);
    c.view = View::Calendars;
    draw(&mut c, &cx);
    tap(&mut c, &cx, files::IMPORT_KEY);
    assert_eq!(c.view, View::Files);
    // The card and the PC are both asked for their files.
    let ask = c.card_ask().expect("the card asked");
    assert_eq!((ask.what, ask.arg.get()), (&b"CARDFILES"[..], &b"ics"[..]));
    c.net_answer(true, b"0|WORK.ICS\n1|old.ics\n", &cx);
    let ask = c.net_ask().expect("the PC asked");
    assert_eq!((ask.what, ask.arg.get()), (&b"FILES"[..], &b"ics"[..]));
    c.net_answer(false, b"", &cx);
    draw(&mut c, &cx);
    // The card's first file chosen and read.
    let row = c.io_row(0).expect("a row");
    tap(&mut c, &cx, row);
    let ask = c.card_ask().expect("the file asked");
    assert_eq!((ask.what, ask.arg.get()), (&b"CARDFILE"[..], &b"0 ics"[..]));
    c.net_answer(true, FILE, &cx);
    assert_eq!(c.view, View::Calendars);
    assert_eq!(c.status_text.get(), b"Imported 2 appointments");
    {
        let g = pim.borrow();
        let s = g.as_ref().unwrap();
        let cal = s
            .all(Kind::Calendar)
            .find(|r| r.text(tag::NAME) == b"WORK")
            .expect("its own calendar");
        let ny = s
            .all(Kind::Appointment)
            .find(|r| r.text(tag::TITLE) == b"Call New York")
            .unwrap();
        assert_eq!(ny.num(tag::CAL), Some(cal.id));
        assert_eq!(ny.text(tag::ZONE), b"America/New_York");
        assert_eq!(ny.text(tag::NOTE), b"Bring the numbers");
        assert_eq!(ny.num(tag::REMIND), Some(10));
        // Kept as 3 PM New York time.
        assert_eq!(ny.num(tag::START), Some(date::start_of(19_723) + 15 * 60));
    }
    // Shown at noon in Los Angeles.
    c.view = View::Agenda;
    draw(&mut c, &cx);
    let (list, n) = c.on(19_723);
    let a = list[..n]
        .iter()
        .find(|a| a.title.get() == b"Call New York")
        .unwrap();
    assert_eq!(a.start, 12 * 60);
    // The same file again: nothing doubled, and no empty calendar left.
    c.view = View::Calendars;
    draw(&mut c, &cx);
    tap(&mut c, &cx, files::IMPORT_KEY);
    let _ = c.card_ask();
    c.net_answer(true, b"0|WORK.ICS\n", &cx);
    let _ = c.net_ask();
    c.net_answer(false, b"", &cx);
    draw(&mut c, &cx);
    let row = c.io_row(0).unwrap();
    tap(&mut c, &cx, row);
    let _ = c.card_ask();
    c.net_answer(true, FILE, &cx);
    assert_eq!(c.status_text.get(), b"Imported 0 (2 already here)");
    let g = pim.borrow();
    let s = g.as_ref().unwrap();
    assert_eq!(s.all(Kind::Appointment).count(), 2);
    assert_eq!(
        s.all(Kind::Calendar)
            .filter(|r| r.text(tag::NAME) == b"WORK")
            .count(),
        1
    );
}

#[test]
fn the_calendar_is_exported_a_piece_at_a_time_and_reads_back_whole() {
    let pim = store();
    let cx = ctx(pim);
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        for k in 0..40u32 {
            let at = (date::start_of(19_723 + k as i32) + 540).to_le_bytes();
            let title = std::format!("Meeting number {k}, with a title long enough to fold");
            s.put(
                Kind::Appointment,
                None,
                &[
                    (tag::TITLE, title.as_bytes()),
                    (tag::START, &at),
                    (tag::MINUTES, &45u32.to_le_bytes()),
                    (
                        tag::ZONE,
                        if k % 2 == 0 {
                            &b"Europe/London"[..]
                        } else {
                            b""
                        },
                    ),
                ],
            )
            .unwrap();
        }
    }
    let mut c = opened(19_723);
    in_los_angeles(&mut c, &cx);
    c.view = View::Calendars;
    draw(&mut c, &cx);
    tap(&mut c, &cx, files::EXPORT_KEY);
    draw(&mut c, &cx);
    tap(&mut c, &cx, files::TO_CARD_FOR_TEST);
    let ask = c.card_ask().expect("the card asked to keep it");
    assert_eq!(ask.what, b"CARDPUT");
    let arg = ask.arg.get();
    let sp = arg.iter().position(|&b| b == b' ').unwrap();
    let len: usize = std::str::from_utf8(&arg[..sp]).unwrap().parse().unwrap();
    assert_eq!(&arg[sp + 1..], b"Calendar 2024-01-01.ics");
    // In pieces of 700 bytes, as the device asks.
    let mut file = std::vec::Vec::new();
    let mut piece = [0u8; 700];
    loop {
        let n = c.net_body(file.len(), &mut piece, &cx);
        if n == 0 {
            break;
        }
        file.extend_from_slice(&piece[..n]);
    }
    assert_eq!(file.len(), len);
    // Asked whole (the card's way): the same bytes.
    let mut whole = vec![0u8; len];
    let mut at = 0;
    while at < len {
        let n = c.net_body(at, &mut whole[at..], &cx);
        assert!(n > 0);
        at += n;
    }
    assert_eq!(file, whole, "pieces and one read agree");
    let mut back = std::vec::Vec::new();
    rp_pim::ics::ics_each(&file, |e| {
        back.push(*e);
        true
    });
    assert_eq!(back.len(), 40);
    assert_eq!(back[0].zone.get(), b"Europe/London");
    // A floating time is written in the device's zone.
    assert_eq!(back[1].zone.get(), b"America/Los_Angeles");
    assert_eq!(back[1].minutes, 45);
    assert!(
        std::str::from_utf8(&file)
            .unwrap()
            .contains("BEGIN:VTIMEZONE\r\nTZID:Europe/London")
    );
    c.net_answer(true, b"", &cx);
    assert_eq!(c.status, b"Saved in the card's RetroPane folder");
}

#[test]
fn the_time_zone_is_guessed_from_the_pc_chosen_from_the_list_and_moves_times() {
    let pim = store();
    let mut cx = ctx(pim);
    // The PC says UTC-5 in January: New York.
    cx.utc_offset = -300;
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        let at = (date::start_of(19_723) + 9 * 60).to_le_bytes();
        s.put(
            Kind::Appointment,
            None,
            &[
                (tag::TITLE, b"Tokyo stand-up"),
                (tag::START, &at),
                (tag::ZONE, b"Asia/Tokyo"),
            ],
        )
        .unwrap();
    }
    let mut c = opened(19_723);
    c.day = 19_722;
    draw(&mut c, &cx);
    let mut name = [0u8; 40];
    let n = c.io.device.unwrap().name(&mut name);
    assert_eq!(&name[..n], b"America/New_York");
    assert!(c.io.guessed);
    // 9 AM in Tokyo is 7 PM the day before in New York.
    let (list, k) = c.on(19_722);
    assert!(
        list[..k]
            .iter()
            .any(|a| a.title.get() == b"Tokyo stand-up" && a.start == 19 * 60)
    );
    // Chosen from the list by typing.
    c.view = View::Calendars;
    draw(&mut c, &cx);
    tap(&mut c, &cx, files::ZONE_ROW);
    assert_eq!(c.view, View::Zones);
    draw(&mut c, &cx);
    c.event(
        Event::PenUp {
            x: 100,
            y: 36 + kit::TITLE_BAND + 20,
        },
        &cx,
    );
    assert_eq!(c.keyboard(), Some(Field::Line));
    for ch in b"tokyo" {
        c.event(Event::Char(*ch), &cx);
    }
    draw(&mut c, &cx);
    let row = c.zone_row(1).expect("Tokyo listed");
    tap(&mut c, &cx, row);
    assert_eq!(c.view, View::Calendars);
    c.day = 19_723;
    draw(&mut c, &cx);
    assert!(!c.io.guessed);
    let (list, k) = c.on(19_723);
    assert!(
        list[..k]
            .iter()
            .any(|a| a.title.get() == b"Tokyo stand-up" && a.start == 9 * 60)
    );
    // A new appointment is kept in the device's zone.
    c.open_edit(&cx, None);
    assert_eq!(c.edit.unwrap().zone.get(), b"Asia/Tokyo");
}

#[test]
fn made_in_its_slot_it_is_the_same_app() {
    let pim = store();
    let cx = ctx(pim);
    let mut slot = Box::new(core::mem::MaybeUninit::<Calendar>::uninit());
    let c = opened_in(&mut slot, 19_723);
    draw(c, &cx);
    assert_eq!(c.day, 19_723);
    assert_eq!(c.view, View::Home);
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

/// R375: the editor's chips say only "WEEKLY" or "MONTHLY"; a rule they
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
    // The owner's pick D (2026-11-23): frame 0 captures the arriving view
    // into the renderer's layer B; frames 1-6 show it scaled into a card
    // growing out of the chosen day's cell, the last the whole page; every
    // frame also holds the arriving view whole (a renderer without layers
    // makes the change at once) and the arriving view's targets.
    let cx = ctx(store());
    let mut c = opened(19_723);
    c.view = View::Month;
    draw(&mut c, &cx);
    let cell = c.hits.find(Hit::Day(19_723)).unwrap();
    let tap = Event::PenUp {
        x: cell.x + 5,
        y: cell.y + 5,
    };
    assert_eq!(c.event(tap, &cx), Reply::Animate);
    assert_eq!(c.view, View::Day);
    let op = |c: &mut Calendar| {
        let mut f = Box::new(Frame::new());
        f.clip(Some(rp_app::CONTENT));
        c.draw(&mut f, &cx);
        assert!(!f.overflowed(), "a zoom frame fits the frame");
        assert_eq!(f.clip_now(), rp_app::CONTENT, "the window's clip kept");
        assert!(
            c.hits.find(Hit::View(View::Month)).is_some(),
            "the day's targets"
        );
        f.cmds().find_map(|m| match m {
            rp_app::Cmd::Layers { op } => Some(*op),
            _ => None,
        })
    };
    assert_eq!(op(&mut c), Some(rp_app::LayerOp::Capture { part: 0 }));
    assert_eq!(c.event(Event::Tick, &cx), Reply::Animate);
    assert_eq!(op(&mut c), Some(rp_app::LayerOp::Capture { part: 1 }));
    let mut last = cell;
    for k in 1..=motion::FRAMES {
        assert_eq!(c.event(Event::Tick, &cx), Reply::Animate, "frame {k}");
        let Some(rp_app::LayerOp::Show {
            card,
            from,
            arriving_in_card,
        }) = op(&mut c)
        else {
            panic!("frame {k} shows the layers");
        };
        assert!(arriving_in_card && from == rp_app::CONTENT);
        assert!(card.intersect(&last) == last, "frame {k} shrank");
        last = card;
    }
    assert_eq!(last, rp_app::CONTENT, "the last frame is the whole page");
    // One frame more lands the captured view as the screen's.
    assert_eq!(c.event(Event::Tick, &cx), Reply::Animate);
    assert_eq!(op(&mut c), Some(rp_app::LayerOp::Land));
    assert_eq!(c.event(Event::Tick, &cx), Reply::Redraw);
    assert_eq!(c.motion, None);
    assert_eq!(op(&mut c), None, "an ordinary frame after");
    // And out again: Month's tab shrinks the day back into its cell (A, the
    // day as left, in the card over B, the month).
    let r = c.hits.find(Hit::View(View::Month)).unwrap();
    let up = Event::PenUp {
        x: r.x + 4,
        y: r.y + 4,
    };
    assert_eq!(c.event(up, &cx), Reply::Animate);
    assert_eq!(op(&mut c), Some(rp_app::LayerOp::Capture { part: 0 }));
    for _ in 0..motion::FRAMES {
        c.event(Event::Tick, &cx);
    }
    c.event(Event::Tick, &cx);
    let Some(rp_app::LayerOp::Show {
        card,
        arriving_in_card,
        ..
    }) = op(&mut c)
    else {
        panic!("the last frame shows the layers");
    };
    assert!(!arriving_in_card);
    assert_eq!(
        card,
        c.zoom_cell(View::Day, View::Month, true),
        "into its cell"
    );
    c.event(Event::Tick, &cx);
    assert_eq!(op(&mut c), Some(rp_app::LayerOp::Land));
    assert_eq!(c.event(Event::Tick, &cx), Reply::Redraw);
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

/// R377: every date the calendar writes is in the region's order, "Jan 1"
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
