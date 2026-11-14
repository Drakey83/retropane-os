//! The calculator's tape driven as the stylus drives it.

extern crate std;

use super::*;
use rp_app::{Argb, Fonts, Look, Now, PanelArt, Shapes};
use rp_draw::{Font, Image};
use std::boxed::Box;

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

fn ctx() -> Ctx<'static> {
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
        clip: Box::leak(Box::new(core::cell::RefCell::new(Text::EMPTY))),
        pim: None,
    }
}

/// A calculator whose tape holds 1 + 0 = 1 up to n + 0 = n.
fn with_tape(n: usize) -> Calculator {
    let mut c = Calculator::new(Mode::Basic, 19_723);
    for k in 1..=n {
        let d = std::format!("{k}");
        c.push(d.as_bytes());
        c.push(b"+0");
        c.press(b"=");
        c.n = 0;
    }
    c
}

/// The tape lines' answers shown, top to bottom.
fn shown(c: &Calculator) -> std::vec::Vec<std::string::String> {
    let (first, n) = c.tape_window();
    c.tape[first..first + n]
        .iter()
        .map(|(_, a)| std::string::String::from_utf8_lossy(a.get()).into_owned())
        .collect()
}

fn line_y(k: u16) -> u16 {
    TOP + k * TAPE_LINE + 20
}

const X: u16 = W - MARGIN - 40;

#[test]
fn the_tape_keeps_24_lines_and_shows_the_newest_three() {
    let c = with_tape(30);
    assert_eq!(c.n_tape, TAPE_MAX);
    assert_eq!(shown(&c), ["28", "29", "30"]);
}

#[test]
fn a_drag_down_scrolls_back_and_stops_at_the_oldest() {
    let cx = ctx();
    let mut c = with_tape(10);
    c.event(Event::PenDown { x: X, y: line_y(0) }, &cx);
    c.event(
        Event::PenMove {
            x: X,
            y: line_y(0) + 2 * TAPE_LINE,
        },
        &cx,
    );
    assert_eq!(shown(&c), ["6", "7", "8"]);
    // The drag ends without reusing the line it started on.
    let before = c.n;
    c.event(Event::PenUp { x: X, y: line_y(2) }, &cx);
    assert_eq!(c.n, before);
    // Far past the oldest: it stops there; and back up to the newest.
    c.event(Event::PenDown { x: X, y: TOP + 2 }, &cx);
    c.event(Event::PenMove { x: X, y: 600 }, &cx);
    assert_eq!(shown(&c), ["1", "2", "3"]);
    c.event(Event::PenUp { x: X, y: 600 }, &cx);
    c.event(Event::PenDown { x: X, y: line_y(2) }, &cx);
    // Up 192 pixels: four lines newer (a drag moves as far as the stylus).
    c.event(Event::PenMove { x: X, y: 0 }, &cx);
    assert_eq!(shown(&c), ["5", "6", "7"]);
    c.event(Event::PenUp { x: X, y: 0 }, &cx);
    // A new answer brings it back to the newest.
    c.event(Event::PenDown { x: X, y: line_y(0) }, &cx);
    c.event(
        Event::PenMove {
            x: X,
            y: line_y(0) + TAPE_LINE,
        },
        &cx,
    );
    c.event(Event::PenUp { x: X, y: line_y(1) }, &cx);
    c.push(b"5");
    c.press(b"=");
    assert_eq!(shown(&c), ["9", "10", "5"]);
}

#[test]
fn a_tap_reuses_a_line_and_a_hold_copies_it_without_reusing() {
    let cx = ctx();
    let mut c = with_tape(4);
    // Scrolled back one (to the oldest): the lines are 1, 2, 3.
    c.back = 1;
    c.event(Event::PenDown { x: X, y: line_y(1) }, &cx);
    c.event(Event::PenUp { x: X, y: line_y(1) }, &cx);
    assert_eq!(&c.expr[..c.n], b"2");
    c.n = 0;
    // Held half a second: copied, marked, not typed.
    c.event(Event::PenDown { x: X, y: line_y(0) }, &cx);
    for _ in 0..HOLD_TICKS - 1 {
        assert_eq!(c.event(Event::Tick, &cx), Reply::Animate);
    }
    assert!(
        cx.clip.borrow().get().is_empty(),
        "not before the hold is long enough"
    );
    c.event(Event::Tick, &cx);
    assert_eq!(cx.clip.borrow().get(), b"1");
    assert_eq!(c.copied.map(|(l, _)| l), Some(0));
    c.event(Event::PenUp { x: X, y: line_y(0) }, &cx);
    assert_eq!(c.n, 0, "a hold doesn't also type the answer");
    // The mark goes after about a second, and the ticks stop.
    let mut ticks = 0;
    while c.event(Event::Tick, &cx) == Reply::Animate {
        ticks += 1;
        assert!(ticks < 100);
    }
    assert!(c.copied.is_none());
    assert_eq!(ticks, usize::from(COPIED_TICKS) - 1);
}

#[test]
fn a_shaky_tap_still_reuses_and_a_long_answer_copies_without_grouping() {
    let cx = ctx();
    let mut c = Calculator::new(Mode::Basic, 19_723);
    c.push(b"1234567");
    c.press(b"=");
    c.n = 0;
    c.event(Event::PenDown { x: X, y: line_y(0) }, &cx);
    c.event(
        Event::PenMove {
            x: X,
            y: line_y(0) + SLOP,
        },
        &cx,
    );
    c.event(
        Event::PenUp {
            x: X,
            y: line_y(0) + SLOP,
        },
        &cx,
    );
    assert_eq!(&c.expr[..c.n], b"1234567");
    c.n = 0;
    c.event(Event::PenDown { x: X, y: line_y(0) }, &cx);
    for _ in 0..HOLD_TICKS {
        c.event(Event::Tick, &cx);
    }
    assert_eq!(cx.clip.borrow().get(), b"1234567");
}

#[test]
fn the_tape_draws_its_scroll_mark_only_when_it_scrolls() {
    let cx = ctx();
    let fills = |c: &mut Calculator| {
        let mut f = Box::new(Frame::new());
        c.draw(&mut f, &cx);
        f.cmds()
            .filter(|k| matches!(k, rp_app::Cmd::Fill { r, .. } if r.x == MARGIN + 132 && r.w == 4))
            .count()
    };
    assert_eq!(fills(&mut with_tape(3)), 0);
    assert_eq!(fills(&mut with_tape(5)), 2);
}
