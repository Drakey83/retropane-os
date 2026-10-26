//! The node layer (ADR-0037) on freshly formatted FAT12, FAT16 and FAT32
//! volumes: names in UTF-16, cursors across sectors, writes reserved whole
//! and named only once complete, power cuts cleaned up, renames and moves,
//! hostile locations.

use rp_hal_api::{BlockDevice, BlockError};

use crate::mkfs::{self, Fs, Options};
use crate::node::{Node, ROOT, Stamp, same_name, valid_name};
use crate::{AnyVolume, Error, SECTOR};

struct Disk {
    data: Vec<u8>,
    writes: usize,
    fail_at: Option<usize>,
}

impl BlockDevice for Disk {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
        let o = lba as usize * SECTOR;
        if o + buf.len() > self.data.len() {
            return Err(BlockError::OutOfRange);
        }
        buf.copy_from_slice(&self.data[o..o + buf.len()]);
        Ok(())
    }
    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
        if self.fail_at == Some(self.writes) {
            return Err(BlockError::Device);
        }
        let o = lba as usize * SECTOR;
        if o + buf.len() > self.data.len() {
            return Err(BlockError::OutOfRange);
        }
        self.writes += 1;
        self.data[o..o + buf.len()].copy_from_slice(buf);
        Ok(())
    }
}

fn format(fs: Fs, sectors: u32, cluster: u32) -> (Disk, u32) {
    let o = Options {
        fs: Some(fs),
        cluster_bytes: Some(cluster),
        align_sectors: None,
        label: "NODES",
        serial: 0x1234_5678,
    };
    let l = mkfs::plan(sectors, &o).unwrap_or_else(|e| panic!("{}", e.name()));
    let mut d = Disk {
        data: vec![0; sectors as usize * SECTOR],
        writes: 0,
        fail_at: None,
    };
    mkfs::write(&mut d, &l, &mut vec![0u8; 16 * 1024]).unwrap();
    (d, l.part_start)
}

fn any(fs: Fs, sectors: u32, cluster: u32) -> AnyVolume<Disk> {
    let (d, base) = format(fs, sectors, cluster);
    AnyVolume::mount(d, base).unwrap()
}

/// FAT32 (512-byte clusters, 64 MiB), FAT16 (fixed root, 32 MiB), FAT12
/// (8 MiB), exFAT (4 KiB clusters, 32 MiB).
fn each(mut f: impl FnMut(AnyVolume<Disk>, &str)) {
    f(any(Fs::Fat32, 131_072, 512), "FAT32");
    f(any(Fs::Fat16, 65_536, 2048), "FAT16");
    f(any(Fs::Fat12, 16_384, 4096), "FAT12");
    f(any(Fs::ExFat, 65_536, 4096), "exFAT");
}

fn u(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn names<D: BlockDevice>(v: &mut AnyVolume<D>, dir: u64) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut c = 0;
    let mut n = Node::default();
    while v.node_child(dir, &mut c, &mut n).unwrap() {
        out.push((String::from_utf16(n.name()).unwrap(), n.loc));
    }
    out
}

fn find<D: BlockDevice>(v: &mut AnyVolume<D>, dir: u64, name: &str) -> Node {
    let mut c = 0;
    let mut n = Node::default();
    while v.node_child(dir, &mut c, &mut n).unwrap() {
        if String::from_utf16(n.name()).unwrap() == name {
            return n;
        }
    }
    panic!("{name} not found");
}

fn health<D: BlockDevice>(v: &mut AnyVolume<D>) -> crate::Health {
    let (cap, _) = v.node_space(&mut vec![0u8; 8192]).unwrap();
    let mut seen = vec![0u8; (cap / 512 / 8) as usize + 16];
    v.check(&mut seen, &mut vec![0u8; 8192]).unwrap()
}

fn read_all<D: BlockDevice>(v: &mut AnyVolume<D>, n: &Node) -> Vec<u8> {
    let mut buf = vec![0u8; n.size as usize];
    let got = v.node_read(n, 0, &mut buf, &mut vec![0u8; 4096]).unwrap();
    assert_eq!(got, buf.len());
    buf
}

fn put<D: BlockDevice>(v: &mut AnyVolume<D>, dir: u64, name: &str, data: &[u8]) -> u64 {
    let s = &mut vec![0u8; 8192];
    let mut r = v.node_reserve(dir, data.len() as u64, s).unwrap();
    // In 4 KiB pieces, as the responder's buffer delivers them.
    for (i, chunk) in data.chunks(4096).enumerate() {
        v.node_write(&mut r, i as u64 * 4096, chunk, s).unwrap();
    }
    v.node_commit(&r, dir, &u(name), false, (None, None), s)
        .unwrap()
}

fn pattern(n: usize, seed: u8) -> Vec<u8> {
    (0..n)
        .map(|i| (i as u8).wrapping_mul(37) ^ seed ^ (i >> 8) as u8)
        .collect()
}

#[test]
fn unicode_names_round_trip_with_their_short_names() {
    each(|mut v, fs| {
        let s = &mut vec![0u8; 8192];
        let names_in = [
            "Résumé 2026.txt",
            "日本語のファイル.doc",
            "emoji 📄 file",
            "plain.txt",
            "MixedCase.TXT",
            "a b c",
        ];
        for n in names_in {
            v.node_create(ROOT, &u(n), false, false, (None, None), s)
                .unwrap();
        }
        let listed: Vec<String> = names(&mut v, ROOT).into_iter().map(|(n, _)| n).collect();
        for n in names_in {
            assert!(listed.contains(&n.to_string()), "{fs}: {n} in {listed:?}");
        }
        assert!(health(&mut v).clean(), "{fs}");
        // The old ASCII API still finds the ASCII ones (by their names) and
        // the short names exist for every file.
        let mut ascii = Vec::new();
        v.list("", &mut |n, _, _| {
            ascii.push(String::from_utf8_lossy(n).into_owned())
        })
        .unwrap();
        assert!(ascii.contains(&"plain.txt".to_string()), "{fs}: {ascii:?}");
    });
}

#[test]
fn names_are_taken_whatever_their_case() {
    each(|mut v, fs| {
        let s = &mut vec![0u8; 8192];
        v.node_create(ROOT, &u("Café"), true, false, (None, None), s)
            .unwrap();
        for clash in ["café", "CAFÉ", "Café"] {
            assert_eq!(
                v.node_create(ROOT, &u(clash), false, false, (None, None), s),
                Err(Error::Exists),
                "{fs}: {clash}"
            );
        }
        assert!(
            v.node_create(ROOT, &u("Cafe"), false, false, (None, None), s)
                .is_ok(),
            "{fs}"
        );
        for bad in [
            "a/b",
            "a:b",
            "trailing.",
            "trailing ",
            "..",
            "",
            "q?",
            "tab\t",
        ] {
            assert_eq!(
                v.node_create(ROOT, &u(bad), false, false, (None, None), s),
                Err(Error::BadName),
                "{fs}: {bad:?}"
            );
        }
    });
    assert!(same_name(&u("ÀÉÎ"), &u("àéî")));
    assert!(!same_name(&u("a"), &u("b")));
    assert!(valid_name(&[0x41; 255]));
    assert!(!valid_name(&[0x41; 256]));
}

#[test]
fn a_255_unit_name_round_trips() {
    each(|mut v, fs| {
        let s = &mut vec![0u8; 8192];
        let long: String = std::iter::repeat_n('é', 255).collect();
        let loc = v
            .node_create(ROOT, &u(&long), false, false, (None, None), s)
            .unwrap();
        let mut n = Node::default();
        v.node_stat(loc, &mut n).unwrap();
        assert_eq!(String::from_utf16(n.name()).unwrap(), long, "{fs}");
        assert!(health(&mut v).clean(), "{fs}");
    });
}

#[test]
fn cursors_walk_many_entries_across_sectors_and_clusters() {
    each(|mut v, fs| {
        let s = &mut vec![0u8; 8192];
        let dir = v
            .node_create(ROOT, &u("Many"), true, false, (None, None), s)
            .unwrap();
        // Long names: 2-3 entries each, so sets cross sector edges.
        for i in 0..150 {
            v.node_create(
                dir,
                &u(&format!("a file with a long name number {i:03}.txt")),
                false,
                false,
                (None, None),
                s,
            )
            .unwrap();
        }
        let got = names(&mut v, dir);
        assert_eq!(got.len(), 150, "{fs}");
        for (i, (n, loc)) in got.iter().enumerate() {
            assert_eq!(
                n,
                &format!("a file with a long name number {i:03}.txt"),
                "{fs}"
            );
            let mut st = Node::default();
            v.node_stat(*loc, &mut st).unwrap();
            assert_eq!(String::from_utf16(st.name()).unwrap(), *n);
        }
        assert!(health(&mut v).clean(), "{fs}");
    });
}

#[test]
fn a_reserved_file_is_invisible_until_committed() {
    each(|mut v, fs| {
        let s = &mut vec![0u8; 8192];
        let data = pattern(10_000, 3);
        let mut r = v.node_reserve(ROOT, data.len() as u64, s).unwrap();
        assert!(r.contiguous, "{fs}");
        assert!(
            names(&mut v, ROOT).is_empty(),
            "{fs}: the temporary entry isn't listed"
        );
        for (i, c) in data.chunks(4096).enumerate() {
            v.node_write(&mut r, i as u64 * 4096, c, s).unwrap();
        }
        let loc = v
            .node_commit(&r, ROOT, &u("ready.bin"), false, (None, None), s)
            .unwrap();
        let mut n = Node::default();
        v.node_stat(loc, &mut n).unwrap();
        assert_eq!(read_all(&mut v, &n), data, "{fs}");
        assert_eq!(names(&mut v, ROOT).len(), 1);
        let h = health(&mut v);
        assert!(h.clean(), "{fs}: {h:?}");
        // The ASCII API reads it the same.
        let mut back = Vec::new();
        v.read_back("ready.bin", &mut vec![0u8; 4096], &mut |p| {
            back.extend_from_slice(p)
        })
        .unwrap();
        assert_eq!(back, data, "{fs}");
    });
}

#[test]
fn an_abandoned_reservation_gives_its_space_back() {
    each(|mut v, fs| {
        let s = &mut vec![0u8; 8192];
        let (_, free0) = v.node_space(s).unwrap();
        let r = v.node_reserve(ROOT, 50_000, s).unwrap();
        let (_, free1) = v.node_space(s).unwrap();
        assert!(free1 < free0, "{fs}");
        v.node_abandon(&r).unwrap();
        let (_, free2) = v.node_space(s).unwrap();
        assert_eq!(free2, free0, "{fs}");
        assert!(health(&mut v).clean(), "{fs}");
        assert!(names(&mut v, ROOT).is_empty());
    });
}

#[test]
fn no_room_is_refused_before_anything_changes() {
    each(|mut v, fs| {
        let s = &mut vec![0u8; 8192];
        let (cap, _) = v.node_space(s).unwrap();
        let before = v.device().writes;
        assert_eq!(v.node_reserve(ROOT, cap + 1, s), Err(Error::NoRoom), "{fs}");
        assert_eq!(v.device().writes, before, "{fs}: nothing written");
    });
}

#[test]
fn a_power_cut_mid_write_is_cleaned_up_at_the_next_mount() {
    each(|mut v, fs| {
        let s = &mut vec![0u8; 8192];
        let (_, free0) = v.node_space(s).unwrap();
        let mut r = v.node_reserve(ROOT, 9000, s).unwrap();
        v.node_write(&mut r, 0, &pattern(4096, 1), s).unwrap();
        // The power goes: the reservation is never committed.
        let h = health(&mut v);
        assert!(!h.clean() || h.lost == 0, "{fs}");
        assert_eq!(v.node_recover().unwrap(), 1, "{fs}");
        let (_, free1) = v.node_space(s).unwrap();
        assert_eq!(free1, free0, "{fs}");
        assert!(health(&mut v).clean(), "{fs}");
        assert_eq!(v.node_recover().unwrap(), 0);
    });
}

#[test]
fn a_power_cut_mid_commit_keeps_a_named_file_or_frees_an_unnamed_one() {
    each(|mut v, fs| {
        let s = &mut vec![0u8; 8192];
        let data = pattern(5000, 9);
        // Cut after the real entry is written, before the temporary entry
        // goes: the file is kept.
        let mut r = v.node_reserve(ROOT, data.len() as u64, s).unwrap();
        v.node_write(&mut r, 0, &data[..4096], s).unwrap();
        v.node_write(&mut r, 4096, &data[4096..], s).unwrap();
        v.node_commit(&r, ROOT, &u("kept.bin"), false, (None, None), s)
            .unwrap();
        // As if the cut came just before the temporary entry's deletion:
        // put it back (FAT: marked committing; exFAT: its set in use again).
        let (lba, slot) = (((r.temp & !(1 << 63)) >> 4) as u32, (r.temp & 15) as usize);
        let mut sec = [0u8; SECTOR];
        match &mut v {
            AnyVolume::Fat(f) => {
                f.dev.read_blocks(lba, &mut sec).unwrap();
                sec[slot * 32] = b'~';
                f.dev.write_blocks(lba, &sec).unwrap();
            }
            AnyVolume::ExFat(x) => {
                x.dev.read_blocks(lba, &mut sec).unwrap();
                for k in slot..slot + 3 {
                    sec[k * 32] |= 0x80;
                }
                x.dev.write_blocks(lba, &sec).unwrap();
            }
        }
        assert_eq!(v.node_recover().unwrap(), 1, "{fs}");
        let n = find(&mut v, ROOT, "kept.bin");
        assert_eq!(read_all(&mut v, &n), data, "{fs}");
        assert!(health(&mut v).clean(), "{fs}: {:?}", health(&mut v));
    });
}

#[test]
fn deletes_free_space_and_folders_must_be_empty() {
    each(|mut v, fs| {
        let s = &mut vec![0u8; 8192];
        let dir = v
            .node_create(ROOT, &u("Folder"), true, false, (None, None), s)
            .unwrap();
        let f = put(&mut v, dir, "inside.bin", &pattern(7000, 2));
        assert_eq!(v.node_delete(dir), Err(Error::NotEmpty), "{fs}");
        v.node_delete(f).unwrap();
        v.node_delete(dir).unwrap();
        assert!(names(&mut v, ROOT).is_empty(), "{fs}");
        let mut n = Node::default();
        assert_eq!(v.node_stat(f, &mut n), Err(Error::NotFound), "{fs}");
        assert!(health(&mut v).clean(), "{fs}");
    });
}

#[test]
fn renames_keep_the_data_and_dates_and_case_only_changes_work() {
    each(|mut v, fs| {
        let s = &mut vec![0u8; 8192];
        let when = Stamp {
            year: 2026,
            month: 10,
            day: 25,
            hour: 22,
            minute: 30,
            second: 14,
            centis: None,
            zone: None,
        };
        let mut r = v.node_reserve(ROOT, 3000, s).unwrap();
        v.node_write(&mut r, 0, &pattern(3000, 5), s).unwrap();
        let loc = v
            .node_commit(
                &r,
                ROOT,
                &u("short.txt"),
                false,
                (Some(when), Some(when)),
                s,
            )
            .unwrap();
        let new = v
            .node_rename(ROOT, loc, &u("a much longer name for the same file.txt"), s)
            .unwrap();
        let mut n = Node::default();
        v.node_stat(new, &mut n).unwrap();
        assert_eq!(read_all(&mut v, &n), pattern(3000, 5), "{fs}");
        assert_eq!(
            n.modified.map(|m| (m.hour, m.minute, m.second)),
            Some((22, 30, 14)),
            "{fs}"
        );
        let again = v
            .node_rename(ROOT, new, &u("A MUCH LONGER NAME FOR THE SAME FILE.TXT"), s)
            .unwrap();
        v.node_stat(again, &mut n).unwrap();
        assert_eq!(
            String::from_utf16(n.name()).unwrap(),
            "A MUCH LONGER NAME FOR THE SAME FILE.TXT"
        );
        assert_eq!(names(&mut v, ROOT).len(), 1, "{fs}");
        v.node_create(ROOT, &u("other"), false, false, (None, None), s)
            .unwrap();
        assert_eq!(
            v.node_rename(ROOT, again, &u("OTHER"), s),
            Err(Error::Exists),
            "{fs}"
        );
        assert!(health(&mut v).clean(), "{fs}");
    });
}

#[test]
fn moves_rewrite_the_entry_and_folders_follow() {
    each(|mut v, fs| {
        let s = &mut vec![0u8; 8192];
        let a = v
            .node_create(ROOT, &u("A"), true, false, (None, None), s)
            .unwrap();
        let b = v
            .node_create(ROOT, &u("B"), true, false, (None, None), s)
            .unwrap();
        let f = put(&mut v, a, "file.bin", &pattern(4000, 7));
        let moved = v.node_move(a, f, b, s).unwrap();
        assert!(names(&mut v, a).is_empty(), "{fs}");
        let mut n = Node::default();
        v.node_stat(moved, &mut n).unwrap();
        assert_eq!(read_all(&mut v, &n), pattern(4000, 7));
        // A folder into another: its `..` follows (the check walks it).
        let a2 = v.node_move(ROOT, a, b, s).unwrap();
        assert!(health(&mut v).clean(), "{fs}: {:?}", health(&mut v));
        // Into itself or below: refused.
        assert_eq!(v.node_move(ROOT, b, a2, s), Err(Error::Exists), "{fs}");
        assert_eq!(v.node_move(ROOT, b, b, s), Err(Error::Exists), "{fs}");
        // From the wrong folder: refused.
        assert_eq!(
            v.node_move(ROOT, moved, ROOT, s),
            Err(Error::NotFound),
            "{fs}"
        );
        let in_b: Vec<String> = names(&mut v, b).into_iter().map(|(n, _)| n).collect();
        assert_eq!(in_b, ["file.bin", "A"], "{fs}");
        let mut back = Vec::new();
        v.read_back("B/file.bin", &mut vec![0u8; 4096], &mut |p| {
            back.extend_from_slice(p)
        })
        .unwrap();
        assert_eq!(back, pattern(4000, 7), "{fs}: the ASCII API agrees");
    });
}

#[test]
fn a_fragmented_card_still_reserves_and_reads_back() {
    for (mut v, fs, cb) in [
        (any(Fs::Fat32, 131_072, 512), "FAT32", 512),
        (any(Fs::ExFat, 65_536, 4096), "exFAT", 4096),
    ] {
        let s = &mut vec![0u8; 8192];
        // Every other one-cluster file deleted: free space in one-cluster
        // holes only, once the rest is filled.
        let mut locs = Vec::new();
        for i in 0..40 {
            locs.push(put(&mut v, ROOT, &format!("f{i}"), &pattern(cb, i as u8)));
        }
        for l in locs.iter().step_by(2) {
            v.node_delete(*l).unwrap();
        }
        let (_, free) = v.node_space(s).unwrap();
        put(
            &mut v,
            ROOT,
            "filler",
            &vec![0u8; (free - 20 * cb as u64) as usize],
        );
        let data = pattern(10 * cb, 77);
        let mut r = v.node_reserve(ROOT, data.len() as u64, s).unwrap();
        assert!(!r.contiguous, "{fs}");
        for (i, c) in data.chunks(4096).enumerate() {
            v.node_write(&mut r, i as u64 * 4096, c, s).unwrap();
        }
        let loc = v
            .node_commit(&r, ROOT, &u("chained.bin"), false, (None, None), s)
            .unwrap();
        let mut n = Node::default();
        v.node_stat(loc, &mut n).unwrap();
        assert_eq!(read_all(&mut v, &n), data, "{fs}");
        assert!(health(&mut v).clean(), "{fs}: {:?}", health(&mut v));
    }
}

#[test]
fn hostile_locations_are_refused() {
    each(|mut v, fs| {
        let s = &mut vec![0u8; 8192];
        let loc = v
            .node_create(
                ROOT,
                &u("a long name needing entries.txt"),
                false,
                false,
                (None, None),
                s,
            )
            .unwrap();
        let mut n = Node::default();
        v.node_stat(loc, &mut n).unwrap();
        // The set's middle, past the volume, inside the FAT, the boot
        // sector, a deleted set. (A set's short entry alone reads as its
        // short name: locations come from this layer's own walks, never
        // from the PC, which only ever sends the responder's handles.)
        for bad in [
            loc + 1,
            0x000F_FFFF_FFF0,
            16,
            1 << 4,
            u64::from(u32::MAX) << 4,
        ] {
            assert_eq!(
                v.node_stat(bad, &mut n),
                Err(Error::NotFound),
                "{fs}: {bad:#x}"
            );
        }
        let mut c = 0xDEAD_BEEF_u64;
        let _ = v.node_child(ROOT, &mut c, &mut n);
        v.node_delete(loc).unwrap();
        assert_eq!(v.node_stat(loc, &mut n), Err(Error::NotFound), "{fs}");
        let mut c = 0;
        assert_eq!(
            v.node_child(loc, &mut c, &mut n),
            Err(Error::NotFound),
            "{fs}: a deleted folder"
        );
    });
}

#[test]
fn the_fixed_root_fills_and_says_so() {
    let mut v = any(Fs::Fat16, 65_536, 2048);
    let s = &mut vec![0u8; 8192];
    let mut made = 0;
    loop {
        match v.node_create(ROOT, &u(&format!("F{made}")), false, false, (None, None), s) {
            Ok(_) => made += 1,
            Err(Error::NoRoom) => break,
            Err(e) => panic!("{e:?}"),
        }
        assert!(made < 1000);
    }
    assert!(made >= 500, "{made}");
    // A reservation needs a temporary entry in the root: refused, nothing kept.
    let (_, free0) = v.node_space(s).unwrap();
    assert_eq!(v.node_reserve(ROOT, 100, s), Err(Error::NoRoom));
    assert_eq!(v.node_space(s).unwrap().1, free0);
    assert!(health(&mut v).clean());
}

#[test]
fn exfat_name_hashes_are_over_the_up_cased_name() {
    let mut v = any(Fs::ExFat, 65_536, 4096);
    let s = &mut vec![0u8; 8192];
    for name in ["Ärger über Öl.txt", "plain name.TXT", "ελληνικά"] {
        let loc = v
            .node_create(ROOT, &u(name), false, false, (None, None), s)
            .unwrap();
        // The stream extension follows the file entry; its hash (bytes 4-5)
        // is over the up-cased name's units, low byte first (exFAT 7.6.4).
        let (lba, slot) = (((loc & !(1 << 63)) >> 4) as u32, (loc & 15) as usize);
        let AnyVolume::ExFat(x) = &mut v else {
            unreachable!()
        };
        let mut two = [0u8; 2 * SECTOR];
        x.dev.read_blocks(lba, &mut two).unwrap();
        let stream = &two[(slot + 1) * 32..(slot + 2) * 32];
        assert_eq!(stream[0], 0xC0);
        let mut want = 0u16;
        for c in name.to_uppercase().encode_utf16() {
            for b in c.to_le_bytes() {
                want = want.rotate_right(1).wrapping_add(u16::from(b));
            }
        }
        assert_eq!(u16::from_le_bytes([stream[4], stream[5]]), want, "{name}");
    }
}

#[test]
fn a_folder_with_a_long_name_grows_on_every_format() {
    each(|mut v, fs| {
        let s = &mut vec![0u8; 8192];
        let long: String = std::iter::repeat_n('d', 200).collect();
        let dir = v
            .node_create(ROOT, &u(&long), true, false, (None, None), s)
            .unwrap();
        // Enough long-named files to fill several clusters of the folder.
        for i in 0..120 {
            v.node_create(
                dir,
                &u(&format!(
                    "{i:03} a file name long enough to need entries.txt"
                )),
                false,
                false,
                (None, None),
                s,
            )
            .unwrap_or_else(|e| panic!("{fs}: file {i}: {e:?}"));
        }
        assert_eq!(names(&mut v, dir).len(), 120, "{fs}");
        let h = health(&mut v);
        assert!(h.clean(), "{fs}: {h:?}");
    });
}
