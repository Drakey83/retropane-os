//! Calendar (docs/design/calendar-rule0.md): the agenda, a day by the
//! hour, a month with the chosen day's appointments, the quick-add line
//! and the editor's values.
//!
//! Iterate (rule 0): Windows Mobile opened on a day grid and added through
//! a ten-row form of drop-downs; ours opens on what is next, and adds from
//! one typed line whose reading is shown before it is kept.

use rp_draw::{Cmd, Frame, Rect};
use rp_pim::date::{self, DAY_MIN, Date};
use rp_pim::quick;
use rp_pim::store::{Kind, Rec, tag};
use rp_weather::Line;

use super::form::{LASTS, REMIND};
use super::{
    Act, DIM, FIELD, FOCUS, MARGIN, PITCH, ROW, TEXT, TOP, Typing, View, WIDE, day_name, num, upper,
};
use crate::{PRESSED_UNDER, Shell, W, keyboard, panel};

/// Days the agenda looks ahead.
const AHEAD: i32 = 60;
/// Entries the agenda shows (labels and appointments).
const AGENDA_MAX: usize = 64;
/// A table entry that is a day's label, not a record.
const LABEL: u32 = 0x8000_0000;
/// The agenda's day label height.
const LABEL_H: u16 = 24;
/// An hour of the day view.
const HOUR_H: u16 = 36;
/// Where the day view's appointments start (right of the hour labels).
const HOUR_X: u16 = MARGIN + 62;
/// Appointments a day view places.
const DAY_MAX: usize = 16;
/// The header's view chips.
const CHIPS: [(View, &[u8]); 3] = [
    (View::Agenda, b"AGENDA"),
    (View::Day, b"DAY"),
    (View::Month, b"MONTH"),
];
/// The row with the day's or month's name and its arrows.
const NAV: Rect = Rect::new(MARGIN, TOP + 2, WIDE, 36);
const NAV_BACK: Rect = Rect::new(MARGIN, TOP + 2, 48, 36);
const NAV_ON: Rect = Rect::new(W - MARGIN - 48, TOP + 2, 48, 36);
/// The month grid: seven columns of 64, six rows of 44.
const GRID_Y: u16 = TOP + 64;
const CELL_W: u16 = 64;
const CELL_H: u16 = 44;
/// The chosen day's list under the grid.
const MONTH_LIST_Y: u16 = GRID_Y + 6 * CELL_H + 8;
const MONTH_LIST: usize = 4;

#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn chip_rect(j: u16) -> Rect {
    Rect::new(206 + j * 88, 50, 84, 30)
}

/// An appointment's start (local minutes).
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub(crate) fn start_of(r: &Rec<'_>) -> u32 {
    r.num(tag::START).unwrap_or(0)
}

#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn all_day(r: &Rec<'_>) -> bool {
    r.num(tag::ALL_DAY) == Some(1)
}

/// When an appointment is over.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub(crate) fn end_of(r: &Rec<'_>) -> u32 {
    let len = if all_day(r) {
        DAY_MIN
    } else {
        r.num(tag::MINUTES).unwrap_or(60)
    };
    start_of(r).saturating_add(len)
}

/// When its reminder is due, if it has one.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub(crate) fn remind_at(r: &Rec<'_>) -> Option<u32> {
    Some(start_of(r).saturating_sub(r.num(tag::REMIND)?))
}

/// All-day first, then by start.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn by_time(a: &Rec<'_>, b: &Rec<'_>) -> core::cmp::Ordering {
    (!all_day(a), start_of(a), a.id).cmp(&(!all_day(b), start_of(b), b.id))
}

impl Shell {
    /// "3:00 PM" or "All day".
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn event_time(&self, r: &Rec<'_>) -> Line {
        if all_day(r) {
            Line::new().push(b"All day")
        } else {
            let m = start_of(r) % DAY_MIN;
            self.region.time((m / 60) as u8, (m % 60) as u8, true)
        }
    }

    /// "Fri 13 Nov, 3:00 PM".
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn event_when(&self, r: &Rec<'_>) -> Line {
        day_name(date::day_of(start_of(r)), self.this_year())
            .push(b", ")
            .push(self.event_time(r).as_bytes())
    }

    /// The table filled with the appointments of day `day`, in order: how
    /// many (at most `max`).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn day_fill(&mut self, day: i32, max: usize) -> usize {
        let (from, to) = (date::start_of(day), date::start_of(day + 1));
        let n = self.apps.fill(Kind::Appointment, 0, |r| {
            start_of(r) >= from && start_of(r) < to
        });
        self.apps.sort(0, n, &by_time);
        n.min(max)
    }

    /// The view's list, made when the records or the day change.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn calendar_build(&mut self, v: View) {
        let today = self.today();
        match v {
            View::Agenda => {
                // Everything not over yet, up to AHEAD days on, by start;
                // then a label before each day's first.
                let (from, to) = (date::start_of(today), date::start_of(today + AHEAD));
                let all = self.apps.fill(Kind::Appointment, AGENDA_MAX, |r| {
                    end_of(r) > from && start_of(r) < to
                });
                self.apps.sort(AGENDA_MAX, all, &|a, b| {
                    (start_of(a), a.id).cmp(&(start_of(b), b.id))
                });
                let (mut n, mut height, mut last) = (0, 0i32, None);
                for k in 0..all {
                    let handle = self.apps.entry(AGENDA_MAX + k);
                    let Some(start) = self.apps.rec(handle).map(|r| start_of(&r)) else {
                        continue;
                    };
                    // One begun before today shows under today.
                    let day = date::day_of(start).max(today);
                    if last != Some(day) {
                        if n + 2 > AGENDA_MAX {
                            break;
                        }
                        self.apps.set_entry(n, LABEL | day as u32);
                        n += 1;
                        height += i32::from(LABEL_H);
                        last = Some(day);
                    }
                    if n == AGENDA_MAX {
                        break;
                    }
                    self.apps.set_entry(n, handle);
                    n += 1;
                    height += i32::from(PITCH);
                }
                self.apps.n = n;
                self.apps.height = height;
            }
            View::Day => {
                let day = self.apps.day;
                let n = self.day_fill(day, DAY_MAX);
                // The hours shown: 6 AM to 10 PM, wider when something
                // lies outside them.
                let (mut h0, mut h1, mut whole) = (6u32, 22u32, 0);
                for k in 0..n {
                    if let Some(r) = self.apps.rec(self.apps.entry(k)) {
                        if all_day(&r) {
                            whole += 1;
                            continue;
                        }
                        let s = start_of(&r) % DAY_MIN;
                        let e = (s + r.num(tag::MINUTES).unwrap_or(60)).min(DAY_MIN);
                        h0 = h0.min(s / 60);
                        h1 = h1.max(e.div_ceil(60));
                    }
                }
                self.apps.n = n;
                self.apps.span = (h0 as u8, h1 as u8, whole);
                self.apps.height =
                    i32::from(whole) * i32::from(PITCH) + (h1 - h0) as i32 * i32::from(HOUR_H) + 8;
            }
            _ => {
                // Month: a count a cell, then the chosen day's list.
                let first = self.grid_start();
                let mut dots = [0u8; 42];
                if let Some(s) = &self.apps.store {
                    for r in s.all(Kind::Appointment) {
                        let d = date::day_of(start_of(&r)) - first;
                        if let Some(c) = usize::try_from(d).ok().and_then(|d| dots.get_mut(d)) {
                            *c = (*c + 1).min(3);
                        }
                    }
                }
                self.apps.dots = dots;
                let day = self.apps.day;
                self.apps.n = self.day_fill(day, MONTH_LIST);
                self.apps.height = 0;
            }
        }
    }

    /// The first day of the month grid: the Monday on or before the 1st
    /// of the chosen day's month.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn grid_start(&self) -> i32 {
        let first = date::days_from(date::month_start(date::date_of(self.apps.day), 0));
        first - i32::from(date::weekday(first))
    }

    /// The day view's appointment `k` of `n`: its box, and whether it is
    /// an all-day row.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn day_box(&self, k: usize) -> Option<(Rect, bool, u32)> {
        let r = self.apps.rec(self.apps.entry(k))?;
        let (h0, h1, whole) = self.apps.span;
        if all_day(&r) {
            return Some((
                self.row_at(View::Day, k as i32 * i32::from(PITCH), ROW, MARGIN, WIDE),
                true,
                r.id,
            ));
        }
        // Side by side with the ones it overlaps (three columns at most):
        // its column is how many earlier ones it overlaps.
        let over = |a: &Rec<'_>, b: &Rec<'_>| start_of(a) < end_of(b) && start_of(b) < end_of(a);
        let (mut col, mut cols) = (0u16, 1u16);
        for j in usize::from(whole)..self.apps.n {
            if j != k
                && self
                    .apps
                    .rec(self.apps.entry(j))
                    .is_some_and(|o| over(&o, &r))
            {
                cols += 1;
                col += u16::from(j < k);
            }
        }
        let cols = cols.min(3);
        let minutes = (start_of(&r) % DAY_MIN).saturating_sub(u32::from(h0) * 60);
        let len = r
            .num(tag::MINUTES)
            .unwrap_or(60)
            .min(u32::from(h1) * 60 - (start_of(&r) % DAY_MIN).min(u32::from(h1) * 60));
        let y = i32::from(whole) * i32::from(PITCH) + (minutes * u32::from(HOUR_H) / 60) as i32;
        let h = (len * u32::from(HOUR_H) / 60).max(26) as u16;
        let w = (W - MARGIN - HOUR_X) / cols;
        Some((
            self.row_at(View::Day, y, h - 2, HOUR_X + (col % cols) * w, w - 2),
            false,
            r.id,
        ))
    }

    /// The view's targets.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn calendar_targets(&mut self, v: View) {
        for (j, (to, _)) in CHIPS.iter().enumerate() {
            self.app_target(chip_rect(j as u16), Act::Chip(*to));
        }
        match v {
            View::Agenda => {
                let mut y = 0i32;
                for k in 0..self.apps.n {
                    let e = self.apps.entry(k);
                    if e & LABEL != 0 {
                        y += i32::from(LABEL_H);
                        continue;
                    }
                    let r = self.row_at(v, y, ROW, MARGIN, WIDE);
                    if let (true, Some(id)) = (r.h == ROW, self.apps.rec(e).map(|r| r.id)) {
                        self.app_target(r, Act::Open(id));
                    }
                    y += i32::from(PITCH);
                }
            }
            View::Day => {
                self.app_target(NAV_BACK, Act::Nav(-1));
                self.app_target(NAV_ON, Act::Nav(1));
                // Appointments in front of the hours they cover.
                for k in 0..self.apps.n {
                    if let Some((r, _, id)) = self.day_box(k)
                        && r.h >= 20
                    {
                        self.app_target(r, Act::Open(id));
                    }
                }
                let (h0, h1, whole) = self.apps.span;
                for h in h0..h1 {
                    let y =
                        i32::from(whole) * i32::from(PITCH) + i32::from(h - h0) * i32::from(HOUR_H);
                    let r = self.row_at(v, y, HOUR_H, MARGIN, WIDE);
                    if r.h == HOUR_H {
                        self.app_target(r, Act::Hour(h));
                    }
                }
            }
            _ => {
                self.app_target(NAV_BACK, Act::Nav(-1));
                self.app_target(NAV_ON, Act::Nav(1));
                let first = self.grid_start();
                for c in 0..42u16 {
                    self.app_target(
                        Rect::new(
                            MARGIN + c % 7 * CELL_W,
                            GRID_Y + c / 7 * CELL_H,
                            CELL_W,
                            CELL_H,
                        ),
                        Act::Cell(first + i32::from(c)),
                    );
                }
                for k in 0..self.apps.n {
                    if let Some(id) = self.apps.rec(self.apps.entry(k)).map(|r| r.id) {
                        self.app_target(
                            Rect::new(MARGIN, MONTH_LIST_Y + k as u16 * PITCH, WIDE, ROW),
                            Act::Open(id),
                        );
                    }
                }
            }
        }
    }

    /// The day's or month's arrows.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn calendar_nav(&mut self, v: View, by: i8) {
        self.apps.day = if v == View::Month {
            // The same day of the next month, or its last.
            let d = date::date_of(self.apps.day);
            let m = date::month_start(d, i32::from(by));
            date::days_from(Date {
                day: d.day.min(date::days_in_month(m.year, m.month)),
                ..m
            })
        } else {
            (self.apps.day + i32::from(by)).max(0)
        };
        self.apps.scroll = 0;
        self.app_changed();
    }

    /// A day of the month grid: chosen; tapped again, opened.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn calendar_cell(&mut self, day: i32) {
        if day == self.apps.day {
            return self.open_app(View::Day);
        }
        self.apps.day = day.max(0);
        self.app_changed();
        // The grid may have turned to another month under the focus.
        self.focus = self.focus.and(self.app_find(Act::Cell(day)));
    }

    // ---- Adding and editing ---------------------------------------------

    /// The quick-add line, starting at `hour` of the chosen day when the
    /// day view's hour was tapped.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn event_new(&mut self, hour: Option<u8>) {
        self.apps.clear_form();
        self.apps.open = None;
        // The form's numbers, as the line will fill them: the chosen day,
        // the next whole hour (or the one tapped), an hour, 15 minutes.
        let at = match hour {
            Some(h) => i32::from(h) * 60,
            None if self.apps.day == self.today() => (i32::from(self.time.hour) + 1).min(23) * 60,
            None => 9 * 60,
        };
        self.apps.nums = [self.apps.day.max(self.today()), at, 3, 3];
        if hour.is_some() {
            self.apps.nums[0] = self.apps.day;
        }
        self.apps.typing = Some(Typing::Line);
        self.open_app(View::NewEvent);
    }

    /// What the line says, over the defaults: the title's length (in
    /// `title`), the day, the minute of the day, the length (0: all day)
    /// and the reminder.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn event_read(&self, title: &mut [u8; quick::TITLE_MAX]) -> (usize, i32, i32, u16) {
        let (q, n) = quick::parse(self.apps.line(), self.today(), self.month_first(), title);
        let [day, at, lasts, _] = self.apps.nums;
        (
            n,
            q.day.unwrap_or(day),
            q.time.map_or(at, i32::from),
            q.minutes.unwrap_or(LASTS[lasts.clamp(0, 8) as usize]),
        )
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn event_new_targets(&mut self) {
        self.app_target(FIELD, Act::Field(super::LINE_SLOT as u8));
        self.app_target(Rect::new(MARGIN, FIELD.y + 46 + 70, WIDE, ROW), Act::More);
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn event_new_scene(&self, f: &mut Frame) {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        self.line_field(f, "Dentist tue 3pm 1h");
        // What the line was understood as, before it is kept.
        let mut title = [0u8; quick::TITLE_MAX];
        let (n, day, at, len) = self.event_read(&mut title);
        let p = Rect::new(MARGIN, FIELD.y + 46, WIDE, 64);
        panel::draw(f, p, self.light());
        let name: &[u8] = if n == 0 {
            b"New appointment"
        } else {
            &title[..n]
        };
        self.text_cut(f, (p.x + 12, p.y + 26, p.w - 24), ui, TEXT, name);
        let from = self.region.time((at / 60) as u8, (at % 60) as u8, true);
        let end = (at + i32::from(len)).rem_euclid(24 * 60);
        let to = self.region.time((end / 60) as u8, (end % 60) as u8, true);
        let when = upper(day_name(day, self.this_year()).as_bytes())
            .push(b"   ")
            .push(upper(from.as_bytes()).as_bytes())
            .push(b" - ")
            .push(upper(to.as_bytes()).as_bytes());
        f.text(p.x + 12, p.y + 52, label, FOCUS, when.as_bytes());
        let more = Rect::new(MARGIN, p.y + 70, WIDE, ROW);
        self.row_glass(f, more, Act::More);
        f.text(more.x + 12, more.y + 25, ui, TEXT, b"More details");
        self.text_right(f, more.x + more.w - 12, more.y + 25, ui, DIM, b">");
        self.app_marks(f, Act::More);
    }

    /// The line's reading into the form's values.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn event_take_line(&mut self) {
        let mut title = [0u8; quick::TITLE_MAX];
        let (n, day, at, len) = self.event_read(&mut title);
        self.apps.set_slot(0, &title[..n]);
        self.apps.nums[0] = day;
        self.apps.nums[1] = at;
        if let Some(k) = LASTS.iter().position(|l| *l == len) {
            self.apps.nums[2] = k as i32;
        } else {
            // A length the stepper has no step for: the nearest.
            self.apps.nums[2] = LASTS[..8]
                .iter()
                .enumerate()
                .min_by_key(|(_, l)| l.abs_diff(len))
                .map_or(3, |(k, _)| k as i32);
        }
        self.apps.len[super::LINE_SLOT] = 0;
    }

    /// Quick add's "More details": the editor with the line's reading.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn event_more(&mut self) {
        self.event_take_line();
        self.apps.typing = None;
        self.open_app(View::EventEdit);
    }

    /// Quick add's Add.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn event_add(&mut self) {
        self.event_take_line();
        self.event_done();
    }

    /// An appointment into the editor.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn event_edit(&mut self, id: u32) {
        self.apps.clear_form();
        let Some(s) = self.apps.store.as_ref() else {
            return;
        };
        let Some(r) = s.get(id) else {
            return;
        };
        let (start, whole) = (start_of(&r), all_day(&r));
        let len = r.num(tag::MINUTES).unwrap_or(60);
        let remind = r.num(tag::REMIND);
        let nums = [
            date::day_of(start),
            (start % DAY_MIN) as i32,
            if whole {
                8
            } else {
                LASTS[..8]
                    .iter()
                    .position(|l| u32::from(*l) >= len)
                    .unwrap_or(7) as i32
            },
            REMIND
                .iter()
                .position(|m| m.map(u32::from) == remind)
                .unwrap_or(0) as i32,
        ];
        let mut text = [[0u8; super::SLOT]; 3];
        let mut lens = [0usize; 3];
        for (k, t) in [tag::TITLE, tag::PLACE, tag::NOTE].into_iter().enumerate() {
            let v = r.text(t);
            lens[k] = v.len().min(super::SLOT);
            text[k][..lens[k]].copy_from_slice(&v[..lens[k]]);
        }
        for k in 0..3 {
            self.apps.set_slot(k, &text[k][..lens[k]]);
        }
        self.apps.nums = nums;
        self.apps.open = Some(id);
        self.open_app(View::EventEdit);
    }

    /// The editor's Done: the appointment kept.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn event_done(&mut self) {
        let [day, at, lasts, remind] = self.apps.nums;
        let len = LASTS[lasts.clamp(0, 8) as usize];
        let whole = len == 0;
        let start = date::start_of(day) + if whole { 0 } else { at.clamp(0, 1439) as u32 };
        let remind = REMIND[remind.clamp(0, 6) as usize];
        let id = self.apps.open;
        let (start_b, len_b, one, remind_b) = (
            num(start),
            num(u32::from(len)),
            num(1),
            num(u32::from(remind.unwrap_or(0))),
        );
        let apps = &mut self.apps;
        let (Some(store), Some(_)) = (apps.store.as_mut(), apps.scratch.as_deref()) else {
            return;
        };
        // The texts are copied out of the form's memory first (the store
        // is beside it, not in it).
        let mut text = [[0u8; super::SLOT]; 3];
        let mut lens = [0usize; 3];
        for k in 0..3 {
            let at = super::TABLE + rp_pim::store::TEXT_MAX + super::SLOT * k;
            lens[k] = usize::from(apps.len[k]);
            if let Some(s) = apps.scratch.as_deref() {
                text[k][..lens[k]].copy_from_slice(&s[at..at + lens[k]]);
            }
        }
        let title: &[u8] = if lens[0] == 0 {
            b"Appointment"
        } else {
            &text[0][..lens[0]]
        };
        let r = store.put(
            Kind::Appointment,
            id,
            &[
                (tag::TITLE, title),
                (tag::PLACE, &text[1][..lens[1]]),
                (tag::NOTE, &text[2][..lens[2]]),
                (tag::START, &start_b),
                (tag::MINUTES, if whole { &[] } else { &len_b }),
                (tag::ALL_DAY, if whole { &one } else { &[] }),
                (tag::REMIND, if remind.is_some() { &remind_b } else { &[] }),
            ],
        );
        if r.is_err() {
            self.show_notice("The organiser is full");
            return;
        }
        self.apps.day = day;
        self.apps.typing = None;
        self.apps.open = None;
        self.open_app(View::Agenda);
    }

    // ---- Drawing ----------------------------------------------------------

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn nav_row(&self, f: &mut Frame, words: &[u8]) {
        let ui = self.fonts.ui;
        panel::draw(f, NAV, self.light());
        for (r, by, mark) in [(NAV_BACK, -1, b"<"), (NAV_ON, 1, b">")] {
            if self.app_pressed(Act::Nav(by)) {
                f.push(Cmd::Fill {
                    r,
                    color: PRESSED_UNDER,
                });
            }
            keyboard::centre_in(f, r, ui, TEXT, mark);
            self.app_marks(f, Act::Nav(by));
        }
        keyboard::centre_in(f, NAV, self.fonts.label, TEXT, words);
    }

    /// An appointment's row: its time, title, and place.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn event_row(&self, f: &mut Frame, r: Rect, rec: &Rec<'_>) {
        let ui = self.fonts.ui;
        self.row_glass(f, r, Act::Open(rec.id));
        let base = r.y + 25;
        f.text(r.x + 12, base, ui, DIM, self.event_time(rec).as_bytes());
        let place = rec.text(tag::PLACE);
        let place_w = ui.width(place).min(130);
        let title = rec.text(tag::TITLE);
        let room = r.w - 104 - 12 - if place.is_empty() { 0 } else { place_w + 12 };
        self.text_cut(f, (r.x + 104, base, room), ui, TEXT, title);
        if !place.is_empty() {
            let (n, _) = self.cut(ui, place, 130);
            self.text_right(f, r.x + r.w - 12, base, ui, DIM, &place[..n]);
        }
        self.app_marks(f, Act::Open(rec.id));
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn calendar_scene(&self, f: &mut Frame, v: View) {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        for (j, (to, word)) in CHIPS.iter().enumerate() {
            self.chip(f, chip_rect(j as u16), word, *to == v, Act::Chip(*to));
        }
        let today = self.today();
        let year = self.this_year();
        let (top, bottom) = (self.list_top(v), self.app_bottom());
        match v {
            View::Agenda => {
                // Today's date above the list.
                let d = date::date_of(today);
                let head = upper(date::WEEKDAYS[usize::from(date::weekday(today))].as_bytes())
                    .push(b" ")
                    .num(i32::from(d.day))
                    .push(b" ")
                    .push(upper(date::MONTHS_LONG[usize::from(d.month - 1)].as_bytes()).as_bytes())
                    .push(b" ")
                    .num(d.year);
                panel::draw(f, NAV, self.light());
                keyboard::centre_in(f, NAV, label, TEXT, head.as_bytes());
                if self.apps.n == 0 {
                    self.empty_panel(f, top, "Nothing planned.", "New adds an appointment.");
                    return;
                }
                f.clip(Some(Rect::new(0, top, W, bottom - top)));
                let mut y = i32::from(top) - self.apps.scroll;
                for k in 0..self.apps.n {
                    let e = self.apps.entry(k);
                    let h = if e & LABEL != 0 { LABEL_H } else { PITCH };
                    if y + i32::from(h) > i32::from(top) && y < i32::from(bottom) {
                        if e & LABEL != 0 {
                            let day = (e & !LABEL) as i32;
                            let name = upper(day_name(day, year).as_bytes());
                            let words = match day - today {
                                0 => Line::new().push(b"TODAY - ").push(name.as_bytes()),
                                1 => Line::new().push(b"TOMORROW - ").push(name.as_bytes()),
                                _ => name,
                            };
                            // On its own glass: it must read over any
                            // wallpaper (the settings' section labels').
                            let lr = Rect::new(
                                MARGIN - 2,
                                y as u16 + 2,
                                label.width(words.as_bytes()) + 12,
                                LABEL_H - 4,
                            );
                            panel::draw(f, lr, self.light());
                            keyboard::centre_in(f, lr, label, TEXT, words.as_bytes());
                        } else if let Some(rec) = self.apps.rec(e) {
                            self.event_row(f, Rect::new(MARGIN, y as u16, WIDE, ROW), &rec);
                        }
                    }
                    y += i32::from(h);
                }
            }
            View::Day => {
                let d = date::date_of(self.apps.day);
                let head =
                    upper(date::WEEKDAYS[usize::from(date::weekday(self.apps.day))].as_bytes())
                        .push(b" ")
                        .num(i32::from(d.day))
                        .push(b" ")
                        .push(
                            upper(date::MONTHS_LONG[usize::from(d.month - 1)].as_bytes())
                                .as_bytes(),
                        )
                        .push(b" ")
                        .num(d.year);
                self.nav_row(f, head.as_bytes());
                f.clip(Some(Rect::new(0, top, W, bottom - top)));
                let (h0, h1, whole) = self.apps.span;
                let y0 = i32::from(top) - self.apps.scroll + i32::from(whole) * i32::from(PITCH);
                // The hours' glass, as much as shows.
                let (a, b) = (
                    y0.max(i32::from(top)),
                    (y0 + i32::from(h1 - h0) * i32::from(HOUR_H)).min(i32::from(bottom)),
                );
                if b > a {
                    panel::draw(
                        f,
                        Rect::new(MARGIN, a as u16, WIDE, (b - a) as u16),
                        self.light(),
                    );
                }
                for h in h0..h1 {
                    let y = y0 + i32::from(h - h0) * i32::from(HOUR_H);
                    if y + i32::from(HOUR_H) <= i32::from(top) || y >= i32::from(bottom) {
                        continue;
                    }
                    let y = y as u16;
                    if self.app_pressed(Act::Hour(h)) {
                        f.push(Cmd::Fill {
                            r: Rect::new(MARGIN + 1, y, WIDE - 2, HOUR_H),
                            color: PRESSED_UNDER,
                        });
                    }
                    if h > h0 {
                        f.push(Cmd::Fill {
                            r: Rect::new(HOUR_X - 6, y, W - MARGIN - HOUR_X + 5, 1),
                            color: DIM,
                        });
                    }
                    let name = self.region.time(h, 0, false);
                    f.text(
                        MARGIN + 8,
                        y + 18,
                        label,
                        DIM,
                        upper(name.as_bytes()).as_bytes(),
                    );
                    self.app_marks(f, Act::Hour(h));
                }
                // The appointments over their hours.
                for k in 0..self.apps.n {
                    let Some(rec) = self.apps.rec(self.apps.entry(k)) else {
                        continue;
                    };
                    if all_day(&rec) {
                        let y = i32::from(top) - self.apps.scroll + k as i32 * i32::from(PITCH);
                        if y + i32::from(ROW) > i32::from(top) && y < i32::from(bottom) {
                            self.event_row(f, Rect::new(MARGIN, y as u16, WIDE, ROW), &rec);
                        }
                        continue;
                    }
                    let Some((r, _, id)) = self.day_box(k) else {
                        continue;
                    };
                    if r.h < 8 {
                        continue;
                    }
                    // The pressed glass: a shade darker than the hours' own.
                    // (Flat: most are too short for the glass's corners.)
                    f.push(Cmd::Fill {
                        r,
                        color: if self.app_pressed(Act::Open(id)) {
                            PRESSED_UNDER
                        } else {
                            panel::pressed(self.light()).rows[32]
                        },
                    });
                    f.push(Cmd::Fill {
                        r: Rect::new(r.x, r.y + 3, 3, r.h.saturating_sub(6)),
                        color: FOCUS,
                    });
                    if r.h >= 22 {
                        self.text_cut(
                            f,
                            (r.x + 10, r.y + 19, r.w.saturating_sub(18)),
                            ui,
                            TEXT,
                            rec.text(tag::TITLE),
                        );
                    }
                    if r.h >= 46 {
                        f.text(
                            r.x + 10,
                            r.y + 39,
                            label,
                            DIM,
                            upper(self.event_time(&rec).as_bytes()).as_bytes(),
                        );
                    }
                    self.app_marks(f, Act::Open(id));
                }
                // Now, on today's page.
                if self.apps.day == today {
                    let m = u32::from(self.time.hour) * 60 + u32::from(self.time.minute);
                    if m >= u32::from(h0) * 60 && m < u32::from(h1) * 60 {
                        let y = y0 + ((m - u32::from(h0) * 60) * u32::from(HOUR_H) / 60) as i32;
                        if y >= i32::from(top) && y < i32::from(bottom) {
                            f.push(Cmd::Fill {
                                r: Rect::new(HOUR_X - 6, y as u16, W - MARGIN - HOUR_X + 5, 2),
                                color: FOCUS,
                            });
                        }
                    }
                }
            }
            _ => {
                let d = date::date_of(self.apps.day);
                let head = upper(date::MONTHS_LONG[usize::from(d.month - 1)].as_bytes())
                    .push(b" ")
                    .num(d.year);
                self.nav_row(f, head.as_bytes());
                for (c, name) in date::WEEKDAYS.iter().enumerate() {
                    keyboard::centre_in(
                        f,
                        Rect::new(MARGIN + c as u16 * CELL_W, TOP + 42, CELL_W, 20),
                        label,
                        TEXT,
                        upper(name.as_bytes()).as_bytes(),
                    );
                }
                panel::draw(f, Rect::new(MARGIN, GRID_Y, WIDE, 6 * CELL_H), self.light());
                let first = self.grid_start();
                for c in 0..42u16 {
                    let day = first + i32::from(c);
                    let cd = date::date_of(day);
                    let r = Rect::new(
                        MARGIN + c % 7 * CELL_W,
                        GRID_Y + c / 7 * CELL_H,
                        CELL_W,
                        CELL_H,
                    );
                    let inner = Rect::new(r.x + 2, r.y + 2, r.w - 4, r.h - 4);
                    if day == self.apps.day || self.app_pressed(Act::Cell(day)) {
                        f.push(Cmd::Fill {
                            r: inner,
                            color: PRESSED_UNDER,
                        });
                    }
                    if day == today {
                        f.push(Cmd::Outline {
                            r: inner,
                            color: FOCUS,
                        });
                    }
                    let n = Line::new().num(i32::from(cd.day));
                    keyboard::centre_in(
                        f,
                        Rect::new(r.x, r.y + 2, r.w, 24),
                        ui,
                        if cd.month == d.month { TEXT } else { DIM },
                        n.as_bytes(),
                    );
                    let dots = usize::from(self.apps.dots[usize::from(c)]);
                    if dots > 0 {
                        keyboard::centre_in(
                            f,
                            Rect::new(r.x, r.y + 22, r.w, 18),
                            ui,
                            FOCUS,
                            &[0x80; 3][..dots],
                        );
                    }
                    self.app_marks(f, Act::Cell(day));
                }
                // The chosen day's appointments.
                if self.apps.n == 0 {
                    let words = day_name(self.apps.day, year).push(b": nothing planned");
                    let p = Rect::new(MARGIN, MONTH_LIST_Y, WIDE, ROW);
                    panel::draw(f, p, self.light());
                    f.text(p.x + 12, p.y + 25, ui, DIM, words.as_bytes());
                }
                for k in 0..self.apps.n {
                    if let Some(rec) = self.apps.rec(self.apps.entry(k)) {
                        self.event_row(
                            f,
                            Rect::new(MARGIN, MONTH_LIST_Y + k as u16 * PITCH, WIDE, ROW),
                            &rec,
                        );
                    }
                }
            }
        }
    }
}
