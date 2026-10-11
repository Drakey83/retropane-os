//! RetroPane HAL for the Intel PXA27x.
//!
//! HAL crates are the only place real MMIO (and therefore `unsafe`) is allowed
//! (DESIGN.md §3.1). Every `unsafe` block carries a `SAFETY:` comment, and
//! clippy enforces it.

#![no_std]

use core::ptr::NonNull;

use rp_hal_api::RegisterBus;

#[cfg(all(target_arch = "arm", target_os = "none"))]
pub mod rt;

/// Physical base of the full-function UART (FFUART).
pub const FFUART_BASE: usize = 0x4010_0000;
/// Size of each UART's register window.
pub const UART_WINDOW: usize = 0x30;

/// A memory-mapped register bank accessed with volatile 32-bit loads and stores.
#[derive(Debug)]
pub struct Mmio {
    base: NonNull<u32>,
    len: usize,
}

impl Mmio {
    /// Wraps the register bank at `base`, spanning `len` bytes.
    ///
    /// # Safety
    /// `base..base + len` must be a mapped, 4-byte-aligned register window that
    /// is valid for volatile reads and writes for as long as this `Mmio` lives,
    /// and nothing else may access it concurrently.
    #[must_use]
    pub const unsafe fn new(base: NonNull<u32>, len: usize) -> Self {
        Self { base, len }
    }

    fn reg(&self, offset: usize) -> *mut u32 {
        assert!(
            offset.is_multiple_of(4) && offset + 4 <= self.len,
            "register offset {offset:#x} out of range or unaligned"
        );
        // `offset / 4` is in bounds per the assertion above.
        self.base.as_ptr().wrapping_add(offset / 4)
    }
}

impl RegisterBus for Mmio {
    fn read32(&mut self, offset: usize) -> u32 {
        let reg = self.reg(offset);
        // SAFETY: `reg` is aligned and inside the window, which the caller of
        // `Mmio::new` guaranteed is valid for volatile access.
        unsafe { reg.read_volatile() }
    }

    fn write32(&mut self, offset: usize, value: u32) {
        let reg = self.reg(offset);
        // SAFETY: as in `read32`.
        unsafe { reg.write_volatile(value) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_hal_api::Field;

    // Volatile access works on ordinary memory too, so a local array stands in
    // for a register window on the host.
    fn with_window(f: impl FnOnce(&mut Mmio)) {
        let mut regs = [0u32; 4];
        let base = NonNull::new(regs.as_mut_ptr()).unwrap();
        // SAFETY: `regs` outlives `mmio`, is aligned and is 16 bytes long.
        let mut mmio = unsafe { Mmio::new(base, 16) };
        f(&mut mmio);
    }

    #[test]
    fn reads_back_writes() {
        with_window(|mmio| {
            mmio.write32(0x8, 0xDEAD_BEEF);
            assert_eq!(mmio.read32(0x8), 0xDEAD_BEEF);
            mmio.modify(0x8, Field::new(0, 16), 0);
            assert_eq!(mmio.read32(0x8), 0xDEAD_0000);
        });
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn rejects_out_of_range() {
        with_window(|mmio| {
            mmio.read32(0x10);
        });
    }
}
