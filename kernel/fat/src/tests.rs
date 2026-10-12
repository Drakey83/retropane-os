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
    assert_eq!(v.find("bad\\name"), Err(Error::BadName));
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
