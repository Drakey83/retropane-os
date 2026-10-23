# ADR-0006: Physical memory: designed for this chip, not the textbook

- **Status:** Accepted (2026-10-11, by the project owner)
- **Date:** 2026-10-11
- **Last verified:** 2026-10-13 (against commit 3158b26)

## Context

The kernel now has a physical frame allocator (`kernel/mm`, a binary buddy
allocator; worst-case alloc + free 567 ns, typical alloc or free ~80 ns in the
emulator). That is the textbook design, carefully done: an imitation. The
project's rule is to *iterate, not imitate*, which means using the X51v's
specific constraints to design something better than a generic kernel would.

The constraints that matter here, from the PXA27x and XScale manuals:

- **Tiny TLBs:** 32 instruction and 32 data entries. With 4 KiB pages, the data
  TLB covers just 128 KiB, so most memory accesses in a large working set (a
  browser, a game) risk a TLB miss and a page-table walk to SDRAM.
- **VIVT caches:** 32 KiB D-cache. Anything the CPU streams through the cache
  evicts useful data. Zeroing one 4 KiB page with `memset` evicts an eighth of
  the D-cache.
- **256 KiB of on-chip SRAM** at 0x5C000000 (docs/hw/memory-map.md): far lower
  latency than SDRAM, and largely unused by Windows Mobile.
- **A general-purpose DMA engine** that can do memory-to-memory transfers
  while the CPU sleeps.
- **Three kinds of memory:** SDRAM (64 MiB), on-chip SRAM (256 KiB) and the
  2700G's local memory (16 MiB).

## Decision

Five changes to how physical memory is used, each to be measured before and
after on real hardware.

1. **Page tables live in on-chip SRAM.** The FCSE single address space
   (ADR-0003) needs only one 16 KiB L1 table. It goes in SRAM, along with the
   hottest L2 tables, so TLB refills read on-chip memory instead of SDRAM. The
   exception stacks and the scheduler's run queues go there as well.
   *Status (2026-10-15):* the kernel's first-level table is in SRAM: built
   in SDRAM, moved after the switch to the kernel map
   (`rp_hal_pxa27x::diag::move_l1_table_at`; the SRAM is reached through
   its window only under that map), the SDRAM copy freed; `emu-kernel`
   passes every VM, IPC and driver-host check with it there. Device
   measurement (the kernel on the X51v) and the L2 tables, stacks and run
   queues are still to do.
2. **Large mappings wherever possible, to make the 32-entry TLBs go further.**
   The kernel is mapped with 1 MiB sections. Code and read-only data of
   executables and shared DLLs are mapped with 64 KiB large pages whenever the
   allocator can supply an aligned 64 KiB block (order 4), which a buddy
   allocator produces naturally. One TLB entry then covers 16 times as much.
   Demand paging (ADR-0005) still works at 4 KiB granularity by faulting in a
   whole 64 KiB block when a page in it is first touched and the file data is
   contiguous.
3. **Pre-zeroed pages, zeroed by DMA while idle.** The allocator keeps a pool
   of frames already known to be zero. When the CPU goes idle, the DMA engine
   zeroes free frames from a zero buffer in SRAM. A zero-fill page fault then
   costs no CPU time and evicts nothing from the D-cache. When the pool is
   empty, the CPU falls back to `memset`.
4. **One allocator type, three pools.** SDRAM, SRAM and 2700G memory are
   separate `Frames` instances with their own policies: SRAM is reserved for
   the uses above, and 2700G memory holds surfaces, textures and glyph caches
   (ADR-0005 rule 5).
5. **Fragmentation is resisted, not just measured.** Movable and unmovable
   allocations come from different ends of SDRAM (page-cache and user pages
   from the top, kernel objects from the bottom), so long-lived kernel data
   doesn't split the 64 KiB blocks that rule 2 depends on. The share of free
   memory held in order-4 and larger blocks becomes a tracked metric.

## Amendment: the SRAM layout (owner, 2026-10-22)

The performance gate's T2 (run E's E1e, X51v: a 1 MiB texture staged in
SRAM through two 32 KiB buffers, overlapped with DMA, 32.2 ms against 34.8
written straight in) asked who owns the 256 KiB. **The owner's decision: a
shared layout, not a single owner.** Page tables, stacks and run queues keep
their reservation (rules 1-4 above, with their measured wins); upload
staging borrows two 32 KiB buffers only while a transfer is in flight;
the kept log (`rp_hal_x51v::kept`, 16 KiB at +64 KiB and +192 KiB) stays.
There is no real conflict: the L1 table is 16 KiB, staging 64 KiB and the
kept log 32 KiB, leaving about 144 KiB for stacks, run queues and the zero
buffer (the owner counted 150+ KiB without the kept log). The addresses are
fixed when the upload path is built, in `docs/hw/memory-map.md` with the
rest.

## Amendment: page sizes, page tables and lock-down (owner, 2026-10-22)

The gate's C4 (CPU), measured on the X51v by `rp-probe`'s pages runs 1-3
(docs/hw/cpu.md "Page sizes and page-table placement"): a D-TLB miss on 4
KiB pages costs ~550 ns with both table levels in SDRAM, ~418 with the
second level in the SRAM, ~329 with the first, **~24 with both in the
SRAM**; even a new page every hop is then 5% over 1 MiB sections (2.26x
with the tables in SDRAM). Walks don't appear to use the D-cache (~544 ns
with a table almost certainly cached), which answers this ADR's open
question. 64 KiB pages miss 16 times less, at the same cost a miss.
D-TLB lock-down works (`c10, c8, 0`, verified: locked pages stop
missing).

**Decided (the owner, 2026-10-22: "I accept what you think is best", on
the recommendation):**

1. **4 KiB pages** are the kernel's normal map.
2. **The first-level table and the busiest second-level tables live in
   the SRAM, within a fixed 32 KiB** (32 MiB of mapped memory); the rest
   stay in SDRAM, and the kernel moves second-level tables in and out of
   the SRAM as their use changes. Rule 1 above, sized by measurement; it
   fits the SRAM layout amendment (about 144 KiB beyond the first-level
   table, staging and the kept log).
3. **Three hot kernel pages are locked in the D-TLB:** the exception
   vectors, KData and the running kernel stack (29 entries left for the
   rest).
4. **64 KiB pages (rule 2) become a lower-priority optimization** for
   large read-only code, not part of the base design: with the tables in
   the SRAM a miss is nearly free.

The CPU gate's V7 stays open for its other half: the same hop timed under
Windows Mobile, a sourced number instead of the unsourced "~280 ns".

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Each rule is a before/after benchmark on device: TLB-miss-heavy workloads (a large `memcpy` walk, Doom), page-fault cost, D-cache misses after a fault, and the fragmentation metric |
| ✨ Modernize | Large pages, pre-zeroed pools and anti-fragmentation grouping are what modern kernels do, adapted to ARMv5 |
| 🗄️ Legacy support | Invisible to WM5/6.x apps: same address space layout and page semantics (ADR-0003) |
| ⚡ Better / faster | Fewer TLB misses, cheaper misses (SRAM walks), zero-fill faults that don't pollute the cache: things stock Windows Mobile doesn't do on this chip |

## Alternatives considered

- **Textbook only (status quo):** simplest, but leaves the chip's best features
  unused.
- **Bitmap allocator with `CLZ` scanning:** much smaller metadata (about 12 KiB
  instead of 80 KiB) and lowest-address-first allocation. Worth benchmarking
  later; the saving is about 0.1% of RAM, so it isn't the priority.

## Rollout (review conditions)

- **One rule at a time.** Each rule is built, measured before and after on
  the device, and kept or dropped before work on the next one starts. All five
  at once would be too much machinery to debug on a device with no JTAG.
- **Rule 2 is measured with a sparse-access workload too**, not only Doom.
  Faulting in a whole 64 KiB block reads up to 16 times the data an app
  touching one page in a block actually needs. The benchmark set includes a
  program that touches one page per 64 KiB of a large mapped file, and the
  rule only stays if it wins or breaks even there. If it doesn't, the fallback
  is to promote a block to a large page only after several of its pages have
  been touched.

## Consequences

The VM must handle mixed page sizes in one address space, and the idle loop
gains a DMA zeroing task, which must stop immediately when an interrupt
arrives. Measuring TLB effects needs real hardware, since QEMU doesn't model
latency; SD boot via HaRET is the prerequisite.

## Open questions (measure on device)

- Can XScale page-table walks use the D-cache? If they can, the SRAM gain for
  page tables is smaller, and SRAM is better spent on stacks and DMA buffers.
- Real SDRAM versus SRAM latency on the X51v at 624 MHz.
