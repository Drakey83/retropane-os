//! Setting a time to the minute, and an alert's lead (the owner on the
//! X51v, 2026-11-15: "setting tasks and calendar alerts by time should be
//! able to be selected by the min not by every 15 mins ... look at how
//! other programs handle this").
//!
//! Iterate, don't imitate (rule 0). How others do it:
//! - **Android (Material)**: a clock dial, or its keyboard mode: an hour
//!   field and a minute field typed into, an AM/PM switch beside them.
//! - **iOS**: wheels (Calendar's step five minutes); a tap on the time
//!   lets it be typed.
//! - **Windows Mobile 5**: the time as parts (hour, minute, AM/PM), the
//!   one chosen stepped by a spinner or the D-pad.
//! - **Google Calendar's alerts**: a number and a unit (minutes, hours,
//!   days) "before".
//! - The first RetroPane build: keys a quarter of an hour earlier and
//!   later (nobody's design, and no way to say 3:07).
//!
//! Ours: Material's keyboard mode, since a stylus on a 3.7 inch screen
//! types four digits faster and more exactly than it turns a dial or a
//! wheel: two fields and a number pad, three taps for "3:07"; the hour
//! moves on to the minutes by itself when it can only be complete; quick
//! keys for the minutes most asked for; and Windows Mobile's D-pad on top
//! (left and right choose the part, up and down step it by one, held to
//! run). The same pad sets an alert's lead as Google's does: a number and
//! its unit. One piece in the kit, so Tasks and Calendar are the same.

use crate::kit::{self, DIM, KeyState, TEXT};
use crate::{Ctx, Event, Frame, Key, MARGIN, Rect, W, WIDE};

/// What the pad sets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// A time of day, to the minute.
    Clock,
    /// How long before (an alert): a number and a unit.
    Before,
}

/// What a press on the pad came to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Answer {
    /// Nothing of the pad's.
    Nothing,
    /// The pad changed: draw it again.
    Redraw,
    /// Left without setting.
    Cancel,
    /// Set: minutes from midnight (Clock) or minutes before (Before);
    /// `None`: no time, no alert.
    Set(Option<u32>),
}

/// The pad's state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TimePad {
    mode: Mode,
    /// Clock: the hour (1-12, or 0-23 on the 24-hour clock) and the
    /// minute; Before: the number in `a`.
    a: u16,
    b: u16,
    pm: bool,
    /// Clock: the 24-hour clock (R373): the hour 0-23 typed in full, no AM
    /// and PM keys.
    h24: bool,
    /// Before: 0 minutes, 1 hours, 2 days.
    unit: u8,
    /// The part typed into: 0 the hour (or the number), 1 the minute.
    part: u8,
    /// The part's digits typed so far (0: the next digit starts it anew).
    typed: u8,
    /// The key under the D-pad, if it is among the keys.
    focus: Option<u8>,
}

const TOP: u16 = 44;
/// The two fields and the switch beside them.
const HOUR: Rect = Rect::new(MARGIN + 40, TOP + 40, 120, 84);
const MINUTE: Rect = Rect::new(MARGIN + 190, TOP + 40, 120, 84);
const SIDE: [Rect; 3] = [
    Rect::new(MARGIN + 330, TOP + 40, 102, 26),
    Rect::new(MARGIN + 330, TOP + 69, 102, 26),
    Rect::new(MARGIN + 330, TOP + 98, 102, 26),
];
/// The number pad: three across, four down.
const PAD: Rect = Rect::new(MARGIN, TOP + 144, 318, 252);
/// The quick keys beside it, four down.
const QUICK: Rect = Rect::new(MARGIN + 330, TOP + 144, 118, 252);
/// The foot: no time, cancel, set.
const FOOT: Rect = Rect::new(MARGIN, TOP + 410, WIDE, 48);

const DIGITS: [&[u8]; 12] = [
    b"1", b"2", b"3", b"4", b"5", b"6", b"7", b"8", b"9", b"CLEAR", b"0", b"DEL",
];
const QUICK_MIN: [(&[u8], u16); 4] = [(b":00", 0), (b":15", 15), (b":30", 30), (b":45", 45)];
const QUICK_BEFORE: [(&[u8], u16, u8); 4] = [
    (b"AT START", 0, 0),
    (b"15 MIN", 15, 0),
    (b"1 HOUR", 1, 1),
    (b"1 DAY", 1, 2),
];

fn cell(r: Rect, cols: u16, rows: u16, i: u16) -> Rect {
    let (w, h) = ((r.w - 6 * (cols - 1)) / cols, (r.h - 6 * (rows - 1)) / rows);
    Rect::new(r.x + (i % cols) * (w + 6), r.y + (i / cols) * (h + 6), w, h)
}

impl TimePad {
    /// The pad on a time of day (`None`: 9:00 AM to start from), on the
    /// 12-hour clock (`clock12`, [`Ctx::clock12`]) or the 24-hour one.
    #[must_use]
    pub fn clock(minutes: Option<u16>, clock12: bool) -> Self {
        let m = minutes.unwrap_or(9 * 60) % 1440;
        Self {
            mode: Mode::Clock,
            a: if clock12 {
                match (m / 60) % 12 {
                    0 => 12,
                    h => h,
                }
            } else {
                m / 60
            },
            b: m % 60,
            pm: m >= 720,
            h24: !clock12,
            unit: 0,
            part: 0,
            typed: 0,
            focus: None,
        }
    }

    /// The pad on an alert's lead in minutes (`None`: 15 minutes to start
    /// from): shown in the largest unit that holds it whole.
    #[must_use]
    pub fn before(minutes: Option<u32>) -> Self {
        let m = minutes.unwrap_or(15);
        let (a, unit) = if m != 0 && m.is_multiple_of(1440) {
            (m / 1440, 2)
        } else if m != 0 && m.is_multiple_of(60) {
            (m / 60, 1)
        } else {
            (m, 0)
        };
        Self {
            mode: Mode::Before,
            a: a.min(999) as u16,
            b: 0,
            pm: false,
            h24: false,
            unit,
            part: 0,
            typed: 0,
            focus: None,
        }
    }

    /// What the pad holds now: minutes from midnight, or minutes before.
    #[must_use]
    pub fn value(&self) -> u32 {
        match self.mode {
            Mode::Clock if self.h24 => u32::from(self.a % 24) * 60 + u32::from(self.b),
            Mode::Clock => {
                (u32::from(self.a % 12) + if self.pm { 12 } else { 0 }) * 60 + u32::from(self.b)
            }
            Mode::Before => u32::from(self.a) * [1, 60, 1440][usize::from(self.unit.min(2))],
        }
    }

    /// Every key, in the D-pad's order, with what it is.
    fn keys(&self) -> impl Iterator<Item = (Rect, u8)> + '_ {
        let fields = if self.mode == Mode::Clock { 2 } else { 1 };
        [HOUR, MINUTE]
            .into_iter()
            .take(fields)
            .zip(0u8..)
            .chain(
                (0..match self.mode {
                    Mode::Clock if self.h24 => 0,
                    Mode::Clock => 2,
                    Mode::Before => 3,
                })
                    .map(|k| (self.side(k), 10 + k as u8)),
            )
            .chain((0..12).map(|i| (cell(PAD, 3, 4, i), 20 + i as u8)))
            .chain((0..4).map(|i| (cell(QUICK, 1, 4, i), 40 + i as u8)))
            .chain((0..3).map(|i| (cell(FOOT, 3, 1, i), 50 + i as u8)))
    }

    /// Key `k` beside the fields: AM and PM (two, the fields' height
    /// between them), or the three units.
    fn side(&self, k: usize) -> Rect {
        match self.mode {
            Mode::Clock => Rect::new(SIDE[0].x, HOUR.y + k as u16 * 44, SIDE[0].w, 40),
            Mode::Before => SIDE[k.min(2)],
        }
    }

    /// A digit typed into the part chosen.
    fn digit(&mut self, d: u16) {
        match (self.mode, self.part) {
            (Mode::Clock, 0) if self.h24 => {
                // The hour 0-23: a second digit after a 0 or a 1, or after
                // a 2 when it is 0-3; any other digit is the hour whole, and
                // an hour complete moves on to the minutes ("7" is 07; "2"
                // then "5" is 02, the 5 the first of the minutes).
                if self.typed == 1 && (self.a <= 1 || (self.a == 2 && d <= 3)) {
                    self.a = self.a * 10 + d;
                    self.next_part();
                } else if self.typed == 1 {
                    self.next_part();
                    self.digit(d);
                } else {
                    self.a = d;
                    self.typed = 1;
                    if d > 2 {
                        self.next_part();
                    }
                }
            }
            (Mode::Clock, 0) => {
                // The hour: a second digit only after a 1 (10, 11, 12);
                // complete, it moves on to the minutes.
                if self.typed == 1 && self.a == 1 && d <= 2 {
                    self.a = 10 + d;
                    self.next_part();
                } else if self.typed == 1 && self.a == 1 {
                    // 1 then 5: one o'clock, and the 5 is the minutes'.
                    self.next_part();
                    self.digit(d);
                } else if d == 0 {
                    // (No hour starts with 0 on a twelve-hour clock.)
                } else {
                    self.a = d;
                    self.typed = 1;
                    if d != 1 {
                        self.next_part();
                    }
                }
            }
            (Mode::Clock, _) => {
                if self.typed == 1 && self.b < 6 {
                    self.b = self.b * 10 + d;
                    self.typed = 0;
                } else {
                    self.b = d;
                    self.typed = 1;
                }
            }
            (Mode::Before, _) => {
                self.a = if self.typed == 0 {
                    d
                } else {
                    (self.a * 10 + d).min(999)
                };
                self.typed = 1;
            }
        }
    }

    fn next_part(&mut self) {
        self.part = 1;
        self.typed = 0;
    }

    /// The part chosen stepped by `by` (the D-pad's up and down), round
    /// its range.
    fn step(&mut self, by: i16) {
        self.typed = 0;
        match (self.mode, self.part) {
            (Mode::Clock, 0) if self.h24 => self.a = (self.a as i16 + by).rem_euclid(24) as u16,
            (Mode::Clock, 0) => {
                let h = (self.a as i16 - 1 + by).rem_euclid(12) + 1;
                // (Through 12 the half of the day turns, as a clock's.)
                if (self.a == 11 && h == 12) || (self.a == 12 && h == 11) {
                    self.pm = !self.pm;
                }
                self.a = h as u16;
            }
            (Mode::Clock, _) => self.b = (self.b as i16 + by).rem_euclid(60) as u16,
            (Mode::Before, _) => self.a = (self.a as i16 + by).clamp(0, 999) as u16,
        }
    }

    /// A key pressed.
    fn press(&mut self, key: u8) -> Answer {
        match key {
            0 | 1 => {
                self.part = key;
                self.typed = 0;
            }
            10..=12 => match self.mode {
                Mode::Clock if self.h24 => {}
                Mode::Clock => self.pm = key == 11,
                Mode::Before => self.unit = key - 10,
            },
            29 => {
                // CLEAR: the part chosen back to nothing typed.
                match (self.mode, self.part) {
                    (Mode::Clock, 0) => self.a = if self.h24 { 0 } else { 12 },
                    (Mode::Clock, _) => self.b = 0,
                    (Mode::Before, _) => self.a = 0,
                }
                self.typed = 0;
            }
            31 => {
                // Back a digit.
                match (self.mode, self.part) {
                    (Mode::Clock, 0) if self.h24 => self.a /= 10,
                    (Mode::Clock, 0) => self.a = if self.a >= 10 { 1 } else { 12 },
                    (Mode::Clock, _) => self.b /= 10,
                    (Mode::Before, _) => self.a /= 10,
                }
                // (A first digit left that can take a second: waiting for it.)
                self.typed = u8::from(
                    self.mode == Mode::Clock
                        && self.part == 0
                        && (self.a == 1 || (self.h24 && self.a <= 2)),
                );
            }
            30 => self.digit(0),
            20..=28 => self.digit(u16::from(key - 19)),
            40..=43 => {
                let i = usize::from(key - 40);
                match self.mode {
                    Mode::Clock => {
                        self.b = QUICK_MIN[i].1;
                        self.part = 1;
                    }
                    Mode::Before => (self.a, self.unit) = (QUICK_BEFORE[i].1, QUICK_BEFORE[i].2),
                }
                self.typed = 0;
            }
            50 => return Answer::Set(None),
            51 => return Answer::Cancel,
            52 => return Answer::Set(Some(self.value())),
            _ => return Answer::Nothing,
        }
        Answer::Redraw
    }

    /// The pad's answer to an event (the app's softkeys: Cancel and Set).
    pub fn event(&mut self, e: Event) -> Answer {
        match e {
            Event::PenDown { .. } => {
                self.focus = None;
                Answer::Nothing
            }
            Event::PenUp { x, y } => {
                let hit = self
                    .keys()
                    .find(|(r, _)| r.contains_point(x, y))
                    .map(|k| k.1);
                hit.map_or(Answer::Nothing, |k| self.press(k))
            }
            Event::Key(Key::SoftLeft) => Answer::Cancel,
            Event::Key(Key::SoftRight) => Answer::Set(Some(self.value())),
            // The D-pad as Windows Mobile's time field: left and right
            // choose the part, up and down step it; the centre sets.
            Event::Key(Key::Left | Key::Right) if self.mode == Mode::Clock => {
                self.part ^= 1;
                self.typed = 0;
                Answer::Redraw
            }
            Event::Key(Key::Left | Key::Right) => {
                self.unit = if e == Event::Key(Key::Right) {
                    (self.unit + 1).min(2)
                } else {
                    self.unit.saturating_sub(1)
                };
                Answer::Redraw
            }
            Event::Key(Key::Up) => {
                self.step(1);
                Answer::Redraw
            }
            Event::Key(Key::Down) => {
                self.step(-1);
                Answer::Redraw
            }
            Event::Key(Key::Enter) => Answer::Set(Some(self.value())),
            Event::Char(c @ b'0'..=b'9') => {
                self.digit(u16::from(c - b'0'));
                Answer::Redraw
            }
            _ => Answer::Nothing,
        }
    }

    /// The pad drawn over the app's content, under `title` (what is being
    /// set: "TIME", "STARTS", "ALERT").
    pub fn draw(&self, f: &mut Frame, cx: &Ctx, title: &[u8]) {
        kit::panel(f, cx, Rect::new(8, TOP - 4, W - 16, 140));
        kit::text(f, MARGIN + 8, TOP + 22, cx.fonts.label, DIM, title);
        let two = |v: u16, pad: bool| -> ([u8; 3], usize) {
            let mut b = [0u8; 3];
            let mut n = 0;
            if v >= 100 {
                b[n] = b'0' + (v / 100) as u8;
                n += 1;
            }
            if v >= 10 || pad {
                b[n] = b'0' + (v / 10 % 10) as u8;
                n += 1;
            }
            b[n] = b'0' + (v % 10) as u8;
            (b, n + 1)
        };
        let field = |f: &mut Frame, r: Rect, on: bool, v: u16, pad: bool| {
            kit::key(f, cx, r, if on { KeyState::On } else { KeyState::Rest });
            let (b, n) = two(v, pad);
            kit::centred(f, r, r.y + 62, cx.fonts.big, TEXT, &b[..n]);
        };
        match self.mode {
            Mode::Clock => {
                field(f, HOUR, self.part == 0, self.a, self.h24);
                kit::centred(
                    f,
                    Rect::new(HOUR.right(), 0, MINUTE.x - HOUR.right(), 1),
                    HOUR.y + 58,
                    cx.fonts.big,
                    TEXT,
                    b":",
                );
                field(f, MINUTE, self.part == 1, self.b, true);
                // (The 24-hour clock has no half of the day to choose.)
                let halves: &[&[u8]] = if self.h24 { &[] } else { &[b"AM", b"PM"] };
                for (k, w) in halves.iter().enumerate() {
                    let r = self.side(k);
                    let on = self.pm == (k == 1);
                    kit::key(f, cx, r, if on { KeyState::On } else { KeyState::Rest });
                    kit::centred(f, r, r.y + 26, cx.fonts.ui, TEXT, w);
                }
            }
            Mode::Before => {
                field(f, HOUR, true, self.a, false);
                kit::text(
                    f,
                    MINUTE.x - 14,
                    MINUTE.y + 50,
                    cx.fonts.ui,
                    DIM,
                    b"before it starts",
                );
                for (k, w) in [&b"MINUTES"[..], b"HOURS", b"DAYS"].iter().enumerate() {
                    let on = usize::from(self.unit) == k;
                    kit::key(
                        f,
                        cx,
                        SIDE[k],
                        if on { KeyState::On } else { KeyState::Rest },
                    );
                    kit::centred(f, SIDE[k], SIDE[k].y + 18, cx.fonts.label, TEXT, w);
                }
            }
        }
        for (i, w) in DIGITS.iter().enumerate() {
            let r = cell(PAD, 3, 4, i as u16);
            kit::key(f, cx, r, KeyState::Rest);
            let (font, base) = if w.len() == 1 {
                (cx.fonts.num, r.y + 42)
            } else {
                (cx.fonts.label, r.y + 35)
            };
            kit::centred(f, r, base, font, TEXT, w);
        }
        for i in 0..4usize {
            let r = cell(QUICK, 1, 4, i as u16);
            kit::key(f, cx, r, KeyState::Rest);
            let w = match self.mode {
                Mode::Clock => QUICK_MIN[i].0,
                Mode::Before => QUICK_BEFORE[i].0,
            };
            kit::centred(f, r, r.y + 35, cx.fonts.label, TEXT, w);
        }
        let none: &[u8] = if self.mode == Mode::Clock {
            b"NO TIME"
        } else {
            b"NO ALERT"
        };
        for (i, w) in [none, b"CANCEL", b"SET"].iter().enumerate() {
            let r = cell(FOOT, 3, 1, i as u16);
            kit::key(f, cx, r, if i == 2 { KeyState::On } else { KeyState::Rest });
            kit::centred(f, r, r.y + 30, cx.fonts.ui, TEXT, w);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tap(p: &mut TimePad, key: u8) -> Answer {
        let r = p.keys().find(|k| k.1 == key).expect("a key").0;
        p.event(Event::PenUp {
            x: r.x + r.w / 2,
            y: r.y + r.h / 2,
        })
    }

    #[test]
    fn three_taps_set_seven_past_three() {
        // "3:07 PM": 3, 0, 7, PM, SET.
        let mut p = TimePad::clock(None, true);
        assert_eq!(p.value(), 9 * 60);
        for d in [22, 30, 26] {
            assert_eq!(tap(&mut p, d), Answer::Redraw);
        }
        tap(&mut p, 11);
        assert_eq!(tap(&mut p, 52), Answer::Set(Some(15 * 60 + 7)));
        // 12:30 AM is half past midnight; 12:30 PM half past noon.
        let mut p = TimePad::clock(None, true);
        for d in [20, 21, 22, 30] {
            tap(&mut p, d);
        }
        tap(&mut p, 10);
        assert_eq!(p.value(), 30);
        tap(&mut p, 11);
        assert_eq!(p.value(), 12 * 60 + 30);
        // 10, 11 and 12 take their second digit; 1 then 5 is 1:5_.
        let mut p = TimePad::clock(None, true);
        tap(&mut p, 20);
        tap(&mut p, 24);
        tap(&mut p, 28);
        assert_eq!(p.value(), 13 * 60 - 12 * 60 + 59, "1:59 AM");
        // Every minute can be said, and none past 59.
        let mut p = TimePad::clock(Some(0), true);
        tap(&mut p, 1);
        tap(&mut p, 28);
        tap(&mut p, 28);
        assert_eq!(p.value() % 60, 9, "9 then 9: a new minute, not 99");
        // The quick keys, no time, cancel.
        assert_eq!(tap(&mut p, 42), Answer::Redraw);
        assert_eq!(p.value() % 60, 30);
        assert_eq!(tap(&mut p, 50), Answer::Set(None));
        assert_eq!(tap(&mut p, 51), Answer::Cancel);
    }

    #[test]
    fn the_d_pad_steps_a_part_round_its_range() {
        let mut p = TimePad::clock(Some(11 * 60 + 59), true);
        p.event(Event::Key(Key::Up));
        assert_eq!(p.value(), 12 * 60 + 59, "11 AM up is noon's hour");
        p.event(Event::Key(Key::Right));
        p.event(Event::Key(Key::Up));
        assert_eq!(p.value(), 12 * 60, "59 up is 00, the hour its own");
        p.event(Event::Key(Key::Down));
        p.event(Event::Key(Key::Down));
        assert_eq!(p.value(), 12 * 60 + 58);
        assert_eq!(
            p.event(Event::Key(Key::Enter)),
            Answer::Set(Some(12 * 60 + 58))
        );
        assert_eq!(p.event(Event::Key(Key::SoftLeft)), Answer::Cancel);
    }

    #[test]
    fn the_24_hour_clock_types_the_hour_whole_with_no_am_or_pm() {
        // "15:07": 1, 5, 0, 7, SET; no AM or PM key among the keys.
        let mut p = TimePad::clock(None, false);
        assert_eq!(p.value(), 9 * 60);
        assert!(p.keys().all(|k| !(10..=12).contains(&k.1)));
        for d in [20, 24, 30, 26] {
            tap(&mut p, d);
        }
        assert_eq!(tap(&mut p, 52), Answer::Set(Some(15 * 60 + 7)));
        // 0 then 0: midnight's hour; 7 alone: 07, on to the minutes.
        let mut p = TimePad::clock(Some(600), false);
        tap(&mut p, 30);
        tap(&mut p, 30);
        assert_eq!(p.value(), 0);
        let mut p = TimePad::clock(Some(600), false);
        tap(&mut p, 26);
        tap(&mut p, 23);
        tap(&mut p, 30);
        assert_eq!(p.value(), 7 * 60 + 40);
        // 2 then 3 is 23; 2 then 5 is 02, the 5 the minutes' first digit.
        let mut p = TimePad::clock(Some(600), false);
        tap(&mut p, 21);
        tap(&mut p, 22);
        assert_eq!(p.value() / 60, 23);
        let mut p = TimePad::clock(Some(600), false);
        tap(&mut p, 21);
        tap(&mut p, 24);
        tap(&mut p, 30);
        assert_eq!(p.value(), 2 * 60 + 50);
        // The D-pad steps the hour round 0-23; DEL and CLEAR.
        let mut p = TimePad::clock(Some(23 * 60 + 5), false);
        p.event(Event::Key(Key::Up));
        assert_eq!(p.value(), 5);
        p.event(Event::Key(Key::Down));
        p.event(Event::Key(Key::Down));
        assert_eq!(p.value(), 22 * 60 + 5);
        tap(&mut p, 31);
        assert_eq!(p.value() / 60, 2, "22 less a digit: 2, waiting");
        tap(&mut p, 20);
        assert_eq!(p.value() / 60, 21);
        tap(&mut p, 0);
        tap(&mut p, 29);
        assert_eq!(p.value() / 60, 0);
        // The same moment on both clocks.
        for m in [0u16, 59, 720, 779, 1439] {
            assert_eq!(TimePad::clock(Some(m), true).value(), u32::from(m));
            assert_eq!(TimePad::clock(Some(m), false).value(), u32::from(m));
        }
    }

    #[test]
    fn an_alert_is_a_number_and_a_unit() {
        let p = TimePad::before(Some(90));
        assert_eq!(p.value(), 90);
        let mut p = TimePad::before(Some(120));
        assert_eq!((p.a, p.unit), (2, 1));
        let mut p2 = TimePad::before(Some(2880));
        assert_eq!((p2.a, p2.unit), (2, 2));
        // 7 minutes: 7, SET. 45, HOURS: 45 hours.
        tap(&mut p, 26);
        tap(&mut p, 10);
        assert_eq!(tap(&mut p, 52), Answer::Set(Some(7)));
        tap(&mut p2, 23);
        tap(&mut p2, 24);
        tap(&mut p2, 11);
        assert_eq!(p2.value(), 45 * 60);
        assert_eq!(tap(&mut p2, 40), Answer::Redraw);
        assert_eq!(p2.value(), 0, "at start");
        assert_eq!(tap(&mut p2, 50), Answer::Set(None));
    }
}
