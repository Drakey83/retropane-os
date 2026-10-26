use std::collections::BTreeMap;

use rp_hal_api::{BlockDevice, BlockError};

use super::*;
use crate::exfat::ExVolume;
use crate::{AnyVolume, Volume};

/// A card of any size in memory: only sectors that aren't all zero are
/// kept, so a 2 TiB format costs a few MB. It can be made to fail after a
/// number of writes (power lost).
struct Sparse {
    total: u32,
    sectors: BTreeMap<u32, [u8; SECTOR]>,
    writes: usize,
    fail_at: Option<usize>,
}

impl Sparse {
    fn new(total: u32) -> Self {
        Self {
            total,
            sectors: BTreeMap::new(),
            writes: 0,
            fail_at: None,
        }
    }

    fn sector(&self, lba: u32) -> [u8; SECTOR] {
        self.sectors.get(&lba).copied().unwrap_or([0; SECTOR])
    }
}

impl BlockDevice for Sparse {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
        assert_eq!(buf.len() % SECTOR, 0);
        let n = (buf.len() / SECTOR) as u32;
        if lba.checked_add(n).is_none_or(|end| end > self.total) {
            return Err(BlockError::OutOfRange);
        }
        for (i, chunk) in buf.chunks_mut(SECTOR).enumerate() {
            chunk.copy_from_slice(&self.sector(lba + i as u32));
        }
        Ok(())
    }

    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
        assert_eq!(buf.len() % SECTOR, 0);
        let n = (buf.len() / SECTOR) as u32;
        if lba.checked_add(n).is_none_or(|end| end > self.total) {
            return Err(BlockError::OutOfRange);
        }
        if self.fail_at == Some(self.writes) {
            return Err(BlockError::Device);
        }
        self.writes += 1;
        for (i, chunk) in buf.chunks(SECTOR).enumerate() {
            let at = lba + i as u32;
            if chunk.iter().all(|&b| b == 0) {
                self.sectors.remove(&at);
            } else {
                self.sectors.insert(at, chunk.try_into().unwrap());
            }
        }
        Ok(())
    }
}

const MIB: u32 = 2048; // sectors

fn opts(fs: Option<Fs>) -> Options<'static> {
    Options {
        fs,
        cluster_bytes: None,
        align_sectors: None,
        label: "RETROPANE",
        serial: 0x2026_1025,
    }
}

fn format(total: u32, o: &Options<'_>) -> (Sparse, Layout) {
    let l = plan(total, o).unwrap_or_else(|e| panic!("{total} sectors: {}", e.name()));
    let mut d = Sparse::new(total);
    write(&mut d, &l, &mut vec![0u8; 16 * 1024]).unwrap();
    (d, l)
}

fn pattern(len: usize, seed: u32) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u32).wrapping_mul(2_654_435_761).wrapping_add(seed) as u8 ^ (i >> 9) as u8)
        .collect()
}

/// Puts files of sizes around cluster edges and in a folder, reads them
/// back, checks the volume and removes one: every format must do all of it.
fn round_trip<D: BlockDevice>(v: &mut AnyVolume<D>, cluster: usize) {
    let mut scratch = vec![0u8; 8192];
    let sizes = [0, 1, cluster - 1, cluster, cluster + 1, 3 * cluster + 17];
    for (i, &n) in sizes.iter().enumerate() {
        let name = format!("file {i} with a long name.bin");
        v.put(&name, &pattern(n, i as u32), &mut scratch).unwrap();
    }
    v.put("Sub/inner/deep.txt", b"in a folder", &mut scratch)
        .unwrap();
    for (i, &n) in sizes.iter().enumerate() {
        let name = format!("file {i} with a long name.bin");
        let (got, _) = read(v, &name);
        assert_eq!(got, pattern(n, i as u32), "{name}");
    }
    assert_eq!(read(v, "sub/INNER/deep.txt").0, b"in a folder");
    v.remove("file 2 with a long name.bin").unwrap();
    let mut seen = vec![0u8; 1 << 20];
    let h = v.check(&mut seen, &mut scratch).unwrap();
    assert!(h.clean(), "{h:?}");
    assert_eq!(h.lost, 0);
}

fn read<D: BlockDevice>(v: &mut AnyVolume<D>, name: &str) -> (Vec<u8>, ()) {
    let mut out = Vec::new();
    let mut scratch = vec![0u8; 4096];
    v.read_back(name, &mut scratch, &mut |c| out.extend_from_slice(c))
        .unwrap();
    (out, ())
}

fn kind_of<D: BlockDevice>(v: &AnyVolume<D>) -> Fs {
    match v {
        AnyVolume::Fat(v) => match v.kind() {
            crate::FatKind::Fat12 => Fs::Fat12,
            crate::FatKind::Fat16 => Fs::Fat16,
            crate::FatKind::Fat32 => Fs::Fat32,
        },
        AnyVolume::ExFat(_) => Fs::ExFat,
    }
}

#[test]
fn the_up_case_table_is_the_specifications_byte_for_byte() {
    let words: Vec<u16> = Upcase::new().collect();
    assert_eq!(words.len() * 2, UPCASE_BYTES as usize);
    let sum = words
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .fold(0u32, |s, b| s.rotate_right(1).wrapping_add(u32::from(b)));
    assert_eq!(sum, UPCASE_CHECKSUM);
    // Table 24: the mandatory first 128 (stored uncompressed).
    for c in 0..128u16 {
        let up = if (0x61..=0x7A).contains(&c) {
            c - 32
        } else {
            c
        };
        assert_eq!(words[c as usize], up, "{c:#x}");
    }
    // Expanded, it maps all 65,536 characters; a few known pairs.
    let mut map = Vec::new();
    let mut i = 0;
    while i < words.len() {
        if words[i] == 0xFFFF && i + 1 < words.len() {
            let start = map.len() as u16;
            map.extend((0..words[i + 1]).map(|k| start + k));
            i += 2;
        } else {
            map.push(words[i]);
            i += 1;
        }
    }
    assert_eq!(map.len(), 65_536);
    for (lower, upper) in [
        (0xE9, 0xC9),
        (0x3B1, 0x391),
        (0x430, 0x410),
        (0xFF41, 0xFF21),
    ] {
        assert_eq!(map[lower], upper);
    }
    assert_eq!(map[0x41], 0x41);
}

#[test]
fn the_default_format_follows_the_card_size() {
    for (mib, fs, cluster) in [
        (4u64, Fs::Fat12, 8 << 10),
        (32, Fs::Fat12, 16 << 10),
        (64, Fs::Fat12, 16 << 10),
        (65, Fs::Fat16, 16 << 10),
        (512, Fs::Fat16, 16 << 10),
        (2048, Fs::Fat16, 32 << 10),
        (4096, Fs::Fat32, 32 << 10),
        (32 * 1024, Fs::Fat32, 32 << 10),
        (64 * 1024, Fs::ExFat, 32 << 10),
        (1024 * 1024, Fs::ExFat, 128 << 10),
    ] {
        let total = u32::try_from(mib * 2048).unwrap_or(u32::MAX);
        let l = plan(total, &opts(None)).unwrap();
        assert_eq!((l.fs, l.cluster_bytes()), (fs, cluster), "{mib} MiB");
    }
}

#[test]
fn every_format_mounts_writes_and_checks_clean() {
    let cases: [(u32, Option<Fs>, Option<u32>); 10] = [
        (4 * MIB, None, None),                     // FAT12, 8 KiB
        (3 * MIB / 2, Some(Fs::Fat12), Some(512)), // FAT12, sector clusters
        (64 * MIB, None, None),                    // FAT12, 16 KiB
        (300 * MIB, Some(Fs::Fat16), Some(8192)),  // FAT16, 8 KiB
        (512 * MIB, Some(Fs::Fat32), None),        // FAT32 on a small card
        (4096 * MIB, None, None),                  // FAT32, 32 KiB
        (8 * MIB, Some(Fs::ExFat), Some(4096)),    // exFAT, small
        (64 * MIB, Some(Fs::ExFat), None),         // exFAT
        (65_536 * MIB, None, None),                // exFAT on SDXC
        (2 * MIB, Some(Fs::ExFat), Some(512)),     // exFAT near its minimum
    ];
    for (total, fs, cluster) in cases {
        let o = Options {
            cluster_bytes: cluster,
            ..opts(fs)
        };
        let (d, l) = format(total, &o);
        let mut v = AnyVolume::mount(d, l.part_start).unwrap();
        assert_eq!(kind_of(&v), l.fs, "{total} sectors");
        round_trip(&mut v, l.cluster_bytes() as usize);
    }
}

#[test]
fn the_layout_is_aligned_to_the_erase_unit() {
    for total in [
        3 * MIB,
        64 * MIB,
        1000 * MIB + 7,
        4096 * MIB + 123,
        70_000 * MIB,
    ] {
        for fs in [None, Some(Fs::ExFat)] {
            let Ok(l) = plan(total, &opts(fs)) else {
                continue;
            };
            assert!(l.align.is_power_of_two());
            assert_eq!(l.part_start % l.align, 0, "{total} {:?}", l.fs);
            assert_eq!(
                (l.part_start + l.data_offset) % l.align,
                0,
                "{total} {:?}",
                l.fs
            );
            assert!(l.align >= l.sectors_per_cluster);
            assert!(l.align <= 8192);
            // Everything inside the card.
            let data_end = l.data_offset + l.clusters * l.sectors_per_cluster;
            assert!(data_end <= l.part_sectors);
        }
    }
}

#[test]
fn the_fat_type_at_the_cluster_count_edges_is_what_mount_decides() {
    // Card sizes whose cluster count lands on each side of FAT12/16's
    // limit (4,085) and FAT16/32's (65,525), with sector-sized clusters:
    // the formatter's choice and the reader's must agree on every one.
    let one = |total: u32, fs: Fs| {
        let o = Options {
            cluster_bytes: Some(512),
            align_sectors: Some(1),
            ..opts(Some(fs))
        };
        plan(total, &o).ok().map(|l| {
            let mut d = Sparse::new(total);
            write(&mut d, &l, &mut [0u8; 4096]).unwrap();
            let v = AnyVolume::mount(d, l.part_start).unwrap();
            assert_eq!(kind_of(&v), fs, "{total}: {} clusters", l.clusters);
            l.clusters
        })
    };
    let mut seen = [false; 4];
    for total in 4_100..4_200 {
        for (fs, got) in [
            (Fs::Fat12, one(total, Fs::Fat12)),
            (Fs::Fat16, one(total, Fs::Fat16)),
        ] {
            match (fs, got) {
                (Fs::Fat12, Some(4084)) => seen[0] = true,
                (Fs::Fat16, Some(4085)) => seen[1] = true,
                _ => {}
            }
        }
    }
    for total in 65_900..66_700 {
        for (fs, got) in [
            (Fs::Fat16, one(total, Fs::Fat16)),
            (Fs::Fat32, one(total, Fs::Fat32)),
        ] {
            match (fs, got) {
                (Fs::Fat16, Some(65_524)) => seen[2] = true,
                (Fs::Fat32, Some(65_525)) => seen[3] = true,
                _ => {}
            }
        }
    }
    assert_eq!(seen, [true; 4], "every edge reached");
}

#[test]
fn choices_that_cannot_work_are_refused_before_anything_is_written() {
    let bad = |total: u32, o: Options<'_>| plan(total, &o).unwrap_err();
    assert_eq!(
        bad(4096 * MIB, opts(Some(Fs::Fat12))),
        PlanError::ClusterCount
    );
    assert_eq!(
        bad(16 * MIB, opts(Some(Fs::Fat32))),
        PlanError::ClusterCount
    );
    assert_eq!(bad(MIB / 2, opts(Some(Fs::ExFat))), PlanError::TooSmall);
    assert_eq!(bad(16, opts(None)), PlanError::TooSmall);
    for label in ["TWELVE CHARS", "a.b", "star*", "tab\t", "é"] {
        assert_eq!(
            bad(
                64 * MIB,
                Options {
                    label,
                    ..opts(None)
                }
            ),
            PlanError::Label
        );
    }
    for cluster in [0, 100, 256, 3 << 10] {
        let o = Options {
            cluster_bytes: Some(cluster),
            ..opts(None)
        };
        assert_eq!(bad(64 * MIB, o), PlanError::ClusterSize, "{cluster}");
    }
    // FAT can't have 128 KiB clusters; exFAT can.
    let big = Some(128 << 10);
    let o = Options {
        cluster_bytes: big,
        ..opts(Some(Fs::Fat32))
    };
    assert_eq!(bad(64 * 1024 * MIB, o), PlanError::ClusterSize);
    let o = Options {
        cluster_bytes: big,
        ..opts(Some(Fs::ExFat))
    };
    assert!(plan(64 * 1024 * MIB, &o).is_ok());
    for align in [0, 3, 6000] {
        let o = Options {
            align_sectors: Some(align),
            ..opts(None)
        };
        assert_eq!(bad(64 * MIB, o), PlanError::Alignment, "{align}");
    }
    // An empty label is fine (FAT's boot sector says NO NAME).
    assert!(
        plan(
            64 * MIB,
            &Options {
                label: "",
                ..opts(None)
            }
        )
        .is_ok()
    );
}

#[test]
fn a_cut_format_leaves_no_mountable_volume_until_the_last_write() {
    for fs in [Fs::Fat12, Fs::Fat16, Fs::Fat32, Fs::ExFat] {
        // A card that held a FAT32 volume at the old 2048-sector start.
        let total = if fs == Fs::Fat32 { 600 * MIB } else { 40 * MIB };
        let old = plan(
            total,
            &Options {
                align_sectors: Some(2048),
                ..opts(Some(Fs::Fat32))
            },
        );
        let mut base = Sparse::new(total);
        if let Ok(old) = old {
            write(&mut base, &old, &mut [0u8; 8192]).unwrap();
        } else {
            let old = plan(
                total,
                &Options {
                    align_sectors: Some(2048),
                    ..opts(Some(Fs::Fat16))
                },
            )
            .unwrap();
            write(&mut base, &old, &mut [0u8; 8192]).unwrap();
        }
        assert!(AnyVolume::mount(&mut base, 2048).is_ok());
        let l = plan(total, &opts(Some(fs))).unwrap();
        let mut full = Sparse::new(total);
        full.sectors = base.sectors.clone();
        write(&mut full, &l, &mut [0u8; 8192]).unwrap();
        let writes = full.writes;
        // Cut at the first write: nothing changed, the old volume whole.
        let mut d = Sparse::new(total);
        d.sectors = base.sectors.clone();
        d.fail_at = Some(0);
        assert!(write(&mut d, &l, &mut [0u8; 8192]).is_err());
        assert_eq!(d.sectors, base.sectors);
        for k in 1..writes {
            let mut d = Sparse::new(total);
            d.sectors = base.sectors.clone();
            d.fail_at = Some(k);
            assert!(write(&mut d, &l, &mut [0u8; 8192]).is_err());
            d.fail_at = None;
            for start in [2048, l.part_start] {
                assert!(
                    AnyVolume::mount(&mut d, start).is_err(),
                    "{:?}: mountable at {start} after {k} of {writes} writes",
                    fs
                );
            }
        }
        assert!(AnyVolume::mount(&mut full, l.part_start).is_ok());
    }
}

#[test]
fn fat12_and_fat16_fill_to_the_last_cluster_and_read_back() {
    // Sector-sized clusters so a FAT12 card has many entries straddling a
    // sector boundary, filled with files of every few sizes, half removed,
    // filled again (scattered), every file read back, the check clean.
    for (total, fs) in [(2 * MIB, Fs::Fat12), (5 * MIB, Fs::Fat16)] {
        let o = Options {
            cluster_bytes: Some(512),
            ..opts(Some(fs))
        };
        let (d, l) = format(total, &o);
        let mut v = Volume::mount(d, l.part_start).unwrap();
        assert_eq!(v.clusters(), l.clusters);
        let mut scratch = vec![0u8; 4096];
        let mut files = Vec::new();
        v.mkdir("data", &mut scratch).unwrap();
        let mut i = 0u32;
        loop {
            let n = 1 + (i as usize * 7919) % 2600;
            let name = format!("data/f{i}.bin");
            match v.create(&name, &pattern(n, i), &mut scratch) {
                Ok(_) => files.push((name, n, i)),
                Err(crate::Error::NoRoom) => break,
                Err(e) => panic!("{e:?}"),
            }
            i += 1;
        }
        assert!(files.len() > 100);
        let _ = &mut files;
        for (k, (name, ..)) in files.iter().enumerate() {
            if k % 2 == 0 {
                v.remove(name).unwrap();
            }
        }
        files = files
            .into_iter()
            .enumerate()
            .filter(|(k, _)| k % 2 == 1)
            .map(|(_, f)| f)
            .collect();
        loop {
            let n = 600 + (i as usize * 104_729) % 4000;
            let name = format!("data/g{i}.bin");
            match v.create(&name, &pattern(n, i), &mut scratch) {
                Ok(_) => files.push((name, n, i)),
                Err(crate::Error::NoRoom) => break,
                Err(e) => panic!("{e:?}"),
            }
            i += 1;
        }
        // The last free clusters, one-byte files, until none is left.
        loop {
            let name = format!("data/h{i}.bin");
            match v.create(&name, &pattern(1, i), &mut scratch) {
                Ok(_) => files.push((name, 1, i)),
                Err(crate::Error::NoRoom) => break,
                Err(e) => panic!("{e:?}"),
            }
            i += 1;
        }
        for (name, n, seed) in &files {
            let f = v.find(name).unwrap();
            let mut got = Vec::new();
            v.read_file(&f, &mut scratch, |c| got.extend_from_slice(c))
                .unwrap();
            assert_eq!(&got, &pattern(*n, *seed), "{name}");
        }
        let mut seen = vec![0u8; 1 << 16];
        let h = v.check(&mut seen, &mut scratch).unwrap();
        assert!(h.clean(), "{fs:?}: {h:?}");
        assert_eq!(h.lost, 0);
        assert_eq!(h.used, l.clusters, "{fs:?}: every cluster in use");
    }
}

#[test]
fn the_fixed_root_holds_its_entries_and_never_grows() {
    let o = Options {
        cluster_bytes: Some(2048),
        ..opts(Some(Fs::Fat12))
    };
    let (d, l) = format(4 * MIB, &o);
    let mut v = Volume::mount(d, l.part_start).unwrap();
    let mut scratch = vec![0u8; 4096];
    // 512 entries, one the label: 511 short-named files fit, no more.
    for i in 0..511 {
        v.create(&format!("F{i}.TXT"), b"x", &mut scratch).unwrap();
    }
    assert_eq!(
        v.create("ONEMORE.TXT", b"x", &mut scratch),
        Err(crate::Error::NoRoom)
    );
    // ... while a folder grows as on FAT32.
    v.remove("F0.TXT").unwrap();
    v.mkdir("FOLDER", &mut scratch).unwrap();
    for i in 0..200 {
        v.create(
            &format!("Folder/a long name number {i}.txt"),
            b"y",
            &mut scratch,
        )
        .unwrap();
    }
    let mut n = 0;
    v.for_each_in("folder", |_| n += 1).unwrap();
    assert_eq!(n, 200);
    // `..` of a top-level folder names the root as cluster 0.
    let mut up = None;
    let dir = v.resolve_dir("Folder").unwrap();
    let kind = v.kind();
    v.walk_dir(dir, &mut |_, _, e, _| {
        if &e[..11] == b"..         " {
            up = Some(crate::entry_first(e, kind));
        }
        false
    })
    .unwrap();
    assert_eq!(up, Some(0));
    let mut seen = vec![0u8; 4096];
    assert!(v.check(&mut seen, &mut scratch).unwrap().clean());
}

#[test]
fn exfat_formats_are_read_by_the_exfat_reader_with_the_label_and_tables() {
    let (d, l) = format(64 * MIB, &opts(Some(Fs::ExFat)));
    // The boot region's checksum and both copies.
    for base in [l.part_start, l.part_start + 12] {
        let mut sum = 0u32;
        for i in 0..11 {
            sum = crate::exfat::boot_checksum(sum, i, &d.sector(base + i as u32));
        }
        let check = d.sector(base + 11);
        assert!(
            check
                .as_chunks::<4>()
                .0
                .iter()
                .all(|w| u32::from_le_bytes(*w) == sum)
        );
    }
    let mut v = ExVolume::mount(d, l.part_start).unwrap();
    let mut scratch = vec![0u8; 8192];
    v.put("hello.txt", b"exFAT from RetroPane", &mut scratch)
        .unwrap();
    let f = v.find("HELLO.TXT").unwrap();
    let mut got = Vec::new();
    v.read_file(&f, &mut scratch, |c| got.extend_from_slice(c))
        .unwrap();
    assert_eq!(got, b"exFAT from RetroPane");
    let mut seen = vec![0u8; 1 << 16];
    let h = v.check(&mut seen, &mut scratch).unwrap();
    assert!(h.clean(), "{h:?}");
    let d = v.into_inner();
    // The partition table: one exFAT partition where the layout says.
    let mbr = d.sector(0);
    assert_eq!(mbr[446 + 4], 0x07);
    assert_eq!(
        u32::from_le_bytes(mbr[454..458].try_into().unwrap()),
        l.part_start
    );
    assert_eq!(&mbr[510..], &[0x55, 0xAA]);
}

#[test]
fn fat_boot_sectors_carry_the_label_serial_and_partition_type() {
    for (total, fs, ptype) in [
        (4 * MIB, Fs::Fat12, 0x01),
        (20 * MIB, Fs::Fat16, 0x04),
        (64 * MIB, Fs::Fat16, 0x06),
        (4096 * MIB, Fs::Fat32, 0x0C),
    ] {
        let (d, l) = format(total, &opts(Some(fs)));
        let b = d.sector(l.part_start);
        let ext = if fs == Fs::Fat32 { 64 } else { 36 };
        assert_eq!(&b[ext + 7..ext + 18], b"RETROPANE  ");
        assert_eq!(
            u32::from_le_bytes(b[ext + 3..ext + 7].try_into().unwrap()),
            0x2026_1025
        );
        assert_eq!(&b[ext + 18..ext + 23], fs.name().as_bytes());
        assert_eq!(d.sector(0)[446 + 4], ptype, "{fs:?}");
        if fs == Fs::Fat32 {
            assert_eq!(d.sector(l.part_start + 6), b, "the backup boot sector");
        }
        // The label as the root's first entry too.
        let mut v = Volume::mount(d, l.part_start).unwrap();
        let mut label = None;
        let root = v.root_cluster;
        v.walk_dir(root, &mut |_, _, e, _| {
            if e[11] == 0x08 {
                label = Some(e[..11].to_vec());
            }
            false
        })
        .unwrap();
        assert_eq!(label.as_deref(), Some(&b"RETROPANE  "[..]));
    }
}

#[test]
fn names_parse_both_ways() {
    for fs in [Fs::Fat12, Fs::Fat16, Fs::Fat32, Fs::ExFat] {
        assert_eq!(Fs::parse(fs.name()), Some(fs));
        assert_eq!(Fs::parse(&fs.name().to_lowercase()), Some(fs));
    }
    assert_eq!(Fs::parse("ntfs"), None);
}

#[test]
fn a_survey_reads_back_every_layout_the_formatter_writes() {
    for (total, fs) in [
        (4 * MIB, Fs::Fat12),
        (64 * MIB, Fs::Fat16),
        (4096 * MIB, Fs::Fat32),
        (64 * MIB, Fs::ExFat),
    ] {
        let (mut d, l) = format(total, &opts(Some(fs)));
        let s = crate::survey::survey(&mut d).unwrap();
        assert_eq!(s.fs, Some(fs));
        assert_eq!(
            (s.part_start, s.part_sectors),
            (l.part_start, l.part_sectors)
        );
        assert_eq!(s.part_type, d.sector(0)[446 + 4]);
        assert_eq!(s.cluster_bytes, l.cluster_bytes(), "{fs:?}");
        assert_eq!(
            (s.fat_offset, s.fat_sectors, s.fats),
            (l.fat_offset, l.fat_sectors, l.fats)
        );
        assert_eq!(
            (s.data_offset, s.clusters),
            (l.data_offset, l.clusters),
            "{fs:?}"
        );
        assert!(s.data_alignment() >= l.align, "{fs:?}");
        if fs != Fs::ExFat {
            assert_eq!(&s.label, b"RETROPANE  ");
            assert_eq!(&s.oem, b"MSWIN4.1");
        }
    }
    // No MBR: the boot sector at sector 0 ("superfloppy").
    let (d, l) = format(64 * MIB, &opts(Some(Fs::Fat16)));
    let mut flat = Sparse::new(d.total);
    let boot = d.sector(l.part_start);
    flat.write_blocks(0, &boot).unwrap();
    let s = crate::survey::survey(&mut flat).unwrap();
    assert_eq!((s.part_type, s.part_start, s.fs), (0, 0, Some(Fs::Fat16)));
    // Nothing there: an error, no panic.
    assert!(crate::survey::survey(&mut Sparse::new(4096)).is_err());
}

#[test]
fn a_range_of_any_file_reads_exactly_those_bytes_on_every_format() {
    for (total, fs) in [
        (2 * MIB, Fs::Fat12),
        (20 * MIB, Fs::Fat16),
        (600 * MIB, Fs::Fat32),
        (8 * MIB, Fs::ExFat),
    ] {
        let o = Options {
            cluster_bytes: Some(1024),
            ..opts(Some(fs))
        };
        let (d, l) = format(total, &o);
        let mut v = AnyVolume::mount(d, l.part_start).unwrap();
        let mut scratch = vec![0u8; 4096];
        // A contiguous file, and one scattered over one-cluster holes.
        let whole = pattern(20_000, 9);
        v.put("whole.bin", &whole, &mut scratch).unwrap();
        for i in 0..40 {
            v.put(&format!("h/{i}"), &pattern(1000, i), &mut scratch)
                .unwrap();
        }
        // Fill everything else (halving chunks until not one cluster fits
        // outside the holes), so the next file can only go into them.
        let filler = pattern(total as usize * 512, 3);
        let (mut size, mut k) = (filler.len(), 0);
        while size >= 1024 {
            if v.put(&format!("f/{k}"), &filler[..size], &mut scratch)
                .is_ok()
            {
                k += 1;
            } else {
                size /= 2;
            }
        }
        // Then the holes: every other small file, one cluster each.
        for i in (0..40).step_by(2) {
            v.remove(&format!("h/{i}")).unwrap();
        }
        let scattered = pattern(18_000, 5);
        v.put("scattered.bin", &scattered, &mut scratch).unwrap();
        assert_eq!(
            v.extent("scattered.bin").err(),
            Some(crate::Error::Fragmented),
            "{fs:?}: the file is scattered"
        );
        for (name, data) in [("whole.bin", &whole), ("scattered.bin", &scattered)] {
            let size = data.len() as u64;
            for (start, len) in [
                (0, size),
                (0, 1),
                (1, 1),
                (1023, 2),
                (1024, 1024),
                (3 * 1024 + 5, 10_000),
                (511, 514),
                (size - 1, 10),
                (size, 5),
                (size + 1000, 5),
                (100, 0),
                (0, u64::MAX),
            ] {
                let mut got = Vec::new();
                let (s, n) = v
                    .read_range(name, start, len, &mut scratch, &mut |c| {
                        got.extend_from_slice(c)
                    })
                    .unwrap();
                let a = start.min(size) as usize;
                let b = start.saturating_add(len).min(size) as usize;
                assert_eq!(s, size);
                assert_eq!(n as usize, b - a, "{fs:?} {name} {start}+{len}");
                assert_eq!(got, data[a..b], "{fs:?} {name} {start}+{len}");
            }
            // The whole range is the plain read.
            let mut plain = Vec::new();
            v.read_back(name, &mut scratch, &mut |c| plain.extend_from_slice(c))
                .unwrap();
            assert_eq!(&plain, data);
        }
    }
}
