//! The radar map kept on the card for a place (R184 step 3, sheet
//! docs/research/sheets/r184-radar-map-card.md, way A; the file's header
//! in [`rp_weather::mapcard`]): its header read and checked with where the
//! file lies (its sectors then read straight off the card a pass at a
//! time: run r184s3b's reads through the folder took 445 ms), and the
//! dock's map written a piece a pass by rp-fat's staged write (one `put`
//! held the loop 532 ms, run r184s3a), with the folder's cap.
//! Generic over the card so the host tests run them on a card image; the
//! image settles its card log's controller first and logs only after.
//! FAT and exFAT cards alike (`rp_fat::AnyVolume`, audit C98).

use rp_fat::{AnyVolume, Error, Reserved, Staged};
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
/// into `out` ([`HEAD`] bytes at least), and where the file lies when its
/// clusters are one run (then read with [`sectors`]; else with
/// [`piece`]). The reason in words when it isn't usable.
///
/// # Errors
/// No volume, no file, a short or damaged header, another place's map
/// under its name, or a file whose length isn't its header's.
pub fn head<D: BlockDevice>(
    card: D,
    start: u32,
    key: Key,
    out: &mut [u8],
) -> Result<(Head, Option<Reserved>), &'static str> {
    let mut vol = AnyVolume::mount(card, start).map_err(|_| "the card not readable")?;
    let path = key.path();
    let path = core::str::from_utf8(&path).map_err(|_| "no name")?;
    let mut scratch = [0u8; SCRATCH];
    let mut at = 0usize;
    let out = out.get_mut(..HEAD).ok_or("no room")?;
    let (size, _) = vol
        .read_range(path, 0, HEAD as u64, &mut scratch, &mut |c| {
            if let Some(o) = out.get_mut(at..at + c.len()) {
                o.copy_from_slice(c);
            }
            at += c.len();
        })
        .map_err(|e| match e {
            Error::NotFound => "not on the card",
            _ => "its header not readable",
        })?;
    if at != HEAD {
        return Err("too short");
    }
    let h = mapcard::check(out, key).map_err(|e| match e {
        mapcard::Error::Key => "another place's under its name",
        _ => "its header damaged",
    })?;
    if size != (HEAD + h.len) as u64 {
        return Err("its length not its header's");
    }
    Ok((h, vol.extent(path).ok()))
}

/// Sectors `at..` of a kept map lying at `run` straight off the card into
/// `out` (whole sectors, up to [`PIECE`]): the bytes read, or `None`. The
/// file was found one run by [`head`] in this run of the program, and
/// nothing writes it while it is read (a new map from the dock stops the
/// read first).
pub fn sectors<D: BlockDevice>(
    mut card: D,
    run: Reserved,
    at: u32,
    out: &mut [u8],
) -> Option<usize> {
    let left = run.sectors.checked_sub(at).filter(|&n| n > 0)? as usize * 512;
    let n = left.min(PIECE as usize).min(out.len() / 512 * 512);
    if n == 0 {
        return None;
    }
    card.read_blocks(run.lba + at, &mut out[..n]).ok()?;
    Some(n)
}

/// Finds the free run for `key`'s new kept map of `bytes` (the header and
/// the map), its folder made if missing: the first step of a write a
/// piece a pass ([`stage_piece`], then [`commit`]).
///
/// # Errors
/// The card's or the volume's ([`Error::Fragmented`]: no run that long).
pub fn stage<D: BlockDevice>(
    card: D,
    start: u32,
    key: Key,
    bytes: usize,
    scratch: &mut [u8],
) -> Result<Staged, Error> {
    let mut vol = AnyVolume::mount(card, start)?;
    let path = key.path();
    let path = core::str::from_utf8(&path).map_err(|_| Error::BadName)?;
    vol.stage(path, bytes, scratch)
}

/// Writes sectors `at..` of a staged map from `data` (whole sectors).
///
/// # Errors
/// The card's, or a piece past the run.
pub fn stage_piece<D: BlockDevice>(
    card: D,
    start: u32,
    s: &Staged,
    at: u32,
    data: &[u8],
) -> Result<(), Error> {
    AnyVolume::mount(card, start)?.stage_write(s, at, data)
}

/// Links a staged map as `key`'s kept map (the old one whole until then),
/// then keeps the folder to its cap as [`write`] does: the place removed,
/// if any.
///
/// # Errors
/// The card's or the volume's ([`Error::Fragmented`]: the run was taken).
pub fn commit<D: BlockDevice>(
    card: D,
    start: u32,
    key: Key,
    s: Staged,
    keep: &dyn Fn(Key) -> bool,
    scratch: &mut [u8],
) -> Result<Option<Key>, Error> {
    let mut vol = AnyVolume::mount(card, start)?;
    let path = key.path();
    let path = core::str::from_utf8(&path).map_err(|_| Error::BadName)?;
    vol.commit(path, s, scratch)?;
    cap(&mut vol, key, keep)
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
    let mut vol = AnyVolume::mount(card, start).ok()?;
    let path = key.path();
    let path = core::str::from_utf8(&path).ok()?;
    let mut scratch = [0u8; SCRATCH];
    let mut at = 0usize;
    let mut over = false;
    let len = PIECE.min(out.len() as u64);
    vol.read_range(path, off, len, &mut scratch, &mut |c| {
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
    let mut vol = AnyVolume::mount(card, start)?;
    let path = key.path();
    let path = core::str::from_utf8(&path).map_err(|_| Error::BadName)?;
    vol.put(path, file, scratch)?;
    cap(&mut vol, key, keep)
}

/// Past [`CAP`] maps, removes the oldest whose place is neither `key`'s
/// nor one `keep` names (a damaged one first): the place removed, if any.
/// The folder's maps listed by name, their headers read only when there
/// are too many.
fn cap<D: BlockDevice>(
    vol: &mut AnyVolume<D>,
    key: Key,
    keep: &dyn Fn(Key) -> bool,
) -> Result<Option<Key>, Error> {
    let mut names = [[0u8; 12]; CAP + 4];
    let (mut n, mut all) = (0usize, 0usize);
    vol.list(DIR, &mut |name, is_dir, _| {
        if is_dir || name.len() != 12 || !name[8..].eq_ignore_ascii_case(b".RPM") {
            return;
        }
        all += 1;
        if n < names.len() {
            names[n].copy_from_slice(name);
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
        let _ = vol.read_range(p, 0, HEAD as u64, &mut small, &mut |c| {
            if let Some(o) = h.get_mut(at..at + c.len()) {
                o.copy_from_slice(c);
            }
            at += c.len();
        });
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
    use rp_fat::Volume;
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

    /// An exFAT card (SDXC, or formatted exFAT by `cargo xtask format`)
    /// with the same other file on it (audit C98), and where its
    /// partition starts.
    fn exfat_card() -> (Card, u32) {
        let sectors = 64 * 1024 * 1024 / 512;
        let o = rp_fat::mkfs::Options {
            fs: Some(rp_fat::mkfs::Fs::ExFat),
            cluster_bytes: Some(4096),
            align_sectors: None,
            label: "KEPT",
            serial: 0x1234_5678,
        };
        let l = rp_fat::mkfs::plan(sectors, &o).expect("an exFAT layout");
        let mut c = Card(vec![0; sectors as usize * 512]);
        rp_fat::mkfs::write(&mut c, &l, &mut vec![0u8; 16 * 1024]).expect("formatted");
        let mut v = AnyVolume::mount(&mut c, l.part_start).expect("mounted");
        assert!(matches!(v, AnyVolume::ExFat(_)), "an exFAT card");
        v.put("RPIC0A.PAK", b"the icon pack", &mut [0u8; 4096])
            .expect("the other file");
        (c, l.part_start)
    }

    /// The card's other file, untouched (the shared card rule): its size.
    fn other_size(c: &mut Card, start: u32) -> u64 {
        let mut v = AnyVolume::mount(c, start).unwrap();
        v.read_range("RPIC0A.PAK", 0, 0, &mut [0u8; 512], &mut |_| {})
            .unwrap()
            .0
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

    /// Reads `key`'s kept map back a piece at a time, as the image does:
    /// straight off the card when it lies in one run (checked equal to the
    /// read through the folder), else through the folder.
    fn read_back(c: &mut Card, start: u32, key: Key) -> Result<Vec<u8>, &'static str> {
        let mut h = [0u8; HEAD];
        let (head, run) = head(&mut *c, start, key, &mut h)?;
        if let Some(run) = run {
            let mut all = vec![0u8; run.sectors as usize * 512];
            let mut at = 0u32;
            while (at as usize) < run.sectors as usize {
                let n =
                    sectors(&mut *c, run, at, &mut all[at as usize * 512..]).ok_or("sectors")?;
                assert!(n > 0 && n as u64 <= PIECE && n % 512 == 0);
                at += (n / 512) as u32;
            }
            assert_eq!(sectors(&mut *c, run, at, &mut all), None, "past the end");
            let map = all[HEAD..HEAD + head.len].to_vec();
            if mapcard::check_map(&head, &map).is_err() {
                return Err("checksum");
            }
            let mut through = vec![0u8; head.len];
            let mut got = 0;
            while got < head.len {
                got += piece(
                    &mut *c,
                    start,
                    key,
                    (HEAD + got) as u64,
                    &mut through[got..],
                )
                .ok_or("a piece")?;
            }
            assert!(through == map, "both reads agree");
            return Ok(map);
        }
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
        let (c, start) = card();
        written_map_reads_back(c, start);
    }

    /// Audit C98: the same on an exFAT card (the folder made, the map
    /// replaced, read back both ways).
    #[test]
    fn a_written_map_reads_back_on_an_exfat_card() {
        let (c, start) = exfat_card();
        written_map_reads_back(c, start);
    }

    fn written_map_reads_back(mut c: Card, start: u32) {
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
        assert_eq!(other_size(&mut c, start), 13);
    }

    /// The image's write: staged, a piece a pass (the volume mounted again
    /// each time), committed; read back the same; a reset before the commit
    /// leaves the old map whole.
    #[test]
    fn a_map_written_a_piece_a_pass_reads_back_and_a_reset_keeps_the_old() {
        let (c, start) = card();
        staged_write_reads_back(c, start);
    }

    /// Audit C98: the staged write on an exFAT card, a reset before the
    /// commit leaving the old map whole there too.
    #[test]
    fn a_map_written_a_piece_a_pass_on_an_exfat_card() {
        let (c, start) = exfat_card();
        staged_write_reads_back(c, start);
    }

    fn staged_write_reads_back(mut c: Card, start: u32) {
        let mut scratch = vec![0u8; 32 * 1024];
        let old: Vec<u8> = (0..90_000u32).map(|i| (i * 3) as u8).collect();
        let f = kept_file(LH, &old, 1);
        assert_eq!(
            write(&mut c, start, LH, &f, &|_| false, &mut scratch),
            Ok(None)
        );
        let map: Vec<u8> = (0..176_574u32).map(|i| (i * 7 + i / 300) as u8).collect();
        let mut f = kept_file(LH, &map, 2);
        let len = f.len();
        f.resize(len.div_ceil(512) * 512, 0);
        let s = stage(&mut c, start, LH, len, &mut scratch).unwrap();
        let mut at = 0usize;
        let mut pieces = 0;
        let before_commit = loop {
            let n = (f.len() - at).min(PIECE as usize);
            stage_piece(&mut c, start, &s, (at / 512) as u32, &f[at..at + n]).unwrap();
            at += n;
            pieces += 1;
            if pieces == 3 {
                // A copy of the card mid-way: as if reset here.
                break Card(c.0.clone());
            }
        };
        while at < f.len() {
            let n = (f.len() - at).min(PIECE as usize);
            stage_piece(&mut c, start, &s, (at / 512) as u32, &f[at..at + n]).unwrap();
            at += n;
        }
        assert_eq!(
            commit(&mut c, start, LH, s, &|_| false, &mut scratch),
            Ok(None)
        );
        assert_eq!(read_back(&mut c, start, LH), Ok(map));
        let mut reset = before_commit;
        assert_eq!(
            read_back(&mut reset, start, LH),
            Ok(old),
            "the old map whole"
        );
        assert_eq!(other_size(&mut c, start), 13);
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
