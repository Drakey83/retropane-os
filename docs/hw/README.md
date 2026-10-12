# Hardware specifications

This folder is the **specification** side of the clean room (see
[`../CLEANROOM.md`](../CLEANROOM.md)). Drivers and emulator models are written
from these documents only.

What the emulator reproduces of these facts, and what it can't:
[`emulator.md`](emulator.md).

## Format

One file per hardware block (`cpld.md`, `mbx/registers.md`, `acx100.md`...).
Every fact is a table row:

| Field / register | Offset / bits | Meaning | Confidence | Evidence |
|---|---|---|---|---|
| example | `0x00` [3:0] | ... | confirmed / likely / guess | experiment or trace ID, ROM version |

Rules:

- Facts only: no code, no pseudo-code transcribed from disassembly, no listings.
- Each `confirmed` fact needs a matching spec test in the driver crate.
- Record what we **don't** know too (an "Open questions" section).

## First facts from a real X51v (2026-10-11)

Read by `rp-light` (the first-light image) on the owner's device (WM 6.1 Classic ROM, CE 5.2.20273),
booted through HaRET. Details in the per-block files.

| Fact | Value | Meaning | Confidence | Evidence |
|---|---|---|---|---|
| CPU ID (CP15 c0) | `0x69054117` | Intel XScale, PXA27x core, revision 7 | confirmed | rp-light on device |
| CP15 c1 at handoff | `0x00002A7A` | MMU, D-cache and I-cache off; alignment checking **on**; write buffer, branch prediction and high vectors set (HaRET's state) | confirmed | rp-light on device |
| Panel (CPLD ID) | 3.7" Sharp VGA | `cpld.md` panel ID decode is right | confirmed | rp-light on device |
| FFUART | transmitter drains | clocked and working once enabled (no connector pins verified) | confirmed | rp-light `serial: alive` |
| RAM start | `0xA8000000` | HaRET must be told (`ramaddr`); nothing at `0xA0000000` | confirmed | `memory-map.md` |
| Framebuffer | `0x0C000000`, RGB565 480 × 640 | WM's plane, reused as-is | confirmed | `2700g.md` |
| **MMU/cache enable hazard** | the instructions after the CPWAIT that follows `mcr c1` must sit in the **same 32-byte line** | a probe build whose CPWAIT branch was the last word of a line took an undefined-instruction fault on the first word of the next line (memory verified intact); a build where the sequence stayed inside one line ran. Same hazard as Linux's aligned `__turn_mmu_on`. RetroPane does both transitions in 32-byte-aligned assembly (`rt::init_mmu`, `handoff::launch`) | confirmed | real X51v (2026-10-11), two builds |
