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
        clock12: true,
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

/// The arrows that take the focus from `from` to `to` among `t` (breadth
/// first), `None` where no arrows do.
fn way(t: &[Rect], from: Rect, to: Rect) -> Option<std::vec::Vec<Key>> {
    let mut seen = std::vec![from];
    let mut queue = std::collections::VecDeque::from([(from, std::vec::Vec::new())]);
    while let Some((at, path)) = queue.pop_front() {
        if at == to {
            return Some(path);
        }
        for k in [Key::Up, Key::Down, Key::Left, Key::Right] {
            let mut f = kit::Focus::default();
            f.set(at);
            if f.step(k, t.iter().copied()) {
                let next = t.iter().copied().find(|&r| kit::Focus::at(r) == f.point());
                if let Some(next) = next.filter(|n| !seen.contains(n)) {
                    seen.push(next);
                    let mut p = path.clone();
                    p.push(k);
                    queue.push_back((next, p));
                }
            }
        }
    }
    None
}

/// R203, the class: in every mode, everything a stylus can tap is within
/// the arrows' reach from the mode's chip.
#[test]
fn the_dpad_reaches_every_key_chip_and_tape_line_in_every_mode() {
    for (m, &mode) in MODE_LIST.iter().enumerate() {
        let mut c = with_tape(5);
        c.mode = mode;
        c.n_rates = 2;
        let (t, n) = c.targets();
        assert!(n > 6, "{mode:?}");
        for &r in &t[..n] {
            assert!(way(&t[..n], t[m], r).is_some(), "{mode:?}: no way to {r:?}");
        }
    }
}

#[test]
fn the_dpad_steps_the_modes_types_on_the_keys_and_walks_the_tape() {
    let cx = ctx();
    // Left and right step the modes, as they always did.
    let mut c = Calculator::new(Mode::Basic, 19_723);
    c.event(Event::Key(Key::Right), &cx);
    assert_eq!(c.mode, Mode::Scientific);
    c.event(Event::Key(Key::Left), &cx);
    assert_eq!(c.mode, Mode::Basic);
    // To the 7 key and the centre: it types, twice, and = works the sum.
    let (t, _) = c.targets();
    let seven = Calculator::key_rect(c.keypad_top(), 2, 0);
    for k in way(&t, t[0], seven).unwrap() {
        c.event(Event::Key(k), &cx);
    }
    assert_eq!(c.mode, Mode::Basic);
    c.event(Event::Key(Key::Enter), &cx);
    c.event(Event::Key(Key::Enter), &cx);
    assert_eq!(&c.expr[..c.n], b"77");
    let (t, n) = c.targets();
    let equals = (0..6)
        .flat_map(|r| (0..4).map(move |col| (r, col)))
        .find(|&(r, col)| BASIC[r][col].1 == b"=")
        .map(|(r, col)| Calculator::key_rect(c.keypad_top(), r, col))
        .unwrap();
    for k in way(&t[..n], seven, equals).unwrap() {
        c.event(Event::Key(k), &cx);
    }
    c.event(Event::Key(Key::Enter), &cx);
    assert_eq!(shown(&c), ["77"]);
    // The focus is on = still, where the new tape line moved it.
    let moved = (0..6)
        .flat_map(|r| (0..4).map(move |col| (r, col)))
        .find(|&(r, col)| BASIC[r][col].1 == b"=")
        .map(|(r, col)| Calculator::key_rect(c.keypad_top(), r, col))
        .unwrap();
    assert_ne!(moved, equals);
    assert_eq!(c.focus.point(), kit::Focus::at(moved));
    // On the tape's top line, up brings older lines in before leaving it.
    let mut c = with_tape(6);
    let (t, n) = c.targets();
    for k in way(&t[..n], t[0], c.tape_rect(0)).unwrap() {
        c.event(Event::Key(k), &cx);
    }
    assert_eq!(shown(&c), ["4", "5", "6"]);
    c.event(Event::Key(Key::Up), &cx);
    assert_eq!(shown(&c), ["3", "4", "5"]);
    assert_eq!(c.focus.point(), kit::Focus::at(c.tape_rect(0)));
    // The centre on a line reuses its answer, as a tap does.
    c.event(Event::Key(Key::Enter), &cx);
    assert_eq!(&c.expr[..c.n], b"3");
    // A tap takes over: no ring left behind.
    c.event(Event::PenDown { x: 5, y: 5 }, &cx);
    assert_eq!(c.focus.point(), None);
}

#[test]
fn a_number_alone_is_shown_once_and_equals_adds_no_line() {
    // The owner's photo, 2026-11-15: "25" small over "25" large, and the
    // tape filling with 25s.
    let mut c = Calculator::new(Mode::Basic, 19_723);
    c.push(b"5x5");
    assert!(c.is_sum());
    c.press(b"=");
    assert_eq!(shown(&c), ["25"]);
    assert!(!c.is_sum(), "the answer alone");
    for _ in 0..3 {
        c.press(b"=");
    }
    assert_eq!(shown(&c), ["25"], "no line for a number alone");
    // A sum on the answer is a sum again.
    c.push(b"+1");
    c.press(b"=");
    assert_eq!(shown(&c), ["25", "26"]);
    // A negative number alone is still a number.
    let mut c = Calculator::new(Mode::Basic, 19_723);
    c.push(b"~7");
    assert!(!c.is_sum());
}

#[test]
fn the_answer_is_large_and_the_sum_s_line_stays_clear_of_it() {
    // The owner on the X51v, 2026-11-15: first the small and the large
    // number ran into each other, then the large one was gone.
    use rp_app::Cmd;
    static WIDE_FONT: Font = Font {
        line: 101,
        ascent: 80,
        first: 32,
        glyphs: &[],
    };
    let mut cx = ctx();
    let fonts = Fonts {
        big: &WIDE_FONT,
        ..FONTS
    };
    cx.fonts = Box::leak(Box::new(fonts));
    let mut c = Calculator::new(Mode::Basic, 19_723);
    c.push(b"5x5");
    let mut f = Box::new(Frame::new());
    c.draw(&mut f, &cx);
    let lines: std::vec::Vec<(u16, bool)> = f
        .cmds()
        .filter_map(|c| match *c {
            Cmd::Text { baseline, font, .. } => Some((baseline, core::ptr::eq(font, &WIDE_FONT))),
            _ => None,
        })
        .collect();
    let big = lines
        .iter()
        .find(|l| l.1)
        .expect("the answer in the large numerals")
        .0;
    // The sum's line: the last text above the answer's digits (53 px
    // tall), with its own descent to spare.
    let above = lines
        .iter()
        .filter(|l| !l.1 && l.0 < big)
        .map(|l| l.0)
        .max()
        .unwrap();
    assert!(above + 6 <= big - 53, "sum at {above}, answer at {big}");
}
