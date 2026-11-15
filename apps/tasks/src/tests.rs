//! Tasks driven as the stylus drives it, against a real store.

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

fn draw(t: &mut Tasks, cx: &Ctx) {
    let mut f = Box::new(Frame::new());
    t.draw(&mut f, cx);
}

/// A task as the tests read it back: title, done, due, flag, repeat.
type Row = (
    std::vec::Vec<u8>,
    bool,
    Option<u32>,
    Option<u32>,
    Option<u32>,
);

fn tasks(pim: &core::cell::RefCell<Option<Store<'static>>>) -> std::vec::Vec<Row> {
    let g = pim.borrow();
    let s = g.as_ref().unwrap();
    s.all(Kind::Task)
        .map(|r| {
            (
                r.text(tag::TITLE).to_vec(),
                r.num(tag::DONE).is_some(),
                r.num(tag::DUE),
                r.num(tag::PRIORITY),
                r.num(tag::REPEAT),
            )
        })
        .collect()
}

fn typed(t: &mut Tasks, cx: &Ctx, line: &[u8]) {
    t.event(
        Event::PenUp {
            x: ADD_LINE.x + 20,
            y: ADD_LINE.y + 10,
        },
        cx,
    );
    for &c in line {
        t.event(Event::Char(c), cx);
    }
    t.event(Event::Done, cx);
}

#[test]
fn the_typed_line_keeps_date_flag_list_and_repeat() {
    let pim = store();
    let cx = ctx(pim);
    let mut t = opened(19_723);
    t.show(View::List(None));
    draw(&mut t, &cx);
    typed(&mut t, &cx, b"pay rent fri !1 every month");
    let all = tasks(pim);
    assert_eq!(all.len(), 1);
    let (title, done, due, flag, rep) = &all[0];
    // The quick parser capitalises the first word.
    assert_eq!(title, b"Pay rent");
    assert!(!done);
    // Monday 1 January 2024: Friday is the 5th.
    assert_eq!(*due, Some(19_727));
    assert_eq!(*flag, Some(3));
    let rule = rp_pim::repeat::Rule::unpack(rep.expect("a repeat"), 19_727).unwrap();
    assert_eq!(rule.every, rp_pim::repeat::Every::MonthDate);
}

#[test]
fn a_right_swipe_ticks_after_the_tick_s_motion() {
    let pim = store();
    let cx = ctx(pim);
    let mut t = opened(19_723);
    t.show(View::List(None));
    draw(&mut t, &cx);
    typed(&mut t, &cx, b"call the bank today");
    draw(&mut t, &cx);
    let (r, _) = t.rows_at[0];
    t.event(
        Event::PenDown {
            x: r.x + 60,
            y: r.y + 20,
        },
        &cx,
    );
    assert_eq!(
        t.event(
            Event::PenUp {
                x: r.x + 200,
                y: r.y + 20
            },
            &cx
        ),
        Reply::Animate
    );
    // Not done until the motion ends (five frames).
    assert!(!tasks(pim)[0].1);
    let mut steps = 0;
    while t.event(Event::Tick, &cx) == Reply::Animate {
        steps += 1;
        assert!(steps < 10);
    }
    assert!(tasks(pim)[0].1, "done after {steps} frames");
}

#[test]
fn a_left_swipe_offers_tomorrow_and_delete() {
    let pim = store();
    let cx = ctx(pim);
    let mut t = opened(19_723);
    t.show(View::List(None));
    draw(&mut t, &cx);
    typed(&mut t, &cx, b"water the plants today");
    typed(&mut t, &cx, b"buy stamps today");
    draw(&mut t, &cx);
    let (r, _) = t.rows_at[0];
    t.event(
        Event::PenDown {
            x: r.x + 300,
            y: r.y + 20,
        },
        &cx,
    );
    t.event(
        Event::PenUp {
            x: r.x + 100,
            y: r.y + 20,
        },
        &cx,
    );
    draw(&mut t, &cx);
    // TOMORROW.
    let a = t.actions[0];
    t.event(
        Event::PenUp {
            x: a.x + 10,
            y: a.y + 10,
        },
        &cx,
    );
    assert!(tasks(pim).iter().any(|x| x.2 == Some(19_724)));
    // And DELETE on the other one.
    t.show(View::List(None));
    draw(&mut t, &cx);
    let (r, _) = t.rows_at[0];
    t.event(
        Event::PenDown {
            x: r.x + 300,
            y: r.y + 20,
        },
        &cx,
    );
    t.event(
        Event::PenUp {
            x: r.x + 100,
            y: r.y + 20,
        },
        &cx,
    );
    draw(&mut t, &cx);
    let a = t.actions[2];
    t.event(
        Event::PenUp {
            x: a.x + 10,
            y: a.y + 10,
        },
        &cx,
    );
    assert_eq!(tasks(pim).len(), 1);
}

/// The class of 2026-11-14: a change rewrote the task with each field cut
/// to 64 bytes and only the fields it listed.
#[test]
fn a_tick_keeps_a_long_note_whole_and_fields_it_doesnt_know() {
    let pim = store();
    let cx = ctx(pim);
    let note = [b'n'; 300];
    let id = pim
        .borrow_mut()
        .as_mut()
        .unwrap()
        .put(
            Kind::Task,
            None,
            &[(tag::TITLE, b"Taxes"), (tag::NOTE, &note), (99, b"later")],
        )
        .unwrap();
    let mut t = opened(19_723);
    t.tick(&cx, id);
    let s = pim.borrow();
    let r = s.as_ref().unwrap().get(id).unwrap();
    assert!(r.num(tag::DONE).is_some());
    assert_eq!(r.text(tag::NOTE), &note[..]);
    assert_eq!(r.text(99), b"later");
}

#[test]
fn the_home_counts_the_day_and_everything_on_it_answers_the_stylus() {
    // Design language version 3: the home's ring, numbers, tiles and rows
    // are from the store, and every one is a target (the version 2 home
    // answered no tap at all).
    let pim = store();
    let cx = ctx(pim);
    let mut t = opened(19_723);
    t.show(View::List(None));
    draw(&mut t, &cx);
    typed(&mut t, &cx, b"call the dentist today");
    typed(&mut t, &cx, b"pick up the parts today");
    t.show(View::Home);
    draw(&mut t, &cx);
    // Two due, none done.
    assert_eq!(t.counts(19_723).0, 2);
    // A row's box ticks its task.
    let id = t.tasks.iter().flatten().next().expect("a task").id;
    let r = t.hits.find(Hit::Tick(id)).expect("its box is a target");
    assert_eq!(
        t.event(
            Event::PenUp {
                x: r.x + 10,
                y: r.y + 10
            },
            &cx
        ),
        Reply::Redraw
    );
    draw(&mut t, &cx);
    assert!(
        t.tasks
            .iter()
            .flatten()
            .any(|k| k.id == id && k.done_day == Some(19_723))
    );
    assert_eq!(t.counts(19_723).0, 1);
    // The hero opens Today; so does the D-pad's centre.
    assert!(t.hits.find(Hit::List(None)).is_some());
    t.event(Event::PenUp { x: 300, y: 100 }, &cx);
    assert_eq!(t.view, View::List(None));
    t.show(View::Home);
    assert_eq!(t.event(Event::Key(Key::Enter), &cx), Reply::Redraw);
    assert_eq!(t.view, View::List(None));
}
