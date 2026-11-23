//! Where device windows are in the virtual address space.
//!
//! Bring-up images run on an identity map: a device's virtual address is its
//! physical one. The kernel can't keep that: the X51v's chip-select windows
//! (2700G at `0x0C000000`, CPLD at `0x10000000`) sit in the process slots, and
//! the peripheral block (`0x40000000+`) runs into the shared mapped-file area
//! (ADR-0003, ADR-0008). So the kernel maps every window into kernel space
//! ([`KERNEL_WINDOWS`], from `0x90000000`, above the RAM direct map) and then
//! calls [`use_kernel_map`]. From then on every HAL handout and register
//! access goes through [`va`]. Handles taken before the switch keep their old
//! addresses, so the kernel switches before taking any.

use crate::mmu::Memory;

/// One device window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    /// Physical base (1 MiB aligned).
    pub phys: u32,
    /// Kernel virtual base (1 MiB aligned).
    pub virt: u32,
    /// Length (multiple of 1 MiB).
    pub len: u32,
    /// Memory type.
    pub memory: Memory,
}

const MIB: u32 = 1 << 20;

/// The kernel's device map.
pub const KERNEL_WINDOWS: &[Window] = &[
    // PXA27x peripherals: UARTs, timers, interrupt controller, GPIO, clocks,
    // keypad, SSP, MMC (0x40000000-0x43FFFFFF).
    Window {
        phys: 0x4000_0000,
        virt: 0x9000_0000,
        len: 64 * MIB,
        memory: Memory::Device,
    },
    // LCD controller, memory controller, USB host, internal-memory control.
    Window {
        phys: 0x4400_0000,
        virt: 0x9400_0000,
        len: MIB,
        memory: Memory::Device,
    },
    Window {
        phys: 0x4800_0000,
        virt: 0x9410_0000,
        len: MIB,
        memory: Memory::Device,
    },
    Window {
        phys: 0x4C00_0000,
        virt: 0x9420_0000,
        len: MIB,
        memory: Memory::Device,
    },
    Window {
        phys: 0x5800_0000,
        virt: 0x9430_0000,
        len: MIB,
        memory: Memory::Device,
    },
    // 256 KiB internal SRAM.
    Window {
        phys: 0x5C00_0000,
        virt: 0x9440_0000,
        len: MIB,
        memory: Memory::Normal,
    },
    // 2700G: local memory (write-combining), then its register MiB.
    Window {
        phys: 0x0C00_0000,
        virt: 0x9800_0000,
        len: 63 * MIB,
        memory: Memory::WriteCombine,
    },
    Window {
        phys: 0x0FF0_0000,
        virt: 0x9BF0_0000,
        len: MIB,
        memory: Memory::Device,
    },
    // Board CPLD (CS4).
    Window {
        phys: 0x1000_0000,
        virt: 0x9C00_0000,
        len: MIB,
        memory: Memory::Device,
    },
    // CompactFlash (PC Card socket 1): its attribute and common memory
    // windows' first MiB (the X51v HAL hands out 4 KiB of each; the card
    // log writes there). Missing until 2026-11-23: the kernel's first run
    // on the X51v panicked on its first line to the CF card's log.
    Window {
        phys: 0x3800_0000,
        virt: 0x9C10_0000,
        len: MIB,
        memory: Memory::Device,
    },
    Window {
        phys: 0x3C00_0000,
        virt: 0x9C20_0000,
        len: MIB,
        memory: Memory::Device,
    },
];

/// The kernel virtual address of physical `phys` under `windows`.
#[must_use]
pub fn translate(windows: &[Window], phys: u32) -> Option<u32> {
    windows
        .iter()
        .find(|w| phys >= w.phys && phys - w.phys < w.len)
        .map(|w| w.virt + (phys - w.phys))
}

#[cfg(all(target_arch = "arm", target_os = "none"))]
mod target {
    static mut KERNEL_MAP: bool = false;

    /// The virtual address to use for device physical address `phys` under
    /// the current map.
    ///
    /// # Panics
    /// Under the kernel map, if no window covers `phys` (a device the kernel
    /// didn't map: a bug, never a silent wrong address).
    #[must_use]
    pub fn va(phys: usize) -> usize {
        // SAFETY: a plain read of a flag written once, before any device
        // handle is taken under the new map.
        if unsafe { core::ptr::addr_of!(KERNEL_MAP).read() } {
            super::translate(super::KERNEL_WINDOWS, phys as u32)
                .expect("device not in the kernel map") as usize
        } else {
            phys
        }
    }

    /// Whether the kernel device map is in use.
    #[must_use]
    pub fn kernel_map() -> bool {
        // SAFETY: as in `va`.
        unsafe { core::ptr::addr_of!(KERNEL_MAP).read() }
    }

    /// Switches the HAL to the kernel device map. Call once, right after the
    /// MMU starts using tables that map [`super::KERNEL_WINDOWS`], before
    /// taking any device handle.
    pub fn use_kernel_map() {
        // SAFETY: single core, called during kernel setup with IRQs masked.
        unsafe { core::ptr::addr_of_mut!(KERNEL_MAP).write(true) }
    }
}

#[cfg(all(target_arch = "arm", target_os = "none"))]
pub use target::{kernel_map, use_kernel_map, va};

/// Identity on the host (tests never touch hardware).
#[cfg(not(all(target_arch = "arm", target_os = "none")))]
#[must_use]
pub fn va(phys: usize) -> usize {
    phys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_are_aligned_disjoint_and_in_kernel_space() {
        for (i, a) in KERNEL_WINDOWS.iter().enumerate() {
            assert!(
                a.phys % MIB == 0 && a.virt % MIB == 0 && a.len % MIB == 0,
                "{a:?}"
            );
            assert!(
                a.virt >= 0x9000_0000 && u64::from(a.virt) + u64::from(a.len) <= 0xA800_0000,
                "{a:?}"
            );
            for b in &KERNEL_WINDOWS[i + 1..] {
                let overlap = |x0: u32, xl: u32, y0: u32, yl: u32| x0 < y0 + yl && y0 < x0 + xl;
                assert!(
                    !overlap(a.virt, a.len, b.virt, b.len),
                    "virtual overlap {a:?} {b:?}"
                );
                assert!(
                    !overlap(a.phys, a.len, b.phys, b.len),
                    "physical overlap {a:?} {b:?}"
                );
            }
        }
    }

    #[test]
    fn every_device_the_hal_uses_is_mapped() {
        // UART, OS timer, interrupt controller, GPIO, clocks, keypad, SSP1,
        // MMC, memory controller, IMPMCR, SRAM, 2700G memory and registers,
        // CPLD.
        for phys in [
            0x4010_0000,
            0x40A0_0000,
            0x40D0_0000,
            0x40E0_0000,
            0x4130_0000,
            0x4150_0000,
            0x4100_0000,
            0x4110_0000,
            0x4800_0000,
            0x5800_0000,
            0x5C00_0000,
            0x0C06_0000,
            0x0FFE_0000,
            0x1000_0000,
        ] {
            assert!(
                translate(KERNEL_WINDOWS, phys).is_some(),
                "{phys:#x} unmapped"
            );
        }
        assert_eq!(translate(KERNEL_WINDOWS, 0x4010_0004), Some(0x9010_0004));
        assert_eq!(translate(KERNEL_WINDOWS, 0x0FFE_0000), Some(0x9BFE_0000));
        assert_eq!(
            translate(KERNEL_WINDOWS, 0xA800_0000),
            None,
            "RAM isn't a device"
        );
    }

    /// Every HAL module that names a peripheral by its physical address and
    /// reaches it through a raw pointer goes through [`va`]: under the
    /// kernel's map the physical address is not mapped (2026-11-23, Game
    /// Mode G7a: `dma.rs` wrote `DCSR` at `0x40000000` and the kernel took a
    /// data abort; the bring-up map's identity had hidden it in every image).
    #[test]
    fn raw_peripheral_registers_go_through_the_device_map() {
        extern crate std;
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut checked = 0;
        for e in std::fs::read_dir(&dir).unwrap() {
            let path = e.unwrap().path();
            if path.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            // Code only: a comment naming `devmap::va` proves nothing.
            let text = std::fs::read_to_string(&path).unwrap();
            let src: std::string::String = text
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .flat_map(|l| {
                    [
                        l, "
",
                    ]
                })
                .collect();
            let names_peripheral = src.lines().any(|l| {
                let l = l.trim_start();
                (l.starts_with("const ") || l.starts_with("pub const "))
                    && l.contains(": usize = 0x4")
            });
            let raw = src.contains(" as *mut u32).write_volatile")
                || src.contains(" as *const u32).read_volatile");
            if names_peripheral && raw {
                checked += 1;
                assert!(
                    src.contains("devmap::va(") || src.contains("use crate::devmap::va;"),
                    "{} reaches peripheral registers by raw pointer without devmap::va",
                    path.display()
                );
            }
        }
        assert!(
            checked >= 2,
            "the scan found {checked} modules: the pattern went stale"
        );
    }
}
