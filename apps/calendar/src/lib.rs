//! Calendar, version 2 ([calendar-rule0.md](../../../docs/design/calendar-rule0.md)):
//! an app program ([`rp_app::Program`]).
//!
//! Iterate, don't imitate (rule 0): Windows Mobile 5 opened on an empty
//! hour grid and added through a ten-row form; Google's Schedule and
//! Fantastical answer "what's next" at a glance and add from one sentence.
//! Ours opens on the agenda under a large title and a week strip, puts the
//! forecast the device already holds on each day, sizes Day's blocks to
//! their length in their calendar's colour, zooms between month and day,
//! and highlights the words the quick-add line understood as they are
//! typed, all offline.

#![no_std]
#![forbid(unsafe_code)]

use rp_app::kit::{self, DIM, TEXT, tint};
use rp_app::{Ctx, Event, Field, Frame, Key, MARGIN, Pic, Program, Rect, Reply, W, WIDE};
use rp_pim::date;
use rp_pim::store::{Kind, tag};

mod files;
pub mod motion;

/// The calendars' screen's Import and Export keys (the previews tap them).
pub const IMPORT_KEY_FOR_PREVIEW: Rect = files::IMPORT_KEY;
/// See [`IMPORT_KEY_FOR_PREVIEW`].
pub const EXPORT_KEY_FOR_PREVIEW: Rect = files::EXPORT_KEY;

/// Appointments a view holds (a repeat's days count each).
pub const MAX: usize = 96;
/// The days a view's list covers, from a week before the chosen day.
const WINDOW: i32 = 49;

/// A short text kept with a list's entry (the store's own bytes can't be
/// held across a change).
#[derive(Clone, Copy, Debug)]
pub struct Text {
    b: [u8; 40],
    n: u8,
}

impl Text {
    /// No text.
    pub const EMPTY: Self = Self { b: [0; 40], n: 0 };

    /// `s`, cut to 40 bytes.
    #[must_use]
    pub fn of(s: &[u8]) -> Self {
        let mut t = Self::EMPTY;
        let n = s.len().min(40);
        t.b[..n].copy_from_slice(&s[..n]);
        t.n = n as u8;
        t
    }

    /// The bytes.
    #[must_use]
    pub fn get(&self) -> &[u8] {
        &self.b[..usize::from(self.n)]
    }
}

/// One calendar: a name and its colour.
#[derive(Clone, Copy, Debug)]
pub struct Cal {
    /// Its name.
    pub name: &'static [u8],
    /// Its identity colour.
    pub colour: u32,
}

/// A calendar as the store holds it: its record, name and colour.
#[derive(Clone, Copy, Debug)]
struct CalRow {
    id: u32,
    name: Text,
    colour: u32,
    hidden: bool,
    /// The Birthdays calendar: its days are the contacts' own
    /// ([`rp_pim::birthdays`]), none of them records of its own.
    contacts: bool,
}

/// One appointment as a view shows it.
#[derive(Clone, Copy, Debug)]
pub struct Appt {
    /// Days since 1970.
    pub day: i32,
    /// Start, minutes from midnight.
    pub start: u16,
    /// Length in minutes (0: all day).
    pub len: u16,
    /// Its title.
    pub title: Text,
    /// Where, if anywhere.
    pub place: Text,
    /// Which calendar (an index in the view's calendars).
    pub cal: u8,
    /// It repeats.
    pub repeats: bool,
    /// A task due that day (Tasks' record), not an appointment.
    pub task: bool,
    /// Its record.
    pub id: u32,
}

/// The views.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    /// The day's board (design-language.md version 3): the date as a
    /// hero, the day as a ribbon, what is next with its countdown, the
    /// week's load.
    Home,
    /// What is coming, day by day.
    Agenda,
    /// One day as blocks.
    Day,
    /// Seven days.
    Week,
    /// A month and the chosen day.
    Month,
    /// The quick-add sheet.
    New,
    /// The calendars, and getting online ones (calendar-rule0.md: an iCal
    /// link first, CalDAV and Google sign-in after).
    Calendars,
    /// An appointment's details as grouped cards.
    Edit,
    /// The search: every appointment kept whose title or place holds
    /// what is typed, whenever it is.
    Find,
    /// The device's time zone chosen (calendar-rule0.md "Time zones").
    Zones,
    /// A calendar file chosen to import, or where to export (".ics import
    /// and export").
    Files,
}

/// The editor's text field being typed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EditText {
    Title,
    Place,
}

/// An appointment being edited (a new one has no record yet).
#[derive(Clone, Copy, Debug)]
struct Edit {
    id: Option<u32>,
    title: [u8; 64],
    title_n: usize,
    place: [u8; 64],
    place_n: usize,
    day: i32,
    start: u16,
    len: u16,
    all_day: bool,
    /// 0 none, then Every's order: day, week, month, year.
    repeat: u8,
    /// The record's own rule and the chip it showed as: kept whole on
    /// save while that chip is chosen (every 2 weeks on Mon and Wed, the
    /// 2nd Tuesday, an end day: the chips say only "WEEKLY", "MONTHLY"),
    /// so an edit of the title never rewrites the repeat (R375).
    kept_rule: Option<(rp_pim::repeat::Rule, u8)>,
    /// The alerts on (several, calendar-rule0.md): bit k for chip k, 1 at
    /// the start, 2 15 min, 3 1 h, 4 1 day before, 5 its own lead
    /// ([`Edit::alert_other`]); none on is chip 0, NONE.
    alerts: u8,
    /// The own lead in minutes (chip 5).
    alert_other: u32,
    /// The D-pad's chip on the alert row (left and right move it, the
    /// centre turns it on or off: a row of several needs a cursor apart
    /// from what is on).
    alert_at: u8,
    /// The record's alerts as they were ([`tag::REMIND`]), kept as they
    /// are unless the alert row is changed (an imported appointment's
    /// leads the chips can't show, two of its own, are never lost by an
    /// edit of its title).
    kept: [u8; 4 * rp_pim::remind::LEADS_MAX],
    kept_n: u8,
    alerts_changed: bool,
    cal: usize,
    /// From a calendar link: shown, not changed.
    read_only: bool,
    typing: Option<EditText>,
    confirm_delete: bool,
    /// "Make a task" was pressed: the task is in Tasks (the key says so
    /// and makes no second one).
    task_made: bool,
    /// The D-pad's row ([`EDIT_ROWS`]): up and down move it, left and
    /// right change its value, the centre types in it or presses it.
    row: Option<u8>,
    /// The zone its time is kept in (empty: the device's own); the editor
    /// shows and sets the time in the device's zone.
    zone: rp_pim::net::Short<40>,
}

const ALERTS: [Option<u32>; 5] = [None, Some(0), Some(15), Some(60), Some(1440)];
const ALERT_NAMES: [&[u8]; 5] = [b"NONE", b"AT START", b"15 MIN", b"1 HOUR", b"1 DAY"];

impl Edit {
    /// The alerts' leads in minutes (none: no alert), latest first.
    fn leads(&self) -> ([u32; 5], usize) {
        let mut v = [0u32; 5];
        let mut n = 0;
        for k in (1..=5u8).rev() {
            if self.alerts & (1 << k) != 0 {
                v[n] = ALERTS
                    .get(usize::from(k))
                    .copied()
                    .flatten()
                    .unwrap_or(self.alert_other);
                n += 1;
            }
        }
        v[..n].sort_unstable_by(|a, b| b.cmp(a));
        (v, n)
    }

    /// The own lead (the sixth chip's pad) set to `lead` minutes before:
    /// a quick one lights its own chip; `None` takes the own one off.
    fn set_alert(&mut self, lead: Option<u32>) {
        self.alerts &= !(1 << 5);
        match ALERTS.iter().position(|a| *a == lead) {
            Some(0) => {}
            Some(k) => self.alerts |= 1 << k,
            None => {
                self.alerts |= 1 << 5;
                self.alert_other = lead.unwrap_or(0);
            }
        }
        self.alerts_changed = true;
    }

    /// Chip `k` of the alert row tapped: NONE takes every alert off, a
    /// quick one turns on or off.
    fn toggle_alert(&mut self, k: usize) {
        match k {
            0 => self.alerts = 0,
            1..=4 => self.alerts ^= 1 << k,
            _ => return,
        }
        self.alerts_changed = true;
    }

    /// The alerts read from a record's leads.
    fn load_alerts(&mut self, r: &rp_pim::store::Rec<'_>) {
        self.alerts = 0;
        for lead in rp_pim::remind::leads(r) {
            match ALERTS.iter().position(|a| *a == Some(lead)) {
                Some(k) => self.alerts |= 1 << k,
                None => {
                    self.alerts |= 1 << 5;
                    self.alert_other = lead;
                }
            }
        }
        let raw = r.text(tag::REMIND);
        let n = raw.len().min(self.kept.len()) / 4 * 4;
        self.kept[..n].copy_from_slice(&raw[..n]);
        self.kept_n = n as u8;
        self.alerts_changed = false;
    }
}
const REPEAT_NAMES: [&[u8]; 5] = [b"NONE", b"DAILY", b"WEEKLY", b"MONTHLY", b"YEARLY"];

/// The editor's rows (y), from under the title band.
const E_TITLE: u16 = 118;
const E_PLACE: u16 = 162;
const E_DAY: u16 = 216;
const E_START: u16 = 260;
const E_LEN: u16 = 304;
const E_REPEAT: u16 = 360;
const E_ALERT: u16 = 412;
const E_CAL: u16 = 464;
const E_DELETE: u16 = 520;
/// The editor's rows for the D-pad: each one's top and height.
const EDIT_ROWS: [(u16, u16); 9] = [
    (E_TITLE, 40),
    (E_PLACE, 40),
    (E_DAY, 40),
    (E_START, 40),
    (E_LEN, 40),
    (E_REPEAT + 14, 30),
    (E_ALERT + 14, 30),
    (E_CAL + 14, 30),
    (E_DELETE, 40),
];

/// The editor's last row: "Make a task" and, for an appointment of this
/// device, Delete beside it (a calendar link's is read only: Make a task
/// alone, the width of the row).
const fn edit_keys(read_only: bool) -> (Rect, Option<Rect>) {
    if read_only {
        (Rect::new(MARGIN, E_DELETE, WIDE, 40), None)
    } else {
        let half = (WIDE - 8) / 2;
        (
            Rect::new(MARGIN, E_DELETE, half, 40),
            Some(Rect::new(MARGIN + half + 8, E_DELETE, half, 40)),
        )
    }
}

/// The key on the title band that opens the calendars.
const CALENDARS_KEY: Rect = Rect::new(W - MARGIN - 128, 46, 128, 30);
/// The search's field.
const FIND_FIELD: Rect = Rect::new(MARGIN, 36 + kit::TITLE_BAND + 4, WIDE, 38);
/// The calendars screen's "Get calendars" key.
const GET_KEY: Rect = Rect::new(MARGIN, 540, WIDE, 44);
/// Events read from one calendar link at a time.
const PARSED: usize = 160;

/// What the calendar last asked the network.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Asked {
    Nothing,
    List,
    Link(u8),
    Place,
}

/// What a stylus tap on a Calendar screen hits (the owner, 2026-11-14:
/// "i should be able to click on everything (agenda, day, week, month)
/// including individual days"; recorded as each screen is drawn).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    /// A view's tab.
    View(View),
    /// A day chosen (the week strip, the month's grid).
    Day(i32),
    /// A day opened in Day (the week's columns).
    OpenDay(i32),
    /// An appointment opened.
    Appt(u32),
    /// The quick-add sheet opened.
    New,
    /// The search opened (Agenda's search field).
    Find,
}

/// The Calendar program.
pub struct Calendar {
    /// Online calendars: the links to read next (their numbers on the PC),
    /// whether to ask for the list, what was asked, and what to say.
    queue: [u8; 8],
    n_queue: usize,
    /// The pad setting the appointment's start (0) or its alert's lead
    /// (1), if it is up.
    pad: Option<(u8, rp_app::timepad::TimePad)>,
    /// The appointment being edited, and the agenda's rows as drawn.
    edit: Option<Edit>,
    /// Where's suggestions (street and city lines), and the text last
    /// asked about.
    places: [(Text, Text); 3],
    n_places: usize,
    place_asked: Text,
    rows_at: [(Rect, u32, bool); 12],
    n_rows: usize,
    /// What a tap hits, as last drawn.
    hits: kit::Hits<Hit, 96>,
    /// The D-pad's target among them (R203).
    focus: kit::Focus,
    /// The search: what is typed, whether the keyboard is up, the rows
    /// on screen with their appointments, the first match shown and how
    /// many there are.
    find: [u8; 32],
    find_n: usize,
    find_typing: bool,
    find_rows: [(Rect, u32); 9],
    n_find: usize,
    find_first: u16,
    find_total: u16,
    /// The calendars' rows as drawn (a tap shows or hides one).
    cal_rows: [(Rect, u32); 8],
    n_cal_rows: usize,
    want_list: bool,
    asked: Asked,
    status: &'static [u8],
    synced: u32,
    cals: [CalRow; 8],
    n_cals: usize,
    appts: [Option<Appt>; MAX],
    /// The store's change count and the window's first day the list was
    /// made at (P1: made when the records change, not each frame).
    seen: Option<(u32, i32)>,
    /// The store's change count the Birthdays calendar was looked for at.
    bdays_seen: Option<u32>,
    view: View,
    /// The chosen day.
    day: i32,
    /// The quick-add line.
    line: [u8; 64],
    line_n: usize,
    /// Pictures painted once: today's ring and the chosen day's disc.
    ring: Option<Pic>,
    chosen: Option<Pic>,
    painted: bool,
    /// The board's countdown ring: its picture, the part filled and the
    /// look it was painted for (painted again when either changes).
    gauge: Option<(Pic, u16, bool)>,
    /// The view the quick-add sheet rose over (and goes back to).
    under: View,
    /// Motion under way (motion.rs): the week strip settling, or a zoom
    /// between month, week and day.
    motion: Option<Motion>,
    /// The week strip's offset while the pen drags it (px; 0 at rest).
    slide: i16,
    /// The pen on the week strip: where it went down, its last x, its
    /// last step, and whether it has moved far enough to drag.
    drag: Option<(u16, u16, i16, bool)>,
    /// Where the week strip was last drawn (a pen landing on it may drag
    /// it).
    strip_at: Option<Rect>,
    /// Today's ring as four 10 px slices, painted the first time the ring
    /// is partly left of the screen in a sliding strip (motion.rs).
    ring_slices: [Option<Pic>; 4],
    /// The files' and zones' screens, and the device's zone.
    io: files::Io,
    /// A status line made at run time (an import's counts): shown when
    /// `status` is the mark `\x01`.
    status_text: Text,
}

/// An iCalendar event's fields for the store (a link's or a file's):
/// its zone and its first alarm kept with it.
struct EventFields<'a> {
    e: &'a rp_pim::ics::Event,
    st: [u8; 4],
    mi: [u8; 4],
    ca: [u8; 4],
    one: [u8; 4],
    rule: Option<[u8; 4]>,
    alarm: Option<[u8; 4]>,
}

impl<'a> EventFields<'a> {
    fn of(e: &'a rp_pim::ics::Event, cal: u32) -> Self {
        let le = |v: u32| v.to_le_bytes();
        Self {
            e,
            st: le(e.start),
            mi: le(e.minutes.max(1)),
            ca: le(cal),
            one: le(1),
            rule: e.rule.map(|r| le(r.pack(date::day_of(e.start)))),
            alarm: e.alarm.map(le),
        }
    }

    fn list(&self) -> [(u8, &[u8]); 11] {
        let e = self.e;
        [
            (tag::TITLE, e.title.get()),
            (tag::PLACE, e.place.get()),
            (tag::NOTE, e.note.get()),
            (tag::START, &self.st),
            (tag::MINUTES, &self.mi),
            (tag::CAL, &self.ca),
            (tag::ALL_DAY, if e.all_day { &self.one } else { &[] }),
            (tag::REPEAT, self.rule.as_ref().map_or(&[][..], |r| &r[..])),
            (tag::ZONE, e.zone.get()),
            (tag::REMIND, self.alarm.as_ref().map_or(&[][..], |r| &r[..])),
            (0, &[]),
        ]
    }
}

/// Motion the user started (design-language.md "Motion": started only by
/// the user's own act, [`motion::FRAMES`] frames, each inside the 33 ms
/// rule).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Motion {
    /// The week strip from `from` px to `to` px (`to` a whole strip: the
    /// week changes when it lands), frame `k`.
    Slide { from: i16, to: i16, k: u8 },
    /// A zoom from view `from` to view `to` (the view is `to` already),
    /// growing out of or shrinking into the shallower view's `cell` (the
    /// day's cell, the week's row, the day's column), frame `k`.
    Zoom {
        from: View,
        to: View,
        cell: Rect,
        k: u8,
    },
}

/// Where the tab views start under the large title (`title` + 6).
const VIEW_TOP: u16 = 36 + kit::TITLE_BAND + 6;
/// The month grid's first row (the month's name 28, the weekdays 20).
const MONTH_GRID: u16 = VIEW_TOP + 48;
/// A month cell's height.
const MONTH_CH: u16 = 46;
/// The week view: the hours' column's right edge, a slim day's width.
const WEEK_X0: u16 = MARGIN + 36;
const WEEK_SLIM: u16 = 40;

/// The views along the foot, and their words.
const TABS: [View; 5] = [View::Home, View::Agenda, View::Day, View::Week, View::Month];
const LABELS: [&[u8]; 5] = [b"Today", b"Agenda", b"Day", b"Week", b"Month"];
/// Where a view's content ends: the foot tabs' top.
const BOTTOM: u16 = kit::FOOT.y - 2;
const WEEKDAYS: [&[u8]; 7] = [b"MON", b"TUE", b"WED", b"THU", b"FRI", b"SAT", b"SUN"];
const WEEKDAY_NAMES: [&[u8]; 7] = [
    b"Monday",
    b"Tuesday",
    b"Wednesday",
    b"Thursday",
    b"Friday",
    b"Saturday",
    b"Sunday",
];
const MONTHS: [&[u8]; 12] = [
    b"January",
    b"February",
    b"March",
    b"April",
    b"May",
    b"June",
    b"July",
    b"August",
    b"September",
    b"October",
    b"November",
    b"December",
];

/// Day `day`'s place in its week (0 the week's first day): Sunday starts
/// the week in the US (the region's custom), Monday elsewhere.
fn in_week(day: i32, us: bool) -> i32 {
    let w = i32::from(date::weekday(day)); // 0 Monday
    if us { (w + 1) % 7 } else { w }
}

/// The weekday's short name for column `i` of a week.
fn column_name(i: u16, us: bool) -> &'static [u8] {
    let k = if us {
        (usize::from(i) + 6) % 7
    } else {
        usize::from(i)
    };
    WEEKDAYS[k]
}

/// A small writer into a fixed buffer (no allocation).
struct Buf<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Buf<N> {
    const fn new() -> Self {
        Self { b: [0; N], n: 0 }
    }
    fn s(&mut self, s: &[u8]) -> &mut Self {
        for &c in s {
            if self.n < N {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        self
    }
    fn num(&mut self, v: u32) -> &mut Self {
        let mut d = [0u8; 10];
        let mut i = d.len();
        let mut v = v;
        loop {
            i -= 1;
            d[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        let tail = d;
        self.s(&tail[i..])
    }
    /// Day `day`'s date, short month, in the region's order: "Jan 1" in
    /// the US, "1 Jan" elsewhere (R377: every date the calendar writes
    /// goes through here, so none is written one way only).
    fn date(&mut self, day: i32, us: bool) -> &mut Self {
        let d = date::date_of(day);
        let month = &MONTHS[usize::from(d.month - 1)][..3];
        if us {
            self.s(month).s(b" ").num(u32::from(d.day))
        } else {
            self.num(u32::from(d.day)).s(b" ").s(month)
        }
    }
    fn get(&self) -> &[u8] {
        &self.b[..self.n]
    }
}

/// ":00" to ":59", one after another (the gauge's minutes on the 24-hour
/// clock, a slice each).
static MINUTES: [u8; 180] = {
    let mut b = [0u8; 180];
    let mut i = 0;
    while i < 60 {
        b[i * 3] = b':';
        b[i * 3 + 1] = b'0' + (i / 10) as u8;
        b[i * 3 + 2] = b'0' + (i % 10) as u8;
        i += 1;
    }
    b
};

/// A time of day in the clock chosen: "3:00 PM" or "15:00" (R383: the
/// one writer every app shares).
fn clock(m: u16, clock12: bool) -> rp_app::Text<8> {
    rp_app::clock::time(m, clock12)
}

/// Every field's start, made as a value or straight into a slot
/// (`rp_hal_mem::in_place!`): one list for [`Calendar::new`] and
/// [`Calendar::new_in`].
macro_rules! calendar_start {
    ($rows:expr, $n_cals:expr, $today:expr; $($how:tt)*) => {
        rp_hal_mem::in_place!($($how)* Calendar {
            queue: [0; 8],
            n_queue: 0,
            cal_rows: [(Rect::new(0, 0, 0, 0), 0); 8],
            pad: None,
            edit: None,
            places: [(Text::EMPTY, Text::EMPTY); 3],
            n_places: 0,
            place_asked: Text::EMPTY,
            rows_at: [(Rect::new(0, 0, 0, 0), 0, false); 12],
            n_rows: 0,
            hits: kit::Hits::new(),
            focus: kit::Focus::default(),
            find: [0; 32],
            find_n: 0,
            find_typing: false,
            find_rows: [(Rect::new(0, 0, 0, 0), 0); 9],
            n_find: 0,
            find_first: 0,
            find_total: 0,
            n_cal_rows: 0,
            want_list: false,
            asked: Asked::Nothing,
            status: b"",
            synced: 0,
            cals: $rows,
            n_cals: $n_cals,
            appts: @each None,
            seen: None,
            bdays_seen: None,
            view: View::Home,
            day: $today,
            line: [0; 64],
            line_n: 0,
            ring: None,
            chosen: None,
            painted: false,
            gauge: None,
            under: View::Home,
            motion: None,
            slide: 0,
            drag: None,
            strip_at: None,
            ring_slices: [None; 4],
            io: files::Io::new(),
            status_text: Text::EMPTY,
        })
    };
}

impl Calendar {
    /// A calendar on `today` showing the store's calendars, `cals` until
    /// the store holds one.
    #[must_use]
    pub fn new(cals: &'static [Cal], today: i32) -> Self {
        let mut rows = [CalRow {
            id: 0,
            name: Text::EMPTY,
            colour: kit::IDENTITY[0],
            hidden: false,
            contacts: false,
        }; 8];
        for (r, c) in rows.iter_mut().zip(cals) {
            r.name = Text::of(c.name);
            r.colour = c.colour;
        }
        calendar_start!(rows, cals.len().clamp(1, 8), today; value)
    }

    /// [`Calendar::new`] made straight into `slot` (the program's, in
    /// `.bss`), a field at a time: made on the stack and moved it was a
    /// 22 KB frame (the stack rule, P6: 4 KB).
    pub fn new_in<'s>(
        slot: &'s mut core::mem::MaybeUninit<Self>,
        cals: &'static [Cal],
        today: i32,
    ) -> &'s mut Self {
        let rows = Self::rows_of(cals);
        calendar_start!(rows, cals.len().clamp(1, 8), today; slot,)
    }

    /// The calendars' rows before the store has any.
    fn rows_of(cals: &'static [Cal]) -> [CalRow; 8] {
        let mut rows = [CalRow {
            id: 0,
            name: Text::EMPTY,
            colour: kit::IDENTITY[0],
            hidden: false,
            contacts: false,
        }; 8];
        for (r, c) in rows.iter_mut().zip(cals) {
            r.name = Text::of(c.name);
            r.colour = c.colour;
        }
        rows
    }

    /// The list made again from the store when its records changed or
    /// the chosen day left the window: the calendars, then every
    /// appointment in the window, a repeating one on each day it occurs.
    fn refresh(&mut self, cx: &Ctx) {
        let Some(cell) = cx.pim else { return };
        let Ok(guard) = cell.try_borrow() else { return };
        let Some(store) = guard.as_ref() else { return };
        let from = self.day - 7;
        let changes = store.changes();
        if self
            .seen
            .is_some_and(|(c, f)| c == changes && self.day >= f + 7 && self.day < f + WINDOW - 7)
        {
            return;
        }
        self.seen = Some((changes, from));
        (self.io.device, self.io.guessed) = files::device_zone(store, cx);
        let device = self.io.device;
        let mut n_cals = 0;
        for r in store.all(Kind::Calendar) {
            if n_cals < self.cals.len() {
                self.cals[n_cals] = CalRow {
                    id: r.id,
                    name: Text::of(r.text(tag::NAME)),
                    colour: kit::IDENTITY[(r.num(tag::COLOUR).unwrap_or(0) % 8) as usize],
                    hidden: r.num(tag::HIDDEN) == Some(1),
                    contacts: r.text(tag::SOURCE) == rp_pim::birthdays::SOURCE,
                };
                n_cals += 1;
            }
        }
        if n_cals > 0 {
            self.n_cals = n_cals;
        }
        self.appts = [None; MAX];
        let mut k = 0;
        for r in store.all(Kind::Appointment) {
            let start = r.num(tag::START).unwrap_or(0);
            let first = date::day_of(start);
            let minute = (start % date::DAY_MIN) as u16;
            let len = if r.num(tag::ALL_DAY) == Some(1) {
                0
            } else {
                r.num(tag::MINUTES).unwrap_or(60).clamp(1, 24 * 60) as u16
            };
            let cal = r
                .num(tag::CAL)
                .and_then(|id| self.cals[..self.n_cals].iter().position(|c| c.id == id))
                .unwrap_or(0) as u8;
            // A hidden calendar's appointments stay out of every view.
            if self.cals[usize::from(cal)].hidden {
                continue;
            }
            let rule = r
                .num(tag::REPEAT)
                .and_then(|v| rp_pim::repeat::Rule::unpack(v, first));
            // A time kept in another zone, shown in the device's: each
            // occurrence moved on its own day (a clock change between
            // them moves one and not the other).
            let zone = r.text(tag::ZONE);
            let at = |day: i32| {
                let wall = date::start_of(day) + u32::from(minute);
                if len == 0 {
                    (day, minute)
                } else {
                    let s = files::shown(device, zone, wall);
                    (date::day_of(s), (s % date::DAY_MIN) as u16)
                }
            };
            let make = |day: i32| Appt {
                day: at(day).0,
                start: at(day).1,
                len,
                title: Text::of(r.text(tag::TITLE)),
                place: Text::of(r.text(tag::PLACE)),
                cal,
                repeats: rule.is_some(),
                task: false,
                id: r.id,
            };
            let inside = |day: i32| {
                let d = at(day).0;
                d >= from && d < from + WINDOW
            };
            match rule {
                None if inside(first) && k < MAX => {
                    self.appts[k] = Some(make(first));
                    k += 1;
                }
                None => {}
                Some(rule) => {
                    // A day either side: a zone's day can be the device's
                    // day before or after.
                    for day in (from - 1).max(first)..from + WINDOW + 1 {
                        if k < MAX && rule.occurs(first, day) && inside(day) {
                            self.appts[k] = Some(make(day));
                            k += 1;
                        }
                    }
                }
            }
        }
        // The Birthdays calendar: the contacts' birthdays and anniversaries
        // on their days, all day, unless it is hidden. Its rows carry the
        // contact's id: the editor opens appointments only.
        if let Some(b) = self.cals[..self.n_cals]
            .iter()
            .position(|c| c.contacts && !c.hidden)
        {
            let appts = &mut self.appts;
            rp_pim::birthdays::all(store, from, WINDOW, |e| {
                let Some(r) = store.get(e.id) else { return };
                if k >= MAX {
                    return;
                }
                let mut t = [0u8; 40];
                let n = rp_pim::birthdays::title(&r, &e, &mut t);
                appts[k] = Some(Appt {
                    day: e.day,
                    start: 0,
                    len: 0,
                    title: Text::of(&t[..n]),
                    place: Text::EMPTY,
                    cal: b as u8,
                    repeats: true,
                    task: false,
                    id: e.id,
                });
                k += 1;
            });
        }
        // Tasks on their due days (calendar-rule0.md, tasks-rule0.md: the
        // tie-in), the open ones, in the tasks' blue; a repeating one on
        // each of its days to come as well (Fantastical shows repeating
        // reminders once; the agenda is where the week is planned). An
        // "after done" task shows on its due day only: its next day is
        // not known until it is done.
        for r in store
            .all(Kind::Task)
            .filter(|r| r.num(tag::DONE).is_none() && r.num(tag::PARENT).is_none())
        {
            let Some(due) = r.num(tag::DUE).map(|d| d as i32) else {
                continue;
            };
            let rule = r
                .num(tag::REPEAT)
                .and_then(|v| rp_pim::repeat::Rule::unpack(v, due));
            for day in due.max(from)..from + WINDOW {
                if k >= MAX {
                    break;
                }
                if day != due && !rule.is_some_and(|x| x.occurs(due, day)) {
                    if rule.is_none() {
                        break;
                    }
                    continue;
                }
                self.appts[k] = Some(Appt {
                    day,
                    start: 0,
                    len: 0,
                    title: Text::of(r.text(tag::TITLE)),
                    place: Text::EMPTY,
                    cal: 0,
                    repeats: rule.is_some(),
                    task: true,
                    id: r.id,
                });
                k += 1;
            }
        }
    }

    /// The Birthdays calendar's record made the first time a contact has a
    /// birthday or an anniversary (in the eighth colour, the previews'
    /// Birthdays calendar's; shown and hidden as any calendar). A store
    /// with no calendar yet keeps this device's own first, so appointments
    /// with none still belong to it. Looked at once per change of the
    /// store.
    fn ensure_birthdays(&mut self, cx: &Ctx) {
        let Some(cell) = cx.pim else { return };
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return;
        };
        let Some(store) = guard.as_mut() else { return };
        if self.bdays_seen == Some(store.changes()) {
            return;
        }
        self.bdays_seen = Some(store.changes());
        if rp_pim::birthdays::calendar(store).is_some() || !rp_pim::birthdays::any(store) {
            return;
        }
        if store.all(Kind::Calendar).next().is_none() {
            let first = &self.cals[0];
            let c = kit::IDENTITY
                .iter()
                .position(|&x| x == first.colour)
                .unwrap_or(0) as u32;
            let _ = store.put(
                Kind::Calendar,
                None,
                &[
                    (tag::NAME, first.name.get()),
                    (tag::COLOUR, &c.to_le_bytes()),
                ],
            );
        }
        let _ = store.put(
            Kind::Calendar,
            None,
            &[
                (tag::NAME, b"Birthdays"),
                (tag::COLOUR, &7u32.to_le_bytes()),
                (tag::SOURCE, rp_pim::birthdays::SOURCE),
            ],
        );
        self.bdays_seen = Some(store.changes());
        self.seen = None;
    }

    /// Opens appointment `id` (or a new one from the quick-add line) in the
    /// editor.
    fn open_edit(&mut self, cx: &Ctx, id: Option<u32>) {
        let mut e = Edit {
            id,
            title: [0; 64],
            title_n: 0,
            place: [0; 64],
            place_n: 0,
            day: self.day,
            start: 9 * 60,
            len: 60,
            all_day: false,
            repeat: 0,
            kept_rule: None,
            // At its time (it was a quarter of an hour before: the owner on
            // the X51v, 2026-11-15, set one a few minutes off, the alert's
            // time was past as it was made, and "it never went off when it
            // was supposed to"; run today-0150 has no reminder in 814 s).
            alerts: 1 << 1,
            alert_other: 0,
            alert_at: 1,
            kept: [0; 4 * rp_pim::remind::LEADS_MAX],
            kept_n: 0,
            alerts_changed: true,
            cal: 0,
            read_only: false,
            typing: None,
            confirm_delete: false,
            task_made: false,
            row: None,
            zone: rp_pim::net::Short::new(),
        };
        match id {
            Some(id) => {
                let Some(cell) = cx.pim else { return };
                let Ok(guard) = cell.try_borrow() else { return };
                let Some(store) = guard.as_ref() else { return };
                let Some(r) = store.get(id) else { return };
                // A birthday's row is its contact: nothing to edit here.
                if r.kind != Kind::Appointment {
                    return;
                }
                let t = r.text(tag::TITLE);
                e.title_n = t.len().min(64);
                e.title[..e.title_n].copy_from_slice(&t[..e.title_n]);
                let pl = r.text(tag::PLACE);
                e.place_n = pl.len().min(64);
                e.place[..e.place_n].copy_from_slice(&pl[..e.place_n]);
                e.all_day = r.num(tag::ALL_DAY) == Some(1);
                e.zone.push(r.text(tag::ZONE));
                let at = r.num(tag::START).unwrap_or(0);
                let at = if e.all_day {
                    at
                } else {
                    files::shown(self.io.device, e.zone.get(), at)
                };
                e.day = date::day_of(at);
                e.start = (at % date::DAY_MIN) as u16;
                e.len = r.num(tag::MINUTES).unwrap_or(60).min(24 * 60) as u16;
                let rule = r
                    .num(tag::REPEAT)
                    .and_then(|v| rp_pim::repeat::Rule::unpack(v, e.day));
                e.repeat = rule.map_or(0, |r| match r.every {
                    rp_pim::repeat::Every::Day => 1,
                    rp_pim::repeat::Every::Week => 2,
                    rp_pim::repeat::Every::MonthDate | rp_pim::repeat::Every::MonthWeekday => 3,
                    rp_pim::repeat::Every::Year => 4,
                });
                e.kept_rule = rule.map(|r| (r, e.repeat));
                e.load_alerts(&r);
                let cal = r.num(tag::CAL);
                e.cal = cal
                    .and_then(|c| self.cals[..self.n_cals].iter().position(|x| x.id == c))
                    .unwrap_or(0);
                e.read_only = cal
                    .and_then(|c| store.get(c))
                    .is_some_and(|c| c.text(tag::SOURCE).starts_with(b"ical:"));
            }
            None => {
                // A new one is kept in the device's zone: it stays at its
                // moment if the device later moves to another.
                if let Some(z) = self.io.device {
                    let mut b = [0u8; 40];
                    let n = z.name(&mut b);
                    e.zone.push(&b[..n]);
                }
                // From the quick-add line: what the parser understood.
                let line = &self.line[..self.line_n];
                let mut title = [0u8; rp_pim::quick::TITLE_MAX];
                let (q, tn) = rp_pim::quick::parse(line, cx.now.day, cx.us, &mut title);
                e.title_n = tn.min(64);
                e.title[..e.title_n].copy_from_slice(&title[..e.title_n]);
                e.day = q.day.unwrap_or(cx.now.day);
                match q.time {
                    Some(t) => e.start = t,
                    None => e.all_day = true,
                }
                e.len = q.minutes.unwrap_or(60);
            }
        }
        self.edit = Some(e);
        self.view = View::Edit;
    }

    /// The editor kept to the store (a new appointment added), back to
    /// the agenda.
    fn save_edit(&mut self, cx: &Ctx) -> bool {
        let Some(e) = self.edit else { return false };
        if e.read_only || e.title_n == 0 {
            return false;
        }
        let le = |v: u32| v.to_le_bytes();
        let start = date::start_of(e.day) + if e.all_day { 0 } else { u32::from(e.start) };
        // The device's time typed, kept as the time of the appointment's
        // own zone.
        let start = if e.all_day {
            start
        } else {
            files::kept(self.io.device, e.zone.get(), start)
        };
        let (st, mi, one) = (le(start), le(u32::from(e.len.max(5))), le(1));
        let cal = le(self.cals[e.cal.min(self.n_cals - 1)].id);
        let rule = (e.repeat > 0).then(|| {
            use rp_pim::repeat::{Every, Rule};
            // Its own rule while its chip is chosen; a chip changed is the
            // plain rule it names.
            if let Some((r, _)) = e.kept_rule.filter(|&(_, c)| c == e.repeat) {
                return le(r.pack(e.day));
            }
            let every =
                [Every::Day, Every::Week, Every::MonthDate, Every::Year][usize::from(e.repeat - 1)];
            le(Rule {
                every,
                n: 1,
                days: 0,
                until: None,
                after: false,
            }
            .pack(e.day))
        });
        // Several alerts (rp_pim::remind): the chips' leads, or the
        // record's own as they were when the row wasn't changed.
        let mut alert = [0u8; 4 * rp_pim::remind::LEADS_MAX];
        let alert_n = if e.alerts_changed {
            let (v, n) = e.leads();
            rp_pim::remind::pack(&v[..n], &mut alert)
        } else {
            let n = usize::from(e.kept_n);
            alert[..n].copy_from_slice(&e.kept[..n]);
            n
        };
        let mut fields: [(u8, &[u8]); 9] = [
            (tag::TITLE, &e.title[..e.title_n]),
            (tag::PLACE, &e.place[..e.place_n]),
            (tag::START, &st),
            (tag::MINUTES, &mi),
            (tag::CAL, &[]),
            (tag::ALL_DAY, &[]),
            (tag::REPEAT, &[]),
            (tag::REMIND, &[]),
            (tag::ZONE, if e.all_day { &[] } else { e.zone.get() }),
        ];
        if self.cals[e.cal.min(self.n_cals - 1)].id != 0 {
            fields[4].1 = &cal;
        }
        if e.all_day {
            fields[5].1 = &one;
        }
        if let Some(r) = &rule {
            fields[6].1 = r;
        }
        if alert_n > 0 {
            fields[7].1 = &alert[..alert_n];
        }
        let Some(cell) = cx.pim else { return false };
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return false;
        };
        let Some(store) = guard.as_mut() else {
            return false;
        };
        // An edit changes the editor's fields only (an empty one removed):
        // the store keeps the rest (its calendar, a synced id, later tags).
        let ok = match e.id {
            Some(id) => store.change(id, &fields).is_ok(),
            None => store.put(Kind::Appointment, None, &fields).is_ok(),
        };
        if ok {
            self.seen = None;
            self.day = e.day;
            self.line_n = 0;
        }
        ok
    }

    /// "Make a task" (tasks-rule0.md: the tie-in with Calendar): a task in
    /// Tasks' first list with the appointment's title, due on its day (a
    /// repeating one's next day from the one shown), and a note saying
    /// where it came from: when and where the appointment is. Fantastical
    /// copies the title alone; the note keeps the time and place with the
    /// task, so it reads on its own in Tasks. True once it is kept.
    fn make_task(cx: &Ctx, e: &Edit) -> bool {
        let Some(cell) = cx.pim else { return false };
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return false;
        };
        let Some(store) = guard.as_mut() else {
            return false;
        };
        let rule =
            e.id.and_then(|id| store.get(id))
                .and_then(|r| r.num(tag::REPEAT))
                .and_then(|v| rp_pim::repeat::Rule::unpack(v, e.day));
        let due = match rule {
            Some(r) => {
                let from = e.day.max(cx.now.day);
                (from..from + 1462)
                    .find(|&d| r.occurs(e.day, d))
                    .unwrap_or(e.day)
            }
            None => e.day,
        };
        let mut note = Buf::<96>::new();
        note.s(b"From Calendar: ")
            .s(date::WEEKDAYS[usize::from(date::weekday(due))].as_bytes())
            .s(b" ")
            .date(due, cx.us);
        if !e.all_day {
            note.s(b", ").s(clock(e.start, cx.clock12).get());
        }
        if e.place_n > 0 {
            note.s(b", ").s(&e.place[..e.place_n]);
        }
        let list = store.all(Kind::List).next().map(|l| l.id.to_le_bytes());
        let day = (due.max(0) as u32).to_le_bytes();
        let mut fields: [(u8, &[u8]); 4] = [
            (tag::TITLE, &e.title[..e.title_n]),
            (tag::DUE, &day),
            (tag::NOTE, note.get()),
            (tag::LIST, &[]),
        ];
        let n = match &list {
            Some(l) => {
                fields[3].1 = l;
                4
            }
            None => 3,
        };
        e.title_n > 0 && store.put(Kind::Task, None, &fields[..n]).is_ok()
    }

    /// The calendars an appointment can be put in (the editor's chips,
    /// four at most): every one but Birthdays, whose days are the
    /// contacts'.
    fn chip_cals(&self) -> ([usize; 4], usize) {
        let mut ix = [0; 4];
        let mut n = 0;
        for (i, c) in self.cals[..self.n_cals].iter().enumerate() {
            if !c.contacts && n < ix.len() {
                ix[n] = i;
                n += 1;
            }
        }
        (ix, n)
    }

    /// The editor's grouped cards.
    #[inline(never)]
    fn edit_view(&self, f: &mut Frame, cx: &Ctx) {
        let Some(e) = &self.edit else { return };
        let field = |f: &mut Frame, y: u16, label: &[u8], value: &[u8], on: bool| {
            let r = Rect::new(MARGIN, y, WIDE, 40);
            kit::panel(f, cx, r);
            if on {
                f.push(rp_app::Cmd::Outline {
                    r,
                    color: kit::FOCUS,
                });
            }
            kit::text(f, r.x + 12, r.y + 26, cx.fonts.label, DIM, label);
            kit::fitted(f, r.x + 96, r.y + 27, cx.fonts.ui, TEXT, value, r.w - 108);
        };
        field(
            f,
            E_TITLE,
            b"WHAT",
            &e.title[..e.title_n],
            e.typing == Some(EditText::Title),
        );
        field(
            f,
            E_PLACE,
            b"WHERE",
            &e.place[..e.place_n],
            e.typing == Some(EditText::Place),
        );
        // When: steppers with Aero keys at either end.
        let stepper = |f: &mut Frame, y: u16, label: &[u8], value: &[u8]| {
            let r = Rect::new(MARGIN, y, WIDE, 40);
            kit::panel(f, cx, r);
            kit::text(f, r.x + 12, r.y + 26, cx.fonts.label, DIM, label);
            for (x, w) in [(r.x + 120, b"<"), (r.right() - 52, b">")] {
                let k = Rect::new(x, r.y + 4, 44, 32);
                kit::key(f, cx, k, kit::KeyState::Rest);
                kit::centred(f, k, k.y + 22, cx.fonts.ui, TEXT, w);
            }
            kit::centred(
                f,
                Rect::new(r.x + 164, 0, r.w - 216, 1),
                r.y + 27,
                cx.fonts.ui,
                TEXT,
                value,
            );
        };
        let mut d = Buf::<32>::new();
        d.s(WEEKDAYS[usize::from(date::weekday(e.day))])
            .s(b" ")
            .date(e.day, cx.us);
        stepper(f, E_DAY, b"DAY", d.get());
        if e.all_day {
            stepper(f, E_START, b"STARTS", b"All day");
        } else {
            stepper(f, E_START, b"STARTS", clock(e.start, cx.clock12).get());
            // A time kept in another zone says where it is from, under
            // the row: "9:00 AM in New York".
            let device_name = self.io.device.map(|z| {
                let mut b = [0u8; 40];
                let n = z.name(&mut b);
                (b, n)
            });
            let other =
                !e.zone.is_empty() && device_name.is_none_or(|(b, n)| &b[..n] != e.zone.get());
            if other {
                let wall = files::kept(
                    self.io.device,
                    e.zone.get(),
                    date::start_of(e.day) + u32::from(e.start),
                );
                if let Some(note) = files::zone_note(e.zone.get(), wall, cx.clock12) {
                    kit::right(
                        f,
                        MARGIN + WIDE - 14,
                        E_LEN + 53,
                        cx.fonts.label,
                        tint::CALENDAR,
                        note.get(),
                    );
                }
            }
        }
        let mut l = Buf::<16>::new();
        if e.len >= 60 && e.len % 60 == 0 {
            l.num(u32::from(e.len / 60))
                .s(if e.len == 60 { b" hour" } else { b" hours" });
        } else {
            l.num(u32::from(e.len)).s(b" min");
        }
        stepper(f, E_LEN, b"LASTS", l.get());
        let chips_row = |f: &mut Frame, y: u16, label: &[u8], names: &[&[u8]], chosen: usize| {
            kit::text(f, MARGIN + 4, y - 4, cx.fonts.label, DIM, label);
            kit::chips(
                f,
                cx,
                Rect::new(MARGIN, y, WIDE, 30),
                names,
                chosen,
                tint::CALENDAR,
            );
        };
        chips_row(
            f,
            E_REPEAT + 14,
            b"REPEAT",
            &REPEAT_NAMES,
            usize::from(e.repeat),
        );
        // A rule the chip says only part of, in words beside the label
        // ("Every 2 weeks on Mon, Wed until Mar 3"): what is kept on save.
        if let Some((r, _)) = e
            .kept_rule
            .filter(|&(r, c)| c == e.repeat && !r.plain(e.day))
        {
            let mut w = [0u8; 64];
            let n = r.words(e.day, cx.us, &mut w);
            let x = MARGIN + 4 + cx.fonts.label.width(b"REPEAT") + 10;
            kit::text(f, x, E_REPEAT + 10, cx.fonts.label, tint::CALENDAR, &w[..n]);
        }
        // The quick leads, and any other to the minute (the sixth chip
        // brings the pad: a number and its unit, as Google Calendar's
        // custom alert; the owner, 2026-11-15, "by the min").
        let mut other = Buf::<10>::new();
        if e.alerts & (1 << 5) != 0 {
            let m = e.alert_other;
            if m.is_multiple_of(1440) {
                other.num(m / 1440).s(b" DAY");
            } else if m.is_multiple_of(60) {
                other.num(m / 60).s(b" HR");
            } else {
                other.num(m).s(b" MIN");
            }
        } else {
            other.s(b"OTHER");
        }
        let alerts: [&[u8]; 6] = [
            ALERT_NAMES[0],
            b"START",
            ALERT_NAMES[2],
            ALERT_NAMES[3],
            ALERT_NAMES[4],
            other.get(),
        ];
        // Several on at once (each lit), NONE lit when none is; the
        // D-pad's chip ringed while the row is chosen.
        kit::text(f, MARGIN + 4, E_ALERT + 10, cx.fonts.label, DIM, b"ALERTS");
        let row = Rect::new(MARGIN, E_ALERT + 14, WIDE, 30);
        for (k, word) in alerts.iter().enumerate() {
            let on = if k == 0 {
                e.alerts == 0
            } else {
                e.alerts & (1 << k) != 0
            };
            kit::gel(f, cx, kit::chip(row, alerts.len(), k), word, on);
        }
        if e.row == Some(6) {
            f.push(rp_app::Cmd::Outline {
                r: kit::chip(row, alerts.len(), usize::from(e.alert_at)),
                color: TEXT,
            });
        }
        let mut names: [&[u8]; 4] = [b""; 4];
        let (ix, n) = self.chip_cals();
        for (k, &i) in ix[..n].iter().enumerate() {
            names[k] = self.cals[i].name.get();
        }
        chips_row(
            f,
            E_CAL + 14,
            b"CALENDAR",
            &names[..n],
            ix[..n].iter().position(|&i| i == e.cal).unwrap_or(0),
        );
        // A kept appointment (a calendar link's too) makes a task in
        // Tasks; one of this device's can be deleted beside it.
        if e.id.is_some() {
            let (made, delete) = edit_keys(e.read_only);
            kit::key(f, cx, made, kit::KeyState::Rest);
            kit::centred(
                f,
                made,
                made.y + 27,
                cx.fonts.ui,
                if e.task_made { tint::TASKS } else { TEXT },
                if e.task_made {
                    b"Task made in Tasks"
                } else {
                    b"Make a task"
                },
            );
            if let Some(r) = delete {
                kit::key(f, cx, r, kit::KeyState::Down);
                kit::centred(
                    f,
                    r,
                    r.y + 27,
                    cx.fonts.ui,
                    kit::LATE,
                    if e.confirm_delete {
                        b"Tap again"
                    } else {
                        b"Delete"
                    },
                );
            }
        }
        if e.read_only {
            kit::text(
                f,
                MARGIN + 4,
                E_DELETE + 64,
                cx.fonts.ui,
                DIM,
                b"From a calendar link: read only",
            );
        }
        if let Some(&(top, h)) = e.row.and_then(|r| EDIT_ROWS.get(usize::from(r))) {
            for grow in [1u16, 2] {
                f.push(rp_app::Cmd::Outline {
                    r: Rect::new(MARGIN - grow, top - grow, WIDE + 2 * grow, h + 2 * grow),
                    color: kit::FOCUS,
                });
            }
        }
    }

    /// The D-pad in the editor (R203): up and down move from row to row;
    /// left and right change the row's value (the day, the start, the
    /// length a step; a row of chips to the one beside the chosen); the
    /// centre types in the title or the place and presses Delete. Each is
    /// the tap the stylus would make, so the two never differ. Windows
    /// Mobile's editor walks its fields in tab order and opens a picker
    /// for each.
    fn edit_key(&mut self, cx: &Ctx, k: Key) -> Reply {
        let (ix, n_cals) = self.chip_cals();
        let Some(e) = self.edit.as_mut() else {
            return Reply::Nothing;
        };
        let last = if e.id.is_some() { 8 } else { 7 };
        let to = match (k, e.row) {
            (Key::Down, None) => 0,
            (Key::Up, None) => last,
            (Key::Down, Some(r)) => (r + 1).min(last),
            (Key::Up, Some(r)) => r.saturating_sub(1),
            (_, None) => return Reply::Nothing,
            (_, Some(r)) => {
                let (top, h) = EDIT_ROWS[usize::from(r)];
                let right = k != Key::Left;
                if r == 6 {
                    let row = Rect::new(MARGIN, E_ALERT + 14, WIDE, 30);
                    if k == Key::Enter {
                        let c = kit::chip(row, 6, usize::from(e.alert_at));
                        return self.edit_tap(cx, c.x + c.w / 2, c.y + c.h / 2);
                    }
                    e.alert_at = if right {
                        (e.alert_at + 1).min(5)
                    } else {
                        e.alert_at.saturating_sub(1)
                    };
                    return Reply::Redraw;
                }
                let x = match r {
                    // (The centre on the start: its time pad.)
                    0 | 1 | 3 if k == Key::Enter => W / 2,
                    // The last row: Make a task (the centre and left), and
                    // Delete on the right.
                    8 => {
                        let (made, delete) = edit_keys(e.read_only);
                        match (k, delete) {
                            (Key::Right, Some(d)) => d.x + d.w / 2,
                            (Key::Right, None) => return Reply::Nothing,
                            _ => made.x + made.w / 2,
                        }
                    }
                    2..=4 if k != Key::Enter => {
                        if right {
                            W - MARGIN - 30
                        } else {
                            W / 4
                        }
                    }
                    5..=7 if k != Key::Enter => {
                        let (n, at) = match r {
                            5 => (5, usize::from(e.repeat)),
                            _ => (
                                n_cals,
                                ix[..n_cals].iter().position(|&i| i == e.cal).unwrap_or(0),
                            ),
                        };
                        if n == 0 {
                            return Reply::Nothing;
                        }
                        let chip = if right {
                            (at + 1).min(n - 1)
                        } else {
                            at.saturating_sub(1)
                        } as u16;
                        let n = n as u16;
                        MARGIN + chip * WIDE / n + WIDE / (2 * n)
                    }
                    _ => return Reply::Nothing,
                };
                return self.edit_tap(cx, x, top + h / 2);
            }
        };
        // (Up on the first row: nothing higher, the shell takes it.)
        if e.row == Some(to) {
            return Reply::Nothing;
        }
        e.row = Some(to);
        e.typing = None;
        e.confirm_delete = false;
        Reply::Redraw
    }

    /// The alert's pad up on `lead` minutes (the previews: on the device
    /// the editor's sixth alert chip brings it).
    pub fn show_alert_pad(&mut self, lead: u32) {
        self.pad = Some((1, rp_app::timepad::TimePad::before(Some(lead))));
    }

    /// A tap in the editor.
    fn edit_tap(&mut self, cx: &Ctx, x: u16, y: u16) -> Reply {
        let (ix, n_cals) = self.chip_cals();
        let Some(e) = self.edit.as_mut() else {
            return Reply::Nothing;
        };
        let hit = |top: u16, h: u16| y >= top && y < top + h;
        let chip = |n: usize| usize::from((x.saturating_sub(MARGIN)) * n as u16 / WIDE).min(n - 1);
        let (made, delete) = edit_keys(e.read_only);
        if e.id.is_some() && made.contains_point(x, y) {
            e.confirm_delete = false;
            if !e.task_made {
                let e = *e;
                if let Some(ed) = self.edit.as_mut() {
                    ed.task_made = Self::make_task(cx, &e);
                }
            }
            return Reply::Redraw;
        }
        if e.read_only {
            return Reply::Nothing;
        }
        let on_delete = delete.is_some_and(|d| d.contains_point(x, y));
        if !on_delete {
            e.confirm_delete = false;
        }
        if hit(E_TITLE, 40) {
            e.typing = Some(EditText::Title);
        } else if hit(E_PLACE, 40) {
            e.typing = Some(EditText::Place);
        } else if hit(E_DAY, 40) {
            e.day += if x < W / 2 { -1 } else { 1 };
        } else if hit(E_START, 40) {
            // The arrows step five minutes (iOS's wheel); a touch on the
            // time itself brings the pad, to the minute.
            if e.all_day {
                e.all_day = false;
            } else if x < MARGIN + 164 {
                e.start = e.start.saturating_sub(5);
            } else if x >= W - MARGIN - 56 {
                e.start = (e.start + 5).min(23 * 60 + 59);
            } else {
                self.pad = Some((
                    0,
                    rp_app::timepad::TimePad::clock(Some(e.start), cx.clock12),
                ));
            }
        } else if hit(E_LEN, 40) {
            let step = if e.len >= 60 { 30 } else { 15 };
            e.len = if x < W / 2 {
                e.len.saturating_sub(step).max(15)
            } else {
                (e.len + step).min(24 * 60)
            };
        } else if hit(E_REPEAT + 14, 30) {
            e.repeat = chip(5) as u8;
        } else if hit(E_ALERT + 14, 30) {
            let k = chip(6);
            e.alert_at = k as u8;
            if k == 5 {
                let lead = (e.alerts & (1 << 5) != 0).then_some(e.alert_other);
                self.pad = Some((1, rp_app::timepad::TimePad::before(lead)));
            } else {
                e.toggle_alert(k);
            }
        } else if hit(E_CAL + 14, 30) && n_cals > 0 {
            e.cal = ix[chip(n_cals)];
        } else if on_delete && e.id.is_some() {
            if e.confirm_delete {
                let id = e.id;
                self.edit = None;
                self.view = View::Agenda;
                if let (Some(id), Some(cell)) = (id, cx.pim)
                    && let Ok(mut g) = cell.try_borrow_mut()
                    && let Some(store) = g.as_mut()
                {
                    let _ = store.remove(id);
                }
                self.seen = None;
            } else {
                e.confirm_delete = true;
            }
        } else {
            e.typing = None;
            return Reply::Redraw;
        }
        Reply::Redraw
    }

    /// The calendars and how to get online ones.
    #[inline(never)]
    fn calendars_view(&mut self, f: &mut Frame, cx: &Ctx, top: u16) {
        let mut y = top;
        self.n_cal_rows = 0;
        let guard = cx.pim.and_then(|c| c.try_borrow().ok());
        let store = guard.as_ref().and_then(|g| g.as_ref());
        for c in &self.cals[..self.n_cals] {
            // (The time zone's row and the file keys under the list.)
            if y + 56 > files::ZONE_ROW.y {
                break;
            }
            let r = Rect::new(MARGIN, y, WIDE, 52);
            kit::tinted(f, cx, r, c.colour, 0x40);
            if self.n_cal_rows < self.cal_rows.len() {
                self.cal_rows[self.n_cal_rows] = (r, c.id);
                self.n_cal_rows += 1;
            }
            kit::fitted(
                f,
                r.x + 22,
                r.y + 22,
                cx.fonts.ui,
                if c.hidden { DIM } else { TEXT },
                c.name.get(),
                300,
            );
            kit::right(
                f,
                r.right() - 14,
                r.y + 22,
                cx.fonts.label,
                if c.hidden { DIM } else { kit::DONE },
                if c.hidden { b"HIDDEN" } else { b"SHOWN" },
            );
            let src = store.and_then(|s| s.get(c.id)).map(|r| {
                let t = r.text(tag::SOURCE);
                t.starts_with(b"ical:")
            });
            let from: &[u8] = if src == Some(true) {
                b"From a link on your PC \x80 read only"
            } else {
                b"On this device"
            };
            kit::text(f, r.x + 22, r.y + 44, cx.fonts.ui, DIM, from);
            y += 56;
        }
        let note = Rect::new(MARGIN, y + 4, WIDE, 84);
        // The way to a link, when there is room above the zone's row.
        if note.bottom() + 4 <= files::ZONE_ROW.y {
            Self::link_note(f, cx, note);
        }
        drop(guard);
        self.files_rows(f, cx);
        let status = if self.status == b"\x01" {
            self.status_text.get()
        } else {
            self.status
        };
        if !status.is_empty() {
            kit::text(
                f,
                MARGIN + 6,
                GET_KEY.y - 5,
                cx.fonts.ui,
                kit::FOCUS,
                status,
            );
        }
        kit::key(f, cx, GET_KEY, kit::KeyState::On);
        kit::centred(
            f,
            GET_KEY,
            GET_KEY.y + 29,
            cx.fonts.ui,
            TEXT,
            b"Get calendars and sync",
        );
    }

    /// How to get a calendar's link, on the calendars' screen.
    fn link_note(f: &mut Frame, cx: &Ctx, note: Rect) {
        kit::panel(f, cx, note);
        kit::text(
            f,
            note.x + 14,
            note.y + 24,
            cx.fonts.ui,
            TEXT,
            b"Google, Outlook, iCloud: on your PC,",
        );
        kit::text(
            f,
            note.x + 14,
            note.y + 48,
            cx.fonts.ui,
            TEXT,
            b"copy the calendar's secret iCal",
        );
        kit::text(
            f,
            note.x + 14,
            note.y + 72,
            cx.fonts.ui,
            DIM,
            b"address, then touch the key below",
        );
    }

    /// Calendar `id` shown or hidden (its record kept with HIDDEN set or
    /// left out).
    fn toggle_hidden(&mut self, cx: &Ctx, id: u32) {
        let Some(cell) = cx.pim else { return };
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return;
        };
        let Some(store) = guard.as_mut() else { return };
        let Some(r) = store.get(id) else { return };
        // HIDDEN only: the store keeps the name, colour, source and the rest.
        let one = 1u32.to_le_bytes();
        let hidden: &[u8] = if r.num(tag::HIDDEN) == Some(1) {
            &[]
        } else {
            &one
        };
        if store.change(id, &[(tag::HIDDEN, hidden)]).is_ok() {
            self.seen = None;
        }
    }

    /// The PC's list of calendar links (`n|Name` lines): a calendar each
    /// in the store (its SOURCE `ical:n`), and each queued to read.
    fn take_list(&mut self, body: &[u8], cx: &Ctx) {
        let Some(cell) = cx.pim else { return };
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return;
        };
        let Some(store) = guard.as_mut() else { return };
        self.n_queue = 0;
        for line in body.split(|&c| c == b'\n') {
            let Some(bar) = line.iter().position(|&c| c == b'|') else {
                continue;
            };
            let (num, name) = (&line[..bar], &line[bar + 1..]);
            let Some(n) = num.iter().try_fold(0u8, |v, &c| {
                c.is_ascii_digit()
                    .then(|| v.saturating_mul(10).saturating_add(c - b'0'))
            }) else {
                continue;
            };
            let mut source = [0u8; 8];
            source[..5].copy_from_slice(b"ical:");
            let sl = 5 + usize::from(n > 9);
            if n > 9 {
                source[5] = b'0' + n / 10;
            }
            source[sl] = b'0' + n % 10;
            let source = &source[..=sl];
            let known = store
                .all(Kind::Calendar)
                .any(|r| r.text(tag::SOURCE) == source);
            if !known {
                // The first colour no calendar has yet.
                let used = store
                    .all(Kind::Calendar)
                    .fold(0u8, |m, r| m | 1 << (r.num(tag::COLOUR).unwrap_or(0) % 8));
                let colour = (0..8u32)
                    .find(|&k| used & (1 << k) == 0)
                    .unwrap_or(5)
                    .to_le_bytes();
                let name = &name[..name.len().min(40)];
                let _ = store.put(
                    Kind::Calendar,
                    None,
                    &[
                        (tag::NAME, name),
                        (tag::COLOUR, &colour),
                        (tag::SOURCE, source),
                    ],
                );
            }
            if self.n_queue < self.queue.len() {
                self.queue[self.n_queue] = n;
                self.n_queue += 1;
            }
        }
        self.status = if self.n_queue == 0 {
            b"No iCal address copied on your PC yet"
        } else {
            b"Reading your calendars..."
        };
        self.seen = None;
    }

    /// Calendar link `n`'s file: its events replace that calendar's in the
    /// store (a link is read only: the copy here is the link's).
    fn take_link(&mut self, n: u8, body: &[u8], cx: &Ctx) {
        let Some(cell) = cx.pim else { return };
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return;
        };
        let Some(store) = guard.as_mut() else { return };
        let mut want = [0u8; 8];
        want[..5].copy_from_slice(b"ical:");
        let sl = 5 + usize::from(n > 9);
        if n > 9 {
            want[5] = b'0' + n / 10;
        }
        want[sl] = b'0' + n % 10;
        let Some(cal) = store
            .all(Kind::Calendar)
            .find(|r| r.text(tag::SOURCE) == &want[..=sl])
            .map(|r| r.id)
        else {
            return;
        };
        // The old copy out (ids gathered first: the store changes as they go).
        let mut old = [0u32; 256];
        let mut k = 0;
        for r in store
            .all(Kind::Appointment)
            .filter(|r| r.num(tag::CAL) == Some(cal))
        {
            if k < old.len() {
                old[k] = r.id;
                k += 1;
            }
        }
        for id in &old[..k] {
            let _ = store.remove(*id);
        }
        let mut put = 0;
        let mut full = false;
        // Each event put as the reader hands it over (no array of them on
        // the stack), at most PARSED a link.
        rp_pim::ics::ics_each(body, |e| {
            let fields = EventFields::of(e, cal);
            if store.put(Kind::Appointment, None, &fields.list()).is_ok() {
                put += 1;
                put < PARSED as u32
            } else {
                full = true;
                false
            }
        });
        if full {
            self.status = b"The calendar store is full";
        }
        self.synced += put;
        self.seen = None;
        if self.n_queue == 0 && self.status != b"The calendar store is full" {
            self.status = b"Synced";
        }
    }

    /// The quick-add line kept as an appointment in the store: what the
    /// parser understood, an hour long and reminded 15 minutes before
    /// unless the line says otherwise, in the first calendar.
    fn add_typed(&mut self, cx: &Ctx) -> bool {
        let line = &self.line[..self.line_n];
        let mut title = [0u8; rp_pim::quick::TITLE_MAX];
        let (q, tn) = rp_pim::quick::parse(line, cx.now.day, cx.us, &mut title);
        if tn == 0 {
            return false;
        }
        let day = q.day.unwrap_or(cx.now.day);
        let start = date::start_of(day) + u32::from(q.time.unwrap_or(0));
        let all_day: u32 = u32::from(q.time.is_none());
        let minutes = u32::from(q.minutes.unwrap_or(60));
        let cal = self.cals[0].id;
        let n = |v: u32| v.to_le_bytes();
        let (s, m, a, rmd, c) = (n(start), n(minutes), n(all_day), n(0), n(cal));
        let mut fields: [(u8, &[u8]); 6] = [
            (tag::TITLE, &title[..tn]),
            (tag::START, &s),
            (tag::MINUTES, &m),
            (tag::REMIND, &rmd),
            (tag::ALL_DAY, &[]),
            (tag::CAL, &[]),
        ];
        if all_day == 1 {
            fields[4].1 = &a;
        }
        if cal != 0 {
            fields[5].1 = &c;
        }
        let Some(cell) = cx.pim else { return false };
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return false;
        };
        let Some(store) = guard.as_mut() else {
            return false;
        };
        let ok = store.put(Kind::Appointment, None, &fields).is_ok();
        if ok {
            self.day = day;
            self.seen = None;
            self.line_n = 0;
        }
        ok
    }

    /// Shows `view`.
    pub fn show(&mut self, view: View) {
        self.view = view;
    }

    /// The previews' frame of a motion: the strip dragged `off` px, or
    /// frame `k` of a zoom from `from` to the view shown (the owner's pick
    /// of the looks is made from these, drawn by this code).
    pub fn mock_motion(&mut self, off: i16, zoom: Option<(View, u8)>, us: bool) {
        self.slide = off;
        if let Some((from, k)) = zoom {
            let to = self.view;
            self.motion = Some(Motion::Zoom {
                from,
                to,
                cell: self.zoom_cell(from, to, us),
                k,
            });
        }
    }

    /// Chooses `day`.
    pub fn choose(&mut self, day: i32) {
        self.day = day;
    }

    /// Sets the quick-add line (the previews type it).
    pub fn set_line(&mut self, s: &[u8]) {
        let n = s.len().min(self.line.len());
        self.line[..n].copy_from_slice(&s[..n]);
        self.line_n = n;
    }

    /// The day's appointments in time order (all-day first).
    #[inline(never)]
    fn on(&self, day: i32) -> ([Appt; 16], usize) {
        let mut out = [Appt {
            day: 0,
            start: 0,
            len: 0,
            title: Text::EMPTY,
            place: Text::EMPTY,
            cal: 0,
            repeats: false,
            task: false,
            id: 0,
        }; 16];
        let mut n = 0;
        for a in self.appts.iter().flatten() {
            if a.day == day && n < out.len() {
                out[n] = *a;
                n += 1;
            }
        }
        out[..n].sort_unstable_by_key(|a| (a.len != 0, a.start));
        (out, n)
    }

    fn colour(&self, a: &Appt) -> u32 {
        if a.task {
            return tint::TASKS;
        }
        self.cals[..self.n_cals]
            .get(usize::from(a.cal))
            .map_or(kit::IDENTITY[0], |c| c.colour)
    }

    fn paint(&mut self, cx: &Ctx) {
        if self.painted {
            return;
        }
        if let Some(p) = cx.pics {
            self.ring = kit::disc(p, 40, 0x0000_0000, Some(tint::CALENDAR));
            self.chosen = kit::disc(p, 40, tint::CALENDAR, None);
        }
        self.painted = true;
    }

    /// The title band: the weekday and date, and what today holds.
    fn title(&self, f: &mut Frame, cx: &Ctx) -> u16 {
        let today = cx.now.day;
        let mut t = Buf::<32>::new();
        t.s(WEEKDAY_NAMES[usize::from(date::weekday(self.day))])
            .s(b", ")
            .date(self.day, cx.us);
        let (list, n) = self.on(today);
        let mut sum = Buf::<64>::new();
        let next = list[..n]
            .iter()
            .find(|a| a.len != 0 && a.start + a.len > cx.now.minute);
        match (n, next) {
            (0, _) => sum.s(b"Nothing today"),
            (n, Some(a)) => sum
                .num(n as u32)
                .s(b" today \x80 next ")
                .s(clock(a.start, cx.clock12).get())
                .s(b" ")
                .s(a.title.get()),
            (n, None) => sum.num(n as u32).s(b" today \x80 nothing more"),
        };
        kit::large_title(f, cx, 36, t.get(), sum.get(), tint::CALENDAR)
    }

    /// The week strip: the chosen day's week, today ringed, the chosen day
    /// filled, a colour tick per appointment (three at most). While the
    /// pen drags it or it settles (motion.rs) the weeks either side slide
    /// in, clipped to the strip's band: only the band is drawn again (the
    /// draw-list difference), never the view under it.
    #[inline(never)]
    fn strip(&mut self, f: &mut Frame, cx: &Ctx, y: u16) {
        let band = Rect::new(MARGIN, y, WIDE, 62);
        self.strip_at = Some(band);
        let off = self.strip_offset();
        let first = self.day - in_week(self.day, cx.us);
        if off == 0 {
            self.week_cells(f, cx, y, first, i32::from(MARGIN), true);
            return;
        }
        let outer = f.clip_now();
        f.push(rp_app::Cmd::Clip {
            r: band.intersect(&outer),
        });
        let w = i32::from(motion::STRIP_W);
        for k in -1i32..=1 {
            let x0 = i32::from(MARGIN) + i32::from(off) + k * w;
            if x0 + w <= i32::from(MARGIN) || x0 >= i32::from(W - MARGIN) {
                continue;
            }
            self.week_cells(f, cx, y, first + 7 * k, x0, false);
        }
        f.push(rp_app::Cmd::Clip { r: outer });
        {
            // The week nearest the middle: its dates on a glass chip over
            // the weekday names (the owner's pick B, 2026-11-23: crossing a
            // month is never a guess).
            let w = i32::from(motion::STRIP_W);
            let shift = (-i32::from(off) + w / 2).div_euclid(w);
            let a = date::date_of(first + 7 * shift);
            let b = date::date_of(first + 7 * shift + 6);
            // In the region's order (R383): "Jan 29 - Feb 4", or
            // "29 Jan - 4 Feb"; one month named once ("Jan 1 - 7", "1 - 7 Jan").
            let mut t = Buf::<24>::new();
            let (ma, mb) = (
                &MONTHS[usize::from(a.month - 1)][..3],
                &MONTHS[usize::from(b.month - 1)][..3],
            );
            let crosses = b.month != a.month;
            if cx.us {
                t.s(ma).s(b" ").num(u32::from(a.day)).s(b" - ");
                if crosses {
                    t.s(mb).s(b" ");
                }
                t.num(u32::from(b.day));
            } else {
                t.num(u32::from(a.day));
                if crosses {
                    t.s(b" ").s(ma);
                }
                t.s(b" - ").num(u32::from(b.day)).s(b" ").s(mb);
            }
            let tw = cx.fonts.label.width(t.get()) + 28;
            let chip = Rect::new((W - tw) / 2, y - 6, tw, 24);
            kit::key(f, cx, chip, kit::KeyState::On);
            kit::centred(f, chip, chip.y + 17, cx.fonts.label, TEXT, t.get());
        }
    }

    /// The strip's offset now: the pen's drag, or the settling motion's
    /// frame.
    fn strip_offset(&self) -> i16 {
        match self.motion {
            Some(Motion::Slide { from, to, k }) => motion::lerp(from.into(), to.into(), k) as i16,
            _ => self.slide,
        }
    }

    /// One week of the strip from `first`, its left edge at `x0` (left of
    /// the screen while it slides out: every part placed by
    /// [`motion::span`] and [`motion::on_screen`], so what shows is exactly
    /// what a strip drawn further right would show there). The chosen
    /// day's weekday is lit in every week shown, so nothing appears or
    /// vanishes as a week lands.
    #[inline(never)]
    fn week_cells(&mut self, f: &mut Frame, cx: &Ctx, y: u16, first: i32, x0: i32, hits: bool) {
        let lit = in_week(self.day, cx.us);
        for i in 0..7u16 {
            let d = first + i32::from(i);
            let x = x0 + i32::from(i) * 64;
            if x + 64 <= 0 || x >= i32::from(W) {
                continue;
            }
            if hits && let Some((l, w)) = motion::span(x, 64) {
                self.hits.add(Rect::new(l, y, w, 62), Hit::Day(d));
            }
            let name = column_name(i, cx.us);
            let tx = x + i32::from(64u16.saturating_sub(cx.fonts.label.width(name)) / 2);
            if let Some((px, from)) = motion::on_screen(cx.fonts.label, tx, name) {
                kit::text(f, px, y + 14, cx.fonts.label, DIM, &name[from..]);
            }
            if i32::from(i) == lit {
                if let Some((l, w)) = motion::span(x + 10, 44) {
                    kit::key(f, cx, Rect::new(l, y + 18, w, 40), kit::KeyState::On);
                }
            } else if d == cx.now.day {
                self.ring_at(f, cx, x + 12, y + 18);
            }
            let mut b = Buf::<4>::new();
            b.num(u32::from(date::date_of(d).day));
            let tx = x + i32::from(64u16.saturating_sub(cx.fonts.ui.width(b.get())) / 2);
            if let Some((px, from)) = motion::on_screen(cx.fonts.ui, tx, b.get()) {
                kit::text(f, px, y + 45, cx.fonts.ui, TEXT, &b.get()[from..]);
            }
            let (list, n) = self.on(d);
            for (k, a) in list[..n.min(3)].iter().enumerate() {
                if let Some((l, w)) = motion::span(x + 17 + k as i32 * 11, 8) {
                    f.push(rp_app::Cmd::Fill {
                        r: Rect::new(l, y + 59, w, 3),
                        color: self.colour(a),
                    });
                }
            }
        }
    }

    /// Today's ring at `x` (left of the screen while a strip slides: its
    /// four 10 px slices, each placed whole or not at all; a slice left of
    /// the screen lies inside the strip's clipped 16 px margin).
    fn ring_at(&mut self, f: &mut Frame, cx: &Ctx, x: i32, y: u16) {
        if let Ok(x) = u16::try_from(x) {
            kit::dot(f, self.ring, x, y);
            return;
        }
        if self.ring_slices[0].is_none()
            && let (Some(p), Some(ring)) = (cx.pics, self.ring)
        {
            for (s, slot) in self.ring_slices.iter_mut().enumerate() {
                let Some(pic) = p.alloc(10, 40) else { break };
                let _ = p.read(ring, |src| {
                    p.write(pic, |dst| {
                        for row in 0..40 {
                            let from = row * 40 + s * 10;
                            dst[row * 10..row * 10 + 10].copy_from_slice(&src[from..from + 10]);
                        }
                    })
                });
                *slot = Some(pic);
            }
        }
        for (s, pic) in self.ring_slices.iter().enumerate() {
            if let Ok(sx) = u16::try_from(x + s as i32 * 10) {
                kit::dot(f, *pic, sx, y);
            }
        }
    }

    /// The shallower view's cell a zoom between `a` and `b` grows from or
    /// shrinks into, for the chosen day: its day's cell in the month, its
    /// week's row in the month, or its column in the week (the same
    /// geometry `month_view` and `week_view` draw, checked by a test).
    fn zoom_cell(&self, a: View, b: View, us: bool) -> Rect {
        let pair = |x: View, y: View| (a == x && b == y) || (a == y && b == x);
        if pair(View::Week, View::Day) {
            let lens = in_week(self.day, us) as u16;
            let wide = WIDE - 36 - 6 * WEEK_SLIM;
            return Rect::new(
                WEEK_X0 + lens * WEEK_SLIM,
                VIEW_TOP,
                wide,
                BOTTOM - VIEW_TOP,
            );
        }
        let d = date::date_of(self.day);
        let start = date::days_from(date::Date {
            year: d.year,
            month: d.month,
            day: 1,
        });
        let at = (self.day - start + in_week(start, us)) as u16;
        let (row, col) = (at / 7, at % 7);
        let y = MONTH_GRID + row * MONTH_CH;
        if pair(View::Month, View::Week) {
            Rect::new(MARGIN, y, WIDE, MONTH_CH)
        } else {
            Rect::new(MARGIN + col * 64, y, 64, MONTH_CH)
        }
    }

    /// The day before or after (the D-pad): into another week, the strip
    /// slides it in from that side (the motion's whole frame list drawn,
    /// as a drag's).
    fn step_day(&mut self, by: i32, cx: &Ctx) -> Reply {
        let week = |d: i32| d - in_week(d, cx.us);
        let before = week(self.day);
        self.day += by;
        if week(self.day) == before || !matches!(self.view, View::Day | View::Agenda) {
            return Reply::Redraw;
        }
        self.motion = Some(Motion::Slide {
            from: if by < 0 {
                -motion::STRIP_W
            } else {
                motion::STRIP_W
            },
            to: 0,
            k: 1,
        });
        Reply::Animate
    }

    /// Goes to view `to`: between month, week and day as a zoom (motion.rs:
    /// deeper grows out of the chosen day's cell, shallower shrinks back
    /// into it), any other change at once.
    fn go(&mut self, to: View, cx: &Ctx) -> Reply {
        let from = self.view;
        self.view = to;
        self.focus.clear();
        match (motion::level(from), motion::level(to)) {
            (Some(a), Some(b)) if a != b => {
                self.drag = None;
                self.slide = 0;
                // Frame 0 captures the arriving view (motion.rs, pick D).
                self.motion = Some(Motion::Zoom {
                    from,
                    to,
                    cell: self.zoom_cell(from, to, cx.us),
                    k: 0,
                });
                Reply::Animate
            }
            _ => Reply::Redraw,
        }
    }

    /// A zoom's frame (the owner's pick D, 2026-11-23): the arriving view
    /// drawn whole, and what the renderer does with it. Frames 0 and 1
    /// capture it into the renderer's layer B, a half each (the screen stays
    /// as it was, kept in layer A); the next [`motion::FRAMES`] show the shallower view's
    /// picture whole and the deeper one's scaled into the card growing out
    /// of the chosen day's cell (or shrinking back into it), two 16-bit
    /// copies on the 2D engine (`rp_draw::LayerOp`); one frame more lands
    /// the captured view as the screen's, nothing drawn again. A renderer
    /// without layers draws the arriving view: the change at once.
    #[inline(never)]
    fn zoom_frame(&mut self, f: &mut Frame, cx: &Ctx, from: View, to: View, cell: Rect, k: u8) {
        self.scene(f, cx, to);
        let deeper = motion::level(to) > motion::level(from);
        let page = rp_app::CONTENT;
        // Frames 0 and 1 capture (a part each), 2 to FRAMES + 1 show the
        // motion's frames 1 to FRAMES, then the landing.
        let op = if k < 2 {
            rp_app::LayerOp::Capture { part: k }
        } else if k > motion::FRAMES + 1 {
            rp_app::LayerOp::Land
        } else {
            let k = k - 1;
            rp_app::LayerOp::Show {
                card: if deeper {
                    motion::grow(cell, page, k)
                } else {
                    motion::grow(page, cell, k)
                },
                from: page,
                arriving_in_card: deeper,
            }
        };
        f.push(rp_app::Cmd::Layers { op });
    }

    /// A motion's next frame, or its landing.
    fn step_motion(&mut self) -> Reply {
        match self.motion {
            Some(Motion::Slide { from, to, k }) if k < motion::FRAMES => {
                self.motion = Some(Motion::Slide { from, to, k: k + 1 });
                Reply::Animate
            }
            // (A zoom's frame past the last lands the captured view.)
            Some(Motion::Zoom { from, to, cell, k }) if k <= motion::FRAMES + 1 => {
                self.motion = Some(Motion::Zoom {
                    from,
                    to,
                    cell,
                    k: k + 1,
                });
                Reply::Animate
            }
            Some(_) => {
                self.land();
                Reply::Redraw
            }
            None => Reply::Nothing,
        }
    }

    /// A motion under way put at its end (its last frame, or any input
    /// while it moves: the pen never waits for a motion).
    fn land(&mut self) {
        if let Some(Motion::Slide { to, .. }) = self.motion.take() {
            self.day += 7 * motion::weeks(to);
        }
        self.slide = 0;
    }

    /// The pen and the ticks for motion, before anything else sees them:
    /// `Some` when they were the motion's.
    fn motion_event(&mut self, e: Event) -> Option<Reply> {
        if e == Event::Tick {
            return self.motion.map(|_| self.step_motion());
        }
        if self.motion.is_some() {
            self.land();
            if !matches!(e, Event::PenDown { .. }) {
                return None;
            }
        }
        let on_strip = |x: u16, y: u16| {
            matches!(self.view, View::Day | View::Agenda)
                && self.strip_at.is_some_and(|r| r.contains_point(x, y))
        };
        match e {
            Event::PenDown { x, y } => {
                self.drag = on_strip(x, y).then_some((x, x, 0, false));
                None
            }
            Event::PenMove { x, .. } => {
                let (x0, last, _, moving) = self.drag?;
                let dx = i32::from(x) - i32::from(x0);
                let moving = moving || dx.unsigned_abs() >= u32::from(motion::DRAG_START);
                let step = (i32::from(x) - i32::from(last)) as i16;
                self.drag = Some((x0, x, step, moving));
                if !moving {
                    return Some(Reply::Nothing);
                }
                let w = i32::from(motion::STRIP_W);
                self.slide = dx.clamp(-w, w) as i16;
                Some(Reply::Redraw)
            }
            Event::PenUp { x, .. } => {
                let (x0, last, step, moving) = self.drag.take()?;
                if !moving {
                    return None;
                }
                let w = i32::from(motion::STRIP_W);
                let dx = (i32::from(x) - i32::from(x0)).clamp(-w, w) as i16;
                let step = if x == last {
                    step
                } else {
                    (i32::from(x) - i32::from(last)) as i16
                };
                self.slide = 0;
                self.motion = Some(Motion::Slide {
                    from: dx,
                    to: motion::settle(dx, step),
                    k: 1,
                });
                Some(Reply::Animate)
            }
            _ => None,
        }
    }

    /// A tab view whole: its title (or the day's board), the view, the
    /// views along the foot.
    #[inline(never)]
    fn scene(&mut self, f: &mut Frame, cx: &Ctx, shown: View) {
        if shown == View::Home {
            self.home(f, cx);
        } else {
            let under = self.title(f, cx);
            kit::key(f, cx, CALENDARS_KEY, kit::KeyState::Rest);
            kit::centred(
                f,
                CALENDARS_KEY,
                CALENDARS_KEY.y + 21,
                cx.fonts.label,
                TEXT,
                b"CALENDARS",
            );
            let y = under + 6;
            match shown {
                View::Day => {
                    self.strip(f, cx, y);
                    self.day_view(f, cx, y + 70);
                }
                View::Week => self.week_view(f, cx, y),
                View::Month => self.month_view(f, cx, y),
                _ => {
                    self.strip(f, cx, y);
                    // The way into the search: a field over the agenda.
                    let field = Rect::new(MARGIN, y + 68, WIDE, 34);
                    kit::panel(f, cx, field);
                    kit::text(
                        f,
                        field.x + 14,
                        field.y + 23,
                        cx.fonts.ui,
                        DIM,
                        b"Search every appointment",
                    );
                    self.hits.add(field, Hit::Find);
                    self.agenda(f, cx, y + 108);
                }
            }
        }
        // The views along the foot (as Weather's tabs), each a target.
        let chosen = TABS.iter().position(|v| *v == shown).unwrap_or(0);
        kit::foot_tabs(f, cx, &LABELS, chosen, tint::CALENDAR);
        for (k, v) in TABS.into_iter().enumerate() {
            self.hits.add(kit::foot_tab(TABS.len(), k), Hit::View(v));
        }
    }

    /// The forecast for `day`, if the device holds it.
    fn weather(cx: &Ctx, day: i32) -> Option<rp_app::DayWeather> {
        cx.forecast.iter().copied().find(|w| w.day == day)
    }

    /// The day's board (design-language.md version 3, "Every app's home
    /// has"): the chosen day as a hero with its weather and its hours as
    /// a ribbon; what is next with a countdown ring and what can be done
    /// about it; the week's load; what comes later. Everything drawn is
    /// a target.
    /// The colours of a day's first four appointments (the board's week
    /// load). Its own frame: a second day's list beside the board's own
    /// made the board a 5.1 KB frame, over the stack rule.
    #[inline(never)]
    fn colours_on(&self, day: i32) -> ([u32; 4], usize) {
        let (list, n) = self.on(day);
        let mut out = [0; 4];
        let n = n.min(4);
        for (o, a) in out.iter_mut().zip(&list[..n]) {
            *o = self.colour(a);
        }
        (out, n)
    }

    // Its own frame: drawn into `draw`, the views together were a 5.2 KB
    // frame, over the stack rule.
    #[inline(never)]
    fn home(&mut self, f: &mut Frame, cx: &Ctx) {
        let t = tint::CALENDAR;
        let (ui, label) = (cx.fonts.ui, cx.fonts.label);
        let today = cx.now.day;
        let (list, n) = self.on(self.day);
        let d = date::date_of(self.day);
        // The hero.
        let hero = Rect::new(MARGIN, 44, WIDE, 154);
        kit::card(
            f,
            cx,
            hero,
            if self.day == today {
                b"TODAY"
            } else if self.day == today + 1 {
                b"TOMORROW"
            } else {
                WEEKDAYS[usize::from(date::weekday(self.day))]
            },
            t,
        );
        let mut num = Buf::<4>::new();
        num.num(u32::from(d.day));
        kit::text(f, 30, 138, cx.fonts.big, TEXT, num.get());
        let x = 38 + cx.fonts.big.width(num.get());
        let mut name = Buf::<24>::new();
        name.s(WEEKDAY_NAMES[usize::from(date::weekday(self.day))]);
        kit::text(f, x, 100, cx.fonts.title, TEXT, name.get());
        let mut sub = Buf::<40>::new();
        sub.s(MONTHS[usize::from(d.month - 1)])
            .s(b" ")
            .num(d.year as u32)
            .s(b" \x80 ");
        match n {
            0 => sub.s(b"nothing planned"),
            n => sub.num(n as u32).s(b" planned"),
        };
        kit::text(f, x, 128, ui, DIM, sub.get());
        if let Some(w) = Self::weather(cx, self.day) {
            if let Some(p) = w.icon {
                f.push(rp_app::Cmd::Pic {
                    x: 384,
                    y: 56,
                    pic: p,
                });
            }
            let mut wx = Buf::<16>::new();
            wx.num(w.high.unsigned_abs().into())
                .s(b"\x7F / ")
                .num(w.low.unsigned_abs().into())
                .s(b"\x7F");
            kit::right(f, 448, 128, ui, DIM, wx.get());
        }
        // The day's ribbon, 6 AM to 10 PM: each appointment where it
        // falls (a tap opens it), now marked.
        let at = |minute: u16| 32 + (u32::from(minute.clamp(360, 1320) - 360) * 416 / 960) as u16;
        f.push(rp_app::Cmd::Fill {
            r: Rect::new(32, 152, 416, 14),
            color: kit::alpha(TEXT, 0x26),
        });
        for a in list[..n].iter().filter(|a| a.len != 0) {
            let (x0, x1) = (at(a.start), at(a.start.saturating_add(a.len)));
            let r = Rect::new(x0, 152, (x1 - x0).max(8), 14);
            kit::round_rect(f, cx, r, self.colour(a));
            if !a.task {
                self.hits.add(Rect::new(r.x, 144, r.w, 30), Hit::Appt(a.id));
            }
        }
        let (from, to) = (
            rp_app::clock::hour(6, cx.clock12),
            rp_app::clock::hour(22, cx.clock12),
        );
        kit::text(f, 32, 188, label, DIM, from.get());
        kit::right(f, 448, 188, label, DIM, to.get());
        if self.day == today && (360..1320).contains(&cx.now.minute) {
            let x = at(cx.now.minute);
            f.push(rp_app::Cmd::Fill {
                r: Rect::new(x, 146, 2, 26),
                color: TEXT,
            });
            let lx = x.clamp(92, 388);
            kit::centred(f, Rect::new(lx - 30, 172, 60, 18), 188, label, TEXT, b"NOW");
        }

        // What is next (today: from now; another day: its first).
        let card = Rect::new(MARGIN, 206, WIDE, 124);
        kit::card(f, cx, card, b"", t);
        let next = list[..n].iter().find(|a| {
            !a.task && a.len != 0 && (self.day != today || a.start + a.len > cx.now.minute)
        });
        let (part, big, small): (u16, Buf<4>, &[u8]) = match next {
            Some(a) if self.day == today => {
                let mut b = Buf::<4>::new();
                if a.start <= cx.now.minute {
                    b.s(b"0");
                    (256, b, b"NOW")
                } else {
                    let m = a.start - cx.now.minute;
                    if m < 100 {
                        b.num(u32::from(m));
                        (((60u16.saturating_sub(m)) * 256 / 60).max(12), b, b"MIN")
                    } else {
                        b.num(u32::from((m + 30) / 60));
                        (12, b, b"HR")
                    }
                }
            }
            Some(a) => {
                // Another day: the hour it starts, the ring the day so far;
                // under it AM or PM, or on the 24-hour clock its minutes
                // ("15" over ":30").
                let mut b = Buf::<4>::new();
                let h = a.start / 60 % 24;
                let under: &[u8] = if cx.clock12 {
                    b.num(u32::from((h + 11) % 12 + 1));
                    if h < 12 { b"AM" } else { b"PM" }
                } else {
                    b.num(u32::from(h));
                    let m = usize::from(a.start % 60);
                    &MINUTES[m * 3..m * 3 + 3]
                };
                ((u32::from(a.start) * 256 / 1440) as u16, b, under)
            }
            None => (0, Buf::<4>::new(), b""),
        };
        if self
            .gauge
            .is_none_or(|(_, p, l)| p != part || l != cx.look.light)
            && let Some(pics) = cx.pics
        {
            if let Some((old, _, _)) = self.gauge.take() {
                pics.free(old);
            }
            self.gauge = kit::ring(pics, 96, 9, part, kit::painted(cx, t), kit::track(cx))
                .map(|p| (p, part, cx.look.light));
        }
        if let Some((p, _, _)) = self.gauge {
            f.push(rp_app::Cmd::Pic {
                x: 28,
                y: 220,
                pic: p,
            });
        }
        let dial = Rect::new(28, 220, 96, 96);
        match next {
            Some(a) => {
                kit::centred(f, dial, 278, cx.fonts.num, TEXT, big.get());
                kit::centred(f, dial, 296, label, DIM, small);
                kit::text(
                    f,
                    140,
                    234,
                    label,
                    t,
                    if self.day == today {
                        b"NEXT UP"
                    } else {
                        b"FIRST UP"
                    },
                );
                kit::fitted(f, 140, 264, cx.fonts.title, TEXT, a.title.get(), 310);
                let mut when = Buf::<48>::new();
                when.s(clock(a.start, cx.clock12).get());
                if !a.place.get().is_empty() {
                    when.s(b" \x80 ").s(a.place.get());
                }
                kit::fitted(f, 140, 286, ui, DIM, when.get(), 310);
                let open = Rect::new(140, 294, 84, 28);
                kit::gel(f, cx, open, b"OPEN", true);
                self.hits
                    .add(Rect::new(card.x, card.y, card.w, 84), Hit::Appt(a.id));
                self.hits.add(open, Hit::Appt(a.id));
                let day = Rect::new(232, 294, 108, 28);
                kit::gel(f, cx, day, b"DAY VIEW", false);
                self.hits.add(day, Hit::View(View::Day));
            }
            None => {
                kit::centred(f, dial, 274, label, DIM, b"FREE");
                kit::text(f, 140, 234, label, t, b"NEXT UP");
                kit::text(
                    f,
                    140,
                    264,
                    cx.fonts.title,
                    TEXT,
                    if n == 0 {
                        b"A free day"
                    } else {
                        b"Nothing more"
                    },
                );
                kit::text(f, 140, 286, ui, DIM, b"Add something to it");
                let new = Rect::new(140, 294, 84, 28);
                kit::gel(f, cx, new, b"NEW", true);
                self.hits.add(new, Hit::New);
            }
        }

        // The week: how full each day is; a tap shows that day's board.
        kit::section(f, cx, 336, b"THIS WEEK", t);
        let first = self.day - in_week(self.day, cx.us);
        for i in 0..7u16 {
            let day = first + i32::from(i);
            let r = Rect::new(MARGIN + i * 64, 358, 60, 86);
            self.hits.add(r, Hit::Day(day));
            if day == self.day {
                kit::key(f, cx, r, kit::KeyState::On);
            } else {
                kit::panel(f, cx, r);
            }
            kit::centred(
                f,
                r,
                r.y + 20,
                label,
                if day == self.day {
                    TEXT
                } else if day == today {
                    t
                } else {
                    DIM
                },
                column_name(i, cx.us),
            );
            let mut b = Buf::<4>::new();
            b.num(u32::from(date::date_of(day).day));
            kit::centred(f, r, r.y + 46, ui, TEXT, b.get());
            let (load, m) = self.colours_on(day);
            for (k, &c) in load[..m].iter().enumerate() {
                kit::round_rect(
                    f,
                    cx,
                    Rect::new(r.x + 12, r.y + 54 + k as u16 * 7, 36, 5),
                    c,
                );
            }
        }

        // What else the day holds: two rows, then the way to the rest.
        kit::section(
            f,
            cx,
            450,
            if self.day == today {
                b"LATER TODAY"
            } else {
                b"THAT DAY"
            },
            t,
        );
        let next_id = next.map(|a| a.id);
        let rest = list[..n]
            .iter()
            .filter(|a| Some(a.id) != next_id)
            .filter(|a| self.day != today || a.len == 0 || a.start + a.len > cx.now.minute);
        let more = rest.clone().count();
        for (k, a) in rest.take(2).enumerate() {
            let r = Rect::new(MARGIN, 472 + k as u16 * 42, WIDE, 38);
            kit::panel(f, cx, r);
            kit::colour_bar(f, r, self.colour(a));
            let time = if a.len == 0 {
                rp_app::Text::EMPTY
            } else {
                clock(a.start, cx.clock12)
            };
            kit::text(
                f,
                36,
                r.y + 26,
                ui,
                DIM,
                if a.len == 0 { b"All day" } else { time.get() },
            );
            let room = if k == 1 && more > 2 {
                WIDE - 220
            } else {
                WIDE - 136
            };
            kit::fitted(f, 136, r.y + 26, ui, TEXT, a.title.get(), room);
            if !a.task {
                self.hits.add(r, Hit::Appt(a.id));
            }
            if k == 1 && more > 2 {
                let mut b = Buf::<12>::new();
                b.s(b"+").num((more - 2) as u32).s(b" MORE");
                kit::right(f, r.x + r.w - 14, r.y + 25, label, t, b.get());
                self.hits.add(
                    Rect::new(r.x + r.w - 90, r.y, 90, r.h),
                    Hit::View(View::Agenda),
                );
            }
        }
        if more == 0 {
            let r = Rect::new(MARGIN, 472, WIDE, 38);
            kit::panel(f, cx, r);
            kit::text(f, 36, r.y + 26, ui, DIM, b"Nothing else");
        }
    }

    #[inline(never)]
    fn agenda(&mut self, f: &mut Frame, cx: &Ctx, top: u16) {
        self.n_rows = 0;
        let mut y = top;
        let bottom = BOTTOM;
        let mut day = self.day;
        while y + 82 < bottom && day < self.day + 30 {
            let (list, n) = self.on(day);
            if n > 0 {
                let mut l = Buf::<32>::new();
                if day == cx.now.day {
                    l.s(b"TODAY");
                } else if day == cx.now.day + 1 {
                    l.s(b"TOMORROW");
                } else {
                    l.s(WEEKDAYS[usize::from(date::weekday(day))])
                        .s(b" ")
                        .date(day, cx.us);
                }
                let c = if day == cx.now.day {
                    tint::CALENDAR
                } else {
                    DIM
                };
                kit::text(f, MARGIN + 4, y + 17, cx.fonts.label, c, l.get());
                if let Some(w) = Self::weather(cx, day) {
                    let mut t = Buf::<16>::new();
                    t.num(w.high.unsigned_abs().into())
                        .s(b"\x7F / ")
                        .num(w.low.unsigned_abs().into())
                        .s(b"\x7F");
                    kit::right(f, W - MARGIN - 4, y + 18, cx.fonts.ui, DIM, t.get());
                    if let Some(icon) = w.icon {
                        let tw = cx.fonts.ui.width(t.get());
                        f.push(rp_app::Cmd::Pic {
                            x: W - MARGIN - 4 - tw - icon.w - 6,
                            y: (y + 13) as i16 - (icon.h / 2) as i16,
                            pic: icon,
                        });
                    }
                }
                y += 26;
                for a in &list[..n] {
                    if y + 52 > bottom {
                        return;
                    }
                    let r = Rect::new(MARGIN, y, WIDE, 52);
                    if self.n_rows < self.rows_at.len() {
                        self.rows_at[self.n_rows] = (r, a.id, a.task);
                        self.n_rows += 1;
                    }
                    if !a.task {
                        self.hits.add(r, Hit::Appt(a.id));
                    }
                    kit::panel(f, cx, r);
                    kit::colour_bar(f, r, self.colour(a));
                    if a.task {
                        kit::text(f, MARGIN + 20, y + 32, cx.fonts.label, tint::TASKS, b"TASK");
                    } else if a.len == 0 {
                        kit::text(f, MARGIN + 20, y + 32, cx.fonts.ui, DIM, b"All day");
                    } else {
                        kit::text(
                            f,
                            MARGIN + 20,
                            y + 22,
                            cx.fonts.ui,
                            TEXT,
                            clock(a.start, cx.clock12).get(),
                        );
                        kit::text(
                            f,
                            MARGIN + 20,
                            y + 44,
                            cx.fonts.ui,
                            DIM,
                            clock(a.start + a.len, cx.clock12).get(),
                        );
                    }
                    kit::fitted(
                        f,
                        MARGIN + 122,
                        y + 22,
                        cx.fonts.ui,
                        TEXT,
                        a.title.get(),
                        WIDE - 130,
                    );
                    if !a.place.get().is_empty() {
                        kit::fitted(
                            f,
                            MARGIN + 122,
                            y + 44,
                            cx.fonts.ui,
                            DIM,
                            a.place.get(),
                            WIDE - 130,
                        );
                    }
                    y += 56;
                }
                y += 4;
            }
            day += 1;
        }
    }

    #[inline(never)]
    fn day_view(&mut self, f: &mut Frame, cx: &Ctx, top: u16) {
        let (list, n) = self.on(self.day);
        let mut y = top;
        // All-day items in a strip.
        for a in list[..n].iter().filter(|a| a.len == 0) {
            let r = Rect::new(MARGIN + 52, y, WIDE - 52, 30);
            if !a.task {
                self.hits.add(r, Hit::Appt(a.id));
            }
            kit::tinted(f, cx, r, self.colour(a), 0x70);
            kit::fitted(
                f,
                r.x + 12,
                y + 21,
                cx.fonts.ui,
                TEXT,
                a.title.get(),
                r.w - 20,
            );
            y += 34;
        }
        let timed = || list[..n].iter().filter(|a| a.len != 0);
        let first_h = timed().map(|a| a.start / 60).min().unwrap_or(8);
        let last_h = timed()
            .map(|a| (a.start + a.len).div_ceil(60))
            .max()
            .unwrap_or(18)
            .max(first_h + 4);
        // The day's span fits the screen down to 30 px an hour; past that
        // the rest folds into "Free until later" (a long day scrolls).
        let span = last_h - first_h;
        let per = ((BOTTOM - y) / span).clamp(30, 60);
        let hours = ((BOTTOM - y) / per).min(span);
        for h in 0..hours {
            let hy = y + h * per;
            kit::right(
                f,
                MARGIN + 46,
                hy + 6,
                cx.fonts.label,
                DIM,
                rp_app::clock::hour(first_h + h, cx.clock12).get(),
            );
            f.push(rp_app::Cmd::Fill {
                r: Rect::new(MARGIN + 52, hy, WIDE - 52, 1),
                color: kit::alpha(TEXT, 0x30),
            });
        }
        let end = first_h + hours;
        if last_h > end {
            let r = Rect::new(MARGIN + 52, y + hours * per - 30, WIDE - 52, 28);
            kit::text(f, r.x + 8, r.y + 20, cx.fonts.ui, DIM, b"Free until later");
        }
        // Blocks sized to their length; overlaps side by side.
        let items = timed();
        let all: [Option<&Appt>; 16] = {
            let mut a = [None; 16];
            for (i, x) in items.enumerate().take(16) {
                a[i] = Some(x);
            }
            a
        };
        for (i, a) in all.iter().enumerate() {
            let Some(a) = a else { continue };
            if a.start / 60 >= end {
                continue;
            }
            let lanes = all
                .iter()
                .flatten()
                .filter(|b| b.start < a.start + a.len && a.start < b.start + b.len)
                .count() as u16;
            let lane = all[..i]
                .iter()
                .flatten()
                .filter(|b| b.start < a.start + a.len && a.start < b.start + b.len)
                .count() as u16;
            let x0 = MARGIN + 56;
            let wide = (WIDE - 60) / lanes.max(1);
            let by = y + (a.start - first_h * 60) * per / 60;
            let bh = (a.len * per / 60).max(28).min(BOTTOM - by);
            let r = Rect::new(x0 + lane * wide, by + 1, wide - 4, bh - 2);
            if !a.task {
                self.hits.add(r, Hit::Appt(a.id));
            }
            kit::tinted(f, cx, r, self.colour(a), 0x80);
            kit::colour_bar(f, r, self.colour(a));
            kit::fitted(
                f,
                r.x + 16,
                r.y + 20,
                cx.fonts.ui,
                TEXT,
                a.title.get(),
                r.w - 22,
            );
            if bh > 48 {
                let mut t = Buf::<24>::new();
                t.s(clock(a.start, cx.clock12).get()).s(b" - ").s(clock(
                    a.start + a.len,
                    cx.clock12,
                )
                .get());
                kit::fitted(f, r.x + 16, r.y + 42, cx.fonts.ui, DIM, t.get(), r.w - 22);
            }
        }
        // The now line.
        if self.day == cx.now.day {
            let m = cx.now.minute;
            if m >= first_h * 60 && m < end * 60 {
                let ny = y + (m - first_h * 60) * per / 60;
                f.push(rp_app::Cmd::Fill {
                    r: Rect::new(MARGIN + 48, ny, WIDE - 48, 2),
                    color: tint::CALENDAR,
                });
                f.push(rp_app::Cmd::Fill {
                    r: Rect::new(MARGIN + 44, ny - 3, 8, 8),
                    color: tint::CALENDAR,
                });
            }
        }
    }

    /// The week with a lens on the chosen day: its column is wide enough
    /// to read (titles and times), the other six are slim, their
    /// appointments bars of colour: the week's shape at a glance. Seven
    /// equal columns on 480 pixels (Windows Mobile's, and the first
    /// version here) are 58 px each, where a title is three letters
    /// ("Lunc.."). A tap on a slim day moves the lens to it; on the wide
    /// one, opens its day.
    #[inline(never)]
    fn week_view(&mut self, f: &mut Frame, cx: &Ctx, top: u16) {
        const SLIM: u16 = 40;
        let first = self.day - in_week(self.day, cx.us);
        let x0 = MARGIN + 36;
        let lens = (self.day - first) as u16;
        let wide = WIDE - 36 - 6 * SLIM;
        // Column `i`: its left edge and width.
        let col = |i: u16| {
            let x = x0 + i * SLIM + if i > lens { wide - SLIM } else { 0 };
            (x, if i == lens { wide } else { SLIM })
        };
        let (h0, h1) = (8u16, 21u16);
        let y0 = top + 46;
        let per = (BOTTOM - y0) / (h1 - h0);
        // The lens's column is a pane of the window's glass.
        let (lx, lw) = col(lens);
        kit::panel(f, cx, Rect::new(lx, top, lw, y0 + per * (h1 - h0) - top));
        for i in 0..7u16 {
            let d = first + i32::from(i);
            let (x, w) = col(i);
            let c = Rect::new(x, top, w, 40);
            // A slim column moves the lens to its day; the wide one opens
            // its day (its blocks, added later, open themselves).
            self.hits.add(
                Rect::new(x, top, w, BOTTOM - top),
                if i == lens {
                    Hit::OpenDay(d)
                } else {
                    Hit::Day(d)
                },
            );
            let ink = if d == cx.now.day { tint::CALENDAR } else { DIM };
            let name = column_name(i, cx.us);
            // The slim columns: the day's first letter.
            kit::centred(
                f,
                c,
                top + 14,
                cx.fonts.label,
                ink,
                if i == lens { name } else { &name[..1] },
            );
            let mut b = Buf::<4>::new();
            b.num(u32::from(date::date_of(d).day));
            kit::centred(
                f,
                c,
                top + 36,
                cx.fonts.ui,
                if d == cx.now.day {
                    tint::CALENDAR
                } else {
                    TEXT
                },
                b.get(),
            );
        }
        for h in 0..(h1 - h0) {
            let hy = y0 + h * per;
            let mut b = Buf::<4>::new();
            // The hour as the clock chosen counts it (R383): 3, or 15.
            let hh = h0 + h;
            b.num(u32::from(if cx.clock12 { (hh + 11) % 12 + 1 } else { hh }));
            kit::right(f, x0 - 4, hy + 6, cx.fonts.label, DIM, b.get());
            f.push(rp_app::Cmd::Fill {
                r: Rect::new(x0, hy, WIDE - 36, 1),
                color: kit::alpha(TEXT, 0x28),
            });
        }
        for i in 0..7u16 {
            let d = first + i32::from(i);
            let (cx0, cw) = col(i);
            if d == cx.now.day && i != lens {
                f.push(rp_app::Cmd::Outline {
                    r: Rect::new(cx0, y0, cw, per * (h1 - h0)),
                    color: kit::alpha(tint::CALENDAR, 0xA0),
                });
            }
            let (list, n) = self.on(d);
            let timed = &list[..n];
            for (k, a) in timed.iter().enumerate().filter(|(_, a)| a.len != 0) {
                if a.start < h0 * 60 || a.start >= h1 * 60 {
                    continue;
                }
                let over =
                    |b: &Appt| b.len != 0 && b.start < a.start + a.len && a.start < b.start + b.len;
                let lanes = timed.iter().filter(|b| over(b)).count().max(1) as u16;
                let lane = timed[..k].iter().filter(|b| over(b)).count() as u16;
                let lw = (cw - 4) / lanes;
                let by = y0 + (a.start - h0 * 60) * per / 60;
                // (The glass needs its two corners' height: 24.)
                let bh = (a.len * per / 60).max(if i == lens { 30 } else { 8 });
                let r = Rect::new(cx0 + 2 + lane * lw, by + 1, lw - 2, bh - 2);
                if i != lens {
                    // A bar of its calendar's colour.
                    f.push(rp_app::Cmd::Fill {
                        r,
                        color: self.colour(a) | 0xFF00_0000,
                    });
                    continue;
                }
                if !a.task {
                    self.hits.add(r, Hit::Appt(a.id));
                }
                kit::tinted(f, cx, r, self.colour(a), 0x40);
                // Its title, and its time where the block has the room
                // (and no neighbour beside it).
                kit::fitted(
                    f,
                    r.x + 14,
                    r.y + 17,
                    cx.fonts.ui,
                    TEXT,
                    a.title.get(),
                    r.w.saturating_sub(18),
                );
                if r.h >= 40 {
                    kit::fitted(
                        f,
                        r.x + 14,
                        r.y + 35,
                        cx.fonts.label,
                        DIM,
                        clock(a.start, cx.clock12).get(),
                        r.w.saturating_sub(18),
                    );
                }
            }
        }
    }

    /// A month cell's bars: its day's first three, in their colours (its
    /// own function: with the day's list in the month view's frame beside
    /// the chosen day's, the frame was 5,016 bytes, past the stack rule).
    #[inline(never)]
    fn day_bars(&self, f: &mut Frame, day: i32, x: u16, cy: u16) {
        let (list, n) = self.on(day);
        for (k, a) in list[..n.min(3)].iter().enumerate() {
            f.push(rp_app::Cmd::Fill {
                r: Rect::new(x + 8, cy + 25 + k as u16 * 6, 46, 4),
                color: self.colour(a),
            });
        }
    }

    #[inline(never)]
    fn month_view(&mut self, f: &mut Frame, cx: &Ctx, top: u16) {
        let d = date::date_of(self.day);
        let start = date::days_from(date::Date {
            year: d.year,
            month: d.month,
            day: 1,
        });
        let lead = in_week(start, cx.us);
        let mut y = top;
        let mut name = Buf::<24>::new();
        name.s(MONTHS[usize::from(d.month - 1)])
            .s(b" ")
            .num(d.year as u32);
        kit::text(f, MARGIN + 4, y + 20, cx.fonts.ui, TEXT, name.get());
        y += 28;
        for (i, w) in (0..7u16).map(|i| column_name(i, cx.us)).enumerate() {
            kit::centred(
                f,
                Rect::new(MARGIN + i as u16 * 64, y, 64, 18),
                y + 14,
                cx.fonts.label,
                DIM,
                w,
            );
        }
        y += 20;
        let rows = ((lead + i32::from(date::days_in_month(d.year, d.month)) + 6) / 7) as u16;
        let ch = 46u16;
        for r in 0..rows {
            for c in 0..7u16 {
                let day = start - lead + i32::from(r * 7 + c);
                let x = MARGIN + c * 64;
                let cy = y + r * ch;
                let inside = date::date_of(day).month == d.month;
                self.hits.add(Rect::new(x, cy, 64, ch), Hit::Day(day));
                if day == self.day {
                    kit::key(
                        f,
                        cx,
                        Rect::new(x + 2, cy + 1, 60, ch - 2),
                        kit::KeyState::On,
                    );
                } else if day == cx.now.day {
                    f.push(rp_app::Cmd::Outline {
                        r: Rect::new(x + 2, cy + 1, 60, ch - 2),
                        color: tint::CALENDAR,
                    });
                }
                let mut b = Buf::<4>::new();
                b.num(u32::from(date::date_of(day).day));
                let col = if inside { TEXT } else { kit::alpha(DIM, 0x70) };
                kit::text(f, x + 8, cy + 19, cx.fonts.ui, col, b.get());
                self.day_bars(f, day, x, cy);
            }
        }
        y += rows * ch + 6;
        let (list, n) = self.on(self.day);
        for a in &list[..n] {
            if y + 40 > BOTTOM {
                break;
            }
            let r = Rect::new(MARGIN, y, WIDE, 38);
            if !a.task {
                self.hits.add(r, Hit::Appt(a.id));
            }
            kit::panel(f, cx, r);
            kit::colour_bar(f, r, self.colour(a));
            let t = if a.len == 0 {
                rp_app::Text::EMPTY
            } else {
                clock(a.start, cx.clock12)
            };
            kit::text(
                f,
                MARGIN + 20,
                y + 26,
                cx.fonts.ui,
                DIM,
                if a.len == 0 { b"All day" } else { t.get() },
            );
            kit::fitted(
                f,
                MARGIN + 122,
                y + 26,
                cx.fonts.ui,
                TEXT,
                a.title.get(),
                WIDE - 130,
            );
            y += 42;
        }
    }

    /// The quick-add sheet: the line with the understood words in the
    /// tint, and the preview card of what will be added.
    #[inline(never)]
    fn new_sheet(&self, f: &mut Frame, cx: &Ctx) {
        f.push(rp_app::Cmd::Fill {
            r: Rect::new(0, 36, W, 356),
            color: 0x8000_0000,
        });
        let sheet = Rect::new(8, 120, W - 16, 266);
        kit::panel(f, cx, sheet);
        kit::text(
            f,
            28,
            150,
            cx.fonts.label,
            tint::CALENDAR,
            b"NEW APPOINTMENT",
        );
        let field = Rect::new(24, 162, W - 48, 44);
        kit::panel(f, cx, field);
        f.push(rp_app::Cmd::Outline {
            r: field,
            color: kit::FOCUS,
        });
        let line = &self.line[..self.line_n];
        let mut title = [0u8; rp_pim::quick::TITLE_MAX];
        let (q, tn) = rp_pim::quick::parse(line, cx.now.day, cx.us, &mut title);
        // Each word: in the tint when the parser took it (it isn't in the
        // title, in order), else plain.
        let mut x = field.x + 12;
        let mut ti = 0;
        for w in line.split(|&b| b == b' ').filter(|w| !w.is_empty()) {
            let in_title = title[ti..tn].starts_with(w);
            if in_title {
                ti = (ti + w.len() + 1).min(tn);
            }
            let c = if in_title { TEXT } else { tint::CALENDAR };
            kit::text(f, x, field.y + 29, cx.fonts.ui, c, w);
            x += cx.fonts.ui.width(w) + cx.fonts.ui.width(b" ");
        }
        f.push(rp_app::Cmd::Fill {
            r: Rect::new(x - 2, field.y + 10, 2, 24),
            color: kit::FOCUS,
        });
        // The preview card.
        let card = Rect::new(24, 216, W - 48, 96);
        kit::tinted(f, cx, card, self.cals[0].colour, 0x50);
        kit::colour_bar(f, card, self.cals[0].colour);
        kit::fitted(
            f,
            card.x + 20,
            card.y + 28,
            cx.fonts.title,
            TEXT,
            &title[..tn],
            card.w - 30,
        );
        let day = q.day.unwrap_or(cx.now.day);
        let mut when = Buf::<48>::new();
        when.s(WEEKDAYS[usize::from(date::weekday(day))])
            .s(b" ")
            .date(day, cx.us);
        if let Some(t) = q.time {
            let len = q.minutes.unwrap_or(60);
            when.s(b"  ")
                .s(clock(t, cx.clock12).get())
                .s(b" - ")
                .s(clock(t + len, cx.clock12).get());
        } else {
            when.s(b"  All day");
        }
        kit::text(f, card.x + 20, card.y + 58, cx.fonts.ui, TEXT, when.get());
        let mut cal = Buf::<48>::new();
        cal.s(self.cals[0].name.get()).s(b" \x80 alert at its time");
        kit::text(f, card.x + 20, card.y + 82, cx.fonts.ui, DIM, cal.get());
        let more = Rect::new(24, 322, W - 48, 44);
        kit::panel(f, cx, more);
        kit::text(
            f,
            more.x + 14,
            more.y + 29,
            cx.fonts.ui,
            TEXT,
            b"Repeat, alerts, calendar, notes",
        );
        kit::right(f, more.right() - 14, more.y + 29, cx.fonts.ui, DIM, b">");
    }
}

/// The calendars a first opening shows (the store's own come with the
/// records).
pub static FIRST_CALS: [Cal; 1] = [Cal {
    name: b"Calendar",
    colour: kit::IDENTITY[0],
}];

/// Calendar as it first opens on `today`: the program's entry and the PC's
/// model both make it here, so the two agree.
#[must_use]
pub fn opened(today: i32) -> Calendar {
    Calendar::new(&FIRST_CALS, today)
}

/// [`opened`] made straight into the program's slot.
pub fn opened_in(slot: &mut core::mem::MaybeUninit<Calendar>, today: i32) -> &mut Calendar {
    Calendar::new_in(slot, &FIRST_CALS, today)
}

impl Program for Calendar {
    fn accent(&self) -> Option<rp_app::Argb> {
        Some(rp_app::kit::tint::CALENDAR)
    }

    fn title(&self) -> &'static str {
        "Calendar"
    }

    fn softkeys(&self) -> (&'static str, &'static str) {
        match self.view {
            _ if self.pad.is_some() => ("Cancel", "Set"),
            View::New => ("Cancel", "Add"),
            View::Calendars | View::Files => ("Back", ""),
            View::Zones => ("Back", ""),
            View::Find => ("Back", "Find"),
            View::Edit if self.edit.is_some_and(|e| e.read_only) => ("Back", ""),
            View::Edit => ("Cancel", "Done"),
            _ => ("Today", "New"),
        }
    }

    fn draw(&mut self, f: &mut Frame, cx: &Ctx) {
        self.paint(cx);
        self.ensure_birthdays(cx);
        self.refresh(cx);
        self.hits.clear();
        if let Some((which, pad)) = &self.pad {
            pad.draw(f, cx, if *which == 0 { b"STARTS" } else { b"ALERT" });
            return;
        }
        if self.view == View::Edit {
            self.title(f, cx);
            self.edit_view(f, cx);
            return;
        }
        if self.view == View::Calendars {
            let under = self.title(f, cx);
            self.calendars_view(f, cx, under + 4);
            self.focus.ring(f, self.targets());
            return;
        }
        if self.view == View::Files {
            self.files_view(f, cx);
            self.focus.ring(f, self.targets());
            return;
        }
        if self.view == View::Zones {
            self.zones_view(f, cx);
            self.focus.ring(f, self.targets());
            return;
        }
        if self.view == View::Find {
            self.find_view(f, cx);
            self.focus.ring(f, self.targets());
            return;
        }
        // The quick-add sheet rises over the view it came from.
        let shown = if self.view == View::New {
            self.under
        } else {
            self.view
        };
        if let Some(Motion::Zoom { from, to, cell, k }) = self.motion {
            self.zoom_frame(f, cx, from, to, cell, k);
        } else {
            self.scene(f, cx, shown);
        }
        if self.view == View::New {
            self.new_sheet(f, cx);
        }
        self.focus.ring(f, self.targets());
    }

    fn event(&mut self, e: Event, cx: &Ctx) -> Reply {
        // The files' and zones' screens: the D-pad walks their rows, the
        // centre takes one; the rest is theirs.
        if matches!(self.view, View::Files | View::Zones) {
            if let Event::Key(k @ (Key::Up | Key::Down | Key::Left | Key::Right)) = e {
                let mut focus = self.focus;
                let moved = focus.step(k, self.targets());
                self.focus = focus;
                if moved {
                    return Reply::Redraw;
                }
            }
            if e == Event::Key(Key::Enter)
                && let Some((x, y)) = self.focus.point()
            {
                return self.files_event(Event::PenUp { x, y }, cx);
            }
            if matches!(e, Event::PenDown { .. }) {
                self.focus.clear();
            }
            return self.files_event(e, cx);
        }
        // The time pad takes everything while it is up.
        if let Some((which, mut pad)) = self.pad {
            use rp_app::timepad::Answer;
            return match pad.event(e) {
                Answer::Set(v) => {
                    self.pad = None;
                    if let Some(ed) = self.edit.as_mut() {
                        match (which, v) {
                            (0, Some(m)) => {
                                ed.start = m as u16;
                                ed.all_day = false;
                            }
                            (0, None) => ed.all_day = true,
                            (_, lead) => ed.set_alert(lead),
                        }
                    }
                    Reply::Redraw
                }
                Answer::Cancel => {
                    self.pad = None;
                    Reply::Redraw
                }
                Answer::Redraw => {
                    self.pad = Some((which, pad));
                    Reply::Redraw
                }
                Answer::Nothing => Reply::Nothing,
            };
        }
        // The week strip's drag and the motions' frames (motion.rs).
        if let Some(r) = self.motion_event(e) {
            return r;
        }
        // The stylus takes over from the D-pad (the centre key's tap is
        // a lift only).
        if matches!(e, Event::PenDown { .. }) {
            self.focus.clear();
            if let Some(ed) = self.edit.as_mut() {
                ed.row = None;
            }
        }
        if self.view == View::Edit {
            match e {
                Event::PenUp { x, y } => return self.edit_tap(cx, x, y),
                Event::Key(k @ (Key::Up | Key::Down | Key::Left | Key::Right | Key::Enter)) => {
                    return self.edit_key(cx, k);
                }
                Event::Key(Key::SoftLeft) => {
                    self.edit = None;
                    self.view = View::Agenda;
                    return Reply::Redraw;
                }
                Event::Key(Key::SoftRight) => {
                    if self.save_edit(cx) || self.edit.is_some_and(|e| e.read_only) {
                        self.edit = None;
                        self.view = View::Agenda;
                    }
                    return Reply::Redraw;
                }
                Event::Char(_) | Event::Delete | Event::Done | Event::Suggestion(_) => {
                    if let Some(ed) = self.edit.as_mut() {
                        let (buf, n): (&mut [u8; 64], &mut usize) = match ed.typing {
                            Some(EditText::Title) => (&mut ed.title, &mut ed.title_n),
                            Some(EditText::Place) => (&mut ed.place, &mut ed.place_n),
                            None => return Reply::Nothing,
                        };
                        match e {
                            Event::Char(c) if *n < buf.len() => {
                                buf[*n] = c;
                                *n += 1;
                            }
                            Event::Delete => *n = n.saturating_sub(1),
                            Event::Done => ed.typing = None,
                            // A suggested place: street and city in Where.
                            Event::Suggestion(k)
                                if ed.typing == Some(EditText::Place)
                                    && usize::from(k) < self.n_places =>
                            {
                                let (st, ci) = self.places[usize::from(k)];
                                let mut t = Buf::<64>::new();
                                t.s(st.get());
                                if !ci.get().is_empty() {
                                    t.s(b", ").s(ci.get());
                                }
                                buf[..t.get().len()].copy_from_slice(t.get());
                                *n = t.get().len();
                                self.n_places = 0;
                                self.place_asked = Text::of(&buf[..*n]);
                            }
                            _ => {}
                        }
                    }
                    return Reply::Redraw;
                }
                _ => return Reply::Nothing,
            }
        }
        if self.view == View::New
            && let Event::PenUp { x: _, y } = e
            && (322..366).contains(&y)
        {
            self.open_edit(cx, None);
            return Reply::Redraw;
        }
        // Whatever was drawn where the pen lifted (the tabs, the days, the
        // appointments in every view).
        if let Event::PenUp { x, y } = e
            && TABS.contains(&self.view)
            && let Some(h) = self.hits.at(x, y)
        {
            match h {
                Hit::View(v) => return self.go(v, cx),
                // The chosen day tapped again in the month zooms into it
                // (calendar-rule0.md "The design": Month).
                Hit::Day(d) if self.view == View::Month && d == self.day => {
                    return self.go(View::Day, cx);
                }
                Hit::Day(d) => self.day = d,
                Hit::OpenDay(d) => {
                    self.day = d;
                    return self.go(View::Day, cx);
                }
                Hit::Appt(id) => self.open_edit(cx, Some(id)),
                Hit::New => {
                    self.under = self.view;
                    self.view = View::New;
                }
                Hit::Find => {
                    self.view = View::Find;
                    self.find_typing = true;
                    self.find_first = 0;
                    self.focus.clear();
                }
            }
            return Reply::Redraw;
        }
        // The D-pad (R203) reaches everything a stylus can tap in a view.
        // Windows Mobile's Calendar: left and right change the day, up
        // and down move among the day's appointments, the centre opens
        // one; its views are in a menu. Ours keeps those and adds the
        // rest: up and down walk the screen's targets (appointments, the
        // days of a week or a month, New), the nearest one the arrow
        // points at; up from nothing goes to the views' tabs, where left
        // and right choose the view; a tab or a day is chosen as the
        // focus lands on it; the centre taps the target (with none on the
        // board, it opens what is next).
        // The Calendar button, pressed in Calendar, steps through the
        // views (Windows Mobile's does; ours from any of its screens).
        if e == Event::Key(Key::App) {
            let i = TABS.iter().position(|v| *v == self.view);
            return self.go(TABS[i.map_or(0, |i| (i + 1) % TABS.len())], cx);
        }
        if self.view == View::Find {
            let page = self.find_rows.len() as u16;
            match e {
                Event::Char(c) if self.find_n < self.find.len() => {
                    self.find[self.find_n] = c;
                    self.find_n += 1;
                    self.find_first = 0;
                    return Reply::Redraw;
                }
                Event::Delete => {
                    self.find_n = self.find_n.saturating_sub(1);
                    self.find_first = 0;
                    return Reply::Redraw;
                }
                Event::Done => {
                    self.find_typing = false;
                    return Reply::Redraw;
                }
                Event::Key(Key::SoftRight) => {
                    self.find_typing = !self.find_typing;
                    return Reply::Redraw;
                }
                Event::Key(Key::SoftLeft) => {
                    self.view = View::Agenda;
                    self.find_typing = false;
                    self.focus.clear();
                    return Reply::Redraw;
                }
                Event::PenUp { x, y } => {
                    if FIND_FIELD.contains_point(x, y) {
                        self.find_typing = true;
                        return Reply::Redraw;
                    }
                    let hit = self.find_rows[..self.n_find]
                        .iter()
                        .find(|(r, _)| r.contains_point(x, y))
                        .map(|t| t.1);
                    if let Some(id) = hit {
                        self.find_typing = false;
                        self.open_edit(cx, Some(id));
                        return Reply::Redraw;
                    }
                    return Reply::Nothing;
                }
                // The arrows walk the field and the rows; past the last
                // row or the first, the next page or the one before.
                Event::Key(k @ (Key::Up | Key::Down)) => {
                    let mut focus = self.focus;
                    let moved = focus.step(k, self.targets());
                    self.focus = focus;
                    if !moved {
                        if k == Key::Down && self.find_first + page < self.find_total {
                            self.find_first += page;
                        } else if k == Key::Up && self.find_first > 0 {
                            self.find_first = self.find_first.saturating_sub(page);
                        } else {
                            return Reply::Nothing;
                        }
                        self.focus.clear();
                    }
                    return Reply::Redraw;
                }
                Event::Key(Key::Enter) => {
                    return match self.focus.point() {
                        Some((x, y)) => self.event(Event::PenUp { x, y }, cx),
                        None => Reply::Nothing,
                    };
                }
                _ => return Reply::Nothing,
            }
        }
        let tabs = TABS.contains(&self.view);
        let walked = tabs || matches!(self.view, View::Calendars | View::New);
        if let Event::Key(k @ (Key::Up | Key::Down | Key::Left | Key::Right)) = e
            && walked
        {
            let fresh = self.focus.point().is_none();
            // (The calendars' list and the quick-add sheet have no days
            // to step: every arrow walks their targets.)
            let sideways = tabs && matches!(k, Key::Left | Key::Right);
            if tabs && fresh && k == Key::Up {
                let i = TABS.iter().position(|v| *v == self.view).unwrap_or(0);
                self.focus.set(kit::foot_tab(TABS.len(), i));
                return Reply::Redraw;
            }
            if !(fresh && sideways) {
                let mut focus = self.focus;
                let moved = focus.step(k, self.targets());
                self.focus = focus;
                if moved {
                    match self.focus.point().and_then(|(x, y)| self.hits.at(x, y)) {
                        Some(Hit::View(v)) => {
                            let focus = self.focus;
                            let r = self.go(v, cx);
                            self.focus = focus;
                            return r;
                        }
                        Some(Hit::Day(d)) => self.day = d,
                        _ => {}
                    }
                    return Reply::Redraw;
                }
                if !sideways {
                    return Reply::Nothing;
                }
                // Nothing that way: the day before or after (below).
                self.focus.clear();
            }
        }
        if e == Event::Key(Key::Enter)
            && walked
            && let Some(r) = self.focus.rect()
            && self.targets().any(|t| t == r)
            && let Some((x, y)) = self.focus.point()
        {
            return self.event(Event::PenUp { x, y }, cx);
        }
        if e == Event::Key(Key::Enter) && self.view == View::Home {
            let (list, n) = self.on(self.day);
            let next = list[..n].iter().find(|a| {
                !a.task && a.len != 0 && (self.day != cx.now.day || a.start + a.len > cx.now.minute)
            });
            if let Some(a) = next {
                self.open_edit(cx, Some(a.id));
                return Reply::Redraw;
            }
        }
        if let Event::PenUp { x, y } = e
            && matches!(self.view, View::Agenda)
            && let Some(&(_, id, task)) = self.rows_at[..self.n_rows]
                .iter()
                .find(|(r, _, _)| r.contains_point(x, y))
            && !task
        {
            self.open_edit(cx, Some(id));
            return Reply::Redraw;
        }
        match e {
            Event::PenUp { x, y }
                if self.view == View::Calendars
                    && self.cal_rows[..self.n_cal_rows]
                        .iter()
                        .any(|(r, _)| r.contains_point(x, y)) =>
            {
                if let Some(&(_, id)) = self.cal_rows[..self.n_cal_rows]
                    .iter()
                    .find(|(r, _)| r.contains_point(x, y))
                {
                    self.toggle_hidden(cx, id);
                }
                return Reply::Redraw;
            }
            Event::PenUp { x, y } if self.view == View::Calendars && self.files_tap(x, y) => {
                self.focus.clear();
                return Reply::Redraw;
            }
            Event::PenUp { x, y }
                if self.view == View::Calendars && GET_KEY.contains_point(x, y) =>
            {
                self.want_list = true;
                self.status = b"Asking your PC for its calendars...";
                return Reply::Redraw;
            }
            Event::Key(Key::SoftLeft) if self.view == View::Calendars => {
                self.view = View::Agenda;
                return Reply::Redraw;
            }
            Event::PenUp { x, y }
                if self.view != View::New
                    && self.view != View::Home
                    && CALENDARS_KEY.contains_point(x, y) =>
            {
                self.view = View::Calendars;
                return Reply::Redraw;
            }
            _ => {}
        }
        match e {
            Event::Key(Key::SoftLeft) => {
                if self.view == View::New {
                    self.view = self.under;
                } else {
                    self.day = cx.now.day;
                }
                Reply::Redraw
            }
            Event::Key(Key::SoftRight) | Event::Done if self.view == View::New => {
                if self.add_typed(cx) {
                    self.view = self.under;
                }
                Reply::Redraw
            }
            Event::Key(Key::SoftRight) => {
                self.under = self.view;
                self.view = View::New;
                Reply::Redraw
            }
            Event::Char(c) if self.view == View::New && self.line_n < self.line.len() => {
                self.line[self.line_n] = c;
                self.line_n += 1;
                Reply::Redraw
            }
            Event::Delete if self.view == View::New && self.line_n > 0 => {
                self.line_n -= 1;
                Reply::Redraw
            }
            Event::Key(Key::Left) if tabs => self.step_day(-1, cx),
            Event::Key(Key::Right) if tabs => self.step_day(1, cx),
            _ => Reply::Nothing,
        }
    }

    fn keyboard(&self) -> Option<Field> {
        match (self.view, self.edit.and_then(|e| e.typing)) {
            (View::New, _) => Some(Field::Line),
            (View::Find, _) if self.find_typing => Some(Field::Line),
            (View::Zones, _) if self.io.typing => Some(Field::Line),
            (View::Edit, Some(EditText::Title)) => Some(Field::Name),
            (View::Edit, Some(EditText::Place)) => Some(Field::Address),
            _ => None,
        }
    }

    fn suggestions(&self) -> [&[u8]; 3] {
        let st = |k: usize| {
            if k < self.n_places {
                self.places[k].0.get()
            } else {
                &b""[..]
            }
        };
        match self.edit.and_then(|e| e.typing) {
            Some(EditText::Place) => [st(0), st(1), st(2)],
            _ => [&[], &[], &[]],
        }
    }

    /// The online calendars: the PC's list first, then each link in turn;
    /// Where's address suggestions as it is typed.
    fn net_ask(&mut self) -> Option<rp_app::NetAsk> {
        if let Some(a) = self.files_net_ask() {
            return Some(a);
        }
        if let Some(e) = self.edit
            && e.typing == Some(EditText::Place)
            && e.place_n >= 3
            && self.place_asked.get() != &e.place[..e.place_n]
        {
            self.place_asked = Text::of(&e.place[..e.place_n]);
            self.asked = Asked::Place;
            return Some(rp_app::NetAsk {
                what: b"ADDR",
                arg: rp_app::Text::of(&e.place[..e.place_n]),
            });
        }
        if self.want_list {
            self.want_list = false;
            self.asked = Asked::List;
            return Some(rp_app::NetAsk {
                what: b"ICALS",
                arg: rp_app::Text::EMPTY,
            });
        }
        if self.n_queue > 0 {
            self.n_queue -= 1;
            let n = self.queue[self.n_queue];
            self.asked = Asked::Link(n);
            let digits = [b'0' + n / 10, b'0' + n % 10];
            let arg = if n > 9 { &digits[..] } else { &digits[1..] };
            return Some(rp_app::NetAsk {
                what: b"ICAL",
                arg: rp_app::Text::of(arg),
            });
        }
        None
    }

    fn net_answer(&mut self, ok: bool, body: &[u8], cx: &Ctx) -> Reply {
        if self.files_asking() {
            return self.files_answer(ok, body, cx);
        }
        let asked = core::mem::replace(&mut self.asked, Asked::Nothing);
        if asked == Asked::Place {
            let mut found = [rp_pim::net::Place::EMPTY; 3];
            let n = if ok {
                rp_pim::net::photon(body, &mut found)
            } else {
                0
            };
            for (slot, p) in self.places.iter_mut().zip(&found[..n]) {
                *slot = (Text::of(p.street.get()), Text::of(p.city.get()));
            }
            self.n_places = n;
            return Reply::Redraw;
        }
        if !ok {
            self.status = b"No connection (Wi-Fi or the dock)";
            self.n_queue = 0;
            return Reply::Redraw;
        }
        match asked {
            Asked::List => self.take_list(body, cx),
            Asked::Link(n) => self.take_link(n, body, cx),
            Asked::Nothing | Asked::Place => {}
        }
        Reply::Redraw
    }

    fn card_ask(&mut self) -> Option<rp_app::NetAsk> {
        self.files_card_ask()
    }

    fn net_body(&mut self, at: usize, out: &mut [u8], cx: &Ctx) -> usize {
        self.files_body(at, out, cx)
    }
}

impl Calendar {
    /// Everything on the view a stylus can tap, for the D-pad: what was
    /// drawn (`hits`), Agenda's task rows (ticked by a tap) and the
    /// Calendars key; in the calendars' list its rows and the key that
    /// asks the PC; on the quick-add sheet its way to the full editor.
    fn targets(&self) -> impl Iterator<Item = Rect> + '_ {
        use core::iter::once;
        let view = self.view;
        let tabs = TABS.contains(&view);
        let tasks = self.rows_at[..self.n_rows]
            .iter()
            .filter(move |r| r.2 && view == View::Agenda)
            .map(|r| r.0);
        let key = once(CALENDARS_KEY).filter(move |_| tabs && view != View::Home);
        let cals = self.cal_rows[..self.n_cal_rows]
            .iter()
            .map(|r| r.0)
            .chain([files::ZONE_ROW, files::IMPORT_KEY, files::EXPORT_KEY])
            .chain(once(GET_KEY))
            .filter(move |_| view == View::Calendars);
        let io = self.io.targets(view);
        let new = once(Rect::new(MARGIN, 322, WIDE, 44)).filter(move |_| view == View::New);
        let found = once(FIND_FIELD)
            .chain(self.find_rows[..self.n_find].iter().map(|r| r.0))
            .filter(move |_| view == View::Find);
        self.hits
            .rects()
            .filter(move |_| tabs)
            .chain(tasks)
            .chain(key)
            .chain(cals)
            .chain(new)
            .chain(found)
            .chain(io)
    }

    /// The search (Windows Mobile's Calendar has none; a phone's lists
    /// matches by day): every appointment kept whose title or place holds
    /// what is typed, soonest first, read from the store's records (not
    /// from the window of days the views hold), the match in the system
    /// accent; a row opens its appointment.
    #[inline(never)]
    fn find_view(&mut self, f: &mut Frame, cx: &Ctx) {
        self.n_find = 0;
        kit::large_title(
            f,
            cx,
            36,
            b"Search",
            b"Every appointment kept",
            tint::CALENDAR,
        );
        kit::panel(f, cx, FIND_FIELD);
        if self.find_typing {
            f.push(rp_app::Cmd::Outline {
                r: FIND_FIELD,
                color: kit::FOCUS,
            });
        }
        let q = &self.find[..self.find_n];
        let (ui, label) = (cx.fonts.ui, cx.fonts.label);
        if q.is_empty() {
            kit::text(
                f,
                FIND_FIELD.x + 14,
                FIND_FIELD.y + 25,
                ui,
                DIM,
                b"A word of its title or place",
            );
        } else {
            kit::text(f, FIND_FIELD.x + 14, FIND_FIELD.y + 25, ui, TEXT, q);
        }
        let at = |hay: &[u8]| {
            (!q.is_empty() && hay.len() >= q.len())
                .then(|| hay.windows(q.len()).position(|w| w.eq_ignore_ascii_case(q)))
                .flatten()
        };
        let Some(guard) = cx.pim.and_then(|c| c.try_borrow().ok()) else {
            return;
        };
        let Some(store) = guard.as_ref() else { return };
        // The matches' starts and ids, soonest first (kept in order as
        // they come: the store gives none).
        let mut found = [(0u32, 0u32); MAX];
        let mut n = 0;
        if !q.is_empty() {
            for r in store.all(Kind::Appointment) {
                if at(r.text(tag::TITLE)).is_none() && at(r.text(tag::PLACE)).is_none() {
                    continue;
                }
                let key = (r.num(tag::START).unwrap_or(0), r.id);
                let to = found[..n].iter().position(|k| *k > key).unwrap_or(n);
                if n < MAX {
                    found.copy_within(to..n, to + 1);
                    found[to] = key;
                    n += 1;
                }
            }
        }
        self.find_total = n as u16;
        if usize::from(self.find_first) >= n {
            self.find_first = 0;
        }
        let first = usize::from(self.find_first);
        let mut y = FIND_FIELD.y + FIND_FIELD.h + 8;
        for &(start, id) in found[first..n].iter().take(self.find_rows.len()) {
            let Some(r) = store.get(id) else { continue };
            let row = Rect::new(MARGIN, y, WIDE, 46);
            kit::panel(f, cx, row);
            let d = date::date_of(date::day_of(start));
            let mut when = Buf::<24>::new();
            when.date(date::day_of(start), cx.us)
                .s(b", ")
                .num(d.year.unsigned_abs());
            kit::text(f, row.x + 14, row.y + 20, label, DIM, when.get());
            if r.num(tag::ALL_DAY) != Some(1) {
                kit::text(
                    f,
                    row.x + 14,
                    row.y + 40,
                    label,
                    DIM,
                    clock((start % date::DAY_MIN) as u16, cx.clock12).get(),
                );
            }
            let (title, place) = (r.text(tag::TITLE), r.text(tag::PLACE));
            let x = row.x + 130;
            for (text, ty, col) in [(title, row.y + 20, TEXT), (place, row.y + 40, DIM)] {
                kit::fitted(f, x, ty, ui, col, text, row.w - 144);
                if let Some(m) = at(text) {
                    let mx = x + ui.width(&text[..m]);
                    if mx < x + row.w - 144 {
                        kit::text(f, mx, ty, ui, kit::FOCUS, &text[m..m + q.len()]);
                    }
                }
            }
            self.find_rows[self.n_find] = (row, id);
            self.n_find += 1;
            y += 50;
        }
        let mut b = Buf::<24>::new();
        if q.is_empty() {
            b.s(b"Type to search");
        } else if n == 0 {
            b.s(b"Nothing holds that");
        } else {
            b.num(first as u32 + 1)
                .s(b"-")
                .num((first + self.n_find) as u32)
                .s(b" of ")
                .num(n as u32);
        }
        kit::right(
            f,
            FIND_FIELD.right() - 14,
            FIND_FIELD.y + 24,
            label,
            DIM,
            b.get(),
        );
    }
}

#[cfg(test)]
mod tests;
