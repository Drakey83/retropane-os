//! The organiser's store on the card (docs/design/pim-store.md): where its
//! two slot files lie, found or made once at start-up, then its sectors
//! read and written straight off the card, a piece a pass of the image's
//! loop (the file system is touched once a run: a save is one sector
//! written in place, a few milliseconds, never a directory or FAT write).
//! The store itself (`rp_pim::store`) does no I/O and decides what a
//! sector means; this only moves the sectors it names.
//! Generic over the card so the host tests run on a card image; the image
//! settles its card log's controller first and logs only after.
//! FAT and exFAT cards alike (`rp_fat::AnyVolume`, audit C98).

use rp_fat::{AnyVolume, Error, Reserved};
use rp_hal_api::BlockDevice;
use rp_pim::store::{self, SECTOR, SLOT_SECTORS, Store};

/// The two slot files.
pub const PATHS: [&str; 2] = ["RPPIM/A.RPP", "RPPIM/B.RPP"];
/// A slot file's bytes.
pub const SLOT_BYTES: usize = SLOT_SECTORS as usize * SECTOR;
/// The sectors one load piece reads at most (32 KiB, ~30 ms on the X51v:
/// the icon pack's rate).
pub const PIECE: usize = 64;
/// The bytes of the buffer [`load`] is given.
pub const BUF: usize = PIECE * SECTOR;

/// Where the slot files lie: each one run of sectors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Files {
    /// Each slot's first sector on the card.
    pub lba: [u32; 2],
}

/// The slot files of the card's volume at `start`, each found where it
/// lies, else made (zero-filled in one run: a slot must never show what
/// its clusters held before, an older store's groups would read as this
/// one's): the files, and how many were made.
///
/// # Errors
/// The reason in words: no volume, no free run of 1 MiB, or the card's.
pub fn find<D: BlockDevice>(
    card: D,
    start: u32,
    scratch: &mut [u8],
) -> Result<(Files, u8), &'static str> {
    let mut vol = AnyVolume::mount(card, start).map_err(|_| "the card not readable")?;
    let mut lba = [0u32; 2];
    let mut made = 0;
    for (path, at) in PATHS.iter().zip(&mut lba) {
        let run = match vol.extent(path) {
            // A whole slot in one run: used as it lies.
            Ok(r) if r.sectors >= SLOT_SECTORS => r,
            // Missing, short, or in pieces (copied back by a PC): made
            // again. Its records are lost only if it was the newer slot;
            // the other slot is kept.
            Ok(_) | Err(Error::NotFound | Error::Fragmented) => {
                made += 1;
                make(&mut vol, path, scratch)?
            }
            Err(_) => return Err("the card's folder not readable"),
        };
        *at = run.lba;
    }
    Ok((Files { lba }, made))
}

fn make<D: BlockDevice>(
    vol: &mut AnyVolume<D>,
    path: &str,
    scratch: &mut [u8],
) -> Result<Reserved, &'static str> {
    vol.reserve(path, SLOT_BYTES, scratch).map_err(|e| match e {
        Error::Fragmented | Error::NoRoom => "no free megabyte in one run on the card",
        _ => "the card not writable",
    })
}

/// Starts the store's load: both slots' headers read, the newer valid one
/// picked (none: the store starts empty). The slot and its generation.
pub fn begin<D: BlockDevice>(
    mut card: D,
    files: &Files,
    store: &mut Store<'_>,
) -> Option<(u8, u32)> {
    let mut heads = [[0u8; SECTOR]; 2];
    for (h, lba) in heads.iter_mut().zip(files.lba) {
        // An unreadable header is no header.
        if card.read_blocks(lba, h).is_err() {
            h.fill(0);
        }
    }
    let picked = store::pick([&heads[0], &heads[1]]);
    store.load_start(picked);
    picked
}

/// Reads the load's next piece: up to [`PIECE`] sectors in one read from
/// the one the store wants, fed to it while it wants the next in line (the
/// log's end is in the last piece: what was read past it is dropped).
/// The sectors the store took; `store.loaded()` says when it is over.
pub fn load<D: BlockDevice>(
    mut card: D,
    files: &Files,
    store: &mut Store<'_>,
    buf: &mut [u8],
) -> usize {
    let Some(want) = store.load_want() else {
        return 0;
    };
    let room = (SLOT_SECTORS.saturating_sub(want.sector) as usize)
        .min(PIECE)
        .min(buf.len() / SECTOR);
    let lba = files.lba[usize::from(want.slot & 1)] + want.sector;
    if room == 0 || card.read_blocks(lba, &mut buf[..room * SECTOR]).is_err() {
        // Not readable: the log ends here (what was read so far stays).
        store.load_sector(None);
        return 0;
    }
    let mut fed = 0;
    let (chunks, _) = buf[..room * SECTOR].as_chunks::<SECTOR>();
    for (k, sector) in chunks.iter().enumerate() {
        match store.load_want() {
            Some(at) if at.slot == want.slot && at.sector == want.sector + k as u32 => {
                store.load_sector(Some(sector));
                fed += 1;
            }
            _ => break,
        }
    }
    fed
}

/// Writes the store's waiting sectors, `max` at most: the sectors
/// written, and whether one failed (the store counts failures and stalls
/// after three in a row; `Store::retry` tries the card again).
pub fn save<D: BlockDevice>(
    mut card: D,
    files: &Files,
    store: &mut Store<'_>,
    max: usize,
) -> (usize, bool) {
    let mut sector = [0u8; SECTOR];
    let mut n = 0;
    while n < max {
        let Some(at) = store.next_write(&mut sector) else {
            break;
        };
        let ok = at.sector < SLOT_SECTORS
            && card
                .write_blocks(files.lba[usize::from(at.slot & 1)] + at.sector, &sector)
                .is_ok();
        store.written(ok);
        if !ok {
            return (n, true);
        }
        n += 1;
    }
    (n, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_hal_api::BlockError;
    use rp_pim::store::{Kind, tag};

    /// A card in RAM; writes fail while its flag is set.
    struct Card(Vec<u8>, bool);

    impl BlockDevice for Card {
        fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
            let o = lba as usize * 512;
            let src = self.0.get(o..o + buf.len()).ok_or(BlockError::OutOfRange)?;
            buf.copy_from_slice(src);
            Ok(())
        }
        fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
            if self.1 {
                return Err(BlockError::OutOfRange);
            }
            let o = lba as usize * 512;
            let dst = self
                .0
                .get_mut(o..o + buf.len())
                .ok_or(BlockError::OutOfRange)?;
            dst.copy_from_slice(buf);
            Ok(())
        }
    }

    /// A FAT32 card with another file on it (the shared card), and where
    /// its partition starts.
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
        (Card(img, false), start)
    }

    fn exfat_card() -> (Card, u32) {
        let sectors = 64 * 1024 * 1024 / 512;
        let o = rp_fat::mkfs::Options {
            fs: Some(rp_fat::mkfs::Fs::ExFat),
            cluster_bytes: Some(4096),
            align_sectors: None,
            label: "PIM",
            serial: 0x1234_5678,
        };
        let l = rp_fat::mkfs::plan(sectors, &o).expect("an exFAT layout");
        let mut c = Card(vec![0; sectors as usize * 512], false);
        rp_fat::mkfs::write(&mut c, &l, &mut vec![0u8; 16 * 1024]).expect("formatted");
        (c, l.part_start)
    }

    /// A store loaded from the card's files, as the image does it.
    fn open(c: &mut Card, start: u32) -> (Store<'static>, Files, u8) {
        let (files, made) = find(&mut *c, start, &mut vec![0u8; 8192]).expect("the files");
        let mut s = Store::new(vec![0u8; store::MEMORY].leak());
        begin(&mut *c, &files, &mut s);
        let mut buf = vec![0u8; BUF];
        let mut passes = 0;
        while !s.loaded() {
            load(&mut *c, &files, &mut s, &mut buf);
            passes += 1;
            assert!(passes < 100, "the load never ends");
        }
        (s, files, made)
    }

    fn flush(c: &mut Card, files: &Files, s: &mut Store<'_>) {
        while s.dirty() {
            let (n, failed) = save(&mut *c, files, s, 1);
            assert!(!failed && n == 1);
        }
    }

    #[test]
    fn records_written_are_there_after_a_restart_on_fat_and_exfat() {
        for (mut c, start) in [card(), exfat_card()] {
            let (mut s, files, made) = open(&mut c, start);
            assert_eq!(made, 2, "a new card: both slots made");
            assert!(s.is_empty());
            let a = s
                .put(Kind::Note, None, &[(tag::TEXT, b"Milk, eggs")])
                .unwrap();
            let b = s
                .put(
                    Kind::Contact,
                    None,
                    &[(tag::FIRST, b"Ada"), (tag::LAST, b"Lovelace")],
                )
                .unwrap();
            flush(&mut c, &files, &mut s);
            s.remove(a).unwrap();
            flush(&mut c, &files, &mut s);
            // The restart: the same files found (none made), the same
            // records.
            let (again, files2, made) = open(&mut c, start);
            assert_eq!((files2, made), (files, 0));
            assert_eq!(again.len(), 1);
            assert_eq!(again.get(b).unwrap().text(tag::LAST), b"Lovelace");
            assert!(again.get(a).is_none());
        }
    }

    /// The shared card: the other file is as it was after the slots are
    /// made and written.
    #[test]
    fn the_cards_other_files_stay() {
        let (mut c, start) = card();
        let (mut s, files, _) = open(&mut c, start);
        s.put(Kind::Note, None, &[(tag::TEXT, b"x")]).unwrap();
        flush(&mut c, &files, &mut s);
        let mut v = AnyVolume::mount(&mut c, start).unwrap();
        let mut got = Vec::new();
        v.read_range("RPIC0A.PAK", 0, u64::MAX, &mut [0u8; 1024], &mut |p| {
            got.extend_from_slice(p);
        })
        .unwrap();
        assert_eq!(got, b"the icon pack");
    }

    /// Many records: the load takes whole pieces and stops at the log's
    /// end; a full slot's move to the other slot is read back too.
    #[test]
    fn a_long_log_and_a_packed_slot_load_whole() {
        let (mut c, start) = card();
        let (mut s, files, _) = open(&mut c, start);
        let text = [b'n'; 900];
        let mut ids: Vec<u32> = Vec::new();
        // ~1.4 MiB of changes through a 1 MiB slot: it packs at least once.
        for k in 0..1500u32 {
            // The first 40 are new notes; the rest change one of them.
            let old = (k >= 40).then(|| ids[(k % 40) as usize]);
            let id = s
                .put(
                    Kind::Note,
                    old,
                    &[(tag::TEXT, &text), (tag::CHANGED, &k.to_le_bytes())],
                )
                .unwrap();
            if old.is_none() {
                ids.push(id);
            }
            if k % 7 == 0 {
                flush(&mut c, &files, &mut s);
            }
        }
        flush(&mut c, &files, &mut s);
        let (again, _, made) = open(&mut c, start);
        assert_eq!(made, 0);
        assert_eq!(again.len(), 40);
        for id in ids {
            let (a, b) = (again.get(id).unwrap(), s.get(id).unwrap());
            assert_eq!(a.num(tag::CHANGED), b.num(tag::CHANGED));
            assert!(a.num(tag::CHANGED).unwrap() >= 1460);
        }
    }

    /// A card that stops taking writes: nothing is lost in RAM, the store
    /// stalls after three failures, and a retry writes it all.
    #[test]
    fn a_dead_card_stalls_the_store_and_a_retry_saves_it() {
        let (mut c, start) = card();
        let (mut s, files, _) = open(&mut c, start);
        c.1 = true;
        let id = s
            .put(Kind::Task, None, &[(tag::TITLE, b"Call Bob")])
            .unwrap();
        for _ in 0..3 {
            assert_eq!(save(&mut c, &files, &mut s, 4), (0, true));
        }
        assert!(s.stalled() && s.dirty());
        assert_eq!(
            save(&mut c, &files, &mut s, 4),
            (0, false),
            "stalled: nothing tried"
        );
        assert_eq!(s.get(id).unwrap().text(tag::TITLE), b"Call Bob");
        c.1 = false;
        s.retry();
        flush(&mut c, &files, &mut s);
        let (again, _, _) = open(&mut c, start);
        assert_eq!(again.get(id).unwrap().text(tag::TITLE), b"Call Bob");
    }

    /// A slot file a PC left short is made again; the other slot's records
    /// stay.
    #[test]
    fn a_short_slot_file_is_made_again_and_the_other_kept() {
        let (mut c, start) = card();
        let (mut s, files, _) = open(&mut c, start);
        let id = s
            .put(Kind::Task, None, &[(tag::TITLE, b"Keep me")])
            .unwrap();
        flush(&mut c, &files, &mut s);
        // The records are in slot 0 (a new store starts there): slot 1's
        // file replaced by a short one.
        {
            let mut v = AnyVolume::mount(&mut c, start).unwrap();
            v.put(PATHS[1], b"short", &mut vec![0u8; 8192]).unwrap();
        }
        let (again, _, made) = open(&mut c, start);
        assert_eq!(made, 1);
        assert_eq!(again.get(id).unwrap().text(tag::TITLE), b"Keep me");
    }
}
