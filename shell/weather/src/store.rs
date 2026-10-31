//! The Weather settings as a small stored record (docs/design/weather-settings-build.md
//! "The settings store"): what survives a reset. The station's key is never
//! in it (R130: the protected key store).
//!
//! The record: `RPWS`, a version byte, the fields at fixed places, then a
//! CRC-32 of everything before it. A record that is short, damaged, of
//! another version or holding nonsense reads as the defaults (the key the
//! caller already has kept), never as half a setting.
//!
//! Iterate, don't imitate (rule 0): Windows Mobile kept settings in the
//! registry hive, a database rewritten whole and lost with a cold boot; ours
//! is one fixed 760-byte record, checked as a whole, that any store (the
//! dock today, a card file later) keeps as plain bytes.

use crate::Name;
use crate::settings::{Appearance, PLACES_MAX, Place, Provider, RadarMap, Settings, Station};

/// The record's length.
pub const LEN: usize = HEAD + PLACES_MAX * PLACE + 4;
const MAGIC: &[u8; 4] = b"RPWS";
const VERSION: u8 = 1;
/// The magic, version and the single-byte fields.
const HEAD: usize = 16;
/// A place: name and area (a length byte and 32 bytes each), latitude and
/// longitude (4 bytes each, little-endian).
const PLACE: usize = 2 * 33 + 8;

/// `s` as a record.
#[must_use]
pub fn encode(s: &Settings) -> [u8; LEN] {
    let mut r = [0u8; LEN];
    r[..4].copy_from_slice(MAGIC);
    r[4] = VERSION;
    r[5] = s.region;
    r[6] = u8::from(s.metric);
    r[7] = s.n_places;
    r[8] = s.current;
    r[9] = s.home;
    r[10] = s.provider as u8;
    r[11] = match s.station {
        Station::None => 0,
        Station::Tempest => 1,
    };
    r[12] = s.refresh;
    r[13] = s.pack;
    r[14] = s.radar as u8;
    r[15] = s.appearance as u8;
    for (k, p) in s.places.iter().enumerate() {
        let at = HEAD + k * PLACE;
        put_name(&mut r[at..at + 33], &p.name);
        put_name(&mut r[at + 33..at + 66], &p.area);
        r[at + 66..at + 70].copy_from_slice(&p.lat.to_le_bytes());
        r[at + 70..at + 74].copy_from_slice(&p.lon.to_le_bytes());
    }
    let crc = crc32(&r[..LEN - 4]);
    r[LEN - 4..].copy_from_slice(&crc.to_le_bytes());
    r
}

/// The settings a record holds, with `key` as the station's key; the
/// defaults (and `key`) when it isn't a sound record of this version.
#[must_use]
pub fn decode(r: &[u8], key: crate::settings::Secret) -> Settings {
    let fallback = Settings {
        tempest_key: key,
        ..Settings::DEFAULT
    };
    if r.len() != LEN || &r[..4] != MAGIC || r[4] != VERSION {
        return fallback;
    }
    let crc = u32::from_le_bytes([r[LEN - 4], r[LEN - 3], r[LEN - 2], r[LEN - 1]]);
    if crc != crc32(&r[..LEN - 4]) {
        return fallback;
    }
    let mut s = Settings {
        region: r[5],
        metric: r[6] != 0,
        n_places: r[7],
        current: r[8],
        home: r[9],
        provider: Provider::from_u8(r[10]),
        station: if r[11] == 1 {
            Station::Tempest
        } else {
            Station::None
        },
        refresh: r[12],
        pack: r[13],
        radar: match r[14] {
            0 => RadarMap::Auto,
            2 => RadarMap::Ink,
            _ => RadarMap::Night,
        },
        appearance: match r[15] {
            1 => Appearance::Light,
            2 => Appearance::Dark,
            _ => Appearance::Auto,
        },
        tempest_key: key,
        ..Settings::DEFAULT
    };
    for (k, p) in s.places.iter_mut().enumerate() {
        let at = HEAD + k * PLACE;
        let le = |i: usize| i32::from_le_bytes([r[i], r[i + 1], r[i + 2], r[i + 3]]);
        *p = Place {
            name: get_name(&r[at..at + 33]),
            area: get_name(&r[at + 33..at + 66]),
            lat: le(at + 66),
            lon: le(at + 70),
        };
    }
    // Unused slots blank, used ones real (else the defaults).
    for p in s.places.iter_mut().skip(usize::from(s.n_places)) {
        *p = Place::NONE;
    }
    s.tidy();
    s
}

fn put_name(out: &mut [u8], n: &Name) {
    let b = n.as_bytes();
    out[0] = b.len() as u8;
    out[1..=b.len()].copy_from_slice(b);
}

fn get_name(b: &[u8]) -> Name {
    let n = usize::from(b[0]).min(32);
    // Folded again: a record from anywhere holds only printable ASCII.
    Name::from_text(&b[1..=n])
}

/// CRC-32 (IEEE 802.3, the zlib and PNG one), bit by bit: the record is
/// 760 bytes, read once at start.
#[must_use]
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Secret;

    fn sample() -> Settings {
        let mut s = Settings::DEFAULT;
        s.next_region();
        s.metric = false;
        assert!(s.add_place(Place {
            name: Name::new("Perryville"),
            area: Name::new("Missouri, US"),
            lat: 37_724,
            lon: -89_861,
        }));
        s.home = 1;
        s.provider = Provider::MetNo;
        s.station = Station::Tempest;
        s.refresh = 1;
        s.radar = RadarMap::Auto;
        s.appearance = Appearance::Light;
        s.tempest_key = Secret::new(b"secret-token-1234").unwrap();
        s
    }

    fn same(a: &Settings, b: &Settings) {
        assert_eq!(
            (a.region, a.metric, a.n_places, a.current, a.home),
            (b.region, b.metric, b.n_places, b.current, b.home)
        );
        assert_eq!(
            (a.provider, a.station, a.refresh, a.pack),
            (b.provider, b.station, b.refresh, b.pack)
        );
        assert_eq!(
            (a.radar, a.appearance, a.places),
            (b.radar, b.appearance, b.places)
        );
    }

    #[test]
    fn a_record_round_trips_and_never_holds_the_key() {
        let s = sample();
        let r = encode(&s);
        assert_eq!(LEN, 760);
        assert!(!r.windows(5).any(|w| w == b"token"), "the key never stored");
        let back = decode(&r, Secret::NONE);
        same(&back, &s);
        assert!(!back.tempest_key.is_set());
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn a_damaged_record_reads_as_the_defaults_never_half_a_setting() {
        let r = encode(&sample());
        let key = Secret::new(b"kept").unwrap();
        // Every truncation, every flipped bit.
        for n in 0..LEN {
            let d = decode(&r[..n], key);
            same(&d, &Settings::DEFAULT);
            assert_eq!(d.tempest_key.expose(), b"kept");
        }
        for i in 0..LEN {
            for bit in 0..8 {
                let mut bad = r;
                bad[i] ^= 1 << bit;
                same(&decode(&bad, key), &Settings::DEFAULT);
            }
        }
    }

    #[test]
    fn a_sound_record_with_nonsense_in_it_is_made_sane() {
        // Correct CRC, but indexes past the end and a place out of range.
        let mut s = sample();
        s.current = 9;
        s.home = 200;
        s.n_places = 2;
        let mut r = encode(&s);
        r[8] = 9;
        r[9] = 200;
        let crc = crc32(&r[..LEN - 4]);
        r[LEN - 4..].copy_from_slice(&crc.to_le_bytes());
        let d = decode(&r, Secret::NONE);
        assert_eq!((d.current, d.home), (1, 1));
        s.places[1].lat = 95_000;
        let mut r = encode(&s);
        let crc = crc32(&r[..LEN - 4]);
        r[LEN - 4..].copy_from_slice(&crc.to_le_bytes());
        same(&decode(&r, Secret::NONE), &Settings::DEFAULT);
    }

    #[test]
    fn places_add_pick_and_remove_with_current_and_home_following() {
        let mut s = Settings::DEFAULT;
        let p = |n: &str, lat| Place {
            name: Name::new(n),
            area: Name::new("X"),
            lat,
            lon: 0,
        };
        assert!(s.add_place(p("A", 1)));
        assert!(s.add_place(p("B", 2)));
        assert_eq!((s.n_places, s.current), (3, 2));
        // Already kept: picked, not added.
        assert!(s.add_place(p("A", 1)));
        assert_eq!((s.n_places, s.current), (3, 1));
        s.home = 2;
        s.remove_place(1);
        assert_eq!(s.places()[1].name.as_str(), "B");
        assert_eq!((s.n_places, s.current, s.home), (2, 0, 1));
        for k in 0..20 {
            let _ = s.add_place(Place {
                name: Name::from_text(&[b'C', b'a' + k as u8]),
                ..p("C", k)
            });
        }
        assert_eq!(s.n_places as usize, PLACES_MAX);
        assert!(!s.add_place(p("full", 99)));
        // Never the last one.
        let mut one = Settings::DEFAULT;
        one.remove_place(0);
        assert_eq!(one.n_places, 1);
    }
}
