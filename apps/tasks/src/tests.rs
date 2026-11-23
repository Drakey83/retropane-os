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
        clock12: true,
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

#[test]
fn the_dpad_walks_the_home_and_its_centre_taps() {
    // R203: the D-pad works in every app (kit::Focus over the targets the
    // home records).
    let pim = store();
    let cx = ctx(pim);
    let mut t = opened(19_723);
    t.show(View::List(None));
    draw(&mut t, &cx);
    typed(&mut t, &cx, b"call the dentist today");
    t.show(View::Home);
    draw(&mut t, &cx);
    // Down the home to the task's row, left to its box; the centre ticks
    // it.
    let on_box = |t: &Tasks| {
        t.focus
            .point()
            .is_some_and(|(x, y)| matches!(t.hits.at(x, y), Some(Hit::Tick(_))))
    };
    for _ in 0..12 {
        if on_box(&t) {
            break;
        }
        t.event(Event::Key(Key::Down), &cx);
        t.event(Event::Key(Key::Left), &cx);
        draw(&mut t, &cx);
    }
    assert!(on_box(&t), "{:?}", t.focus);
    assert_eq!(t.event(Event::Key(Key::Enter), &cx), Reply::Redraw);
    draw(&mut t, &cx);
    assert!(t.tasks.iter().flatten().any(|k| k.done_day == Some(19_723)));
}

#[test]
fn a_long_list_goes_on_over_the_page_and_a_done_task_can_be_unticked() {
    // Thirty tasks due today: the list cut them off at the screen's foot
    // (the rest out of the stylus's reach and the D-pad's), and a task
    // ticked by mistake was folded into "N done" for good.
    let pim = store();
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        for k in 0..30u8 {
            let title = [b'T', b'0' + k / 10, b'0' + k % 10];
            s.put(
                Kind::Task,
                None,
                &[(tag::TITLE, &title), (tag::DUE, &19_723u32.to_le_bytes())],
            )
            .unwrap();
        }
    }
    let cx = ctx(pim);
    let mut t = opened(19_723);
    t.show(View::List(None));
    draw(&mut t, &cx);
    assert!(t.n_rows < 30);
    // The D-pad alone: down through every page, each task's row focused.
    let mut met = std::vec::Vec::new();
    for _ in 0..300 {
        if t.event(Event::Key(Key::Down), &cx) == Reply::Nothing {
            break;
        }
        draw(&mut t, &cx);
        if let Some(&(_, id)) = t.rows_at[..t.n_rows]
            .iter()
            .find(|(r, _)| Some(*r) == t.focus.rect())
            && !met.contains(&id)
        {
            met.push(id);
        }
    }
    assert_eq!(met.len(), 30, "tasks reached by the D-pad");
    // Back to the first page by the D-pad; a drag turns the page too.
    for _ in 0..300 {
        if t.event(Event::Key(Key::Up), &cx) == Reply::Nothing {
            break;
        }
        draw(&mut t, &cx);
    }
    assert_eq!(t.first, 0);
    t.event(Event::PenDown { x: 300, y: 500 }, &cx);
    t.event(Event::PenUp { x: 300, y: 380 }, &cx);
    assert!(t.first > 0);
    t.event(Event::PenDown { x: 300, y: 380 }, &cx);
    t.event(Event::PenUp { x: 300, y: 500 }, &cx);
    assert_eq!(t.first, 0);
    draw(&mut t, &cx);
    // A row's box by the D-pad (left of its row), the centre ticks it.
    let on_box = |t: &Tasks| {
        t.boxes[..t.n_boxes]
            .iter()
            .find(|(r, _)| Some(*r) == t.focus.rect())
            .map(|b| b.1)
    };
    t.event(Event::Key(Key::Down), &cx);
    t.event(Event::Key(Key::Down), &cx);
    t.event(Event::Key(Key::Left), &cx);
    let id = on_box(&t).expect("on a tick box");
    assert_eq!(t.event(Event::Key(Key::Enter), &cx), Reply::Animate);
    while t.event(Event::Tick, &cx) == Reply::Animate {}
    draw(&mut t, &cx);
    assert!(t.tasks.iter().flatten().any(|k| k.id == id && k.done));
    assert_eq!(t.total, 29);
    // The done row shows them under the open ones; its box unticks.
    let foot = t.done_row.expect("a done row");
    t.event(
        Event::PenDown {
            x: foot.x + 60,
            y: foot.y + 10,
        },
        &cx,
    );
    t.event(
        Event::PenUp {
            x: foot.x + 60,
            y: foot.y + 10,
        },
        &cx,
    );
    draw(&mut t, &cx);
    assert!(t.show_done);
    assert_eq!(t.total, 30);
    for _ in 0..300 {
        if t.rows_at[..t.n_rows].iter().any(|r| r.1 == id) {
            break;
        }
        assert!(t.page_on());
        draw(&mut t, &cx);
    }
    let (b, _) = *t.boxes[..t.n_boxes]
        .iter()
        .find(|b| b.1 == id)
        .expect("shown");
    t.event(
        Event::PenDown {
            x: b.x + 9,
            y: b.y + 9,
        },
        &cx,
    );
    t.event(
        Event::PenUp {
            x: b.x + 9,
            y: b.y + 9,
        },
        &cx,
    );
    draw(&mut t, &cx);
    assert!(t.tasks.iter().flatten().any(|k| k.id == id && !k.done));
    // The centre on a row shows its actions, which take the arrows:
    // DELETE by the centre.
    t.first = 0;
    t.n_pages = 0;
    draw(&mut t, &cx);
    t.focus.clear();
    t.event(Event::Key(Key::Down), &cx);
    t.event(Event::Key(Key::Down), &cx);
    let (_, gone) = *t.rows_at[..t.n_rows]
        .iter()
        .find(|(r, _)| Some(*r) == t.focus.rect())
        .expect("on a row");
    t.event(Event::Key(Key::Enter), &cx);
    draw(&mut t, &cx);
    assert_eq!(t.revealed, Some(gone));
    assert_eq!(t.list_targets().1, 5);
    // (The line that adds a step first, then down to the actions.)
    t.event(Event::Key(Key::Down), &cx);
    t.event(Event::Key(Key::Down), &cx);
    for _ in 0..3 {
        t.event(Event::Key(Key::Right), &cx);
    }
    assert_eq!(t.focus.rect(), Some(t.actions[2]));
    t.event(Event::Key(Key::Enter), &cx);
    draw(&mut t, &cx);
    assert!(!t.tasks.iter().flatten().any(|k| k.id == gone));
}

#[test]
fn search_finds_tasks_of_every_list_and_the_done_ones() {
    let pim = store();
    {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        let one = 1u32.to_le_bytes();
        s.put(Kind::Task, None, &[(tag::TITLE, b"Pay the rent")])
            .unwrap();
        s.put(
            Kind::Task,
            None,
            &[(tag::TITLE, b"Rent a van"), (tag::LIST, &one)],
        )
        .unwrap();
        s.put(
            Kind::Task,
            None,
            &[(tag::TITLE, b"Parent evening"), (tag::DONE, &one)],
        )
        .unwrap();
        s.put(Kind::Task, None, &[(tag::TITLE, b"Call the bank")])
            .unwrap();
    }
    let cx = ctx(pim);
    let mut t = opened(19_723);
    draw(&mut t, &cx);
    // The home's SEARCH key (in the D-pad's reach, as every target).
    assert!(t.hits.rects().any(|r| r == FIND_KEY));
    t.event(
        Event::PenDown {
            x: FIND_KEY.x + 9,
            y: FIND_KEY.y + 9,
        },
        &cx,
    );
    t.event(
        Event::PenUp {
            x: FIND_KEY.x + 9,
            y: FIND_KEY.y + 9,
        },
        &cx,
    );
    assert!(t.find && t.keyboard().is_some());
    draw(&mut t, &cx);
    assert_eq!(t.n_rows, 4, "nothing typed: every task");
    for &c in b"RENT" {
        t.event(Event::Char(c), &cx);
    }
    draw(&mut t, &cx);
    assert_eq!(t.n_rows, 3, "two lists and a done one");
    // Done leaves the word and the rows; nothing was added.
    t.event(Event::Done, &cx);
    draw(&mut t, &cx);
    assert!(t.keyboard().is_none());
    assert_eq!((t.n_rows, tasks(pim).len()), (3, 4));
    // A found task ticks by its box as in its list.
    let id = t
        .tasks
        .iter()
        .flatten()
        .find(|k| k.title.get() == b"Pay the rent")
        .unwrap()
        .id;
    let (b, _) = *t.boxes[..t.n_boxes].iter().find(|b| b.1 == id).unwrap();
    t.event(
        Event::PenDown {
            x: b.x + 9,
            y: b.y + 9,
        },
        &cx,
    );
    t.event(
        Event::PenUp {
            x: b.x + 9,
            y: b.y + 9,
        },
        &cx,
    );
    while t.event(Event::Tick, &cx) == Reply::Animate {}
    draw(&mut t, &cx);
    assert!(t.tasks.iter().flatten().any(|k| k.id == id && k.done));
    // Back: the home, the search and its word gone.
    t.event(Event::Key(Key::SoftLeft), &cx);
    assert!(t.view == View::Home && !t.find && t.line_n == 0);
}

fn tap(t: &mut Tasks, cx: &Ctx, r: Rect) {
    t.event(
        Event::PenDown {
            x: r.x + 6,
            y: r.y + 6,
        },
        cx,
    );
    t.event(
        Event::PenUp {
            x: r.x + 6,
            y: r.y + 6,
        },
        cx,
    );
}

/// The store's lists: (name, colour number), in the app's order.
fn lists_kept(
    pim: &core::cell::RefCell<Option<Store<'static>>>,
) -> std::vec::Vec<(std::vec::Vec<u8>, u32)> {
    let g = pim.borrow();
    g.as_ref()
        .unwrap()
        .all(Kind::List)
        .map(|r| (r.text(tag::NAME).to_vec(), r.num(tag::COLOUR).unwrap_or(99)))
        .collect()
}

#[test]
fn lists_are_made_renamed_recoloured_and_deleted_by_the_user() {
    // The first version showed the lists a build gave it; none could be
    // made or named, and a task typed in a list went to the first one.
    let pim = store();
    let cx = ctx(pim);
    let mut t = opened(19_723);
    draw(&mut t, &cx);
    let before = t.n_lists;
    assert!(lists_kept(pim).is_empty());
    // The home's EDIT key: the lists' screen (in the D-pad's reach).
    assert!(t.hits.rects().any(|r| r == LISTS_KEY));
    tap(&mut t, &cx, LISTS_KEY);
    assert_eq!(t.view, View::Lists);
    draw(&mut t, &cx);
    assert_eq!(t.n_keys, before * 4);
    // A new list by the line: kept (the ones shown get their records
    // with it), and shown.
    tap(&mut t, &cx, ADD_LINE);
    assert!(t.keyboard().is_some());
    for &c in b"Garden" {
        t.event(Event::Char(c), &cx);
    }
    t.event(Event::Done, &cx);
    draw(&mut t, &cx);
    assert_eq!(t.n_lists, before + 1);
    assert_eq!(lists_kept(pim).len(), before + 1);
    let garden = t.lists[..t.n_lists]
        .iter()
        .position(|l| l.name.get() == b"Garden")
        .expect("the new list") as u8;
    let key = |t: &Tasks, i: u8, what: u8| {
        t.list_keys[..t.n_keys]
            .iter()
            .find(|k| k.1 == i && k.2 == what)
            .unwrap()
            .0
    };
    // Renamed: the line holds its name; retyped and kept.
    let r = key(&t, garden, 2);
    tap(&mut t, &cx, r);
    assert_eq!(&t.line[..t.line_n], b"Garden");
    for _ in 0..6 {
        t.event(Event::Delete, &cx);
    }
    for &c in b"Yard" {
        t.event(Event::Char(c), &cx);
    }
    t.event(Event::Key(Key::SoftRight), &cx);
    draw(&mut t, &cx);
    assert_eq!(t.lists[usize::from(garden)].name.get(), b"Yard");
    assert_eq!(t.n_lists, before + 1);
    // Recoloured: the next colour.
    let was = t.lists[usize::from(garden)].colour;
    let r = key(&t, garden, 1);
    tap(&mut t, &cx, r);
    draw(&mut t, &cx);
    assert_ne!(t.lists[usize::from(garden)].colour, was);
    // Its name opens it; a task typed there is its task.
    let r = key(&t, garden, 0);
    tap(&mut t, &cx, r);
    assert_eq!(t.view, View::List(Some(garden)));
    draw(&mut t, &cx);
    typed(&mut t, &cx, b"Mow the lawn");
    draw(&mut t, &cx);
    let mow = *t
        .tasks
        .iter()
        .flatten()
        .find(|k| k.title.get() == b"Mow the lawn")
        .unwrap();
    assert_eq!(mow.list, garden);
    // Deleted: asked once (another key takes the question back), then
    // gone, its task in another list, not lost. (Back twice: the
    // keyboard, then the list.)
    t.event(Event::Key(Key::SoftLeft), &cx);
    t.event(Event::Key(Key::SoftLeft), &cx);
    draw(&mut t, &cx);
    tap(&mut t, &cx, LISTS_KEY);
    draw(&mut t, &cx);
    let r = key(&t, garden, 3);
    tap(&mut t, &cx, r);
    assert_eq!(t.ask_delete, Some(garden));
    let r = key(&t, 0, 1);
    tap(&mut t, &cx, r);
    assert_eq!(t.ask_delete, None);
    draw(&mut t, &cx);
    let r = key(&t, garden, 3);
    tap(&mut t, &cx, r);
    let r = key(&t, garden, 3);
    tap(&mut t, &cx, r);
    draw(&mut t, &cx);
    assert_eq!(t.n_lists, before);
    assert!(!t.lists[..t.n_lists].iter().any(|l| l.name.get() == b"Yard"));
    assert!(t.tasks.iter().flatten().any(|k| k.id == mow.id));
    // The D-pad: every key of the screen in reach of the line.
    let all: std::vec::Vec<Rect> = core::iter::once(ADD_LINE)
        .chain(t.list_keys[..t.n_keys].iter().map(|k| k.0))
        .collect();
    let mut seen = std::vec![ADD_LINE];
    let mut at = 0;
    while at < seen.len() {
        for k in [Key::Up, Key::Down, Key::Left, Key::Right] {
            let mut f = kit::Focus::default();
            f.set(seen[at]);
            if f.step(k, all.iter().copied())
                && let Some(r) = f.rect()
                && !seen.contains(&r)
            {
                seen.push(r);
            }
        }
        at += 1;
    }
    assert_eq!(seen.len(), all.len());
    t.event(Event::Key(Key::Down), &cx);
    t.event(Event::Key(Key::Down), &cx);
    t.event(Event::Key(Key::Enter), &cx);
    assert_eq!(t.view, View::List(Some(0)));
}

#[test]
fn a_task_s_steps_are_added_ticked_and_go_with_it() {
    // The row said "2 of 5"; there was no way to make a step or tick one.
    let pim = store();
    let cx = ctx(pim);
    let mut t = opened(19_723);
    t.show(View::List(None));
    draw(&mut t, &cx);
    typed(&mut t, &cx, b"Pack for the trip today");
    draw(&mut t, &cx);
    let id = t.rows_at[0].1;
    // Its actions shown (the centre on its row, the keyboard put away
    // first): the line adds steps.
    t.event(Event::Key(Key::SoftLeft), &cx);
    t.event(Event::Key(Key::Down), &cx);
    t.event(Event::Key(Key::Down), &cx);
    t.event(Event::Key(Key::Enter), &cx);
    assert_eq!(t.revealed, Some(id));
    for step in [&b"Passport"[..], b"Charger"] {
        tap(&mut t, &cx, ADD_LINE);
        for &c in step {
            t.event(Event::Char(c), &cx);
        }
        t.event(Event::Done, &cx);
        draw(&mut t, &cx);
        assert_eq!(t.revealed, Some(id), "the task stays open");
    }
    assert_eq!(t.n_steps, 2);
    let me = |t: &Tasks| *t.tasks.iter().flatten().find(|k| k.id == id).unwrap();
    assert_eq!(me(&t).steps, (0, 2));
    // A step ticked, and unticked; the task's count follows.
    let (sr, _, _) = t.steps_at[0];
    tap(&mut t, &cx, sr);
    draw(&mut t, &cx);
    assert_eq!(me(&t).steps, (1, 2));
    assert_eq!(t.revealed, Some(id));
    // The D-pad reaches the steps too.
    let (ts, n) = t.list_targets();
    assert!(ts[..n].contains(&t.steps_at[1].0));
    let r = t.steps_at[0].0;
    tap(&mut t, &cx, r);
    draw(&mut t, &cx);
    assert_eq!(me(&t).steps, (0, 2));
    // Deleted: its steps go with it.
    let r = t.actions[2];
    tap(&mut t, &cx, r);
    draw(&mut t, &cx);
    assert_eq!(pim.borrow().as_ref().unwrap().all(Kind::Task).count(), 0);
}

#[test]
fn a_task_takes_a_time_and_keeps_it_when_moved() {
    // The owner on the X51v, 2026-11-15: "you cant set a time for tasks?".
    let pim = store();
    let cx = ctx(pim);
    let mut t = opened(19_723);
    t.show(View::List(None));
    draw(&mut t, &cx);
    typed(&mut t, &cx, b"Call the bank today 3pm");
    draw(&mut t, &cx);
    let me = |t: &Tasks| *t.tasks.iter().flatten().next().unwrap();
    assert_eq!(me(&t).title.get(), b"Call the bank");
    assert_eq!((me(&t).due, me(&t).time), (Some(19_723), Some(15 * 60)));
    let kept = |pim: &core::cell::RefCell<Option<Store<'static>>>| {
        let g = pim.borrow();
        g.as_ref()
            .unwrap()
            .all(Kind::Task)
            .next()
            .unwrap()
            .num(tag::REMIND_AT)
    };
    assert_eq!(kept(pim), Some(date::start_of(19_723) + 15 * 60));
    // The time as the region writes it.
    let mut b = [0u8; 8];
    let n = clock(15 * 60 + 5, true, &mut b);
    assert_eq!(&b[..n], b"3:05 PM");
    let n = clock(5, true, &mut b);
    assert_eq!(&b[..n], b"12:05 AM");
    let n = clock(15 * 60 + 5, false, &mut b);
    assert_eq!(&b[..n], b"15:05");
    // Tomorrow: the same time, the day after.
    let id = me(&t).id;
    t.act(&cx, id, 0);
    draw(&mut t, &cx);
    assert_eq!((me(&t).due, me(&t).time), (Some(19_724), Some(15 * 60)));
    assert_eq!(kept(pim), Some(date::start_of(19_724) + 15 * 60));
    // A task with no time has none.
    typed(&mut t, &cx, b"Water the plants");
    draw(&mut t, &cx);
    assert!(
        t.tasks
            .iter()
            .flatten()
            .any(|k| k.title.get() == b"Water the plants" && k.time.is_none())
    );
}

#[test]
fn a_task_s_time_is_set_by_keys_and_a_list_s_color_shows() {
    // The owner on the X51v, 2026-11-15: "no way to assign a time still in
    // lists"; "colour doesnt do anything in list".
    let pim = store();
    let cx = ctx(pim);
    let mut t = opened(19_723);
    t.show(View::List(None));
    draw(&mut t, &cx);
    typed(&mut t, &cx, b"Call the bank today");
    t.event(Event::Key(Key::SoftLeft), &cx);
    draw(&mut t, &cx);
    let me = |t: &Tasks| *t.tasks.iter().flatten().next().unwrap();
    assert_eq!(me(&t).time, None);
    // Its actions shown: the time keys under them, in the D-pad's reach.
    let id = me(&t).id;
    t.revealed = Some(id);
    draw(&mut t, &cx);
    let (ts, n) = t.list_targets();
    assert!(t.time_keys.iter().all(|k| ts[..n].contains(k)));
    // A touch on the time row brings the pad; 3, 0, 7 and PM by the
    // keyboard's digits and the pad's keys; the softkey sets it.
    let row = t.time_keys[0];
    tap(&mut t, &cx, row);
    assert!(t.pad.is_some());
    assert_eq!(t.softkeys(), ("Cancel", "Set"));
    for c in *b"307" {
        t.event(Event::Char(c), &cx);
    }
    t.event(Event::Key(Key::SoftRight), &cx);
    assert!(t.pad.is_none());
    draw(&mut t, &cx);
    assert_eq!(me(&t).time, Some(3 * 60 + 7), "3:07, to the minute");
    assert_eq!(t.revealed, Some(id), "the task stays open");
    let g = pim.borrow();
    let at = g.as_ref().unwrap().get(id).unwrap().num(tag::REMIND_AT);
    drop(g);
    assert_eq!(at, Some(date::start_of(19_723) + 3 * 60 + 7));
    // The D-pad steps it a minute; Cancel leaves it as it was.
    tap(&mut t, &cx, row);
    t.event(Event::Key(Key::Right), &cx);
    t.event(Event::Key(Key::Up), &cx);
    t.event(Event::Key(Key::Enter), &cx);
    draw(&mut t, &cx);
    assert_eq!(me(&t).time, Some(3 * 60 + 8));
    tap(&mut t, &cx, row);
    t.event(Event::Key(Key::Up), &cx);
    t.event(Event::Key(Key::SoftLeft), &cx);
    draw(&mut t, &cx);
    assert_eq!(me(&t).time, Some(3 * 60 + 8));
    assert_eq!(
        t.view,
        View::List(None),
        "Cancel leaves the pad, not the list"
    );
    // The lists' COLOR key is drawn in the list's colour.
    t.event(Event::PenDown { x: 5, y: 630 }, &cx);
    t.event(Event::PenUp { x: 5, y: 630 }, &cx);
    t.view = View::Lists;
    let mut f = Box::new(Frame::new());
    t.draw(&mut f, &cx);
    let key = t.list_keys[..t.n_keys]
        .iter()
        .find(|k| k.1 == 0 && k.2 == 1)
        .unwrap()
        .0;
    let colour = t.lists[0].colour;
    tap(&mut t, &cx, key);
    draw(&mut t, &cx);
    assert_ne!(t.lists[0].colour, colour);
}

#[test]
fn a_touch_on_a_task_opens_its_time_keys() {
    // The owner on the X51v, 2026-11-15: "Still no way to assign a time
    // inside tasks!" (the keys were there, behind a swipe).
    let pim = store();
    let cx = ctx(pim);
    let mut t = opened(19_723);
    t.show(View::List(None));
    draw(&mut t, &cx);
    typed(&mut t, &cx, b"Call the bank");
    t.event(Event::Key(Key::SoftLeft), &cx);
    draw(&mut t, &cx);
    let id = t.tasks.iter().flatten().next().unwrap().id;
    assert_eq!(t.revealed, None);
    // The row, right of its circle.
    let row = t.rows_at[0].0;
    tap(&mut t, &cx, Rect::new(row.x + 120, row.y + 4, 10, 10));
    assert_eq!(t.revealed, Some(id));
    draw(&mut t, &cx);
    let row = t.time_keys[0];
    tap(&mut t, &cx, row);
    assert!(t.pad.is_some(), "the time pad");
    t.event(Event::Key(Key::SoftRight), &cx);
    draw(&mut t, &cx);
    assert_eq!(t.tasks.iter().flatten().next().unwrap().time, Some(9 * 60));
    // Its circle still ticks it.
    let mut t = opened(19_723);
    t.show(View::List(None));
    draw(&mut t, &cx);
    let circle = t.boxes[0].0;
    tap(&mut t, &cx, circle);
    assert_eq!(t.revealed, None);
}

/// tasks-rule0.md's repeats as typed: every N, weekdays named (not taken
/// as the due date), and after done.
#[test]
fn the_typed_line_reads_every_kind_of_repeat() {
    use rp_pim::repeat::{Every, Rule};
    let pim = store();
    let cx = ctx(pim);
    let mut t = opened(19_723);
    t.show(View::List(None));
    draw(&mut t, &cx);
    typed(&mut t, &cx, b"water plants every 3 days after done");
    typed(&mut t, &cx, b"gym every tue and thu");
    typed(&mut t, &cx, b"bins every other week fri");
    let all = tasks(pim);
    assert_eq!(all.len(), 3);
    let rule =
        |k: usize| Rule::unpack(all[k].4.expect("a repeat"), all[k].2.unwrap() as i32).unwrap();
    assert_eq!(all[0].0, b"Water plants");
    assert_eq!(all[0].2, Some(19_723));
    let r = rule(0);
    assert_eq!((r.every, r.n, r.after), (Every::Day, 3, true));
    // Tuesday and Thursday: due the first of them, Tuesday the 2nd.
    assert_eq!(all[1].0, b"Gym");
    assert_eq!(all[1].2, Some(19_724));
    let r = rule(1);
    assert_eq!((r.every, r.days), (Every::Week, 0b1010));
    // A date after the repeat still reads as the due date.
    assert_eq!(all[2].0, b"Bins");
    assert_eq!(all[2].2, Some(19_727));
    assert_eq!((rule(2).every, rule(2).n), (Every::Week, 2));
}

/// A repeating task ticked moves to its next day (no copy made), the day
/// it was done counted in its log, its steps opened again; once its rule
/// has ended a tick completes it.
#[test]
fn a_repeating_task_ticked_moves_on_and_counts_the_day() {
    use rp_pim::repeat::{Every, Rule};
    let pim = store();
    let cx = ctx(pim);
    let today = 19_723;
    let daily = Rule {
        every: Every::Day,
        n: 1,
        days: 0,
        until: Some(today + 1),
        after: false,
    }
    .pack(today - 2)
    .to_le_bytes();
    let id = {
        let mut g = pim.borrow_mut();
        let s = g.as_mut().unwrap();
        // Due two days ago: ticked today, it comes back tomorrow.
        let id = s
            .put(
                Kind::Task,
                None,
                &[
                    (tag::TITLE, b"Stretch"),
                    (tag::DUE, &((today - 2) as u32).to_le_bytes()),
                    (tag::REPEAT, &daily),
                    (
                        tag::REMIND_AT,
                        &(date::start_of(today - 2) + 450).to_le_bytes(),
                    ),
                ],
            )
            .unwrap();
        s.put(
            Kind::Task,
            None,
            &[
                (tag::TITLE, b"Neck"),
                (tag::PARENT, &id.to_le_bytes()),
                (tag::DONE, &1u32.to_le_bytes()),
            ],
        )
        .unwrap();
        id
    };
    let mut t = opened(today);
    draw(&mut t, &cx);
    t.tick(&cx, id);
    {
        let g = pim.borrow();
        let s = g.as_ref().unwrap();
        assert_eq!(s.all(Kind::Task).count(), 2, "no copy made");
        let r = s.get(id).unwrap();
        assert_eq!(r.num(tag::DONE), None);
        assert_eq!(r.num(tag::DUE), Some((today + 1) as u32));
        // Its time went with it, and its end is the same day.
        assert_eq!(r.num(tag::REMIND_AT), Some(date::start_of(today + 1) + 450));
        // Tomorrow is its last day: a plain task then.
        assert_eq!(r.num(tag::REPEAT), None);
        assert!(Log::of(r.text(tag::DONE_LOG)).has(today));
        let step = s
            .all(Kind::Task)
            .find(|c| c.num(tag::PARENT) == Some(id))
            .unwrap();
        assert_eq!(step.num(tag::DONE), None, "its step opened again");
    }
    // The home counts today's completion from the log.
    draw(&mut t, &cx);
    let done_today = t
        .tasks
        .iter()
        .flatten()
        .filter(|x| x.log.has(today))
        .count();
    assert_eq!(done_today, 1);
    // Tomorrow is its last day: ticked then, it is done.
    let mut cx2 = cx;
    cx2.now.day = today + 1;
    t.tick(&cx2, id);
    let g = pim.borrow();
    let r = g.as_ref().unwrap().get(id).unwrap();
    assert!(r.num(tag::DONE).is_some());
}

/// The log keeps the newest days and drops the oldest past its size.
#[test]
fn the_done_log_keeps_the_newest_days() {
    let mut log = Log::default();
    for d in 0..40 {
        let (b, n) = log.with(1000 + d);
        log = Log::of(&b[..n]);
    }
    assert!(log.has(1039) && log.has(1008) && !log.has(1007));
}
