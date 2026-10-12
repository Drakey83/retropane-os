# ADR-0019: Where the kernel keeps per-process records

- **Status:** Proposed (drafted for the owner's vote, 2026-10-12)
- **Date:** 2026-10-12

## Context

Each process has an address-space record: its window and its *regions*
(each a range of its memory with one kind of contents: its program's code,
a stack, a heap reservation, a mapped file). Today a record is a fixed
array with room for 64 regions: about 1.6 KB, whether the process uses 3
regions or 60. The kernel image keeps a table of 5 such records.

Two problems, both found while building processes (overnight item 44):

- **Too small for some programs.** In Windows Mobile every memory
  reservation (`VirtualAlloc`) is a region, as is every thread's stack and
  every mapped file. Programs that reserve memory piece by piece, or run
  many threads, can need well over 64.
- **Too big for most.** A small service uses 3 to 10 regions; 31 processes
  of fixed records would take 50 KB of RAM, mostly empty.

ADR-0005 (memory first) asks for memory that follows real use, with bounds
so no process can exhaust the kernel.

## Options

1. **Regions in the process's own kernel pages** (recommended). A process
   record shrinks to a few dozen bytes (31 of them: about 1 KB, fixed).
   Its regions live in 4 KB pages taken from the frame allocator as it
   needs them: one page holds about 200 regions, encoded as plain words (no
   unsafe code needed to read or write them). A per-process quota (say 8
   pages, 1,600 regions) bounds it. When the process ends, its pages go
   back with everything else.
   - Plus: memory follows use; a small service costs ~1 KB + one page;
     big programs get room; nothing is shared between processes, so one
     can't starve another of regions; the cleanup path already exists.
   - Minus: a lookup touches the process's region pages (a few cache
     lines); a sorted array per process is kept, so inserting in a full
     page moves entries (bounded, rare).
2. **One shared region pool** for all processes, each process holding
   indices into it.
   - Plus: simple, one fixed table.
   - Minus: the pool's size is a guess for the whole system, and a greedy
     process could take everyone's regions unless quotas are added anyway.
3. **A general kernel heap** (allocate records of any size).
   - Plus: flexible; useful later for other kernel objects.
   - Minus: fragmentation over weeks of uptime on 64 MB, harder to bound
     per process, and more code to trust.

## Decision (proposed)

Option 1: small fixed process records; regions in per-process pages from
the frame allocator, on demand, with a quota; encoded as words so the
kernel crates stay free of unsafe code. Reclaim (item 41) and exit cleanup
treat region pages like any other kernel memory of the process.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Host tests for insert/find/carve against today's behaviour (same results), then the kernel image switches over; RAM measured before and after |
| ✨ Modernize | Kernel memory charged to the process that uses it, with a quota: the pattern modern kernels use to stop one process exhausting shared kernel tables |
| 🗄️ Legacy support | Windows Mobile programs that reserve memory in many pieces get the regions they need instead of failing at 64 |
| ⚡ Better / faster | ~50 KB of RAM saved at 31 processes; lookups stay a binary search |

## Consequences

- `AddressSpace` changes inside `kernel/mm`; its API stays (find, map,
  carve, protect), so callers barely change.
- The kernel image's process table drops from 8 KB of initialised data to
  about 1 KB, which gives back most of item 44's budget raise.

## For the vote

Plus: memory follows real use, room for big programs, a per-process quota.
Minus: a little more work in `kernel/mm` now, and region lookups touch one
or two extra cache lines.
