//! First-level page tables (ARMv5 / XScale "section" mappings) and the
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
    let (c, b) = match memory {
        Memory::Normal => (1, 1),
        Memory::WriteCombine => (0, 1),
        Memory::Device => (0, 0),
    };
    (phys & 0xFFF0_0000) | (0b11 << 10) | (c << 3) | (b << 2) | 0b10
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

/// Physical address of the bring-up first-level table: the 16 KiB just below
/// images' load address (0xA0008000), 16 KiB aligned as the MMU requires.
pub const L1_TABLE: u32 = 0xA000_4000;
/// Physical page holding the exception vectors, mapped at 0xFFFF0000.
pub const VECTORS_PHYS: u32 = 0xA00F_0000;

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
    // CS4/CS5 (CPLD), PC Card sockets (Wi-Fi, CF), PXA27x peripherals.
    Region {
        virt: 0x1000_0000,
        phys: 0x1000_0000,
        len: 0x3C00_0000,
        memory: Memory::Device,
    },
    // 64 MiB SDRAM.
    Region {
        virt: 0xA000_0000,
        phys: 0xA000_0000,
        len: 64 * MIB,
        memory: Memory::Normal,
    },
    // High vectors: the top MiB maps SDRAM's first MiB, so 0xFFFF0000 is
    // VECTORS_PHYS.
    Region {
        virt: 0xFFF0_0000,
        phys: 0xA000_0000,
        len: MIB,
        memory: Memory::Normal,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_encode_cache_policy() {
        assert_eq!(section(0xA000_0000, Memory::Normal), 0xA000_0C0E);
        assert_eq!(section(0x0C00_0000, Memory::WriteCombine), 0x0C00_0C06);
        assert_eq!(section(0x4000_0000, Memory::Device), 0x4000_0C02);
    }

    #[test]
    fn x51v_map_covers_what_retropane_touches() {
        let mut t = [0u32; 4096];
        build(&mut t, X51V_BRINGUP);
        let at = |a: u32| t[(a >> 20) as usize];
        assert_eq!(at(0xA000_8000), section(0xA000_0000, Memory::Normal)); // image
        assert_eq!(at(0x4010_0000) & 0xC, 0); // FFUART uncached
        assert_eq!(at(0x0FFE_0000), section(0x0FF0_0000, Memory::Device)); // 2700G regs
        assert_eq!(at(0x0C06_0000), section(0x0C00_0000, Memory::WriteCombine)); // framebuffer
        assert_eq!(at(0x1000_0000) & 0xC, 0); // CPLD uncached
        assert_eq!(at(0xFFFF_0000), section(0xA000_0000, Memory::Normal)); // vectors
        assert_eq!(at(0x8000_0000), 0); // unmapped faults
        // The vector page and table live in the first, mapped, MiB of SDRAM.
        assert_eq!(VECTORS_PHYS & 0xFFF0_0000, 0xA000_0000);
        assert_eq!(L1_TABLE % (16 * 1024), 0);
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
