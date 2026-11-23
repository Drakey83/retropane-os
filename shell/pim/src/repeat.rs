//! How an appointment or a task repeats (calendar-rule0.md and
//! tasks-rule0.md version 2), packed into one number of the store.
//!
//! Found: iCalendar's RRULE (RFC 5545) can say almost anything (BYSETPOS,
//! BYWEEKNO, ...), and Windows Mobile offered daily, weekly on days,
//! monthly by date or weekday, yearly. Ours stores the rules people make
//! (WM's set, plus every N and an end) in 32 bits, answers "does it occur
//! on day d" with arithmetic and no expansion, and the CalDAV/iCal reader
//! maps the RRULEs it meets onto it (the rest are kept as single events).
//!
//! Tasks add "after done" (tasks-rule0.md): Todoist writes it "every!",
//! Things and Reminders hide it in a sheet, Windows Mobile's "regenerate"
//! made a new record each time. Ours is one flag in the same 32 bits, and
//! [`Rule::next_after`] moves the one task on: no copy per completion, so
//! a daily task never fills the store (the done days go to a short log,
//! `tag::DONE_LOG`).

use crate::date;

/// How often.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Every {
    /// Every `n` days.
    Day,
    /// Every `n` weeks, on the weekdays in the mask.
    Week,
    /// Every `n` months on the start's date.
    MonthDate,
    /// Every `n` months on the start's weekday of the month ("the 2nd
    /// Tuesday"; the 5th means the last).
    MonthWeekday,
    /// Every `n` years on the start's date.
    Year,
}

/// A repeat rule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rule {
    /// How often.
    pub every: Every,
    /// Every how many (1-63).
    pub n: u8,
    /// Weekly: the weekdays (bit 0 Monday .. bit 6 Sunday); 0 means the
    /// start's own.
    pub days: u8,
    /// The last day it may occur (days from 1970), if it ends.
    pub until: Option<i32>,
    /// A task's "after done": the next one is `n` days, weeks, months or
    /// years after the day it was done, not on a fixed schedule (`every`
    /// gives the unit; `days` is unused).
    pub after: bool,
}

impl Rule {
    /// Packed for the store: bits 0-2 how often, 3-8 n, 9-15 the
    /// weekdays, 16-31 the end as days after the start (0: none). "After
    /// done" is how-often 5 with its unit (day, week, month, year: 0-3) in
    /// the weekdays' bits, which it has no use for; values 6 and 7 stay
    /// free.
    #[must_use]
    pub fn pack(&self, start: i32) -> u32 {
        let unit = match self.every {
            Every::Day => 0,
            Every::Week => 1,
            Every::MonthDate | Every::MonthWeekday => 2,
            Every::Year => 3,
        };
        let (e, days) = if self.after {
            (5, unit)
        } else {
            (
                match self.every {
                    Every::Day => 0,
                    Every::Week => 1,
                    Every::MonthDate => 2,
                    Every::MonthWeekday => 3,
                    Every::Year => 4,
                },
                self.days & 0x7F,
            )
        };
        let until = self
            .until
            .map_or(0, |u| (u - start).clamp(1, 0xFFFF) as u32);
        e | u32::from(self.n.clamp(1, 63)) << 3 | u32::from(days) << 9 | until << 16
    }

    /// Unpacked; `None` for a value no rule packs to.
    #[must_use]
    pub fn unpack(v: u32, start: i32) -> Option<Self> {
        let days = ((v >> 9) & 0x7F) as u8;
        let (every, after) = match v & 7 {
            0 => (Every::Day, false),
            1 => (Every::Week, false),
            2 => (Every::MonthDate, false),
            3 => (Every::MonthWeekday, false),
            4 => (Every::Year, false),
            5 => (
                match days {
                    0 => Every::Day,
                    1 => Every::Week,
                    2 => Every::MonthDate,
                    3 => Every::Year,
                    _ => return None,
                },
                true,
            ),
            _ => return None,
        };
        let n = ((v >> 3) & 63) as u8;
        if n == 0 {
            return None;
        }
        let until = match v >> 16 {
            0 => None,
            d => Some(start + d as i32),
        };
        Some(Self {
            every,
            n,
            days: if after { 0 } else { days },
            until,
            after,
        })
    }

    /// The day a task with this rule, due on `due` (its series began on
    /// `start`) and done on `done`, is due next: "after done", `n` units
    /// after `done`; otherwise the first day it occurs after both its due
    /// day and the day it was done (a late daily task ticked today comes
    /// back tomorrow, not as a pile of late ones: Todoist's "every" leaves
    /// it in the past). `None` once the rule has ended.
    #[must_use]
    pub fn next_after(&self, start: i32, due: i32, done: i32) -> Option<i32> {
        let n = i32::from(self.n.max(1));
        let next = if self.after {
            match self.every {
                Every::Day => done + n,
                Every::Week => done + 7 * n,
                Every::MonthDate | Every::MonthWeekday => add_months(done, n),
                Every::Year => add_months(done, 12 * n),
            }
        } else {
            let from = due.max(done).max(start - 1);
            match self.every {
                // Arithmetic: the first multiple of n days past `from`.
                Every::Day => start + ((from - start).div_euclid(n) + 1) * n,
                // Walked: a whole cycle of weeks holds the next, and the
                // month and year rules find theirs within 4n years (the
                // 29th of February every n years).
                _ => {
                    let span = match self.every {
                        Every::Week => 7 * n,
                        _ => 1462 * n,
                    };
                    (from + 1..=from + span).find(|&d| self.occurs(start, d))?
                }
            }
        };
        self.until.is_none_or(|u| next <= u).then_some(next)
    }
    /// Whether something starting on day `start` with this rule occurs on
    /// day `day`.
    #[must_use]
    pub fn occurs(&self, start: i32, day: i32) -> bool {
        if day < start || self.until.is_some_and(|u| day > u) {
            return false;
        }
        // "After done": only its due day is known until it is done.
        if self.after {
            return day == start;
        }
        let n = i32::from(self.n.max(1));
        let (s, d) = (date::date_of(start), date::date_of(day));
        match self.every {
            Every::Day => (day - start) % n == 0,
            Every::Week => {
                let mask = if self.days == 0 {
                    1 << date::weekday(start)
                } else {
                    self.days
                };
                let monday = |x: i32| x - i32::from(date::weekday(x));
                let weeks = (monday(day) - monday(start)) / 7;
                weeks % n == 0 && mask & (1 << date::weekday(day)) != 0
            }
            Every::MonthDate => {
                let months = (d.year - s.year) * 12 + i32::from(d.month) - i32::from(s.month);
                months % n == 0 && d.day == s.day
            }
            Every::MonthWeekday => {
                let months = (d.year - s.year) * 12 + i32::from(d.month) - i32::from(s.month);
                if months % n != 0 || date::weekday(day) != date::weekday(start) {
                    return false;
                }
                let nth = |x: date::Date| (x.day - 1) / 7;
                let last = |x: date::Date| x.day + 7 > date::days_in_month(x.year, x.month);
                // The 5th of a weekday means the last one of the month.
                if nth(s) >= 4 {
                    last(d)
                } else {
                    nth(d) == nth(s)
                }
            }
            Every::Year => (d.year - s.year) % n == 0 && d.month == s.month && d.day == s.day,
        }
    }

    /// Whether the plain "every day / week / month / year" says all of it
    /// (every one, on the start's own day, never ending): an editor's one
    /// chip then shows the whole rule, and [`Rule::words`] is not needed.
    #[must_use]
    pub fn plain(&self, start: i32) -> bool {
        self.n == 1
            && self.until.is_none()
            && !self.after
            && match self.every {
                Every::Week => self.days == 0 || self.days == 1 << date::weekday(start),
                Every::MonthWeekday => false,
                _ => true,
            }
    }

    /// The rule in words for something starting on `start`: "Every 2
    /// weeks on Mon, Wed until Mar 3", "Every month on the last Fri",
    /// "3 days after done" (`us`: "Mar 3", else "3 Mar"). Its length in
    /// `out` (cut at its end).
    pub fn words(&self, start: i32, us: bool, out: &mut [u8]) -> usize {
        let mut n = 0;
        let mut put = |s: &[u8], n: &mut usize| {
            for &c in s {
                if *n < out.len() {
                    out[*n] = c;
                    *n += 1;
                }
            }
        };
        let num = |v: u32, buf: &mut [u8; 10]| {
            let mut k = buf.len();
            let mut v = v;
            loop {
                k -= 1;
                buf[k] = b'0' + (v % 10) as u8;
                v /= 10;
                if v == 0 {
                    break k;
                }
            }
        };
        let mut b = [0u8; 10];
        let unit: &[u8] = match self.every {
            Every::Day => b"day",
            Every::Week => b"week",
            Every::MonthDate | Every::MonthWeekday => b"month",
            Every::Year => b"year",
        };
        if self.after {
            let k = num(u32::from(self.n.max(1)), &mut b);
            put(&b[k..], &mut n);
            put(b" ", &mut n);
            put(unit, &mut n);
            if self.n > 1 {
                put(b"s", &mut n);
            }
            put(b" after done", &mut n);
        } else {
            put(b"Every ", &mut n);
            if self.n > 1 {
                let k = num(u32::from(self.n), &mut b);
                put(&b[k..], &mut n);
                put(b" ", &mut n);
                put(unit, &mut n);
                put(b"s", &mut n);
            } else {
                put(unit, &mut n);
            }
            match self.every {
                Every::Week if self.days != 0 && self.days != 1 << date::weekday(start) => {
                    put(b" on ", &mut n);
                    // Runs of three or more as a range ("Mon-Fri"): the
                    // longest rule still fits the editor's one line.
                    let on = |i: usize| i < 7 && self.days & 1 << i != 0;
                    let (mut i, mut first) = (0, true);
                    while i < 7 {
                        if !on(i) {
                            i += 1;
                            continue;
                        }
                        let mut j = i;
                        while on(j + 1) {
                            j += 1;
                        }
                        if !first {
                            put(b", ", &mut n);
                        }
                        first = false;
                        put(date::WEEKDAYS[i].as_bytes(), &mut n);
                        if j >= i + 2 {
                            put(b"-", &mut n);
                            put(date::WEEKDAYS[j].as_bytes(), &mut n);
                            i = j;
                        }
                        i += 1;
                    }
                }
                Every::MonthWeekday => {
                    let s = date::date_of(start);
                    put(b" on the ", &mut n);
                    put(
                        match (s.day - 1) / 7 {
                            0 => b"1st ",
                            1 => b"2nd ",
                            2 => b"3rd ",
                            3 => b"4th ",
                            _ => b"last ",
                        },
                        &mut n,
                    );
                    put(
                        date::WEEKDAYS[usize::from(date::weekday(start))].as_bytes(),
                        &mut n,
                    );
                }
                _ => {}
            }
        }
        if let Some(u) = self.until {
            let d = date::date_of(u);
            let k = num(u32::from(d.day), &mut b);
            let month = date::MONTHS[usize::from(d.month - 1)].as_bytes();
            put(b" until ", &mut n);
            if us {
                put(month, &mut n);
                put(b" ", &mut n);
                put(&b[k..], &mut n);
            } else {
                put(&b[k..], &mut n);
                put(b" ", &mut n);
                put(month, &mut n);
            }
        }
        n
    }
}

/// The day `months` after `day`, on the same date, or the month's last
/// day when it is shorter (31 January and a month: 29 February in a leap
/// year).
#[must_use]
pub const fn add_months(day: i32, months: i32) -> i32 {
    let d = date::date_of(day);
    let m = date::month_start(d, months);
    let last = date::days_in_month(m.year, m.month);
    date::days_from(date::Date {
        year: m.year,
        month: m.month,
        day: if d.day > last { last } else { d.day },
    })
}

/// The repeat a typed line names, read from the words after its "every"
/// (tasks-rule0.md: "every day", "every weekday", "every 3 days", "every
/// other week", "every mon wed fri", "every 2 months", "every year", and
/// any of them followed by "after done"): the rule and how many words it
/// took. Todoist reads the same phrases on its server; ours reads them on
/// the device as they are typed, before the date words, so "every mon" is
/// a repeat and not next Monday's date.
#[must_use]
pub fn phrase(words: &[&[u8]]) -> Option<(Rule, usize)> {
    const FULL: [&str; 7] = [
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
    ];
    let is = |w: &[u8], s: &str| w.eq_ignore_ascii_case(s.as_bytes());
    let word = |k: usize| -> &[u8] {
        let w = words.get(k).copied().unwrap_or(b"");
        w.strip_suffix(b",").unwrap_or(w)
    };
    let day_name = |w: &[u8]| {
        (0..7u8)
            .find(|&d| is(w, FULL[usize::from(d)]) || is(w, crate::date::WEEKDAYS[usize::from(d)]))
    };
    let mut at = 0;
    // How many: a number, or "other" (every second one).
    let mut n = 1u8;
    let w = word(0);
    if is(w, "other") {
        n = 2;
        at = 1;
    } else if !w.is_empty() && w.len() <= 2 && w.iter().all(u8::is_ascii_digit) {
        n = w.iter().fold(0u8, |v, b| v * 10 + (b - b'0'));
        if !(1..=63).contains(&n) {
            return None;
        }
        at = 1;
    }
    let unit = word(at);
    // The unit, plural after a number above one ("3 days").
    let one = |s: &str| is(unit, s) || (n > 1 && unit.strip_suffix(b"s").is_some_and(|u| is(u, s)));
    let (every, mut days) = if one("day") || (n == 1 && is(unit, "daily")) {
        (Every::Day, 0)
    } else if n == 1 && (is(unit, "weekday") || is(unit, "weekdays")) {
        (Every::Week, 0b1_1111)
    } else if one("week") || (n == 1 && is(unit, "weekly")) {
        (Every::Week, 0)
    } else if n == 1 && is(unit, "fortnight") {
        n = 2;
        (Every::Week, 0)
    } else if one("month") || (n == 1 && is(unit, "monthly")) {
        (Every::MonthDate, 0)
    } else if one("year") || (n == 1 && (is(unit, "yearly") || is(unit, "annually"))) {
        (Every::Year, 0)
    } else if day_name(unit).is_some() {
        // Weekday names, with "and" or commas between them.
        let mut mask = 0u8;
        while let Some(d) = day_name(word(at)) {
            mask |= 1 << d;
            at += 1;
            if is(word(at), "and") && day_name(word(at + 1)).is_some() {
                at += 1;
            }
        }
        at -= 1;
        (Every::Week, mask)
    } else {
        return None;
    };
    at += 1;
    let after = is(word(at), "after")
        && ["done", "completion", "completed", "finished"]
            .iter()
            .any(|s| is(word(at + 1), s));
    if after {
        at += 2;
        days = 0;
    }
    Some((
        Rule {
            every,
            n,
            days,
            until: None,
            after,
        },
        at,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u8, d: u8) -> i32 {
        date::days_from(date::Date {
            year: y,
            month: m,
            day: d,
        })
    }

    #[test]
    fn every_rule_packs_and_unpacks_to_itself() {
        let start = day(2024, 1, 1);
        for every in [
            Every::Day,
            Every::Week,
            Every::MonthDate,
            Every::MonthWeekday,
            Every::Year,
        ] {
            for (n, after) in [(1, false), (2, false), (63, false), (1, true), (63, true)] {
                if after && every == Every::MonthWeekday {
                    continue;
                }
                for until in [None, Some(start + 1), Some(start + 0xFFFF)] {
                    let r = Rule {
                        every,
                        n,
                        days: if after { 0 } else { 0b0010101 },
                        until,
                        after,
                    };
                    assert_eq!(Rule::unpack(r.pack(start), start), Some(r));
                }
            }
        }
        // A value no rule makes.
        assert_eq!(Rule::unpack(7, start), None);
        assert_eq!(Rule::unpack(0, start), None);
    }

    #[test]
    fn weekly_on_chosen_days_every_other_week() {
        let start = day(2024, 1, 1); // a Monday
        let r = Rule {
            every: Every::Week,
            n: 2,
            days: 0b101,
            until: None,
            after: false,
        }; // Mon, Wed
        assert!(r.occurs(start, start));
        assert!(r.occurs(start, day(2024, 1, 3)));
        assert!(!r.occurs(start, day(2024, 1, 2)));
        assert!(!r.occurs(start, day(2024, 1, 8))); // the week off
        assert!(r.occurs(start, day(2024, 1, 15)));
        assert!(!r.occurs(start, start - 1)); // never before it starts
    }

    #[test]
    fn monthly_by_date_skips_months_without_it() {
        let start = day(2024, 1, 31);
        let r = Rule {
            every: Every::MonthDate,
            n: 1,
            days: 0,
            until: None,
            after: false,
        };
        assert!(!r.occurs(start, day(2024, 2, 29)));
        assert!(r.occurs(start, day(2024, 3, 31)));
    }

    #[test]
    fn monthly_by_weekday_and_the_last_one() {
        // The 2nd Tuesday of January 2024 is the 9th.
        let r = Rule {
            every: Every::MonthWeekday,
            n: 1,
            days: 0,
            until: None,
            after: false,
        };
        let start = day(2024, 1, 9);
        assert!(r.occurs(start, day(2024, 2, 13)));
        assert!(!r.occurs(start, day(2024, 2, 6)));
        // The last Wednesday: 31 January 2024, then 28 February.
        let last = day(2024, 1, 31);
        assert!(r.occurs(last, day(2024, 2, 28)));
        assert!(!r.occurs(last, day(2024, 2, 21)));
    }

    #[test]
    fn yearly_birthdays_and_an_end() {
        let start = day(1990, 3, 14);
        let r = Rule {
            every: Every::Year,
            n: 1,
            days: 0,
            until: None,
            after: false,
        };
        assert!(r.occurs(start, day(2024, 3, 14)));
        assert!(!r.occurs(start, day(2024, 3, 15)));
        let ends = Rule {
            until: Some(day(2000, 1, 1)),
            ..r
        };
        assert!(!ends.occurs(start, day(2024, 3, 14)));
        // Every day, every 3rd.
        let d = Rule {
            every: Every::Day,
            n: 3,
            days: 0,
            until: None,
            after: false,
        };
        assert!(d.occurs(start, start + 9) && !d.occurs(start, start + 10));
    }

    #[test]
    fn a_done_task_moves_to_its_next_day() {
        let rule = |every, n, days, after| Rule {
            every,
            n,
            days,
            until: None,
            after,
        };
        let mon = day(2024, 1, 1);
        // Daily, ticked on time: tomorrow. Ticked 5 days late: the day
        // after it was done, not a late one.
        let daily = rule(Every::Day, 1, 0, false);
        assert_eq!(daily.next_after(mon, mon, mon), Some(mon + 1));
        assert_eq!(daily.next_after(mon, mon, mon + 5), Some(mon + 6));
        // Every 3 days keeps its own beat from the start.
        let three = rule(Every::Day, 3, 0, false);
        assert_eq!(three.next_after(mon, mon, mon + 4), Some(mon + 6));
        // Weekdays: Friday's goes to Monday.
        let weekdays = rule(Every::Week, 1, 0b1_1111, false);
        assert_eq!(weekdays.next_after(mon, mon + 4, mon + 4), Some(mon + 7));
        // Weekly on Tuesday and Thursday, every other week.
        let tt = rule(Every::Week, 2, 0b1010, false);
        let tue = mon + 1;
        assert_eq!(tt.next_after(tue, tue, tue), Some(tue + 2));
        assert_eq!(tt.next_after(tue, tue + 2, tue + 2), Some(tue + 14));
        // Monthly on the 31st skips the short months; done early, it
        // still moves past its own due day.
        let jan31 = day(2024, 1, 31);
        let monthly = rule(Every::MonthDate, 1, 0, false);
        assert_eq!(
            monthly.next_after(jan31, jan31, jan31 - 3),
            Some(day(2024, 3, 31))
        );
        // Yearly on the 29th of February: the next leap year.
        let leap = day(2024, 2, 29);
        let yearly = rule(Every::Year, 1, 0, false);
        assert_eq!(yearly.next_after(leap, leap, leap), Some(day(2028, 2, 29)));
        // After done: counted from the day it was done.
        let after = rule(Every::Day, 10, 0, true);
        assert_eq!(after.next_after(mon, mon, mon + 3), Some(mon + 13));
        let after_m = rule(Every::MonthDate, 1, 0, true);
        assert_eq!(
            after_m.next_after(jan31, jan31, jan31),
            Some(day(2024, 2, 29))
        );
        let after_y = rule(Every::Year, 2, 0, true);
        assert_eq!(after_y.next_after(leap, leap, leap), Some(day(2026, 2, 28)));
        let after_w = rule(Every::Week, 2, 0, true);
        assert_eq!(after_w.next_after(mon, mon, mon + 1), Some(mon + 15));
        // An after-done task shows only on its due day.
        assert!(after.occurs(mon, mon) && !after.occurs(mon, mon + 10));
        // An end: nothing after it.
        let ends = Rule {
            until: Some(mon + 1),
            ..daily
        };
        assert_eq!(ends.next_after(mon, mon, mon), Some(mon + 1));
        assert_eq!(ends.next_after(mon, mon + 1, mon + 1), None);
    }

    #[test]
    fn typed_repeats_read_as_people_write_them() {
        let read = |line: &str| {
            let words: [&[u8]; 8] =
                core::array::from_fn(|k| line.split(' ').nth(k).map_or(&b""[..], str::as_bytes));
            let n = line.split(' ').count().min(8);
            phrase(&words[..n]).map(|(r, used)| (r.every, r.n, r.days, r.after, used))
        };
        assert_eq!(read("day"), Some((Every::Day, 1, 0, false, 1)));
        assert_eq!(read("daily"), Some((Every::Day, 1, 0, false, 1)));
        assert_eq!(read("3 days"), Some((Every::Day, 3, 0, false, 2)));
        assert_eq!(read("weekday"), Some((Every::Week, 1, 0b1_1111, false, 1)));
        assert_eq!(read("other week"), Some((Every::Week, 2, 0, false, 2)));
        assert_eq!(read("fortnight"), Some((Every::Week, 2, 0, false, 1)));
        assert_eq!(
            read("mon wed fri"),
            Some((Every::Week, 1, 0b1_0101, false, 3))
        );
        assert_eq!(
            read("tuesday and thursday rent"),
            Some((Every::Week, 1, 0b1010, false, 3))
        );
        assert_eq!(read("mon, thu"), Some((Every::Week, 1, 0b1001, false, 2)));
        assert_eq!(read("2 months"), Some((Every::MonthDate, 2, 0, false, 2)));
        assert_eq!(read("year"), Some((Every::Year, 1, 0, false, 1)));
        assert_eq!(
            read("10 days after done"),
            Some((Every::Day, 10, 0, true, 4))
        );
        assert_eq!(
            read("month after completion x"),
            Some((Every::MonthDate, 1, 0, true, 3))
        );
        // Not repeats: the words stay in the title.
        assert_eq!(read("body"), None);
        assert_eq!(read("3 apples"), None);
        assert_eq!(read("99 days"), None);
        assert_eq!(read("0 days"), None);
        assert_eq!(read(""), None);
        // "after" without done is a word of the title.
        assert_eq!(read("day after lunch"), Some((Every::Day, 1, 0, false, 1)));
        // A plural needs a number: "every days" is no repeat.
        assert_eq!(read("days"), None);
    }

    #[test]
    fn a_rule_in_words_and_whether_one_chip_says_it() {
        // Wednesday 1 November 2023.
        let wed = day(2023, 11, 1);
        let say = |r: Rule, us: bool| {
            let mut b = [0u8; 64];
            let n = r.words(wed, us, &mut b);
            String::from_utf8(b[..n].to_vec()).unwrap()
        };
        let r = |every, n, days, until, after| Rule {
            every,
            n,
            days,
            until,
            after,
        };
        let weekly = r(Every::Week, 1, 0, None, false);
        assert!(weekly.plain(wed));
        // Its own weekday named is still plain.
        assert!(r(Every::Week, 1, 1 << 2, None, false).plain(wed));
        assert_eq!(say(weekly, true), "Every week");
        let two = r(Every::Week, 2, 1 | 1 << 2, Some(day(2024, 3, 3)), false);
        assert!(!two.plain(wed));
        assert_eq!(say(two, true), "Every 2 weeks on Mon, Wed until Mar 3");
        assert_eq!(say(two, false), "Every 2 weeks on Mon, Wed until 3 Mar");
        let nth = r(Every::MonthWeekday, 1, 0, None, false);
        assert!(!nth.plain(wed));
        assert_eq!(say(nth, true), "Every month on the 1st Wed");
        assert_eq!(
            Rule { ..nth }.words(day(2023, 11, 29), true, &mut [0u8; 64]),
            "Every month on the last Wed".len()
        );
        assert_eq!(say(r(Every::Day, 3, 0, None, false), true), "Every 3 days");
        // Runs of three as ranges, two as a pair; Sunday ends the week.
        assert_eq!(
            say(r(Every::Week, 1, 0b0101_1111, None, false), true),
            "Every week on Mon-Fri, Sun"
        );
        assert_eq!(
            say(r(Every::Week, 1, 0b0110_0011, None, false), true),
            "Every week on Mon, Tue, Sat, Sun"
        );
        assert_eq!(
            say(r(Every::Year, 1, 0, None, true), true),
            "1 year after done"
        );
        assert!(!r(Every::Day, 1, 0, None, true).plain(wed));
        assert!(!r(Every::Day, 1, 0, Some(wed + 9), false).plain(wed));
        // Cut at the end of a short buffer, never past it.
        let mut b = [0u8; 8];
        assert_eq!(two.words(wed, true, &mut b), 8);
        assert_eq!(&b, b"Every 2 ");
    }
}
