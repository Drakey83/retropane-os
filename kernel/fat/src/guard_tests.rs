//! The guard against what a PC does to a mounted card: this crate's own
//! FAT writer stands in for Windows (it writes entries, chains in both FATs
//! and data the same way), plus raw sector writes for a label change, a
//! rename and a format.

use rp_hal_api::BlockError;

use super::*;
use crate::FAT_PER_SECTOR;
use crate::format::{self, Entry, Image, PARTITION_START};

const BOOT: [&str; 4] = ["retropane.bin", "haret.exe", "startup.txt", "default.txt"];

/// A card behind the guard, as the loader's drive offers it.
struct Guarded {
    data: Vec<u8>,
    guard: Option<Guard>,
    refused: usize,
}

impl BlockDevice for Guarded {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
        let o = lba as usize * SECTOR;
        let src = self
            .data
            .get(o..o + buf.len())
            .ok_or(BlockError::OutOfRange)?;
        buf.copy_from_slice(src);
        Ok(())
    }
    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
        if let Some(g) = &self.guard
            && !g.allows(lba, buf)
        {
            self.refused += 1;
            return Err(BlockError::ReadOnly);
        }
        let o = lba as usize * SECTOR;
        let dst = self
            .data
            .get_mut(o..o + buf.len())
            .ok_or(BlockError::OutOfRange)?;
        dst.copy_from_slice(buf);
        Ok(())
    }
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31) ^ seed)
        .collect()
}

/// A card like the owner's: the boot files (one fragmented over `blocked`
/// clusters), a README, and room to spare; the guard built and armed.
fn card(blocked: &[u32], loader_len: usize) -> (Volume<Guarded>, Vec<u8>) {
    let loader = pattern(loader_len, 7);
    let haret = pattern(5000, 3);
    let img = format::build(&Image {
        total_sectors: 2048 + 2048,
        sectors_per_cluster: 2,
        files: &[
            Entry {
                name: "retropane.bin",
                data: &loader,
            },
            Entry {
                name: "haret.exe",
                data: &haret,
            },
            Entry {
                name: "startup.txt",
                data: b"set ramaddr 0xa8000000\n",
            },
            Entry {
                name: "README.txt",
                data: b"hello",
            },
        ],
        blocked_clusters: blocked,
        ballast: 0,
    });
    let dev = Guarded {
        data: img,
        guard: None,
        refused: 0,
    };
    let mut v = Volume::mount(dev, PARTITION_START).unwrap();
    let g = v.guard(&BOOT).unwrap();
    assert_eq!(g.files(), 3, "default.txt is absent and skipped");
    v.dev.guard = Some(g);
    (v, loader)
}

fn read(v: &mut Volume<Guarded>, name: &str) -> Vec<u8> {
    let f = v.find(name).unwrap();
    let mut out = Vec::new();
    v.read_file(&f, &mut [0u8; 4096], |c| out.extend_from_slice(c))
        .unwrap();
    out
}

fn sector(v: &Volume<Guarded>, lba: u32) -> Vec<u8> {
    let o = lba as usize * SECTOR;
    v.dev.data[o..o + SECTOR].to_vec()
}

fn raw_write(v: &mut Volume<Guarded>, lba: u32, s: &[u8]) -> Result<(), BlockError> {
    v.dev.write_blocks(lba, s)
}

#[test]
fn everyday_changes_pass() {
    let (mut v, loader) = card(&[], 3000);
    let mut scratch = vec![0u8; 4096];
    // New files and folders, changing and deleting other files.
    v.put("photos/cat.jpg", &pattern(9000, 1), &mut scratch)
        .unwrap();
    v.put("README.txt", b"changed", &mut scratch).unwrap();
    v.put("notes.txt", b"x", &mut scratch).unwrap();
    v.remove("notes.txt").unwrap();
    // Renaming the volume: the boot sector's label and the root's label
    // entry (Windows writes both).
    let mut bs = sector(&v, PARTITION_START);
    bs[71..82].copy_from_slice(b"SDX51V     ");
    raw_write(&mut v, PARTITION_START, &bs).unwrap();
    raw_write(&mut v, PARTITION_START + 6, &bs).unwrap();
    assert_eq!(v.dev.refused, 0);
    assert_eq!(read(&mut v, "retropane.bin"), loader);
    assert_eq!(read(&mut v, "README.txt"), b"changed");
}

#[test]
fn boot_files_cannot_be_deleted_replaced_or_renamed() {
    let (mut v, loader) = card(&[], 3000);
    let mut scratch = vec![0u8; 4096];
    assert!(v.remove("retropane.bin").is_err());
    assert!(
        v.put("retropane.bin", &pattern(4000, 9), &mut scratch)
            .is_err()
    );
    assert!(v.remove("haret.exe").is_err());
    // A rename: the entry's short name changes.
    let f = v.find("startup.txt").unwrap();
    let mut s = sector(&v, f.entry_lba);
    let o = usize::from(f.entry_index) * 32;
    s[o..o + 8].copy_from_slice(b"STARTUP2");
    assert_eq!(
        raw_write(&mut v, f.entry_lba, &s),
        Err(BlockError::ReadOnly)
    );
    // Overwriting its data in place.
    let first = v.find("retropane.bin").unwrap().first_cluster;
    let lba = v.cluster_lba(first);
    assert_eq!(
        raw_write(&mut v, lba, &[0u8; SECTOR]),
        Err(BlockError::ReadOnly)
    );
    assert!(v.dev.refused >= 5);
    // Everything that boots the device is as it was.
    assert_eq!(read(&mut v, "retropane.bin"), loader);
    assert_eq!(read(&mut v, "haret.exe"), pattern(5000, 3));
    assert_eq!(read(&mut v, "startup.txt"), b"set ramaddr 0xa8000000\n");
}

#[test]
fn a_format_or_a_new_partition_table_is_refused() {
    let (mut v, _) = card(&[], 3000);
    let mut bs = sector(&v, PARTITION_START);
    bs[13] = 8; // sectors per cluster
    assert_eq!(
        raw_write(&mut v, PARTITION_START, &bs),
        Err(BlockError::ReadOnly)
    );
    assert_eq!(
        raw_write(&mut v, PARTITION_START + 6, &bs),
        Err(BlockError::ReadOnly)
    );
    let mut mbr = sector(&v, 0);
    mbr[446 + 4] = 0x07; // exFAT
    assert_eq!(raw_write(&mut v, 0, &mbr), Err(BlockError::ReadOnly));
    // Zeroing a FAT sector (what a quick format does) is refused too.
    let (fat, fat_sectors) = (v.fat_start, v.fat_sectors);
    assert_eq!(
        raw_write(&mut v, fat, &[0u8; SECTOR]),
        Err(BlockError::ReadOnly)
    );
    assert_eq!(
        raw_write(&mut v, fat + fat_sectors, &[0u8; SECTOR]),
        Err(BlockError::ReadOnly),
        "the second FAT copy is protected too"
    );
    // The MBR's other bytes (Windows' disk signature) may change.
    let mut mbr = sector(&v, 0);
    mbr[440] = 0x5A;
    raw_write(&mut v, 0, &mbr).unwrap();
}

#[test]
fn a_fragmented_boot_file_is_protected_run_by_run() {
    // Blocked clusters split the loader into runs.
    let (mut v, loader) = card(&[5, 8, 9], 6000);
    let mut scratch = vec![0u8; 4096];
    assert!(
        v.put("retropane.bin", &pattern(100, 0), &mut scratch)
            .is_err()
    );
    assert!(v.remove("retropane.bin").is_err());
    assert_eq!(read(&mut v, "retropane.bin"), loader);
    // Other files still come and go in the space around it.
    v.put("other.bin", &pattern(20_000, 4), &mut scratch)
        .unwrap();
    v.remove("other.bin").unwrap();
}

/// A multi-sector write is judged sector by sector: a protected sector
/// hidden in a long run of legitimate ones still refuses the whole write.
#[test]
fn a_protected_sector_inside_a_long_write_is_caught() {
    let (mut v, _) = card(&[], 3000);
    let first = v.find("retropane.bin").unwrap().first_cluster;
    let lba = v.cluster_lba(first) - 4;
    let mut run = vec![0u8; 16 * SECTOR];
    v.dev.read_blocks(lba, &mut run).unwrap();
    run[5 * SECTOR] ^= 1;
    assert_eq!(raw_write(&mut v, lba, &run), Err(BlockError::ReadOnly));
}

#[test]
fn a_file_too_fragmented_to_record_gives_no_guard() {
    let blocked: Vec<u32> = (5..60).step_by(2).collect();
    let loader = pattern(20 * 1024, 7);
    let img = format::build(&Image {
        total_sectors: 2048 + 2048,
        sectors_per_cluster: 2,
        files: &[Entry {
            name: "retropane.bin",
            data: &loader,
        }],
        blocked_clusters: &blocked,
        ballast: 0,
    });
    let mut v = Volume::mount(
        Guarded {
            data: img,
            guard: None,
            refused: 0,
        },
        PARTITION_START,
    )
    .unwrap();
    assert!(
        v.guard(&BOOT).is_err(),
        "the caller keeps the card read-only"
    );
}

/// Relinking one cluster in the middle of a boot file's chain (in either
/// FAT copy), with everything else in the sector unchanged, is refused.
#[test]
fn a_boot_files_chain_cannot_be_relinked() {
    let (mut v, _) = card(&[], 3000);
    let first = v.find("retropane.bin").unwrap().first_cluster;
    for copy in 0..2 {
        let lba = v.fat_start + copy * v.fat_sectors + first / FAT_PER_SECTOR;
        let mut s = sector(&v, lba);
        let o = (first % FAT_PER_SECTOR) as usize * 4;
        s[o..o + 4].copy_from_slice(&(first + 40).to_le_bytes());
        assert_eq!(
            raw_write(&mut v, lba, &s),
            Err(BlockError::ReadOnly),
            "copy {copy}"
        );
    }
}

#[test]
fn a_boot_files_chain_end_cannot_be_moved() {
    // 3000 bytes in 1 KiB clusters, written in one run: the third cluster
    // ends the chain. Pointing it on (the file would grow into another
    // file's clusters) is refused, in both copies of the FAT.
    let (mut v, _) = card(&[], 3000);
    let last = v.find("retropane.bin").unwrap().first_cluster + 2;
    for copy in 0..2 {
        let lba = v.fat_start + copy * v.fat_sectors + last / FAT_PER_SECTOR;
        let mut s = sector(&v, lba);
        let o = (last % FAT_PER_SECTOR) as usize * 4;
        assert!(
            u32::from_le_bytes([s[o], s[o + 1], s[o + 2], s[o + 3]]) & 0x0FFF_FFFF >= 0x0FFF_FFF8,
            "the chain's end"
        );
        s[o..o + 4].copy_from_slice(&(last + 40).to_le_bytes());
        assert_eq!(
            raw_write(&mut v, lba, &s),
            Err(BlockError::ReadOnly),
            "copy {copy}"
        );
    }
}

/// A write the USB host aims at the last block numbers: a run past
/// u32::MAX is refused, not wrapped onto the card's first sectors.
#[test]
fn a_write_wrapping_past_the_last_block_is_refused() {
    let (v, _) = card(&[], 3000);
    // Past the card, then the MBR as it is: wrapped, the second sector
    // would land on block 0 with its partition table unchanged (allowed).
    let mut run = vec![0u8; SECTOR];
    run.extend_from_slice(&sector(&v, 0));
    let g = v.dev.guard.as_ref().unwrap();
    assert!(g.allows(0, &run[SECTOR..]), "the MBR unchanged is allowed");
    assert!(!g.allows(u32::MAX, &run));
    assert!(!g.allows(u32::MAX - 1, &[0u8; 3 * SECTOR]));
}

/// The CF card's boot files live in a folder: the folder can't be renamed
/// (or so deleted), its boot files can't be changed, and everything else in
/// it and on the card still changes.
#[test]
fn a_boot_folder_keeps_its_entry_and_its_files() {
    const CF_BOOT: [&str; 5] = [
        "RetroPane",
        "RetroPane/retropane.bin",
        "RetroPane/haret.exe",
        "RetroPane/startup.txt",
        "RetroPane/default.txt",
    ];
    let (mut v, _) = card(&[], 3000);
    v.dev.guard = None;
    let mut scratch = vec![0u8; 4096];
    let loader = pattern(2500, 5);
    v.put("RetroPane/retropane.bin", &loader, &mut scratch)
        .unwrap();
    v.put("RetroPane/haret.exe", &pattern(5000, 6), &mut scratch)
        .unwrap();
    v.put(
        "RetroPane/startup.txt",
        b"boot
",
        &mut scratch,
    )
    .unwrap();
    let g = v.guard(&CF_BOOT).unwrap();
    assert_eq!(
        g.files(),
        4,
        "the folder and three files (default.txt absent)"
    );
    v.dev.guard = Some(g);
    // What the folder holds otherwise changes, many files at once too (the
    // folder's chain grows).
    for i in 0..40 {
        v.put(&format!("RetroPane/note{i}.txt"), b"x", &mut scratch)
            .unwrap();
    }
    v.remove("RetroPane/note3.txt").unwrap();
    v.put("elsewhere.txt", b"y", &mut scratch).unwrap();
    assert_eq!(v.dev.refused, 0);
    // The boot files: not replaced, not deleted.
    assert!(
        v.put("RetroPane/retropane.bin", b"z", &mut scratch)
            .is_err()
    );
    assert!(v.remove("RetroPane/haret.exe").is_err());
    // The folder: not renamed.
    let f = v.find_file_or_folder("RetroPane").unwrap();
    let mut s = sector(&v, f.entry_lba);
    let o = usize::from(f.entry_index) * 32;
    s[o..o + 8].copy_from_slice(b"RETROPAX");
    assert_eq!(
        raw_write(&mut v, f.entry_lba, &s),
        Err(BlockError::ReadOnly)
    );
    assert_eq!(read(&mut v, "RetroPane/retropane.bin"), loader);
}

// --- Every format (ADR-0036 point 2) ---------------------------------------

use crate::AnyVolume;
use crate::mkfs::{self, Fs};

/// A card formatted by `mkfs` as `fs`, the boot files and a README put on
/// it, the guard built and armed; its partition's first sector. With
/// `scatter`, the card is filled first and only one-cluster holes are left
/// for the loader, so its chain is several runs.
fn formatted(fs: Fs, total: u32, cluster: u32, scatter: bool) -> (Guarded, u32, Vec<u8>) {
    let l = mkfs::plan(
        total,
        &mkfs::Options {
            fs: Some(fs),
            cluster_bytes: Some(cluster),
            align_sectors: None,
            label: "X51V",
            serial: 7,
        },
    )
    .unwrap();
    let mut dev = Guarded {
        data: vec![0u8; total as usize * SECTOR],
        guard: None,
        refused: 0,
    };
    mkfs::write(&mut dev, &l, &mut [0u8; 8192]).unwrap();
    let cb = cluster as usize;
    let loader = pattern(5 * cb + 100, 7); // six clusters
    let mut scratch = vec![0u8; 8192];
    let g = {
        let mut v = AnyVolume::mount(&mut dev, l.part_start).unwrap();
        v.put("haret.exe", &pattern(3000, 3), &mut scratch).unwrap();
        v.put("startup.txt", b"set ramaddr 0xa8000000\n", &mut scratch)
            .unwrap();
        v.put("README.txt", b"hello", &mut scratch).unwrap();
        if scatter {
            for i in 0..16 {
                v.put(&format!("fill/h{i}"), &pattern(cb, i), &mut scratch)
                    .unwrap();
            }
            for size in [1024 * cb, 64 * cb, cb] {
                for i in 0.. {
                    let name = format!("fill/f{size}-{i}");
                    if v.put(&name, &pattern(size, 9), &mut scratch).is_err() {
                        break;
                    }
                }
            }
            // Eight holes: six for the loader, room for its entry.
            for i in (1..16).step_by(2) {
                v.remove(&format!("fill/h{i}")).unwrap();
            }
        }
        v.put("retropane.bin", &loader, &mut scratch).unwrap();
        let g = v.guard(&BOOT).unwrap();
        assert_eq!(g.files(), 3, "{fs:?}: default.txt is absent and skipped");
        g
    };
    dev.guard = Some(g);
    (dev, l.part_start, loader)
}

fn read_any(dev: &mut Guarded, base: u32, name: &str) -> Vec<u8> {
    let mut v = AnyVolume::mount(dev, base).unwrap();
    let mut out = Vec::new();
    v.read_back(name, &mut [0u8; 4096], &mut |c| out.extend_from_slice(c))
        .unwrap();
    out
}

const FORMATS: [(Fs, u32, u32); 4] = [
    (Fs::Fat12, 4096, 512),
    (Fs::Fat16, 20_000, 512),
    (Fs::Fat32, 140_000, 512),
    (Fs::ExFat, 8192, 512),
];

#[test]
fn every_format_lets_everyday_changes_through() {
    for (fs, total, cluster) in FORMATS {
        let (mut dev, base, loader) = formatted(fs, total, cluster, false);
        let mut scratch = vec![0u8; 8192];
        {
            let mut v = AnyVolume::mount(&mut dev, base).unwrap();
            v.put("photos/cat.jpg", &pattern(9000, 1), &mut scratch)
                .unwrap();
            v.put("README.txt", b"changed", &mut scratch).unwrap();
            v.put("notes.txt", b"x", &mut scratch).unwrap();
            v.remove("notes.txt").unwrap();
            v.remove("photos/cat.jpg").unwrap();
        }
        assert_eq!(dev.refused, 0, "{fs:?}");
        assert_eq!(read_any(&mut dev, base, "retropane.bin"), loader, "{fs:?}");
        assert_eq!(read_any(&mut dev, base, "README.txt"), b"changed");
    }
}

#[test]
fn every_format_refuses_deleting_replacing_or_formatting_a_boot_file() {
    for (fs, total, cluster) in FORMATS {
        let (mut dev, base, loader) = formatted(fs, total, cluster, false);
        let mut scratch = vec![0u8; 8192];
        {
            let mut v = AnyVolume::mount(&mut dev, base).unwrap();
            assert!(v.remove("retropane.bin").is_err(), "{fs:?}");
            assert!(v.remove("haret.exe").is_err(), "{fs:?}");
            assert!(
                v.put("retropane.bin", &pattern(4000, 9), &mut scratch)
                    .is_err()
            );
        }
        // A format from the PC, to any format, at the same place.
        for to in [Fs::Fat12, Fs::Fat16, Fs::Fat32, Fs::ExFat] {
            let o = mkfs::Options {
                fs: Some(to),
                cluster_bytes: Some(cluster),
                align_sectors: Some(base),
                label: "",
                serial: 1,
            };
            if let Ok(l) = mkfs::plan(total, &o) {
                assert!(
                    mkfs::write(&mut dev, &l, &mut scratch).is_err(),
                    "{fs:?} to {to:?}"
                );
            }
        }
        assert!(dev.refused >= 3);
        assert_eq!(read_any(&mut dev, base, "retropane.bin"), loader, "{fs:?}");
        assert_eq!(read_any(&mut dev, base, "haret.exe"), pattern(3000, 3));
    }
}

#[test]
fn every_format_refuses_a_boot_files_data_and_its_partition_table() {
    for (fs, total, cluster) in FORMATS {
        let (mut dev, base, _) = formatted(fs, total, cluster, false);
        let mut mbr = dev.data[..SECTOR].to_vec();
        // The MBR's boot code may change, the table not.
        mbr[0] = 0xFA;
        assert!(dev.write_blocks(0, &mbr).is_ok(), "{fs:?}");
        mbr[446 + 8] ^= 1;
        assert!(dev.write_blocks(0, &mbr).is_err(), "{fs:?}");
        // A sector inside the loader's data, alone and inside a long write.
        let at = {
            let mut v = AnyVolume::mount(&mut dev, base).unwrap();
            v.extent("retropane.bin").unwrap().lba
        };
        assert!(dev.write_blocks(at + 1, &[0u8; SECTOR]).is_err(), "{fs:?}");
        assert!(dev.write_blocks(at - 4, &vec![0u8; 16 * SECTOR]).is_err());
        // The gap before the partition.
        assert!(dev.write_blocks(base - 1, &[0u8; SECTOR]).is_err());
    }
}

#[test]
fn every_format_protects_a_scattered_boot_file_run_by_run() {
    for (fs, total, cluster) in FORMATS {
        let (mut dev, base, loader) = formatted(fs, total, cluster, true);
        let spc = cluster / SECTOR as u32;
        // Where each of the loader's clusters is, read without the guard.
        let mut lbas = Vec::new();
        {
            let g = dev.guard.take();
            match AnyVolume::mount(&mut dev, base).unwrap() {
                AnyVolume::Fat(mut v) => {
                    let mut c = v.find("retropane.bin").unwrap().first_cluster();
                    loop {
                        lbas.push(v.cluster_lba(c));
                        match v.next_cluster(c).unwrap() {
                            Some(n) => c = n,
                            None => break,
                        }
                    }
                }
                AnyVolume::ExFat(_) => {}
            }
            dev.guard = g;
        }
        if let Some(crate::guard::Kind::Ex(ex)) = dev.guard.as_ref().map(|g| &g.kind) {
            let p = ex
                .files
                .iter()
                .take(ex.count)
                .find(|p| p.set[2][2] == b'r')
                .unwrap();
            assert!(p.chained, "exFAT: chained through the FAT");
            for &(s, n) in &p.extents[..p.runs] {
                for c in s..s + n {
                    lbas.push(ex.heap_start + ((c - 2) << ex.cluster_shift));
                }
            }
        }
        assert_eq!(lbas.len(), 6, "{fs:?}");
        assert!(
            lbas.windows(2).any(|w| w[1] != w[0] + spc),
            "{fs:?}: scattered"
        );
        for &lba in &lbas {
            assert!(
                dev.write_blocks(lba, &[0xAA; SECTOR]).is_err(),
                "{fs:?} {lba}"
            );
        }
        assert_eq!(read_any(&mut dev, base, "retropane.bin"), loader, "{fs:?}");
    }
}

/// A FAT12 card whose loader's chain has an entry straddling FAT sectors 0
/// and 1 (cluster 341: bytes 511 and 512), and none at cluster 340.
#[test]
fn fat12_a_write_covering_half_a_protected_entry_is_judged_on_that_half() {
    let (mut dev, base, _) = formatted(Fs::Fat12, 4096, 512, false);
    dev.guard = None;
    let mut scratch = vec![0u8; 8192];
    let g = {
        let mut v = AnyVolume::mount(&mut dev, base).unwrap();
        for name in ["retropane.bin", "haret.exe", "startup.txt", "README.txt"] {
            v.remove(name).unwrap();
        }
        // Clusters 2 to 340 for a filler, the loader from 341.
        v.put("filler.bin", &pattern(339 * 512, 4), &mut scratch)
            .unwrap();
        v.put("retropane.bin", &pattern(4 * 512, 5), &mut scratch)
            .unwrap();
        v.guard(&BOOT).unwrap()
    };
    dev.guard = Some(g);
    let (fat_start, fat_sectors) = {
        let mut v = Volume::mount(&mut dev, base).unwrap();
        assert_eq!(v.find("retropane.bin").unwrap().first_cluster(), 341);
        (v.fat_start, v.fat_sectors)
    };
    let sector = |dev: &Guarded, lba: u32| dev.data[lba as usize * SECTOR..][..SECTOR].to_vec();
    let (s0, s1) = (sector(&dev, fat_start), sector(&dev, fat_start + 1));
    // Byte 511: its low nibble is cluster 340's (the filler's end), its
    // high nibble cluster 341's lowest bits; byte 512 (the next sector's
    // first) the rest of 341.
    let mut w = s0.clone();
    w[511] ^= 0x01;
    assert!(
        dev.write_blocks(fat_start, &w).is_ok(),
        "340 is not protected"
    );
    let mut w = s0.clone();
    w[511] ^= 0x10;
    assert!(dev.write_blocks(fat_start, &w).is_err(), "341's low bits");
    let mut w = s1.clone();
    w[0] ^= 0x80;
    assert!(
        dev.write_blocks(fat_start + 1, &w).is_err(),
        "341's high bits"
    );
    // The second FAT copy alike.
    assert!(dev.write_blocks(fat_start + fat_sectors + 1, &w).is_err());
    // Unchanged sectors pass.
    assert!(dev.write_blocks(fat_start, &s0).is_ok());
    assert!(dev.write_blocks(fat_start + 1, &s1).is_ok());
}

#[test]
fn fat16_the_fixed_root_keeps_boot_entries_and_takes_new_ones() {
    let (mut dev, base, _) = formatted(Fs::Fat16, 20_000, 512, false);
    let (root, offset) = {
        let mut v = Volume::mount(&mut dev, base).unwrap();
        let f = v.find("haret.exe").unwrap();
        assert!(
            f.entry_lba >= v.root_lba && f.entry_lba < v.data_start,
            "in the fixed root"
        );
        (f.entry_lba, usize::from(f.entry_index) * 32)
    };
    let s = dev.data[root as usize * SECTOR..][..SECTOR].to_vec();
    let mut w = s.clone();
    w[offset] = 0xE5; // deleted
    assert!(dev.write_blocks(root, &w).is_err());
    let mut w = s.clone();
    w[offset + 3] = b'Z'; // renamed
    assert!(dev.write_blocks(root, &w).is_err());
    // A new entry in a free slot of the same sector passes.
    let mut w = s.clone();
    let free = (0..16).find(|&i| w[i * 32] == 0).unwrap();
    w[free * 32..free * 32 + 11].copy_from_slice(b"NEW     TXT");
    w[free * 32 + 11] = 0x20;
    assert!(dev.write_blocks(root, &w).is_ok());
    // The boot sector's label may change, its layout not.
    let bs = base as usize * SECTOR;
    let mut b = dev.data[bs..bs + SECTOR].to_vec();
    b[43..54].copy_from_slice(b"RENAMED    ");
    assert!(dev.write_blocks(base, &b).is_ok());
    b[13] = 8; // sectors per cluster
    assert!(dev.write_blocks(base, &b).is_err());
}

#[test]
fn exfat_flags_and_timestamps_pass_names_bitmap_and_boot_region_do_not() {
    let (mut dev, base, _) = formatted(Fs::ExFat, 8192, 512, false);
    let bs = base as usize * SECTOR;
    // VolumeFlags and PercentInUse: Windows' own.
    let mut b = dev.data[bs..bs + SECTOR].to_vec();
    b[106] |= 0x02;
    b[112] = 40;
    assert!(dev.write_blocks(base, &b).is_ok());
    assert!(dev.write_blocks(base + 12, &b).is_ok(), "the backup too");
    // The layout, an extended boot sector, the checksum: refused.
    let mut w = b.clone();
    w[109] += 1; // cluster size
    assert!(dev.write_blocks(base, &w).is_err());
    let mut e = dev.data[bs + SECTOR..bs + 2 * SECTOR].to_vec();
    e[0] = 1;
    assert!(dev.write_blocks(base + 1, &e).is_err());
    assert!(dev.write_blocks(base + 13, &e).is_err());
    assert!(dev.write_blocks(base + 11, &[0u8; SECTOR]).is_err());
    // The loader's entry set: timestamps (and the checksum with them) pass;
    // its deletion, name and size don't.
    let g = dev.guard.clone().unwrap();
    let crate::guard::Kind::Ex(ex) = &g.kind else {
        panic!("an exFAT guard")
    };
    let p = ex
        .files
        .iter()
        .take(ex.count)
        .find(|p| p.set[2][2] == b'r')
        .unwrap();
    let at = |dev: &Guarded, k: usize| {
        let (lba, o) = p.at[k];
        (
            lba,
            usize::from(o),
            dev.data[lba as usize * SECTOR..][..SECTOR].to_vec(),
        )
    };
    let (lba, o, s) = at(&dev, 0);
    let mut w = s.clone();
    w[o + 8..o + 12].copy_from_slice(&0x5B21_7A00u32.to_le_bytes()); // created
    w[o + 2] ^= 0x55; // the set checksum follows
    assert!(dev.write_blocks(lba, &w).is_ok(), "timestamps");
    let mut w = s.clone();
    w[o] &= 0x7F; // deleted
    assert!(dev.write_blocks(lba, &w).is_err());
    let (lba, o, mut w) = at(&dev, 2);
    w[o + 2] = b'R'; // renamed
    assert!(dev.write_blocks(lba, &w).is_err());
    let (lba, o, mut w) = at(&dev, 1);
    w[o + 24] ^= 1; // its size
    assert!(dev.write_blocks(lba, &w).is_err());
    // Its clusters' bits in the bitmap stay set; others may change.
    let (bm, _) = ex.bitmap;
    let first = p.extents[0].0;
    let mut w = dev.data[bm as usize * SECTOR..][..SECTOR].to_vec();
    let bit = (first - 2) as usize;
    w[bit / 8] &= !(1 << (bit % 8));
    assert!(dev.write_blocks(bm, &w).is_err());
    let mut w = dev.data[bm as usize * SECTOR..][..SECTOR].to_vec();
    w[SECTOR - 1] |= 0x80; // a free cluster near the end
    assert!(dev.write_blocks(bm, &w).is_ok());
}

#[test]
fn exfat_a_chained_boot_file_keeps_its_links() {
    let (mut dev, _, _) = formatted(Fs::ExFat, 8192, 512, true);
    let g = dev.guard.clone().unwrap();
    let crate::guard::Kind::Ex(ex) = &g.kind else {
        panic!("an exFAT guard")
    };
    let p = ex
        .files
        .iter()
        .take(ex.count)
        .find(|p| p.set[2][2] == b'r')
        .unwrap();
    assert!(p.chained && p.runs > 1, "scattered");
    let (s, n) = p.extents[0];
    let last = s + n - 1;
    let lba = ex.fat_start + last / 128;
    let o = (last % 128) as usize * 4;
    let mut w = dev.data[lba as usize * SECTOR..][..SECTOR].to_vec();
    w[o..o + 4].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // cut short
    assert!(dev.write_blocks(lba, &w).is_err());
    // Its end moved: a link where the chain ended.
    let (s, n) = p.extents[p.runs - 1];
    let end = s + n - 1;
    let lba = ex.fat_start + end / 128;
    let o = (end % 128) as usize * 4;
    let mut w = dev.data[lba as usize * SECTOR..][..SECTOR].to_vec();
    w[o..o + 4].copy_from_slice(&(end + 1).to_le_bytes());
    assert!(dev.write_blocks(lba, &w).is_err());
}

#[test]
fn an_exfat_boot_file_with_a_long_name_gives_no_guard() {
    let (mut dev, base, _) = formatted(Fs::ExFat, 8192, 512, false);
    dev.guard = None;
    let mut v = AnyVolume::mount(&mut dev, base).unwrap();
    v.put("RetroPane/a boot file name.bin", b"x", &mut [0u8; 4096])
        .unwrap();
    assert!(v.guard(&["RetroPane/a boot file name.bin"]).is_err());
    assert!(
        v.guard(&["RetroPane"]).is_ok(),
        "a short folder name is fine"
    );
}

/// The FAT sectors that differ between two card images.
fn changed_fat_sectors(before: &[u8], after: &[u8]) -> Vec<(u32, Vec<u8>)> {
    before
        .chunks(SECTOR)
        .zip(after.chunks(SECTOR))
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, (_, b))| (i as u32, b.to_vec()))
        .collect()
}

#[test]
fn every_fat_width_keeps_a_boot_files_chain() {
    // As Windows would: the change made on a copy with no guard, then its
    // FAT sectors written to the guarded card. Cutting the loader's chain
    // short and moving its end (a link where the end was) are refused on
    // FAT12, FAT16 and FAT32; the same change to another file passes.
    for (fs, total, cluster) in &FORMATS[..3] {
        let (mut dev, base, _) = formatted(*fs, *total, *cluster, false);
        let attempt = |dev: &mut Guarded,
                       name: &str,
                       change: &dyn Fn(&mut Volume<&mut Guarded>, u32, u32)| {
            let before = dev.data.clone();
            let mut copy = Guarded {
                data: before.clone(),
                guard: None,
                refused: 0,
            };
            {
                let mut v = Volume::mount(&mut copy, base).unwrap();
                let f = v.find(name).unwrap();
                let n = f.size().div_ceil(*cluster);
                change(&mut v, f.first_cluster(), n);
                v.fat_flush().unwrap();
            }
            let sectors = changed_fat_sectors(&before, &copy.data);
            assert!(!sectors.is_empty());
            sectors
                .iter()
                .all(|(lba, s)| dev.write_blocks(*lba, s).is_ok())
        };
        let cut: &dyn Fn(&mut Volume<&mut Guarded>, u32, u32) = &|v, first, _| {
            v.fat_set(first, crate::EOC).unwrap();
        };
        let moved: &dyn Fn(&mut Volume<&mut Guarded>, u32, u32) = &|v, first, n| {
            let free = v.clusters() + 1; // the last cluster, free on this card
            v.fat_set(first + n - 1, free).unwrap();
            v.fat_set(free, crate::EOC).unwrap();
        };
        assert!(
            !attempt(&mut dev, "retropane.bin", cut),
            "{fs:?}: chain cut short"
        );
        assert!(
            !attempt(&mut dev, "retropane.bin", moved),
            "{fs:?}: chain end moved"
        );
        assert!(
            attempt(&mut dev, "README.txt", moved),
            "{fs:?}: another file's chain"
        );
    }
}

/// The drive as the loader serves it (ADR-0014 rule 3): reads shown through
/// the guard's view, writes judged by the guard and put back to the card's
/// real attributes.
struct Viewed<'a>(&'a mut Guarded);

impl BlockDevice for Viewed<'_> {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
        self.0.read_blocks(lba, buf)?;
        if let Some(g) = &self.0.guard {
            g.view(lba, buf);
        }
        Ok(())
    }
    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
        let Some(g) = self.0.guard.clone() else {
            return self.0.write_blocks(lba, buf);
        };
        if !g.allows(lba, buf) {
            self.0.refused += 1;
            return Err(BlockError::ReadOnly);
        }
        let mut real = buf.to_vec();
        g.unview(lba, &mut real);
        let o = lba as usize * SECTOR;
        self.0.data[o..o + real.len()].copy_from_slice(&real);
        Ok(())
    }
}

/// The attributes the protected `name`'s entry holds in `data` (FAT: the
/// short entry's byte 11; exFAT: the file entry's bytes 4-5), and, for
/// exFAT, whether its set checksum holds.
fn attrs_of(data: &[u8], base: u32, name: &str, fs: Fs) -> (u16, bool) {
    let mut dev = Guarded {
        data: data.to_vec(),
        guard: None,
        refused: 0,
    };
    match fs {
        Fs::ExFat => {
            let mut v = crate::exfat::ExVolume::mount(&mut dev, base).unwrap();
            let g = v.guard(&[name]).unwrap();
            let Kind::Ex(ex) = g.kind else { unreachable!() };
            let p = ex.files[0];
            let (lba, o) = (p.at[0].0 as usize, usize::from(p.at[0].1));
            let e: [u8; 32] = data[lba * SECTOR + o..][..32].try_into().unwrap();
            let mut set = p.set;
            set[0] = e;
            let sum = crate::exfat::set_checksum(&set[..p.entries]);
            (
                u16::from_le_bytes([e[4], e[5]]),
                sum == u16::from_le_bytes([e[2], e[3]]),
            )
        }
        _ => {
            let mut v = Volume::mount(&mut dev, base).unwrap();
            let f = v.find_file_or_folder(name).unwrap();
            let at = f.entry_lba as usize * SECTOR + usize::from(f.entry_index) * 32;
            (u16::from(data[at + 11]), true)
        }
    }
}

#[test]
fn the_pc_sees_boot_files_hidden_and_the_card_keeps_them_as_they_are() {
    for (fs, total, cluster) in FORMATS {
        let (mut dev, base, loader) = formatted(fs, total, cluster, false);
        let (real, ok) = attrs_of(&dev.data, base, "haret.exe", fs);
        assert!(ok);
        assert_eq!(real & 0x07, 0, "{fs:?}: an ordinary file on the card");
        let mut scratch = vec![0u8; 8192];
        {
            let mut pc = Viewed(&mut dev);
            // What the PC reads: hidden + system + read-only, a valid set.
            let mut seen = Vec::new();
            {
                let mut v = AnyVolume::mount(&mut pc, base).unwrap();
                v.read_back("haret.exe", &mut scratch, &mut |c| {
                    seen.extend_from_slice(c)
                })
                .unwrap();
                assert_eq!(
                    seen,
                    pattern(3000, 3),
                    "{fs:?}: found and read through the view"
                );
                // A busy session: new files beside the boot entries (their
                // directory sectors written back as the PC sees them), a
                // delete, a replacement.
                for i in 0..30 {
                    v.put(
                        &format!("photo {i}.jpg"),
                        &pattern(700, i as u8),
                        &mut scratch,
                    )
                    .unwrap();
                }
                v.remove("README.txt").unwrap();
                v.put("photo 3.jpg", b"replaced", &mut scratch).unwrap();
            }
            assert_eq!(pc.0.refused, 0, "{fs:?}: nothing refused");
        }
        // The PC's view of the directory, read raw through the drive.
        let mut shown_data = dev.data.clone();
        let mut view_dev = Guarded {
            data: dev.data.clone(),
            guard: dev.guard.clone(),
            refused: 0,
        };
        let n = (shown_data.len() / SECTOR) as u32;
        for lba in 0..n {
            let o = lba as usize * SECTOR;
            let mut s = [0u8; SECTOR];
            view_dev.read_blocks(lba, &mut s).unwrap();
            dev.guard.as_ref().unwrap().view(lba, &mut s);
            shown_data[o..o + SECTOR].copy_from_slice(&s);
        }
        let (shown, ok) = attrs_of(&shown_data, base, "haret.exe", fs);
        assert!(ok, "{fs:?}: the set checksum holds in the PC's view");
        assert_eq!(
            shown & 0x07,
            0x07,
            "{fs:?}: hidden, system, read-only to the PC"
        );
        // The card: the real attributes, a clean volume, the files whole.
        let (after, ok) = attrs_of(&dev.data, base, "haret.exe", fs);
        assert!(ok, "{fs:?}: the card's set checksum");
        assert_eq!(after, real, "{fs:?}: the card's attributes as they were");
        assert_eq!(read_any(&mut dev, base, "retropane.bin"), loader);
        assert_eq!(read_any(&mut dev, base, "photo 3.jpg"), b"replaced");
        let mut v = AnyVolume::mount(&mut dev, base).unwrap();
        let h = v.check(&mut vec![0u8; 1 << 16], &mut scratch).unwrap();
        assert!(h.clean(), "{fs:?}: {h:?}");
    }
}

#[test]
fn only_the_shown_attributes_are_taken_back_from_the_pc() {
    for (fs, total, cluster) in [FORMATS[1], FORMATS[3]] {
        let (mut dev, base, _) = formatted(fs, total, cluster, false);
        let g = dev.guard.clone().unwrap();
        // The haret.exe entry's sector and where its attributes are.
        let (lba, at) = match &g.kind {
            Kind::Ex(ex) => {
                let p = ex
                    .files
                    .iter()
                    .take(ex.count)
                    .find(|p| p.set[2][2] == b'h')
                    .unwrap();
                (p.at[0].0, usize::from(p.at[0].1) + 4)
            }
            Kind::Fat(_) => {
                let mut v = Volume::mount(&mut dev, base).unwrap();
                let f = v.find("haret.exe").unwrap();
                (f.entry_lba, usize::from(f.entry_index) * 32 + 11)
            }
        };
        let mut s = dev.data[lba as usize * SECTOR..][..SECTOR].to_vec();
        let real = s[at];
        g.view(lba, &mut s);
        assert_eq!(s[at], real | 0x07);
        // As shown, or as the card has it: both taken (written back real).
        assert!(g.allows(lba, &s));
        let mut w = s.clone();
        g.unview(lba, &mut w);
        assert_eq!(w[at], real);
        assert_eq!(
            &w,
            &dev.data[lba as usize * SECTOR..][..SECTOR],
            "unview gives the card's bytes"
        );
        // Anything else: refused (read-only cleared, the archive bit flipped,
        // only read-only set).
        for other in [real | 0x06, (real ^ 0x20) | 0x07, real | 0x01] {
            let mut x = s.clone();
            x[at] = other;
            if fs == Fs::ExFat {
                // The checksum made to match, as a PC would.
                let Kind::Ex(ex) = &g.kind else {
                    unreachable!()
                };
                let p = ex
                    .files
                    .iter()
                    .take(ex.count)
                    .find(|p| p.set[2][2] == b'h')
                    .unwrap();
                let o = usize::from(p.at[0].1);
                let mut set = p.set;
                set[0].copy_from_slice(&x[o..o + 32]);
                let sum = crate::exfat::set_checksum(&set[..p.entries]);
                x[o + 2..o + 4].copy_from_slice(&sum.to_le_bytes());
            }
            assert!(!g.allows(lba, &x), "{fs:?}: attributes {other:#x} refused");
        }
        assert!(g.touches(lba, 1) && !g.touches(lba + 1000, 1));
    }
}
