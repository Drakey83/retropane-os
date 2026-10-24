//! First-level page tables (ARMv5 / XScale "section" mappings, plus the top
//! MiB as 4 KiB pages for the vectors and KData: [`high_pages`]) and the
//! bring-up memory map.
//!
//! This is a flat identity map for bring-up: it turns on the MMU so the caches,
//! the write buffer and high exception vectors work. The kernel's real virtual
//! memory (demand paging, ADR-0005; CE5 slots, ADR-0003) replaces it.

/// One 1 MiB section's memory type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Memory {
    /// Write-back cacheable, bufferable: ordinary RAM.
    Normal,
    /// Uncached but bufferable: writes may be combined, reads go to memory.
    /// For framebuffers and other memory the CPU mostly writes.
    WriteCombine,
    /// Uncached, unbuffered: device registers, accessed exactly as written.
    Device,
    /// Cached in the XScale's mini data cache, not the main one: the X bit
    /// (bit 12, TEX[0]) with C 1, B 0 (Intel's XScale core developer's
    /// manual, the X bit's table; unproven on the X51v until the CPU gate's
    /// V1/C5 probe run). For streaming data that would evict hot data.
    Mini,
}

/// One region of the map: `len` bytes (a multiple of 1 MiB) from `virt` to
/// `phys`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    /// Virtual base (1 MiB aligned).
    pub virt: u32,
    /// Physical base (1 MiB aligned).
    pub phys: u32,
    /// Length in bytes (multiple of 1 MiB).
    pub len: u32,
    /// Memory type.
    pub memory: Memory,
}

const MIB: u32 = 1 << 20;

/// A section descriptor: type `0b10`, domain 0, AP = read/write (privileged
/// and user, bring-up only), cache bits per `memory`.
#[must_use]
pub const fn section(phys: u32, memory: Memory) -> u32 {
    let (c, b, x) = match memory {
        Memory::Normal => (1, 1, 0),
        Memory::WriteCombine => (0, 1, 0),
        Memory::Device => (0, 0, 0),
        Memory::Mini => (1, 0, 1),
    };
    (phys & 0xFFF0_0000) | (x << 12) | (0b11 << 10) | (c << 3) | (b << 2) | 0b10
}

/// Fills a 4096-entry first-level table: every listed region mapped, the rest
/// faulting.
///
/// # Panics
/// If a region isn't 1 MiB aligned or wraps the address space.
pub fn build(table: &mut [u32; 4096], regions: &[Region]) {
    table.fill(0); // unmapped: translation fault
    for r in regions {
        assert!(
            r.virt % MIB == 0 && r.phys % MIB == 0 && r.len % MIB == 0,
            "region not 1 MiB aligned"
        );
        let first = (r.virt / MIB) as usize;
        let count = (r.len / MIB) as usize;
        assert!(first + count <= 4096, "region wraps the address space");
        for i in 0..count {
            table[first + i] = section(r.phys + i as u32 * MIB, r.memory);
        }
    }
}

/// SDRAM, identity-mapped by the bring-up map: 64 MiB.
pub const SDRAM: core::ops::Range<u32> = 0xA800_0000..0xAC00_0000;

/// Physical address of the bring-up first-level table: the 16 KiB just below
/// images' load address (0xA8008000), 16 KiB aligned as the MMU requires.
pub const L1_TABLE: u32 = 0xA800_4000;
/// Physical page holding the exception vectors, mapped at 0xFFFF0000.
pub const VECTORS_PHYS: u32 = 0xA80F_0000;

/// The KData page's user address (ADR-0010 rule 8): `PUserKData`
/// (`0xFFFFC800`) is inside it.
pub const KDATA_VIRT: u32 = 0xFFFF_C000;
/// Physical page behind [`KDATA_VIRT`] (reserved SDRAM, boot/link.x).
pub const KDATA_PHYS: u32 = 0xA80F_1000;
/// Physical second-level table for the top MiB once it is split into
/// pages ([`high_pages`]; 1 KiB, reserved SDRAM, boot/link.x).
pub const HIGH_L2: u32 = 0xA80F_2000;
/// The top MiB's index in the first-level table.
pub const HIGH_L1_INDEX: usize = 0xFFF;

/// A first-level coarse-table descriptor (domain 0) for the 1 KiB-aligned
/// second-level table at `table`.
#[must_use]
pub const fn coarse(table: u32) -> u32 {
    (table & !0x3FF) | 0b01
}

/// A second-level small-page (4 KiB) descriptor, write-back cacheable,
/// with `ap` (`0b01` kernel only, `0b10` kernel read/write and user
/// read-only) for all four subpages.
#[must_use]
pub const fn small_page(phys: u32, ap: u32) -> u32 {
    let ap4 = (ap << 10) | (ap << 8) | (ap << 6) | (ap << 4);
    (phys & 0xFFFF_F000) | ap4 | (1 << 3) | (1 << 2) | 0b10
}

/// A second-level large-page (64 KiB) descriptor, write-back cacheable,
/// `ap` for all four subpages; the table holds it in 16 consecutive
/// entries (one per 4 KiB of the page).
#[must_use]
pub const fn large_page(phys: u32, ap: u32) -> u32 {
    let ap4 = (ap << 10) | (ap << 8) | (ap << 6) | (ap << 4);
    (phys & 0xFFFF_0000) | ap4 | (1 << 3) | (1 << 2) | 0b01
}

/// How a test window's memory is mapped (the CPU gate's C4 and V7: the
/// random hop by page size).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pages {
    /// 1 MiB sections (the bring-up map's way).
    Sections,
    /// 64 KiB large pages.
    Large,
    /// 4 KiB small pages (the kernel's map).
    Small,
    /// 1 MiB sections cached in the mini data cache ([`Memory::Mini`]).
    MiniSections,
}

/// The second-level tables mapping `mib` MiB of physical memory from
/// `phys` (1 MiB aligned) as `pages`, kernel read/write, write-back: 256
/// words a MiB into `l2`. Nothing to do for [`Pages::Sections`].
///
/// # Panics
/// If `l2` holds fewer than 256 words a MiB.
pub fn window_tables(l2: &mut [u32], phys: u32, mib: u32, pages: Pages) {
    assert!(
        l2.len() >= 256 * mib as usize,
        "second-level tables too small"
    );
    for (k, d) in l2[..256 * mib as usize].iter_mut().enumerate() {
        let at = phys + (k as u32) * 4096;
        *d = match pages {
            Pages::Sections | Pages::MiniSections => 0,
            Pages::Large => large_page(at & !0xFFFF, 0b01),
            Pages::Small => small_page(at, 0b01),
        };
    }
}

/// The top MiB as pages instead of a section: the vector page (kernel
/// only: the section left it, and the rest of SDRAM's first MiB,
/// user-writable) and the KData page (user read-only, kernel read/write);
/// the rest faults. A section can't give one page its own permissions.
pub fn high_pages(l2: &mut [u32; 256]) {
    l2.fill(0);
    l2[((0xFFFF_0000u32 >> 12) & 0xFF) as usize] = small_page(VECTORS_PHYS, 0b01);
    l2[((KDATA_VIRT >> 12) & 0xFF) as usize] = small_page(KDATA_PHYS, 0b10);
}

/// The X51v bring-up map (docs/hw: PXA27x memory map, X51v chip selects).
pub const X51V_BRINGUP: &[Region] = &[
    // CS0-CS2: boot ROM / DiskOnChip and other static memory: devices.
    Region {
        virt: 0x0000_0000,
        phys: 0x0000_0000,
        len: 0x0C00_0000,
        memory: Memory::Device,
    },
    // CS3: 2700G local memory (framebuffers): write-combining...
    Region {
        virt: 0x0C00_0000,
        phys: 0x0C00_0000,
        len: 0x03F0_0000,
        memory: Memory::WriteCombine,
    },
    // ...except the last MiB, which holds its registers at +0x3FE0000.
    Region {
        virt: 0x0FF0_0000,
        phys: 0x0FF0_0000,
        len: MIB,
        memory: Memory::Device,
    },
    // CS4/CS5 (CPLD), PC Card sockets (Wi-Fi, CF), PXA27x peripherals,
    // the LCD and memory controllers, and the USB host controller's MiB.
    Region {
        virt: 0x1000_0000,
        phys: 0x1000_0000,
        len: 0x3C10_0000,
        memory: Memory::Device,
    },
    // Internal-memory control (IMPMCR): device.
    Region {
        virt: 0x5800_0000,
        phys: 0x5800_0000,
        len: MIB,
        memory: Memory::Device,
    },
    // 256 KiB internal SRAM (the rest of the MiB is unused): normal memory.
    Region {
        virt: 0x5C00_0000,
        phys: 0x5C00_0000,
        len: MIB,
        memory: Memory::Normal,
    },
    // 64 MiB SDRAM.
    Region {
        virt: SDRAM.start,
        phys: SDRAM.start,
        len: SDRAM.end - SDRAM.start,
        memory: Memory::Normal,
    },
    // High vectors: the top MiB maps SDRAM's first MiB, so 0xFFFF0000 is
    // VECTORS_PHYS.
    Region {
        virt: 0xFFF0_0000,
        phys: 0xA800_0000,
        len: MIB,
        memory: Memory::Normal,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_encode_cache_policy() {
        assert_eq!(section(0xA800_0000, Memory::Normal), 0xA800_0C0E);
        assert_eq!(section(0x0C00_0000, Memory::WriteCombine), 0x0C00_0C06);
        assert_eq!(section(0x4000_0000, Memory::Device), 0x4000_0C02);
    }

    #[test]
    fn x51v_map_covers_what_retropane_touches() {
        let mut t = [0u32; 4096];
        build(&mut t, X51V_BRINGUP);
        let at = |a: u32| t[(a >> 20) as usize];
        assert_eq!(at(0xA800_8000), section(0xA800_0000, Memory::Normal)); // image
        assert_eq!(at(0x4010_0000) & 0xC, 0); // FFUART uncached
        assert_eq!(at(0x0FFE_0000), section(0x0FF0_0000, Memory::Device)); // 2700G regs
        assert_eq!(at(0x0C06_0000), section(0x0C00_0000, Memory::WriteCombine)); // framebuffer
        assert_eq!(at(0x1000_0000) & 0xC, 0); // CPLD uncached
        assert_eq!(at(0xFFFF_0000), section(0xA800_0000, Memory::Normal)); // vectors
        assert_eq!(at(0x8000_0000), 0); // unmapped faults
        assert_eq!(at(0x5C00_0000), section(0x5C00_0000, Memory::Normal)); // SRAM
        assert_eq!(at(0x5800_0000) & 0xC, 0); // IMPMCR uncached
        // The vector page and table live in the first, mapped, MiB of SDRAM.
        assert_eq!(VECTORS_PHYS & 0xFFF0_0000, 0xA800_0000);
        assert_eq!(L1_TABLE % (16 * 1024), 0);
    }

    #[test]
    fn the_top_mib_as_pages_holds_vectors_and_kdata_only() {
        let mut l2 = [0xFFFF_FFFFu32; 256];
        high_pages(&mut l2);
        let at = |a: u32| l2[((a >> 12) & 0xFF) as usize];
        // Vectors: kernel-only (AP 01 in every subpage), write-back.
        assert_eq!(at(0xFFFF_0000), 0xA80F_055E);
        // KData: user read-only (AP 10), write-back.
        assert_eq!(at(0xFFFF_C800), 0xA80F_1AAE);
        assert_eq!(l2.iter().filter(|&&d| d != 0).count(), 2, "the rest faults");
        assert_eq!(coarse(HIGH_L2), 0xA80F_2001);
        // Reserved pages between the vectors and the D-cache clean region.
        for p in [KDATA_PHYS, HIGH_L2] {
            assert!(p > VECTORS_PHYS && p + 4096 <= 0xA80F_8000);
        }
        assert_eq!(HIGH_L2 % 1024, 0);
        assert_eq!(HIGH_L1_INDEX, (KDATA_VIRT >> 20) as usize);
    }

    #[test]
    fn window_tables_map_every_4k_of_each_page_size() {
        let phys = 0xA900_0000;
        let mut l2 = [0xFFFF_FFFFu32; 512];
        window_tables(&mut l2, phys, 2, Pages::Small);
        // Each entry its own 4 KiB, kernel read/write (AP 01), write-back,
        // type small (0b10); the second MiB's table follows the first's.
        assert_eq!(l2[0], 0xA900_055E);
        assert_eq!(l2[1], 0xA900_155E);
        assert_eq!(l2[256], 0xA910_055E);
        assert_eq!(l2[511], 0xA91F_F55E);
        window_tables(&mut l2, phys, 2, Pages::Large);
        // Sixteen entries a 64 KiB page, each naming the page's base, type
        // large (0b01).
        assert!(l2[..16].iter().all(|&d| d == 0xA900_055D));
        assert_eq!(l2[16], 0xA901_055D);
        assert_eq!(l2[511], 0xA91F_055D);
        // What a walk would make of a virtual offset, both ways: the same
        // physical address.
        for off in [0u32, 0x1234, 0xF_FFFC, 0x10_0000, 0x1F_FFFC] {
            let d = l2[(off >> 12) as usize];
            assert_eq!((d & 0xFFFF_0000) | (off & 0xFFFF), phys + off);
        }
        window_tables(&mut l2, phys, 2, Pages::Sections);
        assert!(l2.iter().all(|&d| d == 0));
    }

    #[test]
    fn a_mini_cached_section_sets_x_with_c_and_not_b() {
        let d = section(0xA950_1234, Memory::Mini);
        assert_eq!(d, 0xA950_0000 | (1 << 12) | (0b11 << 10) | (1 << 3) | 0b10);
        // The other types leave the X bit clear (no section of ours moved
        // into the mini cache by this change).
        for m in [Memory::Normal, Memory::WriteCombine, Memory::Device] {
            assert_eq!(section(0xA950_0000, m) & (1 << 12), 0, "{m:?}");
        }
        let mut l2 = [0xFFFF_FFFFu32; 256];
        window_tables(&mut l2, 0xA900_0000, 1, Pages::MiniSections);
        assert!(l2.iter().all(|&d| d == 0), "sections need no tables");
    }

    #[test]
    #[should_panic(expected = "too small")]
    fn window_tables_refuse_a_short_table() {
        window_tables(&mut [0; 255], 0xA900_0000, 1, Pages::Small);
    }

    #[test]
    fn regions_do_not_overlap() {
        let mut seen = [false; 4096];
        for r in X51V_BRINGUP {
            for i in 0..r.len / MIB {
                let idx = (r.virt / MIB + i) as usize;
                assert!(!seen[idx], "section {idx:#x} mapped twice");
                seen[idx] = true;
            }
        }
    }

    #[test]
    #[should_panic(expected = "aligned")]
    fn rejects_unaligned_regions() {
        let mut t = [0u32; 4096];
        build(
            &mut t,
            &[Region {
                virt: 0x1000,
                phys: 0,
                len: MIB,
                memory: Memory::Device,
            }],
        );
    }
}
