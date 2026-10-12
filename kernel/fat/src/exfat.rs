//! exFAT reading (Microsoft's published exFAT specification, 2019): what
//! SDXC cards (64 GB and up) come formatted with, and what stock Windows
//! Mobile never read.
//!
//! Mount checks the boot region's checksum and every field it uses; the
//! root directory's entry sets are checked against their set checksums;
//! files are read contiguously ("no FAT chain") or through the FAT, with
//! bytes past the valid data length read as zeros. Read-only for now.
//!
//! Security (docs/security.md): every length, offset and cluster number
//! from the card is checked before use, chains and directories are bounded,
//! and a malformed card gives an error, never a panic or a loop.

use rp_hal_api::BlockDevice;

use crate::{Error, NAME_MAX, SECTOR};

const ENTRY: usize = 32;
const FAT_EOC: u32 = 0xFFFF_FFFF;
const FAT_BAD: u32 = 0xFFFF_FFF7;
/// Entry types (in use: bit 7 set).
const T_FILE: u8 = 0x85;
const T_STREAM: u8 = 0xC0;
const T_NAME: u8 = 0xC1;
const ATTR_DIRECTORY: u16 = 0x10;
/// A file entry set: the file entry, the stream extension, up to 17 name
/// entries (255 characters).
const MAX_SECONDARY: usize = 18;

/// A root-directory file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExFile {
    first_cluster: u32,
    /// Bytes in the file.
    pub size: u64,
    /// Bytes actually written; past this the file reads as zeros.
    pub valid: u64,
    contiguous: bool,
    /// A subdirectory.
    pub is_dir: bool,
}

/// A mounted exFAT volume.
pub struct ExVolume<D> {
    dev: D,
    fat_start: u32,
    heap_start: u32,
    cluster_shift: u32,
    clusters: u32,
    root_cluster: u32,
    buf: [u8; SECTOR],
}

fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn le64(b: &[u8], o: usize) -> u64 {
    u64::from(le32(b, o)) | u64::from(le32(b, o + 4)) << 32
}

/// The boot region checksum over `sector` (sector `index` of 0..11): bytes
/// 106, 107 (VolumeFlags) and 112 (PercentInUse) of sector 0 don't count.
fn boot_checksum(mut sum: u32, index: usize, sector: &[u8]) -> u32 {
    for (i, &b) in sector.iter().enumerate() {
        if index == 0 && matches!(i, 106 | 107 | 112) {
            continue;
        }
        sum = sum.rotate_right(1).wrapping_add(u32::from(b));
    }
    sum
}

/// The entry set checksum: all entries, without bytes 2-3 of the first.
fn set_checksum(set: &[[u8; ENTRY]]) -> u16 {
    let mut sum = 0u16;
    for (k, e) in set.iter().enumerate() {
        for (i, &b) in e.iter().enumerate() {
            if k == 0 && (i == 2 || i == 3) {
                continue;
            }
            sum = sum.rotate_right(1).wrapping_add(u16::from(b));
        }
    }
    sum
}

impl<D: BlockDevice> ExVolume<D> {
    /// Mounts the exFAT volume whose boot sector is at `base`.
    pub fn mount(mut dev: D, base: u32) -> Result<Self, Error> {
        let mut buf = [0u8; SECTOR];
        // The boot region: 11 sectors checksummed, sector 11 repeats the sum.
        let mut sum = 0u32;
        let mut boot = [0u8; SECTOR];
        for i in 0..11 {
            dev.read_blocks(base.checked_add(i as u32).ok_or(Error::NotExFat)?, &mut buf)?;
            if i == 0 {
                boot = buf;
            }
            sum = boot_checksum(sum, i, &buf);
        }
        dev.read_blocks(base.checked_add(11).ok_or(Error::NotExFat)?, &mut buf)?;
        let b = &boot;
        if &b[3..11] != b"EXFAT   "
            || b[11..64].iter().any(|&x| x != 0)
            || b[510..512] != [0x55, 0xAA]
            || buf
                .as_chunks::<4>()
                .0
                .iter()
                .any(|w| u32::from_le_bytes(*w) != sum)
        {
            return Err(Error::NotExFat);
        }
        let fat_offset = le32(b, 80);
        let fat_length = le32(b, 84);
        let heap_offset = le32(b, 88);
        let clusters = le32(b, 92);
        let root_cluster = le32(b, 96);
        let sector_shift = u32::from(b[108]);
        let cluster_shift = u32::from(b[109]);
        let volume_length = le64(b, 72);
        // Only 512-byte sectors (the block size of every card here); at most
        // 32 MiB clusters, as the specification allows.
        if sector_shift != 9
            || cluster_shift > 16
            || clusters == 0
            || clusters > 0xFFFF_FFF5
            || !(2..2 + clusters).contains(&root_cluster)
            || u64::from(fat_length) * (SECTOR as u64 / 4) < u64::from(clusters) + 2
            || u64::from(heap_offset) + (u64::from(clusters) << cluster_shift) > volume_length
            || u64::from(fat_offset) + u64::from(fat_length) > u64::from(heap_offset)
        {
            return Err(Error::NotExFat);
        }
        let end = u64::from(base) + volume_length;
        if end > u64::from(u32::MAX) {
            // Beyond 32-bit block numbers: 2 TiB, more than any SD card.
            return Err(Error::NotExFat);
        }
        Ok(Self {
            dev,
            fat_start: base + fat_offset,
            heap_start: base + heap_offset,
            cluster_shift,
            clusters,
            root_cluster,
            buf,
        })
    }

    /// Bytes per cluster.
    #[must_use]
    pub fn cluster_bytes(&self) -> usize {
        SECTOR << self.cluster_shift
    }

    /// Data clusters on the volume.
    #[must_use]
    pub fn clusters(&self) -> u32 {
        self.clusters
    }

    /// Gives the device back.
    pub fn into_inner(self) -> D {
        self.dev
    }

    fn valid_cluster(&self, c: u32) -> bool {
        (2..2 + self.clusters).contains(&c)
    }

    fn cluster_lba(&self, c: u32) -> u32 {
        // In range by mount's checks for any valid cluster.
        self.heap_start + ((c - 2) << self.cluster_shift)
    }

    fn fat_next(&mut self, c: u32) -> Result<Option<u32>, Error> {
        let sector = self.fat_start + c / (SECTOR as u32 / 4);
        self.dev.read_blocks(sector, &mut self.buf)?;
        let v = le32(&self.buf, (c % (SECTOR as u32 / 4)) as usize * 4);
        if v == FAT_EOC {
            Ok(None)
        } else if v == FAT_BAD || !self.valid_cluster(v) {
            Err(Error::Corrupt)
        } else {
            Ok(Some(v))
        }
    }

    /// Calls `f` for each root-directory file whose entry set checks out,
    /// with its name (ASCII; other characters as `?`, cut at [`NAME_MAX`]);
    /// stops when `f` returns `true`.
    fn walk_root(&mut self, f: &mut dyn FnMut(&[u8], &ExFile) -> bool) -> Result<(), Error> {
        let mut set = [[0u8; ENTRY]; MAX_SECONDARY + 1];
        let mut want = 0usize; // entries still to collect for the set
        let mut have = 0usize;
        let mut cluster = self.root_cluster;
        let mut visited = 0u32;
        loop {
            let lba = self.cluster_lba(cluster);
            for s in 0..1u32 << self.cluster_shift {
                self.dev.read_blocks(lba + s, &mut self.buf)?;
                for i in 0..SECTOR / ENTRY {
                    let mut e = [0u8; ENTRY];
                    e.copy_from_slice(&self.buf[i * ENTRY..(i + 1) * ENTRY]);
                    let t = e[0];
                    if t == 0 {
                        return Ok(());
                    }
                    if want > 0 {
                        set[have] = e;
                        have += 1;
                        want -= 1;
                        if want == 0
                            && let Some((name, n, file)) = self.parse_set(&set[..have])
                            && f(&name[..n], &file)
                        {
                            return Ok(());
                        }
                        continue;
                    }
                    if t == T_FILE {
                        let secondary = usize::from(e[1]);
                        if (2..=MAX_SECONDARY).contains(&secondary) {
                            set[0] = e;
                            have = 1;
                            want = secondary;
                        }
                    }
                }
            }
            visited += 1;
            match self.fat_next(cluster)? {
                Some(c) if visited < self.clusters => cluster = c,
                Some(_) => return Err(Error::Corrupt),
                None => return Ok(()),
            }
        }
    }

    /// Checks and decodes a file entry set.
    fn parse_set(&self, set: &[[u8; ENTRY]]) -> Option<([u8; NAME_MAX], usize, ExFile)> {
        if set_checksum(set) != le16(&set[0], 2) || set[1][0] != T_STREAM {
            return None;
        }
        let st = &set[1];
        let name_len = usize::from(st[3]);
        let file = ExFile {
            first_cluster: le32(st, 20),
            size: le64(st, 24),
            valid: le64(st, 8).min(le64(st, 24)),
            contiguous: st[1] & 0x02 != 0,
            is_dir: le16(&set[0], 4) & ATTR_DIRECTORY != 0,
        };
        let mut name = [0u8; NAME_MAX];
        let mut n = 0;
        for e in set[2..].iter().filter(|e| e[0] == T_NAME) {
            for k in 0..15 {
                if n >= name_len.min(NAME_MAX) {
                    break;
                }
                let c = le16(e, 2 + 2 * k);
                name[n] = if c < 0x80 { c as u8 } else { b'?' };
                n += 1;
            }
        }
        (n > 0).then_some((name, n, file))
    }

    /// Calls `f` with each root file's name and details.
    pub fn for_each_root(&mut self, mut f: impl FnMut(&[u8], &ExFile)) -> Result<(), Error> {
        self.walk_root(&mut |n, file| {
            f(n, file);
            false
        })
    }

    /// Finds a root file by name (ASCII, case-insensitive).
    pub fn find(&mut self, name: &str) -> Result<ExFile, Error> {
        let want = name.as_bytes();
        let mut found = None;
        self.walk_root(&mut |n, file| {
            let hit = n.eq_ignore_ascii_case(want);
            if hit {
                found = Some(*file);
            }
            hit
        })?;
        found.ok_or(Error::NotFound)
    }

    /// Reads `file` in order, calling `f` with each piece (at most
    /// `scratch`'s length, whole sectors); bytes past the valid data length
    /// come as zeros.
    ///
    /// # Panics
    /// If `scratch` is shorter than a sector.
    pub fn read_file(
        &mut self,
        file: &ExFile,
        scratch: &mut [u8],
        mut f: impl FnMut(&[u8]),
    ) -> Result<(), Error> {
        let max = scratch.len() / SECTOR * SECTOR;
        assert!(max >= SECTOR, "scratch must hold a sector");
        if file.size == 0 {
            return Ok(());
        }
        if !self.valid_cluster(file.first_cluster) {
            return Err(Error::Corrupt);
        }
        let cb = self.cluster_bytes() as u64;
        let total_clusters = file.size.div_ceil(cb);
        if total_clusters > u64::from(self.clusters) {
            return Err(Error::Corrupt);
        }
        if file.contiguous
            && u64::from(file.first_cluster) + total_clusters > 2 + u64::from(self.clusters)
        {
            return Err(Error::Corrupt);
        }
        let mut done = 0u64;
        let mut cluster = file.first_cluster;
        for k in 0..total_clusters {
            let lba = self.cluster_lba(cluster);
            let mut off = 0u64;
            while off < cb && done < file.size {
                let n = (max as u64).min(cb - off).min(file.size - done) as usize;
                let secs = n.div_ceil(SECTOR);
                let piece = &mut scratch[..secs * SECTOR];
                if done >= file.valid {
                    piece.fill(0);
                } else {
                    self.dev
                        .read_blocks(lba + (off / SECTOR as u64) as u32, piece)?;
                    // Zeros from the valid length on, within this piece.
                    let keep = (file.valid - done).min(n as u64) as usize;
                    piece[keep..n].fill(0);
                }
                f(&piece[..n]);
                done += n as u64;
                off += n as u64;
            }
            if k + 1 < total_clusters {
                cluster = if file.contiguous {
                    cluster + 1
                } else {
                    self.fat_next(cluster)?.ok_or(Error::Corrupt)?
                };
            }
        }
        Ok(())
    }
}

/// A small exFAT formatter (host only), for the tests and the emulator's
/// card: MBR with one partition (type 0x07), the boot region with its
/// checksum, the FAT, an allocation bitmap, an up-case table, and files in
/// the root directory (contiguous, or chained through the FAT when
/// `fragment` asks for it).
#[cfg(any(test, feature = "std"))]
pub mod format {
    use super::{ENTRY, SECTOR, boot_checksum, set_checksum};

    /// Where the partition starts.
    pub const PARTITION_START: u32 = 2048;

    /// A file for the root directory.
    pub struct Entry<'a> {
        /// Name (ASCII).
        pub name: &'a str,
        /// Contents.
        pub data: &'a [u8],
        /// Store it through the FAT, every other cluster (fragmented).
        pub fragment: bool,
    }

    /// Builds a whole disk image of `total_sectors` with 2^`cluster_shift`
    /// sectors per cluster.
    ///
    /// # Panics
    /// If the files don't fit.
    #[must_use]
    pub fn build(total_sectors: u32, cluster_shift: u32, files: &[Entry<'_>]) -> Vec<u8> {
        let spc = 1u32 << cluster_shift;
        let cb = (spc as usize) * SECTOR;
        let vol = total_sectors - PARTITION_START;
        let fat_offset = 24u32;
        let clusters_est = vol / spc;
        let fat_length = (clusters_est + 2).div_ceil(128);
        let heap_offset = (fat_offset + fat_length).next_multiple_of(spc);
        let clusters = (vol - heap_offset) / spc;
        let mut disk = vec![0u8; total_sectors as usize * SECTOR];
        let e = 446;
        disk[e + 4] = 0x07;
        disk[e + 8..e + 12].copy_from_slice(&PARTITION_START.to_le_bytes());
        disk[e + 12..e + 16].copy_from_slice(&vol.to_le_bytes());
        disk[510] = 0x55;
        disk[511] = 0xAA;

        let base = PARTITION_START as usize * SECTOR;
        let mut fat = vec![0u32; fat_length as usize * 128];
        fat[0] = 0xFFFF_FFF8;
        fat[1] = 0xFFFF_FFFF;
        let mut bitmap = vec![0u8; (clusters as usize).div_ceil(8)];
        let mut next = 2u32;
        let mut used = |n: u32, chained: bool, fat: &mut Vec<u32>, bitmap: &mut Vec<u8>| -> u32 {
            let first = next;
            for k in 0..n {
                let c = next;
                bitmap[(c - 2) as usize / 8] |= 1 << ((c - 2) % 8);
                if chained {
                    fat[c as usize] = if k + 1 == n { 0xFFFF_FFFF } else { c + 2 };
                    next += 2;
                } else {
                    next += 1;
                }
                assert!(next < clusters + 2, "image too small");
            }
            if !chained && n > 0 {
                fat[(first + n - 1) as usize] = 0; // contiguous: no FAT chain
            }
            first
        };
        let cl = |c: u32| base + ((heap_offset + (c - 2) * spc) as usize) * SECTOR;

        let bitmap_cluster = used(
            (bitmap.len() as u32).div_ceil(cb as u32).max(1),
            true,
            &mut fat,
            &mut bitmap,
        );
        let upcase: Vec<u8> = (0u16..128)
            .map(|c| {
                if (b'a' as u16..=b'z' as u16).contains(&c) {
                    c - 32
                } else {
                    c
                }
            })
            .flat_map(u16::to_le_bytes)
            .collect();
        let upcase_cluster = used(1, true, &mut fat, &mut bitmap);
        let root_cluster = used(1, true, &mut fat, &mut bitmap);
        let mut root = vec![0u8; cb];
        let mut at = 0;
        let mut put = |root: &mut Vec<u8>, e: [u8; ENTRY]| {
            root[at..at + ENTRY].copy_from_slice(&e);
            at += ENTRY;
        };
        let mut e = [0u8; ENTRY];
        e[0] = 0x81;
        e[20..24].copy_from_slice(&bitmap_cluster.to_le_bytes());
        e[24..32].copy_from_slice(&(bitmap.len() as u64).to_le_bytes());
        put(&mut root, e);
        let mut e = [0u8; ENTRY];
        e[0] = 0x82;
        let up_sum = upcase
            .iter()
            .fold(0u32, |s, &b| s.rotate_right(1).wrapping_add(u32::from(b)));
        e[4..8].copy_from_slice(&up_sum.to_le_bytes());
        e[20..24].copy_from_slice(&upcase_cluster.to_le_bytes());
        e[24..32].copy_from_slice(&(upcase.len() as u64).to_le_bytes());
        put(&mut root, e);

        let mut placed = Vec::new();
        for f in files {
            let n = (f.data.len() as u32).div_ceil(cb as u32);
            let first = if n == 0 {
                0
            } else {
                used(n, f.fragment, &mut fat, &mut bitmap)
            };
            placed.push((first, n));
            let name: Vec<u16> = f.name.encode_utf16().collect();
            let names = name.len().div_ceil(15);
            let mut set = vec![[0u8; ENTRY]; 2 + names];
            set[0][0] = 0x85;
            set[0][1] = (1 + names) as u8;
            set[0][4] = 0x20;
            set[1][0] = 0xC0;
            set[1][1] = 0x01 | if f.fragment { 0 } else { 0x02 };
            set[1][3] = name.len() as u8;
            set[1][8..16].copy_from_slice(&(f.data.len() as u64).to_le_bytes());
            set[1][20..24].copy_from_slice(&first.to_le_bytes());
            set[1][24..32].copy_from_slice(&(f.data.len() as u64).to_le_bytes());
            for (k, chunk) in name.chunks(15).enumerate() {
                let s = &mut set[2 + k];
                s[0] = 0xC1;
                for (i, &c) in chunk.iter().enumerate() {
                    s[2 + 2 * i..4 + 2 * i].copy_from_slice(&c.to_le_bytes());
                }
            }
            let sum = set_checksum(&set);
            set[0][2..4].copy_from_slice(&sum.to_le_bytes());
            for s in set {
                put(&mut root, s);
            }
        }
        disk[cl(root_cluster)..cl(root_cluster) + cb].copy_from_slice(&root);
        disk[cl(bitmap_cluster)..cl(bitmap_cluster) + bitmap.len()].copy_from_slice(&bitmap);
        disk[cl(upcase_cluster)..cl(upcase_cluster) + upcase.len()].copy_from_slice(&upcase);
        for (f, &(first, n)) in files.iter().zip(&placed) {
            for k in 0..n as usize {
                let c = if f.fragment {
                    first + 2 * k as u32
                } else {
                    first + k as u32
                };
                let piece = &f.data[k * cb..((k + 1) * cb).min(f.data.len())];
                disk[cl(c)..cl(c) + piece.len()].copy_from_slice(piece);
            }
        }
        let fo = base + fat_offset as usize * SECTOR;
        for (i, v) in fat.iter().enumerate() {
            disk[fo + i * 4..fo + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }

        // Boot sector and its region checksum.
        let b = &mut disk[base..base + SECTOR];
        b[0..3].copy_from_slice(&[0xEB, 0x76, 0x90]);
        b[3..11].copy_from_slice(b"EXFAT   ");
        b[64..72].copy_from_slice(&u64::from(PARTITION_START).to_le_bytes());
        b[72..80].copy_from_slice(&u64::from(vol).to_le_bytes());
        b[80..84].copy_from_slice(&fat_offset.to_le_bytes());
        b[84..88].copy_from_slice(&fat_length.to_le_bytes());
        b[88..92].copy_from_slice(&heap_offset.to_le_bytes());
        b[92..96].copy_from_slice(&clusters.to_le_bytes());
        b[96..100].copy_from_slice(&root_cluster.to_le_bytes());
        b[100..104].copy_from_slice(&0x2026_1012u32.to_le_bytes());
        b[104..106].copy_from_slice(&0x0100u16.to_le_bytes());
        b[108] = 9;
        b[109] = cluster_shift as u8;
        b[110] = 1;
        b[111] = 0x80;
        b[510] = 0x55;
        b[511] = 0xAA;
        let mut sum = 0u32;
        for i in 0..11 {
            let s = &disk[base + i * SECTOR..base + (i + 1) * SECTOR];
            sum = boot_checksum(sum, i, s);
        }
        for w in disk[base + 11 * SECTOR..base + 12 * SECTOR]
            .as_chunks_mut::<4>()
            .0
        {
            w.copy_from_slice(&sum.to_le_bytes());
        }
        disk
    }
}

#[cfg(test)]
mod tests {
    use super::format::{Entry, PARTITION_START, build};
    use super::*;
    use rp_hal_api::BlockError;

    struct Mem(Vec<u8>);

    impl BlockDevice for Mem {
        fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
            let o = lba as usize * SECTOR;
            let src = self.0.get(o..o + buf.len()).ok_or(BlockError::OutOfRange)?;
            buf.copy_from_slice(src);
            Ok(())
        }
        fn write_blocks(&mut self, _: u32, _: &[u8]) -> Result<(), BlockError> {
            Err(BlockError::ReadOnly)
        }
    }

    fn pattern(n: usize, seed: u8) -> Vec<u8> {
        (0..n).map(|i| (i as u8).wrapping_mul(29) ^ seed).collect()
    }

    fn read_all(v: &mut ExVolume<Mem>, name: &str, scratch: usize) -> Vec<u8> {
        let f = v.find(name).unwrap();
        let mut out = Vec::new();
        let mut s = vec![0u8; scratch];
        v.read_file(&f, &mut s, |c| out.extend_from_slice(c))
            .unwrap();
        out
    }

    fn image() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let a = pattern(9000, 1);
        let b = pattern(5000, 2);
        let img = build(
            2048 + 2048,
            2,
            &[
                Entry {
                    name: "retropane.bin",
                    data: &a,
                    fragment: false,
                },
                Entry {
                    name: "A Fragmented File With A Long Name.dat",
                    data: &b,
                    fragment: true,
                },
                Entry {
                    name: "empty.txt",
                    data: b"",
                    fragment: false,
                },
            ],
        );
        (img, a, b)
    }

    #[test]
    fn mounts_lists_and_reads_contiguous_and_chained_files() {
        let (img, a, b) = image();
        let mut v = ExVolume::mount(Mem(img), PARTITION_START).unwrap();
        assert_eq!(v.cluster_bytes(), 2048);
        let mut names = Vec::new();
        v.for_each_root(|n, f| names.push((String::from_utf8_lossy(n).into_owned(), f.size)))
            .unwrap();
        assert_eq!(
            names,
            [
                ("retropane.bin".to_string(), 9000),
                ("A Fragmented File With A Long Name.dat".to_string(), 5000),
                ("empty.txt".to_string(), 0),
            ]
        );
        for scratch in [512, 2048, 4096, 65536] {
            assert_eq!(read_all(&mut v, "RETROPANE.BIN", scratch), a);
            assert_eq!(
                read_all(&mut v, "a fragmented file with a long name.dat", scratch),
                b
            );
        }
        assert_eq!(read_all(&mut v, "empty.txt", 512), b"");
        assert_eq!(v.find("missing"), Err(Error::NotFound));
    }

    #[test]
    fn bytes_past_the_valid_length_read_as_zeros() {
        let (mut img, a, _) = image();
        // Lower retropane.bin's valid data length to 1000 (and fix the set
        // checksum, as a real writer would).
        let mut v = ExVolume::mount(Mem(img.clone()), PARTITION_START).unwrap();
        let root = v.cluster_lba(v.root_cluster) as usize * SECTOR;
        let set_at = root + 2 * ENTRY;
        img[set_at + ENTRY + 8..set_at + ENTRY + 16].copy_from_slice(&1000u64.to_le_bytes());
        let mut set = [[0u8; ENTRY]; 3];
        for (k, s) in set.iter_mut().enumerate() {
            s.copy_from_slice(&img[set_at + k * ENTRY..set_at + (k + 1) * ENTRY]);
        }
        let sum = set_checksum(&set);
        img[set_at + 2..set_at + 4].copy_from_slice(&sum.to_le_bytes());
        v = ExVolume::mount(Mem(img), PARTITION_START).unwrap();
        let got = read_all(&mut v, "retropane.bin", 1536);
        assert_eq!(&got[..1000], &a[..1000]);
        assert!(got[1000..].iter().all(|&x| x == 0));
        assert_eq!(got.len(), 9000);
    }

    #[test]
    fn a_damaged_entry_set_is_skipped_and_a_damaged_boot_region_refused() {
        let (mut img, _, _) = image();
        let v = ExVolume::mount(Mem(img.clone()), PARTITION_START).unwrap();
        let root = v.cluster_lba(v.root_cluster) as usize * SECTOR;
        img[root + 2 * ENTRY + 2 * ENTRY + 4] ^= 1; // a name character
        let mut v = ExVolume::mount(Mem(img.clone()), PARTITION_START).unwrap();
        assert_eq!(v.find("retropane.bin"), Err(Error::NotFound));
        assert!(v.find("empty.txt").is_ok(), "the others still read");
        let base = PARTITION_START as usize * SECTOR;
        img[base + 3 * SECTOR + 7] ^= 1; // inside the checksummed region
        assert_eq!(
            ExVolume::mount(Mem(img), PARTITION_START).err(),
            Some(Error::NotExFat)
        );
    }

    #[test]
    fn a_fat32_volume_is_not_exfat() {
        let fat32 = crate::format::build(&crate::format::Image {
            total_sectors: 3072,
            sectors_per_cluster: 2,
            files: &[],
            blocked_clusters: &[],
        });
        assert_eq!(
            ExVolume::mount(Mem(fat32), PARTITION_START).err(),
            Some(Error::NotExFat)
        );
    }

    /// Hostile cards: random corruption anywhere in the structures, with the
    /// boot checksum fixed up so the damage gets past mount. Nothing may
    /// panic or loop.
    #[test]
    fn hostile_exfat_cards_never_panic_or_loop() {
        let (base_img, _, _) = image();
        let base = PARTITION_START as usize * SECTOR;
        let mut x: u64 = 0xE0FA_7201;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for i in 0..2000 {
            let mut img = base_img.clone();
            for _ in 0..1 + rnd() % 6 {
                let region = match i % 3 {
                    0 => base..base + SECTOR,
                    1 => base + 24 * SECTOR..base + 40 * SECTOR,
                    _ => base..img.len(),
                };
                let at = region.start + (rnd() as usize % region.len());
                img[at] = rnd() as u8;
            }
            // Re-sign the boot region so mount sees the corrupted fields.
            let mut sum = 0u32;
            for k in 0..11 {
                sum = boot_checksum(sum, k, &img[base + k * SECTOR..base + (k + 1) * SECTOR]);
            }
            for w in img[base + 11 * SECTOR..base + 12 * SECTOR]
                .as_chunks_mut::<4>()
                .0
            {
                w.copy_from_slice(&sum.to_le_bytes());
            }
            let Ok(mut v) = ExVolume::mount(Mem(img), PARTITION_START) else {
                continue;
            };
            let mut scratch = vec![0u8; 4096];
            let _ = v.for_each_root(|_, _| {});
            for name in ["retropane.bin", "A Fragmented File With A Long Name.dat"] {
                if let Ok(f) = v.find(name) {
                    let mut total = 0u64;
                    let _ = v.read_file(&f, &mut scratch, |c| total += c.len() as u64);
                    assert!(total <= f.size);
                }
            }
        }
    }
}
