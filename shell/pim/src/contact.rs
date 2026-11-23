//! A contact's richer fields (contacts-rule0.md "Many fields" and
//! "Duplicates"): a birthday and an anniversary with the year optional, a
//! structured address, any number of extra phones and e-mails with their
//! own labels, and the keys two cards are told to be one person by.
//!
//! Iterate, don't imitate (rule 0): vCard and Windows Mobile keep a field
//! per kind of number (Outlook: 19 fixed phone slots, most empty), and
//! iOS a list of labelled values in a database row. The store's records
//! hold at most 16 fields of up to 2,000 bytes, so ours keeps the three
//! common phones and the first e-mail in their own fields (every older
//! program reads them as before), and everything past them in one packed
//! field ([`tag::MORE`]) of labelled entries: an extra number costs its
//! label and digits and nothing else, and a record never runs out of
//! fields however many numbers a person has. A birthday is one number
//! (the day count) whose year 0004 says "year unknown": the same leap year
//! keeps 29 February, and the Birthdays calendar (calendar-rule0.md) reads
//! it with no other field.
//!
//! [`tag::MORE`]: crate::store::tag::MORE

use crate::date::{self, Date};

/// The year a day without one is kept in: a leap year, so 29 February
/// is a day too.
pub const YEAR_UNKNOWN: i32 = 4;

/// A birthday or an anniversary: a day of a month, the year when known.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Day {
    /// The year, when known.
    pub year: Option<i32>,
    /// 1..=12.
    pub month: u8,
    /// 1..=31.
    pub day: u8,
}

impl Day {
    /// The field as kept ([`crate::store::tag::BIRTHDAY`],
    /// [`crate::store::tag::ANNIVERSARY`]): the day count from 1970 as a
    /// little-endian two's-complement number; `None` for a day that does
    /// not exist (31 April, 29 February of 2023).
    #[must_use]
    pub fn field(self) -> Option<[u8; 4]> {
        let d = Date {
            year: self.year.unwrap_or(YEAR_UNKNOWN),
            month: self.month,
            day: self.day,
        };
        // A known year of 4 would read back as unknown: the years a card
        // can mean are 1000 on.
        if !date::valid(d) || self.year.is_some_and(|y| !(1000..=9999).contains(&y)) {
            return None;
        }
        Some((date::days_from(d) as u32).to_le_bytes())
    }

    /// A kept field read back (`None` for a field of another length).
    #[must_use]
    pub fn of_field(raw: &[u8]) -> Option<Self> {
        let b: [u8; 4] = raw.try_into().ok()?;
        Some(Self::of_days(u32::from_le_bytes(b) as i32))
    }

    /// The day of a day count, year 0004 read as unknown.
    #[must_use]
    pub const fn of_days(days: i32) -> Self {
        let d = date::date_of(days);
        Self {
            year: if d.year == YEAR_UNKNOWN {
                None
            } else {
                Some(d.year)
            },
            month: d.month,
            day: d.day,
        }
    }

    /// The day count it falls on in `year` (29 February on 28 February in
    /// a year without it, as most people keep it).
    #[must_use]
    pub const fn in_year(self, year: i32) -> i32 {
        let mut d = Date {
            year,
            month: self.month,
            day: self.day,
        };
        if !date::valid(d) {
            d.day = 28;
        }
        date::days_from(d)
    }

    /// The first day on or after day count `from` that it falls on: what
    /// the Birthdays calendar shows next.
    #[must_use]
    pub const fn next_from(self, from: i32) -> i32 {
        let y = date::date_of(from).year;
        let this = self.in_year(y);
        if this >= from {
            this
        } else {
            self.in_year(y + 1)
        }
    }

    /// The years since it, on day count `on` (an age, the years married);
    /// `None` without a year or before it.
    #[must_use]
    pub const fn years_on(self, on: i32) -> Option<u32> {
        let Some(born) = self.year else { return None };
        let d = date::date_of(on);
        let mut n = d.year - born;
        if d.month < self.month || (d.month == self.month && d.day < self.day) {
            n -= 1;
        }
        if n < 0 { None } else { Some(n as u32) }
    }
}

// ---- The address ------------------------------------------------------

/// An address in its parts. Kept in [`crate::store::tag::ADDRESS`] as five
/// lines: street, city, region, postal code, country. A record of the
/// first version kept two lines, the street and "City, ST 12345"; it is
/// read into its parts the same way.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Address<'a> {
    /// "410 Pine St, Apt 4".
    pub street: &'a [u8],
    /// "Seattle".
    pub city: &'a [u8],
    /// The state or province: "WA".
    pub region: &'a [u8],
    /// The ZIP or postal code.
    pub postal: &'a [u8],
    /// The country, empty for the device's own.
    pub country: &'a [u8],
}

fn trim(s: &[u8]) -> &[u8] {
    let a = s
        .iter()
        .position(|c| !c.is_ascii_whitespace())
        .unwrap_or(s.len());
    let b = s
        .iter()
        .rposition(|c| !c.is_ascii_whitespace())
        .map_or(a, |i| i + 1);
    &s[a..b.max(a)]
}

impl<'a> Address<'a> {
    /// The parts of a kept address.
    #[must_use]
    pub fn of(raw: &'a [u8]) -> Self {
        let mut lines = raw.split(|&c| c == b'\n');
        let street = trim(lines.next().unwrap_or(&[]));
        let second = lines.next();
        let third = lines.next();
        match (second, third) {
            // Five lines (or more: the rest ignored).
            (Some(city), Some(region)) => Self {
                street,
                city: trim(city),
                region: trim(region),
                postal: trim(lines.next().unwrap_or(&[])),
                country: trim(lines.next().unwrap_or(&[])),
            },
            (Some(line), None) => {
                let mut a = Self::city_line(line);
                a.street = street;
                a
            }
            (None, _) => Self {
                street,
                ..Self::default()
            },
        }
    }

    /// A city line as people and address services write it, in its parts:
    /// "Seattle, WA 98101", "Seattle, Washington 98101-1234", "Seattle,
    /// WA", "Seattle". A line it can't take apart is the city whole.
    #[must_use]
    pub fn city_line(line: &'a [u8]) -> Self {
        let line = trim(line);
        let Some(comma) = line.iter().rposition(|&c| c == b',') else {
            return Self {
                city: line,
                ..Self::default()
            };
        };
        let city = trim(&line[..comma]);
        let rest = trim(&line[comma + 1..]);
        // The last word is the postal code when it holds a digit.
        let (region, postal) = match rest.iter().rposition(|&c| c == b' ') {
            Some(sp) if rest[sp + 1..].iter().any(u8::is_ascii_digit) => {
                (trim(&rest[..sp]), &rest[sp + 1..])
            }
            None if rest.iter().any(u8::is_ascii_digit) => (&b""[..], rest),
            _ => (rest, &b""[..]),
        };
        Self {
            street: &[],
            city,
            region,
            postal,
            country: &[],
        }
    }

    /// Whether every part is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parts().iter().all(|p| p.is_empty())
    }

    /// The parts in their kept order.
    #[must_use]
    pub const fn parts(&self) -> [&'a [u8]; 5] {
        [
            self.street,
            self.city,
            self.region,
            self.postal,
            self.country,
        ]
    }

    /// The field as kept into `out`: five lines (empty when every part
    /// is), a part's own newlines turned to spaces. Its length, or `None`
    /// when it doesn't fit.
    #[must_use]
    pub fn field(&self, out: &mut [u8]) -> Option<usize> {
        if self.is_empty() {
            return Some(0);
        }
        let mut n = 0;
        for (k, p) in self.parts().iter().enumerate() {
            if k > 0 {
                *out.get_mut(n)? = b'\n';
                n += 1;
            }
            for &c in trim(p) {
                *out.get_mut(n)? = if c == b'\n' || c == b'\r' { b' ' } else { c };
                n += 1;
            }
        }
        Some(n)
    }

    /// The city line as the card shows it: "Seattle, WA 98101" (the
    /// region's order; a country other than the device's after it).
    #[must_use]
    pub fn city_text(&self, out: &mut [u8]) -> usize {
        let mut n = 0;
        let mut put = |s: &[u8], n: &mut usize| {
            for &c in s {
                if let Some(o) = out.get_mut(*n) {
                    *o = c;
                    *n += 1;
                }
            }
        };
        put(self.city, &mut n);
        if !self.region.is_empty() {
            if n > 0 {
                put(b", ", &mut n);
            }
            put(self.region, &mut n);
        }
        if !self.postal.is_empty() {
            if n > 0 {
                put(b" ", &mut n);
            }
            put(self.postal, &mut n);
        }
        if !self.country.is_empty() {
            if n > 0 {
                put(b", ", &mut n);
            }
            put(self.country, &mut n);
        }
        n
    }
}

// ---- Extra phones and e-mails -----------------------------------------

/// What an extra entry is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A phone number, kept as E.164 when it could be read as one.
    Phone = 1,
    /// An e-mail address.
    Email = 2,
}

/// The longest label kept.
pub const LABEL_MAX: usize = 24;
/// The longest value kept.
pub const VALUE_MAX: usize = 64;

/// One extra phone or e-mail and its label ("Cabin", "School").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry<'a> {
    /// A phone or an e-mail.
    pub kind: Kind,
    /// Its label, as typed (empty: "Other").
    pub label: &'a [u8],
    /// The number or the address.
    pub value: &'a [u8],
}

/// The entries of a kept [`crate::store::tag::MORE`] field, in order: each
/// a kind byte, the label's length and the label, the value's length and
/// the value. A damaged entry ends the list (hostile input: the field can
/// come from an older program or a damaged card).
pub fn entries(raw: &[u8]) -> impl Iterator<Item = Entry<'_>> {
    let mut at = raw;
    core::iter::from_fn(move || {
        let (&k, rest) = at.split_first()?;
        let kind = match k {
            1 => Kind::Phone,
            2 => Kind::Email,
            _ => {
                at = &[];
                return None;
            }
        };
        let (&ln, rest) = rest.split_first()?;
        let (label, rest) = rest.split_at_checked(usize::from(ln))?;
        let (&vn, rest) = rest.split_first()?;
        let Some((value, rest)) = rest.split_at_checked(usize::from(vn)) else {
            at = &[];
            return None;
        };
        at = rest;
        Some(Entry { kind, label, value })
    })
}

/// `e` added at `out[n..]` (its label and value cut to [`LABEL_MAX`] and
/// [`VALUE_MAX`]): the new length, or `None` when it doesn't fit.
#[must_use]
pub fn push_entry(out: &mut [u8], n: usize, e: Entry<'_>) -> Option<usize> {
    let label = &e.label[..e.label.len().min(LABEL_MAX)];
    let value = &e.value[..e.value.len().min(VALUE_MAX)];
    let end = n + 3 + label.len() + value.len();
    let b = out.get_mut(n..end)?;
    b[0] = e.kind as u8;
    b[1] = label.len() as u8;
    b[2..2 + label.len()].copy_from_slice(label);
    b[2 + label.len()] = value.len() as u8;
    b[3 + label.len()..].copy_from_slice(value);
    Some(end)
}

// ---- Duplicates -------------------------------------------------------

/// A phone number's digits for matching: the last ten (the national
/// number in the US and most plans; a stored "+1 206..." and an imported
/// "206..." are the same line). `None` under seven digits (an extension,
/// a short code: too common to mean the same person).
#[must_use]
pub fn phone_key(number: &[u8]) -> Option<[u8; 10]> {
    let n = number.iter().filter(|c| c.is_ascii_digit()).count();
    if n < 7 {
        return None;
    }
    let mut k = [0u8; 10];
    let mut i = 10;
    for &c in number.iter().rev().filter(|c| c.is_ascii_digit()) {
        if i == 0 {
            break;
        }
        i -= 1;
        k[i] = c;
    }
    Some(k)
}

/// Whether two e-mail addresses are one (case ignored, as every mail
/// system treats them in practice; spaces around them dropped).
#[must_use]
pub fn same_email(a: &[u8], b: &[u8]) -> bool {
    let (a, b) = (trim(a), trim(b));
    !a.is_empty() && a.contains(&b'@') && a.eq_ignore_ascii_case(b)
}

/// Whether two names are one person's ("Ada Lovelace" and "ada
/// lovelace"), both non-empty.
#[must_use]
pub fn same_name(first_a: &[u8], last_a: &[u8], first_b: &[u8], last_b: &[u8]) -> bool {
    let (fa, la, fb, lb) = (trim(first_a), trim(last_a), trim(first_b), trim(last_b));
    !(fa.is_empty() && la.is_empty()) && fa.eq_ignore_ascii_case(fb) && la.eq_ignore_ascii_case(lb)
}

/// A contact record's matching keys (a phone's last ten digits, an
/// e-mail's FNV-1a hash, case ignored), up to `out`'s room: how many. Two
/// records with a key in common are offered as one person
/// ([`same_email`], [`phone_key`]).
#[must_use]
pub fn keys(r: &crate::store::Rec<'_>, out: &mut [u64; 16]) -> usize {
    use crate::store::tag;
    let mut n = 0;
    let mut add = |k: u64| {
        if n < out.len() && !out[..n].contains(&k) {
            out[n] = k;
            n += 1;
        }
    };
    let phone = |v: &[u8]| {
        phone_key(v).map(|k| {
            k.iter()
                .fold(0u64, |a, &d| a * 10 + u64::from(d.wrapping_sub(b'0')))
        })
    };
    let mail = |v: &[u8]| {
        let v = trim(v);
        v.contains(&b'@').then(|| {
            // Phones' keys are under 10^10; a mail's hash is kept above.
            v.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &c| {
                (h ^ u64::from(c.to_ascii_lowercase())).wrapping_mul(0x100_0000_01b3)
            }) | 1 << 63
        })
    };
    for t in [tag::MOBILE, tag::HOME, tag::WORK] {
        if let Some(k) = phone(r.text(t)) {
            add(k);
        }
    }
    if let Some(k) = mail(r.text(tag::EMAIL)) {
        add(k);
    }
    for e in entries(r.text(tag::MORE)) {
        let k = match e.kind {
            Kind::Phone => phone(e.value),
            Kind::Email => mail(e.value),
        };
        if let Some(k) = k {
            add(k);
        }
    }
    n
}

#[cfg(test)]
#[path = "contact_tests.rs"]
mod tests;
