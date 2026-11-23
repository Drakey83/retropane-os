//! The Birthdays calendar (calendar-rule0.md, contacts-rule0.md): each
//! contact's birthday and anniversary ([`crate::contact::Day`]) as an
//! all-day event on the day it falls, worked out from the contact itself
//! each time, never copied into appointments.
//!
//! Found: Google Calendar copies its contacts' birthdays into a calendar of
//! its own on the server (a change in Contacts shows there hours later);
//! iOS works them out from the contacts, as here; Windows Mobile 5 made an
//! appointment per birthday when one was typed, which stayed behind when
//! the contact changed or went. Ours reads the one store: a birthday typed
//! in Contacts is in Calendar and on Today at once, gone with its contact,
//! and the age is the year's own ("Sam's 41st birthday"), with no records
//! made at all.

use crate::contact::Day;
use crate::date;
use crate::store::{Kind, Rec, Store, tag};

/// The calendar record's [`tag::SOURCE`]: this device's contacts.
pub const SOURCE: &[u8] = b"contacts";

/// Which day of a contact.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum What {
    /// Their birthday.
    Birthday,
    /// Their anniversary.
    Anniversary,
}

/// One contact's day falling on a day.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Event {
    /// The contact's record.
    pub id: u32,
    /// The day (days from 1970).
    pub day: i32,
    /// Birthday or anniversary.
    pub what: What,
    /// The years it is on that day (an age, the years married), when the
    /// year is known and it is not the day itself.
    pub years: Option<u32>,
}

/// Contact `r`'s days falling from `from` for `days` days, in order of
/// birthday then anniversary, each year's: given to `out`.
pub fn of(r: &Rec<'_>, from: i32, days: i32, mut out: impl FnMut(Event)) {
    for (t, what) in [
        (tag::BIRTHDAY, What::Birthday),
        (tag::ANNIVERSARY, What::Anniversary),
    ] {
        let Some(d) = Day::of_field(r.text(t)) else {
            continue;
        };
        let (first, last) = (
            date::date_of(from).year,
            date::date_of(from + days - 1).year,
        );
        for y in first..=last {
            let day = d.in_year(y);
            if day < from || day >= from + days || d.year.is_some_and(|b| y < b) {
                continue;
            }
            out(Event {
                id: r.id,
                day,
                what,
                years: d
                    .year
                    .and_then(|b| u32::try_from(y - b).ok())
                    .filter(|&n| n > 0),
            });
        }
    }
}

/// Every contact's days from `from` for `days` days (in the store's order).
pub fn all(store: &Store<'_>, from: i32, days: i32, mut out: impl FnMut(Event)) {
    for r in store.all(Kind::Contact) {
        of(&r, from, days, &mut out);
    }
}

/// The calendar record holding the Birthdays calendar's name, colour and
/// whether it is shown, if made.
#[must_use]
pub fn calendar(store: &Store<'_>) -> Option<u32> {
    store
        .all(Kind::Calendar)
        .find(|c| c.text(tag::SOURCE) == SOURCE)
        .map(|c| c.id)
}

/// Whether any contact has a birthday or an anniversary.
#[must_use]
pub fn any(store: &Store<'_>) -> bool {
    store.all(Kind::Contact).any(|r| {
        Day::of_field(r.text(tag::BIRTHDAY)).is_some()
            || Day::of_field(r.text(tag::ANNIVERSARY)).is_some()
    })
}

/// The event's title in `out`: "Sam Lee's 41st birthday", "Sam Lee's
/// birthday" (no year), "Ann and Bo's 10th anniversary" is the contact's
/// name as kept ("First Last", else the company). Its length.
pub fn title(r: &Rec<'_>, e: &Event, out: &mut [u8]) -> usize {
    let mut n = 0;
    let mut put = |s: &[u8], n: &mut usize| {
        for &c in s {
            if *n < out.len() {
                out[*n] = c;
                *n += 1;
            }
        }
    };
    let (first, last) = (r.text(tag::FIRST), r.text(tag::LAST));
    if first.is_empty() && last.is_empty() {
        put(r.text(tag::COMPANY), &mut n);
    } else {
        put(first, &mut n);
        if !first.is_empty() && !last.is_empty() {
            put(b" ", &mut n);
        }
        put(last, &mut n);
    }
    // "James's", as most style guides write it today.
    put(b"'s ", &mut n);
    if let Some(y) = e.years {
        let mut d = [0u8; 10];
        let mut k = d.len();
        let mut v = y;
        loop {
            k -= 1;
            d[k] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        put(&d[k..], &mut n);
        let suffix: &[u8] = match (y % 100, y % 10) {
            (11..=13, _) => b"th",
            (_, 1) => b"st",
            (_, 2) => b"nd",
            (_, 3) => b"rd",
            _ => b"th",
        };
        put(suffix, &mut n);
        put(b" ", &mut n);
    }
    put(
        match e.what {
            What::Birthday => b"birthday",
            What::Anniversary => b"anniversary",
        },
        &mut n,
    );
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::date::Date;

    fn day(year: i32, month: u8, d: u8) -> i32 {
        date::days_from(Date {
            year,
            month,
            day: d,
        })
    }

    #[test]
    fn birthdays_and_anniversaries_fall_on_their_days_with_their_years() {
        let mut mem = vec![0u8; crate::store::MEMORY];
        let mut s = Store::new(&mut mem);
        s.load_start(None);
        let born = Day {
            year: Some(1983),
            month: 3,
            day: 14,
        }
        .field()
        .unwrap();
        // No year; and the 29th of February.
        let wed = Day {
            year: None,
            month: 2,
            day: 29,
        }
        .field()
        .unwrap();
        let sam = s
            .put(
                Kind::Contact,
                None,
                &[
                    (tag::FIRST, b"Sam"),
                    (tag::LAST, b"Lee"),
                    (tag::BIRTHDAY, &born),
                    (tag::ANNIVERSARY, &wed),
                ],
            )
            .unwrap();
        s.put(Kind::Contact, None, &[(tag::FIRST, b"Jo")]).unwrap();
        assert!(any(&s));
        let mut seen = vec![];
        // 1 January 2024 for two years and a bit.
        all(&s, day(2024, 1, 1), 800, |e| seen.push(e));
        let days: Vec<_> = seen.iter().map(|e| (e.day, e.what, e.years)).collect();
        assert_eq!(
            days,
            [
                (day(2024, 3, 14), What::Birthday, Some(41)),
                (day(2025, 3, 14), What::Birthday, Some(42)),
                (day(2024, 2, 29), What::Anniversary, None),
                // Not a leap year: the 28th.
                (day(2025, 2, 28), What::Anniversary, None),
                (day(2026, 2, 28), What::Anniversary, None),
            ]
        );
        let r = s.get(sam).unwrap();
        let mut b = [0u8; 40];
        let n = title(&r, &seen[0], &mut b);
        assert_eq!(&b[..n], b"Sam Lee's 41st birthday");
        let n = title(&r, &seen[2], &mut b);
        assert_eq!(&b[..n], b"Sam Lee's anniversary");
        for (y, want) in [
            (1, "1st"),
            (2, "2nd"),
            (3, "3rd"),
            (11, "11th"),
            (12, "12th"),
            (13, "13th"),
            (22, "22nd"),
            (101, "101st"),
            (111, "111th"),
        ] {
            let e = Event {
                years: Some(y),
                ..seen[0]
            };
            let n = title(&r, &e, &mut b);
            assert_eq!(&b[..n], format!("Sam Lee's {want} birthday").as_bytes());
        }
        // Never before the year it began, and no age on the day itself.
        let mut early = vec![];
        all(&s, day(1983, 1, 1), 365, |e| early.push(e));
        assert_eq!(early.len(), 2);
        assert_eq!(early[0].years, None);
        assert_eq!(calendar(&s), None);
        let cal = s
            .put(
                Kind::Calendar,
                None,
                &[(tag::NAME, b"Birthdays"), (tag::SOURCE, SOURCE)],
            )
            .unwrap();
        assert_eq!(calendar(&s), Some(cal));
    }
}
