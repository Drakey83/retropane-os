//! The editor's fields (contacts-rule0.md "Many fields"): the typed
//! fields, the extra phones and e-mails with their own labels, and the
//! days typed the region's way.
//!
//! Iterate, don't imitate (rule 0): Windows Mobile's editor was one long
//! form of every Outlook field, most empty; iOS shows the common ones and
//! "add phone", "add e-mail", "add field" for the rest. Ours shows the
//! common ones with **+ Add** under the phones and e-mails (a new row with
//! its own label, typed or picked from the strip over the keyboard), the
//! address in its five parts (an address service's line split into them),
//! and a birthday or anniversary typed as digits the region's way
//! (12/10 or 12/10/1815 in the US, 10/12 elsewhere), checked as typed.

use rp_pim::contact::Day;

/// A field the editor shows, in the form's order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditField {
    /// The first name.
    First,
    /// The last name.
    Last,
    /// The company.
    Company,
    /// The job title.
    Job,
    /// The mobile number.
    Mobile,
    /// The home number.
    Home,
    /// The work number.
    Work,
    /// The e-mail address.
    Email,
    /// The street.
    Street,
    /// The city.
    City,
    /// The state or province.
    Region,
    /// The ZIP or postal code.
    Postal,
    /// The country.
    Country,
    /// The birthday.
    Birthday,
    /// The anniversary.
    Anniversary,
    /// A web address.
    Website,
    /// A favourite or not (a switch).
    Favourite,
    /// Delete this person (asked twice).
    Delete,
    /// Adds a phone row.
    AddPhone,
    /// Adds an e-mail row.
    AddEmail,
    /// Extra phone or e-mail `k`'s value.
    Extra(u8),
    /// Extra `k`'s label being typed.
    Label(u8),
}

/// The typed fields' room.
pub const SLOTS: usize = 16;
/// Extra phones and e-mails at most (the record's packed field holds
/// more; the editor shows this many).
pub const EXTRAS: usize = 7;

impl EditField {
    /// Its typed text's slot ([`FORM`]); `None` for a switch, a key or an
    /// extra.
    #[must_use]
    pub const fn slot(self) -> Option<usize> {
        Some(match self {
            Self::First => 0,
            Self::Last => 1,
            Self::Company => 2,
            Self::Job => 3,
            Self::Mobile => 4,
            Self::Home => 5,
            Self::Work => 6,
            Self::Email => 7,
            Self::Street => 8,
            Self::City => 9,
            Self::Region => 10,
            Self::Postal => 11,
            Self::Country => 12,
            Self::Birthday => 13,
            Self::Anniversary => 14,
            Self::Website => 15,
            _ => return None,
        })
    }

    /// Whether it takes typing.
    #[must_use]
    pub const fn typed(self) -> bool {
        self.slot().is_some() || matches!(self, Self::Extra(_) | Self::Label(_))
    }
}

/// The typed fields: label, hint and the most they hold.
pub const FORM: [(&[u8], &[u8], usize); SLOTS] = [
    (b"FIRST", b"First name", 24),
    (b"LAST", b"Last name", 24),
    (b"COMPANY", b"Company", 32),
    (b"JOB TITLE", b"Job title", 32),
    (b"MOBILE", b"(555) 555-0100", 20),
    (b"HOME", b"Home number", 20),
    (b"WORK", b"Work number", 20),
    (b"E-MAIL", b"name@example.com", 40),
    (b"STREET", b"Start typing an address", 40),
    (b"CITY", b"City", 32),
    (b"STATE", b"State or province", 24),
    (b"ZIP", b"ZIP or postal code", 12),
    (b"COUNTRY", b"Country (blank: this one)", 32),
    (b"BIRTHDAY", b"MM/DD/YYYY, the year if known", 10),
    (b"ANNIV.", b"MM/DD/YYYY, the year if known", 10),
    (b"WEBSITE", b"example.com", 40),
];

/// The labels the keyboard's strip offers for an extra phone, and for an
/// extra e-mail (any other is typed).
pub const PHONE_LABELS: [&[u8]; 3] = [b"Mobile", b"Main", b"Other"];
/// See [`PHONE_LABELS`].
pub const EMAIL_LABELS: [&[u8]; 3] = [b"Home", b"Work", b"Other"];

/// An extra phone or e-mail being edited.
#[derive(Clone, Copy, Debug)]
pub struct Extra {
    /// A phone (else an e-mail).
    pub phone: bool,
    /// Its label.
    pub label: [u8; 24],
    /// The label's length.
    pub label_n: usize,
    /// The number or the address.
    pub value: [u8; 40],
    /// The value's length.
    pub value_n: usize,
}

impl Extra {
    /// An empty row.
    pub const EMPTY: Self = Self {
        phone: true,
        label: [0; 24],
        label_n: 0,
        value: [0; 40],
        value_n: 0,
    };

    /// A row of `label` and `value`.
    #[must_use]
    pub fn of(phone: bool, label: &[u8], value: &[u8]) -> Self {
        let mut e = Self::EMPTY;
        e.phone = phone;
        e.label_n = label.len().min(24);
        e.label[..e.label_n].copy_from_slice(&label[..e.label_n]);
        e.value_n = value.len().min(40);
        e.value[..e.value_n].copy_from_slice(&value[..e.value_n]);
        e
    }

    /// Its label, "Other" when none was given.
    #[must_use]
    pub fn label(&self) -> &[u8] {
        if self.label_n == 0 {
            b"Other"
        } else {
            &self.label[..self.label_n]
        }
    }
}

/// A day typed the region's way ("12/10/1815" or "12/10" in the US, the
/// day first elsewhere; any of `/ - . ` between): `Some(None)` for nothing
/// typed, `None` for no such day.
#[must_use]
pub fn day_typed(s: &[u8], us: bool) -> Option<Option<Day>> {
    let mut n = [0u32; 3];
    let mut k = 0;
    let mut digits = 0;
    for &c in s {
        if c.is_ascii_digit() {
            if k == 3 || digits == 4 {
                return None;
            }
            n[k] = n[k] * 10 + u32::from(c - b'0');
            digits += 1;
        } else if b"/-. ".contains(&c) {
            if digits > 0 {
                k += 1;
                digits = 0;
            }
        } else {
            return None;
        }
    }
    let parts = k + usize::from(digits > 0);
    if parts == 0 {
        return Some(None);
    }
    if parts == 1 {
        return None;
    }
    let (month, day) = if us { (n[0], n[1]) } else { (n[1], n[0]) };
    let year = (parts == 3).then(|| {
        let y = n[2] as i32;
        // Two digits: this century's if not past this year's thirties.
        match y {
            0..=29 => 2000 + y,
            30..=99 => 1900 + y,
            _ => y,
        }
    });
    let d = Day {
        year,
        month: u8::try_from(month).ok()?,
        day: u8::try_from(day).ok()?,
    };
    d.field().map(|_| Some(d))
}

/// A day as typed back into the editor.
#[must_use]
pub fn day_text(d: Day, us: bool) -> ([u8; 10], usize) {
    let mut b = [0u8; 10];
    let mut n = 0;
    let two = |v: u8, b: &mut [u8; 10], n: &mut usize| {
        b[*n] = b'0' + v / 10;
        b[*n + 1] = b'0' + v % 10;
        *n += 2;
    };
    let (a, c) = if us {
        (d.month, d.day)
    } else {
        (d.day, d.month)
    };
    two(a, &mut b, &mut n);
    b[n] = b'/';
    n += 1;
    two(c, &mut b, &mut n);
    if let Some(y) = d.year {
        b[n] = b'/';
        n += 1;
        let y = y.clamp(0, 9999) as u16;
        for p in [1000, 100, 10, 1] {
            b[n] = b'0' + (y / p % 10) as u8;
            n += 1;
        }
    }
    (b, n)
}

/// A day as the card shows it: "December 10, 1815" in the US, "10
/// December 1815" elsewhere, the year left out when unknown.
#[must_use]
pub fn day_shown(d: Day, us: bool) -> ([u8; 24], usize) {
    const MONTHS: [&[u8]; 12] = [
        b"January",
        b"February",
        b"March",
        b"April",
        b"May",
        b"June",
        b"July",
        b"August",
        b"September",
        b"October",
        b"November",
        b"December",
    ];
    let mut b = [0u8; 24];
    let mut n = 0;
    let put = |s: &[u8], b: &mut [u8; 24], n: &mut usize| {
        for &c in s {
            if *n < 24 {
                b[*n] = c;
                *n += 1;
            }
        }
    };
    let dd = [b'0' + d.day / 10, b'0' + d.day % 10];
    let day = if d.day >= 10 { &dd[..] } else { &dd[1..] };
    let month = MONTHS[usize::from(d.month.clamp(1, 12) - 1)];
    if us {
        put(month, &mut b, &mut n);
        put(b" ", &mut b, &mut n);
        put(day, &mut b, &mut n);
    } else {
        put(day, &mut b, &mut n);
        put(b" ", &mut b, &mut n);
        put(month, &mut b, &mut n);
    }
    if let Some(y) = d.year {
        put(if us { b", " } else { b" " }, &mut b, &mut n);
        let y = y.clamp(0, 9999) as u16;
        let digits = [
            b'0' + (y / 1000) as u8,
            b'0' + (y / 100 % 10) as u8,
            b'0' + (y / 10 % 10) as u8,
            b'0' + (y % 10) as u8,
        ];
        put(&digits, &mut b, &mut n);
    }
    (b, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_are_typed_the_region_s_way_and_checked() {
        let d = |y, m, dd| {
            Some(Some(Day {
                year: y,
                month: m,
                day: dd,
            }))
        };
        assert_eq!(day_typed(b"12/10/1815", true), d(Some(1815), 12, 10));
        assert_eq!(day_typed(b"10/12/1815", false), d(Some(1815), 12, 10));
        assert_eq!(day_typed(b"12/10", true), d(None, 12, 10));
        assert_eq!(day_typed(b"2-29", true), d(None, 2, 29));
        assert_eq!(day_typed(b"7.8.35", true), d(Some(1935), 7, 8));
        assert_eq!(day_typed(b"7 8 05", true), d(Some(2005), 7, 8));
        assert_eq!(day_typed(b"", true), Some(None));
        assert_eq!(day_typed(b"  ", true), Some(None));
        for bad in [
            &b"13/01"[..],
            b"02/30",
            b"2/29/2023",
            b"12",
            b"1/2/3/4",
            b"ab/cd",
            b"12345/1",
            b"1/1/99999",
        ] {
            assert_eq!(
                day_typed(bad, true),
                None,
                "{}",
                core::str::from_utf8(bad).unwrap()
            );
        }
        let x = Day {
            year: Some(1815),
            month: 12,
            day: 10,
        };
        let (b, n) = day_text(x, true);
        assert_eq!(day_typed(&b[..n], true), Some(Some(x)));
        let (b, n) = day_text(Day { year: None, ..x }, false);
        assert_eq!(&b[..n], b"10/12");
        let (b, n) = day_shown(x, true);
        assert_eq!(&b[..n], b"December 10, 1815");
        let (b, n) = day_shown(
            Day {
                year: None,
                month: 7,
                day: 8,
            },
            false,
        );
        assert_eq!(&b[..n], b"8 July");
    }
}
