//! A small FAT32 formatter (host only): builds a whole disk image, MBR and
//! one partition, with files in the root directory. For the crate's tests
//! and the emulator's card image; not a general tool.
//!
//! [`write_sparse`] emits only the non-zero parts (MBR, boot sectors, the
//! FATs' used sectors, the root directory, file data), so a card image as
//! large as a real card (4 or 32 GB) can be a sparse file of a few MB, and
//! [`Image::ballast`] fills most of it with allocated clusters whose data is
//! never written: the FAT is as full as on a well-used card.

use crate::{SECTOR, short_checksum};

/// Where the partition starts (what Windows and cameras use).
pub const PARTITION_START: u32 = 2048;
const RESERVED: u32 = 32;
const FATS: u32 = 2;
const ROOT_CLUSTER: u32 = 2;

/// A file to put in the root directory.
pub struct Entry<'a> {
    /// Its name (ASCII; a long name entry is written when it isn't 8.3).
    pub name: &'a str,
    /// Its contents.
    pub data: &'a [u8],
}

/// Image parameters.
pub struct Image<'a> {
    /// Total sectors on the disk (partition = all but the first 2048).
    pub total_sectors: u32,
    /// Sectors per cluster (power of two).
    pub sectors_per_cluster: u32,
    /// Root-directory files, placed in order from the first free cluster.
    pub files: &'a [Entry<'a>],
    /// Clusters to mark as in use by nobody (a bad-cluster mark), so tests
    /// can make the free space fragmented.
    pub blocked_clusters: &'a [u32],
    /// Bytes of ballast after the files: `BALLAST0.BIN`, `BALLAST1.BIN`...
    /// (each under 4 GiB, FAT32's file limit), their clusters allocated
    /// and chained, their contents zero and never written by
    /// [`write_sparse`]. 0 for none.
    pub ballast: u64,
}

fn put16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

/// Whether `name` is already a valid upper-case 8.3 name.
fn is_short(name: &str) -> bool {
    let (base, ext) = name.split_once('.').unwrap_or((name, ""));
    let ok = |s: &str| {
        s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    };
    !base.is_empty()
        && base.len() <= 8
        && ext.len() <= 3
        && ok(base)
        && ok(ext)
        && !name.ends_with('.')
}

/// The 11-byte entry name for `name` (`~n` form for long names).
fn short_name(name: &str, n: usize) -> [u8; 11] {
    let mut out = [b' '; 11];
    let (base, ext) = name.split_once('.').unwrap_or((name, ""));
    if is_short(name) {
        out[..base.len()].copy_from_slice(base.as_bytes());
        out[8..8 + ext.len()].copy_from_slice(ext.as_bytes());
        return out;
    }
    let clean: Vec<u8> = base
        .bytes()
        .filter(|b| b.is_ascii_alphanumeric())
        .map(|b| b.to_ascii_uppercase())
        .take(6)
        .collect();
    let tail = format!("~{n}");
    out[..clean.len()].copy_from_slice(&clean);
    out[clean.len()..clean.len() + tail.len()].copy_from_slice(tail.as_bytes());
    let e: Vec<u8> = ext
        .bytes()
        .filter(|b| b.is_ascii_alphanumeric())
        .map(|b| b.to_ascii_uppercase())
        .take(3)
        .collect();
    out[8..8 + e.len()].copy_from_slice(&e);
    out
}

/// Builds the image.
///
/// # Panics
/// If the files don't fit, or a name is too long for one directory cluster.
#[must_use]
pub fn build(img: &Image<'_>) -> Vec<u8> {
    let mut disk = vec![0u8; img.total_sectors as usize * SECTOR];
    write_sparse(img, &mut |off, data| {
        let o = off as usize;
        disk[o..o + data.len()].copy_from_slice(data);
    });
    disk
}

/// Emits the image's non-zero parts as `(byte offset, bytes)`, in no
/// particular order; everything not emitted is zero.
///
/// # Panics
/// As [`build`].
pub fn write_sparse(img: &Image<'_>, out: &mut dyn FnMut(u64, &[u8])) {
    let spc = img.sectors_per_cluster;
    let part_sectors = img.total_sectors - PARTITION_START;
    // FAT size for a volume of this many data sectors (approximate: a FAT
    // sector covers 128 clusters; round up generously).
    let data_est = part_sectors - RESERVED;
    let fat_sectors = (data_est / spc + 2).div_ceil(128) + 1;
    let data_start = PARTITION_START + RESERVED + FATS * fat_sectors;
    let clusters = (img.total_sectors - data_start) / spc;
    let cluster_bytes = (spc * SECTOR as u32) as usize;
    let sector_at = |sector: u32| u64::from(sector) * SECTOR as u64;

    // MBR.
    let mut mbr = [0u8; SECTOR];
    let e = 446;
    mbr[e + 4] = 0x0C;
    put32(&mut mbr, e + 8, PARTITION_START);
    put32(&mut mbr, e + 12, part_sectors);
    mbr[510] = 0x55;
    mbr[511] = 0xAA;
    out(0, &mbr);

    // Boot sector.
    let mut boot = [0u8; SECTOR];
    let b = &mut boot;
    b[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    b[3..11].copy_from_slice(b"MSWIN4.1");
    put16(b, 11, SECTOR as u16);
    b[13] = spc as u8;
    put16(b, 14, RESERVED as u16);
    b[16] = FATS as u8;
    b[21] = 0xF8;
    put16(b, 24, 63);
    put16(b, 26, 255);
    put32(b, 28, PARTITION_START);
    put32(b, 32, part_sectors);
    put32(b, 36, fat_sectors);
    put32(b, 44, ROOT_CLUSTER);
    put16(b, 48, 1); // FSInfo
    put16(b, 50, 6); // backup boot sector
    b[66] = 0x29;
    put32(b, 67, 0x2026_1011);
    b[71..82].copy_from_slice(b"RETROPANE  ");
    b[82..90].copy_from_slice(b"FAT32   ");
    b[510] = 0x55;
    b[511] = 0xAA;
    out(sector_at(PARTITION_START), &boot);
    out(sector_at(PARTITION_START + 6), &boot);

    // FAT: cluster 0/1 marks, root directory at cluster 2, files after it.
    let mut fat = vec![0u32; (fat_sectors as usize * SECTOR) / 4];
    fat[0] = 0x0FFF_FFF8;
    fat[1] = 0x0FFF_FFFF;
    fat[ROOT_CLUSTER as usize] = 0x0FFF_FFFF;
    for &c in img.blocked_clusters {
        fat[c as usize] = 0x0FFF_FFF7;
    }
    let mut next_free = ROOT_CLUSTER + 1;
    let mut alloc = |n: usize, fat: &mut Vec<u32>| -> Vec<u32> {
        let mut got = Vec::new();
        while got.len() < n {
            assert!(next_free < 2 + clusters, "image too small for its files");
            if fat[next_free as usize] == 0 {
                got.push(next_free);
            }
            next_free += 1;
        }
        for (i, &c) in got.iter().enumerate() {
            fat[c as usize] = got.get(i + 1).copied().unwrap_or(0x0FFF_FFFF);
        }
        got
    };

    // Root directory entries (one cluster: enough for the kit).
    let mut dir = vec![0u8; cluster_bytes];
    let mut at = 0;
    let mut put_entry = |dir: &mut Vec<u8>, e: [u8; 32]| {
        assert!(at + 32 <= dir.len(), "root directory full");
        dir[at..at + 32].copy_from_slice(&e);
        at += 32;
    };
    let mut label = [0u8; 32];
    label[..11].copy_from_slice(b"RETROPANE  ");
    label[11] = 0x08;
    put_entry(&mut dir, label);

    let mut used = 0u32;
    for (n, f) in img.files.iter().enumerate() {
        let short = short_name(f.name, n + 1);
        if !is_short(f.name) {
            let sum = short_checksum(&short);
            let chars: Vec<u16> = f.name.encode_utf16().collect();
            let pieces = chars.len().div_ceil(13);
            for p in (0..pieces).rev() {
                let mut e = [0xFFu8; 32];
                e[0] = (p + 1) as u8 | if p + 1 == pieces { 0x40 } else { 0 };
                e[11] = 0x0F;
                e[12] = 0;
                e[13] = sum;
                put16(&mut e, 26, 0);
                const AT: [usize; 13] = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
                for (i, &o) in AT.iter().enumerate() {
                    let idx = p * 13 + i;
                    let c = match idx.cmp(&chars.len()) {
                        std::cmp::Ordering::Less => chars[idx],
                        std::cmp::Ordering::Equal => 0,
                        std::cmp::Ordering::Greater => 0xFFFF,
                    };
                    put16(&mut e, o, c);
                }
                put_entry(&mut dir, e);
            }
        }
        let n_clusters = f.data.len().div_ceil(cluster_bytes);
        let chain = alloc(n_clusters, &mut fat);
        for (i, &c) in chain.iter().enumerate() {
            let piece = &f.data[i * cluster_bytes..((i + 1) * cluster_bytes).min(f.data.len())];
            out(sector_at(data_start + (c - 2) * spc), piece);
        }
        used += n_clusters as u32;
        let first = chain.first().copied().unwrap_or(0);
        let mut e = [0u8; 32];
        e[..11].copy_from_slice(&short);
        e[11] = 0x20;
        put16(&mut e, 20, (first >> 16) as u16);
        put16(&mut e, 26, first as u16);
        put32(&mut e, 28, f.data.len() as u32);
        put_entry(&mut dir, e);
    }
    // Ballast: files whose clusters are allocated and chained but whose
    // contents (zero) are never written.
    let max_file = 0xFFFF_FFFF / cluster_bytes as u64 * cluster_bytes as u64;
    let mut left = img.ballast;
    let mut k = 0;
    while left > 0 {
        let size = left.min(max_file);
        left -= size;
        let n_clusters = size.div_ceil(cluster_bytes as u64) as usize;
        let first = alloc(n_clusters, &mut fat).first().copied().unwrap_or(0);
        used += n_clusters as u32;
        let mut e = [0u8; 32];
        e[..11].copy_from_slice(format!("BALLAST{k}BIN").as_bytes());
        e[11] = 0x20;
        put16(&mut e, 20, (first >> 16) as u16);
        put16(&mut e, 26, first as u16);
        put32(&mut e, 28, size as u32);
        put_entry(&mut dir, e);
        k += 1;
    }
    out(sector_at(data_start), &dir);

    // The FATs: only the sectors that aren't all zero.
    let mut sector = [0u8; SECTOR];
    for (i, entries) in fat.chunks(SECTOR / 4).enumerate() {
        if entries.iter().all(|&v| v == 0) {
            continue;
        }
        sector.fill(0);
        for (j, v) in entries.iter().enumerate() {
            put32(&mut sector, j * 4, *v);
        }
        for copy in 0..FATS {
            out(
                sector_at(PARTITION_START + RESERVED + copy * fat_sectors + i as u32),
                &sector,
            );
        }
    }

    // FSInfo.
    let mut fsinfo = [0u8; SECTOR];
    let f = &mut fsinfo;
    put32(f, 0, 0x4161_5252);
    put32(f, 484, 0x6141_7272);
    put32(
        f,
        488,
        clusters - 1 - used - img.blocked_clusters.len() as u32,
    );
    put32(f, 492, next_free);
    f[510] = 0x55;
    f[511] = 0xAA;
    out(sector_at(PARTITION_START + 1), &fsinfo);
}
