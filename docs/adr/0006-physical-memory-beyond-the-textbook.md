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
