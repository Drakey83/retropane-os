# Hardware specifications

This folder is the **specification** side of the clean room (see
[`../CLEANROOM.md`](../CLEANROOM.md)). Drivers and emulator models are written
from these documents only.

What the emulator reproduces of these facts, and what it can't:
[`emulator.md`](emulator.md).

## Not published anywhere else

Facts about the Dell Axim X50/X50v/X51/X51v that, as far as we know, no
public source had before RetroPane found them on the owner's X51v. They
should hold for the whole X50/X51 family (same board design), but were
measured on one X51v. Each links to its spec, with sources and confidence.

| Finding | Where |
|---|---|
| **The CompactFlash slot's power switch and reset**: CPLD `0x16` bit 1 + `0x10` bit 12 power it (in that order, 10 ms apart), CPLD `0x18` bit 1 is the card's RESET, GPIO84 reads high while the slot is powered, CPLD `0x04` bit 1 clear = card present. With these the slot works outside Windows Mobile (no earlier port ever got CF working) | [`cf.md`](cf.md) "Slot power and reset" |
| **GPIO36 is not the CF reset** (AxDroid's board file says it is) | [`cf.md`](cf.md) |
| **The CPLD doesn't read back**, and Windows Mobile's drivers share one shadow copy per register; writing a register blind switches other devices off (we turned the backlight off this way) | [`cpld.md`](cpld.md) |
| **A map of the CPLD's bits** by which Windows Mobile driver uses them: backlight enable (`0x18` bit 2), SD, CF, Wi-Fi, Bluetooth/serial, audio, display, battery, USB, notification bits | [`cpld.md`](cpld.md) |
| **The Wi-Fi power register is `0x02`, not `0x00`** as in AxDroid (same bit, same order) | [`cpld.md`](cpld.md) |
| Windows Mobile leaves the graphics plane at offset 0 of the 2700G's memory, not 0x60000 | [`2700g.md`](2700g.md) |
| 2700G plane composition (video plane, ARGB4444 overlay, blend modes) confirmed on the device | [`2700g.md`](2700g.md) |
| The MMU/cache enable hazard on the X51v's XScale (CPWAIT and what follows must share a cache line) | "First facts" below |
| RAM starts at `0xA8000000` (HaRET needs `ramaddr`) | [`memory-map.md`](memory-map.md) |
| SD card detect GPIO12 (low = in) and write-protect GPIO27 (high = locked); an SD multi-block write needs the stop-transmission bit for its CMD12 | [`sd.md`](sd.md) |
| USB client quirks of the PXA27x UDC as wired on the X51v (fixed pull-up: the PC never sees a disconnect) | [`udc.md`](udc.md) |

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
| Power button | GPIO0, reads 0 at rest; WM arms both edges (`GRER0`/`GFER0` bit 0) | the button's line; pressed level unconfirmed (the loader doesn't depend on it) | likely | probe via the loader, 2026-10-12 |
| FFUART | transmitter drains | clocked and working once enabled (no connector pins verified) | confirmed | rp-light `serial: alive` |
| RAM start | `0xA8000000` | HaRET must be told (`ramaddr`); nothing at `0xA0000000` | confirmed | `memory-map.md` |
| Framebuffer | `0x0C000000`, RGB565 480 × 640 | WM's plane, reused as-is | confirmed | `2700g.md` |
| **MMU/cache enable hazard** | the instructions after the CPWAIT that follows `mcr c1` must sit in the **same 32-byte line** | a probe build whose CPWAIT branch was the last word of a line took an undefined-instruction fault on the first word of the next line (memory verified intact); a build where the sequence stayed inside one line ran. Same hazard as Linux's aligned `__turn_mmu_on`. RetroPane does both transitions in 32-byte-aligned assembly (`rt::init_mmu`, `handoff::launch`) | confirmed | real X51v (2026-10-11), two builds |
