//! The X50v's StrataFlash images: a flash translation layer between the
//! `.nb0` file's bytes and the sectors the partition table names.
//!
//! The X51v's DiskOnChip images are flat: the table's sectors (512 bytes)
//! are file offsets. The X50v's (seen on `WM50_K_A12_X50v_WM65_EN.nb0`,
//! 2026-11-13) carry an `MSFLSH50` header at 0x1000 and count partitions in
//! **4 KiB logical sectors**: each 256 KiB erase block holds 63 sectors of
//! data and, in its last 4 KiB, one 8-byte entry a sector, `u32` logical
//! sector number then `u32` status (`0xFFFBFFFF` in use; `0xFFFFFFFF` free).
//! The IMGFS header blocks' "next" pointers are logical too (the first
//! block's next, 0x3F000, lies at partition offset 0x40000 in the file).
//! Blocks before the managed area carry no map and are read as they are.

/// The signature at 0x1000 of a StrataFlash image.
pub const SIGNATURE: &[u8; 8] = b"MSFLSH50";
/// A logical sector.
pub const SECTOR: usize = 0x1000;
/// An erase block.
pub const BLOCK: usize = 0x4_0000;
/// Data sectors a block (the last 4 KiB is the map).
pub const PER_BLOCK: usize = BLOCK / SECTOR - 1;
/// A map entry's status for a sector in use.
pub const IN_USE: u32 = 0xFFFB_FFFF;

/// Whether `image` is a StrataFlash image.
#[must_use]
pub fn is_strataflash(image: &[u8]) -> bool {
    image.get(0x1000..0x1008) == Some(SIGNATURE.as_slice())
}

/// The image as its logical sectors, each mapped sector placed at
/// `number * SECTOR`; sectors no block maps are `0xFF`. The unmanaged
/// blocks at the start (no valid map) are copied where they lie.
#[must_use]
pub fn logical(image: &[u8]) -> Vec<u8> {
    let mut placed: Vec<(usize, usize)> = Vec::new();
    let mut first_managed = None;
    for b in 0..image.len() / BLOCK {
        let tail = b * BLOCK + PER_BLOCK * SECTOR;
        let mut any = false;
        for i in 0..PER_BLOCK {
            let e = tail + i * 8;
            let n = u32::from_le_bytes([image[e], image[e + 1], image[e + 2], image[e + 3]]);
            let s = u32::from_le_bytes([image[e + 4], image[e + 5], image[e + 6], image[e + 7]]);
            if s == IN_USE && n != u32::MAX {
                placed.push((n as usize, b * BLOCK + i * SECTOR));
                any = true;
            }
        }
        if any && first_managed.is_none() {
            first_managed = Some(b * BLOCK);
        }
    }
    let lowest = placed.iter().map(|&(n, _)| n * SECTOR).min();
    let raw = first_managed
        .unwrap_or(image.len())
        .min(lowest.unwrap_or(usize::MAX));
    // The ROM's partitions end where the table says, past the last mapped
    // sector (their free space is never written); the user store (type
    // 0x04, FAT) holds no ROM and is dropped from the table.
    let mut table_end = 0;
    let mut dropped = Vec::new();
    for i in 0..4 {
        let e = 0x1BE + i * 16;
        let Some(entry) = image.get(e..e + 16) else {
            break;
        };
        let lba = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
        let count = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
        match entry[4] {
            0 => {}
            USER_STORE => dropped.push(e),
            _ => table_end = table_end.max((lba + count) * SECTOR),
        }
    }
    let end = placed
        .iter()
        .map(|&(n, _)| (n + 1) * SECTOR)
        .max()
        .unwrap_or(0)
        .max(raw)
        .max(table_end);
    let mut out = vec![0xFF; end];
    out[..raw].copy_from_slice(&image[..raw]);
    for (n, at) in placed {
        out[n * SECTOR..(n + 1) * SECTOR].copy_from_slice(&image[at..at + SECTOR]);
    }
    for e in dropped {
        out[e..e + 16].fill(0);
    }
    out
}

/// The partition type of the user store (FAT), which holds no ROM.
pub const USER_STORE: u8 = 0x04;

#[cfg(test)]
mod tests {
    use super::*;

    /// Two raw blocks, then a managed block holding logical sectors 0x81
    /// and 0x80 in that order.
    fn image() -> Vec<u8> {
        let mut d = vec![0u8; 3 * BLOCK];
        d[0x1000..0x1008].copy_from_slice(SIGNATURE);
        for b in 0..2 {
            let tail = b * BLOCK + PER_BLOCK * SECTOR;
            d[tail..tail + SECTOR].fill(0x5A); // not a map
        }
        let base = 2 * BLOCK;
        d[base..base + SECTOR].fill(0xB1);
        d[base + SECTOR..base + 2 * SECTOR].fill(0xB0);
        let tail = base + PER_BLOCK * SECTOR;
        d[tail..base + BLOCK].fill(0xFF);
        d[tail..tail + 4].copy_from_slice(&0x81u32.to_le_bytes());
        d[tail + 4..tail + 8].copy_from_slice(&IN_USE.to_le_bytes());
        d[tail + 8..tail + 12].copy_from_slice(&0x80u32.to_le_bytes());
        d[tail + 12..tail + 16].copy_from_slice(&IN_USE.to_le_bytes());
        d
    }

    #[test]
    fn places_each_sector_by_its_map_entry_and_keeps_the_raw_blocks() {
        let d = image();
        assert!(is_strataflash(&d));
        let l = logical(&d);
        assert_eq!(l.len(), 0x82 * SECTOR);
        assert!(l[0x80 * SECTOR..0x81 * SECTOR].iter().all(|&b| b == 0xB0));
        assert!(l[0x81 * SECTOR..0x82 * SECTOR].iter().all(|&b| b == 0xB1));
        assert_eq!(
            &l[0x1000..0x1008],
            SIGNATURE,
            "the raw blocks where they lie"
        );
        assert!(l[2 * BLOCK..0x80 * SECTOR].iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn the_rom_partitions_are_padded_to_the_table_and_the_user_store_dropped() {
        let mut d = image();
        let put = |d: &mut Vec<u8>, i: usize, kind: u8, lba: u32, count: u32| {
            let e = 0x1BE + i * 16;
            d[e + 4] = kind;
            d[e + 8..e + 12].copy_from_slice(&lba.to_le_bytes());
            d[e + 12..e + 16].copy_from_slice(&count.to_le_bytes());
        };
        put(&mut d, 0, 0x25, 0x80, 0x10);
        put(&mut d, 1, USER_STORE, 0x90, 0x1000);
        let l = logical(&d);
        assert_eq!(l.len(), 0x90 * SECTOR, "the IMGFS's free space kept");
        assert_eq!(l[0x1BE + 16 + 4], 0, "the user store dropped");
        assert_eq!(l[0x1BE + 4], 0x25);
    }

    #[test]
    fn a_flat_image_is_not_strataflash() {
        assert!(!is_strataflash(&vec![0u8; 0x2000]));
        assert!(!is_strataflash(&[0u8; 16]));
    }
}
