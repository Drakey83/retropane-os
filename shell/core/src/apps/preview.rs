//! Where the organiser apps' controls are, for the PC's previews and the
//! emulator check (`cargo xtask ui-preview`, `emu-today`): they tap by
//! these, so a moved control moves its taps with it. Nothing on the device
//! uses this.

use rp_draw::Rect;
use rp_pim::calc::{Key, Op};

use super::{MARGIN, PITCH, ROW, TOP, View, WIDE};
use crate::keyboard::{CAPS, Cap, Keyboard};
use crate::{Target, W};

/// The search or quick-add field under the header.
pub const FIELD: Rect = super::FIELD;
/// Quick add's "More details" row.
pub const MORE: Rect = Rect::new(MARGIN, FIELD.y + 46 + 70, WIDE, ROW);

/// The Calendar header's chip for a view.
#[must_use]
pub fn chip(v: View) -> Rect {
    let j = match v {
        View::Day => 1,
        View::Month => 2,
        _ => 0,
    };
    Rect::new(206 + j * 88, 50, 84, 30)
}

/// Cell `c` (0-41) of the month grid.
#[must_use]
pub const fn month_cell(c: u16) -> Rect {
    Rect::new(MARGIN + c % 7 * 64, TOP + 64 + c / 7 * 44, 64, 44)
}

/// Row `k` of a form, unscrolled.
#[must_use]
pub const fn form_row(k: u16) -> Rect {
    Rect::new(MARGIN, TOP + k * PITCH, WIDE, ROW)
}

/// Appointment `k` of the agenda's first day (under its label),
/// unscrolled.
#[must_use]
pub const fn agenda_row(k: u16) -> Rect {
    Rect::new(MARGIN, TOP + 44 + 24 + k * PITCH, WIDE, ROW)
}

/// Row `k` of the Contacts or Tasks list, unscrolled (a task's row right
/// of its box).
#[must_use]
pub const fn list_row(k: u16) -> Rect {
    Rect::new(
        MARGIN + 60,
        FIELD.y + FIELD.h + 6 + k * PITCH,
        WIDE - 100,
        ROW,
    )
}

/// Task `k`'s tick box, unscrolled.
#[must_use]
pub const fn task_box(k: u16) -> Rect {
    Rect::new(MARGIN, FIELD.y + FIELD.h + 6 + k * PITCH, 44, ROW)
}

/// Row `k` of the Notes list, unscrolled.
#[must_use]
pub const fn note_row(k: u16) -> Rect {
    Rect::new(MARGIN, FIELD.y + FIELD.h + 6 + k * 56, WIDE, 52)
}

/// The Contacts strip's letter (`b'A'`..=`b'Z'`).
#[must_use]
pub fn letter(c: u8) -> Rect {
    Rect::new(
        W - 24,
        FIELD.y + FIELD.h + 6 + u16::from(c.to_ascii_uppercase().saturating_sub(b'A')) * 17,
        22,
        17,
    )
}

/// The calculator's key for a sign: digits, `. + - x / = %`, `C`, `<`
/// (delete), `n` (plus or minus).
#[must_use]
pub fn calc_key(c: char) -> Option<Rect> {
    let key = match c {
        '0'..='9' => Key::Digit(c as u8 - b'0'),
        '.' => Key::Point,
        '+' => Key::Op(Op::Add),
        '-' => Key::Op(Op::Sub),
        'x' => Key::Op(Op::Mul),
        '/' => Key::Op(Op::Div),
        '=' => Key::Equals,
        '%' => Key::Percent,
        'C' => Key::Clear,
        '<' => Key::Delete,
        'n' => Key::Negate,
        _ => return None,
    };
    super::calculator::key_place(key)
}

/// The keys to tap, in order, to type `c` on the on-screen keyboard from
/// its letters layer (a sign on the symbols layer takes three: to it, the
/// key, and back), and how many.
#[must_use]
pub fn key_taps(c: u8) -> Option<([Rect; 3], usize)> {
    let letters = Keyboard::default();
    if let Some(r) = letters.cap_of(c).and_then(|i| letters.cap(i)) {
        return Some(([r.1; 3], 1));
    }
    let symbols = Keyboard {
        symbols: true,
        ..Keyboard::default()
    };
    let switch = |k: &Keyboard| {
        (0..CAPS)
            .filter_map(|i| k.cap(i))
            .find(|(cap, _)| *cap == Cap::Symbols)
            .map(|(_, r)| r)
    };
    let key = symbols.cap(symbols.cap_of(c)?)?.1;
    Some(([switch(&letters)?, key, switch(&symbols)?], 3))
}

/// The target an app control is, for a test that reads the focus.
#[must_use]
pub const fn target(i: u8) -> Target {
    Target::App(i)
}
