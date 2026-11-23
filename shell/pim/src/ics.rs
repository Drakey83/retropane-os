//! iCalendar files read and written (RFC 5545; calendar-rule0.md ".ics
//! import and export" and "Time zones"): an online calendar's link, a
//! file from the PC or the card, and the calendar written back out.
//!
//! Every file is hostile input (security.md): read with bounds on every
//! length, never trusting a count or an offset in it; a malformed event is
//! left out, never a panic; an old file without zones, alarms or notes is
//! read for what it has (the legacy rule).
//!
//! Iterate, don't imitate (rule 0): Windows Mobile read no .ics at all
//! (ActiveSync only); desktop readers (libical) expand a tree of
//! components with the whole tz database behind it. Ours reads in one pass
//! with a 256-byte line and keeps what the store holds (title, place,
//! notes, start, length, all-day, the repeat rules people make, the first
//! alarm). A time keeps its zone: a tzdata TZID as it is, a file's own
//! VTIMEZONE (Outlook's "Pacific Standard Time") matched rule for rule to
//! its tzdata name ([`crate::zone::of_rule`]), a time in UTC as UTC; the
//! views show each in the device's zone. Written files carry a VTIMEZONE
//! for every zone they use, made from the same rules, so Outlook (which
//! needs one) and Google (which doesn't) both read them.

use crate::date::{self, Date};
use crate::net::Short;
use crate::repeat::{Every, Rule};
use crate::zone::{self, Edge};

/// One event read from an iCalendar file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    /// SUMMARY.
    pub title: Short<64>,
    /// LOCATION.
    pub place: Short<64>,
    /// DESCRIPTION.
    pub note: Short<256>,
    /// The start: the wall time in [`Event::zone`], minutes from 1970.
    pub start: u32,
    /// Minutes long (0 for all-day: one day).
    pub minutes: u32,
    /// All day.
    pub all_day: bool,
    /// How it repeats, when its RRULE is one the store keeps.
    pub rule: Option<Rule>,
    /// Its time zone's tzdata name ("America/New_York", "UTC"); empty: a
    /// floating time, the device's own.
    pub zone: Short<40>,
    /// The first alarm, minutes before the start.
    pub alarm: Option<u32>,
}

impl Event {
    const fn new() -> Self {
        Self {
            title: Short::new(),
            place: Short::new(),
            note: Short::new(),
            start: 0,
            minutes: 60,
            all_day: false,
            rule: None,
            zone: Short::new(),
            alarm: None,
        }
    }
}

/// The file's lines unfolded (RFC 5545 3.1: a line starting with a space
/// or a tab continues the one before), one at a time, into `buf`.
struct Lines<'a> {
    text: &'a [u8],
    at: usize,
}

impl Lines<'_> {
    fn next_into<'b>(&mut self, buf: &'b mut [u8; 256]) -> Option<&'b [u8]> {
        if self.at >= self.text.len() {
            return None;
        }
        let mut n = 0;
        loop {
            let rest = &self.text[self.at..];
            let end = rest.iter().position(|&c| c == b'\n').unwrap_or(rest.len());
            let mut line = &rest[..end];
            if line.last() == Some(&b'\r') {
                line = &line[..line.len() - 1];
            }
            let take = line.len().min(256 - n);
            buf[n..n + take].copy_from_slice(&line[..take]);
            n += take;
            self.at += end + 1;
            match self.text.get(self.at) {
                Some(b' ' | b'\t') => self.at += 1,
                _ => break,
            }
        }
        Some(&buf[..n])
    }
}

fn num(a: &[u8]) -> Option<i32> {
    if a.is_empty() {
        return None;
    }
    a.iter().try_fold(0i32, |v, &c| {
        c.is_ascii_digit().then(|| v * 10 + i32::from(c - b'0'))
    })
}

/// "20240101T153000Z" or "20240101": the wall minutes from 1970, whether
/// it was a date alone, and whether it was in UTC. A date before 1970 (a
/// birthday) is moved to the same day of 1970: the caller keeps it only
/// when it repeats yearly ([`early`]).
fn time(v: &[u8]) -> Option<(u32, bool, bool)> {
    let mut d = Date {
        year: num(v.get(0..4)?)?,
        month: num(v.get(4..6)?)? as u8,
        day: num(v.get(6..8)?)? as u8,
    };
    if !date::valid(d) {
        return None;
    }
    if d.year < 1970 {
        d.year = 1970;
        // 29 February of a leap year before 1970 has no 1970 day.
        if !date::valid(d) {
            d.day = 28;
        }
    }
    let day = date::days_from(d);
    if v.len() < 13 || v[8] != b'T' {
        return Some((date::start_of(day), true, false));
    }
    let (h, m) = (num(v.get(9..11)?)?, num(v.get(11..13)?)?);
    if h > 23 || m > 59 {
        return None;
    }
    let utc = v.get(15) == Some(&b'Z');
    let t = i64::from(date::start_of(day)) + i64::from(h * 60 + m);
    u32::try_from(t).ok().map(|t| (t, false, utc))
}

/// Whether a DTSTART's date is before 1970.
fn early(v: &[u8]) -> bool {
    v.get(0..4).and_then(num).is_some_and(|y| y < 1970)
}

/// "PT1H30M", "P1D", "-PT15M" as minutes (the sign apart).
fn duration(v: &[u8]) -> Option<(bool, u32)> {
    let (neg, v) = match v.first() {
        Some(b'-') => (true, &v[1..]),
        Some(b'+') => (false, &v[1..]),
        _ => (false, v),
    };
    if v.first() != Some(&b'P') {
        return None;
    }
    let (mut total, mut n) = (0u32, 0u32);
    for &c in &v[1..] {
        match c {
            b'0'..=b'9' => n = n.checked_mul(10)?.checked_add(u32::from(c - b'0'))?,
            b'W' => (total, n) = (total.checked_add(n.checked_mul(7 * 1440)?)?, 0),
            b'D' => (total, n) = (total.checked_add(n.checked_mul(1440)?)?, 0),
            b'H' => (total, n) = (total.checked_add(n.checked_mul(60)?)?, 0),
            b'M' => (total, n) = (total.checked_add(n)?, 0),
            b'S' | b'T' => n = 0,
            _ => return None,
        }
    }
    Some((neg, total))
}

/// An RRULE's value onto the store's rule; `None` for one it doesn't keep
/// (the event is then kept once).
fn rrule(v: &[u8], start_day: i32) -> Option<Rule> {
    let mut every = None;
    let mut n = 1u8;
    let mut days = 0u8;
    let mut until = None;
    let mut count: Option<u32> = None;
    let mut by_month_weekday = false;
    for part in v.split(|&c| c == b';') {
        let (k, val) = match part.iter().position(|&c| c == b'=') {
            Some(i) => (&part[..i], &part[i + 1..]),
            None => continue,
        };
        match k {
            b"FREQ" => {
                every = Some(match val {
                    b"DAILY" => Every::Day,
                    b"WEEKLY" => Every::Week,
                    b"MONTHLY" => Every::MonthDate,
                    b"YEARLY" => Every::Year,
                    _ => return None,
                });
            }
            b"INTERVAL" => {
                n = val.iter().try_fold(0u8, |v, &c| {
                    c.is_ascii_digit()
                        .then(|| v.saturating_mul(10).saturating_add(c - b'0'))
                })?;
            }
            b"BYDAY" => {
                for d in val.split(|&c| c == b',') {
                    // "2TU" (the 2nd Tuesday) makes a monthly rule by weekday.
                    let name = &d[d.len().saturating_sub(2)..];
                    if d.len() > 2 {
                        by_month_weekday = true;
                    }
                    let k = [&b"MO"[..], b"TU", b"WE", b"TH", b"FR", b"SA", b"SU"]
                        .iter()
                        .position(|w| *w == name)?;
                    days |= 1 << k;
                }
            }
            b"UNTIL" => until = time(val).map(|(t, _, _)| date::day_of(t)),
            b"COUNT" => {
                count = val.iter().try_fold(0u32, |v, &c| {
                    c.is_ascii_digit()
                        .then(|| v.saturating_mul(10).saturating_add(u32::from(c - b'0')))
                });
            }
            // Rules the store can't say keep the event single.
            b"BYSETPOS" | b"BYWEEKNO" | b"BYYEARDAY" | b"BYHOUR" | b"BYMINUTE" => return None,
            _ => {}
        }
    }
    let mut every = every?;
    if every == Every::MonthDate && by_month_weekday {
        every = Every::MonthWeekday;
        days = 0;
    }
    let mut rule = Rule {
        every,
        n: n.max(1),
        days: if every == Every::Week { days } else { 0 },
        until,
        after: false,
    };
    // COUNT becomes the day of its last occurrence (walked, at most 3
    // years ahead).
    if let Some(c) = count {
        let mut seen = 0;
        let mut d = start_day;
        while d < start_day + 3 * 366 {
            if rule.occurs(start_day, d) {
                seen += 1;
                if seen == c {
                    rule.until = Some(d);
                    break;
                }
            }
            d += 1;
        }
    }
    Some(rule)
}

/// RFC 5545's text escapes undone (`\,` `\;` `\n` `\\`), a line break as
/// `nl`.
fn unescape<const N: usize>(v: &[u8], nl: u8, out: &mut Short<N>) {
    let mut esc = false;
    for &c in v {
        if esc {
            out.push(&[if c == b'n' || c == b'N' { nl } else { c }]);
            esc = false;
        } else if c == b'\\' {
            esc = true;
        } else {
            out.push(&[c]);
        }
    }
}

// ---- Zones in the file -------------------------------------------------

/// The VTIMEZONEs a file defines, read before its events.
const FILE_ZONES: usize = 6;

#[derive(Clone, Copy)]
struct FileZone {
    tzid: Short<64>,
    /// The tzdata zone its rule is, or (none matched) its rule.
    name: Short<40>,
    rule: zone::Rule,
}

/// One STANDARD or DAYLIGHT part: when it starts each year, its offset,
/// the year its DTSTART names (the latest is the rule in force).
#[derive(Clone, Copy, Default)]
struct Part {
    edge: Option<Edge>,
    to: Option<i16>,
    year: i32,
    ended: bool,
}

/// "+0530", "-0800": minutes east.
fn utc_offset(v: &[u8]) -> Option<i16> {
    let (sign, d) = match v.first()? {
        b'+' => (1, &v[1..]),
        b'-' => (-1, &v[1..]),
        _ => return None,
    };
    let h = num(d.get(0..2)?)?;
    let m = num(d.get(2..4)?)?;
    (h < 24 && m < 60).then(|| (sign * (h * 60 + m)) as i16)
}

/// A part's RRULE ("FREQ=YEARLY;BYMONTH=3;BYDAY=2SU", or Apple's older
/// "BYMONTHDAY=8,9,...,14;BYDAY=SU") as an edge, at `at` minutes.
fn yearly_edge(v: &[u8], at: i16) -> Option<Edge> {
    let mut month = None;
    let mut week = None;
    let mut weekday = None;
    for part in v.split(|&c| c == b';') {
        let Some(i) = part.iter().position(|&c| c == b'=') else {
            continue;
        };
        let (k, val) = (&part[..i], &part[i + 1..]);
        match k {
            b"BYMONTH" => month = num(val).filter(|m| (1..=12).contains(m)),
            b"BYDAY" => {
                let name = &val[val.len().saturating_sub(2)..];
                weekday = [&b"SU"[..], b"MO", b"TU", b"WE", b"TH", b"FR", b"SA"]
                    .iter()
                    .position(|w| *w == name);
                let nth = &val[..val.len().saturating_sub(2)];
                if nth == b"-1" {
                    week = Some(5);
                } else if !nth.is_empty() {
                    week =
                        num(nth.strip_prefix(b"+").unwrap_or(nth)).filter(|w| (1..=5).contains(w));
                }
            }
            b"BYMONTHDAY" if week.is_none() => {
                let first = num(val.split(|&c| c == b',').next().unwrap_or(&[]));
                week = first.map(|d| {
                    if (0..22).contains(&d) {
                        (d - 1) / 7 + 1
                    } else {
                        5
                    }
                });
            }
            b"FREQ" if val != b"YEARLY" => return None,
            _ => {}
        }
    }
    Some(Edge {
        month: month? as u8,
        week: week.unwrap_or(1) as u8,
        weekday: weekday? as u8,
        at,
    })
}

/// The VTIMEZONEs of `text`, each matched to a tzdata zone where one has
/// its rule.
#[inline(never)]
fn file_zones(text: &[u8], out: &mut [Option<FileZone>; FILE_ZONES]) {
    let mut lines = Lines { text, at: 0 };
    let mut buf = [0u8; 256];
    let mut tzid: Short<64> = Short::new();
    let (mut std, mut dst) = (Part::default(), Part::default());
    let mut cur: Option<(bool, Part)> = None;
    let mut inside = false;
    let mut k = 0;
    while let Some(line) = lines.next_into(&mut buf) {
        let Some(colon) = line.iter().position(|&c| c == b':') else {
            continue;
        };
        let (name, value) = (&line[..colon], &line[colon + 1..]);
        let key = &name[..name.iter().position(|&c| c == b';').unwrap_or(name.len())];
        match (key, value) {
            (b"BEGIN", b"VTIMEZONE") => {
                inside = true;
                tzid = Short::new();
                (std, dst) = (Part::default(), Part::default());
            }
            (b"BEGIN", b"STANDARD") if inside => cur = Some((false, Part::default())),
            (b"BEGIN", b"DAYLIGHT") if inside => cur = Some((true, Part::default())),
            (b"END", b"STANDARD" | b"DAYLIGHT") => {
                if let Some((summer, p)) = cur.take() {
                    let slot = if summer { &mut dst } else { &mut std };
                    // The latest part in force wins (an older rule ended).
                    if slot.to.is_none() || (!p.ended && (slot.ended || p.year >= slot.year)) {
                        *slot = p;
                    }
                }
            }
            (b"TZID", _) if inside => tzid.push(value),
            (b"DTSTART", _) => {
                if let Some((_, p)) = cur.as_mut() {
                    p.year = value.get(0..4).and_then(num).unwrap_or(0);
                    let at = match (
                        value.get(9..11).and_then(num),
                        value.get(11..13).and_then(num),
                    ) {
                        (Some(h), Some(m)) => (h * 60 + m) as i16,
                        _ => 0,
                    };
                    // The edge keeps its time until the RRULE gives the day.
                    p.edge = Some(Edge {
                        month: 0,
                        week: 0,
                        weekday: 0,
                        at,
                    });
                }
            }
            (b"RRULE", _) => {
                if let Some((_, p)) = cur.as_mut() {
                    let at = p.edge.map_or(120, |e| e.at);
                    p.edge = yearly_edge(value, at);
                    p.ended = value.windows(6).any(|w| w == b"UNTIL=");
                }
            }
            (b"TZOFFSETTO", _) => {
                if let Some((_, p)) = cur.as_mut() {
                    p.to = utc_offset(value);
                }
            }
            (b"END", b"VTIMEZONE") if inside => {
                inside = false;
                let Some(s) = std.to.or(dst.to) else { continue };
                let rule = match (dst.to, dst.edge, std.edge) {
                    (Some(d), Some(on), Some(off))
                        if on.month != 0 && off.month != 0 && !dst.ended && !std.ended =>
                    {
                        zone::Rule {
                            std: s,
                            dst: d,
                            on,
                            off,
                        }
                    }
                    _ => zone::Rule::fixed(s),
                };
                let mut name = Short::new();
                if let Some(z) = zone::of_rule(&rule) {
                    let mut b = [0u8; 40];
                    let n = z.name(&mut b);
                    name.push(&b[..n]);
                }
                if let Some(slot) = out.get_mut(k) {
                    *slot = Some(FileZone { tzid, name, rule });
                    k += 1;
                }
            }
            _ => {}
        }
    }
}

/// A TZID's zone: a tzdata name ("America/New_York", or ending one, as
/// old Thunderbird files wrote "/mozilla.org/20050126_1/America/New_York"),
/// or the file's own VTIMEZONE of that TZID. Its tzdata name, or its rule
/// when no tzdata zone has it.
fn zone_of(tzid: &[u8], files: &[Option<FileZone>; FILE_ZONES]) -> Option<(Short<40>, zone::Rule)> {
    let tzid = tzid
        .strip_prefix(b"\"")
        .and_then(|t| t.strip_suffix(b"\""))
        .unwrap_or(tzid);
    let named = |n: &[u8]| {
        let z = zone::find(n)?;
        let mut s = Short::new();
        let mut b = [0u8; 40];
        let k = z.name(&mut b);
        s.push(&b[..k]);
        Some((s, z.rule()))
    };
    // A tzdata name is its own zone (a VTIMEZONE beside it says the
    // same); a name of the file's own is its VTIMEZONE's rule.
    if let Some(z) = named(tzid) {
        return Some(z);
    }
    if let Some(f) = files.iter().flatten().find(|f| f.tzid.get() == tzid) {
        return Some((f.name, f.rule));
    }
    let mut t = tzid;
    loop {
        if let Some(z) = named(t) {
            return Some(z);
        }
        let slash = t.iter().position(|&c| c == b'/')?;
        t = &t[slash + 1..];
    }
}

/// The value of a parameter (`TZID=...`) in a property's name part.
fn param<'a>(name: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    name.split(|&c| c == b';').skip(1).find_map(|p| {
        let eq = p.iter().position(|&c| c == b'=')?;
        p[..eq].eq_ignore_ascii_case(key).then(|| &p[eq + 1..])
    })
}

/// A zone's tzdata name (empty: none has its rule) and its rule.
type Zoned = (Short<40>, zone::Rule);

/// A date-time property's wall time in the event's zone: its own wall
/// time when it names the event's zone; else (a zone no tzdata name has, a
/// UTC end of a zoned start) moved into it.
fn wall_in(
    name: &[u8],
    value: &[u8],
    zone_rule: Option<zone::Rule>,
    files: &[Option<FileZone>; FILE_ZONES],
) -> Option<(u32, bool, Option<Zoned>)> {
    let (t, date_only, utc) = time(value)?;
    let own = if utc {
        let mut s = Short::new();
        s.push(b"UTC");
        Some((s, zone::UTC))
    } else {
        param(name, b"TZID").and_then(|z| zone_of(z, files))
    };
    // An end in another zone than the start: moved to the start's.
    if let (Some(to), Some((_, from))) = (zone_rule, own)
        && to != from
    {
        let moved = from.to(i64::from(t), &to);
        return Some((u32::try_from(moved).ok()?, date_only, own));
    }
    Some((t, date_only, own))
}

/// The events of an iCalendar file, into `out`: how many (see
/// [`ics_each`]).
pub fn ics(text: &[u8], out: &mut [Event]) -> usize {
    let mut n = 0;
    ics_each(text, |e| {
        if n < out.len() {
            out[n] = *e;
            n += 1;
            true
        } else {
            false
        }
    });
    n
}

/// Each event of an iCalendar file handed to `take` as it is read (no
/// array of them: a program's stack is the shell's, and 160 events were
/// 27 KB), until it answers false. Each keeps its wall time and its zone
/// ([`Event::zone`]): a time in a zone no tzdata name has is moved to UTC.
pub fn ics_each(text: &[u8], mut take: impl FnMut(&Event) -> bool) {
    each_dyn(text, &mut take);
}

/// [`ics_each`] built once (a reader per caller's closure was 14 KB in an
/// app built for speed).
#[inline(never)]
fn each_dyn(text: &[u8], take: &mut dyn FnMut(&Event) -> bool) {
    let mut files = [None; FILE_ZONES];
    file_zones(text, &mut files);
    let mut lines = Lines { text, at: 0 };
    let mut buf = [0u8; 256];
    let mut cur: Option<Event> = None;
    let mut rule_of_zone: Option<zone::Rule> = None;
    let (mut end, mut dur, mut rr) = (None::<u32>, None::<u32>, Short::<128>::new());
    let mut is_early = false;
    let mut alarm = false;
    while let Some(line) = lines.next_into(&mut buf) {
        let Some(colon) = line.iter().position(|&c| c == b':') else {
            continue;
        };
        let (name, value) = (&line[..colon], &line[colon + 1..]);
        let key = &name[..name.iter().position(|&c| c == b';').unwrap_or(name.len())];
        match key {
            b"BEGIN" if value == b"VEVENT" => {
                cur = Some(Event::new());
                rule_of_zone = None;
                (end, dur, rr) = (None, None, Short::new());
                is_early = false;
                alarm = false;
            }
            b"BEGIN" if value == b"VALARM" => alarm = true,
            b"END" if value == b"VALARM" => alarm = false,
            b"END" if value == b"VEVENT" => {
                if let Some(mut e) = cur.take()
                    && (e.start != 0 || is_early)
                {
                    e.minutes = match (end, dur) {
                        (Some(t), _) if t > e.start => t - e.start,
                        (_, Some(m)) if m > 0 => m,
                        _ if e.all_day => 1440,
                        _ => 60,
                    };
                    if !rr.is_empty() {
                        e.rule = rrule(rr.get(), date::day_of(e.start));
                    }
                    // Before 1970 only a yearly repeat (a birthday) is kept,
                    // from its 1970 day.
                    if (!is_early || e.rule.is_some_and(|r| r.every == Every::Year)) && !take(&e) {
                        return;
                    }
                }
            }
            _ if alarm => {
                // The first alarm before the start (an absolute or after-
                // the-start one is left: the store reminds before).
                if key == b"TRIGGER"
                    && let Some(e) = cur.as_mut()
                    && e.alarm.is_none()
                    && param(name, b"VALUE").is_none_or(|v| v == b"DURATION")
                    && param(name, b"RELATED").is_none_or(|v| v == b"START")
                    && let Some((neg, m)) = duration(value)
                    && (neg || m == 0)
                {
                    e.alarm = Some(m);
                }
            }
            b"SUMMARY" => {
                if let Some(e) = cur.as_mut() {
                    unescape(value, b' ', &mut e.title);
                }
            }
            b"LOCATION" => {
                if let Some(e) = cur.as_mut() {
                    unescape(value, b' ', &mut e.place);
                }
            }
            b"DESCRIPTION" => {
                if let Some(e) = cur.as_mut() {
                    unescape(value, b'\n', &mut e.note);
                }
            }
            b"DTSTART" => {
                if let Some(e) = cur.as_mut()
                    && let Some((t, all, z)) = wall_in(name, value, None, &files)
                {
                    e.start = t;
                    e.all_day = all;
                    is_early = early(value);
                    e.zone = Short::new();
                    rule_of_zone = None;
                    if !all && let Some((zn, zr)) = z {
                        if zn.is_empty() {
                            // No tzdata zone has its rule: kept in UTC.
                            if let Ok(u) = u32::try_from(zr.utc_of(i64::from(t))) {
                                e.start = u;
                            }
                            e.zone.push(b"UTC");
                            rule_of_zone = Some(zone::UTC);
                        } else {
                            e.zone = zn;
                            rule_of_zone = Some(zr);
                        }
                    }
                }
            }
            b"DTEND" => end = wall_in(name, value, rule_of_zone, &files).map(|(t, _, _)| t),
            b"DURATION" => dur = duration(value).map(|(_, m)| m),
            b"RRULE" => {
                rr = Short::new();
                rr.push(value);
            }
            _ => {}
        }
    }
}

// ---- Writing ------------------------------------------------------------

/// An event to write: the store's appointment as the calendar keeps it.
#[derive(Clone, Copy, Debug)]
pub struct Out<'a> {
    /// The record's id (its UID: the same on every export, so a calendar
    /// that reads the file twice updates the event, not doubles it).
    pub id: u32,
    /// Title.
    pub title: &'a [u8],
    /// Place.
    pub place: &'a [u8],
    /// Notes.
    pub note: &'a [u8],
    /// The wall time in `zone`, minutes from 1970.
    pub start: u32,
    /// Minutes long.
    pub minutes: u32,
    /// All day (`minutes` then says how many days, in minutes).
    pub all_day: bool,
    /// How it repeats.
    pub rule: Option<Rule>,
    /// Its tzdata zone name (empty: a floating time).
    pub zone: &'a [u8],
    /// Remind this many minutes before.
    pub alarm: Option<u32>,
}

type Folder<'f> = crate::vcard::Folder<'f>;

fn digits(f: &mut Folder<'_>, v: u32, width: usize) {
    let mut d = [b'0'; 10];
    let mut k = d.len();
    let mut v = v;
    loop {
        k -= 1;
        d[k] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    let k = k.min(d.len() - width);
    f.put(&d[k..]);
}

/// "20261123" or "20261123T090000".
fn put_time(f: &mut Folder<'_>, t: i64, with_time: bool) {
    let day = t.div_euclid(1440) as i32;
    let d = date::date_of(day);
    digits(f, d.year.clamp(0, 9999) as u32, 4);
    digits(f, u32::from(d.month), 2);
    digits(f, u32::from(d.day), 2);
    if with_time {
        let m = t.rem_euclid(1440) as u32;
        f.put(b"T");
        digits(f, m / 60, 2);
        digits(f, m % 60, 2);
        f.put(b"00");
    }
}

fn put_offset(f: &mut Folder<'_>, m: i16) {
    f.put(if m < 0 { b"-" } else { b"+" });
    let a = u32::from(m.unsigned_abs());
    digits(f, a / 60, 2);
    digits(f, a % 60, 2);
}

/// The calendar's head (`VCALENDAR`) and a VTIMEZONE for each of `zones`
/// (tzdata names; one no table has is left out, its events written in
/// UTC by [`event`]).
pub fn head(zones: &[&[u8]], mut out: impl FnMut(&[u8])) {
    head_dyn(zones, &mut out);
}

#[inline(never)]
fn head_dyn(zones: &[&[u8]], out: &mut dyn FnMut(&[u8])) {
    let mut f = Folder { out, col: 0 };
    for l in [
        &b"BEGIN:VCALENDAR"[..],
        b"VERSION:2.0",
        b"PRODID:-//RetroPane//Calendar 3//EN",
        b"CALSCALE:GREGORIAN",
    ] {
        f.put(l);
        f.end();
    }
    for name in zones {
        let Some(z) = zone::find(name) else { continue };
        if *name == b"UTC" || *name == b"Etc/UTC" {
            continue;
        }
        let r = z.rule();
        f.put(b"BEGIN:VTIMEZONE");
        f.end();
        f.put(b"TZID:");
        f.put(name);
        f.end();
        let parts: [(&[u8], i16, i16, Option<Edge>); 2] = if r.has_summer() {
            [
                (b"STANDARD", r.dst, r.std, Some(r.off)),
                (b"DAYLIGHT", r.std, r.dst, Some(r.on)),
            ]
        } else {
            [(b"STANDARD", r.std, r.std, None), (b"", 0, 0, None)]
        };
        for (kind, from, to, edge) in parts {
            if kind.is_empty() {
                continue;
            }
            f.put(b"BEGIN:");
            f.put(kind);
            f.end();
            f.put(b"DTSTART:");
            let start = edge.map_or(0, |e| {
                i64::from(e.day(1970)) * 1440 + i64::from(e.at.clamp(0, 1439))
            });
            put_time(&mut f, start, true);
            f.end();
            f.put(b"TZOFFSETFROM:");
            put_offset(&mut f, from);
            f.end();
            f.put(b"TZOFFSETTO:");
            put_offset(&mut f, to);
            f.end();
            if let Some(e) = edge {
                f.put(b"RRULE:FREQ=YEARLY;BYMONTH=");
                digits(&mut f, u32::from(e.month), 1);
                f.put(b";BYDAY=");
                if e.week >= 5 {
                    f.put(b"-1");
                } else {
                    digits(&mut f, u32::from(e.week), 1);
                }
                f.put(
                    [&b"SU"[..], b"MO", b"TU", b"WE", b"TH", b"FR", b"SA"]
                        [usize::from(e.weekday % 7)],
                );
                f.end();
            }
            f.put(b"END:");
            f.put(kind);
            f.end();
        }
        f.put(b"END:VTIMEZONE");
        f.end();
    }
}

/// One event, `stamp` the export's moment (UTC minutes from 1970).
pub fn event(e: &Out<'_>, stamp: i64, mut out: impl FnMut(&[u8])) {
    event_dyn(e, stamp, &mut out);
}

#[inline(never)]
fn event_dyn(e: &Out<'_>, stamp: i64, out: &mut dyn FnMut(&[u8])) {
    let mut f = Folder { out, col: 0 };
    f.put(b"BEGIN:VEVENT");
    f.end();
    f.put(b"UID:");
    digits(&mut f, e.id, 1);
    f.put(b"@retropane.local");
    f.end();
    f.put(b"DTSTAMP:");
    put_time(&mut f, stamp, true);
    f.put(b"Z");
    f.end();
    // A zone the table doesn't know (the store keeps only its names) is
    // written as a floating time: never a TZID without its VTIMEZONE.
    let known = !e.zone.is_empty() && zone::find(e.zone).is_some();
    let utc = e.zone == b"UTC" || e.zone == b"Etc/UTC";
    let when = |f: &mut Folder<'_>, key: &[u8], t: i64| {
        f.put(key);
        if e.all_day {
            f.put(b";VALUE=DATE:");
            put_time(f, t, false);
        } else if utc {
            f.put(b":");
            put_time(f, t, true);
            f.put(b"Z");
        } else if known {
            f.put(b";TZID=");
            f.put(e.zone);
            f.put(b":");
            put_time(f, t, true);
        } else {
            f.put(b":");
            put_time(f, t, true);
        }
        f.end();
    };
    let start = i64::from(e.start);
    when(&mut f, b"DTSTART", start);
    let len = if e.all_day {
        i64::from(e.minutes.max(1440)).div_euclid(1440) * 1440
    } else {
        i64::from(e.minutes)
    };
    when(&mut f, b"DTEND", start + len);
    for (key, v) in [
        (&b"SUMMARY:"[..], e.title),
        (b"LOCATION:", e.place),
        (b"DESCRIPTION:", e.note),
    ] {
        if !v.is_empty() || key == b"SUMMARY:" {
            f.put(key);
            f.escaped(v);
            f.end();
        }
    }
    if let Some(r) = e.rule {
        let day = date::day_of(e.start);
        f.put(b"RRULE:FREQ=");
        f.put(match r.every {
            Every::Day => b"DAILY",
            Every::Week => b"WEEKLY",
            Every::MonthDate | Every::MonthWeekday => b"MONTHLY",
            Every::Year => b"YEARLY",
        });
        if r.n > 1 {
            f.put(b";INTERVAL=");
            digits(&mut f, u32::from(r.n), 1);
        }
        const DAYS: [&[u8]; 7] = [b"MO", b"TU", b"WE", b"TH", b"FR", b"SA", b"SU"];
        match r.every {
            Every::Week if r.days != 0 => {
                f.put(b";BYDAY=");
                let mut first = true;
                for (k, d) in DAYS.iter().enumerate() {
                    if r.days & (1 << k) != 0 {
                        if !first {
                            f.put(b",");
                        }
                        f.put(d);
                        first = false;
                    }
                }
            }
            Every::MonthWeekday => {
                let d = date::date_of(day);
                f.put(b";BYDAY=");
                if (d.day - 1) / 7 >= 4 {
                    f.put(b"-1");
                } else {
                    digits(&mut f, u32::from((d.day - 1) / 7 + 1), 1);
                }
                f.put(DAYS[usize::from(date::weekday(day))]);
            }
            _ => {}
        }
        if let Some(u) = r.until {
            f.put(b";UNTIL=");
            let end = i64::from(u) * 1440 + 1439;
            if e.all_day {
                put_time(&mut f, end, false);
            } else {
                // RFC 5545: a zoned start's UNTIL is in UTC.
                let rule = if known {
                    zone::find(e.zone).map(|z| z.rule())
                } else {
                    None
                };
                put_time(&mut f, rule.map_or(end, |r| r.utc_of(end)), true);
                f.put(b"Z");
            }
        }
        f.end();
    }
    if let Some(a) = e.alarm {
        for l in [
            &b"BEGIN:VALARM"[..],
            b"ACTION:DISPLAY",
            b"DESCRIPTION:Reminder",
        ] {
            f.put(l);
            f.end();
        }
        f.put(b"TRIGGER:-PT");
        digits(&mut f, a, 1);
        f.put(b"M");
        f.end();
        f.put(b"END:VALARM");
        f.end();
    }
    f.put(b"END:VEVENT");
    f.end();
}

/// The calendar's end.
pub fn tail(mut out: impl FnMut(&[u8])) {
    out(b"END:VCALENDAR\r\n");
}

#[cfg(test)]
#[path = "ics_tests.rs"]
mod tests;
