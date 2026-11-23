//! What the organiser's apps read from the network (contacts-rule0.md,
//! calendar-rule0.md and calculator-rule0.md version 2): address
//! suggestions (Photon's GeoJSON), calendars (iCalendar, RFC 5545) and
//! money rates (the European Central Bank's daily XML).
//!
//! Every answer is hostile input (security.md): read with bounds on every
//! length, never trusting a count or an offset in it, and a malformed
//! answer yields fewer results, never a panic. The same readers serve the
//! dock's relay now and the device's own TLS later: only the transport
//! changes (quality audit C95).
//!
//! Iterate, don't imitate (rule 0): a full JSON or XML library is tens of
//! KB and allocates; these readers scan for exactly the fields used, in
//! place, with no allocation, and the iCalendar reader keeps only what the
//! store holds (title, place, start, length, all-day, the repeat rule),
//! mapping the RRULEs people make onto [`crate::repeat::Rule`] and keeping
//! any other as a single event.

use crate::date::{self, Date};
use crate::repeat::{Every, Rule};

/// A field's text, cut to `N` bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Short<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Default for Short<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Short<N> {
    const fn new() -> Self {
        Self { b: [0; N], n: 0 }
    }
    fn push(&mut self, s: &[u8]) {
        for &c in s {
            if self.n < N {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
    }
    /// The text.
    #[must_use]
    pub fn get(&self) -> &[u8] {
        &self.b[..self.n]
    }
}

// ---- Photon (address suggestions) ------------------------------------

/// One suggested address: the street line and the city line, and where it
/// is when the service said (the contact's map: contacts-rule0.md "Where
/// they are", geocoded from the very answer the suggestions came in, no
/// second question).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Place {
    /// "410 Pine St".
    pub street: Short<48>,
    /// "Seattle, WA 98101".
    pub city: Short<48>,
    /// Latitude and longitude, millionths of a degree (`None`: the answer
    /// gave none, or one out of range).
    pub at: Option<(i32, i32)>,
}

impl Place {
    /// No address.
    pub const EMPTY: Self = Self {
        street: Short::new(),
        city: Short::new(),
        at: None,
    };
}

/// A JSON number (`-122.3365`, `47.6`, `1e2` refused) as millionths, read
/// from the start of `s` after any spaces: `None` for anything else, more
/// than three whole digits or past `limit` degrees. Fractions past six
/// places are cut (a millionth of a degree is ~11 cm).
fn micro_degrees(s: &[u8], limit: i32) -> Option<i32> {
    let mut i = s.iter().take_while(|&&c| c == b' ').count();
    let neg = s.get(i) == Some(&b'-');
    i += usize::from(neg);
    let (mut whole, mut digits) = (0i32, 0);
    while let Some(c) = s.get(i).filter(|c| c.is_ascii_digit()) {
        whole = whole * 10 + i32::from(c - b'0');
        digits += 1;
        i += 1;
        if digits > 3 {
            return None;
        }
    }
    if digits == 0 {
        return None;
    }
    let mut frac = 0i32;
    let mut places = 0;
    if s.get(i) == Some(&b'.') {
        i += 1;
        while let Some(c) = s.get(i).filter(|c| c.is_ascii_digit()) {
            if places < 6 {
                frac = frac * 10 + i32::from(c - b'0');
                places += 1;
            }
            i += 1;
        }
    }
    if matches!(s.get(i), Some(b'e' | b'E')) {
        return None;
    }
    while places < 6 {
        frac *= 10;
        places += 1;
    }
    let v = whole * 1_000_000 + frac;
    (v <= limit * 1_000_000).then_some(if neg { -v } else { v })
}

/// GeoJSON's `"coordinates":[lon,lat]` in `obj`, as (latitude, longitude)
/// in millionths; `None` when absent or not a place on Earth.
fn coordinates(obj: &[u8]) -> Option<(i32, i32)> {
    let k = find(obj, b"\"coordinates\"")?;
    let rest = &obj[k + 13..];
    let open = rest.iter().take(4).position(|&c| c == b'[')?;
    let rest = &rest[open + 1..];
    let lon = micro_degrees(rest, 180)?;
    let comma = rest.iter().take(24).position(|&c| c == b',')?;
    let lat = micro_degrees(&rest[comma + 1..], 90)?;
    Some((lat, lon))
}

/// The value of `"key":"..."` in `obj` (a JSON object's text), escapes
/// left as they are but `\"` honoured; `None` when absent.
fn json_str<'a>(obj: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let mut at = 0;
    while at + key.len() + 3 < obj.len() {
        if obj[at] == b'"'
            && obj[at + 1..].starts_with(key)
            && obj.get(at + 1 + key.len()) == Some(&b'"')
        {
            let mut k = at + 2 + key.len();
            while k < obj.len() && (obj[k] == b' ' || obj[k] == b':') {
                k += 1;
            }
            if obj.get(k) != Some(&b'"') {
                return None;
            }
            let start = k + 1;
            let mut e = start;
            while e < obj.len() && obj[e] != b'"' {
                if obj[e] == b'\\' {
                    e += 1;
                }
                e += 1;
            }
            return obj.get(start..e.min(obj.len()));
        }
        at += 1;
    }
    None
}

/// Photon's answer (GeoJSON: `features[].properties`) as up to `out.len()`
/// places: how many. A US state is written by its name as Photon gives it.
pub fn photon(answer: &[u8], out: &mut [Place]) -> usize {
    let mut n = 0;
    let mut at = 0;
    while n < out.len() {
        // Each feature whole (JSON fixes no key order: a feature's geometry
        // may come before its properties or after), else, with no features
        // list, the answer as one.
        let Some(feature) = next_feature(answer, &mut at) else {
            break;
        };
        let Some(k) = find(feature, b"\"properties\"") else {
            continue;
        };
        // (Its braces matched outside strings: a name may hold one.)
        let obj = object(&feature[k..]).unwrap_or(&feature[k..]);
        let mut p = Place::EMPTY;
        p.at = coordinates(feature);
        let house = json_str(obj, b"housenumber");
        let street = json_str(obj, b"street").or_else(|| json_str(obj, b"name"));
        if let Some(h) = house {
            p.street.push(h);
            p.street.push(b" ");
        }
        if let Some(s) = street {
            p.street.push(s);
        }
        if let Some(c) = json_str(obj, b"city").or_else(|| json_str(obj, b"name")) {
            p.city.push(c);
        }
        if let Some(s) = json_str(obj, b"state") {
            p.city.push(b", ");
            p.city.push(s);
        }
        if let Some(z) = json_str(obj, b"postcode") {
            p.city.push(b" ");
            p.city.push(z);
        }
        if p.street.n > 0 {
            out[n] = p;
            n += 1;
        }
    }
    n
}

/// The next object of the answer's `"features"` list from `*at` (its
/// braces matched outside strings, escapes honoured; one cut short runs to
/// the answer's end), `*at` moved past it. With no features list, the
/// whole answer once.
fn next_feature<'a>(answer: &'a [u8], at: &mut usize) -> Option<&'a [u8]> {
    if *at == 0 {
        match find(answer, b"\"features\"") {
            Some(k) => *at = k + answer[k..].iter().position(|&c| c == b'[')? + 1,
            None => {
                // (Past the end: an empty answer is taken once too.)
                *at = answer.len() + 1;
                return Some(answer);
            }
        }
    }
    let start = *at
        + answer
            .get(*at..)?
            .iter()
            .position(|&c| c == b'{' || c == b']')?;
    if answer[start] == b']' {
        *at = answer.len();
        return None;
    }
    let obj = object(&answer[start..]).unwrap_or(&answer[start..]);
    *at = start + obj.len();
    Some(obj)
}

/// The JSON object starting at the first `{` of `s`, its braces matched
/// outside strings (escapes honoured); cut short, it runs to `s`'s end.
/// `None` with no `{`.
fn object(s: &[u8]) -> Option<&[u8]> {
    let start = s.iter().position(|&c| c == b'{')?;
    let (mut depth, mut in_str, mut esc) = (0u32, false, false);
    for (k, &c) in s.iter().enumerate().skip(start) {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else if c == b'"' {
            in_str = true;
        } else if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(&s[start..=k]);
            }
        }
    }
    Some(&s[start..])
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

// ---- ECB rates --------------------------------------------------------

/// One rate: the currency's code and how many of it one euro buys, in
/// millionths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rate {
    /// "USD".
    pub code: [u8; 3],
    /// Units per euro, x 1,000,000.
    pub micro: u64,
}

/// The ECB's daily file (`<Cube currency='USD' rate='1.0812'/>`) as rates,
/// and its date (`<Cube time='2024-01-01'>`): how many, and the day.
pub fn ecb(answer: &[u8], out: &mut [Rate]) -> (usize, Option<i32>) {
    let mut n = 0;
    let day = find(answer, b"time='").and_then(|k| iso_day(answer.get(k + 6..k + 16)?));
    let mut at = 0;
    while n < out.len() {
        let Some(k) = find(&answer[at..], b"currency='") else {
            break;
        };
        let c = at + k + 10;
        at = c;
        let Some(code) = answer.get(c..c + 3) else {
            break;
        };
        let Some(r) = find(&answer[c..], b"rate='") else {
            break;
        };
        let rs = c + r + 6;
        let re = answer[rs..]
            .iter()
            .position(|&b| b == b'\'')
            .map_or(rs, |e| rs + e);
        let Some(micro) = decimal_micro(&answer[rs..re]) else {
            continue;
        };
        if code.iter().all(u8::is_ascii_uppercase) {
            out[n] = Rate {
                code: [code[0], code[1], code[2]],
                micro,
            };
            n += 1;
        }
    }
    (n, day)
}

/// "1.0812" as millionths.
fn decimal_micro(s: &[u8]) -> Option<u64> {
    let (mut v, mut frac, mut seen) = (0u64, 0u32, false);
    for &c in s {
        match c {
            b'0'..=b'9' if !seen || frac < 6 => {
                v = v.checked_mul(10)?.checked_add(u64::from(c - b'0'))?;
                if seen {
                    frac += 1;
                }
            }
            b'0'..=b'9' => {}
            b'.' if !seen => seen = true,
            _ => return None,
        }
    }
    while frac < 6 {
        v = v.checked_mul(10)?;
        frac += 1;
    }
    Some(v)
}

/// "2024-01-01" as a day count.
fn iso_day(s: &[u8]) -> Option<i32> {
    let num = |a: &[u8]| {
        a.iter().try_fold(0i32, |v, &c| {
            c.is_ascii_digit().then(|| v * 10 + i32::from(c - b'0'))
        })
    };
    let d = Date {
        year: num(s.get(0..4)?)?,
        month: num(s.get(5..7)?)? as u8,
        day: num(s.get(8..10)?)? as u8,
    };
    date::valid(d).then(|| date::days_from(d))
}

// ---- iCalendar ----------------------------------------------------------

/// One event read from an iCalendar file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    /// SUMMARY.
    pub title: Short<64>,
    /// LOCATION.
    pub place: Short<64>,
    /// The start, local minutes from 1970 (a UTC time moved by
    /// `utc_offset` minutes).
    pub start: u32,
    /// Minutes long (0 for all-day: one day).
    pub minutes: u32,
    /// All day.
    pub all_day: bool,
    /// How it repeats, when its RRULE is one the store keeps.
    pub rule: Option<Rule>,
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

/// "20240101T153000Z" or "20240101": the local minutes from 1970 (a UTC
/// time moved by `utc_offset`), and whether it was a date alone. A date
/// before 1970 (a birthday) is moved to the same day of 1970: the caller
/// keeps it only when it repeats yearly ([`ics_early`]).
fn ics_time(v: &[u8], utc_offset: i32) -> Option<(u32, bool)> {
    let num = |a: &[u8]| {
        a.iter().try_fold(0i32, |v, &c| {
            c.is_ascii_digit().then(|| v * 10 + i32::from(c - b'0'))
        })
    };
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
    if v.len() < 15 || v[8] != b'T' {
        return Some((date::start_of(day), true));
    }
    let m = num(v.get(9..11)?)? * 60 + num(v.get(11..13)?)?;
    let utc = v.get(15) == Some(&b'Z');
    let t = date::start_of(day) as i64 + i64::from(m) + if utc { i64::from(utc_offset) } else { 0 };
    u32::try_from(t).ok().map(|t| (t, false))
}

/// Whether a DTSTART's date is before 1970.
fn ics_early(v: &[u8]) -> bool {
    v.get(0..4)
        .and_then(|y| {
            y.iter().try_fold(0i32, |a, &c| {
                c.is_ascii_digit().then(|| a * 10 + i32::from(c - b'0'))
            })
        })
        .is_some_and(|y| y < 1970)
}

/// "PT1H30M" or "P1D" as minutes.
fn ics_duration(v: &[u8]) -> Option<u32> {
    let (mut total, mut n) = (0u32, 0u32);
    for &c in v.get(1..)? {
        match c {
            b'0'..=b'9' => n = n.checked_mul(10)?.checked_add(u32::from(c - b'0'))?,
            b'W' => (total, n) = (total + n * 7 * 1440, 0),
            b'D' => (total, n) = (total + n * 1440, 0),
            b'H' => (total, n) = (total + n * 60, 0),
            b'M' => (total, n) = (total + n, 0),
            b'S' | b'T' => n = 0,
            _ => return None,
        }
    }
    Some(total)
}

/// An RRULE's value onto the store's rule; `None` for one it doesn't keep
/// (the event is then kept once).
fn ics_rule(v: &[u8], start_day: i32, utc_offset: i32) -> Option<Rule> {
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
            b"UNTIL" => until = ics_time(val, utc_offset).map(|(t, _)| date::day_of(t)),
            b"COUNT" => {
                count = val.iter().try_fold(0u32, |v, &c| {
                    c.is_ascii_digit().then(|| v * 10 + u32::from(c - b'0'))
                })
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

/// The events of an iCalendar file, into `out`: how many (see
/// [`ics_each`]).
pub fn ics(text: &[u8], utc_offset: i32, out: &mut [Event]) -> usize {
    let mut n = 0;
    ics_each(text, utc_offset, |e| {
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
/// 27 KB), until it answers false. Times in UTC are moved by `utc_offset`
/// minutes to the device's zone; a TZID'd time is taken as local (the
/// zone database is a later item).
pub fn ics_each(text: &[u8], utc_offset: i32, mut take: impl FnMut(&Event) -> bool) {
    let mut lines = Lines { text, at: 0 };
    let mut buf = [0u8; 256];
    let mut cur: Option<Event> = None;
    let (mut end, mut dur, mut rrule) = (None::<u32>, None::<u32>, Short::<128>::new());
    let mut early = false;
    while let Some(line) = lines.next_into(&mut buf) {
        let Some(colon) = line.iter().position(|&c| c == b':') else {
            continue;
        };
        let (name, value) = (&line[..colon], &line[colon + 1..]);
        let key = &name[..name.iter().position(|&c| c == b';').unwrap_or(name.len())];
        match key {
            b"BEGIN" if value == b"VEVENT" => {
                cur = Some(Event {
                    title: Short::new(),
                    place: Short::new(),
                    start: 0,
                    minutes: 60,
                    all_day: false,
                    rule: None,
                });
                (end, dur, rrule) = (None, None, Short::new());
                early = false;
            }
            b"END" if value == b"VEVENT" => {
                if let Some(mut e) = cur.take()
                    && (e.start != 0 || early)
                {
                    e.minutes = match (end, dur) {
                        (Some(t), _) if t > e.start => t - e.start,
                        (_, Some(m)) => m,
                        _ if e.all_day => 1440,
                        _ => 60,
                    };
                    if !rrule.get().is_empty() {
                        e.rule = ics_rule(rrule.get(), date::day_of(e.start), utc_offset);
                    }
                    // Before 1970 only a yearly repeat (a birthday) is kept,
                    // from its 1970 day.
                    if (!early || e.rule.is_some_and(|r| r.every == Every::Year)) && !take(&e) {
                        return;
                    }
                }
            }
            b"SUMMARY" => {
                if let Some(e) = cur.as_mut() {
                    e.title.push(unescape(value).get());
                }
            }
            b"LOCATION" => {
                if let Some(e) = cur.as_mut() {
                    e.place.push(unescape(value).get());
                }
            }
            b"DTSTART" => {
                if let (Some(e), Some((t, all))) = (cur.as_mut(), ics_time(value, utc_offset)) {
                    e.start = t;
                    e.all_day = all;
                    early = ics_early(value);
                }
            }
            b"DTEND" => end = ics_time(value, utc_offset).map(|(t, _)| t),
            b"DURATION" => dur = ics_duration(value),
            b"RRULE" => {
                rrule = Short::new();
                rrule.push(value);
            }
            _ => {}
        }
    }
}

/// RFC 5545's text escapes (`\,` `\;` `\n` `\\`) undone, cut to 64 bytes.
fn unescape(v: &[u8]) -> Short<64> {
    let mut out = Short::new();
    let mut esc = false;
    for &c in v {
        if esc {
            out.push(&[if c == b'n' || c == b'N' { b' ' } else { c }]);
            esc = false;
        } else if c == b'\\' {
            esc = true;
        } else {
            out.push(&[c]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHOTON: &[u8] = br#"{"features":[{"geometry":{"coordinates":[-122.33,47.61],"type":"Point"},"type":"Feature","properties":{"osm_id":1,"country":"United States","city":"Seattle","countrycode":"US","postcode":"98101","housenumber":"410","type":"house","street":"Pine Street","state":"Washington"}},{"type":"Feature","properties":{"name":"Pike Place Market","city":"Seattle","state":"Washington","postcode":"98101"}}],"type":"FeatureCollection"}"#;

    #[test]
    fn photon_answers_become_street_and_city_lines() {
        let mut out = [Place::EMPTY; 5];
        let n = photon(PHOTON, &mut out);
        assert_eq!(n, 2);
        assert_eq!(out[0].street.get(), b"410 Pine Street");
        assert_eq!(out[0].city.get(), b"Seattle, Washington 98101");
        assert_eq!(out[1].street.get(), b"Pike Place Market");
        // Where each is: the first's from its geometry, the second had none
        // (never the first's carried over).
        assert_eq!(out[0].at, Some((47_610_000, -122_330_000)));
        assert_eq!(out[1].at, None);
        // Geometry after the properties (JSON fixes no order), a string
        // holding braces and quotes: each place still its own.
        let after = br#"{"features":[{"type":"Feature","properties":{"name":"A {odd} \"one\"","street":"First St"},"geometry":{"coordinates":[1.5,2.5]}},{"properties":{"street":"Second St"},"geometry":{"coordinates":[-3,-4]}}]}"#;
        let n = photon(after, &mut out);
        assert_eq!(n, 2);
        assert_eq!(out[0].street.get(), b"First St");
        assert_eq!(out[0].at, Some((2_500_000, 1_500_000)));
        assert_eq!(out[1].at, Some((-4_000_000, -3_000_000)));
    }

    #[test]
    fn coordinates_read_exactly_and_refuse_what_is_no_place() {
        let at = |s: &[u8]| coordinates(s);
        assert_eq!(
            at(br#""coordinates":[-122.3365,47.6116]"#),
            Some((47_611_600, -122_336_500))
        );
        // Spaces, seven places (cut), the edges of the Earth, a whole number.
        assert_eq!(
            at(br#""coordinates": [ 180.0, -90.1234567]"#),
            None,
            "past the south pole"
        );
        assert_eq!(
            at(br#""coordinates": [ 180.0000009, -90]"#),
            Some((-90_000_000, 180_000_000))
        );
        assert_eq!(at(br#""coordinates":[0,0]"#), Some((0, 0)));
        assert_eq!(
            at(br#""coordinates":[151.2093,-33.8688]"#),
            Some((-33_868_800, 151_209_300))
        );
        // Past the limits, exponents, junk, nothing.
        for bad in [
            &br#""coordinates":[180.000001,0]"#[..],
            br#""coordinates":[0,90.000001]"#,
            br#""coordinates":[1e2,3]"#,
            br#""coordinates":[1000,3]"#,
            br#""coordinates":["a",3]"#,
            br#""coordinates":[-,3]"#,
            br#""coordinates":[12]"#,
            br#""coordinates":"#,
            b"",
        ] {
            assert_eq!(at(bad), None, "{:?}", core::str::from_utf8(bad));
        }
    }

    #[test]
    fn photon_junk_gives_nothing_and_never_panics() {
        let mut out = [Place::EMPTY; 5];
        assert_eq!(photon(b"", &mut out), 0);
        assert_eq!(
            photon(b"{\"properties\":{\"street\":\"unterminated", &mut out),
            1
        );
        for cut in 0..PHOTON.len() {
            let _ = photon(&PHOTON[..cut], &mut out);
        }
    }

    const ECB: &[u8] = b"<gesmes:Envelope><Cube><Cube time='2024-01-02'><Cube currency='USD' rate='1.0956'/><Cube currency='JPY' rate='155.38'/><Cube currency='GBP' rate='0.86578'/></Cube></Cube></gesmes:Envelope>";

    #[test]
    fn ecb_rates_and_their_day() {
        let mut out = [Rate {
            code: [0; 3],
            micro: 0,
        }; 40];
        let (n, day) = ecb(ECB, &mut out);
        assert_eq!(n, 3);
        assert_eq!(
            out[0],
            Rate {
                code: *b"USD",
                micro: 1_095_600
            }
        );
        assert_eq!(out[1].micro, 155_380_000);
        assert_eq!(day, Some(19_724));
        for cut in 0..ECB.len() {
            let _ = ecb(&ECB[..cut], &mut out);
        }
    }

    const ICS: &[u8] = b"BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:Dentist\\, Dr. Lee\r\nLOCATION:410 Pine St\r\nDTSTART:20240102T230000Z\r\nDTEND:20240103T000000Z\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Mum's bir\r\n thday\r\nDTSTART;VALUE=DATE:19600315\r\nRRULE:FREQ=YEARLY\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Stand-up\r\nDTSTART:20240101T093000\r\nDURATION:PT15M\r\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR;COUNT=6\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn icalendar_events_with_folds_escapes_zones_and_rules() {
        let mut out = [Event {
            title: Short::new(),
            place: Short::new(),
            start: 0,
            minutes: 0,
            all_day: false,
            rule: None,
        }; 8];
        // Seattle in winter: UTC-8.
        let n = ics(ICS, -480, &mut out);
        assert_eq!(n, 3);
        assert_eq!(out[0].title.get(), b"Dentist, Dr. Lee");
        assert_eq!(out[0].place.get(), b"410 Pine St");
        // 23:00 UTC on the 2nd is 15:00 in Seattle.
        assert_eq!(out[0].start, date::start_of(19_724) + 15 * 60);
        assert_eq!(out[0].minutes, 60);
        assert_eq!(out[1].title.get(), b"Mum's birthday");
        assert!(out[1].all_day);
        assert_eq!(out[1].rule.map(|r| r.every), Some(Every::Year));
        assert_eq!(out[2].minutes, 15);
        let r = out[2].rule.expect("weekly kept");
        assert_eq!(r.days, 0b10101);
        // Six meetings from Monday 1 January: the 6th is Friday the 12th.
        assert_eq!(r.until, Some(19_734));
    }

    #[test]
    fn icalendar_junk_and_cuts_never_panic() {
        let mut out = [Event {
            title: Short::new(),
            place: Short::new(),
            start: 0,
            minutes: 0,
            all_day: false,
            rule: None,
        }; 8];
        assert_eq!(ics(b"", 0, &mut out), 0);
        assert_eq!(
            ics(b"BEGIN:VEVENT\nDTSTART:2024XX01\nEND:VEVENT\n", 0, &mut out),
            0
        );
        for cut in 0..ICS.len() {
            let _ = ics(&ICS[..cut], 0, &mut out);
        }
    }
}
