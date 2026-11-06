//! The radar map kept on the card for a place (R184 step 3, sheet
//! docs/research/sheets/r184-radar-map-card.md, way A; the file's header
//! in [`rp_weather::mapcard`]): its header read and checked, its bytes read
//! a piece at a time, and the dock's map written with the folder's cap.
//! Generic over the card so the host tests run them on a card image; the
//! image settles its card log's controller first and logs only after.

use rp_fat::{Error, Volume};
use rp_hal_api::BlockDevice;
use rp_weather::mapcard::{self, CAP, DIR, HEAD, Head, Key};

/// The bytes one piece reads (~30 ms on the X51v: the icon pack's rate).
pub const PIECE: u64 = 32 * 1024;

/// The bytes of a sector-sized scratch the reads use.
const SCRATCH: usize = 1024;

/// A kept map's file name in [`DIR`] from its 12-byte 8.3 name.
fn path_of(name: &[u8; 12]) -> [u8; 19] {
    let mut p = *b"RPMAPS/00000000.RPM";
    p[7..].copy_from_slice(name);
    p
}

/// `key`'s kept map's header, checked as hostile input and as `key`'s,
/// into `out` ([`HEAD`] bytes at least). The reason in words when it isn't
/// usable.
///
/// # Errors
/// No volume, no file, a short or damaged header, another place's map
/// under its name, or a file whose length isn't its header's.
pub fn head<D: BlockDevice>(
    card: D,
    start: u32,
    key: Key,
    out: &mut [u8],
) -> Result<Head, &'static str> {
    let mut vol = Volume::mount(card, start).map_err(|_| "the card not readable")?;
    let path = key.path();
    let path = core::str::from_utf8(&path).map_err(|_| "no name")?;
    let file = vol.find(path).map_err(|_| "not on the card")?;
    let mut scratch = [0u8; SCRATCH];
    let mut at = 0usize;
    let out = out.get_mut(..HEAD).ok_or("no room")?;
    vol.read_range(&file, 0, HEAD as u64, &mut scratch, |c| {
        if let Some(o) = out.get_mut(at..at + c.len()) {
            o.copy_from_slice(c);
        }
        at += c.len();
    })
    .map_err(|_| "its header not readable")?;
    if at != HEAD {
        return Err("too short");
    }
    let h = mapcard::check(out, key).map_err(|e| match e {
        mapcard::Error::Key => "another place's under its name",
        _ => "its header damaged",
    })?;
    if u64::from(file.size()) != (HEAD + h.len) as u64 {
        return Err("its length not its header's");
    }
    Ok(h)
}

/// The next piece (up to [`PIECE`]) of `key`'s kept map from byte `off`
/// into `out`: the bytes read, or `None`.
pub fn piece<D: BlockDevice>(
    card: D,
    start: u32,
    key: Key,
    off: u64,
    out: &mut [u8],
) -> Option<usize> {
    let mut vol = Volume::mount(card, start).ok()?;
    let path = key.path();
    let file = vol.find(core::str::from_utf8(&path).ok()?).ok()?;
    let mut scratch = [0u8; SCRATCH];
    let mut at = 0usize;
    let mut over = false;
    let len = PIECE.min(out.len() as u64);
    vol.read_range(&file, off, len, &mut scratch, |c| {
        match out.get_mut(at..at + c.len()) {
            Some(o) => o.copy_from_slice(c),
            None => over = true,
        }
        at += c.len();
    })
    .ok()?;
    (!over).then_some(at)
}

/// Writes `file` (the header, then the relay's map) as `key`'s kept map
/// (rp-fat's copy-then-swap: the old one whole until the new one is
/// linked; the folder made if missing), then, past [`CAP`] maps, removes
/// the oldest whose place is neither `key`'s nor one `keep` names (a
/// damaged one first): the place removed, if any. `scratch` is what the
/// FAT's free clusters are searched with (the larger, the fewer commands).
///
/// # Errors
/// The card's or the volume's.
pub fn write<D: BlockDevice>(
    card: D,
    start: u32,
    key: Key,
    file: &[u8],
    keep: &dyn Fn(Key) -> bool,
    scratch: &mut [u8],
) -> Result<Option<Key>, Error> {
    let mut vol = Volume::mount(card, start)?;
    let path = key.path();
    let path = core::str::from_utf8(&path).map_err(|_| Error::BadName)?;
    vol.put(path, file, scratch)?;
    // The cap: the folder's maps listed by name, their headers read only
    // when there are too many.
    let mut names = [[0u8; 12]; CAP + 4];
    let (mut n, mut all) = (0usize, 0usize);
    vol.for_each_in(DIR, |e| {
        if e.is_dir || e.name.len() != 12 || !e.name[8..].eq_ignore_ascii_case(b".RPM") {
            return;
        }
        all += 1;
        if n < names.len() {
            names[n].copy_from_slice(e.name);
            n += 1;
        }
    })?;
    if all <= CAP {
        return Ok(None);
    }
    const DAMAGED: Key = Key {
        lat: i32::MIN,
        lon: i32::MIN,
        zoom: 0,
    };
    let mut kept = [(DAMAGED, 0u32); CAP + 4];
    let mut small = [0u8; SCRATCH];
    for (i, name) in names[..n].iter().enumerate() {
        let p = path_of(name);
        let Ok(p) = core::str::from_utf8(&p) else {
            continue;
        };
        let mut h = [0u8; HEAD];
        let mut at = 0usize;
        if let Ok(f) = vol.find(p) {
            let _ = vol.read_range(&f, 0, HEAD as u64, &mut small, |c| {
                if let Some(o) = h.get_mut(at..at + c.len()) {
                    o.copy_from_slice(c);
                }
                at += c.len();
            });
        }
        kept[i] = mapcard::read(&h).map_or((DAMAGED, 0), |h| (h.key, h.at));
    }
    let Some(i) = mapcard::evict(&kept[..n], |k| k == key || keep(k)) else {
        return Ok(None);
    };
    let p = path_of(&names[i]);
    let p = core::str::from_utf8(&p).map_err(|_| Error::BadName)?;
    vol.remove(p)?;
    Ok(Some(kept[i].0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_hal_api::BlockError;

    /// A card in RAM.
    struct Card(Vec<u8>);

    impl BlockDevice for Card {
        fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
            let o = lba as usize * 512;
            let src = self.0.get(o..o + buf.len()).ok_or(BlockError::OutOfRange)?;
            buf.copy_from_slice(src);
            Ok(())
        }
        fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
            let o = lba as usize * 512;
            let dst = self
                .0
                .get_mut(o..o + buf.len())
                .ok_or(BlockError::OutOfRange)?;
            dst.copy_from_slice(buf);
            Ok(())
        }
    }

    /// A FAT32 card with a file already on it (the shared card: others'
    /// files stay), and where its partition starts.
    fn card() -> (Card, u32) {
        let other = rp_fat::format::Entry {
            name: "RPIC0A.PAK",
            data: b"the icon pack",
        };
        let img = rp_fat::format::build(&rp_fat::format::Image {
            total_sectors: 16 * 1024 * 1024 / 512,
            sectors_per_cluster: 8,
            files: &[other],
            blocked_clusters: &[],
            ballast: 0,
        });
        let start = u32::from_le_bytes([img[454], img[455], img[456], img[457]]);
        (Card(img), start)
    }

    const LH: Key = Key {
        lat: 30_645,
        lon: -97_838,
        zoom: 7,
    };

    /// A kept file as the image writes it: the header, then the map.
    fn kept_file(key: Key, map: &[u8], at: u32) -> Vec<u8> {
        let mut f = mapcard::head(key, map, at).to_vec();
        f.extend_from_slice(map);
        f
    }

    /// Reads `key`'s kept map back a piece at a time, as the image does.
    fn read_back(c: &mut Card, start: u32, key: Key) -> Result<Vec<u8>, &'static str> {
        let mut h = [0u8; HEAD];
        let head = head(&mut *c, start, key, &mut h)?;
        let mut map = vec![0u8; head.len];
        let mut got = 0;
        while got < head.len {
            let n = piece(&mut *c, start, key, (HEAD + got) as u64, &mut map[got..])
                .ok_or("a piece")?;
            assert!(n > 0 && n as u64 <= PIECE);
            got += n;
        }
        mapcard::check_map(&head, &map).map_err(|_| "checksum")?;
        Ok(map)
    }

    #[test]
    fn a_written_map_reads_back_whole_a_piece_at_a_time() {
        let (mut c, start) = card();
        let mut h = [0u8; HEAD];
        assert_eq!(head(&mut c, start, LH, &mut h), Err("not on the card"));
        let map: Vec<u8> = (0..180_000u32).map(|i| (i * 7 + i / 300) as u8).collect();
        let mut scratch = vec![0u8; 32 * 1024];
        let f = kept_file(LH, &map, 1_793_457_000);
        assert_eq!(
            write(&mut c, start, LH, &f, &|_| false, &mut scratch),
            Ok(None)
        );
        assert_eq!(read_back(&mut c, start, LH), Ok(map.clone()));
        // Written again (the relay's newer map): replaced, read back new.
        let newer: Vec<u8> = map.iter().map(|b| b ^ 0x5A).collect();
        let f = kept_file(LH, &newer, 1_793_460_000);
        assert_eq!(
            write(&mut c, start, LH, &f, &|_| false, &mut scratch),
            Ok(None)
        );
        assert_eq!(read_back(&mut c, start, LH), Ok(newer));
        // The other file on the card untouched (the shared card rule).
        let mut vol = Volume::mount(&mut c, start).unwrap();
        let other = vol.find("RPIC0A.PAK").unwrap();
        assert_eq!(other.size(), 13);
    }

    /// Hostile or broken files on the card: never used, never a fault.
    #[test]
    fn a_damaged_or_foreign_kept_file_is_not_used() {
        let (mut c, start) = card();
        let mut scratch = vec![0u8; 4096];
        let map = vec![9u8; 5000];
        // Another place's map under this place's name (a name collision).
        let other = Key { lat: 1, ..LH };
        let f = kept_file(other, &map, 1);
        Volume::mount(&mut c, start)
            .unwrap()
            .put(core::str::from_utf8(&LH.path()).unwrap(), &f, &mut scratch)
            .unwrap();
        let mut h = [0u8; HEAD];
        assert_eq!(
            head(&mut c, start, LH, &mut h),
            Err("another place's under its name")
        );
        // A file cut short (its header whole): its length isn't its header's.
        let f = kept_file(LH, &map, 1);
        Volume::mount(&mut c, start)
            .unwrap()
            .put(
                core::str::from_utf8(&LH.path()).unwrap(),
                &f[..f.len() - 1],
                &mut scratch,
            )
            .unwrap();
        assert_eq!(
            head(&mut c, start, LH, &mut h),
            Err("its length not its header's")
        );
        // A byte of the map changed: the checksum says so.
        let mut f = kept_file(LH, &map, 1);
        f[HEAD + 100] ^= 1;
        Volume::mount(&mut c, start)
            .unwrap()
            .put(core::str::from_utf8(&LH.path()).unwrap(), &f, &mut scratch)
            .unwrap();
        assert_eq!(read_back(&mut c, start, LH), Err("checksum"));
        // A damaged header.
        let mut f = kept_file(LH, &map, 1);
        f[3] = b'X';
        Volume::mount(&mut c, start)
            .unwrap()
            .put(core::str::from_utf8(&LH.path()).unwrap(), &f, &mut scratch)
            .unwrap();
        assert_eq!(head(&mut c, start, LH, &mut h), Err("its header damaged"));
        // No volume at all.
        let mut blank = Card(vec![0u8; 1024 * 1024]);
        assert_eq!(
            head(&mut blank, 2048, LH, &mut h),
            Err("the card not readable")
        );
    }

    /// Past the cap, the oldest map of a place neither shown nor saved goes;
    /// saved places' maps stay whatever their age.
    #[test]
    fn past_the_cap_the_oldest_unsaved_map_goes() {
        let (mut c, start) = card();
        let mut scratch = vec![0u8; 32 * 1024];
        let map = vec![1u8; 600];
        let k = |i: i32| Key {
            lat: 40_000 + i,
            ..LH
        };
        // Place 0 is saved and the oldest; place 1 the oldest unsaved.
        for i in 0..CAP as i32 {
            let f = kept_file(k(i), &map, 1000 + i as u32);
            let saved = |x: Key| x == k(0);
            assert_eq!(
                write(&mut c, start, k(i), &f, &saved, &mut scratch),
                Ok(None)
            );
        }
        let f = kept_file(k(99), &map, 5000);
        let saved = |x: Key| x == k(0);
        assert_eq!(
            write(&mut c, start, k(99), &f, &saved, &mut scratch),
            Ok(Some(k(1)))
        );
        let mut h = [0u8; HEAD];
        assert_eq!(head(&mut c, start, k(1), &mut h), Err("not on the card"));
        assert!(head(&mut c, start, k(0), &mut h).is_ok(), "saved: kept");
        assert!(head(&mut c, start, k(99), &mut h).is_ok(), "the new one");
        assert!(head(&mut c, start, k(2), &mut h).is_ok());
    }
}
