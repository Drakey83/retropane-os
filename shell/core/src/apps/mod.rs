//! The organiser apps of batch 1 (docs/design/core-apps-list.md): Calendar,
//! Contacts, Tasks, Notes and the Calculator, inside the shell's frame
//! (docs/design/design-language.md). Each app's Rule 0 sheet is
//! `docs/design/<app>-rule0.md`; the four that keep records share one
//! store (`rp_pim::store`, docs/design/pim-store.md).
//!
//! What they share lives here: the memory they are given, today's date,
//! the list they scroll, the targets on screen, the form an editor is, the
//! text being typed, and the reminders' banner. The apps' own screens are
//! in the modules beside this one.
//!
//! Iterate, don't imitate (rule 0): Windows Mobile gave each of these its
//! own forms, pickers and database; ours are one frame, one store, one
//! quick-add line and one form engine, so five apps cost the code of two.

use rp_draw::{Argb, Cmd, Font, Frame, Rect};
use rp_pim::calc::Calc;
use rp_pim::date::{self, Date};
use rp_pim::store::{Kind, Rec, Store};
use rp_weather::Line;

use super::keyboard::{self, Cap};
use super::{DIM, FOCUS, H, Screen, Shell, TEXT, Target, W, panel};

mod calculator;
mod calendar;
mod contacts;
mod form;
mod notes;
pub mod preview;
mod tasks;
#[cfg(test)]
mod tests;
mod text;

/// The memory the apps are given ([`Shell::give_apps`]): the store's, then
/// the views' table, the text being typed and the form's fields.
pub const MEMORY: usize = rp_pim::store::MEMORY + SCRATCH;
/// The views' table: [`TABLE_MAX`] record handles of four bytes.
const TABLE: usize = 4 * TABLE_MAX;
/// Entries a list holds.
pub(crate) const TABLE_MAX: usize = 1024;
/// A form's text fields: [`SLOTS`] of [`SLOT`] bytes.
const SLOT: usize = 96;
const SLOTS: usize = 10;
/// The slot the search and quick-add lines are typed into.
const LINE_SLOT: usize = 9;
const SCRATCH: usize = TABLE + rp_pim::store::TEXT_MAX + SLOT * SLOTS;

/// The content's top (under the header) and bottom (above the softkeys).
pub(crate) const TOP: u16 = 98;
const BOTTOM: u16 = 594;
/// A row and the gap under it.
pub(crate) const ROW: u16 = 36;
pub(crate) const PITCH: u16 = 40;
/// The side margin and the content's width.
pub(crate) const MARGIN: u16 = 16;
pub(crate) const WIDE: u16 = W - 2 * MARGIN;
/// The text field under the header (search, quick add).
pub(crate) const FIELD: Rect = Rect::new(MARGIN, 100, WIDE, 40);
/// Red for late and delete, green for done (design-language.md).
pub(crate) const LATE: Argb = rp_draw::look::theme(0xFFFF_7A6B);
pub(crate) const DONE: Argb = rp_draw::look::theme(0xFF5C_E07A);
/// How far the pen moves before a press is a drag.
const SLOP: i32 = 8;
/// The reminder banner (under the title bar, on any screen).
const BANNER: Rect = Rect::new(8, 40, W - 16, 64);
const BANNER_DISMISS: Rect = Rect::new(W - 8 - 200, 68, 96, 32);
const BANNER_OPEN: Rect = Rect::new(W - 8 - 100, 68, 92, 32);
/// Reminders waiting at once.
const DUE_MAX: usize = 4;

/// An app's screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    /// Calendar: what is coming, by day.
    Agenda,
    /// Calendar: one day by the hour.
    Day,
    /// Calendar: a month and the chosen day's appointments.
    Month,
    /// Calendar: the quick-add line.
    NewEvent,
    /// Calendar: an appointment's details.
    EventEdit,
    /// Contacts: the list.
    Contacts,
    /// Contacts: one person.
    Card,
    /// Contacts: a card being changed.
    ContactEdit,
    /// Tasks: the list.
    Tasks,
    /// Tasks: one task's details.
    TaskEdit,
    /// Notes: the list.
    Notes,
    /// Notes: a note being written.
    Note,
    /// The calculator.
    Calculator,
}

/// What a target on an app's screen does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Act {
    /// Nothing (an unused slot).
    None,
    /// A chip of the header's row: the view to show.
    Chip(View),
    /// Earlier (-1) or later (1): a day, a month.
    Nav(i8),
    /// A record's row: open it (its id).
    Open(u32),
    /// A day of the month grid.
    Cell(i32),
    /// An hour row of the day view: a new appointment then.
    Hour(u8),
    /// A text field of the form, or the search or quick-add field.
    Field(u8),
    /// A stepper's arrow: the value `which` down or up.
    Step(u8, i8),
    /// A choice chip: value `which` set to `to`.
    Choice(u8, u8),
    /// The form's Delete row.
    Delete,
    /// Quick add's "More details".
    More,
    /// A task's box: done or open again (its id).
    Tick(u32),
    /// A letter of the contacts' strip.
    Letter(u8),
    /// A calculator key.
    Key(rp_pim::calc::Key),
    /// The note's text: the caret goes where the pen is.
    Text,
    /// The reminder banner's Dismiss and Open.
    Dismiss,
    /// See [`Act::Dismiss`].
    OpenDue,
}

/// Targets a screen has at most.
const TARGETS: usize = 80;

/// What is being typed into.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Typing {
    /// A form's text field (its slot).
    Slot(u8),
    /// The search or quick-add line.
    Line,
    /// The note's text.
    Note,
}

/// The apps' state (small: the big parts are in the memory given).
pub(crate) struct Apps {
    pub(crate) store: Option<Store<'static>>,
    scratch: Option<&'static mut [u8]>,
    /// The year, when the caller has said ([`Shell::set_year`]).
    year: Option<u16>,
    /// Entries in the table, and the store's change count and the day
    /// they were made at.
    pub(crate) n: usize,
    seen: Option<(u32, i32)>,
    /// The list's scroll (px), its content's height, and the pen's drag.
    pub(crate) scroll: i32,
    pub(crate) height: i32,
    drag: Option<(u16, i32, bool)>,
    targets: [(Rect, Act); TARGETS],
    n_targets: usize,
    /// The calendar's chosen day.
    pub(crate) day: i32,
    /// The record open (a card, an editor): `None` for a new one.
    pub(crate) open: Option<u32>,
    /// The form's text lengths and numbers.
    pub(crate) len: [u8; SLOTS],
    pub(crate) nums: [i32; 4],
    /// The note's length, its caret and its first line shown.
    pub(crate) text_n: usize,
    pub(crate) caret: usize,
    pub(crate) text_top: usize,
    pub(crate) typing: Option<Typing>,
    /// Tasks: done ones hidden.
    pub(crate) hide_done: bool,
    /// Delete was tapped once: the next tap on it removes.
    pub(crate) confirm: bool,
    pub(crate) calc: Calc,
    /// The month view's dots, a count a cell.
    pub(crate) dots: [u8; 42],
    /// The day view's first and last hour and its all-day rows.
    pub(crate) span: (u8, u8, u8),
    /// Contacts: the strip's letters that have someone (a bit each; bit
    /// 26 for names that start with no letter).
    pub(crate) letters: u32,
    /// Reminders waiting (appointment ids), the minute last checked and
    /// the next one due.
    due: [u32; DUE_MAX],
    n_due: usize,
    checked: Option<u32>,
    next_due: Option<u32>,
    /// The store's change count the next reminder was worked out at.
    planned: Option<u32>,
}

impl Apps {
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) const fn new() -> Self {
        Self {
            store: None,
            scratch: None,
            year: None,
            n: 0,
            seen: None,
            scroll: 0,
            height: 0,
            drag: None,
            targets: [(Rect::new(0, 0, 0, 0), Act::None); TARGETS],
            n_targets: 0,
            day: 0,
            open: None,
            len: [0; SLOTS],
            nums: [0; 4],
            text_n: 0,
            caret: 0,
            text_top: 0,
            typing: None,
            hide_done: false,
            confirm: false,
            calc: Calc::new(),
            dots: [0; 42],
            span: (6, 22, 0),
            letters: 0,
            due: [0; DUE_MAX],
            n_due: 0,
            checked: None,
            next_due: None,
            planned: None,
        }
    }

    // ---- The memory's parts -------------------------------------------

    /// Table entry `k`.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn entry(&self, k: usize) -> u32 {
        self.scratch
            .as_deref()
            .and_then(|s| s.get(4 * k..4 * k + 4))
            .and_then(|b| b.try_into().ok())
            .map_or(0, u32::from_le_bytes)
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn set_entry(&mut self, k: usize, v: u32) {
        if let Some(b) = self
            .scratch
            .as_deref_mut()
            .and_then(|s| s.get_mut(4 * k..4 * k + 4))
        {
            b.copy_from_slice(&v.to_le_bytes());
        }
    }

    /// The note's text buffer.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn note(&self) -> &[u8] {
        match self.scratch.as_deref() {
            Some(s) => &s[TABLE..TABLE + self.text_n],
            None => &[],
        }
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn note_buf(&mut self) -> &mut [u8] {
        match self.scratch.as_deref_mut() {
            Some(s) => &mut s[TABLE..TABLE + rp_pim::store::TEXT_MAX],
            None => &mut [],
        }
    }

    /// Text slot `k` as typed.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn slot(&self, k: usize) -> &[u8] {
        let at = TABLE + rp_pim::store::TEXT_MAX + SLOT * k;
        match self.scratch.as_deref() {
            Some(s) if k < SLOTS => &s[at..at + usize::from(self.len[k])],
            _ => &[],
        }
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn set_slot(&mut self, k: usize, text: &[u8]) {
        let at = TABLE + rp_pim::store::TEXT_MAX + SLOT * k;
        if let (Some(s), true) = (self.scratch.as_deref_mut(), k < SLOTS) {
            let n = text.len().min(SLOT);
            s[at..at + n].copy_from_slice(&text[..n]);
            self.len[k] = n as u8;
        }
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn slot_push(&mut self, k: usize, b: u8) {
        let at = TABLE + rp_pim::store::TEXT_MAX + SLOT * k;
        let n = usize::from(self.len[k]);
        if let (Some(s), true) = (self.scratch.as_deref_mut(), n < SLOT) {
            s[at + n] = b;
            self.len[k] += 1;
        }
    }

    /// The line being typed (search, quick add).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn line(&self) -> &[u8] {
        self.slot(LINE_SLOT)
    }

    /// The lists are made again at the next refresh (the day turned).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) const fn forget_lists(&mut self) {
        self.seen = None;
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn clear_form(&mut self) {
        self.len = [0; SLOTS];
        self.nums = [0; 4];
        self.text_n = 0;
        self.caret = 0;
        self.text_top = 0;
        self.typing = None;
        self.confirm = false;
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn target(&mut self, r: Rect, act: Act) {
        if self.n_targets < TARGETS && r.w > 0 && r.h > 0 {
            self.targets[self.n_targets] = (r, act);
            self.n_targets += 1;
        }
    }

    /// The table from entry `from` filled with the handles of the `kind`
    /// records `keep` takes: how many.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn fill(
        &mut self,
        kind: Kind,
        from: usize,
        keep: impl Fn(&Rec<'_>) -> bool,
    ) -> usize {
        let (Some(store), Some(s)) = (self.store.as_ref(), self.scratch.as_deref_mut()) else {
            return 0;
        };
        let mut n = 0;
        for (handle, r) in store.handles(kind) {
            if from + n == TABLE_MAX {
                break;
            }
            if keep(&r) {
                s[4 * (from + n)..4 * (from + n) + 4].copy_from_slice(&handle.to_le_bytes());
                n += 1;
            }
        }
        n
    }

    /// Entries `from..from + n` of the table put in `order`: one heap
    /// sort for every list (the library's generic sort was about 12 KB a
    /// list in the image; this is a few hundred bytes for all of them).
    /// Run when the records change, never while the pen moves (P1).
    #[inline(never)]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn sort(
        &mut self,
        from: usize,
        n: usize,
        order: &dyn Fn(&Rec<'_>, &Rec<'_>) -> core::cmp::Ordering,
    ) {
        let (Some(store), Some(s)) = (self.store.as_ref(), self.scratch.as_deref_mut()) else {
            return;
        };
        let (e, _) = s[4 * from..4 * (from + n)].as_chunks_mut::<4>();
        let after = |a: &[u8; 4], b: &[u8; 4]| match (
            store.at(u32::from_le_bytes(*a)),
            store.at(u32::from_le_bytes(*b)),
        ) {
            (Some(x), Some(y)) => order(&x, &y) == core::cmp::Ordering::Greater,
            _ => false,
        };
        // Entry `root` sinks to its place in the heap `e[..end]`.
        let sink = |e: &mut [[u8; 4]], mut root: usize, end: usize| {
            loop {
                let mut child = 2 * root + 1;
                if child >= end {
                    break;
                }
                if child + 1 < end && after(&e[child + 1], &e[child]) {
                    child += 1;
                }
                if !after(&e[child], &e[root]) {
                    break;
                }
                e.swap(root, child);
                root = child;
            }
        };
        for root in (0..n / 2).rev() {
            sink(e, root, n);
        }
        for end in (1..n).rev() {
            e.swap(0, end);
            sink(e, 0, end);
        }
    }

    /// The store's record at a handle of the table.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn rec(&self, handle: u32) -> Option<Rec<'_>> {
        self.store.as_ref()?.at(handle)
    }
}

/// A number field of a record as bytes for [`Store::put`].
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub(crate) const fn num(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

/// "Fri 13 Nov" (and the year when it isn't `this_year`).
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub(crate) fn day_name(day: i32, this_year: i32) -> Line {
    let d = date::date_of(day);
    let l = Line::new()
        .push(date::WEEKDAYS[usize::from(date::weekday(day))].as_bytes())
        .push(b" ")
        .num(i32::from(d.day))
        .push(b" ")
        .push(date::MONTHS[usize::from(d.month - 1)].as_bytes());
    if d.year == this_year {
        l
    } else {
        l.push(b" ").num(d.year)
    }
}

/// `text` in capitals (ASCII), for the label font.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub(crate) fn upper(text: &[u8]) -> Line {
    let mut l = Line::new();
    for b in text {
        l = l.push(&[b.to_ascii_uppercase()]);
    }
    l
}

impl Shell {
    // ---- Memory, the clock --------------------------------------------

    /// Gives the organiser apps their memory ([`MEMORY`] bytes): the store
    /// starts unloaded, and the caller loads it from the card
    /// ([`Shell::pim`]). Without it the apps say so and keep nothing.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn give_apps(&mut self, memory: &'static mut [u8]) {
        if memory.len() < MEMORY {
            return;
        }
        let (store, scratch) = memory.split_at_mut(memory.len() - SCRATCH);
        scratch.fill(0);
        self.apps.store = Some(Store::new(store));
        self.apps.scratch = Some(scratch);
        self.apps.seen = None;
    }

    /// The organiser's store, for the caller to load from the card and
    /// write to it (`rp_pim::store`: `load_want`, `next_write`).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn pim(&mut self) -> Option<&mut Store<'static>> {
        self.apps.store.as_mut()
    }

    /// The store finished loading (or its records were changed from
    /// outside the shell): the screens show them, and reminders are
    /// looked for.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn pim_loaded(&mut self) {
        self.apps.seen = None;
        self.app_refresh();
        self.reminders_tick();
        if self.view().is_some() {
            self.dirty(Rect::new(0, 0, W, H));
        }
    }

    /// The year (the shell's [`super::Time`] has none): the calendar's
    /// dates need it. Without it the year is the first from 2024 on whose
    /// date falls on the weekday the clock shows.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn set_year(&mut self, year: u16) {
        if self.apps.year != Some(year) {
            self.apps.year = Some(year);
            self.apps.seen = None;
        }
    }

    /// Today, as a day count.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn today(&self) -> i32 {
        let t = self.time;
        let on = |year: i32| {
            date::days_from(Date {
                year,
                month: t.month.clamp(1, 12),
                day: t.day.max(1),
            })
        };
        match self.apps.year {
            Some(y) => on(i32::from(y)),
            None => (2024..2052)
                .map(on)
                .find(|&d| date::weekday(d) == t.weekday)
                .unwrap_or_else(|| on(2024)),
        }
    }

    /// Now, in local minutes from 1970.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn minute_now(&self) -> u32 {
        date::start_of(self.today()) + u32::from(self.time.hour) * 60 + u32::from(self.time.minute)
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn this_year(&self) -> i32 {
        date::date_of(self.today()).year
    }

    /// Whether typed dates read month first (the US) or day first.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn month_first(&self) -> bool {
        self.region.clock12
    }

    // ---- Opening and leaving ------------------------------------------

    /// The view up, if an app is.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) const fn view(&self) -> Option<View> {
        match self.screen {
            Screen::App(v) => Some(v),
            _ => None,
        }
    }

    /// Opens an app's view.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn open_app(&mut self, v: View) {
        let from = self.view();
        self.screen = Screen::App(v);
        self.place_menu = false;
        self.chart = None;
        self.pressed = None;
        self.anim = None;
        self.gesture = super::Gesture::None;
        self.kb = keyboard::Keyboard::default();
        self.apps.drag = None;
        self.apps.confirm = false;
        // A list keeps its place when its editor closes; a new app starts
        // at its top.
        let list = |v| matches!(v, View::Agenda | View::Contacts | View::Tasks | View::Notes);
        if !(list(v) && from.is_some_and(|f| !list(f))) {
            self.apps.scroll = 0;
        }
        if matches!(v, View::Agenda | View::Day | View::Month) && from.is_none() {
            self.apps.day = self.today();
        }
        // The keyboard stays only for a screen that types (its caller set
        // what is typed into).
        if !matches!(
            v,
            View::NewEvent | View::EventEdit | View::ContactEdit | View::TaskEdit | View::Note
        ) {
            self.apps.typing = None;
        }
        self.apps.seen = None;
        self.app_refresh();
        if self.focus.is_some() {
            self.focus = Some(if self.apps.n_targets > 0 {
                Target::App(0)
            } else {
                Target::Back
            });
        }
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// One screen back from an app's view.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_back(&mut self, v: View) {
        if self.apps.typing.is_some() && !matches!(v, View::Note | View::NewEvent) {
            // The keyboard goes first.
            self.app_typing(None);
            return;
        }
        match v {
            View::NewEvent | View::EventEdit => self.open_app(View::Agenda),
            View::Card => self.open_app(View::Contacts),
            View::ContactEdit => match self.apps.open {
                Some(_) => self.open_app(View::Card),
                None => self.open_app(View::Contacts),
            },
            View::TaskEdit => self.open_app(View::Tasks),
            View::Note => {
                // A note keeps itself on every way out (notes-rule0.md).
                self.note_keep();
                self.open_app(View::Notes);
            }
            _ => {
                self.apps.typing = None;
                self.go_today();
            }
        }
    }

    /// Leaving the apps for Today (Home): a note open is kept.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_leave(&mut self) {
        if self.view() == Some(View::Note) {
            self.note_keep();
        }
        self.apps.typing = None;
    }

    /// Shows or hides the keyboard for what is typed into.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_typing(&mut self, t: Option<Typing>) {
        if self.apps.typing != t {
            self.apps.typing = t;
            self.kb = keyboard::Keyboard::default();
            self.app_refresh();
            self.dirty(Rect::new(0, 0, W, H));
        }
    }

    // ---- The frame around every app screen ------------------------------

    /// The app's name in the title bar.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) const fn app_title(v: View) -> &'static str {
        match v {
            View::Agenda | View::Day | View::Month => "Calendar",
            View::NewEvent => "New appointment",
            View::EventEdit => "Appointment",
            View::Contacts => "Contacts",
            View::Card => "Contact",
            View::ContactEdit => "Contact",
            View::Tasks => "Tasks",
            View::TaskEdit => "Task",
            View::Notes => "Notes",
            View::Note => "Note",
            View::Calculator => "Calculator",
        }
    }

    /// The softkeys' names on the app's view.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_softkeys(&self, v: View) -> (&'static str, &'static str) {
        let typing = self.apps.typing.is_some();
        let del = if self.apps.confirm {
            "Delete?"
        } else {
            "Delete"
        };
        match v {
            View::Agenda | View::Day | View::Month => ("Today", "New"),
            View::NewEvent => ("Cancel", "Add"),
            View::EventEdit | View::ContactEdit | View::TaskEdit => ("Cancel", "Done"),
            View::Contacts | View::Notes if typing => ("Cancel", "Done"),
            View::Contacts | View::Notes => ("Back", "New"),
            View::Card => ("Back", "Edit"),
            View::Tasks if typing => ("Cancel", "Add"),
            View::Tasks if self.apps.hide_done => ("Back", "Show done"),
            View::Tasks => ("Back", "Hide done"),
            View::Note => (del, "Done"),
            View::Calculator => ("Back", ""),
        }
    }

    /// A softkey on the app's view (`right`: the right one).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_soft(&mut self, v: View, right: bool) {
        match (v, right) {
            (View::Agenda | View::Day | View::Month, false) => {
                self.apps.day = self.today();
                self.apps.scroll = 0;
                self.app_changed();
            }
            (View::Agenda | View::Day | View::Month, true) => self.event_new(None),
            (View::NewEvent, true) => self.event_add(),
            (View::EventEdit, true) => self.event_done(),
            (View::ContactEdit, true) => self.contact_done(),
            (View::TaskEdit, true) => self.task_done(),
            (View::Contacts | View::Notes, _) if self.apps.typing.is_some() => {
                if !right {
                    self.apps.len[LINE_SLOT] = 0;
                }
                self.app_typing(None);
                self.app_changed();
            }
            (View::Contacts, true) => self.contact_edit(None),
            (View::Notes, true) => self.note_open(None),
            (View::Card, true) => self.contact_edit(self.apps.open),
            (View::Tasks, _) if self.apps.typing.is_some() => {
                if right {
                    self.task_add();
                } else {
                    self.apps.len[LINE_SLOT] = 0;
                    self.app_typing(None);
                }
            }
            (View::Tasks, true) => {
                self.apps.hide_done = !self.apps.hide_done;
                self.apps.scroll = 0;
                self.app_changed();
            }
            (View::Note, false) => self.note_delete(),
            (View::Calculator, true) => {}
            _ => self.app_back(v),
        }
    }

    /// The store's state in words when the apps can't work yet.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn store_says(&self) -> Option<&'static str> {
        match &self.apps.store {
            None => Some("The organiser has no memory here"),
            Some(s) if !s.loaded() => Some("Reading the card"),
            Some(_) => None,
        }
    }

    // ---- Lists, targets, refresh ----------------------------------------

    /// The content's bottom: above the keyboard while one is up.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_bottom(&self) -> u16 {
        if self.apps.typing.is_some() {
            keyboard::PANEL.y - 6
        } else {
            BOTTOM
        }
    }

    /// Something the screen shows changed: its list and targets again.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_changed(&mut self) {
        self.apps.seen = None;
        self.app_refresh();
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// The view's list (when the store or the day changed since it was
    /// made) and its targets. Called after every change and before a
    /// frame: never while the pen moves a list (P1), where only the
    /// targets are placed again.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_refresh(&mut self) {
        let Some(v) = self.view() else {
            self.apps.n_targets = 0;
            self.banner_targets();
            return;
        };
        let now = (
            self.apps.store.as_ref().map_or(0, Store::changes),
            self.today(),
        );
        if self.apps.seen != Some(now) {
            self.apps.seen = Some(now);
            match v {
                View::Agenda | View::Day | View::Month => self.calendar_build(v),
                View::Contacts => self.contacts_build(),
                View::Tasks => self.tasks_build(),
                View::Notes => self.notes_build(),
                _ => {}
            }
        }
        let view_h = i32::from(self.app_bottom()) - i32::from(self.list_top(v));
        self.apps.scroll = self
            .apps
            .scroll
            .clamp(0, (self.apps.height - view_h).max(0));
        self.apps.n_targets = 0;
        // The banner's buttons first: they lie over the header's chips.
        self.banner_targets();
        match v {
            View::Agenda | View::Day | View::Month => self.calendar_targets(v),
            View::NewEvent => self.event_new_targets(),
            View::EventEdit | View::ContactEdit | View::TaskEdit => self.form_targets(v),
            View::Contacts => self.contacts_targets(),
            View::Card => {}
            View::Tasks => self.tasks_targets(),
            View::Notes => self.notes_targets(),
            View::Note => self.note_targets(),
            View::Calculator => self.calculator_targets(),
        }
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn banner_targets(&mut self) {
        if self.apps.n_due > 0 {
            self.apps.target(BANNER_DISMISS, Act::Dismiss);
            self.apps.target(BANNER_OPEN, Act::OpenDue);
        }
    }

    /// Where the view's scrolling list starts.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn list_top(&self, v: View) -> u16 {
        match v {
            View::Contacts | View::Notes | View::Tasks => FIELD.y + FIELD.h + 6,
            View::Agenda => TOP + 44,
            View::Day => TOP + 44,
            _ => TOP,
        }
    }

    /// A list row at content offset `y` of height `h`, as it shows now
    /// (cut to the list's viewport; empty when outside it).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn row_at(&self, v: View, y: i32, h: u16, x: u16, w: u16) -> Rect {
        let top = i32::from(self.list_top(v));
        let bottom = i32::from(self.app_bottom());
        let y0 = top + y - self.apps.scroll;
        let (a, b) = (y0.max(top), (y0 + i32::from(h)).min(bottom));
        if b <= a {
            Rect::new(0, 0, 0, 0)
        } else {
            Rect::new(x, a as u16, w, (b - a) as u16)
        }
    }

    /// Adds a target (the apps' modules place theirs through this).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_target(&mut self, r: Rect, act: Act) {
        self.apps.target(r, act);
    }

    /// How many targets the screen has ([`Target::App`]'s range).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) const fn app_targets(&self) -> u8 {
        self.apps.n_targets as u8
    }

    /// Where target `i` is.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_rect(&self, i: u8) -> Rect {
        self.apps
            .targets
            .get(usize::from(i))
            .filter(|_| usize::from(i) < self.apps.n_targets)
            .map_or(Rect::new(0, 0, 0, 0), |t| t.0)
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_act(&self, i: u8) -> Act {
        self.apps
            .targets
            .get(usize::from(i))
            .filter(|_| usize::from(i) < self.apps.n_targets)
            .map_or(Act::None, |t| t.1)
    }

    /// The target doing `act`, to draw its press and focus.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_find(&self, act: Act) -> Option<Target> {
        self.apps.targets[..self.apps.n_targets]
            .iter()
            .position(|t| t.1 == act)
            .map(|i| Target::App(i as u8))
    }

    /// Press and focus marks for the target doing `act`.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_marks(&self, f: &mut Frame, act: Act) {
        if let Some(t) = self.app_find(act) {
            self.decorate(f, t);
        }
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_pressed(&self, act: Act) -> bool {
        self.app_find(act).is_some_and(|t| self.pressed == Some(t))
    }

    // ---- Input ----------------------------------------------------------

    /// The pen on an app's list: a drag scrolls it (and is no tap). `None`
    /// leaves the event to the shell's own handling.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_pen(&mut self, e: super::Input) -> Option<super::Action> {
        use super::{Action, Input};
        let v = self.view()?;
        if self.menu_open {
            return None;
        }
        let scrolls = matches!(
            v,
            View::Agenda
                | View::Day
                | View::Contacts
                | View::Tasks
                | View::Notes
                | View::EventEdit
                | View::ContactEdit
                | View::TaskEdit
                | View::Card
        );
        match e {
            Input::PenDown { x, y } => {
                let inside = y >= self.list_top(v) && y < self.app_bottom() && x < W - 24;
                self.apps.drag = (scrolls && inside).then_some((y, self.apps.scroll, false));
                None
            }
            Input::PenMove { y, .. } => {
                let (y0, from, moved) = self.apps.drag?;
                let dy = i32::from(y0) - i32::from(y);
                if !moved && dy.abs() <= SLOP {
                    return None;
                }
                self.apps.drag = Some((y0, from, true));
                self.set_pressed(None);
                let was = self.apps.scroll;
                self.apps.scroll = from + dy;
                self.app_refresh();
                if self.apps.scroll != was {
                    self.dirty(Rect::new(0, TOP, W, BOTTOM - TOP));
                    return Some(Action::Redraw);
                }
                Some(Action::Nothing)
            }
            Input::PenUp { x, y } => {
                let drag = self.apps.drag.take();
                if matches!(drag, Some((_, _, true))) {
                    self.set_pressed(None);
                    return Some(Action::Nothing);
                }
                // A tap in the note's text places the caret.
                if v == View::Note
                    && self.pressed.is_some_and(
                        |t| matches!(t, Target::App(i) if self.app_act(i) == Act::Text),
                    )
                {
                    self.set_pressed(None);
                    self.note_tap(x, y);
                    return Some(Action::Redraw);
                }
                None
            }
            Input::Key(k) => self.app_key(v, k),
            _ => None,
        }
    }

    /// A key on an app's view, before the shell's focus handling.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn app_key(&mut self, v: View, k: super::Key) -> Option<super::Action> {
        use super::{Action, Key};
        match (v, k) {
            // The caret in a note: left and right a character, up and
            // down a line (while the focus isn't on the keyboard's keys).
            (View::Note, Key::Left | Key::Right | Key::Up | Key::Down)
                if !matches!(
                    self.focus,
                    Some(Target::Cap(_) | Target::SoftLeft | Target::SoftRight)
                ) =>
            {
                self.note_move(k);
                Some(Action::Redraw)
            }
            // Contacts: left and right jump a letter.
            (View::Contacts, Key::Left | Key::Right) if self.apps.typing.is_none() => {
                self.contacts_jump(if k == Key::Left { -1 } else { 1 });
                Some(Action::Redraw)
            }
            // Down past the last row showing (or up past the first): the
            // list moves a row and the focus stays where the next row
            // arrives.
            (_, Key::Up | Key::Down) => {
                let Some(Target::App(i)) = self.focus else {
                    return None;
                };
                let (top, bottom) = (self.list_top(v), self.app_bottom());
                let r = self.app_rect(i);
                let in_list = |r: Rect| r.y >= top && r.y + r.h <= bottom && r.h > 0;
                if !in_list(r) || matches!(self.app_act(i), Act::Letter(_)) {
                    return None;
                }
                // Another row in that direction: the shell's own move.
                let on = self
                    .neighbour(Target::App(i), k)
                    .is_some_and(|t| matches!(t, Target::App(j) if in_list(self.app_rect(j))));
                let by = if k == Key::Down {
                    i32::from(r.h) + 4
                } else {
                    -(i32::from(r.h) + 4)
                };
                let was = self.apps.scroll;
                if on {
                    return None;
                }
                self.apps.scroll += by;
                self.app_refresh();
                if self.apps.scroll == was {
                    return None;
                }
                // The row now nearest where the focus was.
                let (cx, cy) = (
                    i32::from(r.x) + i32::from(r.w) / 2,
                    i32::from(r.y) + i32::from(r.h) / 2,
                );
                let near = (0..self.app_targets())
                    .filter(|j| {
                        in_list(self.app_rect(*j)) && !matches!(self.app_act(*j), Act::Letter(_))
                    })
                    .min_by_key(|j| {
                        let q = self.app_rect(*j);
                        (i32::from(q.x) + i32::from(q.w) / 2 - cx).abs()
                            + (i32::from(q.y) + i32::from(q.h) / 2 - cy).abs()
                    });
                self.focus = near.map(Target::App).or(self.focus);
                self.dirty(Rect::new(0, TOP, W, BOTTOM - TOP));
                Some(Action::Redraw)
            }
            _ => None,
        }
    }

    /// After the D-pad moved the focus: the list scrolls to show it.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_follow_focus(&mut self) {
        let (Some(v), Some(Target::App(i))) = (self.view(), self.focus) else {
            return;
        };
        let r = self.app_rect(i);
        let (top, bottom) = (self.list_top(v), self.app_bottom());
        if r.y >= top && r.y + r.h <= bottom && r.h >= ROW.min(r.h) {
            return;
        }
        let act = self.app_act(i);
        // A row cut by the viewport's edge: a row's height up or down.
        let by = if r.y <= top {
            -i32::from(PITCH)
        } else {
            i32::from(PITCH)
        };
        self.apps.scroll += by;
        self.app_refresh();
        self.focus = self.app_find(act).or(self.focus);
        self.dirty(Rect::new(0, TOP, W, BOTTOM - TOP));
    }

    /// A tap (or Enter) on an app target.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_tapped(&mut self, i: u8) {
        let act = self.app_act(i);
        let Some(v) = self.view() else {
            // Only the banner's buttons exist outside the apps.
            return self.banner_act(act);
        };
        if act != Act::Delete {
            self.apps.confirm = false;
        }
        match act {
            Act::None => {}
            Act::Dismiss | Act::OpenDue => self.banner_act(act),
            Act::Chip(to) => self.open_app(to),
            Act::Key(k) => {
                self.apps.calc.press(k);
                self.dirty(calculator::DISPLAY);
                self.dirty(self.app_rect(i));
            }
            Act::Text => {}
            Act::Field(k) => self.field_tapped(v, k),
            Act::Step(which, by) => self.form_step(v, which, by),
            Act::Choice(which, to) => {
                self.apps.nums[usize::from(which)] = i32::from(to);
                self.dirty(Rect::new(0, TOP, W, BOTTOM - TOP));
            }
            Act::Delete => self.form_delete(v),
            Act::More => self.event_more(),
            Act::Nav(by) => self.calendar_nav(v, by),
            Act::Cell(day) => self.calendar_cell(day),
            Act::Hour(h) => self.event_new(Some(h)),
            Act::Open(id) => match v {
                View::Agenda | View::Day | View::Month => self.event_edit(id),
                View::Contacts => {
                    self.apps.open = Some(id);
                    self.apps.typing = None;
                    self.open_app(View::Card);
                }
                View::Tasks => self.task_edit(id),
                View::Notes => self.note_open(Some(id)),
                _ => {}
            },
            Act::Tick(id) => self.task_tick(id),
            Act::Letter(l) => self.contacts_letter(l),
        }
    }

    /// A field tapped: the keyboard for it.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn field_tapped(&mut self, v: View, k: u8) {
        let t = match v {
            View::Contacts | View::Notes | View::Tasks | View::NewEvent => Typing::Line,
            _ => Typing::Slot(k),
        };
        self.app_typing(Some(t));
        if let Typing::Slot(_) = t {
            self.form_show_typed(v);
        }
    }

    /// A key of the on-screen keyboard while an app types.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_cap(&mut self, i: u8) {
        let (Some(v), Some((cap, r))) = (self.view(), self.kb.cap(i)) else {
            return;
        };
        let Some(typing) = self.apps.typing else {
            return;
        };
        let before = self.kb;
        let slot = match typing {
            Typing::Slot(k) => usize::from(k),
            Typing::Line => LINE_SLOT,
            Typing::Note => SLOTS,
        };
        match self.kb.press(cap) {
            Cap::Char(c) => {
                if typing == Typing::Note {
                    self.note_type(c);
                } else {
                    self.apps.slot_push(slot, c);
                }
            }
            Cap::Delete => {
                if typing == Typing::Note {
                    self.note_delete_back();
                } else {
                    self.apps.len[slot] = self.apps.len[slot].saturating_sub(1);
                }
            }
            Cap::Done => match (v, typing) {
                // In a note the big key is Return (notes-rule0.md).
                (_, Typing::Note) => self.note_type(b'\n'),
                (View::NewEvent, _) => return self.event_add(),
                (View::Tasks, _) => return self.task_add(),
                // A form: on to the next text field, the keyboard down
                // after the last.
                (_, Typing::Slot(k)) => return self.form_next(v, k),
                _ => return self.app_typing(None),
            },
            Cap::Paste => self.show_notice("Clipboard empty"),
            Cap::FromPc => self.show_notice("Not in this app yet"),
            Cap::Shift | Cap::Symbols => {}
        }
        if before.shift != self.kb.shift || before.symbols != self.kb.symbols {
            self.dirty(keyboard::PANEL);
        }
        self.dirty(r);
        match typing {
            // A search filters as it is typed; the quick-add preview follows.
            Typing::Line => self.app_changed(),
            _ => self.dirty(Rect::new(0, TOP, W, keyboard::PANEL.y - TOP)),
        }
    }

    // ---- Drawing shared by the apps -------------------------------------

    /// An app's screen under the title bar.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn app_scene(&self, f: &mut Frame, v: View) {
        self.sub_header(f, upper(Self::app_header(v).as_bytes()).as_bytes());
        if let Some(why) = self.store_says().filter(|_| v != View::Calculator) {
            self.empty_panel(f, TOP + 8, why, "");
        } else {
            match v {
                View::Agenda | View::Day | View::Month => self.calendar_scene(f, v),
                View::NewEvent => self.event_new_scene(f),
                View::EventEdit | View::ContactEdit | View::TaskEdit => self.form_scene(f, v),
                View::Contacts => self.contacts_scene(f),
                View::Card => self.card_scene(f),
                View::Tasks => self.tasks_scene(f),
                View::Notes => self.notes_scene(f),
                View::Note => self.note_scene(f),
                View::Calculator => self.calculator_scene(f),
            }
        }
        f.clip(None);
        if let Some(t) = self.apps.typing {
            let (pressed, focus) = (
                match self.pressed {
                    Some(Target::Cap(i)) => Some(i),
                    _ => None,
                },
                match self.focus {
                    Some(Target::Cap(i)) => Some(i),
                    _ => None,
                },
            );
            self.kb.draw_named(
                f,
                (self.fonts.ui, self.fonts.label),
                pressed,
                focus,
                FOCUS,
                if t == Typing::Note {
                    b"RETURN"
                } else {
                    b"DONE"
                },
            );
        }
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    const fn app_header(v: View) -> &'static str {
        match v {
            View::Agenda | View::Day | View::Month => "Calendar",
            View::NewEvent => "New appointment",
            View::EventEdit => "Appointment",
            View::Contacts => "Contacts",
            View::Card => "Contact",
            View::ContactEdit => "Contact",
            View::Tasks => "Tasks",
            View::TaskEdit => "Task",
            View::Notes => "Notes",
            View::Note => "Note",
            View::Calculator => "Calculator",
        }
    }

    /// A panel saying what an empty list is for.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn empty_panel(&self, f: &mut Frame, y: u16, what: &str, how: &str) {
        let r = Rect::new(MARGIN, y, WIDE, if how.is_empty() { 44 } else { 64 });
        panel::draw(f, r, self.light());
        f.text(r.x + 12, r.y + 28, self.fonts.ui, TEXT, what.as_bytes());
        if !how.is_empty() {
            f.text(r.x + 12, r.y + 52, self.fonts.ui, DIM, how.as_bytes());
        }
    }

    /// A row's glass (pressed when its target is under the pen).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn row_glass(&self, f: &mut Frame, r: Rect, act: Act) {
        if self.app_pressed(act) {
            panel::draw_pressed(f, r, self.light());
        } else {
            panel::draw(f, r, self.light());
        }
    }

    /// Text cut to `w` px with ".." when it doesn't fit.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn cut(&self, font: &'static Font, text: &[u8], w: u16) -> (usize, bool) {
        if font.width(text) <= w {
            return (text.len(), false);
        }
        let dots = font.width(b"..");
        let mut n = text.len();
        while n > 0 && font.width(&text[..n]) + dots > w {
            n -= 1;
        }
        (n, true)
    }

    /// `text` at (`x`, `baseline`), cut to `w`.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn text_cut(
        &self,
        f: &mut Frame,
        (x, baseline, w): (u16, u16, u16),
        font: &'static Font,
        color: Argb,
        text: &[u8],
    ) {
        let (n, cut) = self.cut(font, text, w);
        f.text(x, baseline, font, color, &text[..n]);
        if cut {
            f.text(x + font.width(&text[..n]), baseline, font, color, b"..");
        }
    }

    /// `text` ending at `right`.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn text_right(
        &self,
        f: &mut Frame,
        right: u16,
        baseline: u16,
        font: &'static Font,
        color: Argb,
        text: &[u8],
    ) {
        f.text(
            right.saturating_sub(font.width(text)),
            baseline,
            font,
            color,
            text,
        );
    }

    /// The field under the header (search, quick add): its text or its
    /// hint, the caret while it is typed into.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn line_field(&self, f: &mut Frame, hint: &str) {
        let typing = self.apps.typing == Some(Typing::Line);
        self.row_glass(f, FIELD, Act::Field(LINE_SLOT as u8));
        let text = self.apps.line();
        let ui = self.fonts.ui;
        if text.is_empty() && !typing {
            f.text(FIELD.x + 12, FIELD.y + 27, ui, DIM, hint.as_bytes());
        } else {
            self.text_cut(
                f,
                (FIELD.x + 12, FIELD.y + 27, FIELD.w - 30),
                ui,
                TEXT,
                text,
            );
        }
        if typing {
            let x = (FIELD.x + 12 + ui.width(text)).min(FIELD.x + FIELD.w - 14);
            f.push(Cmd::Fill {
                r: Rect::new(x + 1, FIELD.y + 9, 2, 22),
                color: FOCUS,
            });
            f.push(Cmd::Outline {
                r: FIELD,
                color: FOCUS,
            });
        }
        self.app_marks(f, Act::Field(LINE_SLOT as u8));
    }

    /// A chip: a small box with a word, filled when chosen.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn chip(&self, f: &mut Frame, r: Rect, word: &[u8], chosen: bool, act: Act) {
        if chosen {
            f.push(Cmd::Fill { r, color: FOCUS });
        } else if self.app_pressed(act) {
            f.push(Cmd::Fill {
                r,
                color: super::PRESSED_UNDER,
            });
        }
        f.push(Cmd::Outline {
            r,
            color: super::GLASS_EDGE,
        });
        keyboard::centre_in(
            f,
            r,
            self.fonts.label,
            if chosen { keyboard::DONE_TEXT } else { TEXT },
            word,
        );
        self.app_marks(f, act);
    }

    // ---- Reminders: the notification centre's first piece -----------------

    /// The next reminder's minute, worked out when the records change.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn reminders_plan(&mut self) {
        let now = self.minute_now();
        let after = self
            .apps
            .checked
            .unwrap_or(now.saturating_sub(date::DAY_MIN));
        let mut next: Option<u32> = None;
        if let Some(s) = &self.apps.store {
            for r in s.all(Kind::Appointment) {
                if let Some(at) = calendar::remind_at(&r)
                    && at > after
                    && next.is_none_or(|n| at < n)
                {
                    next = Some(at);
                }
            }
        }
        self.apps.next_due = next;
    }

    /// The clock's minute changed: reminders now due join the banner.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn reminders_tick(&mut self) {
        let now = self.minute_now();
        if self.apps.store.as_ref().is_none_or(|s| !s.loaded()) {
            return;
        }
        // The next one due is worked out again only when the records
        // changed (P1): a minute's tick is one comparison.
        let changes = self.apps.store.as_ref().map(Store::changes);
        if self.apps.planned != changes {
            self.apps.planned = changes;
            self.reminders_plan();
        }
        let after = self
            .apps
            .checked
            .unwrap_or(now.saturating_sub(date::DAY_MIN));
        if self.apps.next_due.is_none_or(|n| n > now) {
            self.apps.checked = Some(now.max(after));
            return;
        }
        let mut fresh = [0u32; DUE_MAX];
        let mut n = 0;
        if let Some(s) = &self.apps.store {
            for r in s.all(Kind::Appointment) {
                let Some(at) = calendar::remind_at(&r) else {
                    continue;
                };
                // Due since the last look, and not already over.
                if at > after && at <= now && calendar::end_of(&r) > now && n < DUE_MAX {
                    fresh[n] = r.id;
                    n += 1;
                }
            }
        }
        for &id in &fresh[..n] {
            if self.apps.n_due < DUE_MAX && !self.apps.due[..self.apps.n_due].contains(&id) {
                self.apps.due[self.apps.n_due] = id;
                self.apps.n_due += 1;
            }
        }
        self.apps.checked = Some(now);
        self.reminders_plan();
        if n > 0 {
            self.app_refresh();
            self.dirty(Rect::new(0, 0, W, H));
        }
    }

    /// Whether a reminder is showing.
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn reminder_shown(&self) -> bool {
        self.apps.n_due > 0
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn banner_act(&mut self, act: Act) {
        if self.apps.n_due == 0 {
            return;
        }
        let id = self.apps.due[0];
        self.apps.due.copy_within(1.., 0);
        self.apps.n_due -= 1;
        self.set_focus(None);
        if act == Act::OpenDue
            && self
                .apps
                .store
                .as_ref()
                .is_some_and(|s| s.get(id).is_some())
        {
            self.event_edit(id);
        }
        self.app_refresh();
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// The reminder banner over whatever screen is up.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn banner_scene(&self, f: &mut Frame) {
        if self.apps.n_due == 0 {
            return;
        }
        let Some(r) = self
            .apps
            .store
            .as_ref()
            .and_then(|s| s.get(self.apps.due[0]))
        else {
            return;
        };
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        // Solid under the glass: it must read over any screen.
        f.push(Cmd::Fill {
            r: BANNER,
            color: super::APP_TITLE_LEFT,
        });
        panel::draw(f, BANNER, self.light());
        f.push(Cmd::Outline {
            r: BANNER,
            color: FOCUS,
        });
        let when = self.event_when(&r);
        let more = self.apps.n_due - 1;
        let head = if more > 0 {
            Line::new().push(b"REMINDER  +").num(more as i32)
        } else {
            Line::new().push(b"REMINDER")
        };
        f.text(BANNER.x + 12, BANNER.y + 20, label, FOCUS, head.as_bytes());
        f.text(
            BANNER.x + 12 + label.width(head.as_bytes()) + 12,
            BANNER.y + 20,
            label,
            DIM,
            upper(when.as_bytes()).as_bytes(),
        );
        let title = r.text(rp_pim::store::tag::TITLE);
        let title = if title.is_empty() {
            &b"Appointment"[..]
        } else {
            title
        };
        self.text_cut(f, (BANNER.x + 12, BANNER.y + 50, 230), ui, TEXT, title);
        self.chip(f, BANNER_DISMISS, b"DISMISS", false, Act::Dismiss);
        self.chip(f, BANNER_OPEN, b"OPEN", true, Act::OpenDue);
    }
}
