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
    /// No text.
    #[must_use]
    pub const fn new() -> Self {
        Self { b: [0; N], n: 0 }
    }
    /// `s` added, as much as fits.
    pub fn push(&mut self, s: &[u8]) {
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
    /// Whether it holds nothing.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.n == 0
    }
    /// Emptied (its bytes too: two empty texts are equal).
    pub const fn clear(&mut self) {
        *self = Self::new();
    }
    /// Whether it is full (a longer text was cut).
    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.n == N
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

// ---- iCalendar ---------------------------------------------------------

pub use crate::ics::{Event, ics, ics_each};

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
}
