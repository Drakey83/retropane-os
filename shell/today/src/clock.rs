//! Wall-clock time from a count of minutes, and where the count starts:
//! the PC's time at the load (`RPTIM`, written by `cargo xtask load` beside
//! the settings), else the build machine's (R182: the build's time alone
//! went stale, a build of changed code keeping the minute `build.rs` last
//! ran, and Today read SUN 1 NOVEMBER on Monday the 2nd).

use rp_shell::Time;

/// The time file's magic.
pub const MAGIC: &[u8; 4] = b"RPTM";
/// Its length: magic, UTC minutes (i64), the local offset in minutes
/// (i32), a CRC-32 of the first 16 bytes.
pub const LEN: usize = 20;
/// The minutes accepted: 2020-01-01 to 2100-01-01 (hostile or damaged
/// values never set the clock).
const SANE: core::ops::Range<i64> = 26_297_280..68_383_680;
/// The local offsets accepted, minutes (UTC-14 to UTC+14).
const OFFSET: core::ops::RangeInclusive<i32> = -14 * 60..=14 * 60;

/// The time file for `utc` minutes since 1970 and `offset` minutes from UTC.
#[must_use]
pub fn card_time(utc: i64, offset: i32) -> [u8; LEN] {
    let mut f = [0u8; LEN];
    f[..4].copy_from_slice(MAGIC);
    f[4..12].copy_from_slice(&utc.to_le_bytes());
    f[12..16].copy_from_slice(&offset.to_le_bytes());
    let crc = rp_weather::store::crc32(&f[..16]);
    f[16..].copy_from_slice(&crc.to_le_bytes());
    f
}

/// The local minutes (UTC plus its offset) a time file holds, checked as
/// hostile input: `None` if it is short, damaged, or out of range.
#[must_use]
pub fn from_card(f: &[u8]) -> Option<i64> {
    let f = f.get(..LEN)?;
    if &f[..4] != MAGIC {
        return None;
    }
    let crc = u32::from_le_bytes(f[16..20].try_into().ok()?);
    if rp_weather::store::crc32(&f[..16]) != crc {
        return None;
    }
    let utc = i64::from_le_bytes(f[4..12].try_into().ok()?);
    let off = i32::from_le_bytes(f[12..16].try_into().ok()?);
    (SANE.contains(&utc) && OFFSET.contains(&off)).then_some(utc + i64::from(off))
}

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

    /// R182: the PC's time at the load reads back as local minutes; a
    /// damaged, foreign, short or out-of-range file never sets the clock.
    #[test]
    fn the_load_time_reads_back_and_bad_files_are_refused() {
        // Monday 2026-11-02 23:11 in UTC-6: Tuesday 05:11 UTC.
        let utc = 20_760 * 24 * 60 + 5 * 60 + 11;
        let f = card_time(utc, -6 * 60);
        let local = from_card(&f).expect("a good file");
        let t = time_at(local);
        assert_eq!(
            (t.weekday, t.day, t.month, t.hour, t.minute),
            (0, 2, 11, 23, 11)
        );
        let mut bad = f;
        bad[5] ^= 1;
        assert_eq!(from_card(&bad), None, "a changed byte");
        assert_eq!(from_card(&f[..LEN - 1]), None, "short");
        let mut foreign = f;
        foreign[0] = b'X';
        assert_eq!(from_card(&foreign), None);
        assert_eq!(from_card(&card_time(0, 0)), None, "1970");
        assert_eq!(
            from_card(&card_time(utc, 15 * 60)),
            None,
            "an offset past 14 h"
        );
    }

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
