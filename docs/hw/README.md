# Hardware specifications

This folder is the **specification** side of the clean room (see
[`../CLEANROOM.md`](../CLEANROOM.md)). Drivers and emulator models are written
from these documents only.

What the emulator reproduces of these facts, and what it can't:
[`emulator.md`](emulator.md).

**Start here: [`datasheet.md`](datasheet.md)**, the whole board on one
page: chips, memory map, the GPIO pinout, the CPLD's bits and each device's
interface, with drawings ([board](img/board.svg),
[GPIO map](img/gpio-map.svg), [CPLD map](img/cpld-map.svg)) and links to
the evidence here.

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
| A peripheral read with its clock off (`CKEN`) | the first `rp-wmaudio` (2026-10-18) read the I2S and I2C registers while Windows Mobile had their clocks off (audio idle): **Windows Mobile froze on every start** | Read `CKEN` first; never touch a clocked unit (I2S bit 8, I2C 14, power I2C 15, ...) whose bit is clear | likely (froze at the first snapshot, before its first box; one cause read in the code) | owner, 2026-10-18 |
| CP15 c1 at handoff | `0x00002A7A` | MMU, D-cache and I-cache off; alignment checking **on**; write buffer, branch prediction and high vectors set (HaRET's state) | confirmed | rp-light on device |
| Panel (CPLD ID) | 3.7" Sharp VGA | `cpld.md` panel ID decode is right | confirmed | rp-light on device |
| Power button | GPIO0, reads 0 at rest; WM arms both edges (`GRER0`/`GFER0` bit 0) | the button's line; pressed level unconfirmed (the loader doesn't depend on it) | likely | probe via the loader, 2026-10-12 |
| FFUART | transmitter drains | clocked and working once enabled (no connector pins verified) | confirmed | rp-light `serial: alive` |
| RAM start | `0xA8000000` | HaRET must be told (`ramaddr`); nothing at `0xA0000000` | confirmed | `memory-map.md` |
| Framebuffer | `0x0C000000`, RGB565 480 × 640 | WM's plane, reused as-is | confirmed | `2700g.md` |
| **MMU/cache enable hazard** | the instructions after the CPWAIT that follows `mcr c1` must sit in the **same 32-byte line** | a probe build whose CPWAIT branch was the last word of a line took an undefined-instruction fault on the first word of the next line (memory verified intact); a build where the sequence stayed inside one line ran. Same hazard as Linux's aligned `__turn_mmu_on`. RetroPane does both transitions in 32-byte-aligned assembly (`rt::init_mmu`, `handoff::launch`) | confirmed | real X51v (2026-10-11), two builds |

## Software reset (2026-10-13, item 64)

> **Source:** (D) the Windows Mobile 6.1 kernel's OAL (`nk.exe`, ROM A08,
> owner's device), its `IOCTL_HAL_REBOOT` handler, analysed locally for
> its hardware accesses and their order (no code from it is here). Not yet
> tried on the device.
>
> **Caution (2026-10-14):** the `nk.exe` this section was read from
> (`local/re/reboot/nk.exe`) is **not byte-identical to the A08 ROM's**
> (code shifted from about `0x8C248000`, data placed differently; it
> matches neither L11 nor L16). Recheck these findings against the copy
> extracted from the ROM (`local/re/oal/nk.exe`, `interrupts.md`) before
> the device test.

Today only the stylus reset (a hardware reset) brings Windows Mobile back;
a watchdog reset stops at Dell's factory prompt, which is in Dell's
bootloader (flash boot block), not in the `.nb0` images. Windows Mobile's
own software reboot uses neither:

| Step | What | Confidence | Evidence |
|---|---|---|---|
| 1 | GPIO99 made a plain output (function 0) | likely | D |
| 2 | `PCFR` bit 4 (GPR_EN, the PXA27x GPIO reset) set; GPIO15 driven low as an output | likely | D; probe: `PCFR` = `0x00005031` under WM, bit 4 already set |
| 3 | CPLD `0x08` bit 1 set (through the shared shadow) | likely | D (`cpld.md`) |
| 4 | **GPIO99 driven high**: the board resets | likely (the reset itself unconfirmed) | D; probe: GPIO99 an input reading low under WM (`GPDR3` bit 3 clear, `GPLR3` bit 3 clear) |

Also in the handler, not reproduced: the display blanked first; two
words in RAM (`0xA8001300` = 0, `0xA8001314` = `0x1A2B3C4D`, the kernel's
own boot notes, also written by its start-up table; after RetroPane runs
that RAM is overwritten anyway, as before a stylus reset); two byte
writes near physical address 0 (flash; purpose unknown). The handler does
nothing while a "reboot disabled" flag in the same RAM block is set.

RetroPane: `cargo xtask reboot-wm` (loader `reboot wm`) does steps 1-4 and
reports `RPL ERR reboot wm: no reset after 1 s` if the board is still
running. **Device test (owner present):** expected Windows Mobile booting
as after the stylus reset, with no factory prompt; if the prompt appears,
the reset type is what Dell's bootloader keys on and item 64 ends as a
dead end.
**Device test passed (2026-10-17, owner present):** `cargo xtask reboot-wm` from the PC restarted the Axim into Windows Mobile without the factory prompt, twice; the loader does not start by itself afterwards (it is launched from Windows Mobile, as after the stylus reset).
