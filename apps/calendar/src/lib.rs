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

/// Appointments a view holds.
pub const MAX: usize = 64;

/// One calendar: a name and its colour.
#[derive(Clone, Copy, Debug)]
pub struct Cal {
    /// Its name.
    pub name: &'static [u8],
    /// Its identity colour.
    pub colour: u32,
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
    pub title: &'static [u8],
    /// Where, if anywhere.
    pub place: &'static [u8],
    /// Which calendar.
    pub cal: u8,
    /// It repeats.
    pub repeats: bool,
}

/// The views.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
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
}

/// The Calendar program.
pub struct Calendar {
    cals: &'static [Cal],
    appts: [Option<Appt>; MAX],
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
}

const LABELS: [&[u8]; 4] = [b"AGENDA", b"DAY", b"WEEK", b"MONTH"];
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
    fn get(&self) -> &[u8] {
        &self.b[..self.n]
    }
}

/// "3:00 PM" (the US default: 12-hour).
fn clock(m: u16) -> Buf<10> {
    let (h, mm) = (m / 60, m % 60);
    let h12 = match h % 12 {
        0 => 12,
        x => x,
    };
    let mut b = Buf::new();
    b.num(u32::from(h12)).s(b":");
    if mm < 10 {
        b.s(b"0");
    }
    b.num(u32::from(mm)).s(if h < 12 { b" AM" } else { b" PM" });
    b
}

/// "3 PM" for an hour line.
fn hour(h: u16) -> Buf<6> {
    let h12 = match h % 12 {
        0 => 12,
        x => x,
    };
    let mut b = Buf::new();
    b.num(u32::from(h12))
        .s(if h < 12 { b" AM" } else { b" PM" });
    b
}

impl Calendar {
    /// A calendar with `cals` and no appointments, on `today`.
    #[must_use]
    pub fn new(cals: &'static [Cal], today: i32) -> Self {
        Self {
            cals,
            appts: [None; MAX],
            view: View::Agenda,
            day: today,
            line: [0; 64],
            line_n: 0,
            ring: None,
            chosen: None,
            painted: false,
        }
    }

    /// Adds an appointment (false when full).
    pub fn add(&mut self, a: Appt) -> bool {
        match self.appts.iter_mut().find(|s| s.is_none()) {
            Some(s) => {
                *s = Some(a);
                true
            }
            None => false,
        }
    }

    /// Shows `view`.
    pub fn show(&mut self, view: View) {
        self.view = view;
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
    fn on(&self, day: i32) -> ([Appt; 16], usize) {
        let mut out = [Appt {
            day: 0,
            start: 0,
            len: 0,
            title: b"",
            place: b"",
            cal: 0,
            repeats: false,
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
        self.cals
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
        let d = date::date_of(self.day);
        let mut t = Buf::<32>::new();
        t.s(WEEKDAY_NAMES[usize::from(date::weekday(self.day))])
            .s(b", ")
            .s(&MONTHS[usize::from(d.month - 1)][..3])
            .s(b" ")
            .num(u32::from(d.day));
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
                .s(clock(a.start).get())
                .s(b" ")
                .s(a.title),
            (n, None) => sum.num(n as u32).s(b" today \x80 nothing more"),
        };
        kit::large_title(f, cx, 36, t.get(), sum.get(), tint::CALENDAR)
    }

    /// The week strip: the chosen day's week, today ringed, the chosen day
    /// filled, a colour tick per appointment (three at most).
    fn strip(&self, f: &mut Frame, cx: &Ctx, y: u16) {
        let first = self.day - i32::from(date::weekday(self.day));
        for i in 0..7u16 {
            let d = first + i32::from(i);
            let x = MARGIN + i * 64;
            let cell = Rect::new(x, y, 64, 62);
            kit::centred(
                f,
                cell,
                y + 14,
                cx.fonts.label,
                DIM,
                WEEKDAYS[usize::from(i)],
            );
            if d == self.day {
                kit::dot(f, self.chosen, x + 12, y + 18);
            } else if d == cx.now.day {
                kit::dot(f, self.ring, x + 12, y + 18);
            }
            let mut b = Buf::<4>::new();
            b.num(u32::from(date::date_of(d).day));
            kit::centred(f, cell, y + 45, cx.fonts.ui, TEXT, b.get());
            let (list, n) = self.on(d);
            for (k, a) in list[..n.min(3)].iter().enumerate() {
                f.push(rp_app::Cmd::Fill {
                    r: Rect::new(x + 17 + k as u16 * 11, y + 59, 8, 3),
                    color: self.colour(a),
                });
            }
        }
    }

    /// The forecast for `day`, if the device holds it.
    fn weather(cx: &Ctx, day: i32) -> Option<rp_app::DayWeather> {
        cx.forecast.iter().copied().find(|w| w.day == day)
    }

    fn agenda(&self, f: &mut Frame, cx: &Ctx, top: u16) {
        let mut y = top;
        let bottom = 596;
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
                    let d = date::date_of(day);
                    l.s(WEEKDAYS[usize::from(date::weekday(day))])
                        .s(b" ")
                        .num(u32::from(d.day))
                        .s(b" ")
                        .s(&MONTHS[usize::from(d.month - 1)][..3]);
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
                    kit::panel(f, cx, r);
                    kit::colour_bar(f, r, self.colour(a));
                    if a.len == 0 {
                        kit::text(f, MARGIN + 20, y + 32, cx.fonts.ui, DIM, b"All day");
                    } else {
                        kit::text(
                            f,
                            MARGIN + 20,
                            y + 22,
                            cx.fonts.ui,
                            TEXT,
                            clock(a.start).get(),
                        );
                        kit::text(
                            f,
                            MARGIN + 20,
                            y + 44,
                            cx.fonts.ui,
                            DIM,
                            clock(a.start + a.len).get(),
                        );
                    }
                    kit::fitted(
                        f,
                        MARGIN + 122,
                        y + 22,
                        cx.fonts.ui,
                        TEXT,
                        a.title,
                        WIDE - 130,
                    );
                    if !a.place.is_empty() {
                        kit::fitted(
                            f,
                            MARGIN + 122,
                            y + 44,
                            cx.fonts.ui,
                            DIM,
                            a.place,
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

    fn day_view(&self, f: &mut Frame, cx: &Ctx, top: u16) {
        let (list, n) = self.on(self.day);
        let mut y = top;
        // All-day items in a strip.
        for a in list[..n].iter().filter(|a| a.len == 0) {
            let r = Rect::new(MARGIN + 52, y, WIDE - 52, 30);
            kit::tinted(f, cx, r, self.colour(a), 0x70);
            kit::fitted(f, r.x + 12, y + 21, cx.fonts.ui, TEXT, a.title, r.w - 20);
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
        let per = ((596 - y) / span).clamp(30, 60);
        let hours = ((596 - y) / per).min(span);
        for h in 0..hours {
            let hy = y + h * per;
            kit::right(
                f,
                MARGIN + 46,
                hy + 6,
                cx.fonts.label,
                DIM,
                hour(first_h + h).get(),
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
            let bh = (a.len * per / 60).max(28).min(596 - by);
            let r = Rect::new(x0 + lane * wide, by + 1, wide - 4, bh - 2);
            kit::tinted(f, cx, r, self.colour(a), 0x80);
            kit::colour_bar(f, r, self.colour(a));
            kit::fitted(f, r.x + 16, r.y + 20, cx.fonts.ui, TEXT, a.title, r.w - 22);
            if bh > 48 {
                let mut t = Buf::<24>::new();
                t.s(clock(a.start).get())
                    .s(b" - ")
                    .s(clock(a.start + a.len).get());
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

    fn week_view(&self, f: &mut Frame, cx: &Ctx, top: u16) {
        let first = self.day - i32::from(date::weekday(self.day));
        let x0 = MARGIN + 36;
        let cw = (WIDE - 36) / 7;
        let (h0, h1) = (8u16, 21u16);
        for i in 0..7u16 {
            let d = first + i32::from(i);
            let c = Rect::new(x0 + i * cw, top, cw, 40);
            let col = if d == cx.now.day { tint::CALENDAR } else { DIM };
            kit::centred(
                f,
                c,
                top + 14,
                cx.fonts.label,
                col,
                WEEKDAYS[usize::from(i)],
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
        let y0 = top + 46;
        let per = (596 - y0) / (h1 - h0);
        for h in 0..(h1 - h0) {
            let hy = y0 + h * per;
            let mut b = Buf::<4>::new();
            let hh = (h0 + h) % 12;
            b.num(u32::from(if hh == 0 { 12 } else { hh }));
            kit::right(f, x0 - 4, hy + 6, cx.fonts.label, DIM, b.get());
            f.push(rp_app::Cmd::Fill {
                r: Rect::new(x0, hy, WIDE - 36, 1),
                color: kit::alpha(TEXT, 0x28),
            });
        }
        for i in 0..7u16 {
            let d = first + i32::from(i);
            let cx0 = x0 + i * cw;
            if d == cx.now.day {
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
                let lw = cw / lanes;
                let by = y0 + (a.start - h0 * 60) * per / 60;
                let bh = (a.len * per / 60).max(16);
                let r = Rect::new(cx0 + 1 + lane * lw, by + 1, lw - 2, bh - 2);
                f.push(rp_app::Cmd::Fill {
                    r,
                    color: kit::alpha(self.colour(a), 0xD0),
                });
                kit::fitted(
                    f,
                    r.x + 3,
                    r.y + 15,
                    cx.fonts.label,
                    0xFF10_1418,
                    a.title,
                    r.w - 4,
                );
            }
        }
    }

    fn month_view(&self, f: &mut Frame, cx: &Ctx, top: u16) {
        let d = date::date_of(self.day);
        let start = date::days_from(date::Date {
            year: d.year,
            month: d.month,
            day: 1,
        });
        let lead = i32::from(date::weekday(start));
        let mut y = top;
        let mut name = Buf::<24>::new();
        name.s(MONTHS[usize::from(d.month - 1)])
            .s(b" ")
            .num(d.year as u32);
        kit::text(f, MARGIN + 4, y + 20, cx.fonts.ui, TEXT, name.get());
        y += 28;
        for (i, w) in WEEKDAYS.iter().enumerate() {
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
                if day == self.day {
                    kit::tinted(
                        f,
                        cx,
                        Rect::new(x + 2, cy + 1, 60, ch - 2),
                        tint::CALENDAR,
                        0x90,
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
                let (list, n) = self.on(day);
                for (k, a) in list[..n.min(3)].iter().enumerate() {
                    f.push(rp_app::Cmd::Fill {
                        r: Rect::new(x + 8, cy + 25 + k as u16 * 6, 46, 4),
                        color: self.colour(a),
                    });
                }
            }
        }
        y += rows * ch + 6;
        let (list, n) = self.on(self.day);
        for a in &list[..n] {
            if y + 40 > 596 {
                break;
            }
            let r = Rect::new(MARGIN, y, WIDE, 38);
            kit::panel(f, cx, r);
            kit::colour_bar(f, r, self.colour(a));
            let t = if a.len == 0 {
                Buf::<10>::new()
            } else {
                clock(a.start)
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
                a.title,
                WIDE - 130,
            );
            y += 42;
        }
    }

    /// The quick-add sheet: the line with the understood words in the
    /// tint, and the preview card of what will be added.
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
        let dd = date::date_of(day);
        let mut when = Buf::<48>::new();
        when.s(WEEKDAYS[usize::from(date::weekday(day))])
            .s(b" ")
            .num(u32::from(dd.day))
            .s(b" ")
            .s(&MONTHS[usize::from(dd.month - 1)][..3]);
        if let Some(t) = q.time {
            let len = q.minutes.unwrap_or(60);
            when.s(b"  ")
                .s(clock(t).get())
                .s(b" - ")
                .s(clock(t + len).get());
        } else {
            when.s(b"  All day");
        }
        kit::text(f, card.x + 20, card.y + 58, cx.fonts.ui, TEXT, when.get());
        let mut cal = Buf::<48>::new();
        cal.s(self.cals[0].name).s(b" \x80 alert 15 min before");
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

impl Program for Calendar {
    fn title(&self) -> &'static str {
        "Calendar"
    }

    fn softkeys(&self) -> (&'static str, &'static str) {
        match self.view {
            View::New => ("Cancel", "Add"),
            _ => ("Today", "New"),
        }
    }

    fn draw(&mut self, f: &mut Frame, cx: &Ctx) {
        self.paint(cx);
        kit::backdrop(f);
        let under = self.title(f, cx);
        let shown = if self.view == View::New {
            View::Agenda
        } else {
            self.view
        };
        let chosen = match shown {
            View::Agenda | View::New => 0,
            View::Day => 1,
            View::Week => 2,
            View::Month => 3,
        };
        kit::chips(
            f,
            cx,
            Rect::new(MARGIN, under + 4, WIDE, 30),
            &LABELS,
            chosen,
            tint::CALENDAR,
        );
        let y = under + 40;
        match shown {
            View::Agenda | View::New => {
                self.strip(f, cx, y);
                self.agenda(f, cx, y + 70);
            }
            View::Day => {
                self.strip(f, cx, y);
                self.day_view(f, cx, y + 70);
            }
            View::Week => self.week_view(f, cx, y),
            View::Month => self.month_view(f, cx, y),
        }
        if self.view == View::New {
            self.new_sheet(f, cx);
        }
    }

    fn event(&mut self, e: Event, cx: &Ctx) -> Reply {
        match e {
            Event::Key(Key::SoftLeft) => {
                if self.view == View::New {
                    self.view = View::Agenda;
                } else {
                    self.day = cx.now.day;
                }
                Reply::Redraw
            }
            Event::Key(Key::SoftRight) => {
                self.view = if self.view == View::New {
                    View::Agenda
                } else {
                    View::New
                };
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
            Event::Key(Key::Left) => {
                self.day -= 1;
                Reply::Redraw
            }
            Event::Key(Key::Right) => {
                self.day += 1;
                Reply::Redraw
            }
            _ => Reply::Nothing,
        }
    }

    fn keyboard(&self) -> Option<Field> {
        (self.view == View::New).then_some(Field::Line)
    }
}
