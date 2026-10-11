# ADR-0003: CE5-style 32 MB slot memory model with FCSE

- **Status:** Accepted
- **Date:** 2026-10-10

## Context

The PXA270's caches are virtually indexed and virtually tagged (VIVT). With a
conventional per-process address space, every context switch must flush the
caches, a large performance cost. ARMv5 has no ASIDs (those arrived in ARMv6).
WM5/WM6.x apps also expect Windows CE 5's memory layout.

## Decision

Use one 4 GB address space divided into 32 MB process slots, as Windows CE 5
did. The XScale **Fast Context Switch Extension (FCSE)** relocates the running
process into slot 0, so context switches change a PID register instead of
flushing caches. Improvements over CE5 where compatibility allows: per-process
page protection (CE5 let processes read each other's slots; a per-app shim
restores that for legacy apps that rely on it), W^X code pages, and a modern
slab plus per-process heap allocator.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Context-switch cost is a tracked benchmark from Phase 2 |
| ✨ Modernize | Memory protection and W^X that CE5 lacked |
| 🗄️ Legacy support | Matches the layout WM5/WM6.x binaries assume (slots, shared DLL region) |
| ⚡ Better / faster | No cache flush on context switch: the most efficient option on ARMv5 VIVT |

## Alternatives considered

A Linux-style model with cache flushing: simpler, but much slower and less compatible.

## Consequences

32 MB per process and a limited number of slots. Neither matters with 64 MB of RAM.
