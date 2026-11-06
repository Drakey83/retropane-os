//! The radar's map for a place kept on the card (R184 step 3, sheet
//! docs/research/sheets/r184-radar-map-card.md, way A; the owner's go
//! 2026-11-06).
//!
//! The relay draws each place's map once (`RPML`, [`crate::maplayers`]);
//! the device keeps the file as it came, in its own folder on the SD card
//! ([`DIR`]: the shared card rule), one file a place and zoom, behind a
//! small header of ours, `RPMC`: the magic, a version, the zoom, the place
//! (latitude and longitude in thousandths), the map's length and CRC-32,
//! when it was written (Unix seconds, for the cap: the oldest goes first)
//! and the header's own CRC-32. Read back, the file is hostile input
//! (docs/security.md): a wrong magic, place, length or checksum means the
//! file isn't used and the dock is asked; a file torn by a reset fails its
//! CRC (and rp-fat's copy-then-swap keeps the old one whole meanwhile).

use crate::store::crc32;

/// The folder (8.3: the image's packs and rp-fat's short names).
pub const DIR: &str = "RPMAPS";
/// The header's first bytes.
pub const MAGIC: &[u8; 4] = b"RPMC";
/// The header's version.
pub const VERSION: u8 = 1;
/// The header's length.
pub const HEAD: usize = 32;
/// The largest map kept (the dock's answer buffer: 256 KiB).
pub const MAX: usize = 256 * 1024;
/// Maps kept at most (~6 MB at ~180 KB each); the place shown and the
/// saved places' are never the ones dropped.
pub const CAP: usize = 32;

/// Whose map: the place (thousandths of a degree) and the zoom.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Key {
    /// Latitude, thousandths of a degree.
    pub lat: i32,
    /// Longitude, thousandths of a degree.
    pub lon: i32,
    /// The map's zoom.
    pub zoom: u8,
}

impl Key {
    fn bytes(self) -> [u8; 9] {
        let mut b = [0u8; 9];
        b[..4].copy_from_slice(&self.lat.to_le_bytes());
        b[4..8].copy_from_slice(&self.lon.to_le_bytes());
        b[8] = self.zoom;
        b
    }

    /// The file's path: [`DIR`], then the key's CRC-32 in hex (8.3; two
    /// keys sharing a name are told apart by the header's key, the newer
    /// replacing the older).
    #[must_use]
    pub fn path(self) -> [u8; 19] {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        let mut p = *b"RPMAPS/00000000.RPM";
        let h = crc32(&self.bytes());
        for (i, c) in p[7..15].iter_mut().enumerate() {
            *c = HEX[(h >> (28 - 4 * i)) as usize & 15];
        }
        p
    }
}

/// What a header says.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Head {
    /// Whose map.
    pub key: Key,
    /// The map's length.
    pub len: usize,
    /// The map's CRC-32.
    pub crc: u32,
    /// When it was written (Unix seconds; 0 if unknown).
    pub at: u32,
}

/// Why a file wasn't used.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// Not an `RPMC` header of this version, or damaged.
    Header,
    /// Another place's or zoom's map.
    Key,
    /// The map's bytes not its length or checksum.
    Map,
}

/// The header for `map` (the relay's `RPML` bytes) of `key`, written at
/// `at`.
#[must_use]
pub fn head(key: Key, map: &[u8], at: u32) -> [u8; HEAD] {
    let mut h = [0u8; HEAD];
    h[..4].copy_from_slice(MAGIC);
    h[4] = VERSION;
    h[5] = key.zoom;
    h[8..12].copy_from_slice(&key.lat.to_le_bytes());
    h[12..16].copy_from_slice(&key.lon.to_le_bytes());
    h[16..20].copy_from_slice(&(map.len() as u32).to_le_bytes());
    h[20..24].copy_from_slice(&crc32(map).to_le_bytes());
    h[24..28].copy_from_slice(&at.to_le_bytes());
    let c = crc32(&h[..28]);
    h[28..].copy_from_slice(&c.to_le_bytes());
    h
}

/// Reads a header, whoever's map it is.
///
/// # Errors
/// [`Error::Header`]: short, another magic or version, a damaged header,
/// or a length of 0 or over [`MAX`].
pub fn read(h: &[u8]) -> Result<Head, Error> {
    let h = h.get(..HEAD).ok_or(Error::Header)?;
    let word = |i: usize| u32::from_le_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]);
    if &h[..4] != MAGIC || h[4] != VERSION || word(28) != crc32(&h[..28]) {
        return Err(Error::Header);
    }
    let len = word(16) as usize;
    if len == 0 || len > MAX {
        return Err(Error::Header);
    }
    Ok(Head {
        key: Key {
            lat: word(8) as i32,
            lon: word(12) as i32,
            zoom: h[5],
        },
        len,
        crc: word(20),
        at: word(24),
    })
}

/// Reads a header and checks it is `key`'s map.
///
/// # Errors
/// As [`read`], or [`Error::Key`] for another place's or zoom's map.
pub fn check(h: &[u8], key: Key) -> Result<Head, Error> {
    let got = read(h)?;
    if got.key != key {
        return Err(Error::Key);
    }
    Ok(got)
}

/// Checks the map's bytes against their header.
///
/// # Errors
/// [`Error::Map`] if the length or the CRC-32 differs.
pub fn check_map(h: &Head, map: &[u8]) -> Result<(), Error> {
    if map.len() != h.len || crc32(map) != h.crc {
        return Err(Error::Map);
    }
    Ok(())
}

/// Which kept map to drop when the folder is full: of `kept` (each map's
/// key and when it was written), the oldest whose place is neither shown
/// nor saved (`keep`); `None` if every one is.
#[must_use]
pub fn evict(kept: &[(Key, u32)], keep: impl Fn(Key) -> bool) -> Option<usize> {
    kept.iter()
        .enumerate()
        .filter(|(_, (k, _))| !keep(*k))
        .min_by_key(|(_, (_, at))| *at)
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LH: Key = Key {
        lat: 30_645,
        lon: -97_838,
        zoom: 7,
    };

    #[test]
    fn a_header_reads_back_as_written() {
        let map = b"RPML the map's bytes";
        let h = head(LH, map, 1_793_457_000);
        let got = check(&h, LH).expect("its own");
        assert_eq!(
            got,
            Head {
                key: LH,
                len: map.len(),
                crc: crc32(map),
                at: 1_793_457_000
            }
        );
        assert_eq!(check_map(&got, map), Ok(()));
    }

    /// Hostile input (docs/security.md): every field checked, never a fault.
    #[test]
    fn a_damaged_torn_or_foreign_file_is_not_used() {
        let map = b"RPML the map's bytes";
        let h = head(LH, map, 5);
        for at in 0..HEAD {
            let mut b = h;
            b[at] ^= 0x10;
            assert!(check(&b, LH).is_err(), "byte {at}");
        }
        assert_eq!(check(&h[..HEAD - 1], LH), Err(Error::Header), "short");
        let other = Key { lat: 47_606, ..LH };
        assert_eq!(check(&head(other, map, 5), LH), Err(Error::Key));
        let zoom = Key { zoom: 8, ..LH };
        assert_eq!(check(&head(zoom, map, 5), LH), Err(Error::Key));
        let got = read(&h).unwrap();
        assert_eq!(
            check_map(&got, &map[..map.len() - 1]),
            Err(Error::Map),
            "torn"
        );
        let mut bad = *map;
        bad[3] ^= 1;
        assert_eq!(check_map(&got, &bad), Err(Error::Map), "damaged");
        // A length of 0 or past the largest, its header otherwise whole.
        for len in [0u32, MAX as u32 + 1] {
            let mut b = h;
            b[16..20].copy_from_slice(&len.to_le_bytes());
            let c = crc32(&b[..28]);
            b[28..].copy_from_slice(&c.to_le_bytes());
            assert_eq!(read(&b), Err(Error::Header), "length {len}");
        }
    }

    #[test]
    fn each_place_and_zoom_has_its_own_short_name() {
        let p = LH.path();
        let s = core::str::from_utf8(&p).unwrap();
        assert!(s.starts_with("RPMAPS/") && s.ends_with(".RPM"), "{s}");
        assert!(s[7..15].bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(p, Key { zoom: 8, ..LH }.path());
        assert_ne!(p, Key { lat: 30_646, ..LH }.path());
        assert_eq!(p, LH.path());
    }

    #[test]
    fn the_oldest_map_of_a_place_not_kept_is_dropped() {
        let k = |lat| Key { lat, ..LH };
        let kept = [(k(1), 300), (k(2), 100), (k(3), 200), (k(4), 50)];
        // 4 is saved, 2 is shown: 3 is the oldest of the rest.
        assert_eq!(evict(&kept, |x| x == k(4) || x == k(2)), Some(2));
        assert_eq!(evict(&kept, |_| false), Some(3));
        assert_eq!(evict(&kept, |_| true), None, "every one kept");
    }
}
