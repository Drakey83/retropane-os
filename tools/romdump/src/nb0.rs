//! Raw `.nb0` flash images: an MBR-style partition table in sector 0.
//!
//! On the X51v the table describes three partitions (verified on LennySh
//! WM6.1 A08/L11 images): `0x20` XIP (update loader), `0x23` XIP (kernel and
//! boot modules), `0x25` IMGFS (everything else, LZX-compressed).

use crate::Error;
use crate::bytes::{u16_at, u32_at};

/// Sector size used by the partition table.
pub const SECTOR: usize = 512;

/// Partition type: BINFS / XIP image.
pub const TYPE_XIP_ULDR: u8 = 0x20;
/// Partition type: XIP kernel image.
pub const TYPE_XIP: u8 = 0x23;
/// Partition type: IMGFS.
pub const TYPE_IMGFS: u8 = 0x25;

/// One partition table entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Partition {
    /// Partition type byte.
    pub kind: u8,
    /// Byte offset of the partition in the image.
    pub offset: usize,
    /// Size in bytes.
    pub len: usize,
}

impl Partition {
    /// A human-readable name for the partition type.
    #[must_use]
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            TYPE_XIP_ULDR => "XIP (update loader)",
            TYPE_XIP => "XIP",
            TYPE_IMGFS => "IMGFS",
            _ => "unknown",
        }
    }
}

/// Parses the partition table in sector 0.
pub fn partitions(image: &[u8]) -> Result<Vec<Partition>, Error> {
    if u16_at(image, 0x1FE)? != 0xAA55 {
        return Err(Error::Format(
            "no partition table signature (0x55AA)".into(),
        ));
    }
    let mut parts = Vec::new();
    for i in 0..4 {
        let e = 0x1BE + i * 16;
        let kind = image[e + 4];
        let lba = u32_at(image, e + 8)? as usize;
        let count = u32_at(image, e + 12)? as usize;
        if kind == 0 || count == 0 {
            continue;
        }
        let p = Partition {
            kind,
            offset: lba * SECTOR,
            len: count * SECTOR,
        };
        if p.offset + p.len > image.len() {
            return Err(Error::Format(format!(
                "partition {i} runs past end of image"
            )));
        }
        parts.push(p);
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_with(entries: &[(u8, u32, u32)], size: usize) -> Vec<u8> {
        let mut d = vec![0u8; size];
        for (i, &(kind, lba, count)) in entries.iter().enumerate() {
            let e = 0x1BE + i * 16;
            d[e + 4] = kind;
            d[e + 8..e + 12].copy_from_slice(&lba.to_le_bytes());
            d[e + 12..e + 16].copy_from_slice(&count.to_le_bytes());
        }
        d[0x1FE] = 0x55;
        d[0x1FF] = 0xAA;
        d
    }

    #[test]
    fn parses_x51v_style_layout() {
        let d = image_with(&[(0x20, 2, 4), (0x23, 6, 2), (0x25, 8, 8)], 16 * SECTOR);
        let p = partitions(&d).unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(
            p[2],
            Partition {
                kind: TYPE_IMGFS,
                offset: 8 * SECTOR,
                len: 8 * SECTOR
            }
        );
        assert_eq!(p[2].kind_name(), "IMGFS");
    }

    #[test]
    fn rejects_missing_signature() {
        assert!(partitions(&vec![0u8; SECTOR]).is_err());
    }

    #[test]
    fn rejects_partition_past_end() {
        let d = image_with(&[(0x25, 1, 100)], 4 * SECTOR);
        assert!(partitions(&d).is_err());
    }
}
