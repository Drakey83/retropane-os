//! The iCalendar reader and writer's tests: Google's, Outlook's and old
//! Thunderbird's zones, UTC and floating times, alarms, notes, round trips
//! both ways, and hostile input that must never panic.

extern crate std;
use super::*;
use std::vec::Vec;

fn all(text: &[u8]) -> Vec<Event> {
    let mut v = Vec::new();
    ics_each(text, |e| {
        v.push(*e);
        true
    });
    v
}

fn day(y: i32, m: u8, d: u8) -> i32 {
    date::days_from(Date {
        year: y,
        month: m,
        day: d,
    })
}

const ICS: &[u8] = b"BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:Dentist\\, Dr. Lee\r\nLOCATION:410 Pine St\r\nDTSTART:20240102T230000Z\r\nDTEND:20240103T000000Z\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Mum's bir\r\n thday\r\nDTSTART;VALUE=DATE:19600315\r\nRRULE:FREQ=YEARLY\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Stand-up\r\nDTSTART:20240101T093000\r\nDURATION:PT15M\r\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR;COUNT=6\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

#[test]
fn utc_floating_and_all_day_events_with_folds_escapes_and_rules() {
    let ev = all(ICS);
    assert_eq!(ev.len(), 3);
    assert_eq!(ev[0].title.get(), b"Dentist, Dr. Lee");
    assert_eq!(ev[0].place.get(), b"410 Pine St");
    // Kept in UTC with its zone: the views move it to the device's.
    assert_eq!(ev[0].zone.get(), b"UTC");
    assert_eq!(ev[0].start, date::start_of(day(2024, 1, 2)) + 23 * 60);
    assert_eq!(ev[0].minutes, 60);
    assert_eq!(ev[1].title.get(), b"Mum's birthday");
    assert!(ev[1].all_day);
    assert!(ev[1].zone.is_empty());
    assert_eq!(ev[1].rule.map(|r| r.every), Some(Every::Year));
    // A floating time: no zone.
    assert!(ev[2].zone.is_empty());
    assert_eq!(ev[2].minutes, 15);
    let r = ev[2].rule.expect("weekly kept");
    assert_eq!(r.days, 0b10101);
    // Six meetings from Monday 1 January: the 6th is Friday the 12th.
    assert_eq!(r.until, Some(day(2024, 1, 12)));
}

const GOOGLE: &[u8] = b"BEGIN:VCALENDAR\r\nPRODID:-//Google Inc//Google Calendar 70.9054//EN\r\nVERSION:2.0\r\nX-WR-TIMEZONE:America/New_York\r\nBEGIN:VEVENT\r\nDTSTART;TZID=America/New_York:20261109T090000\r\nDTEND;TZID=America/New_York:20261109T093000\r\nRRULE:FREQ=WEEKLY;BYDAY=MO\r\nSUMMARY:Team sync\r\nDESCRIPTION:Agenda:\\nfirst\\, the numbers\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:This is an event reminder\r\nTRIGGER:-P0DT0H10M0S\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

#[test]
fn a_google_event_keeps_its_zone_its_notes_and_its_alarm() {
    let e = all(GOOGLE)[0];
    assert_eq!(e.zone.get(), b"America/New_York");
    assert_eq!(e.start, date::start_of(day(2026, 11, 9)) + 9 * 60);
    assert_eq!(e.minutes, 30);
    assert_eq!(e.note.get(), b"Agenda:\nfirst, the numbers");
    assert_eq!(e.alarm, Some(10));
    assert_eq!(e.rule.map(|r| r.every), Some(Every::Week));
}

const OUTLOOK: &[u8] = b"BEGIN:VCALENDAR\r\nPRODID:-//Microsoft Corporation//Outlook 16.0 MIMEDIR//EN\r\nVERSION:2.0\r\nBEGIN:VTIMEZONE\r\nTZID:Pacific Standard Time\r\nBEGIN:STANDARD\r\nDTSTART:16011104T020000\r\nRRULE:FREQ=YEARLY;BYDAY=1SU;BYMONTH=11\r\nTZOFFSETFROM:-0700\r\nTZOFFSETTO:-0800\r\nEND:STANDARD\r\nBEGIN:DAYLIGHT\r\nDTSTART:16010311T020000\r\nRRULE:FREQ=YEARLY;BYDAY=2SU;BYMONTH=3\r\nTZOFFSETFROM:-0800\r\nTZOFFSETTO:-0700\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\nBEGIN:VTIMEZONE\r\nTZID:Strange Island Time\r\nBEGIN:STANDARD\r\nDTSTART:16010101T000000\r\nTZOFFSETFROM:+0517\r\nTZOFFSETTO:+0517\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nDTSTART;TZID=\"Pacific Standard Time\":20260704T120000\r\nDTEND;TZID=\"Pacific Standard Time\":20260704T150000\r\nSUMMARY:Picnic\r\nBEGIN:VALARM\r\nTRIGGER;RELATED=END:-PT5M\r\nEND:VALARM\r\nBEGIN:VALARM\r\nTRIGGER:-PT1H\r\nEND:VALARM\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nDTSTART;TZID=Strange Island Time:20260101T051700\r\nSUMMARY:Odd\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nDTSTART;TZID=/mozilla.org/20050126_1/Europe/Berlin:20260601T100000\r\nSUMMARY:Thunderbird\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nDTSTART;TZID=Nowhere/Special:20260601T100000\r\nSUMMARY:Unknown zone\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

#[test]
fn outlook_zones_are_matched_to_their_tzdata_names() {
    let ev = all(OUTLOOK);
    assert_eq!(ev.len(), 4);
    // "Pacific Standard Time" has Los Angeles's rule.
    assert_eq!(ev[0].zone.get(), b"America/Los_Angeles");
    assert_eq!(ev[0].start, date::start_of(day(2026, 7, 4)) + 12 * 60);
    assert_eq!(ev[0].minutes, 180);
    // The alarm before the end is not one before the start: the next is.
    assert_eq!(ev[0].alarm, Some(60));
    // A zone no tzdata name has: kept in UTC (+5:17 off).
    assert_eq!(ev[1].zone.get(), b"UTC");
    assert_eq!(ev[1].start, date::start_of(day(2026, 1, 1)));
    // Thunderbird's old path names.
    assert_eq!(ev[2].zone.get(), b"Europe/Berlin");
    // A zone named but never defined: a floating time, never refused.
    assert!(ev[3].zone.is_empty());
    assert_eq!(ev[3].title.get(), b"Unknown zone");
}

fn out<'a>(e: &'a Event, id: u32) -> Out<'a> {
    Out {
        id,
        title: e.title.get(),
        place: e.place.get(),
        note: e.note.get(),
        start: e.start,
        minutes: e.minutes,
        all_day: e.all_day,
        rule: e.rule,
        zone: e.zone.get(),
        alarm: e.alarm,
    }
}

fn write_all(ev: &[Event]) -> Vec<u8> {
    let mut b = Vec::new();
    let zones: Vec<&[u8]> = ev
        .iter()
        .map(|e| e.zone.get())
        .filter(|z| !z.is_empty())
        .collect();
    head(&zones, |s| b.extend_from_slice(s));
    for (k, e) in ev.iter().enumerate() {
        event(&out(e, k as u32 + 1), 29_000_000, |s| {
            b.extend_from_slice(s)
        });
    }
    tail(|s| b.extend_from_slice(s));
    b
}

#[test]
fn events_round_trip_through_a_written_file() {
    let mut ev = all(GOOGLE);
    ev.extend(all(OUTLOOK));
    ev.extend(all(ICS).into_iter().skip(1));
    let mut long = Event::new();
    long.title.push(b"A title; with, every \\ escape");
    long.note.push(b"Line one\nline two of a note long enough to fold past seventy-five bytes, twice over, for sure");
    long.start = date::start_of(day(2026, 12, 31)) + 23 * 60 + 30;
    long.minutes = 90;
    long.zone.push(b"Australia/Sydney");
    long.rule = Some(Rule {
        every: Every::MonthWeekday,
        n: 2,
        days: 0,
        until: Some(day(2027, 12, 31)),
    });
    ev.push(long);
    let file = write_all(&ev);
    for line in file.split(|&c| c == b'\n') {
        assert!(line.len() <= 76);
    }
    let back = all(&file);
    assert_eq!(back.len(), ev.len());
    for (a, b) in ev.iter().zip(&back) {
        assert_eq!(a.title, b.title);
        assert_eq!(a.place, b.place);
        assert_eq!(a.note, b.note);
        assert_eq!(a.start, b.start, "{:?}", a.title);
        assert_eq!(a.minutes, b.minutes, "{:?}", a.title);
        assert_eq!(a.all_day, b.all_day);
        assert_eq!(a.zone, b.zone, "{:?}", a.title);
        assert_eq!(a.alarm, b.alarm);
        match (a.rule, b.rule) {
            (Some(x), Some(y)) => {
                assert_eq!(x.every, y.every);
                assert_eq!(x.n, y.n);
                assert_eq!(x.days, y.days);
                assert_eq!(x.until, y.until, "{:?}", a.title);
            }
            (None, None) => {}
            other => panic!("{other:?}"),
        }
    }
    // The written zones read back as the same rules (the file's own
    // VTIMEZONE is matched, the TZID found by name as well).
    let mut zones = [None; FILE_ZONES];
    file_zones(&file, &mut zones);
    let ny = zones
        .iter()
        .flatten()
        .find(|z| z.tzid.get() == b"America/New_York")
        .unwrap();
    assert_eq!(
        ny.rule,
        crate::zone::find(b"America/New_York").unwrap().rule()
    );
    let syd = zones
        .iter()
        .flatten()
        .find(|z| z.tzid.get() == b"Australia/Sydney")
        .unwrap();
    assert_eq!(
        syd.rule,
        crate::zone::find(b"Australia/Sydney").unwrap().rule()
    );
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u8 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (self.0 >> 33) as u8
    }
}

#[test]
fn hostile_files_never_panic() {
    assert!(all(b"").is_empty());
    assert!(all(b"BEGIN:VEVENT\nDTSTART:2024XX01\nEND:VEVENT\n").is_empty());
    assert!(all(b"BEGIN:VEVENT\nDTSTART:20240101T996000\nEND:VEVENT\n").is_empty());
    for file in [ICS, GOOGLE, OUTLOOK] {
        for cut in 0..file.len() {
            let _ = all(&file[..cut]);
        }
    }
    let mut r = Lcg(11);
    for _ in 0..300 {
        let mut f = OUTLOOK.to_vec();
        for _ in 0..20 {
            let at = usize::from(r.next()) * f.len() / 256;
            f[at] = r.next();
        }
        let _ = all(&f);
    }
    // Huge numbers, a megabyte line, a thousand zones.
    let _ = all(b"BEGIN:VEVENT\nDTSTART:20240101T100000\nDURATION:P99999999999W\nRRULE:FREQ=DAILY;INTERVAL=999999;COUNT=99999999999\nEND:VEVENT\n");
    let mut big = b"BEGIN:VEVENT\nSUMMARY:".to_vec();
    big.extend(std::iter::repeat_n(b'x', 1 << 20));
    big.extend_from_slice(b"\nDTSTART:20240101T100000\nEND:VEVENT\n");
    for _ in 0..1000 {
        big.extend_from_slice(b"BEGIN:VTIMEZONE\nTZID:Z\nBEGIN:STANDARD\nTZOFFSETTO:+0100\nEND:STANDARD\nEND:VTIMEZONE\n");
    }
    let ev = all(&big);
    assert_eq!(ev.len(), 1);
    assert!(ev[0].title.is_full());
    assert_eq!(utc_offset(b"+9999"), None);
    assert_eq!(utc_offset(b"0100"), None);
}
