//! Calendar arithmetic on whole days and minutes: no time zones, no float.
//!
//! A day is its count from 1970-01-01 (day 0, a Thursday); a moment is
//! local minutes from the same start, as the shell's clock counts
//! (`rp_today::clock`). Howard Hinnant's civil-date algorithms, exact for
//! every date the proleptic Gregorian calendar has.

/// Minutes in a day.
pub const DAY_MIN: u32 = 24 * 60;

/// A calendar date.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Date {
    /// The year (1970 and later here).
    pub year: i32,
    /// 1..=12.
    pub month: u8,
    /// 1..=31.
    pub day: u8,
}

/// Whether `year` has a 29 February.
#[must_use]
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub const fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// The days in `month` (1-12) of `year`; 0 for a month that isn't one.
#[must_use]
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub const fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// The day count of a date (a day past its month's end counts on into the
/// next: the caller checks with [`valid`]).
#[must_use]
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub const fn days_from(d: Date) -> i32 {
    let y = if d.month <= 2 { d.year - 1 } else { d.year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (d.month as i32 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d.day as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day count.
#[must_use]
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub const fn date_of(days: i32) -> Date {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    Date { year, month, day }
}

/// Whether the date exists.
#[must_use]
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub const fn valid(d: Date) -> bool {
    d.month >= 1 && d.month <= 12 && d.day >= 1 && d.day <= days_in_month(d.year, d.month)
}

/// The weekday of a day count: 0 Monday ..= 6 Sunday.
#[must_use]
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub const fn weekday(days: i32) -> u8 {
    (days + 3).rem_euclid(7) as u8
}

/// The day a moment (local minutes) falls on.
#[must_use]
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub const fn day_of(minutes: u32) -> i32 {
    (minutes / DAY_MIN) as i32
}

/// The first minute of a day (0 for a day before 1970).
#[must_use]
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub const fn start_of(days: i32) -> u32 {
    if days < 0 { 0 } else { days as u32 * DAY_MIN }
}

/// The first day of the month `months` after `d`'s (negative: before).
#[must_use]
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub const fn month_start(d: Date, months: i32) -> Date {
    let m = d.year * 12 + d.month as i32 - 1 + months;
    Date {
        year: m.div_euclid(12),
        month: (m.rem_euclid(12) + 1) as u8,
        day: 1,
    }
}

/// The three-letter month names.
pub const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
/// The full month names.
pub const MONTHS_LONG: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
/// The three-letter weekday names, Monday first.
pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

#[cfg(test)]
mod tests {
    use super::*;

    const fn d(year: i32, month: u8, day: u8) -> Date {
        Date { year, month, day }
    }

    #[test]
    fn known_days_and_weekdays() {
        assert_eq!(days_from(d(1970, 1, 1)), 0);
        assert_eq!(weekday(0), 3); // a Thursday
        assert_eq!(days_from(d(2000, 3, 1)), 11_017);
        assert_eq!(days_from(d(2026, 11, 13)), 20_770);
        assert_eq!(weekday(days_from(d(2026, 11, 13))), 4); // a Friday
        assert_eq!(weekday(days_from(d(2024, 1, 1))), 0); // a Monday
        assert_eq!(weekday(-1), 2); // Wednesday 1969-12-31
    }

    #[test]
    fn every_day_from_1970_to_2100_round_trips_in_order() {
        let mut prev = date_of(-1);
        for n in 0..days_from(d(2100, 12, 31)) + 1 {
            let x = date_of(n);
            assert!(valid(x), "{x:?}");
            assert_eq!(days_from(x), n);
            // The next day: the same month a day on, or the 1st of the next.
            let next_month = month_start(prev, 1);
            assert!(
                (x.year, x.month, x.day) == (prev.year, prev.month, prev.day + 1)
                    || (x.day == 1 && (x.year, x.month) == (next_month.year, next_month.month)),
                "{prev:?} then {x:?}"
            );
            prev = x;
        }
    }

    #[test]
    fn leap_years_and_month_lengths_at_the_edges() {
        assert!(
            is_leap(2000) && is_leap(2024) && !is_leap(1900) && !is_leap(2100) && !is_leap(2026)
        );
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2100, 2), 28);
        assert_eq!(days_in_month(2026, 13), 0);
        assert!(!valid(d(2026, 2, 29)) && valid(d(2024, 2, 29)));
        assert!(!valid(d(2026, 4, 31)) && !valid(d(2026, 0, 1)) && !valid(d(2026, 1, 0)));
    }

    #[test]
    fn months_step_across_years_both_ways() {
        assert_eq!(month_start(d(2026, 11, 13), 1), d(2026, 12, 1));
        assert_eq!(month_start(d(2026, 11, 13), 2), d(2027, 1, 1));
        assert_eq!(month_start(d(2026, 1, 31), -1), d(2025, 12, 1));
        assert_eq!(month_start(d(2026, 3, 31), -14), d(2025, 1, 1));
        assert_eq!(month_start(d(2026, 6, 30), 0), d(2026, 6, 1));
    }

    #[test]
    fn minutes_and_days_agree() {
        let day = days_from(d(2026, 11, 13));
        assert_eq!(day_of(start_of(day)), day);
        assert_eq!(day_of(start_of(day) + DAY_MIN - 1), day);
        assert_eq!(day_of(start_of(day) + DAY_MIN), day + 1);
        assert_eq!(start_of(-5), 0);
    }
}
