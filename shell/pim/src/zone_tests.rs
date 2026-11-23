//! The zones' tests: the US, European and southern clock changes on
//! their days, the skipped and repeated hours, every zone of the table
//! readable, names both ways.

extern crate std;
use super::*;
use crate::date::{Date, days_from};

fn at(y: i32, m: u8, d: u8, h: i64, min: i64) -> i64 {
    i64::from(days_from(Date {
        year: y,
        month: m,
        day: d,
    })) * 1440
        + h * 60
        + min
}

fn rule(name: &str) -> Rule {
    find(name.as_bytes()).expect(name).rule()
}

#[test]
fn new_york_changes_on_the_second_sunday_of_march_and_first_of_november() {
    let ny = rule("America/New_York");
    // 2026: 8 March and 1 November.
    assert_eq!(
        ny.on.day(2026),
        days_from(Date {
            year: 2026,
            month: 3,
            day: 8
        })
    );
    assert_eq!(
        ny.off.day(2026),
        days_from(Date {
            year: 2026,
            month: 11,
            day: 1
        })
    );
    // 1:59 AM EST is 6:59 UTC; at 7:00 UTC it is 3:00 AM EDT.
    assert_eq!(ny.offset_at(at(2026, 3, 8, 6, 59)), -300);
    assert_eq!(ny.offset_at(at(2026, 3, 8, 7, 0)), -240);
    // 1:59 AM EDT is 5:59 UTC; at 6:00 UTC it is 1:00 AM EST.
    assert_eq!(ny.offset_at(at(2026, 11, 1, 5, 59)), -240);
    assert_eq!(ny.offset_at(at(2026, 11, 1, 6, 0)), -300);
}

#[test]
fn the_skipped_hour_and_the_repeated_one() {
    let ny = rule("America/New_York");
    // 2:30 AM on 8 March 2026 doesn't exist: read as 3:30 AM EDT.
    let u = ny.utc_of(at(2026, 3, 8, 2, 30));
    assert_eq!(u, at(2026, 3, 8, 7, 30));
    // 1:30 AM on 1 November 2026 happens twice: the first (EDT).
    assert_eq!(ny.utc_of(at(2026, 11, 1, 1, 30)), at(2026, 11, 1, 5, 30));
    // Ordinary times both sides.
    assert_eq!(ny.utc_of(at(2026, 1, 15, 9, 0)), at(2026, 1, 15, 14, 0));
    assert_eq!(ny.utc_of(at(2026, 7, 15, 9, 0)), at(2026, 7, 15, 13, 0));
}

#[test]
fn a_weekly_meeting_keeps_its_hour_across_the_change() {
    // 9 AM in New York shown in Los Angeles: 6 AM all year, both sides of
    // both changes (the two change on the same days).
    let (ny, la) = (rule("America/New_York"), rule("America/Los_Angeles"));
    for (m, d) in [(1, 5), (3, 2), (3, 9), (7, 6), (11, 2), (12, 28)] {
        let shown = ny.to(at(2026, m, d, 9, 0), &la);
        assert_eq!(shown, at(2026, m, d, 6, 0), "{m}/{d}");
    }
    // London to New York: 2 PM in London is 9 AM, but 10 AM in the weeks
    // the US has changed and Europe hasn't (9-28 March 2026).
    let lon = rule("Europe/London");
    assert_eq!(lon.to(at(2026, 3, 2, 14, 0), &ny), at(2026, 3, 2, 9, 0));
    assert_eq!(lon.to(at(2026, 3, 16, 14, 0), &ny), at(2026, 3, 16, 10, 0));
    assert_eq!(lon.to(at(2026, 4, 6, 14, 0), &ny), at(2026, 4, 6, 9, 0));
}

#[test]
fn the_south_has_summer_over_the_new_year() {
    let syd = rule("Australia/Sydney");
    assert_eq!(syd.offset_at(at(2026, 1, 15, 0, 0)), 660);
    assert_eq!(syd.offset_at(at(2026, 7, 15, 0, 0)), 600);
    let auck = rule("Pacific/Auckland");
    assert_eq!(auck.offset_at(at(2026, 12, 25, 0, 0)), 780);
    // Phoenix and Tokyo never change; India is half an hour off.
    assert!(!rule("America/Phoenix").has_summer());
    assert_eq!(rule("Asia/Tokyo").offset_at(0), 540);
    assert_eq!(rule("Asia/Kolkata").offset_at(0), 330);
    assert_eq!(rule("Asia/Calcutta"), rule("Asia/Kolkata"));
}

#[test]
fn every_zone_reads_and_finds_itself() {
    let mut n = 0;
    let mut buf = [0u8; 64];
    for z in all() {
        let len = z.name(&mut buf);
        assert!(len > 0);
        assert_eq!(
            find(&buf[..len]),
            Some(z),
            "{}",
            std::str::from_utf8(&buf[..len]).unwrap()
        );
        let r = z.rule();
        // Every moment of a year has an offset, and a wall time read back
        // is the same moment.
        for h in (0..366 * 24).step_by(97) {
            let u = at(2026, 1, 1, h, 0);
            let off = r.offset_at(u);
            assert!((-720..=840).contains(&off));
            assert_eq!(
                r.utc_of(u + i64::from(off)),
                u,
                "{}",
                std::str::from_utf8(&buf[..len]).unwrap()
            );
        }
        n += 1;
    }
    assert!(n > 500);
    let mut p = [0u8; 32];
    let z = find(b"america/port-au-prince").unwrap();
    let k = z.place(&mut p);
    assert_eq!(&p[..k], b"Port-au-Prince");
    let z = find(b"America/New_York").unwrap();
    let k = z.place(&mut p);
    assert_eq!(&p[..k], b"New York");
    assert_eq!(z.country(), *b"US");
    assert_eq!(find(b""), None);
    assert_eq!(find(b"Mars/Olympus_Mons"), None);
}

#[test]
fn a_rule_or_an_offset_finds_a_us_zone_first() {
    let mut buf = [0u8; 48];
    let z = of_rule(&rule("America/Chicago")).unwrap();
    let n = z.name(&mut buf);
    assert_eq!(&buf[..n], b"America/Chicago");
    // -7 hours in January: Denver (not Phoenix, not Edmonton).
    let z = guess(at(2026, 1, 15, 12, 0), -420).unwrap();
    let n = z.name(&mut buf);
    assert_eq!(&buf[..n], b"America/Denver");
    // +1 hour in January: a European zone.
    let z = guess(at(2026, 1, 15, 12, 0), 60).unwrap();
    assert_eq!(z.rule().offset_at(at(2026, 1, 15, 12, 0)), 60);
    assert_eq!(guess(0, 7), None);
}
