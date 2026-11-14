//! The organiser's part of the shell (docs/design/pim-store.md): the one
//! store the app programs share, today's date, and the reminder banner over
//! any screen.
//!
//! The apps themselves are their own programs since 2026-11-14 (the
//! owner's verdict on batch 1; `apps/*`, rp-app): version 1's views,
//! built into the shell, are gone.
//!
//! Iterate, don't imitate (rule 0): Windows Mobile kept each app's records
//! in its own database and its reminders in each app's process; ours keeps
//! one store and one reminder banner in the shell, so every app program
//! reads the same records and a reminder shows over whatever is up.

use rp_draw::{Cmd, Frame, Rect};
use rp_pim::date::{self, DAY_MIN, Date};
use rp_pim::store::{Kind, Rec, Store, tag};
use rp_weather::Line;

use super::{Action, DIM, FOCUS, H, Shell, TEXT, W, panel};

/// The memory the organiser is given ([`Shell::give_apps`]): the store's.
pub const MEMORY: usize = rp_pim::store::MEMORY;

/// The reminder banner (under the title bar, on any screen).
const BANNER: Rect = Rect::new(8, 40, W - 16, 64);
const BANNER_DISMISS: Rect = Rect::new(W - 8 - 200, 68, 96, 32);
const BANNER_OPEN: Rect = Rect::new(W - 8 - 100, 68, 92, 32);
/// Reminders waiting at once.
const DUE_MAX: usize = 4;

/// The previews' and tests' names for the banner's buttons.
pub mod preview {
    /// The banner's Dismiss.
    pub const REMINDER_DISMISS: rp_draw::Rect = super::BANNER_DISMISS;
    /// The banner's Open.
    pub const REMINDER_OPEN: rp_draw::Rect = super::BANNER_OPEN;
}

/// The organiser's state (small: the records are in the memory given).
pub(crate) struct Apps {
    /// The store, lent to the open program through its context
    /// ([`rp_app::Ctx::pim`]) while it draws and answers events.
    pub(crate) store: core::cell::RefCell<Option<Store<'static>>>,
    /// The year, when the caller has said ([`Shell::set_year`]).
    year: Option<u16>,
    /// Reminders waiting (appointment ids), the minute last checked, the
    /// next one due, and the store's change count it was worked out at.
    due: [u32; DUE_MAX],
    n_due: usize,
    checked: Option<u32>,
    next_due: Option<u32>,
    planned: Option<u32>,
}

impl Apps {
    pub(crate) const fn new() -> Self {
        Self {
            store: core::cell::RefCell::new(None),
            year: None,
            due: [0; DUE_MAX],
            n_due: 0,
            checked: None,
            next_due: None,
            planned: None,
        }
    }
}

/// An appointment's start (local minutes).
fn start_of(r: &Rec<'_>) -> u32 {
    r.num(tag::START).unwrap_or(0)
}

fn all_day(r: &Rec<'_>) -> bool {
    r.num(tag::ALL_DAY) == Some(1)
}

/// When an appointment is over.
fn end_of(r: &Rec<'_>) -> u32 {
    let len = if all_day(r) {
        DAY_MIN
    } else {
        r.num(tag::MINUTES).unwrap_or(60)
    };
    start_of(r).saturating_add(len)
}

/// When its reminder is due, if it has one.
fn remind_at(r: &Rec<'_>) -> Option<u32> {
    Some(start_of(r).saturating_sub(r.num(tag::REMIND)?))
}

impl Shell {
    /// Gives the organiser its memory ([`MEMORY`] bytes): the store starts
    /// unloaded, and the caller loads it ([`Shell::pim`]).
    pub fn give_apps(&mut self, memory: &'static mut [u8]) {
        if memory.len() < MEMORY {
            return;
        }
        *self.apps.store.get_mut() = Some(Store::new(memory));
    }

    /// The organiser's store, for the caller to load and write
    /// (`rp_pim::store`: `load_want`, `next_write`).
    pub fn pim(&mut self) -> Option<&mut Store<'static>> {
        self.apps.store.get_mut().as_mut()
    }

    /// The store finished loading (or its records changed outside the
    /// shell): reminders are looked for.
    pub fn pim_loaded(&mut self) {
        self.apps.planned = None;
        self.reminders_tick();
    }

    /// The year (the shell's [`super::Time`] has none). Without it the year
    /// is the first from 2024 on whose date falls on the clock's weekday.
    pub fn set_year(&mut self, year: u16) {
        self.apps.year = Some(year);
    }

    /// Today, as a day count (days since 1970): what a program is opened
    /// on.
    #[must_use]
    pub fn today_days(&self) -> i32 {
        self.today()
    }

    /// Today, as a day count.
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
    pub(crate) fn minute_now(&self) -> u32 {
        date::start_of(self.today()) + u32::from(self.time.hour) * 60 + u32::from(self.time.minute)
    }

    fn reminders_plan(&mut self) {
        let now = self.minute_now();
        let after = self.apps.checked.unwrap_or(now.saturating_sub(DAY_MIN));
        let mut next: Option<u32> = None;
        if let Some(s) = self.apps.store.borrow().as_ref() {
            for r in s.all(Kind::Appointment) {
                if let Some(at) = remind_at(&r)
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
    pub(crate) fn reminders_tick(&mut self) {
        let now = self.minute_now();
        if self
            .apps
            .store
            .borrow()
            .as_ref()
            .is_none_or(|s| !s.loaded())
        {
            return;
        }
        // The next one due is worked out again only when the records
        // changed (P1): a minute's tick is one comparison.
        let changes = self.apps.store.borrow().as_ref().map(Store::changes);
        if self.apps.planned != changes {
            self.apps.planned = changes;
            self.reminders_plan();
        }
        let after = self.apps.checked.unwrap_or(now.saturating_sub(DAY_MIN));
        if self.apps.next_due.is_none_or(|n| n > now) {
            self.apps.checked = Some(now.max(after));
            return;
        }
        let mut fresh = [0u32; DUE_MAX];
        let mut n = 0;
        if let Some(s) = self.apps.store.borrow().as_ref() {
            for r in s.all(Kind::Appointment) {
                let Some(at) = remind_at(&r) else { continue };
                // Due since the last look, and not already over.
                if at > after && at <= now && end_of(&r) > now && n < DUE_MAX {
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
            self.dirty(Rect::new(0, 0, W, H));
        }
    }

    /// Whether a reminder is showing.
    #[must_use]
    pub fn reminder_shown(&self) -> bool {
        self.apps.n_due > 0
    }

    /// A tap at x, y while the banner shows: Dismiss takes it away, Open
    /// takes it away and opens Calendar (program 0); `None` when the tap
    /// isn't the banner's.
    pub(crate) fn banner_tap(&mut self, x: u16, y: u16) -> Option<Action> {
        if self.apps.n_due == 0 || !BANNER.contains_point(x, y) {
            return None;
        }
        let open = BANNER_OPEN.contains_point(x, y);
        if !open && !BANNER_DISMISS.contains_point(x, y) {
            return Some(Action::Nothing);
        }
        self.apps.due.copy_within(1.., 0);
        self.apps.n_due -= 1;
        self.dirty(Rect::new(0, 0, W, H));
        Some(if open && self.programs {
            Action::OpenProgram(0)
        } else {
            Action::Redraw
        })
    }

    /// The reminder banner over whatever screen is up.
    pub(crate) fn banner_scene(&self, f: &mut Frame) {
        if self.apps.n_due == 0 {
            return;
        }
        let store = self.apps.store.borrow();
        let Some(r) = store.as_ref().and_then(|s| s.get(self.apps.due[0])) else {
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
        let more = self.apps.n_due - 1;
        let head = if more > 0 {
            Line::new().push(b"REMINDER  +").num(more as i32)
        } else {
            Line::new().push(b"REMINDER")
        };
        f.text(BANNER.x + 12, BANNER.y + 20, label, FOCUS, head.as_bytes());
        let m = start_of(&r) % DAY_MIN;
        let when = if all_day(&r) {
            Line::new().push(b"ALL DAY")
        } else {
            self.region.time((m / 60) as u8, (m % 60) as u8, true)
        };
        f.text(
            BANNER.x + 12 + label.width(head.as_bytes()) + 12,
            BANNER.y + 20,
            label,
            DIM,
            when.as_bytes(),
        );
        let title = r.text(tag::TITLE);
        let title = if title.is_empty() {
            &b"Appointment"[..]
        } else {
            title
        };
        f.text(
            BANNER.x + 12,
            BANNER.y + 50,
            ui,
            TEXT,
            &title[..title.len().min(24)],
        );
        for (rr, word, chosen) in [
            (BANNER_DISMISS, &b"DISMISS"[..], false),
            (BANNER_OPEN, b"OPEN", true),
        ] {
            f.push(Cmd::Fill {
                r: rr,
                color: if chosen { FOCUS } else { super::APP_TITLE_LEFT },
            });
            let w = label.width(word);
            f.text(
                rr.x + rr.w.saturating_sub(w) / 2,
                rr.y + 21,
                label,
                if chosen {
                    super::keyboard::DONE_TEXT
                } else {
                    TEXT
                },
                word,
            );
        }
    }
}
