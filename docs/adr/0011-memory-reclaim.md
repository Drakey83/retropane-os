# ADR-0011: Reclaiming memory without accessed bits

- **Status:** Accepted (2026-10-11, by the project owner)
- **Date:** 2026-10-11

## Context

With 64 MiB and no swap device worth using (ADR-0005), RetroPane must free
memory under pressure: drop clean pages that can be read again, and compress
idle anonymous pages (compressed RAM, live in the VM since ADR-0008's
update). Textbook reclaim (LRU or clock over every page) depends on the
hardware's *accessed* bits, which tell the kernel which pages were used
recently. **ARMv5 has none.** Emulating them means unmapping pages and taking
a soft fault on each touch: a page fault (microseconds) per sampled page,
forever.

What this system knows for free:

- **Which process ran when.** Every process switch is a PID and DACR write
  (ADR-0008, ADR-0010); the scheduler knows exactly when each process last
  ran.
- **A handheld runs one foreground app** and keeps others in the background
  (Windows Mobile's "smart minimize"). Background apps' pages are idle almost
  by definition.
- **Page-cache references.** The page cache counts the mappings of each page
  (`PageCache::get` / `release`): a cached page with no references is mapped
  by nobody and can be dropped at once.
- **WM apps understand `WM_HIBERNATE`**, the message Windows Mobile sends
  under low memory asking apps to free what they can. Legacy apps already
  handle it.

## Decision

> **As built (note added 2026-10-13, item 79):** `kernel/mm` reclaims with
> a **second-chance clock** over each space's pages (`Vm::reclaim`): the
> first visit marks a page "aged" (unmapped, kept resident); a page still
> untouched on the next visit is compressed or dropped. That is the
> per-page sampling of tier 3 applied to every process, and it needs no
> hardware accessed bit. The process tiers below are still the order the
> reclaimer will visit spaces in once there are several apps; the tiers
> themselves aren't built yet.

Reclaim works on **processes**, in tiers, cheapest first, with no
per-page access tracking and no reverse map.

1. **Unmapped cache pages** (reference count 0): dropped immediately. They
   are re-read from the image view or the file on next use.
2. **Background processes, least recently run first:**
   - their mappings of clean file pages are unmapped (walking the process's
     own regions and page tables; references drop, and pages nobody else
     maps fall into tier 1);
   - their anonymous pages are compressed (`Vm::compress`; poorly
     compressing pages are left alone).
   A backgrounded app that comes back faults its pages in as it touches them:
   decompression costs microseconds per page, far cheaper than relaunching.
3. **The foreground process**, only if tiers 1-2 weren't enough: the same, on
   pages chosen by a rate-limited sampling clock (a page is unmapped from the
   foreground app only if it wasn't touched across a sampling interval).
4. **`WM_HIBERNATE`** to background apps, then to the foreground app (CE's
   semantics), before anything is refused.
5. **Watermarks.** Reclaim starts below a *low* free-frame mark and stops at a
   *high* mark, running in a low-priority kernel thread (ADR-0010); an
   allocation that finds no frame triggers it synchronously.

Ages come from the scheduler (last-run time per process), so "least recently
used" costs nothing to maintain.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Metrics per tier (frames freed, faults caused, time to bring an app back); a memory-pressure benchmark with several apps |
| ✨ Modernize | Compressed RAM and tiered reclaim, as modern phone OSes do, designed around the missing accessed bit |
| 🗄️ Legacy support | `WM_HIBERNATE` keeps its CE meaning; apps aren't killed while memory can still be compressed |
| ⚡ Better / faster | No sampling faults in steady state, no reverse map memory, background apps come back by decompression instead of relaunch |

## Alternatives considered

- **Software accessed bits for every page (soft-fault sampling):** a fault per
  sampled page, constantly; kept only for the rare foreground tier.
- **A reverse map (frame → mappings):** memory per page and complexity; process
  walks plus page-cache reference counts cover the same cases here.
- **Kill background apps (Android's low-memory killer):** loses user state;
  compressing first keeps it.

## Consequences

The scheduler must expose per-process last-run times; the page cache must
expose unreferenced pages in an order to drop them (oldest first); the
personality layer must deliver `WM_HIBERNATE`. Reclaim never needs to know who
maps a page beyond what the page cache already counts.
