//! The organiser's part of the shell (docs/design/pim-store.md): the one
//! store the app programs share, today's date, and the notification
//! service: the reminder banner over any screen, its sound and snooze, for
//! Calendar and Tasks alike (docs/design/notifications.md).
//!
//! The apps themselves are their own programs since 2026-11-14 (the
//! owner's verdict on batch 1; `apps/*`, rp-app): version 1's views,
//! built into the shell, are gone. A reminder never needs its app loaded.
//!
//! Iterate, don't imitate (rule 0): Windows Mobile kept each app's records
//! in its own database and had each app register its alarms with the
//! kernel's notification database (a second copy of the time, stale after
//! a sync); iOS and Android do the same (`UNUserNotificationCenter`,
//! `AlarmManager`, registered again after a reboot). Ours keeps one store
//! and reads the alerts from it ([`rp_pim::remind`]): nothing registered,
//! nothing to forget, and a record changed by a sync reminds at its new
//! time. The next alert is worked out once per change of the records, so a
//! minute's tick is one comparison, and [`Shell::next_reminder`] is the one
//! minute the device must be awake for.

use rp_draw::{Cmd, Frame, Rect};
use rp_pim::date::{self, DAY_MIN, Date};
use rp_pim::remind;
use rp_pim::store::{Kind, Rec, Store, tag};
use rp_weather::Line;

use super::{Action, DIM, FOCUS, H, Shell, TEXT, W, panel};

/// The memory the organiser is given ([`Shell::give_apps`]): the store's.
pub const MEMORY: usize = rp_pim::store::MEMORY;

/// The snoozes the banner offers, in minutes (calendar-rule0.md: "Snooze
/// 5 / 15 min").
pub const SNOOZE_MIN: [u32; 2] = [5, 15];
/// Reminders waiting at once.
const DUE_MAX: usize = 4;
/// How far ahead the next alert is looked for; with none in it, the look
/// is made again then (a repeating appointment's far occurrences).
const PLAN_AHEAD: u32 = 7 * DAY_MIN;
/// An alert missed by a change of the records (one made with its alert
/// already past) still comes up within this many minutes.
const LATE_MIN: u32 = 60;

/// The banner's look (the owner's pick among the mock-ups; until then all
/// three are drawn by this code: docs/design/notifications.md).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BannerStyle {
    /// A. A card under the title bar: what and when over a row of four
    /// keys (5 MIN, 15 MIN, DISMISS, OPEN).
    Card,
    /// B. A compact strip under the title bar: the words left, three keys
    /// and a close mark right.
    Strip,
    /// C. Card A at the bottom, over the softkeys' edge: near the hand,
    /// the app's title left in view.
    Bottom,
}

/// A banner's card and its four keys (5 MIN, 15 MIN, DISMISS, OPEN).
#[derive(Clone, Copy)]
struct Layout {
    card: Rect,
    keys: [Rect; 4],
}

const CARD_H: u16 = 124;
const fn card_keys(y: u16) -> [Rect; 4] {
    let ky = y + CARD_H - 8 - 34;
    [
        Rect::new(16, ky, 106, 34),
        Rect::new(16 + 114, ky, 106, 34),
        Rect::new(16 + 2 * 114, ky, 106, 34),
        Rect::new(16 + 3 * 114, ky, 106, 34),
    ]
}
const LAYOUT_CARD: Layout = Layout {
    card: Rect::new(8, 40, W - 16, CARD_H),
    keys: card_keys(40),
};
const BOTTOM_Y: u16 = rp_app::CONTENT.y + rp_app::CONTENT.h - 8 - CARD_H;
const LAYOUT_BOTTOM: Layout = Layout {
    card: Rect::new(8, BOTTOM_Y, W - 16, CARD_H),
    keys: card_keys(BOTTOM_Y),
};
const LAYOUT_STRIP: Layout = Layout {
    card: Rect::new(8, 40, W - 16, 84),
    keys: [
        Rect::new(W - 16 - 80 - 8 - 66 - 8 - 66, 82, 66, 34),
        Rect::new(W - 16 - 80 - 8 - 66, 82, 66, 34),
        Rect::new(W - 16 - 34, 46, 34, 30),
        Rect::new(W - 16 - 80, 82, 80, 34),
    ],
};

impl BannerStyle {
    const fn layout(self) -> Layout {
        match self {
            Self::Card => LAYOUT_CARD,
            Self::Strip => LAYOUT_STRIP,
            Self::Bottom => LAYOUT_BOTTOM,
        }
    }
}

/// The previews' and tests' names for the banner's keys (the default
/// look's).
pub mod preview {
    use super::LAYOUT_CARD;
    /// The banner's Snooze 5 min.
    pub const REMINDER_SNOOZE: rp_draw::Rect = LAYOUT_CARD.keys[0];
    /// The banner's Snooze 15 min.
    pub const REMINDER_SNOOZE_15: rp_draw::Rect = LAYOUT_CARD.keys[1];
    /// The banner's Dismiss.
    pub const REMINDER_DISMISS: rp_draw::Rect = LAYOUT_CARD.keys[2];
    /// The banner's Open.
    pub const REMINDER_OPEN: rp_draw::Rect = LAYOUT_CARD.keys[3];
    /// The banner's card.
    pub const REMINDER_CARD: rp_draw::Rect = LAYOUT_CARD.card;
}

/// An alert waiting on the banner: the record, when it sounded and the
/// start of the occurrence it is for.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Due {
    id: u32,
    at: u32,
    start: u32,
}

const NO_DUE: Due = Due {
    id: 0,
    at: 0,
    start: 0,
};

/// The organiser's state (small: the records are in the memory given).
pub(crate) struct Apps {
    /// The store, lent to the open program through its context
    /// ([`rp_app::Ctx::pim`]) while it draws and answers events.
    pub(crate) store: core::cell::RefCell<Option<Store<'static>>>,
    /// The year, when the caller has said ([`Shell::set_year`]).
    year: Option<u16>,
    /// Alerts waiting (one per record: a later alert of the same record
    /// takes its place), the minute last checked, the next one due, and
    /// the store's change count it was worked out at.
    due: [Due; DUE_MAX],
    pub(crate) n_due: u8,
    checked: Option<u32>,
    next_due: Option<u32>,
    planned: Option<u32>,
    /// The last alerts answered (record, minute): a change of the records
    /// does not bring them back.
    gone: [(u32, u32); 4],
    /// Snoozed alerts and the minute each comes back.
    snoozed: [(Due, u32); DUE_MAX],
    n_snoozed: u8,
    /// An alert has just come up (or come back from a snooze): the image
    /// sounds it once ([`Shell::take_chime`]).
    chime: bool,
    /// The banner's key under the pen (0-3), and whether the pen went
    /// down on the banner (its lift is the banner's, wherever it is).
    press: Option<u8>,
    down_on_banner: bool,
    style: BannerStyle,
}

impl Apps {
    pub(crate) const fn new() -> Self {
        Self {
            store: core::cell::RefCell::new(None),
            year: None,
            due: [NO_DUE; DUE_MAX],
            n_due: 0,
            checked: None,
            next_due: None,
            planned: None,
            gone: [(0, 0); 4],
            snoozed: [(NO_DUE, 0); DUE_MAX],
            n_snoozed: 0,
            chime: false,
            press: None,
            down_on_banner: false,
            style: BannerStyle::Card,
        }
    }

    /// `d` onto the banner: in place of an earlier alert of its record (a
    /// snoozed one of it dropped), else at the end if there is room.
    /// Whether it is new to the banner.
    fn show(&mut self, d: Due) -> bool {
        let n = usize::from(self.n_snoozed);
        if let Some(k) = self.snoozed[..n].iter().position(|s| s.0.id == d.id) {
            self.snoozed[k] = self.snoozed[n - 1];
            self.n_snoozed -= 1;
        }
        let n = usize::from(self.n_due);
        if let Some(k) = self.due[..n].iter().position(|x| x.id == d.id) {
            let fresh = self.due[k].at < d.at;
            if fresh {
                self.due[k] = d;
            }
            return fresh;
        }
        if n < DUE_MAX {
            self.due[n] = d;
            self.n_due += 1;
            return true;
        }
        false
    }
}

/// A record's colour: its calendar's or its list's (the kit's identity
/// colours), or the tint.
fn colour_of(s: &Store<'_>, r: &Rec<'_>) -> rp_draw::Argb {
    let owner = if r.kind == Kind::Task {
        r.num(tag::LIST)
    } else {
        r.num(tag::CAL)
    };
    owner
        .and_then(|id| s.get(id))
        .and_then(|o| o.num(tag::COLOUR))
        .and_then(|c| rp_app::kit::IDENTITY.get(c as usize).copied())
        .unwrap_or(FOCUS)
}

/// `text` cut to fit `w` pixels in `font`, on a character's boundary.
fn fit<'a>(font: &rp_draw::Font, text: &'a [u8], w: u16) -> &'a [u8] {
    let mut n = text.len();
    while n > 0 && font.width(&text[..n]) > w {
        n -= 1;
        while n > 0 && text[n] & 0xC0 == 0x80 {
            n -= 1;
        }
    }
    &text[..n]
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
        let mut next = after.saturating_add(PLAN_AHEAD);
        if let Some(s) = self.apps.store.borrow().as_ref() {
            for r in s.all(Kind::Appointment).chain(s.all(Kind::Task)) {
                if let Some(at) = remind::next(&r, after, next) {
                    next = next.min(at);
                }
            }
        }
        // With none within the week, looked for again then.
        self.apps.next_due = Some(next);
    }

    /// The clock's minute changed: alerts now due join the banner.
    pub(crate) fn reminders_tick(&mut self) {
        let now = self.minute_now();
        // Snoozed ones back when their time comes.
        let mut k = 0;
        while k < usize::from(self.apps.n_snoozed) {
            let (d, until) = self.apps.snoozed[k];
            if until <= now {
                let last = usize::from(self.apps.n_snoozed) - 1;
                self.apps.snoozed[k] = self.apps.snoozed[last];
                self.apps.n_snoozed -= 1;
                if self.apps.show(d) {
                    self.apps.chime = true;
                    self.dirty(Rect::new(0, 0, W, H));
                }
            } else {
                k += 1;
            }
        }
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
        // The records changed: one made or moved so that its alert's time
        // is already past (an appointment ten minutes off with its alert a
        // quarter of an hour before; the owner on the X51v, 2026-11-15,
        // "it never went off when it was supposed to, no alarm sounded")
        // reminds now, if that time was within the hour, as a phone shows
        // an alert it was late for.
        let changed = self.apps.planned != changes;
        if changed {
            self.apps.planned = changes;
            self.reminders_prune();
            self.reminders_plan();
        }
        let after = self.apps.checked.unwrap_or(now.saturating_sub(DAY_MIN));
        if !changed && self.apps.next_due.is_none_or(|n| n > now) {
            self.apps.checked = Some(now.max(after));
            return;
        }
        let from = if changed {
            after.min(now.saturating_sub(LATE_MIN))
        } else {
            after
        };
        let mut fresh = [NO_DUE; DUE_MAX];
        let mut n = 0;
        if let Some(s) = self.apps.store.borrow().as_ref() {
            let gone = &self.apps.gone;
            for r in s.all(Kind::Appointment).chain(s.all(Kind::Task)) {
                remind::alerts(&r, from, now, &mut |a| {
                    // Due since the last look (or missed by a change and
                    // not yet answered), and not already over.
                    if (a.at > after || !gone.contains(&(a.id, a.at))) && a.end > now && n < DUE_MAX
                    {
                        fresh[n] = Due {
                            id: a.id,
                            at: a.at,
                            start: a.start,
                        };
                        n += 1;
                    }
                });
            }
        }
        let mut new = false;
        for d in &fresh[..n] {
            new |= self.apps.show(*d);
        }
        self.apps.checked = Some(now);
        self.reminders_plan();
        if new {
            self.apps.chime = true;
            self.dirty(Rect::new(0, 0, W, H));
        }
    }

    /// The records changed: an alert waiting or snoozed whose record is
    /// gone, done (a task ticked while its banner showed), moved or no
    /// longer has that alert leaves the banner at once; a card for nothing
    /// is never left catching touches.
    fn reminders_prune(&mut self) {
        let store = self.apps.store.borrow();
        let Some(s) = store.as_ref() else { return };
        let still = |d: &Due| {
            s.get(d.id).is_some_and(|r| {
                let mut ok = false;
                remind::alerts(&r, d.at.saturating_sub(1), d.at, &mut |a| {
                    ok |= a.start == d.start;
                });
                ok
            })
        };
        let had = self.apps.n_due;
        let mut n = 0;
        for k in 0..usize::from(self.apps.n_due) {
            if still(&self.apps.due[k]) {
                self.apps.due[n] = self.apps.due[k];
                n += 1;
            }
        }
        self.apps.n_due = n as u8;
        let mut m = 0;
        for k in 0..usize::from(self.apps.n_snoozed) {
            if still(&self.apps.snoozed[k].0) {
                self.apps.snoozed[m] = self.apps.snoozed[k];
                m += 1;
            }
        }
        self.apps.n_snoozed = m as u8;
        drop(store);
        if self.apps.n_due != had {
            self.apps.press = None;
            self.dirty(Rect::new(0, 0, W, H));
        }
    }

    /// An app program may have changed the records: the banner and the
    /// next alert follow at once, not at the next minute (one comparison
    /// when nothing changed).
    pub(crate) fn reminders_follow(&mut self) {
        let changes = self.apps.store.borrow().as_ref().map(Store::changes);
        if changes != self.apps.planned {
            self.reminders_tick();
        }
    }

    /// The next minute (local, from 1970) a reminder is due, a snoozed one
    /// comes back or the alerts are looked for again: the one time the
    /// device must be awake for (the clock's alarm when the OS sleeps:
    /// docs/design/notifications.md); `None` before the store is loaded.
    #[must_use]
    pub fn next_reminder(&self) -> Option<u32> {
        let snoozed = self.apps.snoozed[..usize::from(self.apps.n_snoozed)]
            .iter()
            .map(|s| s.1)
            .min();
        match (self.apps.next_due, snoozed) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Whether a reminder has come up since this was last asked: the
    /// image plays its sound once for it (the owner, 2026-11-15: "we need
    /// an audible alarm for when the task is set to be done, a
    /// notification system"; the banner was silent).
    pub fn take_chime(&mut self) -> bool {
        core::mem::take(&mut self.apps.chime)
    }

    /// Whether a reminder is showing.
    #[must_use]
    pub fn reminder_shown(&self) -> bool {
        self.apps.n_due > 0
    }

    /// The banner's look (the previews show each; the owner picks).
    pub fn set_banner_style(&mut self, style: BannerStyle) {
        self.apps.style = style;
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// A touch while the banner shows, before anything under it: a touch
    /// that goes down on the banner is the banner's to its lift (a key
    /// shown pressed under the pen; 5 MIN and 15 MIN snooze, DISMISS takes
    /// it away, OPEN takes it away and opens its app). `None` when the
    /// touch isn't the banner's.
    pub(crate) fn banner_input(&mut self, e: super::Input) -> Option<Action> {
        use super::Input;
        let lay = self.apps.style.layout();
        let key = |x: u16, y: u16| lay.keys.iter().position(|k| k.contains_point(x, y));
        match e {
            Input::PenDown { x, y } => {
                if !self.reminder_shown() || !lay.card.contains_point(x, y) {
                    self.apps.down_on_banner = false;
                    return None;
                }
                self.apps.down_on_banner = true;
                self.apps.press = key(x, y).map(|k| k as u8);
                self.dirty(lay.card);
                Some(Action::Redraw)
            }
            Input::PenMove { x, y } if self.apps.down_on_banner => {
                let p = key(x, y).map(|k| k as u8);
                if p != self.apps.press {
                    self.apps.press = p;
                    self.dirty(lay.card);
                    return Some(Action::Redraw);
                }
                Some(Action::Nothing)
            }
            Input::PenUp { x, y } if self.apps.down_on_banner => {
                self.apps.down_on_banner = false;
                let pressed = self.apps.press.take();
                self.dirty(lay.card);
                match key(x, y) {
                    Some(k) if pressed == Some(k as u8) && self.reminder_shown() => {
                        Some(self.banner_key(k))
                    }
                    _ => Some(Action::Redraw),
                }
            }
            _ => None,
        }
    }

    /// The banner's key `k` (0 5 MIN, 1 15 MIN, 2 DISMISS, 3 OPEN) on its
    /// first alert.
    fn banner_key(&mut self, k: usize) -> Action {
        let d = self.apps.due[0];
        // Open goes to its app: Calendar, or Tasks for a task.
        let task = self
            .apps
            .store
            .borrow()
            .as_ref()
            .and_then(|s| s.get(d.id))
            .is_some_and(|r| r.kind == Kind::Task);
        self.apps.due.copy_within(1.., 0);
        self.apps.n_due -= 1;
        self.apps.gone.copy_within(..3, 1);
        self.apps.gone[0] = (d.id, d.at);
        if let Some(&m) = SNOOZE_MIN.get(k)
            && usize::from(self.apps.n_snoozed) < DUE_MAX
        {
            self.apps.snoozed[usize::from(self.apps.n_snoozed)] = (d, self.minute_now() + m);
            self.apps.n_snoozed += 1;
        }
        self.dirty(Rect::new(0, 0, W, H));
        if k == 3 && self.programs {
            Action::OpenProgram(if task { 2 } else { 0 })
        } else {
            Action::Redraw
        }
    }

    /// The reminder banner over whatever screen is up.
    pub(crate) fn banner_scene(&self, f: &mut Frame) {
        if self.apps.n_due == 0 {
            return;
        }
        let d = self.apps.due[0];
        let store = self.apps.store.borrow();
        let Some(s) = store.as_ref() else { return };
        let Some(r) = s.get(d.id) else { return };
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let style = self.apps.style;
        let lay = style.layout();
        let card = lay.card;
        // Solid under the glass: it must read over any screen.
        f.push(Cmd::Fill {
            r: card,
            color: super::APP_TITLE_LEFT,
        });
        panel::draw(f, card, self.light());
        // The record's colour as a bar down its words (colour never alone:
        // the app's name says it too).
        f.push(Cmd::Fill {
            r: Rect::new(card.x + 12, card.y + 10, 5, 66),
            color: colour_of(s, &r),
        });
        let task = r.kind == Kind::Task;
        let now = self.minute_now();
        let x = card.x + 28;
        // The head: the app, when it starts in words, how many wait.
        let mut head = Line::new().push(if task { b"TASKS  " } else { b"CALENDAR  " });
        head = if d.start > now {
            let m = d.start - now;
            if m < 60 {
                head.push(b"IN ").num(m as i32).push(b" MIN")
            } else if m < DAY_MIN {
                head.push(b"IN ").num(m.div_ceil(60) as i32).push(b" H")
            } else {
                head.push(b"IN ")
                    .num(m.div_ceil(DAY_MIN) as i32)
                    .push(b" DAYS")
            }
        } else if task {
            head.push(b"DUE NOW")
        } else {
            head.push(b"NOW")
        };
        let more = usize::from(self.apps.n_due) - 1;
        if more > 0 {
            head = head.push(b"  +").num(more as i32);
        }
        // The words' right edge: before the keys beside them (B) or the
        // card's (A, C).
        let (right, head_right) = match style {
            BannerStyle::Strip => (lay.keys[0].x - 8, lay.keys[2].x - 8),
            _ => (card.x + card.w - 12, card.x + card.w - 12),
        };
        f.text(
            x,
            card.y + 22,
            label,
            FOCUS,
            fit(label, head.as_bytes(), head_right - x),
        );
        let title = r.text(tag::TITLE);
        let title = if title.is_empty() && task {
            &b"Task"[..]
        } else if title.is_empty() {
            &b"Appointment"[..]
        } else {
            title
        };
        f.text(x, card.y + 48, ui, TEXT, fit(ui, title, right - x));
        // When (its day if not today) and where.
        let m = d.start % DAY_MIN;
        let mut when = Line::new();
        let day = date::day_of(d.start);
        if day != self.today() {
            let wd = date::WEEKDAYS[usize::from(date::weekday(day)) % 7].as_bytes();
            when = when.push(wd).push(b" ");
        }
        if !task && r.num(tag::ALL_DAY) == Some(1) {
            when = when.push(b"All day");
        } else {
            when = when.push(
                self.region
                    .time((m / 60) as u8, (m % 60) as u8, true)
                    .as_bytes(),
            );
        }
        let place = if task { &b""[..] } else { r.text(tag::PLACE) };
        if !place.is_empty() {
            when = when.push(b"   ").push(place);
        }
        f.text(
            x,
            card.y + 70,
            label,
            DIM,
            fit(label, when.as_bytes(), right - x),
        );
        // The keys: the apps' Aero keys, OPEN the blue glow.
        let look = self.program_look();
        let names: [&[u8]; 4] = match style {
            BannerStyle::Strip => [b"5 MIN", b"15 MIN", b"X", b"OPEN"],
            _ => [b"5 MIN", b"15 MIN", b"DISMISS", b"OPEN"],
        };
        for (k, (rr, word)) in lay.keys.iter().zip(names).enumerate() {
            let down = self.apps.press == Some(k as u8);
            let art = if k == 3 {
                look.on
            } else if down {
                look.key_down
            } else {
                look.key
            };
            f.push(Cmd::Panel { r: *rr, art });
            if k == 3 && down {
                f.push(Cmd::Outline {
                    r: *rr,
                    color: TEXT,
                });
            }
            let w = label.width(word);
            f.text(
                rr.x + rr.w.saturating_sub(w) / 2,
                rr.y + rr.h / 2 + 5,
                label,
                if k == 3 {
                    super::keyboard::DONE_TEXT
                } else {
                    TEXT
                },
                word,
            );
        }
    }
}

#[cfg(test)]
impl Shell {
    /// The first waiting alert's occurrence start (the tests').
    pub(crate) fn apps_due_start(&self) -> Option<u32> {
        (self.apps.n_due > 0).then_some(self.apps.due[0].start)
    }

    /// The banner look's card and keys (the tests').
    pub(crate) fn banner_layout(&self) -> (Rect, [Rect; 4]) {
        let l = self.apps.style.layout();
        (l.card, l.keys)
    }
}
