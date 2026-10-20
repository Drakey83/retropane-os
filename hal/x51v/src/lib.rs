//! Board description for the Dell Axim X51v.
//!
//! Every hardware fact carries a [`Confidence`] and a source, following the
//! spec rules in `docs/CLEANROOM.md` §4. Facts marked [`Confidence::Guess`] must
//! be confirmed on the device before a driver relies on them.

#![no_std]

#[cfg(all(target_arch = "arm", target_os = "none"))]
pub mod devices;
pub mod kept;
pub mod vcore;

/// How sure we are of a hardware fact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    /// Confirmed by a datasheet, or by working code *and* a test on the device.
    Confirmed,
    /// Supported by working prior code or several sources, not yet tested by us.
    Likely,
    /// Inferred; must be verified before use.
    Guess,
}

/// A region of the physical address space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    /// Short identifier.
    pub name: &'static str,
    /// Physical base address.
    pub base: u32,
    /// Size in bytes.
    pub size: u32,
    /// How sure we are.
    pub confidence: Confidence,
    /// Where the fact comes from.
    pub source: &'static str,
}

impl Region {
    /// One past the last byte, as a `u64` so the top of the 4 GB space fits.
    #[must_use]
    pub const fn end(&self) -> u64 {
        self.base as u64 + self.size as u64
    }

    /// Whether the two regions share any address.
    #[must_use]
    pub const fn overlaps(&self, other: &Region) -> bool {
        (self.base as u64) < other.end() && (other.base as u64) < self.end()
    }
}

const PXA_MANUAL: &str = "PXA27x Developer's Manual, memory map";
const AXDROID: &str = "AxDroid board file (aximx50.c), facts only";

/// The X51v physical memory map, as currently understood.
pub const MEMORY_MAP: &[Region] = &[
    Region {
        name: "cs3_2700g",
        base: 0x0C00_0000,
        size: 0x0400_0000,
        confidence: Confidence::Likely,
        source: AXDROID,
    },
    Region {
        name: "cs4_cpld",
        base: 0x1000_0000,
        size: 0x0400_0000,
        confidence: Confidence::Likely,
        source: AXDROID,
    },
    Region {
        name: "pcmcia0_wifi",
        base: 0x2000_0000,
        size: 0x1000_0000,
        confidence: Confidence::Likely,
        source: AXDROID,
    },
    Region {
        name: "pcmcia1_cf",
        base: 0x3000_0000,
        size: 0x1000_0000,
        confidence: Confidence::Confirmed,
        source: PXA_MANUAL,
    },
    Region {
        name: "peripherals",
        base: 0x4000_0000,
        size: 0x0400_0000,
        confidence: Confidence::Confirmed,
        source: PXA_MANUAL,
    },
    Region {
        name: "lcd",
        base: 0x4400_0000,
        size: 0x0400_0000,
        confidence: Confidence::Confirmed,
        source: PXA_MANUAL,
    },
    Region {
        name: "memory_ctrl",
        base: 0x4800_0000,
        size: 0x0400_0000,
        confidence: Confidence::Confirmed,
        source: PXA_MANUAL,
    },
    Region {
        name: "sdram",
        base: 0xA800_0000,
        size: 64 * 1024 * 1024,
        confidence: Confidence::Confirmed,
        source: "Dell OAL address table (PA 0xA8000000, 64 MiB; docs/hw/memory-map.md) + HaRET ramaddr",
    },
];

/// Offset of the 2700G register block inside its chip-select window.
pub const MBX_REG_OFFSET: u32 = 0x03FE_0000;
/// Size of the modelled 2700G register window.
pub const MBX_REG_SIZE: u32 = 0x2_0000;
/// 2700G local memory size on the X51v (2700G7).
pub const MBX_VRAM_SIZE: u32 = 16 * 1024 * 1024;
/// Confidence in [`MBX_REG_OFFSET`]: Dell's display driver uses the block at
/// the matching virtual address (docs/hw/2700g.md), agreeing with AxDroid.
pub const MBX_REG_CONFIDENCE: Confidence = Confidence::Confirmed;

/// Looks up a region by name.
#[must_use]
pub fn region(name: &str) -> Option<&'static Region> {
    MEMORY_MAP.iter().find(|r| r.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_do_not_overlap() {
        for (i, a) in MEMORY_MAP.iter().enumerate() {
            for b in &MEMORY_MAP[i + 1..] {
                assert!(!a.overlaps(b), "{} overlaps {}", a.name, b.name);
            }
        }
    }

    #[test]
    fn regions_fit_in_32_bits_and_are_page_aligned() {
        for r in MEMORY_MAP {
            assert!(r.end() <= 1 << 32, "{} runs past 4 GB", r.name);
            assert_eq!(r.base % 4096, 0, "{} base not page aligned", r.name);
        }
    }

    #[test]
    fn mbx_registers_sit_inside_the_2700g_window() {
        let cs3 = region("cs3_2700g").unwrap();
        assert!(MBX_REG_OFFSET < cs3.size);
    }

    #[test]
    fn overlap_detection_works() {
        let a = Region {
            name: "a",
            base: 0,
            size: 0x100,
            confidence: Confidence::Guess,
            source: "",
        };
        let b = Region { base: 0xFF, ..a };
        let c = Region { base: 0x100, ..a };
        assert!(a.overlaps(&b));
        assert!(!a.overlaps(&c));
    }

    #[test]
    fn unknown_region_is_none() {
        assert!(region("nope").is_none());
    }
}
