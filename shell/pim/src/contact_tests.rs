//! The contact fields' tests: days with and without years both ways,
//! leap days, addresses old and new, packed entries cut at every byte.

extern crate std;
use super::*;

#[test]
fn a_day_with_and_without_its_year_round_trips() {
    for d in [
        Day {
            year: Some(1815),
            month: 12,
            day: 10,
        },
        Day {
            year: None,
            month: 12,
            day: 10,
        },
        Day {
            year: None,
            month: 2,
            day: 29,
        },
        Day {
            year: Some(2000),
            month: 2,
            day: 29,
        },
        Day {
            year: Some(9999),
            month: 12,
            day: 31,
        },
        Day {
            year: Some(1000),
            month: 1,
            day: 1,
        },
    ] {
        let f = d.field().expect("a real day");
        assert_eq!(Day::of_field(&f), Some(d));
    }
}

#[test]
fn days_that_do_not_exist_are_refused() {
    for (year, month, day) in [
        (Some(2023), 2, 29),
        (None, 4, 31),
        (None, 13, 1),
        (None, 0, 1),
        (None, 1, 0),
        (Some(999), 1, 1),
        (Some(10_000), 1, 1),
        // A known year 4 would read back as unknown.
        (Some(4), 1, 1),
    ] {
        assert_eq!(
            Day { year, month, day }.field(),
            None,
            "{year:?}-{month}-{day}"
        );
    }
    assert_eq!(Day::of_field(&[1, 2, 3]), None);
    assert_eq!(Day::of_field(&[]), None);
}

#[test]
fn the_next_birthday_and_the_age() {
    let ada = Day {
        year: Some(1815),
        month: 12,
        day: 10,
    };
    let on = |y, m, d| {
        date::days_from(Date {
            year: y,
            month: m,
            day: d,
        })
    };
    assert_eq!(ada.next_from(on(2026, 11, 23)), on(2026, 12, 10));
    assert_eq!(ada.next_from(on(2026, 12, 10)), on(2026, 12, 10));
    assert_eq!(ada.next_from(on(2026, 12, 11)), on(2027, 12, 10));
    assert_eq!(ada.years_on(on(2026, 12, 9)), Some(210));
    assert_eq!(ada.years_on(on(2026, 12, 10)), Some(211));
    assert_eq!(ada.years_on(on(1800, 1, 1)), None);
    // A leap-day birthday falls on 28 February in other years.
    let leap = Day {
        year: None,
        month: 2,
        day: 29,
    };
    assert_eq!(leap.next_from(on(2027, 1, 1)), on(2027, 2, 28));
    assert_eq!(leap.next_from(on(2028, 1, 1)), on(2028, 2, 29));
    assert_eq!(leap.years_on(on(2028, 3, 1)), None);
}

#[test]
fn an_address_of_the_first_version_is_read_into_its_parts() {
    let a = Address::of(b"12 St James Sq\nSeattle, WA 98101");
    assert_eq!(
        a,
        Address {
            street: b"12 St James Sq",
            city: b"Seattle",
            region: b"WA",
            postal: b"98101",
            country: b"",
        }
    );
    // Photon's city line, and the shapes people type.
    let a = Address::city_line(b"Seattle, Washington 98101-1234");
    assert_eq!(
        (a.city, a.region, a.postal),
        (&b"Seattle"[..], &b"Washington"[..], &b"98101-1234"[..])
    );
    let a = Address::city_line(b"Portland, OR");
    assert_eq!(
        (a.city, a.region, a.postal),
        (&b"Portland"[..], &b"OR"[..], &b""[..])
    );
    let a = Address::city_line(b"London SW1A 1AA");
    assert_eq!((a.city, a.region), (&b"London SW1A 1AA"[..], &b""[..]));
    let a = Address::city_line(b"Paris, 75001");
    assert_eq!((a.city, a.postal), (&b"Paris"[..], &b"75001"[..]));
    let a = Address::of(b"Just a street");
    assert_eq!(a.street, b"Just a street");
    assert!(Address::of(b"").is_empty());
}

#[test]
fn an_address_round_trips_as_five_lines() {
    let a = Address {
        street: b"10 Downing St",
        city: b"London",
        region: b"",
        postal: b"SW1A 2AA",
        country: b"United Kingdom",
    };
    let mut out = [0u8; 128];
    let n = a.field(&mut out).unwrap();
    assert_eq!(
        &out[..n],
        b"10 Downing St\nLondon\n\nSW1A 2AA\nUnited Kingdom"
    );
    assert_eq!(Address::of(&out[..n]), a);
    // A city with a comma stays the city (no first-version guess on five
    // lines).
    let b = Address {
        city: b"Washington, D.C.",
        ..Address::default()
    };
    let n = b.field(&mut out).unwrap();
    assert_eq!(Address::of(&out[..n]), b);
    assert_eq!(Address::default().field(&mut out), Some(0));
    // Too small: refused, never cut.
    assert_eq!(a.field(&mut [0u8; 8]), None);
    let mut line = [0u8; 64];
    let k = a.city_text(&mut line);
    assert_eq!(&line[..k], b"London SW1A 2AA, United Kingdom");
}

#[test]
fn entries_round_trip_and_a_cut_field_never_panics() {
    let mut buf = [0u8; 256];
    let mut n = 0;
    let list = [
        Entry {
            kind: Kind::Phone,
            label: b"Cabin",
            value: b"+13605550199",
        },
        Entry {
            kind: Kind::Email,
            label: b"",
            value: b"ada@work.example",
        },
        Entry {
            kind: Kind::Phone,
            label: b"School office, main desk, second floor",
            value: b"+12065550100",
        },
    ];
    for e in list {
        n = push_entry(&mut buf, n, e).unwrap();
    }
    let got: std::vec::Vec<_> = entries(&buf[..n]).collect();
    assert_eq!(got.len(), 3);
    assert_eq!(got[0], list[0]);
    assert_eq!(got[1], list[1]);
    // A long label is cut to its room, the value whole.
    assert_eq!(got[2].label.len(), LABEL_MAX);
    assert_eq!(got[2].value, list[2].value);
    for cut in 0..n {
        let k = entries(&buf[..cut]).count();
        assert!(k <= 3);
    }
    // Junk kinds and lengths past the end stop the list.
    assert_eq!(entries(&[9, 0, 0]).count(), 0);
    assert_eq!(entries(&[1, 200, 1]).count(), 0);
    assert_eq!(entries(&[1, 0, 200, 1]).count(), 0);
    for b in 0..=255u8 {
        let _ = entries(&[b; 40]).count();
    }
    // No room: refused.
    assert_eq!(push_entry(&mut [0u8; 4], 0, list[0]), None);
}

#[test]
fn the_same_number_or_mail_however_it_was_written() {
    assert_eq!(phone_key(b"+12065550142"), phone_key(b"(206) 555-0142"));
    assert_eq!(phone_key(b"+1 206 555 0142"), phone_key(b"206.555.0142"));
    assert_ne!(phone_key(b"+12065550142"), phone_key(b"+12065550143"));
    assert_eq!(phone_key(b"x123"), None);
    assert_eq!(phone_key(b""), None);
    assert!(phone_key(b"5550142").is_some());
    assert!(same_email(b"Ada@Example.com", b" ada@example.COM "));
    assert!(!same_email(b"", b""));
    assert!(!same_email(b"ada", b"ada"));
    assert!(same_name(b"Ada", b"Lovelace", b"ada", b"LOVELACE"));
    assert!(!same_name(b"", b"", b"", b""));
    assert!(!same_name(b"Ada", b"Lovelace", b"Ada", b"Byron"));
}

#[test]
fn records_sharing_a_number_or_a_mail_share_a_key() {
    use crate::store::{Kind as K, Store, tag};
    let mut m = std::vec![0u8; 16 * 1024 + crate::store::INDEX_BYTES];
    let mut s = Store::sized(&mut m, 32);
    s.load_start(None);
    let mut more = [0u8; 128];
    let n = push_entry(
        &mut more,
        0,
        Entry {
            kind: Kind::Email,
            label: b"Work",
            value: b"ADA@Work.example",
        },
    )
    .unwrap();
    let a = s
        .put(
            K::Contact,
            None,
            &[
                (tag::FIRST, b"Ada"),
                (tag::MOBILE, b"+12065550142"),
                (tag::MORE, &more[..n]),
            ],
        )
        .unwrap();
    let b = s
        .put(
            K::Contact,
            None,
            &[(tag::FIRST, b"A."), (tag::EMAIL, b" ada@work.example")],
        )
        .unwrap();
    let c = s
        .put(
            K::Contact,
            None,
            &[(tag::FIRST, b"Bo"), (tag::HOME, b"(206) 555-0142")],
        )
        .unwrap();
    let d = s
        .put(
            K::Contact,
            None,
            &[(tag::FIRST, b"Cy"), (tag::HOME, b"+12065550999")],
        )
        .unwrap();
    let keys = |id| {
        let mut k = [0u64; 16];
        let n = keys(&s.get(id).unwrap(), &mut k);
        k[..n].to_vec()
    };
    let common = |x: &[u64], y: &[u64]| x.iter().any(|k| y.contains(k));
    let (ka, kb, kc, kd) = (keys(a), keys(b), keys(c), keys(d));
    assert_eq!(ka.len(), 2);
    assert!(common(&ka, &kb), "the same mail, case and spaces apart");
    assert!(common(&ka, &kc), "the same number, written two ways");
    assert!(!common(&ka, &kd));
    assert!(!common(&kb, &kc));
}
