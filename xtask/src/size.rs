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
///
/// Rules (ADR-0005 rule 7). **OS components** (what ships on the device,
/// starting with `rp-kernel`: RAM taken from apps) only tighten, except by
/// a raise the project owner approves, with the reason written next to the
/// budget. **Test and bring-up images** (never shipped) may be raised
/// without asking, with the reason written here (owner's decision,
/// 2026-10-11). Every raise is *ratcheted*: set to the measured size plus
/// about 10%, not to a round number with room to spare, so growth can't
/// accumulate unreviewed, and logged in docs/perf.md.
pub const BUDGETS: &[Budget] = &[
    Budget {
        image: "rp-hello",
        // 24 KiB until 2026-10-11: cold crates built for size cut it to 18 KB.
        // Raised 20 -> 22 KiB on 2026-10-11 (approved by the owner): shared
        // HAL start-up code grew (DMA stop, fault diagnostics, aligned MMU
        // transitions, user-fault path); 20.0 KiB measured, +~10%.
        max_load: 22 * 1024,
        max_zero: 20 * 1024,
    },
    // Ratcheted 24 -> 27 KiB on 2026-10-12 (test image): the SD driver's
    // recovery for slow and failing reads (block-by-block fallback, CMD12 +
    // CMD13 recovery, slow-clock retry), found on the owner's card;
    // 24.5 KiB measured, +~10%.
    // Ratcheted 27 -> 30 KiB on 2026-10-12 (test image): SD DMA (the DMA
    // channel driver and the controller's DMA path) and a 64-block read
    // row and its on-target check; 28.1 KiB measured, +~7%.
    Budget {
        image: "rp-bench",
        max_load: 30 * 1024,
        max_zero: 20 * 1024,
    },
    // The kernel: frame-allocator bookkeeping for 48 MiB (60 KiB), page-table
    // bookkeeping (9 KiB), stacks including a 4 KiB abort stack.
    // Load budget raised 32 -> 38 KiB on 2026-10-11 (approved by the owner):
    // compressed RAM landed (LZ4 codec, compressed page store, fault-path
    // restore: ~8 KiB); the image crate is already built for size and its
    // printing trimmed. Ratcheted to the measured 34.9 KB + ~10%.
    Budget {
        image: "rp-kernel",
        max_load: 38 * 1024,
        // Raised 100 -> 112 KiB with the load budget (approved by the owner):
        // the 12 KiB compression buffers live here, not on a kernel stack
        // (on the 4 KiB abort stack they overflowed into the frame
        // allocator).
        max_zero: 112 * 1024,
    },
    // Threads test harness (ADR-0010): scheduler tables (~4 KiB, .bss) and
    // the harness itself; one image per subsystem's test keeps each budget
    // honest instead of piling every test into one image.
    Budget {
        image: "rp-threads",
        // First budget = measured + ~10% (13.2 KiB load; 26.4 KiB zeroed:
        // boot stacks, scheduler tables, 128 thread contexts).
        // Raised 15 -> 17 KiB on 2026-10-11 (approved by the owner): the
        // user-mode preemption test (16.0 KiB measured, ratcheted +~9%).
        // Raised 17 -> 27 KiB on 2026-10-11 (approved by the owner): ADR-0010's
        // mutexes with priority inheritance (~2 KiB of scheduler code), the
        // user-fault path, lazy iWMMXt switching (~1.3 KiB of HAL code, in
        // this image only) and interrupts as waitable objects, plus their
        // live tests (23.9 KiB measured, +~10%).
        // Ratcheted 27 -> 32 KiB on 2026-10-12 (test image, owner's rule):
        // results teed into the run log and a return to the loader, so the
        // real X51v's switch and interrupt numbers reach the PC, plus the
        // shared crash path; 28.1 KiB measured, +~10%.
        max_load: 32 * 1024,
        max_zero: 30 * 1024,
    },
    // Diagnostics only, never part of the OS: register names and report text
    // dominate. Built for size (opt-level "s").
    // Raised 40 -> 45 KiB on 2026-10-11 (approved by the owner): the run log
    // and return-to-loader path (ADR-0012), 40.9 KiB measured, +~10%.
    // Ratcheted 45 -> 53 KiB on 2026-10-11 (test image, owner's rule): the
    // card's FAT32 root listing (ADR-0013), 47.5 KiB measured, +~10%.
    // Ratcheted 53 -> 60 KiB on 2026-10-12 (test image): the PC Card
    // registers and CF line, and the exFAT reader for SDXC cards (the
    // probe's emulated card is exFAT); 54.6 KiB measured, +~10%.
    Budget {
        image: "rp-probe",
        max_load: 60 * 1024,
        max_zero: 20 * 1024,
    },
    // Bring-up: the USB link test (ADR-0012 U1): 17.9 KiB measured, +~10%.
    Budget {
        image: "rp-usb",
        max_load: 20 * 1024,
        max_zero: 20 * 1024,
    },
    // Bring-up: the resident loader (ADR-0012 U3): 19.6 KiB measured, +~10%.
    // It must also stay inside its 1 MiB home (boot/loader/link.x).
    // Ratcheted 22 -> 41 KiB on 2026-10-11 (test image, owner's rule): SD
    // card writes and the FAT32 copy-then-swap (ADR-0013), ~37 KiB measured.
    // Ratcheted 41 -> 52 KiB on 2026-10-12 (test image): the SD card as a
    // USB drive (ADR-0014: composite descriptors, the Bulk-Only/SCSI stack,
    // 2.6 KiB of it in rp-usb-msc), 46.6 KiB measured, +~10%.
    // Ratcheted 52 -> 58 KiB on 2026-10-12 (test image): card diagnostics
    // over USB (`drive status`, `card scan`), which found the owner's slow
    // card block, and the USB layout fallback; 52.2 KiB measured, +~10%.
    // Ratcheted 58 -> 70 KiB on 2026-10-12 (test image): creating files on
    // the card (rp-fat: long names, ~N short names, root growth; ~5 KiB) and
    // the SD driver's slow-read recovery; 62.0 KiB measured, +~10%. When
    // rp-fat moves into the OS it counts against rp-kernel's strict budget.
    // Ratcheted 70 -> 78 KiB on 2026-10-12 (test image): card repair tools
    // (scrub, repair, peek), SD hot swap and the lock switch; 70.3 KiB
    // measured, +~10%.
    // Ratcheted 78 -> 87 KiB on 2026-10-12 (test image): card folders
    // (paths, mkdir, put creating parents) and the backlight command;
    // 79.0 KiB measured, +~10%.
    // Ratcheted 87 -> 96 KiB on 2026-10-12 (test image): exFAT reading and
    // writing for SDXC cards (put and crash reports on either file system,
    // ~16 KiB at opt-level 3); the loader is now built for size like the
    // other cold images, which took 11 KiB back; 89.8 KiB measured, +~7%.
    Budget {
        image: "rp-loader",
        max_load: 96 * 1024,
        max_zero: 20 * 1024,
    },
    // Test image: the crash path (panics on purpose); 6.6 KiB measured, +~10%.
    Budget {
        image: "rp-crash",
        max_load: 8 * 1024,
        max_zero: 20 * 1024,
    },
    // Bring-up experiment: USB reconnect candidates (ADR-0012). Provisional.
    Budget {
        image: "rp-reconnect",
        max_load: 24 * 1024,
        max_zero: 20 * 1024,
    },
    // Diagnostics only: the first-light image (14.3 KiB measured, +~10%).
    // Ratcheted 16 -> 18 KiB on 2026-10-12: the shared crash path (crash
    // reports marked for the loader, kept on the card); 16.1 KiB measured.
    Budget {
        image: "rp-light",
        max_load: 18 * 1024,
        max_zero: 20 * 1024,
    },
];

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
pub fn check(release_dir: &Path, images: &[&str]) -> Result<(), String> {
    // Every image needs a budget: an unbudgeted image would grow unseen.
    if let Some(missing) = images
        .iter()
        .find(|i| BUDGETS.iter().all(|b| b.image != **i))
    {
        return Err(format!(
            "{missing} has no size budget in xtask/src/size.rs (ADR-0005 rule 7)"
        ));
    }
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
