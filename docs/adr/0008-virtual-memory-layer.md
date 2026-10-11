# ADR-0008: The virtual memory layer

- **Status:** Proposed
- **Date:** 2026-10-11

## Context

ADR-0003 (CE5 slots with FCSE), ADR-0005 (demand paging, shared pages,
unified page cache) and ADR-0006 (chip-specific physical memory) set the
direction. Building the VM layer (`kernel/mm`: `arm`, `layout`, `pagetable`,
`space`) forced a set of further decisions, most of them consequences of
ARMv5 and the XScale's VIVT caches, which a textbook design would get wrong.

## Decision

1. **One set of page tables for the whole system.** With FCSE, every process
   lives in its own 32 MiB slot of one address space, so there's one
   first-level table, and a process switch changes only the PID register.
   Second-level tables (1 KiB) are created on demand, four to a frame, and
   freed when their last mapping goes.
2. **A kernel direct map of all RAM at `0x80000000`** (64 MiB, 1 MiB sections).
   The kernel reaches any frame (page tables, page cache, page copies) without
   temporary mappings.
3. **Below 2 GiB, CE5's published layout** (slots, shared-code slot, shared
   area for memory-mapped files at `0x42000000`), so WM5/6.x binaries find
   what they expect.
4. **Demand paging with no per-frame metadata yet.** Zero pages, page-cache
   pages and copy-on-write private file pages are told apart by the region
   and the mapping's own permissions: in a private file region, a writable
   mapping is the process's copy and a read-only one is the page cache's.
5. **VIVT ordering is part of the API.** Unmapping cleans a page's cache lines
   while it's still mapped, then clears the entry, then invalidates the TLB;
   above 8 pages, the whole cache is cleaned once instead. Kernel writes into
   frames through the direct map must leave no cache lines behind
   (`PhysMem::zero`/`copy` contract), or a user mapping at another address
   would read stale RAM.
6. **Shared writable file mappings only in the shared area**, at the same
   address in every process, so no frame is ever writable at two addresses
   (two diverging dirty copies in a VIVT cache).
7. **W^X, honestly stated.** ARMv5 has no execute-never bit. RetroPane never
   makes code writable, and refuses writable-and-executable regions unless a
   space opts in for a legacy app that generates code. It can't stop a
   process executing its own data.
8. **The kernel writes user memory only after `prepare_user_write`.** ARMv5's
   "user read-only" permission still lets privileged code write; writing a
   copy-on-write page directly would corrupt the shared page-cache copy for
   every process. The syscall layer must go through this call.
9. **Permission changes never change cache policy**, so a device window can't
   become cacheable by accident.
10. **Bounded everything:** 64 regions per address space, 256 frames of
    second-level tables (1024 tables, 1 GiB of mapped space), all bookkeeping
    in `.bss`.

**Live since 2026-10-11** (`boot/kernel`, checked by `cargo xtask emu`): the
kernel builds its own page tables with `PageTables`, switches the MMU to them,
and resolves real page faults in an abort handler (demand-zero, page-cache
file pages, copy-on-write), with FCSE slot switching and a full unmap that
returns every frame. Lessons from making it run:

- **Rule 8 is real:** the first run's copy-on-write test wrote through a
  read-only mapping from kernel mode, the hardware allowed it, and the page
  cache's copy changed. Kernel writes to user memory go through
  `prepare_user_write`.
- **No instruction decoding:** a translation fault is resolved as a read, a
  permission fault as a write (`space::access_for`); a write to a fresh
  private file page costs one extra fault instead.
- **Page-table writes are synced** (`PhysMem::sync`) until hardware shows
  whether the XScale table walker sees the D-cache.
- **FAR:** the XScale reports the FCSE-modified address, QEMU the address
  before relocation; relocating in the handler is right for both.
- **Device windows live in kernel space:** the bring-up identity map's
  CS0-CS5 windows (below `0x42000000`) overlap the process slots and the
  peripheral block runs into the shared area, so the kernel maps every window
  at `0x90000000+` (`rp_hal_pxa27x::devmap`) and switches the HAL to those
  addresses (`devmap::use_kernel_map`); nothing is mapped at the physical
  device addresses any more.

**Per-process protection (live 2026-10-11):** each active process gets its
own ARM domain (`kernel/mm::domains`: 13 process domains, least-recently-run
process parked to a never-enabled domain when they run out, by retagging its
slot's 32 first-level entries). The DACR enables only the kernel's, the shared
area's and the running process's domain, so the kernel image's check shows
another process's page faulting even for kernel reads. `rt::try_read` /
`try_write` turn an unresolvable fault into an error through a fixup table,
which the system-call layer will use for user pointers.

**Compressed RAM in the VM (live 2026-10-11):** `Vm::compress` packs an idle
anonymous page into the compressed page store and puts the store's handle in
the page's own second-level entry: a fault descriptor only needs bits 1:0 =
00, so the other 30 bits are free (`arm::l2_stored`). No swap map or reverse
map exists. The next touch is an ordinary translation fault, and
`handle_fault` decompresses the page back. Unmapping drains stored entries in
bounded batches (`PageTables::take_stored`). Lesson: nothing page-sized goes
on a kernel stack; the first version overflowed the 4 KiB abort stack into
the frame allocator. Buffers are static (`ZBuffers`) and pages are touched in
place (`PhysMem::with_page`).

Still to come, in order: reclaim of clean pages (ARMv5 has no accessed bit, so
reference tracking needs a software scheme) and ADR-0006's rules one at a
time.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | 49 host tests, including exact cache/TLB operation order; fault and unmap costs become device benchmarks |
| ✨ Modernize | Demand paging, copy-on-write, shared page cache, W^X where the hardware allows |
| 🗄️ Legacy support | CE5's address layout; legacy code-generating apps get an explicit opt-in instead of breaking |
| ⚡ Better / faster | No cache flush or table switch on process switch; one copy of every shared page; four tables per frame |

## Alternatives considered

- **Per-process page tables:** what most kernels do, but on ARMv5 it means a
  full cache flush on every switch (ADR-0003).
- **Per-frame metadata now (a `struct page`):** needed eventually for reclaim
  and reverse mapping; deferred until reclaim needs it, so its layout is
  designed against a real requirement.
- **Temporary kernel mappings instead of a direct map:** saves 64 MiB of
  kernel address space, which isn't scarce here, at the cost of TLB churn
  and VIVT aliasing on every access.

## Consequences

The HAL must implement `PhysMem` (direct-map access plus cache cleaning) and
`Mmu` (per-page and whole-cache maintenance) for the XScale. The syscall layer
inherits rule 8.
