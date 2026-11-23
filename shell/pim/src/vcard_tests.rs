//! The vCard reader and writer's tests: files as iOS, Google, Outlook
//! (2.1) and a 4.0 writer make them, round trips both ways, and hostile
//! input (cut at every byte, junk, a line of a megabyte) that must never
//! panic.

extern crate std;
use super::*;
use std::vec::Vec;

fn all(text: &[u8]) -> Vec<Card> {
    let mut card = Card::new();
    let mut out = Vec::new();
    each(text, &mut card, |c| {
        out.push(*c);
        true
    });
    out
}

const IOS: &[u8] = b"BEGIN:VCARD\r\nVERSION:3.0\r\nPRODID:-//Apple Inc.//iPhone OS 17.0//EN\r\nN:Lovelace;Ada;;;\r\nFN:Ada Lovelace\r\nORG:Analytical Engines;\r\nTITLE:Programmer\r\nitem1.TEL;type=pref:+1 (206) 555-0142\r\nitem1.X-ABLabel:Cabin\r\nTEL;type=CELL;type=VOICE:(206) 555-0100\r\nTEL;type=HOME;type=FAX:206 555 0111\r\nitem2.EMAIL;type=INTERNET;type=pref:ada@example.com\r\nEMAIL;type=INTERNET;type=WORK:ada@engines.example\r\nitem3.ADR;type=HOME;type=pref:;;12 St James Sq;Seattle;WA;98101;United States\r\nitem3.X-ABADR:us\r\nBDAY;X-APPLE-OMIT-YEAR=1604:1604-12-10\r\nitem4.X-ABDATE;type=pref:1835-07-08\r\nitem4.X-ABLabel:_$!<Anniversary>!$_\r\nitem5.URL;type=pref:https://example.com/ada\r\nitem5.X-ABLabel:_$!<HomePage>!$_\r\nNOTE:Met at the\\, Royal Society\\nbring notes\r\nPHOTO;ENCODING=b;TYPE=JPEG:/9j/4AAQSkZJRgABAQ\r\n AAAQABAAD/2wBDAP\r\nEND:VCARD\r\n";

#[test]
fn an_iphone_card_with_labels_groups_and_a_picture() {
    let c = &all(IOS)[0];
    assert_eq!(c.first.get(), b"Ada");
    assert_eq!(c.last.get(), b"Lovelace");
    assert_eq!(c.company.get(), b"Analytical Engines");
    assert_eq!(c.job.get(), b"Programmer");
    assert_eq!(c.n_phones, 3);
    assert_eq!(c.phones[0].label.get(), b"Cabin");
    assert_eq!(c.phones[0].value.get(), b"+1 (206) 555-0142");
    assert_eq!(c.phones[1].label.get(), b"MOBILE");
    assert_eq!(c.phones[2].label.get(), b"Home fax");
    assert_eq!(c.n_emails, 2);
    assert_eq!(c.emails[0].value.get(), b"ada@example.com");
    assert_eq!(c.emails[1].label.get(), b"Work");
    assert_eq!(c.address[0].get(), b"12 St James Sq");
    assert_eq!(c.address[1].get(), b"Seattle");
    assert_eq!(c.address[2].get(), b"WA");
    assert_eq!(c.address[3].get(), b"98101");
    assert_eq!(c.address[4].get(), b"United States");
    // Apple's 1604 is no year.
    assert_eq!(
        c.birthday,
        Some(Day {
            year: None,
            month: 12,
            day: 10
        })
    );
    assert_eq!(
        c.anniversary,
        Some(Day {
            year: Some(1835),
            month: 7,
            day: 8
        })
    );
    assert_eq!(c.url.get(), b"https://example.com/ada");
    assert_eq!(c.note.get(), b"Met at the, Royal Society\nbring notes");
    let p = c.photo.expect("an inline picture");
    let mut px = [0u8; 64];
    let n = photo_bytes(IOS, p, &mut px).unwrap();
    // The JPEG's start-of-image mark, across the fold.
    assert_eq!(&px[..4], &[0xFF, 0xD8, 0xFF, 0xE0]);
    assert_eq!(n, 25);
    assert_eq!(c.skipped, 0);
}

const OUTLOOK_21: &[u8] = b"BEGIN:VCARD\r\nVERSION:2.1\r\nN;CHARSET=ISO-8859-1:M\xfcller;J\xfcrgen\r\nFN:J\xfcrgen M\xfcller\r\nTEL;CELL;VOICE:+49 30 1234567\r\nTEL;WORK;VOICE:030 7654321\r\nEMAIL;PREF;INTERNET:jm@example.de\r\nADR;WORK;ENCODING=QUOTED-PRINTABLE:;;Hauptstra=C3=9Fe 1=0D=0A=\r\nHinterhaus;Berlin;;10115;Deutschland\r\nNOTE;ENCODING=QUOTED-PRINTABLE:Erste Zeile=0D=0Azweite\r\nX-MS-ANNIVERSARY:20010915\r\nBDAY:19700102\r\nEND:VCARD\r\n";

#[test]
fn an_outlook_21_card_with_quoted_printable_and_latin1() {
    let c = &all(OUTLOOK_21)[0];
    assert_eq!(c.first.get(), "J\u{fc}rgen".as_bytes());
    assert_eq!(c.last.get(), "M\u{fc}ller".as_bytes());
    assert_eq!(c.phones[0].label.get(), b"MOBILE");
    assert_eq!(c.phones[1].label.get(), b"WORK");
    assert_eq!(c.emails[0].value.get(), b"jm@example.de");
    // The soft break joined; the street's two lines kept on one.
    assert_eq!(
        c.address[0].get(),
        "Hauptstra\u{df}e 1 Hinterhaus".as_bytes()
    );
    assert_eq!(c.address[1].get(), b"Berlin");
    assert_eq!(c.address[3].get(), b"10115");
    assert_eq!(c.note.get(), b"Erste Zeile\nzweite");
    assert_eq!(
        c.anniversary,
        Some(Day {
            year: Some(2001),
            month: 9,
            day: 15
        })
    );
    assert_eq!(c.birthday.map(|d| d.year), Some(Some(1970)));
}

const V4: &[u8] = b"BEGIN:VCARD\nVERSION:4.0\nFN:Grace Hopper\nTEL;VALUE=uri;TYPE=\"cell,voice\":tel:+1-703-555-0107\nEMAIL;TYPE=work:grace@example.org\nBDAY:--1209\nANNIVERSARY:19300615\nPHOTO:data:image/jpeg;base64,/9j/4AAQ\nNICKNAME:Amazing Grace\nPHOTO;VALUE=uri:https://example.org/g.jpg\nEND:VCARD\nBEGIN:VCARD\nVERSION:4.0\nORG:Acme Corp\nEND:VCARD\n";

#[test]
fn a_40_file_with_uris_dates_without_years_and_two_cards() {
    let cards = all(V4);
    assert_eq!(cards.len(), 2);
    let c = &cards[0];
    // FN alone: the last word is the family name.
    assert_eq!(c.first.get(), b"Grace");
    assert_eq!(c.last.get(), b"Hopper");
    assert_eq!(c.phones[0].value.get(), b"+1-703-555-0107");
    assert_eq!(c.phones[0].label.get(), b"MOBILE");
    assert_eq!(c.emails[0].label.get(), b"Work");
    assert_eq!(
        c.birthday,
        Some(Day {
            year: None,
            month: 12,
            day: 9
        })
    );
    assert_eq!(c.anniversary.and_then(|d| d.year), Some(1930));
    let mut px = [0u8; 8];
    assert_eq!(photo_bytes(V4, c.photo.unwrap(), &mut px), Some(6));
    // The nickname and the linked picture: counted, not kept.
    assert_eq!(c.skipped, 2);
    assert_eq!(cards[1].company.get(), b"Acme Corp");
    assert!(!cards[1].is_empty());
}

fn sample() -> Card {
    let mut c = Card::new();
    c.first.push("Zo\u{eb}".as_bytes());
    c.last.push(b"O'Brien; Jr");
    c.company.push(b"Acme, Inc.");
    c.job.push(b"Chief \\ Engineer");
    c.add_phone(b"MOBILE", b"+12065550142").unwrap();
    c.add_phone(b"HOME", b"+12065550100").unwrap();
    c.add_phone(b"Cabin", b"+13605550199").unwrap();
    c.add_phone(b"Fax", b"+12065550111").unwrap();
    c.add_email(b"Other", b"zoe@example.com").unwrap();
    c.add_email(b"School", b"zoe@school.example").unwrap();
    for (k, p) in [
        &b"1 Long Road, Flat 2"[..],
        b"Seattle",
        b"WA",
        b"98101",
        b"",
    ]
    .iter()
    .enumerate()
    {
        c.address[k].push(p);
    }
    c.birthday = Some(Day {
        year: None,
        month: 2,
        day: 29,
    });
    c.anniversary = Some(Day {
        year: Some(2010),
        month: 6,
        day: 1,
    });
    c.url.push(b"https://example.com/zoe");
    // A note longer than a folded line, with every escaped character.
    c.note.push(b"Line one\nline two; with, commas \\ and a long tail that runs well past the seventy-five byte fold");
    c
}

fn round_trip(v: Version) {
    let c = sample();
    let mut out = Vec::new();
    write(
        &c,
        Some((&[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3][..], &b"jpeg"[..])),
        v,
        |b| {
            out.extend_from_slice(b);
        },
    );
    // Folded at 75 bytes, every line ended by CRLF.
    for line in out.split(|&c| c == b'\n') {
        assert!(
            line.len() <= 76,
            "{}",
            std::string::String::from_utf8_lossy(line)
        );
    }
    let back = all(&out);
    assert_eq!(back.len(), 1);
    let b = &back[0];
    assert_eq!(b.first, c.first);
    assert_eq!(b.last, c.last);
    assert_eq!(b.company, c.company);
    assert_eq!(b.job, c.job);
    assert_eq!(b.n_phones, 4);
    for k in 0..4 {
        assert_eq!(b.phones[k].value, c.phones[k].value, "{v:?} phone {k}");
        assert!(
            b.phones[k]
                .label
                .get()
                .eq_ignore_ascii_case(c.phones[k].label.get()),
            "{v:?} label {k}"
        );
    }
    assert_eq!(b.emails[1].label.get(), b"School");
    assert_eq!(b.emails[0].value, c.emails[0].value);
    assert_eq!(b.address, c.address);
    assert_eq!(b.birthday, c.birthday);
    assert_eq!(b.anniversary, c.anniversary);
    assert_eq!(b.url, c.url);
    assert_eq!(b.note, c.note);
    let mut px = [0u8; 16];
    assert_eq!(photo_bytes(&out, b.photo.unwrap(), &mut px), Some(7));
    assert_eq!(&px[..7], &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3]);
    assert_eq!(b.skipped, 0);
}

#[test]
fn a_card_round_trips_through_30_and_40() {
    round_trip(Version::V3);
    round_trip(Version::V4);
}

#[test]
fn files_read_from_every_test_round_trip_back() {
    // Read, written, read again: the same card (what an export of an
    // import gives).
    for (file, v) in [
        (IOS, Version::V3),
        (OUTLOOK_21, Version::V4),
        (V4, Version::V3),
    ] {
        for c in all(file) {
            let mut out = Vec::new();
            write(&c, None, v, |b| out.extend_from_slice(b));
            let b = all(&out)[0];
            assert_eq!(b.first, c.first);
            assert_eq!(b.last, c.last);
            assert_eq!(b.n_phones, c.n_phones);
            assert_eq!(b.address, c.address);
            assert_eq!(b.birthday, c.birthday);
            assert_eq!(b.note, c.note);
        }
    }
}

/// A small fixed-seed generator: the same junk on every run.
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
    let mut card = Card::new();
    // Every file cut at every byte.
    for file in [IOS, OUTLOOK_21, V4] {
        for cut in 0..file.len() {
            each(&file[..cut], &mut card, |_| true);
            if let Some(p) = card.photo {
                let _ = photo_bytes(&file[..cut], p, &mut [0u8; 64]);
            }
        }
    }
    // Junk, and junk spliced into a good file.
    let mut r = Lcg(7);
    for _ in 0..300 {
        let mut f: Vec<u8> = IOS.to_vec();
        for _ in 0..20 {
            let at = usize::from(r.next()) * f.len() / 256;
            f[at] = r.next();
        }
        each(&f, &mut card, |_| true);
        let junk: Vec<u8> = (0..512).map(|_| r.next()).collect();
        each(&junk, &mut card, |_| true);
    }
    // A line of a megabyte, a value of nothing but escapes and QP marks, a
    // card never ended, thousands of phones.
    let mut big = b"BEGIN:VCARD\r\nNOTE:".to_vec();
    big.extend(std::iter::repeat_n(b'x', 1 << 20));
    big.extend_from_slice(b"\r\nN:\\\\\\;;;\\\r\nADR;ENCODING=QUOTED-PRINTABLE:=\r\n=Z\r\n");
    for _ in 0..5000 {
        big.extend_from_slice(b"TEL:1\r\n");
    }
    big.extend_from_slice(b"END:VCARD\r\nBEGIN:VCARD\r\nFN:x");
    let got = all(&big);
    assert_eq!(got.len(), 1);
    assert!(got[0].note.is_full());
    assert_eq!(got[0].n_phones, PHONES);
    assert_eq!(usize::from(got[0].skipped), 5000 - PHONES);
    // A picture that isn't base64, or bigger than its room: refused.
    assert_eq!(
        photo_bytes(b"@@@@", Photo { start: 0, end: 4 }, &mut [0; 8]),
        None
    );
    assert_eq!(
        photo_bytes(b"AAAAAAAA", Photo { start: 0, end: 8 }, &mut [0; 2]),
        None
    );
    assert_eq!(
        photo_bytes(b"AAAA", Photo { start: 2, end: 9 }, &mut [0; 8]),
        None
    );
}

#[test]
fn a_reader_stops_when_told() {
    let two = [V4, V4].concat();
    let mut card = Card::new();
    let mut seen = 0;
    let n = each(&two, &mut card, |_| {
        seen += 1;
        seen < 2
    });
    assert_eq!((n, seen), (2, 2));
}

fn store_mem() -> Vec<u8> {
    std::vec![0u8; 64 * 1024 + crate::store::INDEX_BYTES]
}

#[test]
fn a_card_kept_in_the_store_reads_back_as_the_same_card() {
    use crate::store::{Kind, Store, tag};
    let mut m = store_mem();
    let mut s = Store::sized(&mut m, 64);
    s.load_start(None);
    let home = crate::phone::plan(*b"US").unwrap();
    let c = sample();
    let mut sc = Scratch::new();
    sc.fill(&c, home);
    let id = s.put(Kind::Contact, None, &sc.fields(&c)).unwrap();
    let r = s.get(id).unwrap();
    // The first three numbers in their own fields as E.164, the rest packed.
    assert_eq!(r.text(tag::MOBILE), b"+12065550142");
    assert_eq!(r.text(tag::HOME), b"+12065550100");
    assert_eq!(r.text(tag::WORK), b"");
    let back = card_of(&r);
    assert_eq!(back.first, c.first);
    assert_eq!(back.n_phones, 4);
    assert_eq!(back.phones[2].label.get(), b"Cabin");
    assert_eq!(back.phones[3].label.get(), b"Fax");
    assert_eq!(back.n_emails, 2);
    assert_eq!(back.emails[1].label.get(), b"School");
    assert_eq!(back.address, c.address);
    assert_eq!(back.birthday, c.birthday);
    assert_eq!(back.anniversary, c.anniversary);
    assert_eq!(back.url, c.url);
    assert_eq!(back.note, c.note);
    // A number typed nationally becomes E.164 on import.
    let mut d = Card::new();
    d.first.push(b"Bo");
    d.add_phone(b"MOBILE", b"(206) 555-0199").unwrap();
    d.add_phone(b"MOBILE", b"+44 20 7946 0958").unwrap();
    sc.fill(&d, home);
    let id = s.put(Kind::Contact, None, &sc.fields(&d)).unwrap();
    let r = s.get(id).unwrap();
    assert_eq!(r.text(tag::MOBILE), b"+12065550199");
    let more = card_of(&r);
    assert_eq!(more.phones[1].label.get(), b"Mobile");
    assert_eq!(more.phones[1].value.get(), b"+442079460958");
}

#[test]
fn duplicates_are_found_by_a_number_or_a_mail_and_merged_whole() {
    let a = sample();
    let mut b = Card::new();
    b.first.push("Zo\u{eb}".as_bytes());
    b.add_phone(b"WORK", b"(206) 555-0142").unwrap();
    b.add_phone(b"WORK", b"+12065550177").unwrap();
    b.add_email(b"Work", b"ZOE@EXAMPLE.COM").unwrap();
    b.add_email(b"Work", b"z@work.example").unwrap();
    b.note.push(b"From the old phone");
    b.anniversary = Some(Day {
        year: Some(1999),
        month: 1,
        day: 1,
    });
    assert!(same_person(&a, &b));
    let mut stranger = Card::new();
    stranger.first.push("Zo\u{eb}".as_bytes());
    stranger.add_phone(b"MOBILE", b"+12065550000").unwrap();
    assert!(
        !same_person(&a, &stranger),
        "a name alone is not the same person"
    );
    let mut m = a;
    merge(&mut m, &b);
    // The shared number and mail once; the new ones added.
    assert_eq!(m.n_phones, 5);
    assert_eq!(m.phones[4].value.get(), b"+12065550177");
    assert_eq!(m.n_emails, 3);
    assert_eq!(m.emails[2].value.get(), b"z@work.example");
    // The kept card's own fields win; its gaps are filled; notes both.
    assert_eq!(m.anniversary, a.anniversary);
    assert!(m.note.get().ends_with(b"\nFrom the old phone"));
    assert_eq!(m.skipped, 0);
    // Merging the other way fills from the first.
    let mut n = b;
    merge(&mut n, &a);
    assert_eq!(n.last, a.last);
    assert_eq!(n.address, a.address);
    assert_eq!(n.birthday, a.birthday);
    // Full lists: counted, never silent.
    let mut full = Card::new();
    for k in 0..PHONES {
        full.add_phone(
            b"Other",
            &[
                b'1',
                b'0' + k as u8,
                b'5',
                b'5',
                b'5',
                b'0',
                b'1',
                b'0',
                b'0',
            ],
        )
        .unwrap();
    }
    merge(&mut full, &b);
    assert!(full.skipped >= 1);
}
