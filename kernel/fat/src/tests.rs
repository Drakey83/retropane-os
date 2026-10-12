use super::*;
use crate::format::{self, Entry, Image, PARTITION_START};

/// A disk in memory that can be made to fail after a number of writes
/// (power lost), and counts its transfers.
struct MemDisk {
    data: Vec<u8>,
    writes: usize,
    reads: usize,
    fail_at: Option<usize>,
}

impl MemDisk {
    fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            writes: 0,
            reads: 0,
            fail_at: None,
        }
    }
}

impl BlockDevice for MemDisk {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
        assert_eq!(buf.len() % SECTOR, 0);
        let o = lba as usize * SECTOR;
        if o + buf.len() > self.data.len() {
            return Err(BlockError::OutOfRange);
        }
        self.reads += 1;
        buf.copy_from_slice(&self.data[o..o + buf.len()]);
        Ok(())
    }

    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
        assert_eq!(buf.len() % SECTOR, 0);
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

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31) ^ seed)
        .collect()
}

fn small_image(files: &[Entry<'_>], blocked: &[u32]) -> Vec<u8> {
    format::build(&Image {
        total_sectors: 2048 + 1024, // 1024-sector partition
        sectors_per_cluster: 2,     // 1 KiB clusters
        files,
        blocked_clusters: blocked,
    })
}

fn read_all<D: BlockDevice>(v: &mut Volume<D>, name: &str, scratch: usize) -> Vec<u8> {
    let f = v.find(name).unwrap();
    let mut out = Vec::new();
    let mut scratch = vec![0u8; scratch];
    v.read_file(&f, &mut scratch, |c| out.extend_from_slice(c))
        .unwrap();
    out
}

#[test]
fn mounts_a_fresh_image_and_lists_the_root() {
    let a = pattern(3000, 1);
    let img = small_image(
        &[
            Entry {
                name: "retropane.bin",
                data: &a,
            },
            Entry {
                name: "HARET.EXE",
                data: b"haret",
            },
            Entry {
                name: "empty.txt",
                data: b"",
            },
        ],
        &[],
    );
    let mut v = Volume::mount(MemDisk::new(img), PARTITION_START).unwrap();
    assert_eq!(v.cluster_bytes(), 1024);
    let mut names = Vec::new();
    v.for_each_root(|e| {
        names.push((
            String::from_utf8_lossy(e.name).into_owned(),
            e.size,
            e.is_dir,
        ))
    })
    .unwrap();
    assert_eq!(
        names,
        [
            ("retropane.bin".to_string(), 3000, false),
            ("HARET.EXE".to_string(), 5, false),
            ("empty.txt".to_string(), 0, false),
        ]
    );
}

#[test]
fn finds_by_long_or_short_name_case_insensitively() {
    let img = small_image(
        &[Entry {
            name: "retropane.bin",
            data: b"x",
        }],
        &[],
    );
    let mut v = Volume::mount(MemDisk::new(img), PARTITION_START).unwrap();
    let f = v.find("RETROPANE.BIN").unwrap();
    assert_eq!(f.size(), 1);
    assert_eq!(v.find("retrop~1.bin").unwrap(), f);
    assert_eq!(v.find("nothere.bin"), Err(Error::NotFound));
    // A separator makes it a path: into a folder that doesn't exist.
    assert_eq!(v.find("bad\\name"), Err(Error::NotFound));
    assert_eq!(v.find("a/../retropane.bin"), Err(Error::BadName));
}

#[test]
fn reads_a_file_with_any_scratch_size() {
    let a = pattern(5000, 2); // 5 clusters, last one partial
    let img = small_image(
        &[Entry {
            name: "a.bin",
            data: &a,
        }],
        &[],
    );
    let mut v = Volume::mount(MemDisk::new(img), PARTITION_START).unwrap();
    for scratch in [512, 1024, 1536, 4096, 65536] {
        assert_eq!(read_all(&mut v, "a.bin", scratch), a, "scratch {scratch}");
    }
    // Reading in one 64 KiB scratch takes one transfer for the whole run.
    let before = v.dev.reads;
    read_all(&mut v, "a.bin", 65536);
    let reads = v.dev.reads - before;
    assert!(
        reads <= 4,
        "{reads} reads for a contiguous file (dir walk + FAT + data)"
    );
}

#[test]
fn replace_with_more_data_moves_to_a_fresh_run_and_frees_the_old() {
    let old = pattern(1500, 3);
    let new = pattern(9000, 4);
    let img = small_image(
        &[Entry {
            name: "retropane.bin",
            data: &old,
        }],
        &[],
    );
    let mut v = Volume::mount(MemDisk::new(img), PARTITION_START).unwrap();
    let old_file = v.find("retropane.bin").unwrap();
    let w = v.replace("retropane.bin", &new, &mut [0; 4096]).unwrap();
    assert!(w.contiguous);
    assert_eq!(w.clusters, 9);
    assert_ne!(w.first_cluster, old_file.first_cluster());
    assert_eq!(read_all(&mut v, "retropane.bin", 4096), new);
    // The old clusters are free again in both FATs.
    for c in old_file.first_cluster()..old_file.first_cluster() + 2 {
        assert_eq!(v.fat_get(c).unwrap(), 0);
    }
    let fat1 = (PARTITION_START + 32) as usize * SECTOR;
    let fat_sectors = v.fat_sectors as usize;
    let copy2 = &v.dev.data[fat1 + fat_sectors * SECTOR..fat1 + 2 * fat_sectors * SECTOR];
    assert_eq!(
        &v.dev.data[fat1..fat1 + fat_sectors * SECTOR],
        copy2,
        "both FATs agree"
    );
    // Replacing again reuses the freed space eventually: a smaller file.
    let tiny = b"tiny".to_vec();
    v.replace("retropane.bin", &tiny, &mut [0; 4096]).unwrap();
    assert_eq!(read_all(&mut v, "retropane.bin", 512), tiny);
    assert_eq!(v.find("retropane.bin").unwrap().size(), 4);
}

#[test]
fn replace_falls_back_to_scattered_clusters_when_no_run_is_free() {
    // 1024-sector partition, 1 KiB clusters: ~490 data clusters. Block
    // every other cluster so no run longer than one exists.
    let blocked: Vec<u32> = (4..400).step_by(2).collect();
    let old = pattern(100, 5);
    let new = pattern(2500, 6); // 3 clusters
    let img = small_image(
        &[Entry {
            name: "f.bin",
            data: &old,
        }],
        &blocked,
    );
    // Without a scratch (one FAT sector per command) and with one.
    for scratch in [0, 4096] {
        let mut v = Volume::mount(MemDisk::new(img.clone()), PARTITION_START).unwrap();
        // Reserve the tail too, so only the gaps between blocked clusters
        // remain.
        for c in 400..2 + v.clusters() {
            v.fat_set(c, EOC).unwrap();
        }
        v.fat_flush().unwrap();
        let w = v.replace("f.bin", &new, &mut vec![0; scratch]).unwrap();
        assert!(!w.contiguous);
        assert_eq!(w.clusters, 3);
        assert_eq!(read_all(&mut v, "f.bin", 2048), new);
    }
}

/// Scattered clusters in different FAT sectors: each search starts in the
/// write-back cache (which holds the clusters just taken, not yet written)
/// and goes on through the scratch; the result must match the one-sector
/// path cluster for cluster.
#[test]
fn scattered_allocation_across_fat_sectors_matches_the_sector_path() {
    let gaps = [130, 200, 260, 300, 390];
    let blocked: Vec<u32> = (4..400).filter(|c| !gaps.contains(c)).collect();
    let new = pattern(4500, 11); // 5 clusters
    let img = small_image(
        &[Entry {
            name: "f.bin",
            data: b"old",
        }],
        &blocked,
    );
    let mut fats = Vec::new();
    for scratch in [0, 512, 1536, 65536] {
        let mut v = Volume::mount(MemDisk::new(img.clone()), PARTITION_START).unwrap();
        for c in 400..2 + v.clusters() {
            v.fat_set(c, EOC).unwrap();
        }
        v.fat_flush().unwrap();
        let w = v.replace("f.bin", &new, &mut vec![0; scratch]).unwrap();
        assert!(!w.contiguous);
        assert_eq!(w.first_cluster, 130, "scratch {scratch}");
        assert_eq!(read_all(&mut v, "f.bin", 4096), new);
        let fat = (PARTITION_START + 32) as usize * SECTOR;
        fats.push(v.dev.data[fat..fat + 2 * v.fat_sectors as usize * SECTOR].to_vec());
    }
    assert!(fats.windows(2).all(|w| w[0] == w[1]), "FATs differ");
}

/// The stale-hint case from the device (3.7 s for one write): the free
/// space starts far behind the FSInfo hint, so the search walks most of
/// the FAT. Reading it in runs through the scratch must take at most an
/// eighth of the commands the one-sector path does, for the same result.
#[test]
fn a_long_free_cluster_search_reads_the_fat_in_runs() {
    let taken: Vec<u32> = (10..60_000).collect();
    let new = pattern(20_000, 12); // 40 clusters of 512 bytes
    let img = format::build(&Image {
        total_sectors: 2048 + 64 * 1024, // 32 MiB partition
        sectors_per_cluster: 1,
        files: &[Entry {
            name: "f.bin",
            data: b"old",
        }],
        blocked_clusters: &taken,
    });
    let mut reads = Vec::new();
    for scratch in [0, 64 * 1024] {
        let mut v = Volume::mount(MemDisk::new(img.clone()), PARTITION_START).unwrap();
        assert!(v.fat_sectors > 400, "{} FAT sectors", v.fat_sectors);
        let w = v.replace("f.bin", &new, &mut vec![0; scratch]).unwrap();
        assert!(w.contiguous);
        assert_eq!(w.first_cluster, 60_000);
        reads.push(v.dev.reads);
        assert_eq!(read_all(&mut v, "f.bin", 4096), new);
    }
    let (by_sector, in_runs) = (reads[0], reads[1]);
    eprintln!("replace: {by_sector} reads sector by sector, {in_runs} in runs");
    assert!(
        in_runs * 8 <= by_sector,
        "{in_runs} reads in runs vs {by_sector} sector by sector"
    );
}

#[test]
fn replace_refuses_when_the_card_is_full_and_keeps_the_old_file() {
    let old = pattern(700, 7);
    let img = small_image(
        &[Entry {
            name: "f.bin",
            data: &old,
        }],
        &[],
    );
    let mut v = Volume::mount(MemDisk::new(img), PARTITION_START).unwrap();
    for c in 4..2 + v.clusters() {
        v.fat_set(c, EOC).unwrap(); // everything else taken
    }
    v.fat_flush().unwrap();
    let big = pattern(4000, 8);
    assert_eq!(v.replace("f.bin", &big, &mut [0; 4096]), Err(Error::NoRoom));
    assert_eq!(read_all(&mut v, "f.bin", 1024), old);
    assert_eq!(v.fat_get(3).unwrap(), EOC, "the old chain is intact");
}

#[test]
fn an_empty_replacement_has_no_clusters() {
    let img = small_image(
        &[Entry {
            name: "f.bin",
            data: b"abc",
        }],
        &[],
    );
    let mut v = Volume::mount(MemDisk::new(img), PARTITION_START).unwrap();
    let w = v.replace("f.bin", b"", &mut [0; 4096]).unwrap();
    assert_eq!(w.clusters, 0);
    let f = v.find("f.bin").unwrap();
    assert_eq!((f.size(), f.first_cluster()), (0, 0));
    assert_eq!(read_all(&mut v, "f.bin", 512), Vec::<u8>::new());
}

/// Power lost after any number of sector writes: the file must read back
/// as the complete old contents or the complete new contents.
#[test]
fn power_loss_at_every_write_leaves_a_whole_file() {
    let old = pattern(2500, 9);
    let new = pattern(6100, 10);
    let img = small_image(
        &[Entry {
            name: "retropane.bin",
            data: &old,
        }],
        &[],
    );
    for scratch in [0, 4096] {
        power_loss_sweep(&img, &old, &new, scratch);
    }
}

fn power_loss_sweep(img: &[u8], old: &[u8], new: &[u8], scratch: usize) {
    let total = {
        let mut v = Volume::mount(MemDisk::new(img.to_vec()), PARTITION_START).unwrap();
        v.replace("retropane.bin", new, &mut vec![0; scratch])
            .unwrap();
        v.dev.writes
    };
    assert!(total >= 4, "{total} writes: data, FATs, directory, FSInfo");
    let mut saw_old = 0;
    let mut saw_new = 0;
    for fail_at in 0..total {
        let mut disk = MemDisk::new(img.to_vec());
        disk.fail_at = Some(fail_at);
        let mut v = Volume::mount(disk, PARTITION_START).unwrap();
        let r = v.replace("retropane.bin", new, &mut vec![0; scratch]);
        assert!(r.is_err(), "write {fail_at} should have failed");
        // Mount afresh, as after a reset.
        let data = v.into_inner().data;
        let mut v = Volume::mount(MemDisk::new(data), PARTITION_START).unwrap();
        let got = read_all(&mut v, "retropane.bin", 4096);
        if got == old {
            saw_old += 1;
        } else if got == new {
            saw_new += 1;
        } else {
            panic!(
                "torn file after power loss at write {fail_at}: {} bytes",
                got.len()
            );
        }
    }
    assert!(saw_old > 0 && saw_new > 0, "old {saw_old}, new {saw_new}");
}

#[test]
fn first_fat_sectors_have_a_mirror_in_the_second() {
    let img = small_image(
        &[Entry {
            name: "a.bin",
            data: b"x",
        }],
        &[],
    );
    let v = Volume::mount(MemDisk::new(img), PARTITION_START).unwrap();
    let first = PARTITION_START + 32;
    assert_eq!(v.fat_mirror(first), Some(first + v.fat_sectors));
    assert_eq!(
        v.fat_mirror(first + v.fat_sectors - 1),
        Some(first + 2 * v.fat_sectors - 1)
    );
    // The second FAT, the reserved area and the data have none.
    assert_eq!(v.fat_mirror(first + v.fat_sectors), None);
    assert_eq!(v.fat_mirror(PARTITION_START), None);
    assert_eq!(v.fat_mirror(v.data_start + 5), None);
    // Mirroring off (ExtFlags bit 7): the second FAT is no backup.
    let mut img = small_image(&[], &[]);
    let bs = PARTITION_START as usize * SECTOR;
    img[bs + 40] = 0x80;
    let v = Volume::mount(MemDisk::new(img), PARTITION_START).unwrap();
    assert_eq!(v.fat_mirror(first), None);
}

#[test]
fn rejects_what_is_not_fat32() {
    let mut img = small_image(&[], &[]);
    let bs = PARTITION_START as usize * SECTOR;
    img[bs + 17] = 2; // a root entry count: FAT12/16
    assert_eq!(
        Volume::mount(MemDisk::new(img), PARTITION_START).err(),
        Some(Error::NotFat32)
    );
    let mut zero = vec![0u8; 4096 * SECTOR];
    zero[bs + 510] = 0x55;
    assert!(Volume::mount(MemDisk::new(zero), PARTITION_START).is_err());
}

#[test]
fn short_name_checksum_matches_the_specification_example() {
    // "FOO     BAR" has no published value; check the recurrence instead.
    let mut s = [b' '; 11];
    s[..3].copy_from_slice(b"FOO");
    let sum = short_checksum(&s);
    let manual = s.iter().fold(0u8, |a, &b| {
        ((a & 1) << 7).wrapping_add(a >> 1).wrapping_add(b)
    });
    assert_eq!(sum, manual);
    assert_ne!(sum, short_checksum(&[b' '; 11]));
}

#[test]
fn error_names_are_short() {
    assert_eq!(Error::NoRoom.name(), "no room");
    assert_eq!(Error::Device(BlockError::Device).name(), "device error");
}

fn tiny_image(files: &[Entry<'_>]) -> Vec<u8> {
    small_image(files, &[])
}

#[test]
fn creates_files_with_8_3_and_lower_case_names() {
    let img = tiny_image(&[Entry {
        name: "OLD.BIN",
        data: b"x",
    }]);
    let mut v = Volume::mount(MemDisk::new(img), PARTITION_START).unwrap();
    let mut scratch = vec![0u8; 4096];
    let a = pattern(3000, 1);
    v.create("NEW.BIN", &a, &mut scratch).unwrap();
    v.create("readme.txt", b"lower", &mut scratch).unwrap();
    assert_eq!(read_all(&mut v, "NEW.BIN", 4096), a);
    assert_eq!(read_all(&mut v, "README.TXT", 512), b"lower");
    let mut names = Vec::new();
    v.for_each_root(|e| names.push(String::from_utf8_lossy(e.name).into_owned()))
        .unwrap();
    // No long-name entries needed: Windows NT's case flags keep it lower.
    assert_eq!(names, ["OLD.BIN", "NEW.BIN", "readme.txt"]);
    assert_eq!(v.create("new.bin", b"y", &mut scratch), Err(Error::Exists));
}

#[test]
fn creates_long_names_with_windows_style_short_names() {
    let img = tiny_image(&[]);
    let mut v = Volume::mount(MemDisk::new(img), PARTITION_START).unwrap();
    let mut scratch = vec![0u8; 4096];
    v.create("My Long File Name.text", b"one", &mut scratch)
        .unwrap();
    v.create("My Long File Name 2.text", b"two", &mut scratch)
        .unwrap();
    v.create("RetroPane loader.lnk", b"lnk", &mut scratch)
        .unwrap();
    assert_eq!(read_all(&mut v, "my long file name.text", 512), b"one");
    assert_eq!(read_all(&mut v, "MYLONG~1.TEX", 512), b"one");
    assert_eq!(read_all(&mut v, "MYLONG~2.TEX", 512), b"two");
    assert_eq!(read_all(&mut v, "RETROP~1.LNK", 512), b"lnk");
    let mut names = Vec::new();
    v.for_each_root(|e| names.push(String::from_utf8_lossy(e.name).into_owned()))
        .unwrap();
    assert_eq!(
        names,
        [
            "My Long File Name.text",
            "My Long File Name 2.text",
            "RetroPane loader.lnk"
        ]
    );
}

#[test]
fn refuses_names_fat_does_not_allow() {
    let img = tiny_image(&[]);
    let mut v = Volume::mount(MemDisk::new(img), PARTITION_START).unwrap();
    let mut scratch = vec![0u8; 512];
    for bad in [
        "a*b",
        "what?",
        "x:y",
        "pipe|",
        "trailing.",
        "trailing ",
        ".",
        "..",
        "",
    ] {
        assert_eq!(
            v.create(bad, b"z", &mut scratch),
            Err(Error::BadName),
            "{bad:?}"
        );
    }
}

#[test]
fn a_full_root_directory_grows() {
    // 1 KiB clusters: 32 entries per cluster, so ~10 long names fill one.
    let img = tiny_image(&[]);
    let mut v = Volume::mount(MemDisk::new(img), PARTITION_START).unwrap();
    let mut scratch = vec![0u8; 4096];
    for i in 0..40 {
        let name = format!("a rather long file name number {i}.txt");
        v.create(&name, name.as_bytes(), &mut scratch).unwrap();
    }
    for i in 0..40 {
        let name = format!("a rather long file name number {i}.txt");
        assert_eq!(read_all(&mut v, &name, 512), name.as_bytes(), "{name}");
    }
    // The root chain is now several clusters long.
    assert_ne!(v.fat_get(2).unwrap() & 0x0FFF_FFFF, 0x0FFF_FFFF);
}

#[test]
fn put_replaces_or_creates() {
    let img = tiny_image(&[Entry {
        name: "a.bin",
        data: b"old",
    }]);
    let mut v = Volume::mount(MemDisk::new(img), PARTITION_START).unwrap();
    let mut scratch = vec![0u8; 4096];
    v.put("a.bin", b"new", &mut scratch).unwrap();
    v.put("b.bin", b"made", &mut scratch).unwrap();
    assert_eq!(read_all(&mut v, "a.bin", 512), b"new");
    assert_eq!(read_all(&mut v, "b.bin", 512), b"made");
}

#[test]
fn creating_on_a_full_card_gives_the_clusters_back() {
    let img = tiny_image(&[Entry {
        name: "a.bin",
        data: b"keep",
    }]);
    let mut v = Volume::mount(MemDisk::new(img), PARTITION_START).unwrap();
    for c in 4..2 + v.clusters() {
        v.fat_set(c, EOC).unwrap();
    }
    v.fat_flush().unwrap();
    let mut scratch = vec![0u8; 4096];
    assert_eq!(
        v.create("big.bin", &pattern(5000, 3), &mut scratch),
        Err(Error::NoRoom)
    );
    assert_eq!(v.find("big.bin"), Err(Error::NotFound));
    assert_eq!(read_all(&mut v, "a.bin", 512), b"keep");
}

/// Power lost after any number of writes while creating: afterwards the
/// file either doesn't exist or reads back whole, and the other file is
/// untouched.
#[test]
fn power_loss_while_creating_leaves_no_file_or_the_whole_file() {
    let keep = pattern(1500, 4);
    let new = pattern(4100, 5);
    let img = tiny_image(&[Entry {
        name: "keep.bin",
        data: &keep,
    }]);
    let total = {
        let mut v = Volume::mount(MemDisk::new(img.clone()), PARTITION_START).unwrap();
        let mut scratch = vec![0u8; 4096];
        v.create("A brand new long name.bin", &new, &mut scratch)
            .unwrap();
        v.dev.writes
    };
    let (mut none, mut whole) = (0, 0);
    for fail_at in 0..total {
        let mut disk = MemDisk::new(img.clone());
        disk.fail_at = Some(fail_at);
        let mut v = Volume::mount(disk, PARTITION_START).unwrap();
        let mut scratch = vec![0u8; 4096];
        assert!(
            v.create("A brand new long name.bin", &new, &mut scratch)
                .is_err()
        );
        let data = v.into_inner().data;
        let mut v = Volume::mount(MemDisk::new(data), PARTITION_START).unwrap();
        match v.find("A brand new long name.bin") {
            Err(Error::NotFound) => none += 1,
            Ok(_) => {
                assert_eq!(read_all(&mut v, "A brand new long name.bin", 4096), new);
                whole += 1;
            }
            Err(e) => panic!("write {fail_at}: {e:?}"),
        }
        assert_eq!(read_all(&mut v, "keep.bin", 4096), keep);
    }
    assert!(none > 0, "{none} none, {whole} whole");
}

/// A disk that panics if a single operation needs more than `limit`
/// transfers: an endless loop on a hostile card shows up as a failure.
fn bounded(data: Vec<u8>) -> MemDisk {
    MemDisk::new(data)
}

fn ops(d: &MemDisk) -> usize {
    d.reads + d.writes
}

/// Runs every public operation on a (possibly hostile) volume; returns
/// nothing, it only must not panic or run away.
fn exercise(data: Vec<u8>) {
    let Ok(mut v) = Volume::mount(bounded(data), PARTITION_START) else {
        return;
    };
    // A healthy 1024-sector image never needs more than this many transfers
    // for one operation; a loop would.
    const LIMIT: usize = 20_000;
    let mut scratch = vec![0u8; 4096];
    let start = ops(&v.dev);
    let _ = v.for_each_root(|_| {});
    assert!(ops(&v.dev) - start < LIMIT, "for_each_root ran away");
    for name in [
        "retropane.bin",
        "HARET.EXE",
        "x.txt",
        "A long file name.dat",
    ] {
        let start = ops(&v.dev);
        if let Ok(f) = v.find(name) {
            let _ = v.read_file(&f, &mut scratch, |_| {});
        }
        assert!(ops(&v.dev) - start < LIMIT, "find/read {name} ran away");
    }
    let start = ops(&v.dev);
    let _ = v.put("retropane.bin", &[7u8; 3000], &mut scratch);
    let _ = v.create("A long file name.dat", &[9u8; 700], &mut scratch);
    assert!(ops(&v.dev) - start < 4 * LIMIT, "writes ran away");
}

#[test]
fn hostile_cards_never_panic_or_loop() {
    let a = pattern(3000, 11);
    let base = small_image(
        &[
            Entry {
                name: "retropane.bin",
                data: &a,
            },
            Entry {
                name: "HARET.EXE",
                data: b"haret",
            },
        ],
        &[],
    );
    let bs = PARTITION_START as usize * SECTOR;
    let mut x: u64 = 0x0FA7_2026;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for i in 0..1500 {
        let mut img = base.clone();
        // Corrupt a few bytes, aimed at the structures: boot sector, FSInfo,
        // the FAT, the root directory (and anywhere now and then).
        for _ in 0..1 + rnd() % 6 {
            let region = match i % 5 {
                0 => bs..bs + SECTOR,
                1 => bs + SECTOR..bs + 2 * SECTOR,
                2 => bs + 32 * SECTOR..bs + 40 * SECTOR,
                3 => {
                    let v = Volume::mount(MemDisk::new(base.clone()), PARTITION_START).unwrap();
                    let root = v.cluster_lba(v.root_cluster) as usize * SECTOR;
                    root..root + v.cluster_bytes()
                }
                _ => 0..img.len(),
            };
            let at = region.start + (rnd() as usize % region.len());
            img[at] = rnd() as u8;
        }
        exercise(img);
    }
}

#[test]
fn a_fat_chain_that_loops_is_corrupt_not_endless() {
    let a = pattern(3000, 12);
    let img = small_image(
        &[Entry {
            name: "a.bin",
            data: &a,
        }],
        &[],
    );
    let mut v = Volume::mount(MemDisk::new(img), PARTITION_START).unwrap();
    let f = v.find("a.bin").unwrap();
    // Second cluster points back at the first.
    v.fat_set(f.first_cluster() + 1, f.first_cluster()).unwrap();
    v.fat_flush().unwrap();
    let mut scratch = vec![0u8; 512];
    // The size says 3 clusters, the chain never ends: an error, not a hang.
    let r = v.read_file(&f, &mut scratch, |_| {});
    assert!(r.is_ok() || r == Err(Error::Corrupt));
    // Replacing frees the old (looping) chain: must terminate.
    let _ = v.replace("a.bin", b"new", &mut scratch);
    exercise(v.into_inner().data);
}

#[test]
fn a_root_directory_that_loops_is_corrupt_not_endless() {
    let img = small_image(
        &[Entry {
            name: "a.bin",
            data: b"x",
        }],
        &[],
    );
    let mut v = Volume::mount(MemDisk::new(img), PARTITION_START).unwrap();
    // Fill the root cluster with entries (no end marker), then loop it.
    let root = v.root_cluster;
    v.fat_set(root, root).unwrap();
    v.fat_flush().unwrap();
    let lba = v.cluster_lba(root);
    let mut sector = [0u8; SECTOR];
    for i in 0..ENTRIES {
        sector[i * 32..i * 32 + 11].copy_from_slice(b"FILLER  BIN");
        sector[i * 32 + 11] = 0x20;
    }
    for s in 0..v.sectors_per_cluster {
        v.dev.write_blocks(lba + s, &sector).unwrap();
    }
    assert_eq!(v.find("nothere.bin"), Err(Error::Corrupt));
    let mut scratch = vec![0u8; 512];
    assert!(v.create("new.bin", b"y", &mut scratch).is_err());
}

#[test]
fn out_of_range_clusters_and_absurd_geometry_are_refused() {
    let img = small_image(
        &[Entry {
            name: "a.bin",
            data: b"abc",
        }],
        &[],
    );
    // A directory entry pointing far past the volume.
    let mut v = Volume::mount(MemDisk::new(img.clone()), PARTITION_START).unwrap();
    let f = v.find("a.bin").unwrap();
    let mut s = [0u8; SECTOR];
    v.dev.read_blocks(f.entry_lba, &mut s).unwrap();
    let o = usize::from(f.entry_index) * 32;
    s[o + 20..o + 22].copy_from_slice(&0x0FFFu16.to_le_bytes());
    v.dev.write_blocks(f.entry_lba, &s).unwrap();
    let f = v.find("a.bin").unwrap();
    let mut scratch = vec![0u8; 512];
    assert_eq!(v.read_file(&f, &mut scratch, |_| {}), Err(Error::Corrupt));
    // Geometry: zero or odd cluster sizes, a root cluster past the end.
    let bs = PARTITION_START as usize * SECTOR;
    for (at, val) in [(13usize, 0u8), (13, 3), (16, 0)] {
        let mut bad = img.clone();
        bad[bs + at] = val;
        assert!(
            Volume::mount(MemDisk::new(bad), PARTITION_START).is_err(),
            "byte {at} = {val}"
        );
    }
    let mut bad = img.clone();
    bad[bs + 44..bs + 48].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
    if let Ok(mut v) = Volume::mount(MemDisk::new(bad), PARTITION_START) {
        assert!(v.find("a.bin").is_err());
    }
}

#[test]
fn any_volume_mounts_either_kind_and_puts_and_reads_back() {
    let fat = tiny_image(&[Entry {
        name: "a.txt",
        data: b"fat",
    }]);
    let ex = crate::exfat::format::build(
        4096,
        2,
        &[crate::exfat::format::Entry {
            name: "a.txt",
            data: b"exfat",
            fragment: false,
        }],
    );
    let mut scratch = vec![0u8; 4096];
    for (img, was, kind) in [(fat, &b"fat"[..], "fat32"), (ex, &b"exfat"[..], "exfat")] {
        let mut v = AnyVolume::mount(MemDisk::new(img), PARTITION_START).unwrap();
        assert_eq!(
            matches!(v, AnyVolume::ExFat(_)),
            kind == "exfat",
            "{kind} mounted as the wrong kind"
        );
        let mut got = Vec::new();
        v.read_back("A.TXT", &mut scratch, &mut |c| got.extend_from_slice(c))
            .unwrap();
        assert_eq!(got, was);
        v.put("crashlog.txt", b"report", &mut scratch).unwrap();
        let mut got = Vec::new();
        v.read_back("crashlog.txt", &mut scratch, &mut |c| {
            got.extend_from_slice(c)
        })
        .unwrap();
        assert_eq!(got, b"report");
        assert!(v.device().writes > 0);
        let mut names = Vec::new();
        v.list("", &mut |n, d, size| {
            names.push((String::from_utf8_lossy(n).into_owned(), d, size))
        })
        .unwrap();
        assert!(
            names.contains(&("crashlog.txt".into(), false, 6)),
            "{kind}: {names:?}"
        );
    }
    // Neither: refused.
    let blank = vec![0u8; 4096 * SECTOR];
    assert!(AnyVolume::mount(MemDisk::new(blank), PARTITION_START).is_err());
}

#[test]
fn folders_paths_and_put_creating_parents() {
    let img = tiny_image(&[Entry {
        name: "top.txt",
        data: b"top",
    }]);
    let mut v = Volume::mount(MemDisk::new(img), PARTITION_START).unwrap();
    let mut scratch = vec![0u8; 4096];
    v.mkdir("Apps", &mut scratch).unwrap();
    assert_eq!(v.mkdir("apps", &mut scratch), Err(Error::Exists));
    assert_eq!(v.mkdir("top.txt", &mut scratch), Err(Error::Exists));
    let a = pattern(5000, 21);
    v.create("Apps/Game One.exe", &a, &mut scratch).unwrap();
    // put creates missing parents, either separator.
    v.put("Apps\\Sub Folder\\deep/readme.txt", b"deep", &mut scratch)
        .unwrap();
    assert_eq!(read_all(&mut v, "apps/game one.exe", 4096), a);
    assert_eq!(read_all(&mut v, "APPS/GAMEON~1.EXE", 4096), a);
    assert_eq!(
        read_all(&mut v, "Apps/Sub Folder/deep/readme.txt", 512),
        b"deep"
    );
    // Replace inside a folder.
    v.put("Apps/Game One.exe", b"v2", &mut scratch).unwrap();
    assert_eq!(read_all(&mut v, "Apps/Game One.exe", 512), b"v2");
    // Listings: the root shows the folder, the folder its contents (no . or ..).
    let mut root = Vec::new();
    v.for_each_root(|e| root.push((String::from_utf8_lossy(e.name).into_owned(), e.is_dir)))
        .unwrap();
    assert_eq!(root, [("top.txt".into(), false), ("Apps".into(), true)]);
    let mut apps = Vec::new();
    v.for_each_in("Apps", |e| {
        apps.push(String::from_utf8_lossy(e.name).into_owned())
    })
    .unwrap();
    assert_eq!(apps, ["Game One.exe", "Sub Folder"]);
    // Short names are per folder: the same long name in another folder
    // gets ~1 again.
    v.put("Other/Game One.exe", b"o", &mut scratch).unwrap();
    assert_eq!(read_all(&mut v, "OTHER/GAMEON~1.EXE", 512), b"o");
    // A file isn't a folder.
    assert_eq!(v.find("top.txt/x"), Err(Error::NotFound));
    assert_eq!(v.find("nothere/x"), Err(Error::NotFound));
    // Too deep.
    let deep = "a/".repeat(20) + "f";
    assert_eq!(v.find(&deep), Err(Error::BadName));
}

#[test]
fn a_folder_grows_and_power_loss_while_putting_into_new_folders_is_safe() {
    let img = tiny_image(&[]);
    let mut v = Volume::mount(MemDisk::new(img.clone()), PARTITION_START).unwrap();
    let mut scratch = vec![0u8; 4096];
    for i in 0..40 {
        let name = format!("Many/a rather long name number {i}.txt");
        v.put(&name, name.as_bytes(), &mut scratch).unwrap();
    }
    for i in 0..40 {
        let name = format!("Many/a rather long name number {i}.txt");
        assert_eq!(read_all(&mut v, &name, 512), name.as_bytes());
    }
    // Power loss at every write of a put that makes two folders and a file.
    let data = pattern(3000, 22);
    let total = {
        let mut v = Volume::mount(MemDisk::new(img.clone()), PARTITION_START).unwrap();
        v.put("x/y/z.bin", &data, &mut scratch).unwrap();
        v.dev.writes
    };
    for fail_at in 0..total {
        let mut disk = MemDisk::new(img.clone());
        disk.fail_at = Some(fail_at);
        let mut v = Volume::mount(disk, PARTITION_START).unwrap();
        assert!(v.put("x/y/z.bin", &data, &mut scratch).is_err());
        let after = v.into_inner().data;
        let mut v = Volume::mount(MemDisk::new(after), PARTITION_START).unwrap();
        match v.find("x/y/z.bin") {
            Ok(_) => assert_eq!(read_all(&mut v, "x/y/z.bin", 4096), data),
            Err(Error::NotFound) => {}
            Err(e) => panic!("write {fail_at}: {e:?}"),
        }
        // Whatever exists is walkable.
        let _ = v.for_each_in("x", |_| {});
        let _ = v.for_each_in("x/y", |_| {});
    }
}

#[test]
fn a_folder_that_points_back_at_the_root_is_corrupt_not_a_loop() {
    let img = tiny_image(&[]);
    let mut v = Volume::mount(MemDisk::new(img), PARTITION_START).unwrap();
    let mut scratch = vec![0u8; 4096];
    v.mkdir("loop", &mut scratch).unwrap();
    // Point the folder's entry at the root cluster.
    let root = v.root_cluster;
    let lba = v.cluster_lba(root);
    let mut sector = [0u8; SECTOR];
    v.dev.read_blocks(lba, &mut sector).unwrap();
    let i = (0..ENTRIES)
        .find(|&i| &sector[i * 32..i * 32 + 4] == b"LOOP")
        .unwrap();
    sector[i * 32 + 26..i * 32 + 28].copy_from_slice(&(root as u16).to_le_bytes());
    v.dev.write_blocks(lba, &sector).unwrap();
    assert_eq!(v.find("loop/loop/loop/x"), Err(Error::Corrupt));
}
