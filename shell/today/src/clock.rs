//! Wall-clock time from a count of minutes (there is no RTC driver yet:
//! the image starts from the build machine's local time).

use rp_shell::Time;

/// The local time `minutes` after 1970-01-01 00:00.
#[must_use]
pub fn time_at(minutes: i64) -> Time {
    let days = minutes.div_euclid(24 * 60);
    let in_day = minutes.rem_euclid(24 * 60);
    // 1970-01-01 was a Thursday (weekday 3 with Monday as 0).
    let weekday = (days + 3).rem_euclid(7) as u8;
    let (_, month, day) = civil(days);
    Time {
        hour: (in_day / 60) as u8,
        minute: (in_day % 60) as u8,
        weekday,
        day,
        month,
    }
}

/// Year, month (1-12) and day (1-31) of the day `days` after 1970-01-01
/// (Howard Hinnant's days-to-civil algorithm).
#[must_use]
pub fn civil(days: i64) -> (i64, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(-1), (1969, 12, 31));
        assert_eq!(civil(11_016), (2000, 2, 29));
        // 2026-10-13, a Tuesday.
        let days = 20_739;
        assert_eq!(civil(days), (2026, 10, 13));
        let t = time_at(days * 24 * 60 + 9 * 60 + 41);
        assert_eq!(
            t,
            Time {
                hour: 9,
                minute: 41,
                weekday: 1,
                day: 13,
                month: 10
            }
        );
    }
}
