//! Phone numbers formatted as they are typed, for their country
//! (contacts-rule0.md version 2; design-language.md "The keyboard knows the
//! field").
//!
//! Found: Windows Mobile 5 kept a number exactly as typed; iOS and Android
//! format as you type from libphonenumber's metadata (about 250 regions,
//! hundreds of KB of patterns, most of them for validation and carriers).
//! Ours keeps only what formatting needs: per country its calling code,
//! its trunk prefix and the group lengths of its common national lengths,
//! a few bytes a country, and formats the digits typed so far, so the
//! groups appear while typing exactly as the finished number will read.
//! Stored as digits with the country (E.164), shown in the region's form.

/// A country's plan: its calling code, and how a national number of each
/// length is grouped (the groups' lengths; the first group in brackets for
/// the North American plan).
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    /// ISO 3166 code ("US").
    pub iso: [u8; 2],
    /// The calling code (1 for the US).
    pub code: u16,
    /// The trunk prefix dialled before a national number (0 in the UK), if
    /// any.
    pub trunk: Option<u8>,
    /// Group lengths for national numbers (without the trunk prefix), by
    /// total length, the first match wins.
    pub groups: &'static [&'static [u8]],
    /// The first group goes in brackets (the North American plan).
    pub brackets: bool,
}

/// The plans known (the international rule: more countries are rows here,
/// made from the public numbering plans).
pub static PLANS: [Plan; 8] = [
    Plan {
        iso: *b"US",
        code: 1,
        trunk: None,
        groups: &[&[3, 3, 4]],
        brackets: true,
    },
    Plan {
        iso: *b"CA",
        code: 1,
        trunk: None,
        groups: &[&[3, 3, 4]],
        brackets: true,
    },
    Plan {
        iso: *b"GB",
        code: 44,
        trunk: Some(0),
        // London 20 xxxx xxxx; mobiles 7xxx xxxxxx; others 1xxx xxxxxx.
        groups: &[&[2, 4, 4], &[4, 6]],
        brackets: false,
    },
    Plan {
        iso: *b"DE",
        code: 49,
        trunk: Some(0),
        groups: &[&[3, 8], &[3, 7], &[4, 7]],
        brackets: false,
    },
    Plan {
        iso: *b"FR",
        code: 33,
        trunk: Some(0),
        groups: &[&[1, 2, 2, 2, 2]],
        brackets: false,
    },
    Plan {
        iso: *b"AU",
        code: 61,
        trunk: Some(0),
        groups: &[&[1, 4, 4], &[3, 3, 3]],
        brackets: false,
    },
    Plan {
        iso: *b"JP",
        code: 81,
        trunk: Some(0),
        groups: &[&[2, 4, 4], &[3, 3, 4]],
        brackets: false,
    },
    Plan {
        iso: *b"MX",
        code: 52,
        trunk: None,
        groups: &[&[2, 4, 4], &[3, 3, 4]],
        brackets: false,
    },
];

/// The plan for an ISO code.
#[must_use]
pub fn plan(iso: [u8; 2]) -> Option<&'static Plan> {
    PLANS.iter().find(|p| p.iso == iso)
}

/// The plan for a calling code.
#[must_use]
pub fn plan_of_code(code: u16) -> Option<&'static Plan> {
    PLANS.iter().find(|p| p.code == code)
}

/// The longest formatted number.
pub const MAX: usize = 32;

/// A formatted number.
#[derive(Clone, Copy, Debug)]
pub struct Formatted {
    /// The text.
    pub b: [u8; MAX],
    /// Its length.
    pub n: usize,
}

impl Formatted {
    const fn new() -> Self {
        Self { b: [0; MAX], n: 0 }
    }
    fn push(&mut self, c: u8) {
        if self.n < MAX {
            self.b[self.n] = c;
            self.n += 1;
        }
    }
    /// The text.
    #[must_use]
    pub fn get(&self) -> &[u8] {
        &self.b[..self.n]
    }
}

/// Groups `digits` (typed so far) by `groups`; digits past the groups
/// follow the last group.
fn grouped(out: &mut Formatted, digits: &[u8], groups: &[u8], brackets: bool) {
    let mut at = 0;
    for (k, &g) in groups.iter().enumerate() {
        if at >= digits.len() {
            break;
        }
        let end = (at + usize::from(g)).min(digits.len());
        let last = k + 1 == groups.len();
        let end = if last { digits.len() } else { end };
        if k == 0 && brackets {
            out.push(b'(');
            digits[at..end].iter().for_each(|&d| out.push(d));
            // The bracket closes once the group is whole.
            if end - at == usize::from(g) && end < digits.len() {
                out.push(b')');
                out.push(b' ');
            } else if end - at == usize::from(g) {
                out.push(b')');
            }
        } else {
            // With brackets the separators are ") " and "-", pushed with
            // the groups before.
            if k > 0 && !brackets {
                out.push(b' ');
            }
            digits[at..end].iter().for_each(|&d| out.push(d));
        }
        if k == 1 && brackets && end < digits.len() {
            out.push(b'-');
        }
        at = end;
    }
}

/// The group lengths for a national number of `len` digits (typed so far:
/// the pattern whose total is at least `len`, else the longest).
fn groups_for(p: &Plan, national: &[u8]) -> &'static [u8] {
    // The UK's London numbers start 2; its others take the second pattern.
    if p.iso == *b"GB" {
        return if national.first() == Some(&b'2') {
            p.groups[0]
        } else {
            p.groups[1]
        };
    }
    let len = national.len();
    p.groups
        .iter()
        .copied()
        .find(|g| g.iter().map(|&x| usize::from(x)).sum::<usize>() >= len)
        .unwrap_or(p.groups[p.groups.len() - 1])
}

/// Formats what was typed (`typed`: digits and a leading `+`, anything else
/// ignored) in `home`'s plan: a national number in the national form
/// ("(206) 555-0142"), a `+` number in the international form
/// ("+44 20 7946 0958").
#[must_use]
pub fn format(typed: &[u8], home: &Plan) -> Formatted {
    let plus = typed.first() == Some(&b'+');
    let mut digits = [0u8; MAX];
    let mut n = 0;
    for &c in typed {
        if c.is_ascii_digit() && n < MAX {
            digits[n] = c;
            n += 1;
        }
    }
    let digits = &digits[..n];
    let mut out = Formatted::new();
    if !plus {
        // The North American plan's leading 1 is the long-distance prefix.
        let (lead, national) = if home.code == 1 && digits.first() == Some(&b'1') {
            (true, &digits[1..])
        } else {
            (false, digits)
        };
        if lead {
            out.push(b'1');
            if !national.is_empty() {
                out.push(b' ');
            }
        }
        let (trunk, national) = match home.trunk {
            Some(t) if national.first() == Some(&(b'0' + t)) => (true, &national[1..]),
            _ => (false, national),
        };
        if trunk {
            out.push(b'0' + home.trunk.unwrap_or(0));
        }
        grouped(
            &mut out,
            national,
            groups_for(home, national),
            home.brackets,
        );
        return out;
    }
    out.push(b'+');
    // The calling code: the longest known prefix of one to three digits.
    let found = (1..=3.min(digits.len())).rev().find_map(|k| {
        let code = digits[..k]
            .iter()
            .fold(0u16, |v, &d| v * 10 + u16::from(d - b'0'));
        plan_of_code(code).map(|p| (k, p))
    });
    match found {
        Some((k, p)) => {
            digits[..k].iter().for_each(|&d| out.push(d));
            let national = &digits[k..];
            if !national.is_empty() {
                out.push(b' ');
            }
            // International form: no brackets, groups by spaces.
            let groups = groups_for(p, national);
            let mut at = 0;
            for (i, &g) in groups.iter().enumerate() {
                if at >= national.len() {
                    break;
                }
                let end = if i + 1 == groups.len() {
                    national.len()
                } else {
                    (at + usize::from(g)).min(national.len())
                };
                if i > 0 {
                    out.push(if p.brackets && i == 2 { b'-' } else { b' ' });
                }
                national[at..end].iter().for_each(|&d| out.push(d));
                at = end;
            }
        }
        None => digits.iter().for_each(|&d| out.push(d)),
    }
    out
}

/// The number kept: E.164 ("+12065550142") from what was typed in `home`.
#[must_use]
pub fn e164(typed: &[u8], home: &Plan) -> Formatted {
    let mut out = Formatted::new();
    out.push(b'+');
    let plus = typed.first() == Some(&b'+');
    let digits = typed.iter().copied().filter(u8::is_ascii_digit);
    if plus {
        digits.for_each(|d| out.push(d));
        return out;
    }
    let mut code = [0u8; 5];
    let mut k = code.len();
    let mut c = home.code;
    loop {
        k -= 1;
        code[k] = b'0' + (c % 10) as u8;
        c /= 10;
        if c == 0 {
            break;
        }
    }
    code[k..].iter().for_each(|&d| out.push(d));
    let mut first = true;
    for d in digits {
        // The national prefixes don't belong in E.164.
        let skip = first && (Some(d - b'0') == home.trunk || (home.code == 1 && d == b'1'));
        first = false;
        if !skip {
            out.push(d);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn us() -> &'static Plan {
        plan(*b"US").unwrap()
    }

    #[test]
    fn a_us_number_forms_as_it_is_typed() {
        let typed = b"2065550142";
        let want: [&[u8]; 10] = [
            b"(2",
            b"(20",
            b"(206)",
            b"(206) 5",
            b"(206) 55",
            b"(206) 555",
            b"(206) 555-0",
            b"(206) 555-01",
            b"(206) 555-014",
            b"(206) 555-0142",
        ];
        for (k, w) in want.iter().enumerate() {
            assert_eq!(
                format(&typed[..=k], us()).get(),
                *w,
                "after {} digits",
                k + 1
            );
        }
    }

    #[test]
    fn the_long_distance_one_and_punctuation_typed_are_handled() {
        assert_eq!(format(b"12065550142", us()).get(), b"1 (206) 555-0142");
        assert_eq!(format(b"206-555.0142", us()).get(), b"(206) 555-0142");
    }

    #[test]
    fn international_numbers_take_their_country_s_groups() {
        assert_eq!(format(b"+442079460958", us()).get(), b"+44 20 7946 0958");
        assert_eq!(format(b"+12065550142", us()).get(), b"+1 206 555-0142");
        assert_eq!(format(b"+33142685300", us()).get(), b"+33 1 42 68 53 00");
        // An unknown code stays digits after the plus.
        assert_eq!(format(b"+999123", us()).get(), b"+999123");
    }

    #[test]
    fn national_numbers_abroad_keep_their_trunk_prefix() {
        let gb = plan(*b"GB").unwrap();
        assert_eq!(format(b"02079460958", gb).get(), b"020 7946 0958");
        assert_eq!(format(b"07700900123", gb).get(), b"07700 900123");
    }

    #[test]
    fn the_number_is_kept_as_e164() {
        assert_eq!(e164(b"(206) 555-0142", us()).get(), b"+12065550142");
        assert_eq!(e164(b"1 206 555 0142", us()).get(), b"+12065550142");
        assert_eq!(
            e164(b"020 7946 0958", plan(*b"GB").unwrap()).get(),
            b"+442079460958"
        );
        assert_eq!(e164(b"+44 20 7946 0958", us()).get(), b"+442079460958");
    }

    #[test]
    fn nothing_typed_formats_to_nothing_and_junk_is_ignored() {
        assert_eq!(format(b"", us()).get(), b"");
        assert_eq!(format(b"abc", us()).get(), b"");
        // Far too many digits never overflow.
        let long = [b'9'; 64];
        assert!(format(&long, us()).n <= MAX);
    }
}
