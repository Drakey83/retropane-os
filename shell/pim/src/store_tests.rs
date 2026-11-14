//! The store's tests: a card of two slot files in memory, power cuts at
//! every write, slots and arenas filled, junk on the card.

extern crate std;
use super::*;
use std::vec;
use std::vec::Vec;

/// A card: two slot files of `sectors` each, and how writes go.
struct Card {
    slots: [Vec<[u8; SECTOR]>; 2],
    writes: usize,
    /// Stop taking writes after this many (a power cut).
    cut_after: Option<usize>,
}

impl Card {
    fn new(sectors: u32) -> Self {
        Self {
            // Old junk, not zeros: a slot is never assumed clean.
            slots: [
                vec![[0xA5; SECTOR]; sectors as usize],
                vec![[0x5A; SECTOR]; sectors as usize],
            ],
            writes: 0,
            cut_after: None,
        }
    }

    /// Writes everything the store has, a sector at a time.
    fn flush(&mut self, s: &mut Store) {
        let mut out = [0u8; SECTOR];
        while let Some(at) = s.next_write(&mut out) {
            if self.cut_after.is_some_and(|n| self.writes >= n) {
                return;
            }
            self.slots[usize::from(at.slot)][at.sector as usize] = out;
            self.writes += 1;
            s.written(true);
        }
    }

    fn load(&self, s: &mut Store) {
        s.load_start(pick([&self.slots[0][0], &self.slots[1][0]]));
        while let Some(at) = s.load_want() {
            s.load_sector(self.slots[usize::from(at.slot)].get(at.sector as usize));
        }
    }
}

fn memory(arena: usize) -> Vec<u8> {
    vec![0xEE; arena + INDEX_BYTES]
}

fn fresh(m: &mut [u8], sectors: u32) -> Store<'_> {
    let mut s = Store::sized(m, sectors);
    s.load_start(None);
    s
}

fn titles(s: &Store, kind: Kind) -> Vec<std::string::String> {
    let mut v: Vec<_> = s
        .all(kind)
        .map(|r| std::string::String::from_utf8_lossy(r.text(tag::TITLE)).into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn a_record_is_put_read_replaced_and_removed() {
    let mut m = memory(4096);
    let mut s = fresh(&mut m, 64);
    let n = 600u32.to_le_bytes();
    let a = s
        .put(
            Kind::Appointment,
            None,
            &[
                (tag::TITLE, b"Dentist"),
                (tag::START, &n),
                (tag::PLACE, b""),
            ],
        )
        .unwrap();
    let b = s
        .put(Kind::Task, None, &[(tag::TITLE, b"Pay rent")])
        .unwrap();
    assert_ne!(a, b);
    let r = s.get(a).unwrap();
    assert_eq!(
        (r.kind, r.text(tag::TITLE), r.num(tag::START)),
        (Kind::Appointment, &b"Dentist"[..], Some(600))
    );
    // An empty field isn't stored; an absent one reads empty or None.
    assert_eq!(r.text(tag::PLACE), b"");
    assert_eq!(r.num(tag::MINUTES), None);
    assert_eq!(
        s.put(Kind::Appointment, Some(a), &[(tag::TITLE, b"Doctor")]),
        Ok(a)
    );
    assert_eq!(s.get(a).unwrap().text(tag::TITLE), b"Doctor");
    assert_eq!(s.get(a).unwrap().num(tag::START), None);
    assert_eq!(s.len(), 2);
    assert_eq!(s.remove(a), Ok(()));
    assert!(s.get(a).is_none());
    assert_eq!(s.remove(a), Err(Error::NotFound));
    assert_eq!(s.put(Kind::Task, Some(999), &[]), Err(Error::NotFound));
    assert_eq!(titles(&s, Kind::Task), ["Pay rent"]);
    assert!(titles(&s, Kind::Appointment).is_empty());
}

#[test]
fn what_is_written_is_read_back_the_same_and_ids_go_on() {
    let mut card = Card::new(64);
    let mut m = memory(8192);
    let mut s = fresh(&mut m, 64);
    let mut ids = Vec::new();
    for i in 0..20 {
        let t = std::format!("task {i}");
        ids.push(
            s.put(Kind::Task, None, &[(tag::TITLE, t.as_bytes())])
                .unwrap(),
        );
        if i % 3 == 0 {
            card.flush(&mut s);
        }
    }
    s.remove(ids[4]).unwrap();
    s.put(Kind::Task, Some(ids[7]), &[(tag::TITLE, b"changed")])
        .unwrap();
    assert!(s.dirty());
    card.flush(&mut s);
    assert!(!s.dirty());
    let want = titles(&s, Kind::Task);
    let mut m2 = memory(8192);
    let mut t = Store::sized(&mut m2, 64);
    assert!(!t.loaded());
    assert_eq!(
        t.put(Kind::Task, None, &[(tag::TITLE, b"early")]),
        Err(Error::Loading)
    );
    card.load(&mut t);
    assert!(t.loaded() && !t.dirty());
    assert_eq!(titles(&t, Kind::Task), want);
    assert_eq!(want.len(), 19);
    // A new record's id is past every id the log ever held.
    let next = t.put(Kind::Note, None, &[(tag::TEXT, b"n")]).unwrap();
    assert!(ids.iter().all(|&i| next > i));
}

#[test]
fn a_power_cut_loses_only_the_change_being_written() {
    // A long note takes four sectors: cut after each write in turn.
    let long = vec![b'x'; 1500];
    for cut in 0..8 {
        let mut card = Card::new(64);
        let mut m = memory(8192);
        let mut s = fresh(&mut m, 64);
        s.put(Kind::Note, None, &[(tag::TEXT, b"kept")]).unwrap();
        card.flush(&mut s);
        let before = card.writes;
        s.put(Kind::Note, None, &[(tag::TEXT, &long)]).unwrap();
        card.cut_after = Some(before + cut);
        card.flush(&mut s);
        let mut m2 = memory(8192);
        let mut t = Store::sized(&mut m2, 64);
        card.load(&mut t);
        let texts: Vec<usize> = t.all(Kind::Note).map(|r| r.text(tag::TEXT).len()).collect();
        // The first note always; the long one only when all of it landed.
        assert!(texts.contains(&4), "cut {cut}: {texts:?}");
        assert_eq!(texts.len(), if cut >= 3 { 2 } else { 1 }, "cut {cut}");
        // And the store goes on from there: a new change lands whole.
        card.cut_after = None;
        t.put(Kind::Note, None, &[(tag::TEXT, b"after")]).unwrap();
        card.flush(&mut t);
        let mut m3 = memory(8192);
        let mut u = Store::sized(&mut m3, 64);
        card.load(&mut u);
        assert_eq!(u.len(), t.len(), "cut {cut}");
    }
}

#[test]
fn a_full_slot_moves_the_live_records_to_the_other_and_back() {
    let mut card = Card::new(16);
    let mut m = memory(4096);
    let mut s = fresh(&mut m, 16);
    let id = s.put(Kind::Task, None, &[(tag::TITLE, b"v0")]).unwrap();
    let keep = s.put(Kind::Contact, None, &[(tag::FIRST, b"Ada")]).unwrap();
    // Each change is a sector: 15 fill a slot, 100 cross it six times.
    for i in 1..=100 {
        let t = std::format!("v{i}");
        s.put(Kind::Task, Some(id), &[(tag::TITLE, t.as_bytes())])
            .unwrap();
        card.flush(&mut s);
        let mut m2 = memory(4096);
        let mut t2 = Store::sized(&mut m2, 16);
        card.load(&mut t2);
        assert_eq!(
            t2.get(id).unwrap().text(tag::TITLE),
            t.as_bytes(),
            "after change {i}"
        );
        assert_eq!(t2.get(keep).unwrap().text(tag::FIRST), b"Ada");
        assert_eq!(t2.len(), 2);
    }
}

#[test]
fn a_cut_during_the_move_keeps_the_old_slot_the_truth() {
    for cut in 0..6 {
        let mut card = Card::new(8);
        let mut m = memory(4096);
        let mut s = fresh(&mut m, 8);
        let id = s.put(Kind::Task, None, &[(tag::TITLE, b"v0")]).unwrap();
        card.flush(&mut s);
        for i in 1..=5 {
            let t = std::format!("v{i}");
            s.put(Kind::Task, Some(id), &[(tag::TITLE, t.as_bytes())])
                .unwrap();
            card.flush(&mut s);
        }
        // The slot is full: the next change moves everything.
        let before = card.writes;
        s.put(Kind::Task, Some(id), &[(tag::TITLE, b"moved")])
            .unwrap();
        card.cut_after = Some(before + cut);
        card.flush(&mut s);
        let mut m2 = memory(4096);
        let mut t = Store::sized(&mut m2, 8);
        card.load(&mut t);
        let got = t.get(id).unwrap().text(tag::TITLE);
        assert!(
            got == b"v5" || got == b"moved",
            "cut {cut}: {:?}",
            core::str::from_utf8(got)
        );
        if cut == 0 {
            assert_eq!(got, b"v5");
        }
    }
}

#[test]
fn a_full_arena_packs_and_then_says_full() {
    let mut card = Card::new(64);
    let mut m = memory(1024);
    let mut s = fresh(&mut m, 64);
    let id = s
        .put(Kind::Note, None, &[(tag::TEXT, &[b'a'; 200])])
        .unwrap();
    // Replacing it over and over fills the arena with dead versions.
    for _ in 0..50 {
        s.put(Kind::Note, Some(id), &[(tag::TEXT, &[b'b'; 200])])
            .unwrap();
        card.flush(&mut s);
    }
    assert_eq!(s.len(), 1);
    // Live records to the brim, then no more.
    let mut n = 1;
    while s
        .put(Kind::Note, None, &[(tag::TEXT, &[b'c'; 200])])
        .is_ok()
    {
        n += 1;
    }
    assert_eq!(n, 1024 / 211);
    assert_eq!(
        s.put(Kind::Note, None, &[(tag::TEXT, &[b'c'; 200])]),
        Err(Error::Full)
    );
    // Removing one makes room again (its tombstone fits after packing).
    s.remove(id).unwrap();
    s.put(Kind::Note, None, &[(tag::TEXT, &[b'd'; 150])])
        .unwrap();
    card.flush(&mut s);
    let mut m2 = memory(1024);
    let mut t = Store::sized(&mut m2, 64);
    card.load(&mut t);
    assert_eq!(t.len(), s.len());
    assert_eq!(
        s.put(Kind::Note, None, &[(tag::TEXT, &[b'e'; TEXT_MAX + 1])]),
        Err(Error::TooLong)
    );
}

#[test]
fn a_log_longer_than_the_arena_loads_packed() {
    // Written with a big arena, read with a small one: only the live
    // record has to fit.
    let mut card = Card::new(64);
    let mut m = memory(16 * 1024);
    let mut s = fresh(&mut m, 64);
    let id = s
        .put(Kind::Note, None, &[(tag::TEXT, &[b'a'; 300])])
        .unwrap();
    for _ in 0..20 {
        s.put(Kind::Note, Some(id), &[(tag::TEXT, &[b'z'; 300])])
            .unwrap();
        card.flush(&mut s);
    }
    let mut m2 = memory(1024);
    let mut t = Store::sized(&mut m2, 64);
    card.load(&mut t);
    assert_eq!(t.get(id).unwrap().text(tag::TEXT), &[b'z'; 300][..]);
    // It rewrites the card packed, and that loads too.
    assert!(t.dirty());
    card.flush(&mut t);
    let mut m3 = memory(1024);
    let mut u = Store::sized(&mut m3, 64);
    card.load(&mut u);
    assert_eq!(u.get(id).unwrap().text(tag::TEXT), &[b'z'; 300][..]);
    assert!(!u.dirty());
}

#[test]
fn junk_and_stale_groups_never_load_and_never_panic() {
    // Nothing valid: the store starts empty and writes its first header.
    let card = Card::new(16);
    let mut m = memory(2048);
    let mut s = Store::sized(&mut m, 16);
    card.load(&mut s);
    assert!(s.loaded() && s.is_empty() && s.dirty());
    // A valid header, then a group that is wrong in one way at a time.
    let mut card = Card::new(16);
    let mut m = memory(2048);
    let mut s = fresh(&mut m, 16);
    s.put(Kind::Task, None, &[(tag::TITLE, b"real")]).unwrap();
    card.flush(&mut s);
    let good = card.slots[0][1];
    type Spoil<'a> = &'a dyn Fn(&mut [u8; SECTOR]);
    let cases: [(&str, Spoil); 5] = [
        ("another generation", &|g| g[4] ^= 1),
        ("a length past the slot", &|g| {
            g[8..12].copy_from_slice(&4000u32.to_le_bytes());
        }),
        ("a zero length", &|g| {
            g[8..12].copy_from_slice(&0u32.to_le_bytes())
        }),
        ("a damaged byte", &|g| g[20] ^= 0x40),
        ("no magic", &|g| g[0] = 0),
    ];
    for (what, f) in cases {
        let mut bad = good;
        f(&mut bad);
        card.slots[0][1] = bad;
        let mut m2 = memory(2048);
        let mut t = Store::sized(&mut m2, 16);
        card.load(&mut t);
        assert!(t.loaded() && t.is_empty(), "{what}");
    }
    // A group with a right CRC over a record that lies about its length.
    let mut bad = [0u8; SECTOR];
    let payload = [3u8, 0, 0xFF, 0x7F, 1, 0, 0, 0, 1, 2, 3, 4];
    bad[..4].copy_from_slice(&GROUP_MAGIC.to_le_bytes());
    bad[4..8].copy_from_slice(&1u32.to_le_bytes());
    bad[8..12].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    bad[12..16].copy_from_slice(&crc32(&payload).to_le_bytes());
    bad[16..16 + payload.len()].copy_from_slice(&payload);
    card.slots[0][1] = bad;
    let mut m2 = memory(2048);
    let mut t = Store::sized(&mut m2, 16);
    card.load(&mut t);
    assert!(t.loaded() && t.is_empty());
    // Every byte pattern in a header or a group sector: never a panic.
    for seed in 0..200u32 {
        let mut h = [0u8; SECTOR];
        let mut x = seed.wrapping_mul(2_654_435_761) | 1;
        for b in &mut h {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *b = x as u8;
        }
        let _ = pick([&h, &good]);
        card.slots[0][1] = h;
        let mut m3 = memory(2048);
        let mut u = Store::sized(&mut m3, 16);
        card.load(&mut u);
        assert!(u.loaded());
    }
}

#[test]
fn the_newer_generation_wins_and_a_refusing_card_stalls_then_recovers() {
    let mut a = [0u8; SECTOR];
    let mut b = [0u8; SECTOR];
    for (h, generation) in [(&mut a, 7u32), (&mut b, 8)] {
        h[..8].copy_from_slice(SLOT_MAGIC);
        h[8..12].copy_from_slice(&generation.to_le_bytes());
        let c = crc32(&h[..12]);
        h[12..16].copy_from_slice(&c.to_le_bytes());
    }
    assert_eq!(pick([&a, &b]), Some((1, 8)));
    assert_eq!(pick([&b, &a]), Some((0, 8)));
    assert_eq!(pick([&a, &[0; SECTOR]]), Some((0, 7)));
    assert_eq!(pick([&[0; SECTOR], &[0xFF; SECTOR]]), None);
    let mut m = memory(2048);
    let mut s = fresh(&mut m, 16);
    s.put(Kind::Task, None, &[(tag::TITLE, b"t")]).unwrap();
    let mut out = [0u8; SECTOR];
    for _ in 0..3 {
        assert!(s.next_write(&mut out).is_some());
        s.written(false);
    }
    assert!(s.stalled() && s.next_write(&mut out).is_none());
    // The record is still there, and a retry writes it.
    assert_eq!(s.len(), 1);
    s.retry();
    let mut card = Card::new(16);
    card.flush(&mut s);
    assert!(!s.dirty());
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
}
