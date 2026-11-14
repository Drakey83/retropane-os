//! How an appointment or a task repeats (calendar-rule0.md and
//! tasks-rule0.md version 2), packed into one number of the store.
//!
//! Found: iCalendar's RRULE (RFC 5545) can say almost anything (BYSETPOS,
//! BYWEEKNO, ...), and Windows Mobile offered daily, weekly on days,
//! monthly by date or weekday, yearly. Ours stores the rules people make
//! (WM's set, plus every N and an end) in 32 bits, answers "does it occur
//! on day d" with arithmetic and no expansion, and the CalDAV/iCal reader
//! maps the RRULEs it meets onto it (the rest are kept as single events).

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
}

impl Rule {
    /// Packed for the store: bits 0-2 how often, 3-8 n, 9-15 the
    /// weekdays, 16-31 the end as days after the start (0: none).
    #[must_use]
    pub fn pack(&self, start: i32) -> u32 {
        let e = match self.every {
            Every::Day => 0,
            Every::Week => 1,
            Every::MonthDate => 2,
            Every::MonthWeekday => 3,
            Every::Year => 4,
        };
        let until = self
            .until
            .map_or(0, |u| (u - start).clamp(1, 0xFFFF) as u32);
        e | u32::from(self.n.clamp(1, 63)) << 3 | u32::from(self.days & 0x7F) << 9 | until << 16
    }

    /// Unpacked; `None` for a value no rule packs to.
    #[must_use]
    pub fn unpack(v: u32, start: i32) -> Option<Self> {
        let every = match v & 7 {
            0 => Every::Day,
            1 => Every::Week,
            2 => Every::MonthDate,
            3 => Every::MonthWeekday,
            4 => Every::Year,
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
            days: ((v >> 9) & 0x7F) as u8,
            until,
        })
    }

    /// Whether something starting on day `start` with this rule occurs on
    /// day `day`.
    #[must_use]
    pub fn occurs(&self, start: i32, day: i32) -> bool {
        if day < start || self.until.is_some_and(|u| day > u) {
            return false;
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
            for n in [1, 2, 63] {
                for until in [None, Some(start + 1), Some(start + 0xFFFF)] {
                    let r = Rule {
                        every,
                        n,
                        days: 0b0010101,
                        until,
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
        };
        assert!(d.occurs(start, start + 9) && !d.occurs(start, start + 10));
    }
}
