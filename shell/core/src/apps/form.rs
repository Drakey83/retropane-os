//! The form an editor is (docs/design/design-language.md "Panel rules"):
//! rows of text fields, steppers (`<` value `>`), choice chips and Delete,
//! one engine for the appointment, contact and task editors.
//!
//! Iterate (rule 0): Windows Mobile's forms were drop-downs 12 px tall and
//! a keyboard that hid the field; a phone's are wheels and sheets. Ours
//! are rows a stylus can't miss (36 px, 44 px arrows), every one reachable
//! by the D-pad, and the row being typed stays in view above the keyboard.

use rp_draw::{Cmd, Frame, Rect};
use rp_weather::Line;

use super::{Act, DIM, FOCUS, LATE, MARGIN, PITCH, ROW, TEXT, TOP, Typing, View, WIDE, day_name};
use crate::{Shell, W};

/// A row of a form.
#[derive(Clone, Copy)]
pub(crate) enum Row {
    /// A text field typed into slot `slot`.
    Text(&'static str, u8),
    /// A stepper over number `which`.
    Step(&'static str, u8),
    /// Chips for number `which`.
    Choice(&'static str, u8, &'static [&'static str]),
    /// Delete (only for a record that exists).
    Delete,
}

const EVENT: [Row; 8] = [
    Row::Text("Title", 0),
    Row::Text("Where", 1),
    Row::Step("Day", 0),
    Row::Step("Starts", 1),
    Row::Step("Lasts", 2),
    Row::Step("Remind me", 3),
    Row::Text("Notes", 2),
    Row::Delete,
];
const CONTACT: [Row; 10] = [
    Row::Text("First", 0),
    Row::Text("Last", 1),
    Row::Text("Company", 2),
    Row::Text("Mobile", 3),
    Row::Text("Home", 4),
    Row::Text("Work", 5),
    Row::Text("E-mail", 6),
    Row::Text("Address", 7),
    Row::Text("Notes", 8),
    Row::Delete,
];
const TASK: [Row; 5] = [
    Row::Text("Title", 0),
    Row::Step("Due", 0),
    Row::Choice("Priority", 1, &["LOW", "NORMAL", "HIGH"]),
    Row::Text("Notes", 1),
    Row::Delete,
];

/// How long an appointment lasts, by the stepper's position (minutes; 0
/// is the whole day).
pub(crate) const LASTS: [u16; 9] = [15, 30, 45, 60, 90, 120, 180, 240, 0];
/// When to remind, by the stepper's position (minutes before; `None`:
/// no reminder).
pub(crate) const REMIND: [Option<u16>; 7] = [
    None,
    Some(0),
    Some(5),
    Some(15),
    Some(30),
    Some(60),
    Some(1440),
];
/// A task with no due day.
pub(crate) const NO_DAY: i32 = -1;

/// The x a text row's value starts at, from the row's left.
const VALUE_X: u16 = 104;

impl Shell {
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn form_rows(&self, v: View) -> &'static [Row] {
        let rows: &'static [Row] = match v {
            View::EventEdit => &EVENT,
            View::ContactEdit => &CONTACT,
            _ => &TASK,
        };
        // Delete is for a record that exists.
        if self.apps.open.is_none() {
            &rows[..rows.len() - 1]
        } else {
            rows
        }
    }

    /// The form's targets.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn form_targets(&mut self, v: View) {
        let rows = self.form_rows(v);
        self.apps.height = rows.len() as i32 * i32::from(PITCH);
        for (k, row) in rows.iter().enumerate() {
            let r = self.row_at(v, k as i32 * i32::from(PITCH), ROW, MARGIN, WIDE);
            if r.h < ROW {
                // A row cut by the edge isn't a target.
                continue;
            }
            match *row {
                Row::Text(_, slot) => self.app_target(r, Act::Field(slot)),
                Row::Step(_, which) => {
                    self.app_target(
                        Rect::new(r.x + r.w - 232, r.y, 44, r.h),
                        Act::Step(which, -1),
                    );
                    self.app_target(Rect::new(r.x + r.w - 44, r.y, 44, r.h), Act::Step(which, 1));
                }
                Row::Choice(_, which, words) => {
                    for j in 0..words.len() as u16 {
                        self.app_target(
                            choice_chip(r, words.len() as u16, j),
                            Act::Choice(which, j as u8),
                        );
                    }
                }
                Row::Delete => self.app_target(r, Act::Delete),
            }
        }
    }

    /// The stepper's value in words.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn step_text(&self, v: View, which: u8) -> Line {
        let n = self.apps.nums[usize::from(which)];
        match (v, which) {
            (View::EventEdit, 0) => day_name(n, self.this_year()),
            (View::EventEdit, 1) => {
                if LASTS[self.apps.nums[2].clamp(0, 8) as usize] == 0 {
                    Line::new().push(b"-")
                } else {
                    self.region.time((n / 60) as u8, (n % 60) as u8, true)
                }
            }
            (View::EventEdit, 2) => match LASTS[n.clamp(0, 8) as usize] {
                0 => Line::new().push(b"All day"),
                m if m < 60 => Line::new().num(i32::from(m)).push(b" min"),
                60 => Line::new().push(b"1 hour"),
                90 => Line::new().push(b"1.5 hours"),
                m => Line::new().num(i32::from(m / 60)).push(b" hours"),
            },
            (View::EventEdit, _) => match REMIND[n.clamp(0, 6) as usize] {
                None => Line::new().push(b"No"),
                Some(0) => Line::new().push(b"At the start"),
                Some(60) => Line::new().push(b"1 hour before"),
                Some(1440) => Line::new().push(b"1 day before"),
                Some(m) => Line::new().num(i32::from(m)).push(b" min before"),
            },
            _ => {
                if n == NO_DAY {
                    Line::new().push(b"No date")
                } else {
                    day_name(n, self.this_year())
                }
            }
        }
    }

    /// A stepper's arrow.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn form_step(&mut self, v: View, which: u8, by: i8) {
        let by = i32::from(by);
        let today = self.today();
        let n = &mut self.apps.nums[usize::from(which)];
        match (v, which) {
            // Any day from 1970 on.
            (View::EventEdit, 0) => *n = (*n + by).max(0),
            // A quarter of an hour at a time, round the clock.
            (View::EventEdit, 1) => *n = (*n + 15 * by).rem_euclid(24 * 60),
            (View::EventEdit, 2) => *n = (*n + by).clamp(0, LASTS.len() as i32 - 1),
            (View::EventEdit, _) => *n = (*n + by).clamp(0, REMIND.len() as i32 - 1),
            // No date, then today, then on a day at a time.
            _ => {
                *n = match (*n, by) {
                    (NO_DAY, 1) => today,
                    (NO_DAY, _) => NO_DAY,
                    (d, -1) if d <= today => NO_DAY,
                    (d, by) => d + by,
                }
            }
        }
        self.dirty(Rect::new(0, TOP, W, self.app_bottom() - TOP));
    }

    /// The keyboard's Done in a form: on to the next text field, or the
    /// keyboard down after the last.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn form_next(&mut self, v: View, slot: u8) {
        let next = self
            .form_rows(v)
            .iter()
            .filter_map(|r| match *r {
                Row::Text(_, s) => Some(s),
                _ => None,
            })
            .skip_while(|s| *s != slot)
            .nth(1);
        match next {
            Some(s) => {
                self.app_typing(Some(Typing::Slot(s)));
                self.form_show_typed(v);
                self.dirty(Rect::new(0, TOP, W, self.app_bottom() - TOP));
            }
            None => self.app_typing(None),
        }
    }

    /// Scrolls the form so the row being typed is above the keyboard.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn form_show_typed(&mut self, v: View) {
        let Some(Typing::Slot(slot)) = self.apps.typing else {
            return;
        };
        let Some(k) = self
            .form_rows(v)
            .iter()
            .position(|r| matches!(*r, Row::Text(_, s) if s == slot))
        else {
            return;
        };
        let (y, view) = (
            k as i32 * i32::from(PITCH),
            i32::from(self.app_bottom()) - i32::from(TOP),
        );
        if y < self.apps.scroll {
            self.apps.scroll = y;
        } else if y + i32::from(ROW) > self.apps.scroll + view {
            self.apps.scroll = y + i32::from(ROW) - view;
        }
        self.app_refresh();
    }

    /// The Delete row: the first tap asks, the second removes.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn form_delete(&mut self, v: View) {
        if !self.apps.confirm {
            self.apps.confirm = true;
            self.dirty(Rect::new(0, TOP, W, self.app_bottom() - TOP));
            return;
        }
        if let (Some(id), Some(s)) = (self.apps.open, self.apps.store.as_mut()) {
            let _ = s.remove(id);
        }
        self.apps.open = None;
        self.apps.typing = None;
        self.open_app(match v {
            View::EventEdit => View::Agenda,
            View::ContactEdit => View::Contacts,
            _ => View::Tasks,
        });
    }

    /// The form.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn form_scene(&self, f: &mut Frame, v: View) {
        let ui = self.fonts.ui;
        let (top, bottom) = (TOP, self.app_bottom());
        f.clip(Some(Rect::new(0, top, W, bottom - top)));
        for (k, row) in self.form_rows(v).iter().enumerate() {
            let y = i32::from(top) + k as i32 * i32::from(PITCH) - self.apps.scroll;
            if y + i32::from(ROW) <= i32::from(top) || y >= i32::from(bottom) {
                continue;
            }
            let r = Rect::new(MARGIN, y as u16, WIDE, ROW);
            let base = r.y + 25;
            match *row {
                Row::Text(name, slot) => {
                    let typed = self.apps.typing == Some(Typing::Slot(slot));
                    self.row_glass(f, r, Act::Field(slot));
                    f.text(r.x + 12, base, ui, DIM, name.as_bytes());
                    let text = self.apps.slot(usize::from(slot));
                    let room = r.w - VALUE_X - 14;
                    // While typing, the text's tail shows (the caret is at
                    // its end).
                    let from = if typed {
                        (0..text.len())
                            .find(|&k| ui.width(&text[k..]) <= room)
                            .unwrap_or(text.len())
                    } else {
                        0
                    };
                    self.text_cut(f, (r.x + VALUE_X, base, room), ui, TEXT, &text[from..]);
                    if typed {
                        f.push(Cmd::Fill {
                            r: Rect::new(
                                r.x + VALUE_X + ui.width(&text[from..]) + 1,
                                r.y + 7,
                                2,
                                22,
                            ),
                            color: FOCUS,
                        });
                        f.push(Cmd::Outline { r, color: FOCUS });
                    }
                    self.app_marks(f, Act::Field(slot));
                }
                Row::Step(name, which) => {
                    crate::panel::draw(f, r, self.light());
                    f.text(r.x + 12, base, ui, DIM, name.as_bytes());
                    let (l, rt) = (
                        Rect::new(r.x + r.w - 232, r.y, 44, r.h),
                        Rect::new(r.x + r.w - 44, r.y, 44, r.h),
                    );
                    for (a, by, mark) in [(l, -1, b"<"), (rt, 1, b">")] {
                        if self.app_pressed(Act::Step(which, by)) {
                            f.push(Cmd::Fill {
                                r: a,
                                color: crate::PRESSED_UNDER,
                            });
                        }
                        crate::keyboard::centre_in(f, a, ui, TEXT, mark);
                        self.app_marks(f, Act::Step(which, by));
                    }
                    let value = self.step_text(v, which);
                    let mid = Rect::new(l.x + l.w, r.y, rt.x - l.x - l.w, r.h);
                    crate::keyboard::centre_in(f, mid, ui, TEXT, value.as_bytes());
                }
                Row::Choice(name, which, words) => {
                    crate::panel::draw(f, r, self.light());
                    f.text(r.x + 12, base, ui, DIM, name.as_bytes());
                    for (j, w) in words.iter().enumerate() {
                        self.chip(
                            f,
                            choice_chip(r, words.len() as u16, j as u16),
                            w.as_bytes(),
                            self.apps.nums[usize::from(which)] == j as i32,
                            Act::Choice(which, j as u8),
                        );
                    }
                }
                Row::Delete => {
                    self.row_glass(f, r, Act::Delete);
                    let word: &[u8] = if self.apps.confirm {
                        b"Tap again to delete"
                    } else {
                        b"Delete"
                    };
                    f.text(r.x + 12, base, ui, LATE, word);
                    self.app_marks(f, Act::Delete);
                }
            }
        }
        f.clip(None);
    }
}

/// Chip `j` of `n` at a row's right.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn choice_chip(r: Rect, n: u16, j: u16) -> Rect {
    Rect::new(r.x + r.w - 8 - (n - j) * 84, r.y + 3, 80, r.h - 6)
}

/// The day a stepper starts from for a task with no date.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub(crate) const fn due_or_none(due: Option<u32>) -> i32 {
    match due {
        Some(d) => d as i32,
        None => NO_DAY,
    }
}
