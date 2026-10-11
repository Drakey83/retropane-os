//! ARMv5 / XScale page-table descriptors: pure encoding and decoding.
//!
//! Facts from the ARM Architecture Reference Manual (ARMv5, "Virtual memory
//! system architecture") and the Intel XScale Core Developer's Manual, with
//! the MMU's S and R bits clear (as `rp-hal-pxa27x` configures it):
//!
//! - First level: 4096 word entries, one per 1 MiB. `0b01` = coarse page
//!   table (base in bits 31:10), `0b10` = section (base in bits 31:20).
//!   Domain in bits 8:5. On XScale, bit 4 must be 0 (it is "should be one" on
//!   other ARMv5 cores; Linux clears it for XScale too).
//! - Coarse second-level tables: 256 word entries (1 KiB), one per 4 KiB.
//!   `0b10` = small page (4 KiB, base in 31:12), `0b01` = large page (64 KiB,
//!   base in 31:16, replicated in 16 consecutive, 16-aligned entries). Four
//!   access-permission fields (one per 1 KiB subpage) in bits 11:4; RetroPane
//!   always sets all four the same. C = bit 3, B = bit 2.
//! - AP (S = R = 0): `00` no access, `01` privileged read/write only, `10`
//!   privileged read/write and user read-only, `11` read/write for all.
//!
//! ARMv5 has **no execute-never bit**: any readable page can be executed.
//! RetroPane's W^X therefore means "code is never writable"; it cannot stop a
//! process executing its own data (see ADR-0008).

/// Cache policy of a mapping.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum Cache {
    /// Write-back, cacheable, bufferable: ordinary RAM.
    WriteBack,
    /// Uncached, bufferable: framebuffers and other write-mostly memory.
    WriteCombine,
    /// Uncached, unbuffered: device registers.
    Device,
}

impl Cache {
    const fn cb(self) -> u32 {
        match self {
            Cache::WriteBack => 0b11 << 2,
            Cache::WriteCombine => 0b01 << 2,
            Cache::Device => 0,
        }
    }

    const fn from_cb(cb: u32) -> Self {
        match cb {
            0b11 => Cache::WriteBack,
            0b01 => Cache::WriteCombine,
            _ => Cache::Device,
        }
    }
}

/// Hardware access permissions (the AP field).
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum Ap {
    /// No access in any mode (used to catch first touches, e.g. for
    /// reference tracking).
    None = 0b00,
    /// Kernel read/write, user no access.
    Kernel = 0b01,
    /// Kernel read/write, user read-only.
    UserRead = 0b10,
    /// Read/write for kernel and user.
    UserWrite = 0b11,
}

impl Ap {
    const fn from_bits(b: u32) -> Self {
        match b & 0b11 {
            0b00 => Ap::None,
            0b01 => Ap::Kernel,
            0b10 => Ap::UserRead,
            _ => Ap::UserWrite,
        }
    }

    /// Whether user mode may write.
    #[must_use]
    pub const fn user_writable(self) -> bool {
        matches!(self, Ap::UserWrite)
    }
}

/// The attributes of a page mapping.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub struct Attrs {
    /// Permissions.
    pub ap: Ap,
    /// Cache policy.
    pub cache: Cache,
}

/// Number of ARM domains.
pub const DOMAINS: u32 = 16;

/// A first-level coarse-table descriptor for the 1 KiB-aligned table at
/// `table_phys`, in `domain`.
#[must_use]
pub const fn l1_coarse(table_phys: u32, domain: u32) -> u32 {
    debug_assert!(table_phys & 0x3FF == 0, "coarse tables are 1 KiB aligned");
    debug_assert!(domain < DOMAINS, "no such domain");
    (table_phys & !0x3FF) | (domain << 5) | 0b01
}

/// A first-level section descriptor mapping 1 MiB at `phys`.
#[must_use]
pub const fn l1_section(phys: u32, domain: u32, attrs: Attrs) -> u32 {
    debug_assert!(phys & 0xF_FFFF == 0, "sections are 1 MiB aligned");
    debug_assert!(domain < DOMAINS, "no such domain");
    (phys & 0xFFF0_0000) | ((attrs.ap as u32) << 10) | (domain << 5) | attrs.cache.cb() | 0b10
}

/// A decoded first-level descriptor.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum L1 {
    /// Unmapped: accesses fault.
    Fault,
    /// Points at a coarse table.
    Coarse {
        /// The table's physical address.
        table_phys: u32,
        /// The domain.
        domain: u32,
    },
    /// A 1 MiB section.
    Section {
        /// Physical base.
        phys: u32,
        /// The domain.
        domain: u32,
        /// Permissions and caching.
        attrs: Attrs,
    },
    /// A fine table: never created by RetroPane.
    Fine,
}

/// Decodes a first-level descriptor.
#[must_use]
pub const fn l1_decode(d: u32) -> L1 {
    let domain = (d >> 5) & 0xF;
    match d & 0b11 {
        0b00 => L1::Fault,
        0b01 => L1::Coarse {
            table_phys: d & !0x3FF,
            domain,
        },
        0b10 => L1::Section {
            phys: d & 0xFFF0_0000,
            domain,
            attrs: Attrs {
                ap: Ap::from_bits(d >> 10),
                cache: Cache::from_cb((d >> 2) & 0b11),
            },
        },
        _ => L1::Fine,
    }
}

/// The AP field replicated into all four subpages.
const fn ap4(ap: Ap) -> u32 {
    let a = ap as u32;
    (a << 10) | (a << 8) | (a << 6) | (a << 4)
}

/// A second-level small-page (4 KiB) descriptor.
#[must_use]
pub const fn l2_small(phys: u32, attrs: Attrs) -> u32 {
    debug_assert!(phys & 0xFFF == 0, "small pages are 4 KiB aligned");
    (phys & 0xFFFF_F000) | ap4(attrs.ap) | attrs.cache.cb() | 0b10
}

/// A second-level large-page (64 KiB) descriptor: to be written to all 16
/// entries of an aligned group.
#[must_use]
pub const fn l2_large(phys: u32, attrs: Attrs) -> u32 {
    debug_assert!(phys & 0xFFFF == 0, "large pages are 64 KiB aligned");
    (phys & 0xFFFF_0000) | ap4(attrs.ap) | attrs.cache.cb() | 0b01
}

/// A decoded second-level descriptor.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum L2 {
    /// Unmapped.
    Fault,
    /// A 4 KiB page.
    Small {
        /// Physical base.
        phys: u32,
        /// Permissions and caching.
        attrs: Attrs,
    },
    /// One entry of a 64 KiB page.
    Large {
        /// Physical base of the whole 64 KiB page.
        phys: u32,
        /// Permissions and caching.
        attrs: Attrs,
    },
}

/// Decodes a second-level descriptor. XScale extended small pages (`0b11`)
/// are never created by RetroPane and decode as small pages.
#[must_use]
pub const fn l2_decode(d: u32) -> L2 {
    let attrs = Attrs {
        ap: Ap::from_bits(d >> 4),
        cache: Cache::from_cb((d >> 2) & 0b11),
    };
    match d & 0b11 {
        0b00 => L2::Fault,
        0b01 => L2::Large {
            phys: d & 0xFFFF_0000,
            attrs,
        },
        _ => L2::Small {
            phys: d & 0xFFFF_F000,
            attrs,
        },
    }
}

/// The Domain Access Control Register value: `client` domains check page
/// permissions (`01`), all others are denied (`00`). Manager mode (`11`,
/// which ignores permissions) is never used.
#[must_use]
pub const fn dacr(client: u32) -> u32 {
    let mut v = 0;
    let mut d = 0;
    while d < DOMAINS {
        if client & (1 << d) != 0 {
            v |= 0b01 << (2 * d);
        }
        d += 1;
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAM: Attrs = Attrs {
        ap: Ap::UserWrite,
        cache: Cache::WriteBack,
    };

    #[test]
    fn section_matches_the_hal_bring_up_encoding() {
        // rp-hal-pxa27x::mmu::section(0xA800_0000, Normal) == 0xA800_0C0E:
        // AP = 11, C = B = 1, domain 0, bit 4 clear (XScale).
        assert_eq!(l1_section(0xA800_0000, 0, RAM), 0xA800_0C0E);
        let dev = Attrs {
            ap: Ap::UserWrite,
            cache: Cache::Device,
        };
        assert_eq!(l1_section(0x4000_0000, 0, dev), 0x4000_0C02);
    }

    #[test]
    fn xscale_bit_4_is_never_set() {
        assert_eq!(l1_coarse(0xA800_0400, 15,) & 0x10, 0);
        assert_eq!(l1_section(0xA800_0000, 15, RAM) & 0x10, 0);
    }

    #[test]
    fn coarse_descriptor_layout() {
        // Base 31:10, domain 8:5, type 01.
        assert_eq!(l1_coarse(0xA812_3C00, 3), 0xA812_3C00 | (3 << 5) | 1);
        assert_eq!(
            l1_decode(l1_coarse(0xA812_3C00, 3)),
            L1::Coarse {
                table_phys: 0xA812_3C00,
                domain: 3
            }
        );
    }

    #[test]
    fn small_page_replicates_ap_into_all_subpages() {
        let ro = Attrs {
            ap: Ap::UserRead,
            cache: Cache::WriteBack,
        };
        // AP = 10 in bits 11:10, 9:8, 7:6, 5:4 -> 0xAA0; C B = 11; type 10.
        assert_eq!(l2_small(0xA800_5000, ro), 0xA800_5AAE);
        assert_eq!(
            l2_decode(0xA800_5AAE),
            L2::Small {
                phys: 0xA800_5000,
                attrs: ro
            }
        );
    }

    #[test]
    fn large_page_layout() {
        let d = l2_large(0xA801_0000, RAM);
        assert_eq!(d, 0xA801_0FFD); // AP 11 x4, C B, type 01
        assert_eq!(
            l2_decode(d),
            L2::Large {
                phys: 0xA801_0000,
                attrs: RAM
            }
        );
    }

    #[test]
    fn decode_round_trips_every_attribute() {
        for ap in [Ap::None, Ap::Kernel, Ap::UserRead, Ap::UserWrite] {
            for cache in [Cache::WriteBack, Cache::WriteCombine, Cache::Device] {
                let a = Attrs { ap, cache };
                assert_eq!(
                    l1_decode(l1_section(0x1230_0000, 7, a)),
                    L1::Section {
                        phys: 0x1230_0000,
                        domain: 7,
                        attrs: a
                    }
                );
                assert_eq!(
                    l2_decode(l2_small(0x1234_5000, a)),
                    L2::Small {
                        phys: 0x1234_5000,
                        attrs: a
                    }
                );
            }
        }
        assert_eq!(l1_decode(0), L1::Fault);
        assert_eq!(l2_decode(0), L2::Fault);
    }

    #[test]
    #[should_panic(expected = "1 KiB aligned")]
    fn misaligned_tables_are_refused() {
        let _ = l1_coarse(0xA800_0200, 0);
    }

    #[test]
    fn dacr_sets_client_bits_only() {
        assert_eq!(dacr(0), 0);
        assert_eq!(dacr(1), 0b01);
        assert_eq!(dacr((1 << 0) | (1 << 15)), 0x4000_0001);
    }
}
