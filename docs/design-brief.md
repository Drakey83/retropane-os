# Design brief: fast and modern from the start

Adopted 2026-10-12 from a brief the owner drafted with another assistant,
reviewed against the project's measurements and decisions. Every new
subsystem, driver or service is checked against it; the commit or ADR says
which points shaped it. It adds to the architecture (layers, the unsafe
policy, test-before-trust, budgets); it never overrides it.

**The governing idea:** perceived performance beats measured throughput.
Touch answered within 50 ms, frames never dropped once promised, instant
wake, smooth motion. When in doubt, predictability over peak speed.

## Corrections to the original brief

- **SD speed.** Not "~133 KB/s, 1-bit polled": 4-bit multiple-block reads
  are done and measured on the owner's X51v at about 1.16 ms per 512-byte
  block (~440 KB/s, 3.4x the single-block path); SD DMA is done in the
  emulator, device run pending (docs/hw/sd.md, docs/perf.md).
- **Flash.** Not "32 MB StrataFlash": the X51v's flash is a DiskOnChip G3,
  about 256 MB, which can't run code in place (DESIGN.md, ADR-0005). To
  confirm on the device before any layout depends on the exact size.
- **Budget raises.** Kernel budgets follow the owner's standing rule (a
  planned subsystem raises its budget to measured + ~10%, logged; over
  8 KB in one step needs the owner); other OS budgets need approval.
- **What lives on the card.** Not "fonts, themes and atlases on SD, the
  fancy look vanishing without a card": ADR-0018 (accepted) keeps the base
  system complete in flash, including a good default font and theme; only
  big extras are packs. An OS that looks broken without a card would feel
  less modern, not more.
- **Per-thread allocation caches** in front of the frame allocator: not
  adopted until a measurement shows frame allocation on a hot path (single
  core; complexity without a measured need).

## The points, and where each is tracked

| Point | Status |
|---|---|
| Input path pinned: pages that touch, keys and the compositor use are never compressed or evicted | Done 2026-10-12: `Prot::pinned()`, `Vm::populate`, per-process pin limit (item 49) |
| The framebuffer is write-only: compose in RAM, write only dirty rectangles, never read back | Rule in docs/hw/2700g.md (measured 61 vs 14.5 ms); item 52 |
| iWMMXt fast paths for fills, blits, alpha and conversion, in one place | Item 52 (lazy iWMMXt switching already live) |
| Real-time priority for input and the compositor | Scheduler has CE's 256 priorities and calls run at the caller's priority (ADR-0016); assigned when those services land |
| Optimistic UI (feedback first), touch prediction, vsync flips, adaptive quality (shed effects, never frames) | Window manager phase (ROADMAP Phase 5), in its design spec |
| Pre-rendered chrome and glyph atlases, fixed-point animation, dark theme as a first-class theme | Phase 5 design spec |
| Dynamic clocking (624 MHz when interacting), instant suspend/resume, batched radio wakeups, idle by default | ROADMAP Phases 2 and 9; the loader's 98% idle is the pattern |
| Boot time budgeted and measured; drivers start on first use | Item 50 |
| Always-on trace ring (frame times, input latency, faults, DMA, clocks) readable over USB | Item 51 |
| Packs stored LZ4-compressed, decompressed as pages load | ADR-0018 (added) |
| 2700G 2D blitter before 3D | ROADMAP Phase 7 order |
| Tests grow with the code: host, simulated hardware, emulator, device; mutation-test the memory manager, scheduler and file systems; fuzz parsers when they land; golden output; benchmarks as tests; `device-check` as hardware CI | Already the practice; the mutation rule is written into docs/testing (item 53) |
| Architecture: lint green, ADRs keep alternatives, unsafe quarantined, budgets as design signals, crate seams allowed to move, periodic structure review | Already the practice |

## What it doesn't ask for

Peak benchmark numbers at the cost of predictability; breaking layering,
the unsafe policy or test-before-trust for speed; gold-plating.
