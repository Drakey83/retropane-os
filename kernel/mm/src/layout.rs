//! The virtual address space (ADR-0003): one 4 GiB space, CE5's layout below
//! 2 GiB so WM5/6.x binaries find what they expect, RetroPane's own kernel
//! layout above.
//!
//! | Virtual range | Use |
//! |---|---|
//! | `0x00000000..0x02000000` | Slot 0: the running process (FCSE relocates it) |
//! | `0x02000000..0x04000000` | Slot 1: shared code (CE5 put XIP DLL code here) |
//! | `0x04000000..0x42000000` | Slots 2-32: one per process |
//! | `0x42000000..0x80000000` | Shared memory-mapped files |
//! | `0x80000000..0x84000000` | Kernel direct map of all 64 MiB of RAM |
//! | `0xFFFF0000` | Exception vectors |
//!
//! The CE5 layout below 2 GiB is from Microsoft's public Windows CE 5.0
//! documentation ("Memory Architecture"). The kernel half is RetroPane's own:
//! the direct map lets the kernel reach any frame (page tables, page cache)
//! without temporary mappings.
//!
//! **FCSE:** the XScale Fast Context Switch Extension adds the process ID to
//! any address in the bottom 32 MiB: the modified virtual address (MVA) is
//! `va | pid << 25` when `va < 32 MiB`. Caches and TLBs see MVAs, so each
//! process lives in its own slot and switching processes changes only the
//! PID register: no table change, no cache flush (ADR-0003). Page tables are
//! indexed by MVA.

/// Size of one process slot.
pub const SLOT_SIZE: u32 = 32 << 20;
/// First slot used for processes.
pub const FIRST_PROCESS_SLOT: u32 = 2;
/// Last slot used for processes (CE5 allows 32 processes).
pub const LAST_PROCESS_SLOT: u32 = 32;
/// Base of the shared memory-mapped file area.
pub const SHARED_BASE: u32 = 0x4200_0000;
/// Base of the kernel half.
pub const KERNEL_BASE: u32 = 0x8000_0000;
/// Kernel direct map: virtual base.
pub const DIRECT_MAP_BASE: u32 = 0x8000_0000;
/// Physical base of SDRAM on the X51v (docs/hw/memory-map.md).
pub const RAM_PHYS: u32 = 0xA800_0000;
/// SDRAM size.
pub const RAM_SIZE: u32 = 64 << 20;
/// Lowest address a process image may use (CE reserves the first 64 KiB, so
/// null-pointer accesses always fault).
pub const PROCESS_IMAGE_BASE: u32 = 0x0001_0000;

/// The modified virtual address the MMU sees for `va` while `pid` runs.
#[must_use]
pub const fn mva(va: u32, pid: u32) -> u32 {
    debug_assert!(pid < 128, "the FCSE PID is 7 bits");
    if va < SLOT_SIZE { va | (pid << 25) } else { va }
}

/// Base address of slot `slot`.
#[must_use]
pub const fn slot_base(slot: u32) -> u32 {
    debug_assert!(slot <= LAST_PROCESS_SLOT, "no such slot");
    slot * SLOT_SIZE
}

/// The slot containing `mva`, if it's in the process area.
#[must_use]
pub const fn slot_of(mva: u32) -> Option<u32> {
    if mva < SHARED_BASE {
        Some(mva / SLOT_SIZE)
    } else {
        None
    }
}

/// Kernel virtual address of physical RAM address `phys`, if it's RAM.
#[must_use]
pub const fn phys_to_kernel(phys: u32) -> Option<u32> {
    if phys >= RAM_PHYS && phys - RAM_PHYS < RAM_SIZE {
        Some(phys - RAM_PHYS + DIRECT_MAP_BASE)
    } else {
        None
    }
}

/// Whether `mva` belongs to the user half (processes and shared mappings).
#[must_use]
pub const fn is_user(mva: u32) -> bool {
    mva < KERNEL_BASE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ce5_layout_facts() {
        // Microsoft, "Memory Architecture (Windows CE 5.0)": 32 MB slots, 33
        // of them below 0x42000000, then the shared area up to 2 GB.
        assert_eq!(SLOT_SIZE, 0x0200_0000);
        assert_eq!(slot_base(LAST_PROCESS_SLOT) + SLOT_SIZE, SHARED_BASE);
        assert_eq!(slot_base(FIRST_PROCESS_SLOT), 0x0400_0000);
        assert_eq!(KERNEL_BASE, 0x8000_0000);
    }

    #[test]
    fn fcse_relocates_only_the_bottom_slot() {
        assert_eq!(mva(0x0001_0000, 5), 0x0A01_0000);
        assert_eq!(mva(0x01FF_FFFF, 5), 0x0BFF_FFFF);
        assert_eq!(mva(0x0200_0000, 5), 0x0200_0000); // slot 1: untouched
        assert_eq!(mva(0x8000_0000, 5), 0x8000_0000);
        assert_eq!(mva(0x0001_0000, 0), 0x0001_0000);
        assert_eq!(slot_of(mva(0x1234, 7)), Some(7));
        assert_eq!(slot_of(SHARED_BASE), None);
    }

    #[test]
    fn direct_map_covers_exactly_ram() {
        assert_eq!(phys_to_kernel(RAM_PHYS), Some(DIRECT_MAP_BASE));
        assert_eq!(
            phys_to_kernel(RAM_PHYS + RAM_SIZE - 1),
            Some(DIRECT_MAP_BASE + RAM_SIZE - 1)
        );
        assert_eq!(phys_to_kernel(RAM_PHYS + RAM_SIZE), None);
        assert_eq!(phys_to_kernel(0x0C00_0000), None); // 2700G memory
        const { assert!(DIRECT_MAP_BASE + RAM_SIZE <= 0xFFFF_0000) };
    }

    #[test]
    fn user_and_kernel_halves() {
        assert!(is_user(0x7FFF_FFFF));
        assert!(!is_user(KERNEL_BASE));
    }
}
