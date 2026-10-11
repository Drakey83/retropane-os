//! Kernel memory management (ADR-0003, ADR-0005).
//!
//! Pure logic, tested on the host. So far: the physical frame allocator
//! ([`frames`]). The VM, page cache and reclaim build on it.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod frames;

/// Page and frame size: 4 KiB, the ARMv5 small page.
pub const PAGE_SIZE: usize = 4096;
