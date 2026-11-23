//! vCard files read and written (contacts-rule0.md "Same people
//! everywhere": vCard 3 and 4 import and export, RFC 2426 and RFC 6350;
//! the 2.1 files old phones and Outlook still write are read too).
//!
//! Every file is hostile input (security.md): no length or count in it is
//! trusted, a line of any length is walked in place (a photo is tens of
//! kilobytes on one folded line) and never copied whole, every field is
//! cut to its room, and a damaged card yields what could be read, never a
//! panic. A card without a name, a number or an address is still a card:
//! an old file is never refused for lacking a newer field (the legacy
//! rule). What the device doesn't keep is counted ([`Card::skipped`]) so
//! the import can say so, never dropped in silence.
//!
//! Iterate, don't imitate (rule 0): Windows Mobile's ActiveSync never read
//! a vCard; Outlook reads 2.1 and 3.0 and writes 2.1; iOS and Google read
//! 3.0 and 4.0 and write 3.0. Ours reads all three, takes the labels each
//! writes its own way (2.1's bare `CELL`, 3.0's `TYPE=`, 4.0's quoted
//! lists and `tel:` URIs, Apple's `item1.X-ABLabel` groups and its 1604
//! "no year" birthdays, Outlook's `X-MS-ANNIVERSARY`), and writes 3.0 by
//! default (what every importer takes) or 4.0 on request, custom labels
//! as the `X-ABLabel` groups Apple and Google both read back. A desktop
//! library (libical, vobject) is a DOM of allocations; this is one pass
//! with a fixed buffer.

use crate::contact::{Address, Day};
use crate::net::Short;

/// Phones a card keeps.
pub const PHONES: usize = 8;
/// E-mails a card keeps.
pub const EMAILS: usize = 6;
/// The longest line this reader copies to decode (a note longer is cut;
/// a photo is never copied, only located).
const VALUE: usize = 512;

/// A labelled value: a phone or an e-mail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Labelled {
    /// "MOBILE", "HOME", "WORK", or a label of its own ("Cabin").
    pub label: Short<24>,
    /// The number as written, or the address.
    pub value: Short<64>,
    /// Its group (`item1`), for Apple's labels on a later line.
    group: Short<8>,
}

/// Where a card's picture lies in the file: the folded value, its bytes
/// from `start` to `end`, base64 with line folds (read with
/// [`photo_bytes`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Photo {
    /// The value's first byte in the file.
    pub start: usize,
    /// One past its last.
    pub end: usize,
}

/// One person, as read from a vCard or to be written as one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Card {
    /// Given name.
    pub first: Short<24>,
    /// Family name.
    pub last: Short<24>,
    /// The organisation.
    pub company: Short<32>,
    /// The job title.
    pub job: Short<32>,
    /// The phones, in the file's order.
    pub phones: [Labelled; PHONES],
    /// How many.
    pub n_phones: usize,
    /// The e-mails.
    pub emails: [Labelled; EMAILS],
    /// How many.
    pub n_emails: usize,
    /// The address's parts: street, city, region, postal code, country.
    pub address: [Short<48>; 5],
    /// The birthday.
    pub birthday: Option<Day>,
    /// The anniversary.
    pub anniversary: Option<Day>,
    /// A web address.
    pub url: Short<96>,
    /// The note.
    pub note: Short<400>,
    /// The picture, when the file carries one inline.
    pub photo: Option<Photo>,
    /// Fields read but not kept: a second address, a picture only linked,
    /// numbers past [`PHONES`], nicknames, chat names.
    pub skipped: u16,
}

impl Default for Card {
    fn default() -> Self {
        Self::new()
    }
}

impl Card {
    /// No one.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            first: Short::new(),
            last: Short::new(),
            company: Short::new(),
            job: Short::new(),
            phones: [Labelled {
                label: Short::new(),
                value: Short::new(),
                group: Short::new(),
            }; PHONES],
            n_phones: 0,
            emails: [Labelled {
                label: Short::new(),
                value: Short::new(),
                group: Short::new(),
            }; EMAILS],
            n_emails: 0,
            address: [Short::new(); 5],
            birthday: None,
            anniversary: None,
            url: Short::new(),
            note: Short::new(),
            photo: None,
            skipped: 0,
        }
    }

    /// Whether anything was read into it (a card of only a VERSION line is
    /// none).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.first.is_empty()
            && self.last.is_empty()
            && self.company.is_empty()
            && self.n_phones == 0
            && self.n_emails == 0
            && self.address.iter().all(Short::is_empty)
    }

    /// A phone added (`None`: no room).
    pub fn add_phone(&mut self, label: &[u8], value: &[u8]) -> Option<()> {
        let p = self.phones.get_mut(self.n_phones)?;
        *p = Labelled::default();
        p.label.push(label);
        p.value.push(value);
        self.n_phones += 1;
        Some(())
    }

    /// An e-mail added (`None`: no room).
    pub fn add_email(&mut self, label: &[u8], value: &[u8]) -> Option<()> {
        let p = self.emails.get_mut(self.n_emails)?;
        *p = Labelled::default();
        p.label.push(label);
        p.value.push(value);
        self.n_emails += 1;
        Some(())
    }

    /// The address as [`Address`] parts.
    #[must_use]
    pub fn address(&self) -> Address<'_> {
        Address {
            street: self.address[0].get(),
            city: self.address[1].get(),
            region: self.address[2].get(),
            postal: self.address[3].get(),
            country: self.address[4].get(),
        }
    }
}

// ---- Reading ----------------------------------------------------------

/// A logical line's bytes with its folds taken out (RFC 6350 3.2: a line
/// break followed by a space or a tab continues the line). In a
/// quoted-printable value (2.1) the breaks are left for the decoder: its
/// soft breaks end in `=`.
#[derive(Clone)]
struct Unfold<'a> {
    raw: &'a [u8],
    at: usize,
    qp: bool,
}

impl Iterator for Unfold<'_> {
    type Item = u8;
    fn next(&mut self) -> Option<u8> {
        loop {
            let c = *self.raw.get(self.at)?;
            self.at += 1;
            match c {
                b'\r' if !self.qp => {}
                b'\n' if !self.qp => {
                    if matches!(self.raw.get(self.at), Some(b' ' | b'\t')) {
                        self.at += 1;
                    }
                }
                _ => return Some(c),
            }
        }
    }
}

/// The logical lines of `text`: each line's raw bytes (folds and all,
/// without its final line break) and where it starts.
struct Lines<'a> {
    text: &'a [u8],
    at: usize,
}

impl<'a> Iterator for Lines<'a> {
    type Item = (usize, &'a [u8]);
    fn next(&mut self) -> Option<(usize, &'a [u8])> {
        let start = self.at;
        if start >= self.text.len() {
            return None;
        }
        let head_end = self.text[start..]
            .iter()
            .position(|&c| c == b':' || c == b'\n')
            .map_or(self.text.len(), |k| start + k);
        let qp = contains_nocase(&self.text[start..head_end], b"QUOTED-PRINTABLE");
        let mut p = start;
        let end = loop {
            match self.text[p..].iter().position(|&c| c == b'\n') {
                None => {
                    self.at = self.text.len();
                    break self.text.len();
                }
                Some(k) => {
                    let nl = p + k;
                    let next = self.text.get(nl + 1);
                    let before = &self.text[start..nl];
                    let before = before.strip_suffix(b"\r").unwrap_or(before);
                    let soft = qp && before.last() == Some(&b'=');
                    if matches!(next, Some(b' ' | b'\t')) || (soft && next.is_some()) {
                        p = nl + 1;
                        continue;
                    }
                    self.at = nl + 1;
                    break nl;
                }
            }
        };
        let mut line = &self.text[start..end];
        if line.last() == Some(&b'\r') {
            line = &line[..line.len() - 1];
        }
        Some((start, line))
    }
}

fn contains_nocase(hay: &[u8], needle: &[u8]) -> bool {
    needle.len() <= hay.len()
        && hay
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle))
}

/// A line's name and parameters, upper-cased, up to the `:` outside quotes.
struct Head {
    b: [u8; 160],
    n: usize,
    /// Where the name (after a group) starts and ends.
    name: (usize, usize),
    group: (usize, usize),
}

impl Head {
    fn name(&self) -> &[u8] {
        &self.b[self.name.0..self.name.1]
    }
    fn group(&self) -> &[u8] {
        &self.b[self.group.0..self.group.1]
    }
    /// The parameters' text: `;TYPE=CELL;PREF=1`.
    fn params(&self) -> &[u8] {
        &self.b[self.name.1..self.n]
    }
    /// Whether a TYPE (or 2.1's bare word) of `word` is given.
    fn has_type(&self, word: &[u8]) -> bool {
        self.params()
            .split(|&c| c == b';' || c == b',' || c == b'=' || c == b'"')
            .any(|w| w == word)
    }
    /// A parameter's value (`ENCODING`, `CHARSET`, `VALUE`).
    fn param(&self, key: &[u8]) -> Option<&[u8]> {
        self.params().split(|&c| c == b';').find_map(|p| {
            let eq = p.iter().position(|&c| c == b'=')?;
            (p[..eq] == *key).then(|| {
                let v = &p[eq + 1..];
                v.strip_prefix(b"\"")
                    .and_then(|v| v.strip_suffix(b"\""))
                    .unwrap_or(v)
            })
        })
    }
}

/// The head of `line`, and the value's unfolded bytes after it.
fn split_line(line: &[u8]) -> Option<(Head, Unfold<'_>)> {
    let mut u = Unfold {
        raw: line,
        at: 0,
        qp: false,
    };
    let mut h = Head {
        b: [0; 160],
        n: 0,
        name: (0, 0),
        group: (0, 0),
    };
    let mut quoted = false;
    let mut name_end = None;
    loop {
        let c = u.next()?;
        if c == b'"' {
            quoted = !quoted;
        }
        if c == b':' && !quoted {
            break;
        }
        if c == b';' && name_end.is_none() {
            name_end = Some(h.n);
        }
        if h.n < h.b.len() {
            h.b[h.n] = c.to_ascii_uppercase();
            h.n += 1;
        }
    }
    let name_end = name_end.unwrap_or(h.n);
    // "item1.TEL": the group before the dot.
    match h.b[..name_end].iter().position(|&c| c == b'.') {
        Some(dot) => {
            h.group = (0, dot);
            h.name = (dot + 1, name_end);
        }
        None => h.name = (0, name_end),
    }
    u.qp = contains_nocase(h.params(), b"QUOTED-PRINTABLE");
    Some((h, u))
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'A'..=b'F' => Some(c - b'A' + 10),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// The value decoded into `out` (quoted-printable undone, Latin-1 made
/// UTF-8, escapes kept for [`components`]): its length.
fn decode(h: &Head, mut v: Unfold<'_>, out: &mut [u8; VALUE]) -> usize {
    let latin1 = h
        .param(b"CHARSET")
        .is_some_and(|c| c != b"UTF-8" && c != b"US-ASCII");
    let mut n = 0;
    let mut put = |c: u8, n: &mut usize| {
        if latin1 && c >= 0x80 {
            if *n + 2 <= VALUE {
                out[*n] = 0xC0 | c >> 6;
                out[*n + 1] = 0x80 | (c & 0x3F);
                *n += 2;
            }
        } else if *n < VALUE {
            out[*n] = c;
            *n += 1;
        }
    };
    if v.qp {
        while let Some(c) = v.next() {
            if c != b'=' {
                if c != b'\r' && c != b'\n' {
                    put(c, &mut n);
                }
                continue;
            }
            let mut ahead = v.clone();
            match (ahead.next(), ahead.next()) {
                (Some(a), Some(b)) if hex(a).is_some() && hex(b).is_some() => {
                    put(hex(a).unwrap_or(0) << 4 | hex(b).unwrap_or(0), &mut n);
                    v = ahead;
                }
                // A soft break: "=" then the line's end.
                (Some(b'\r' | b'\n'), _) => {
                    let mut skip = v.clone();
                    if skip.next() == Some(b'\r') {
                        let _ = skip.next();
                    }
                    v = skip;
                }
                _ => put(b'=', &mut n),
            }
        }
    } else {
        for c in v {
            put(c, &mut n);
        }
    }
    n
}

/// A structured value's `k`th component (`;` apart, `\;` kept), escapes
/// undone (`\n` a line break, `\,` `\;` `\\`), into `out`.
fn component<const N: usize>(v: &[u8], k: usize, out: &mut Short<N>) {
    let mut at = 0;
    let mut esc = false;
    for &c in v {
        if esc {
            if at == k {
                out.push(&[if c == b'n' || c == b'N' { b'\n' } else { c }]);
            }
            esc = false;
        } else if c == b'\\' {
            esc = true;
        } else if c == b';' {
            at += 1;
        } else if at == k && c != b'\r' {
            out.push(&[c]);
        }
    }
}

/// A plain value's escapes undone (a `;` in it is text).
fn plain<const N: usize>(v: &[u8], out: &mut Short<N>) {
    let mut esc = false;
    for &c in v {
        if esc {
            out.push(&[if c == b'n' || c == b'N' { b'\n' } else { c }]);
            esc = false;
        } else if c == b'\\' {
            esc = true;
        } else if c != b'\r' {
            // (A quoted-printable line break is CR LF: one break kept.)
            out.push(&[c]);
        }
    }
}

/// A date as vCards write it: "1815-12-10", "18151210", "--1210",
/// "--12-10", a date-time ("1815-12-10T00:00:00Z"); Apple's year 1604
/// (with `X-APPLE-OMIT-YEAR`) is no year.
fn day(v: &[u8], omit: Option<&[u8]>) -> Option<Day> {
    let v = &v[..v.iter().position(|&c| c == b'T').unwrap_or(v.len())];
    let digits: Short<8> = {
        let mut d = Short::new();
        for &c in v {
            if c.is_ascii_digit() {
                d.push(&[c]);
            } else if c != b'-' {
                return None;
            }
        }
        d
    };
    let d = digits.get();
    let num = |a: &[u8]| a.iter().fold(0i32, |n, &c| n * 10 + i32::from(c - b'0'));
    let (year, md) = if v.starts_with(b"--") && d.len() == 4 {
        (None, d)
    } else if d.len() == 8 {
        (Some(num(&d[..4])), &d[4..])
    } else {
        return None;
    };
    // Apple's "no year" year, and a year 0000 some phones write.
    let omit = omit.and_then(|o| {
        o.iter()
            .try_fold(0i32, |n, &c| c.is_ascii_digit().then(|| n * 10 + i32::from(c - b'0')))
    });
    let year = year.filter(|&y| Some(y) != omit && y >= 1000);
    let out = Day {
        year,
        month: num(&md[..2]) as u8,
        day: num(&md[2..]) as u8,
    };
    out.field().map(|_| out)
}

/// An Apple label (`_$!<HomePage>!$_`) as words.
fn apple_label(v: &[u8]) -> &[u8] {
    v.strip_prefix(b"_$!<")
        .and_then(|v| v.strip_suffix(b">!$_"))
        .unwrap_or(v)
}

/// A phone's label from its TYPEs: one of the three kept on their own, or
/// a word for an extra one.
fn phone_label(h: &Head) -> &'static [u8] {
    let fax = h.has_type(b"FAX");
    if h.has_type(b"CELL") || h.has_type(b"MOBILE") || h.has_type(b"IPHONE") {
        b"MOBILE"
    } else if h.has_type(b"PAGER") {
        b"Pager"
    } else if fax && h.has_type(b"HOME") {
        b"Home fax"
    } else if fax && h.has_type(b"WORK") {
        b"Work fax"
    } else if fax {
        b"Fax"
    } else if h.has_type(b"HOME") {
        b"HOME"
    } else if h.has_type(b"WORK") {
        b"WORK"
    } else if h.has_type(b"MAIN") {
        b"Main"
    } else {
        b"Other"
    }
}

/// Every card of `text`, read into `card` and handed to `take` in turn
/// (the caller's card: a reader on the stack of a program is kept small)
/// until it answers false. The cards it handed over.
pub fn each(text: &[u8], card: &mut Card, mut take: impl FnMut(&Card) -> bool) -> usize {
    let mut inside = false;
    let mut cards = 0;
    let mut fname: Short<48> = Short::new();
    let mut have_n = false;
    let mut have_adr = false;
    let mut abdate: Option<(Day, Short<8>)> = None;
    let mut buf = [0u8; VALUE];
    for (start, line) in (Lines { text, at: 0 }) {
        let Some((h, value)) = split_line(line) else {
            continue;
        };
        let name = h.name();
        if name == b"BEGIN" && contains_nocase(line, b"VCARD") {
            *card = Card::new();
            inside = true;
            fname.clear();
            have_n = false;
            have_adr = false;
            abdate = None;
            continue;
        }
        if !inside {
            continue;
        }
        if name == b"END" {
            inside = false;
            if abdate.take().is_some() {
                card.skipped += 1;
            }
            if !have_n && !fname.is_empty() {
                // FN alone: the last word the family name.
                let f = fname.get();
                match f.iter().rposition(|&c| c == b' ') {
                    Some(sp) => {
                        card.first.push(&f[..sp]);
                        card.last.push(&f[sp + 1..]);
                    }
                    None => card.first.push(f),
                }
            }
            cards += 1;
            if !take(card) {
                return cards;
            }
            continue;
        }
        if name == b"PHOTO" || name == b"LOGO" {
            if name == b"LOGO" {
                continue;
            }
            // Located, not copied: the value's raw bytes (folds and all).
            let raw = &line[value.at.min(line.len())..];
            let is_uri = h.param(b"VALUE").is_some_and(|v| v == b"URI" || v == b"URL")
                || (raw.len() > 5 && raw[..5].eq_ignore_ascii_case(b"http:"))
                || (raw.len() > 6 && raw[..6].eq_ignore_ascii_case(b"https:"));
            if is_uri || card.photo.is_some() {
                card.skipped += 1;
            } else {
                let s = start + value.at.min(line.len());
                card.photo = Some(Photo {
                    start: s,
                    end: start + line.len(),
                });
            }
            continue;
        }
        let n = decode(&h, value, &mut buf);
        let v = &buf[..n];
        match name {
            b"N" => {
                have_n = true;
                component(v, 0, &mut card.last);
                component(v, 1, &mut card.first);
            }
            b"FN" => plain(v, &mut fname),
            b"ORG" => component(v, 0, &mut card.company),
            b"TITLE" => plain(v, &mut card.job),
            b"TEL" => {
                let num = v.strip_prefix(b"tel:").unwrap_or(v);
                let mut number: Short<64> = Short::new();
                plain(num, &mut number);
                if card.add_phone(phone_label(&h), number.get()).is_some() {
                    card.phones[card.n_phones - 1].group.push(h.group());
                } else {
                    card.skipped += 1;
                }
            }
            b"EMAIL" => {
                let mut addr: Short<64> = Short::new();
                plain(v.strip_prefix(b"mailto:").unwrap_or(v), &mut addr);
                let label: &[u8] = if h.has_type(b"WORK") {
                    b"Work"
                } else if h.has_type(b"HOME") {
                    b"Home"
                } else {
                    b"Other"
                };
                if card.add_email(label, addr.get()).is_some() {
                    card.emails[card.n_emails - 1].group.push(h.group());
                } else {
                    card.skipped += 1;
                }
            }
            b"ADR" if !have_adr => {
                have_adr = true;
                // PO box; extended; street; locality; region; code; country.
                let mut street: Short<48> = Short::new();
                let mut ext: Short<48> = Short::new();
                let mut po: Short<48> = Short::new();
                component(v, 2, &mut street);
                component(v, 1, &mut ext);
                component(v, 0, &mut po);
                for part in [street.get(), ext.get(), po.get()] {
                    if part.is_empty() {
                        continue;
                    }
                    if !card.address[0].is_empty() {
                        card.address[0].push(b", ");
                    }
                    // A street of two lines kept on one.
                    for &c in part {
                        card.address[0].push(&[if c == b'\n' { b' ' } else { c }]);
                    }
                }
                for (k, part) in card.address.iter_mut().enumerate().skip(1) {
                    component(v, k + 2, part);
                }
            }
            b"ADR" | b"NICKNAME" | b"IMPP" | b"X-SOCIALPROFILE" | b"RELATED" | b"GEO" => {
                card.skipped += 1;
            }
            b"BDAY" => card.birthday = day(v, h.param(b"X-APPLE-OMIT-YEAR")),
            b"ANNIVERSARY" | b"X-ANNIVERSARY" | b"X-MS-ANNIVERSARY" => {
                card.anniversary = day(v, h.param(b"X-APPLE-OMIT-YEAR"));
            }
            // Apple's other dates: "item2.X-ABDATE", its label on a later
            // line says which.
            b"X-ABDATE" => {
                if abdate.is_some() {
                    card.skipped += 1;
                }
                let mut g = Short::new();
                g.push(h.group());
                abdate = day(v, h.param(b"X-APPLE-OMIT-YEAR")).map(|d| (d, g));
            }
            b"URL" if card.url.is_empty() => plain(v, &mut card.url),
            b"NOTE" => plain(v, &mut card.note),
            b"X-ABLABEL" => {
                let mut label: Short<24> = Short::new();
                plain(apple_label(v), &mut label);
                let g = h.group();
                if let Some((d, dg)) = abdate
                    && !g.is_empty()
                    && dg.get().eq_ignore_ascii_case(g)
                {
                    if contains_nocase(label.get(), b"ANNIVERSARY") && card.anniversary.is_none() {
                        card.anniversary = Some(d);
                    } else {
                        card.skipped += 1;
                    }
                    abdate = None;
                }
                for p in card.phones[..card.n_phones]
                    .iter_mut()
                    .chain(card.emails[..card.n_emails].iter_mut())
                    .filter(|p| !g.is_empty() && p.group.get().eq_ignore_ascii_case(g))
                {
                    p.label = label;
                }
            }
            _ => {}
        }
    }
    cards
}

/// A located picture's bytes: its base64 decoded into `out` (folds,
/// spaces and a `data:...;base64,` head skipped). The length, or `None`
/// when it isn't base64 or doesn't fit.
#[must_use]
pub fn photo_bytes(text: &[u8], p: Photo, out: &mut [u8]) -> Option<usize> {
    let raw = text.get(p.start..p.end)?;
    let raw = match raw.iter().position(|&c| c == b',') {
        Some(k) if raw.len() > 5 && raw[..5].eq_ignore_ascii_case(b"data:") => &raw[k + 1..],
        _ => raw,
    };
    let mut n = 0;
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in raw {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        acc = acc << 6 | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            *out.get_mut(n)? = (acc >> bits) as u8;
            n += 1;
        }
    }
    (n > 0).then_some(n)
}

// ---- Writing ----------------------------------------------------------

/// Which vCard to write.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Version {
    /// 3.0 (RFC 2426): what every importer reads (Outlook, iOS, Google).
    V3,
    /// 4.0 (RFC 6350).
    V4,
}

/// Lines folded at 75 bytes (never inside a UTF-8 character), each ended
/// by CRLF.
struct Folder<'f, F: FnMut(&[u8])> {
    out: &'f mut F,
    col: usize,
}

impl<F: FnMut(&[u8])> Folder<'_, F> {
    fn put(&mut self, s: &[u8]) {
        for (k, &c) in s.iter().enumerate() {
            // A continuation byte stays with its character.
            let starts_char = c & 0xC0 != 0x80;
            if self.col >= 74 && starts_char && k < s.len() {
                (self.out)(b"\r\n ");
                self.col = 1;
            }
            (self.out)(&[c]);
            self.col += 1;
        }
    }
    /// A value's text with `\`, `,`, `;` and line breaks escaped.
    fn escaped(&mut self, s: &[u8]) {
        for &c in s {
            match c {
                b'\\' => self.put(b"\\\\"),
                b',' => self.put(b"\\,"),
                b';' => self.put(b"\\;"),
                b'\n' => self.put(b"\\n"),
                b'\r' => {}
                _ => self.put(&[c]),
            }
        }
    }
    fn end(&mut self) {
        (self.out)(b"\r\n");
        self.col = 0;
    }
}

/// A day as vCard writes it: "1815-12-10", or with no year "--1210" (4.0)
/// or Apple's "1604-12-10" with `X-APPLE-OMIT-YEAR` (3.0, which has no
/// form for it).
fn put_day<F: FnMut(&[u8])>(f: &mut Folder<'_, F>, name: &[u8], d: Day, v: Version) {
    let two = |n: u8| [b'0' + n / 10, b'0' + n % 10];
    f.put(name);
    match (d.year, v) {
        (Some(y), _) => {
            f.put(b":");
            let y = y.clamp(0, 9999) as u16;
            f.put(&[
                b'0' + (y / 1000) as u8,
                b'0' + (y / 100 % 10) as u8,
                b'0' + (y / 10 % 10) as u8,
                b'0' + (y % 10) as u8,
                b'-',
            ]);
            f.put(&two(d.month));
            f.put(b"-");
            f.put(&two(d.day));
        }
        (None, Version::V4) => {
            f.put(b":--");
            f.put(&two(d.month));
            f.put(&two(d.day));
        }
        (None, Version::V3) => {
            f.put(b";X-APPLE-OMIT-YEAR=1604:1604-");
            f.put(&two(d.month));
            f.put(b"-");
            f.put(&two(d.day));
        }
    }
    f.end();
}

/// The TYPE a label is written as, or `None` for a label of its own
/// (written as an `X-ABLabel` group).
fn label_type(label: &[u8], phone: bool) -> Option<&'static [u8]> {
    let l = label;
    let is = |w: &[u8]| l.eq_ignore_ascii_case(w);
    Some(if is(b"MOBILE") || is(b"cell") {
        b"cell"
    } else if is(b"HOME") {
        if phone { b"home,voice" } else { b"home" }
    } else if is(b"WORK") {
        if phone { b"work,voice" } else { b"work" }
    } else if is(b"Fax") {
        b"fax"
    } else if is(b"Home fax") {
        b"home,fax"
    } else if is(b"Work fax") {
        b"work,fax"
    } else if is(b"Pager") {
        b"pager"
    } else if is(b"Main") {
        if phone { b"main" } else { return None }
    } else if is(b"Other") || l.is_empty() {
        if phone { b"voice" } else { b"internet" }
    } else {
        return None;
    })
}

/// `card` written as a vCard of `v`, a piece at a time to `out`; its
/// picture, when given, as an inline PNG or JPEG (`mime`'s subtype).
pub fn write(card: &Card, photo: Option<(&[u8], &[u8])>, v: Version, mut out: impl FnMut(&[u8])) {
    let mut f = Folder {
        out: &mut out,
        col: 0,
    };
    f.put(b"BEGIN:VCARD");
    f.end();
    f.put(match v {
        Version::V3 => b"VERSION:3.0",
        Version::V4 => b"VERSION:4.0",
    });
    f.end();
    f.put(b"N:");
    f.escaped(card.last.get());
    f.put(b";");
    f.escaped(card.first.get());
    f.put(b";;;");
    f.end();
    // FN is required: the name, or the company for a company's card.
    f.put(b"FN:");
    if card.first.is_empty() && card.last.is_empty() {
        f.escaped(card.company.get());
    } else {
        f.escaped(card.first.get());
        if !card.first.is_empty() && !card.last.is_empty() {
            f.put(b" ");
        }
        f.escaped(card.last.get());
    }
    f.end();
    if !card.company.is_empty() {
        f.put(b"ORG:");
        f.escaped(card.company.get());
        f.end();
    }
    if !card.job.is_empty() {
        f.put(b"TITLE:");
        f.escaped(card.job.get());
        f.end();
    }
    let mut item = 0u8;
    let mut labelled = |f: &mut Folder<'_, _>, name: &[u8], l: &Labelled, phone: bool| {
        let ty = label_type(l.label.get(), phone);
        let group = ty.is_none().then(|| {
            item += 1;
            [b'i', b't', b'e', b'm', b'0' + item / 10, b'0' + item % 10]
        });
        if let Some(g) = group {
            f.put(&g);
            f.put(b".");
        }
        f.put(name);
        if let Some(t) = ty {
            f.put(b";TYPE=");
            if v == Version::V4 && t.contains(&b',') {
                f.put(b"\"");
                f.put(t);
                f.put(b"\"");
            } else {
                f.put(t);
            }
        }
        if v == Version::V4 && phone {
            f.put(b";VALUE=uri:tel:");
            f.put(l.value.get());
        } else {
            f.put(b":");
            f.escaped(l.value.get());
        }
        f.end();
        if let Some(g) = group {
            f.put(&g);
            f.put(b".X-ABLabel:");
            f.escaped(l.label.get());
            f.end();
        }
    };
    for p in &card.phones[..card.n_phones.min(PHONES)] {
        labelled(&mut f, b"TEL", p, true);
    }
    for e in &card.emails[..card.n_emails.min(EMAILS)] {
        labelled(&mut f, b"EMAIL", e, false);
    }
    if !card.address().is_empty() {
        f.put(b"ADR;TYPE=home:;;");
        for (k, part) in card.address.iter().enumerate() {
            if k > 0 {
                f.put(b";");
            }
            f.escaped(part.get());
        }
        f.end();
    }
    if let Some(d) = card.birthday {
        put_day(&mut f, b"BDAY", d, v);
    }
    if let Some(d) = card.anniversary {
        put_day(
            &mut f,
            match v {
                Version::V3 => b"X-ANNIVERSARY",
                Version::V4 => b"ANNIVERSARY",
            },
            d,
            v,
        );
    }
    if !card.url.is_empty() {
        f.put(b"URL:");
        f.put(card.url.get());
        f.end();
    }
    if !card.note.is_empty() {
        f.put(b"NOTE:");
        f.escaped(card.note.get());
        f.end();
    }
    if let Some((bytes, mime)) = photo {
        match v {
            Version::V3 => {
                f.put(b"PHOTO;ENCODING=b;TYPE=");
                f.put(mime);
                f.put(b":");
            }
            Version::V4 => {
                f.put(b"PHOTO:data:image/");
                f.put(mime);
                f.put(b";base64,");
            }
        }
        base64(&mut f, bytes);
        f.end();
    }
    f.put(b"END:VCARD");
    f.end();
}

fn base64<F: FnMut(&[u8])>(f: &mut Folder<'_, F>, b: &[u8]) {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for c in b.chunks(3) {
        let n = u32::from(c[0]) << 16
            | u32::from(*c.get(1).unwrap_or(&0)) << 8
            | u32::from(*c.get(2).unwrap_or(&0));
        let q = [
            A[(n >> 18) as usize & 63],
            A[(n >> 12) as usize & 63],
            if c.len() > 1 { A[(n >> 6) as usize & 63] } else { b'=' },
            if c.len() > 2 { A[n as usize & 63] } else { b'=' },
        ];
        f.put(&q);
    }
}

// ---- Cards and the store's records ------------------------------------

/// The packed extra entries' room (every phone and e-mail a card holds).
const MORE_BYTES: usize = (PHONES + EMAILS) * (3 + crate::contact::LABEL_MAX + 64);

/// The store's fields a contact has, in the order [`Scratch::fields`]
/// lists them (an empty one is taken off a record that had it).
pub const FIELDS: usize = 14;

/// Where a card's fields are put together for the store: the numbers as
/// E.164, the address's five lines, the extra entries packed, the days.
pub struct Scratch {
    nums: [crate::phone::Formatted; 3],
    addr: [u8; 5 * 48 + 4],
    addr_n: usize,
    more: [u8; MORE_BYTES],
    more_n: usize,
    days: [[u8; 4]; 2],
    has_day: [bool; 2],
    have: [bool; 4],
}

impl Default for Scratch {
    fn default() -> Self {
        Self::new()
    }
}

impl Scratch {
    /// Empty.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            nums: [crate::phone::Formatted {
                b: [0; crate::phone::MAX],
                n: 0,
            }; 3],
            addr: [0; 5 * 48 + 4],
            addr_n: 0,
            more: [0; MORE_BYTES],
            more_n: 0,
            days: [[0; 4]; 2],
            has_day: [false; 2],
            have: [false; 4],
        }
    }

    /// `card` made ready for the store, its numbers read in `home`'s plan
    /// (a number without a country code is the home country's).
    pub fn fill(&mut self, card: &Card, home: &crate::phone::Plan) {
        use crate::contact::{Entry, Kind, push_entry};
        self.have = [false; 4];
        self.more_n = 0;
        for p in &card.phones[..card.n_phones.min(PHONES)] {
            let slot = [&b"MOBILE"[..], b"HOME", b"WORK"]
                .iter()
                .position(|l| p.label.get() == *l);
            let e164 = crate::phone::e164(p.value.get(), home);
            match slot {
                Some(k) if !self.have[k] => {
                    self.have[k] = true;
                    self.nums[k] = e164;
                }
                _ => {
                    let label: &[u8] = match p.label.get() {
                        b"MOBILE" => b"Mobile",
                        b"HOME" => b"Home",
                        b"WORK" => b"Work",
                        l => l,
                    };
                    if let Some(n) = push_entry(
                        &mut self.more,
                        self.more_n,
                        Entry {
                            kind: Kind::Phone,
                            label,
                            value: e164.get(),
                        },
                    ) {
                        self.more_n = n;
                    }
                }
            }
        }
        for (k, e) in card.emails[..card.n_emails.min(EMAILS)].iter().enumerate() {
            if k == 0 {
                self.have[3] = true;
                continue;
            }
            if let Some(n) = push_entry(
                &mut self.more,
                self.more_n,
                Entry {
                    kind: Kind::Email,
                    label: e.label.get(),
                    value: e.value.get(),
                },
            ) {
                self.more_n = n;
            }
        }
        self.addr_n = card.address().field(&mut self.addr).unwrap_or(0);
        for (k, d) in [card.birthday, card.anniversary].into_iter().enumerate() {
            match d.and_then(Day::field) {
                Some(f) => {
                    self.days[k] = f;
                    self.has_day[k] = true;
                }
                None => self.has_day[k] = false,
            }
        }
    }

    /// The store's fields for `card` (after [`Scratch::fill`] with the same
    /// card): every contact field, empty where the card has none.
    #[must_use]
    pub fn fields<'a>(&'a self, card: &'a Card) -> [(u8, &'a [u8]); FIELDS] {
        use crate::store::tag;
        let num = |k: usize| {
            if self.have[k] {
                self.nums[k].get()
            } else {
                &[][..]
            }
        };
        let day = |k: usize| {
            if self.has_day[k] {
                &self.days[k][..]
            } else {
                &[][..]
            }
        };
        [
            (tag::FIRST, card.first.get()),
            (tag::LAST, card.last.get()),
            (tag::COMPANY, card.company.get()),
            (tag::JOB, card.job.get()),
            (tag::MOBILE, num(0)),
            (tag::HOME, num(1)),
            (tag::WORK, num(2)),
            (
                tag::EMAIL,
                if self.have[3] {
                    card.emails[0].value.get()
                } else {
                    &[]
                },
            ),
            (tag::MORE, &self.more[..self.more_n]),
            (tag::ADDRESS, &self.addr[..self.addr_n]),
            (tag::BIRTHDAY, day(0)),
            (tag::ANNIVERSARY, day(1)),
            (tag::WEBSITE, card.url.get()),
            (tag::NOTE, card.note.get()),
        ]
    }
}

/// A kept contact as a card (to write as a vCard, or to merge).
#[must_use]
pub fn card_of(r: &crate::store::Rec<'_>) -> Card {
    use crate::contact::{Kind, entries};
    use crate::store::tag;
    let mut c = Card::new();
    c.first.push(r.text(tag::FIRST));
    c.last.push(r.text(tag::LAST));
    c.company.push(r.text(tag::COMPANY));
    c.job.push(r.text(tag::JOB));
    for (t, label) in [
        (tag::MOBILE, &b"MOBILE"[..]),
        (tag::HOME, b"HOME"),
        (tag::WORK, b"WORK"),
    ] {
        if !r.text(t).is_empty() {
            let _ = c.add_phone(label, r.text(t));
        }
    }
    if !r.text(tag::EMAIL).is_empty() {
        let _ = c.add_email(b"Other", r.text(tag::EMAIL));
    }
    for e in entries(r.text(tag::MORE)) {
        let added = match e.kind {
            Kind::Phone => c.add_phone(e.label, e.value),
            Kind::Email => c.add_email(e.label, e.value),
        };
        if added.is_none() {
            c.skipped += 1;
        }
    }
    let a = Address::of(r.text(tag::ADDRESS));
    for (k, p) in a.parts().iter().enumerate() {
        c.address[k].push(p);
    }
    c.birthday = Day::of_field(r.text(tag::BIRTHDAY));
    c.anniversary = Day::of_field(r.text(tag::ANNIVERSARY));
    c.url.push(r.text(tag::WEBSITE));
    c.note.push(r.text(tag::NOTE));
    c
}

/// Whether two cards are one person: a phone number or an e-mail in
/// common (contacts-rule0.md "Duplicates": a shared name alone is not,
/// two John Smiths are two people).
#[must_use]
pub fn same_person(a: &Card, b: &Card) -> bool {
    use crate::contact::{phone_key, same_email};
    let phones = |c: &Card| -> [Option<[u8; 10]>; PHONES] {
        let mut k = [None; PHONES];
        for (o, p) in k.iter_mut().zip(&c.phones[..c.n_phones.min(PHONES)]) {
            *o = phone_key(p.value.get());
        }
        k
    };
    let (pa, pb) = (phones(a), phones(b));
    pa.iter()
        .flatten()
        .any(|x| pb.iter().flatten().any(|y| x == y))
        || a.emails[..a.n_emails.min(EMAILS)].iter().any(|x| {
            b.emails[..b.n_emails.min(EMAILS)]
                .iter()
                .any(|y| same_email(x.value.get(), y.value.get()))
        })
}

/// `other` merged into `keep`: every field `keep` lacks taken from
/// `other`, its phones and e-mails that `keep` doesn't have added, both
/// notes kept. Nothing either card holds is lost, unless a list is full
/// or the two addresses differ (counted in [`Card::skipped`]).
pub fn merge(keep: &mut Card, other: &Card) {
    use crate::contact::{phone_key, same_email};
    macro_rules! take {
        ($f:ident) => {
            if keep.$f.is_empty() {
                keep.$f = other.$f;
            }
        };
    }
    take!(first);
    take!(last);
    take!(company);
    take!(job);
    take!(url);
    if keep.address().is_empty() {
        keep.address = other.address;
    } else if !other.address().is_empty() && keep.address != other.address {
        keep.skipped += 1;
    }
    keep.birthday = keep.birthday.or(other.birthday);
    keep.anniversary = keep.anniversary.or(other.anniversary);
    if keep.photo.is_none() {
        keep.photo = other.photo;
    }
    for p in &other.phones[..other.n_phones.min(PHONES)] {
        let key = phone_key(p.value.get());
        let had = keep.phones[..keep.n_phones].iter().any(|q| {
            q.value.get() == p.value.get() || (key.is_some() && phone_key(q.value.get()) == key)
        });
        if !had && keep.add_phone(p.label.get(), p.value.get()).is_none() {
            keep.skipped += 1;
        }
    }
    for e in &other.emails[..other.n_emails.min(EMAILS)] {
        let had = keep.emails[..keep.n_emails]
            .iter()
            .any(|q| same_email(q.value.get(), e.value.get()));
        if !had && keep.add_email(e.label.get(), e.value.get()).is_none() {
            keep.skipped += 1;
        }
    }
    let (kn, on) = (keep.note, other.note);
    if !on.is_empty() && kn.get() != on.get() {
        if !kn.is_empty() {
            keep.note.push(b"\n");
        }
        keep.note.push(on.get());
    }
}

#[cfg(test)]
#[path = "vcard_tests.rs"]
mod tests;
