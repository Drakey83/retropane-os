# ADR-0005: Memory and storage architecture: designed for 64 MB

- **Status:** Proposed (to be accepted before Phase 2 kernel memory management lands)
- **Date:** 2026-10-10

## Context

The constraints that decide everything:

- **64 MB RAM** in total; Windows Mobile reports about 52 MB free on the user's X51v.
- **The flash can't execute in place.** The X51v stores its OS on an M-Systems
  DiskOnChip G3, a block device behind a controller, not memory-mapped NOR. Every
  instruction that runs has to be copied into RAM first, so code competes with
  data for the same 64 MB.
- **Storage is slow and small:** about 256 MB of DiskOnChip flash plus the SD card.
- **No FPU, VIVT caches, 624 MHz, one core.** Decompressing in software is
  affordable; cache flushes are not (ADR-0003).
- Windows Mobile ran in this envelope with demand paging out of IMGFS, and
  RetroPane needs to fit too, with more features (TLS, a modern browser, a
  compositor).

"Brute force", meaning static images, everything resident, duplicated libraries,
uncompressed assets and general-purpose Rust machinery everywhere, doesn't fit.

## Decision

Ten rules, each owned by a layer and checked in CI where possible.

1. **Demand-paged code.** Executables and libraries (native and WM5/6.x PE) are
   mapped, not loaded. Pages come in on first touch, and clean pages are dropped
   under memory pressure and re-read later. Resident code ≈ the working set, not
   the total size. *(Kernel VM, Phase 2.)*
2. **One copy of everything shared.** Libraries live once in the CE5-style shared
   DLL region (ADR-0003) and map into every process. OS services are separate
   processes exposing IPC interfaces, rather than being statically linked into
   every app, so big crates (TLS, layout, fonts) exist once.
3. **Unified page cache.** File data, mapped code and SD/flash reads share one
   cache, with DMA straight into cache pages and no intermediate copies.
4. **Compressed storage, fast decompression.** System images and assets are stored
   compressed (an LZ4-class format; iWMMXt-optimised decompressor). That buys
   more flash and less I/O at a small CPU cost, which matters because flash
   reads are slow.
5. **The 2700G's 16 MB is a second memory pool.** Framebuffers, window surfaces,
   textures, glyph and image caches live in the GPU's local memory, not system
   RAM. That's effectively 25% more memory for graphics-heavy work.
6. **Compressed RAM for idle state.** Anonymous memory of background apps is
   compressed in RAM (zram-style) before anything is discarded, since there's no
   swap device worth using.
7. **Budgets are part of the build.** Every image and subsystem has a size budget
   (DESIGN §11). `cargo xtask` reports sizes and fails when a budget is exceeded,
   and budgets only ever get tightened. *(Code-size check: in place now for the
   bring-up image; RAM budgets arrive with the kernel.)*
8. **Rust built for size where speed doesn't matter.** Per-crate optimisation
   levels (`opt-level = "s"`/`"z"` for cold code, `3` for hot paths), LTO,
   `panic = "abort"`, one codegen unit. No `core::fmt` in the kernel or drivers'
   fast paths (use a tiny formatter); no `Debug` derives in release images;
   avoid generic code instantiated many times (prefer one concrete type or
   `dyn` in drivers that are built for a single board).
9. **Static, bounded allocation in the kernel and drivers.** Fixed pools and
   slabs, no general heap on interrupt paths, and every queue has a hard limit.
10. **Load nothing until it's needed.** The boot image contains the kernel and the
    minimum to mount storage; everything else, drivers included, is loaded on
    first use.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Sizes and budgets are measured on every commit, and each optimisation is locked in by tightening a budget |
| ✨ Modernize | Demand paging, a unified cache and compressed RAM are how modern OSes stretch memory |
| 🗄️ Legacy support | WM5/6.x binaries are demand-paged and share DLLs exactly as CE5 did, so they fit as before |
| ⚡ Better / faster | Less resident code, fewer copies, compressed I/O: more free RAM and faster launches than stock |

## Alternatives considered

- **Load everything at boot (static images):** simplest, but code alone would
  eat a large share of RAM, and every new feature would make it worse.
- **Swap to SD:** too slow and wears the card; compressed RAM plus discarding
  clean pages gives most of the benefit.

## Consequences

The kernel VM (Phase 2) must support page faults on mapped files and page
reclaim from day one: a harder first kernel in exchange for everything
afterwards fitting. Every PR touching an image reports its size impact.
