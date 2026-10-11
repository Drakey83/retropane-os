//! Image size budgets (ADR-0005 rule 7). Budgets only ever get tighter.

use std::path::Path;

/// A size budget for one bare-metal image.
pub struct Budget {
    /// Image name under `target/<target>/release/`.
    pub image: &'static str,
    /// Max bytes loaded from storage (code + read-only data + initialised data).
    pub max_load: u64,
    /// Max bytes of RAM not backed by the image (zeroed data + stack).
    pub max_zero: u64,
}

/// Current budgets.
pub const BUDGETS: &[Budget] = &[Budget {
    image: "rp-hello",
    max_load: 24 * 1024,
    max_zero: 20 * 1024,
}];

/// Loaded and zero-initialised byte counts of an ELF32 image.
#[derive(Debug, PartialEq, Eq)]
pub struct Sizes {
    /// PROGBITS sections that occupy memory.
    pub load: u64,
    /// NOBITS sections that occupy memory (.bss, stack).
    pub zero: u64,
}

const SHF_ALLOC: u32 = 0x2;
const SHT_NOBITS: u32 = 8;

/// Measures an ELF32 little-endian image by its section headers.
pub fn measure(elf: &[u8]) -> Result<Sizes, String> {
    let u16_at = |o: usize| elf.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |o: usize| {
        elf.get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    if elf.get(0..5) != Some(&[0x7F, b'E', b'L', b'F', 1]) {
        return Err("not an ELF32 file".into());
    }
    let shoff = u32_at(0x20).ok_or("truncated ELF header")? as usize;
    let shentsize = usize::from(u16_at(0x2E).ok_or("truncated ELF header")?);
    let shnum = usize::from(u16_at(0x30).ok_or("truncated ELF header")?);
    let mut sizes = Sizes { load: 0, zero: 0 };
    for i in 0..shnum {
        let sh = shoff + i * shentsize;
        let kind = u32_at(sh + 4).ok_or("truncated section header")?;
        let flags = u32_at(sh + 8).ok_or("truncated section header")?;
        let size = u64::from(u32_at(sh + 20).ok_or("truncated section header")?);
        if flags & SHF_ALLOC == 0 {
            continue;
        }
        if kind == SHT_NOBITS {
            sizes.zero += size;
        } else {
            sizes.load += size;
        }
    }
    Ok(sizes)
}

/// Checks every budget against the built images.
pub fn check(release_dir: &Path) -> Result<(), String> {
    for b in BUDGETS {
        let path = release_dir.join(b.image);
        let elf = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let s = measure(&elf)?;
        eprintln!(
            "    {}: {} B loaded (budget {}), {} B zeroed RAM (budget {})",
            b.image, s.load, b.max_load, s.zero, b.max_zero
        );
        if s.load > b.max_load || s.zero > b.max_zero {
            return Err(format!(
                "{} exceeds its size budget (ADR-0005). Shrink it, or justify raising the budget in review",
                b.image
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal ELF32 with three sections: alloc PROGBITS, alloc NOBITS,
    /// and non-alloc (debug) PROGBITS.
    fn elf(sections: &[(u32, u32, u32)]) -> Vec<u8> {
        let shoff = 0x40usize;
        let mut d = vec![0u8; shoff + sections.len() * 40];
        d[0..5].copy_from_slice(&[0x7F, b'E', b'L', b'F', 1]);
        d[0x20..0x24].copy_from_slice(&(shoff as u32).to_le_bytes());
        d[0x2E..0x30].copy_from_slice(&40u16.to_le_bytes());
        d[0x30..0x32].copy_from_slice(&(sections.len() as u16).to_le_bytes());
        for (i, &(kind, flags, size)) in sections.iter().enumerate() {
            let sh = shoff + i * 40;
            d[sh + 4..sh + 8].copy_from_slice(&kind.to_le_bytes());
            d[sh + 8..sh + 12].copy_from_slice(&flags.to_le_bytes());
            d[sh + 20..sh + 24].copy_from_slice(&size.to_le_bytes());
        }
        d
    }

    #[test]
    fn counts_only_allocated_sections() {
        let d = elf(&[
            (1, SHF_ALLOC | 0x4, 1000),
            (SHT_NOBITS, SHF_ALLOC | 0x1, 500),
            (1, 0, 99_999),
        ]);
        assert_eq!(
            measure(&d).unwrap(),
            Sizes {
                load: 1000,
                zero: 500
            }
        );
    }

    #[test]
    fn rejects_non_elf() {
        assert!(measure(b"MZ\x90\x00").is_err());
        assert!(measure(&[0x7F, b'E', b'L', b'F', 1]).is_err());
    }
}
