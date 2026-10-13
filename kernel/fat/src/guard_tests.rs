//! The guard against what a PC does to a mounted card: this crate's own
//! FAT writer stands in for Windows (it writes entries, chains in both FATs
//! and data the same way), plus raw sector writes for a label change, a
//! rename and a format.

use rp_hal_api::BlockError;

use super::*;
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
