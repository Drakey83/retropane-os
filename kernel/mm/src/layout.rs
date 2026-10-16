//! The virtual address space (ADR-0003): one 4 GiB space, CE5's layout below
//! 2 GiB so WM5/6.x binaries find what they expect, RetroPane's own above.
//!
//! | Virtual range | Use |
//! |---|---|
//! | `0x00000000..0x02000000` | Slot 0: the running process (FCSE relocates it) |
//! | `0x02000000..0x04000000` | Slot 1: shared code (CE5 put XIP DLL code here) |
//! | `0x04000000..0x42000000` | Slots 2-32: CE processes (Windows Mobile programs) |
//! | `0x42000000..0x80000000` | Shared memory-mapped files |
//! | `0x80000000..0xC0000000` | The kernel: device windows at `0x90000000` (`rp_hal_pxa27x::devmap`), RAM at its own address `0xA8000000` |
//! | `0xC0000000..0xFE000000` | Slots 96-126: native processes (RetroPane's services, drivers, own programs) |
//! | `0xFE000000..` | Slot 127, the kernel's: exception vectors (`0xFFFF0000`), KData (`0xFFFFC000`) |
//!
//! The CE5 layout below 2 GiB is from Microsoft's public Windows CE 5.0
//! documentation ("Memory Architecture"). Above it is RetroPane's own. CE's
//! 31 slots end where its shared area begins, but FCSE has 128 slots (a
//! 7-bit process ID): RetroPane's own processes take 31 of the slots above
//! the kernel, so Windows Mobile programs keep all of CE's
//! (docs/research/process-limit.md, the owner's vote of 2026-10-15). RAM is
//! mapped at its physical address, so the kernel reaches any frame (page
//! tables, page cache) without temporary mappings.
//!
//! **FCSE:** the XScale Fast Context Switch Extension adds the process ID to
//! any address in the bottom 32 MiB: the modified virtual address (MVA) is
//! `va | pid << 25` when `va < 32 MiB`. Caches and TLBs see MVAs, so each
//! process lives in its own slot and switching processes changes only the
//! PID register: no table change, no cache flush (ADR-0003). Page tables are
//! indexed by MVA.

/// Size of one process slot.
pub const SLOT_SIZE: u32 = 32 << 20;
/// First slot used for CE processes.
pub const FIRST_PROCESS_SLOT: u32 = 2;
/// Last slot used for CE processes: slots 2-32, so 31 (CE5's layout).
pub const LAST_PROCESS_SLOT: u32 = 32;
/// First slot for native processes (`0xC0000000`).
pub const FIRST_NATIVE_SLOT: u32 = 96;
/// Last slot for native processes: 96-126, 31 more (slot 127 holds the
/// vectors and KData).
pub const LAST_NATIVE_SLOT: u32 = 126;
/// Process slots in all, CE and native: the size of per-slot tables.
pub const PROCESS_SLOTS: usize = (LAST_PROCESS_SLOT - FIRST_PROCESS_SLOT + 1 + LAST_NATIVE_SLOT
    - FIRST_NATIVE_SLOT
    + 1) as usize;
/// Base of the shared memory-mapped file area.
pub const SHARED_BASE: u32 = 0x4200_0000;
/// Base of the kernel's range (up to [`NATIVE_BASE`]).
pub const KERNEL_BASE: u32 = 0x8000_0000;
/// Base of the native processes' slots.
pub const NATIVE_BASE: u32 = FIRST_NATIVE_SLOT * SLOT_SIZE;
/// End of the native processes' slots.
pub const NATIVE_END: u32 = (LAST_NATIVE_SLOT + 1) * SLOT_SIZE;
/// Kernel direct map: virtual base. RAM is mapped at its physical address,
/// where the kernel image runs.
pub const DIRECT_MAP_BASE: u32 = RAM_PHYS;
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

/// Whether `slot` holds a process (CE or native).
#[must_use]
pub const fn is_process_slot(slot: u32) -> bool {
    (slot >= FIRST_PROCESS_SLOT && slot <= LAST_PROCESS_SLOT) || is_native_slot(slot)
}

/// Whether `slot` is a native process's (RetroPane's own layout).
#[must_use]
pub const fn is_native_slot(slot: u32) -> bool {
    slot >= FIRST_NATIVE_SLOT && slot <= LAST_NATIVE_SLOT
}

/// A process slot's index in per-slot tables (`0..PROCESS_SLOTS`): CE's
/// slots first, then the native ones.
#[must_use]
pub const fn slot_index(slot: u32) -> Option<usize> {
    if slot >= FIRST_PROCESS_SLOT && slot <= LAST_PROCESS_SLOT {
        Some((slot - FIRST_PROCESS_SLOT) as usize)
    } else if is_native_slot(slot) {
        Some((slot - FIRST_NATIVE_SLOT + LAST_PROCESS_SLOT - FIRST_PROCESS_SLOT + 1) as usize)
    } else {
        None
    }
}

/// Base address of slot `slot`.
#[must_use]
pub const fn slot_base(slot: u32) -> u32 {
    debug_assert!(slot <= LAST_NATIVE_SLOT, "no such slot");
    slot * SLOT_SIZE
}

/// The slot containing `mva`, if it's in a process area (CE's, below the
/// shared area, or the native slots).
#[must_use]
pub const fn slot_of(mva: u32) -> Option<u32> {
    if mva < SHARED_BASE || (mva >= NATIVE_BASE && mva < NATIVE_END) {
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

/// Whether `mva` belongs to user space: CE processes and the shared area
/// below 2 GiB, native processes above the kernel.
#[must_use]
pub const fn is_user(mva: u32) -> bool {
    mva < KERNEL_BASE || (mva >= NATIVE_BASE && mva < NATIVE_END)
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
        const { assert!(DIRECT_MAP_BASE >= KERNEL_BASE && DIRECT_MAP_BASE + RAM_SIZE <= NATIVE_BASE) };
    }

    #[test]
    fn user_and_kernel_ranges() {
        assert!(is_user(0x7FFF_FFFF));
        assert!(!is_user(KERNEL_BASE));
        assert!(!is_user(0xA800_0000), "RAM, where the kernel runs");
        assert!(!is_user(0x9000_0000), "device windows");
        assert!(is_user(NATIVE_BASE));
        assert!(is_user(NATIVE_END - 1));
        assert!(!is_user(NATIVE_END), "slot 127: vectors and KData");
        assert!(!is_user(0xFFFF_0000));
    }

    #[test]
    fn native_slots_sit_between_the_kernel_and_the_vectors() {
        assert_eq!(NATIVE_BASE, 0xC000_0000);
        assert_eq!(NATIVE_END, 0xFE00_0000);
        // The kernel's own windows stay below them (devmap ends at
        // 0x9C100000, RAM at 0xAC000000).
        const { assert!(DIRECT_MAP_BASE + RAM_SIZE <= NATIVE_BASE) };
        const { assert!(0x9C10_0000 <= NATIVE_BASE) };
        // FCSE's process ID is 7 bits: every slot is reachable.
        const { assert!(LAST_NATIVE_SLOT < 128) };
        assert_eq!(PROCESS_SLOTS, 62);
        // FCSE puts a native process's slot-0 view in its own slot.
        assert_eq!(mva(0x0001_0000, 96), 0xC001_0000);
        assert_eq!(slot_of(mva(0x0001_0000, 100)), Some(100));
        assert_eq!(slot_of(NATIVE_END), None);
        assert_eq!(slot_of(KERNEL_BASE), None);
    }

    #[test]
    fn slot_indexes_are_dense_and_distinct() {
        let mut seen = [false; PROCESS_SLOTS];
        for slot in 0..128 {
            match slot_index(slot) {
                Some(i) => {
                    assert!(is_process_slot(slot));
                    assert!(!seen[i], "slot {slot}: index {i} twice");
                    seen[i] = true;
                }
                None => assert!(!is_process_slot(slot), "slot {slot}"),
            }
        }
        assert!(seen.iter().all(|&s| s));
        assert_eq!(slot_index(FIRST_PROCESS_SLOT), Some(0));
        assert_eq!(slot_index(FIRST_NATIVE_SLOT), Some(31));
        assert!(!is_native_slot(LAST_PROCESS_SLOT));
        assert!(is_native_slot(FIRST_NATIVE_SLOT));
    }
}
