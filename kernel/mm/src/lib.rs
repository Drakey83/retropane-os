//! Kernel memory management (ADR-0003, ADR-0005).
//!
//! Pure logic, tested on the host:
//!
//! - [`frames`]: physical frame allocator (buddy).
//! - [`arm`]: ARMv5/XScale page-table descriptors.
//! - [`layout`]: the address space (CE5 slots, FCSE, kernel direct map).
//! - [`pagetable`]: the system's page tables, with VIVT-safe unmapping.
//! - [`space`]: address spaces, demand paging and copy-on-write (ADR-0008).
//! - [`zpool`]: the compressed page store for compressed RAM (ADR-0005
//!   rule 6).
//!
//! Reclaim and the page cache itself build on these.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod arm;
pub mod domains;
pub mod frames;
pub mod layout;
pub mod pagetable;
pub mod regions;
pub mod space;
pub mod zpool;

/// Page and frame size: 4 KiB, the ARMv5 small page.
pub const PAGE_SIZE: usize = 4096;
