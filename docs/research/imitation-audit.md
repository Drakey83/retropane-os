# Imitation audit (2026-10-13)

The owner's hard rule (docs/CONTRIBUTOR-BRIEF.md, "imitation is the last
resort"), applied retroactively to everything RetroPane has built and
decided so far. For each subsystem:

- **Today:** what RetroPane does now.
- **Limit or choice:** whether the hardware forces it (with the evidence),
  or it is a software choice inherited from Windows Mobile, CE, Dell,
  AxDroid or Linux.
- **Better ways:** what beats it, especially the device's parts working
  together (CPU, the 2700G's MBX core and display controller, DMA, the
  buses, power).
- **Verdict:** ✅ already better than the original · 🔧 missed: queued as an
  item (docs/overnight.md, thirteenth tier) · ⚖️ imitation justified (and
  why).

Facts were gathered from the ADRs, `docs/hw/*`, the drivers and the plans
(file references in the commits of this audit's research). Measured numbers
are the device's unless marked "emulator".

## The headline: three patterns that recur

1. **We start from whatever Windows Mobile left running and never take
   ownership.** The CPU clock (measured at 208/520 MHz once, 624 MHz
   another time: "Windows Mobile's choice at boot, not ours"), the SDRAM
   and static-memory timings (including `MSC1`, the bus to the 2700G),
   the display's PLL and plane, the CF socket timings, the USB
   transceiver, GPIO edge set-ups, the CPLD state. All of these are
   software choices Dell made in 2005 for their OS; none is a hardware
   limit. Several bugs this project already hit came from inherited state
   (the CF slot powering down, DMA and USB left running, the 2700G needing
   a 500 ms "settle" whose cause is unknown).
2. **Our own design rules say "interrupts and DMA everywhere, no polling
   loops" (DESIGN.md §11), and almost everything polls.** USB, SD, I2C,
   I2S refills, CF, DMA completion: all polled; 2 of 32 DMA channels used;
   no memory-to-memory DMA. That costs CPU time and battery and adds
   latency. (The loader is a development tool and polls by design; the
   point is that the OS's drivers mustn't inherit that.)
3. **The 2700G is used as a framebuffer.** The CPU draws every pixel and
   pushes it over a ~41 MB/s bus into the chip's memory. The display
   controller's plane blending is now used (item 60, a first win), but the
   MBX 3D core, which can draw 2D as textured quads inside its own memory,
   isn't, and neither are vsync, double buffering in its 16 MB, hardware
   scrolling, or DMA for the copies.

## The order of the work was imitation too

The ROADMAP runs reverse engineering "in parallel from Phase 0", but the
2700G's MBX core was effectively parked until its drivers were due in
Phase 7, after the UI (Phase 5) and the CE Personality's drawing (Phase
4, "GDI on the software backend"). That order designs the compositor, the
drawing model, the split of memory between system RAM and the 2700G's
16 MB, and what legacy GDI calls become, around the CPU alone, and then
retrofits the GPU. The owner (2026-10-13): understand the CPU and the GPU
fully *first*, then build the OS around how they work best.

**Change:** a "know the silicon" track comes before the UI and the
personality's drawing are designed: the MBX core's command interface (from
Dell's 3D drivers, step by step on the device) and the CPU's deeper
features below, each measured. The compositor/UI ADR and the GDI design
wait for its findings and are written GPU-first, with the CPU as the
fallback, not the other way round.

## The CPU: documented is not the same as used well

Intel's manual says what the PXA270 *can* do; Windows Mobile used little
of it, and RetroPane so far little more. What it offers that nothing uses
yet (each a choice, none a limit):

| Feature | What it gives | Use for it |
|---|---|---|
| **Cache locking** (XScale I- and D-cache lines can be locked) | Hot code and data that never miss | The IRQ path, scheduler, system-call entry, `memcpy`: fixed, minimal latency |
| **Mini-data cache** (2 KB, chosen per page) | Streaming data that doesn't evict the main 32 KB D-cache | Pixel rows, audio buffers, LZ4 streams |
| **256 KB internal SRAM** (WM left it largely unused) | Single-cycle-class memory, measured 5.2x cheaper TLB misses for page tables there | Page tables, interrupt stacks, run queues (ADR-0006, unbuilt), GPU command buffers, hot kernel code |
| **Clock modes** (run/turbo, 2N, "alternate memory clock" bit, 13 MHz mode) | The core at 624 MHz, and possibly the memory bus faster than WM ran it | Item 67/68, measured |
| **Voltage scaling and sleep states** (idle, deep idle, standby, sleep, deep sleep; SDRAM self-refresh) | Battery life | Item 67 |
| **iWMMXt SIMD** | 64-bit SIMD integer maths | `memcpy`/`memset`, pixel blends and dither, LZ4, SHA-256, audio mixing |
| **`PLD` preloads and write-buffer coalescing per region** | Memory latency hidden | Copies, the page cache |
| **CP14 performance monitor** (cycle counter, cache/TLB miss events) | Cheap timestamps (vs 846 ns per OS-timer read) and real profiles | Benchmarks, the trace ring, tuning every item here |
| **FIQ** (already the soft watchdog's) | An interrupt with its own banked registers | Low-latency audio or touch, if measured useful |
| **32 DMA channels with completion interrupts** (2 used) | The CPU sleeps while data moves | Item 73 |

**Verdict:** 🔧 **item 80: a CPU deep-dive**, each feature measured on the
device before and after (cache locking and the mini-data cache first,
with the performance monitor built first to measure everything else).

## Subsystem by subsystem

### CPU clock, voltage, power states

- **Today:** never programmed. CCCR/CLKCFG/PWRMODE only read by the probe;
  only XScale idle (CP14) is used. No frequency scaling, no voltage
  scaling, no sleep or deep sleep.
- **Limit or choice:** choice. The PXA270 has run/turbo modes up to 624
  MHz, frequency changes in microseconds, and (with the board's power IC
  on the power-manager I2C) voltage scaling and deep sleep. WM's boot
  values vary between runs.
- **Better ways:** set 624 MHz turbo ourselves at start; drop to a low run
  frequency when idle and back within microseconds on work (the tickless
  scheduler knows when work arrives); voltage scaling once the power IC is
  identified (RE of `PowerStateManager.dll`/`battdrvr.dll`); deep idle with
  SDRAM self-refresh when nothing is due. Also a suspect for the 12 µs
  thread switch (target 2 µs).
- **Verdict:** 🔧 **item 67, highest value** (speed everywhere and battery).

### SDRAM and static-memory (bus) timings

- **Today:** inherited (`MDCNFG`, `MDREFR`, `MSC0-2` only read).
- **Limit or choice:** choice, within the chips' datasheet limits. `MSC1`
  is the 2700G's bus: framebuffer writes measured at ~41 MB/s.
- **Better ways:** measure and set the fastest stable timings (DESIGN.md
  §11 already asks for this; never done): SDRAM CAS latency and refresh
  for the fitted part, the 2700G's chip-select timings (could raise every
  flush's speed), the CF socket's timings (below).
- **Verdict:** 🔧 **item 68** (bounded: device runs with a fallback; a
  watchdog-guarded test that steps timings and verifies memory).

### Display and drawing (2700G)

- **Today:** `Display::adopt` keeps WM's plane, format, gamma and
  location; everything is drawn by the CPU into RAM and flushed by CPU
  `memcpy` into the 2700G. Plane blending at scan-out proven (item 60).
  The boot console scrolls by copying inside the framebuffer (reading
  2700G memory, against our own write-only rule). No vsync, no double
  buffering, cursor plane always off, iWMMXt unused for pixels.
- **Limit or choice:** choice. WM's driver drew on the CPU (Dell's
  choice). The MBX core can draw; the display controller has planes, a
  cursor plane, start-address registers and a vsync status bit.
- **Better ways (several, independent):**
  1. **The MBX core composes and draws** (ROADMAP Phase 7): clear →
     triangle → textured quad → blend, each measured against the CPU.
  2. **DMA for flushes**: dirty rectangles copied to the 2700G by a DMA
     descriptor chain (one descriptor per row), the CPU free meanwhile.
  3. **Tear-free double buffering inside the 2700G's 16 MB**: draw into
     the back buffer, switch the plane's start address at vsync. WM never
     did.
  4. **Hardware scrolling**: scroll the console or a list by moving the
     plane's start address (`GSADR`), not by copying pixels.
  5. **The cursor plane as a free small overlay** (a spinner, a drag
     handle) that never touches the main plane.
  6. **iWMMXt** for the CPU paths that remain (blend, dither, tint).
  7. **Own the display set-up** (PLL, timings) instead of adopting WM's,
     which also removes the unexplained 500 ms settle.
- **Verdict:** 🔧 **items 69 (MBX bring-up: clear), 70 (DMA flush +
  double buffering at vsync + hardware scroll), 71 (iWMMXt pixel paths),
  72 (own the display set-up; find the settle cause)**. ✅ plane
  composition (item 60) already beats WM.

### Interrupts, DMA and polling (all drivers)

- **Today:** ADR-0007's model (mask and signal, drivers in threads) is
  built, but only the keypad, pen and power-button lines use it; DMA
  completion is polled; 2 DMA channels.
- **Limit or choice:** choice; every one of these blocks raises
  interrupts and the DMA controller has 32 channels with completion
  interrupts.
- **Better ways:** when drivers move into their hosts (item 46's model),
  each uses its interrupt and DMA completion interrupts, so the CPU
  sleeps during transfers; memory-to-memory DMA for page pre-zeroing
  (ADR-0006 rule 3, still unbuilt) and for bulk copies.
- **Verdict:** 🔧 **item 73** (a rule for every OS driver, with an
  emulator check that the CPU idles during a transfer). The development
  loader may keep polling.

### USB device controller

- **Today:** polled, no DMA, no double-buffered endpoints, one packet in
  flight.
- **Limit or choice:** full speed is a hardware limit (USB 1.1 UDC);
  everything else is a choice: the UDC has endpoint DMA and
  double-buffering (`DE`).
- **Better ways:** double-buffered bulk endpoints and DMA, so packets go
  back to back (USB full speed's ~1.1 MB/s instead of today's gaps; the
  CF drive measured 0.6 MB/s end to end).
- **Verdict:** 🔧 **item 74** (with the SD overlap from item 58, the drive
  could reach the USB limit).

### SD

- **Today:** 4-bit bus at the controller's 19.5 MHz maximum, multi-block,
  DMA reads, overlap with USB and read-ahead, ACMD23 pre-erase, recovery
  for weak blocks.
- **Limit or choice:** 19.5 MHz and the byte-wide FIFO are hardware
  limits; polling is a choice (above).
- **Verdict:** ✅ well past WM; remaining interrupt/DMA-writes work is in
  items 58 and 73.
- **Measured 2026-10-15 (docs/research/storage-speed.md, part B):** not
  maxed after all. 19.5 MHz is confirmed the top (the reserved clock code
  is slower), but DMA writes are built and off (+52%: 1.9 -> 2.9 MB/s),
  DMA quietly falls back to PIO above 128 KiB (1 MiB reads 1.9 against
  5.2 MB/s), and single-block writes cost 10.8 ms on the owner's card,
  which is the FAT layer's metadata cost. ACMD23 measured no gain.

### CompactFlash

- **Today:** memory mode, polled per sector, 16-bit, WM's exact socket
  timings, no interrupt.
- **Limit or choice:** WM's timings are a choice (conservative values for
  any card); the PXA's PC Card interface has no DMA request for the
  socket, but the memory window can be read by the DMA controller as
  memory.
- **Better ways:** timings tuned to the card's reported PIO mode
  (IDENTIFY words 64/88) instead of one value for all; READ/WRITE
  MULTIPLE; the card's interrupt instead of polling DRQ; DMA
  memory-to-memory from the socket window.
- **Verdict:** 🔧 **item 75**.
- **Measured 2026-10-15 (docs/research/storage-speed.md, part A):** WM's
  timings decode to a 260 ns common-memory cycle at CLK_MEM 208 MHz
  (calibrated on the device), a generic table (529 ns at WM's 104 MHz
  points); the card rates itself 80 ns (CIS and IDENTIFY word 164): 3x
  headroom per field, ~2x on the bus if tuned, at the price of silent
  corruption if wrong (validation plan in the report). Without touching
  timings: 4.1 MB/s today, 4.9 with 32-bit loads from the 0x400-0x7FF
  data window, 5.2 by DMA from it (both checked byte for byte). READ
  MULTIPLE gives nothing on this card (1 sector); GPIO84 is the card's
  busy line (an interrupt for writes; reads wait only 2%). The drive's
  0.6 MB/s is the USB side, not the socket. **Status: report delivered,
  owner decides.**

### Touch screen

- **Today:** AxDroid's wiring, 200 Hz sampling while the pen is down, no
  filtering, a "typical" calibration range, pressure read but unused.
- **Limit or choice:** choice; the TSC2046 converts far faster and gives
  pressure (Z1/Z2).
- **Better ways:** oversample and median/IIR-filter (steady lines,
  modern-feeling drag), pressure for the pen-down threshold, a real
  calibration screen, sampling by SSP DMA so the CPU sleeps, rate raised
  only while moving.
- **Verdict:** 🔧 **item 76**.

### Backlight

- **Today:** WM's PWM period and prescale kept, with a recorded reason (a
  longer period moves towards audible whine); perceptual levels and fades
  are ours.
- **Limit or choice:** partly justified: the circuit's whine is a real
  constraint. But WM's 216-count period makes the lowest step ~7%, too
  bright at night.
- **Better ways:** temporal dithering of the duty (alternate between two
  counts frame by frame) gives finer low steps without lowering the
  frequency; check whether a *shorter* period (higher frequency) is
  clean.
- **Verdict:** 🔧 small, folded into **item 76's** device session (input
  and light feel), measured on the device.

### Keypad and buttons

- **Today:** WM's button jobs, VK codes and repeat timings.
- **Limit or choice:** the owner's rule (buttons keep WM's jobs) is a
  deliberate user-experience decision, recorded; the timings are a
  choice.
- **Verdict:** ⚖️ jobs justified (the owner's decision: muscle memory);
  timings become user settings when Settings exists (Phase 5).

### Audio

- **Today:** DMA ring for playback, refilled by polling; 48 kHz only.
- **Better ways:** DMA completion interrupt (CPU sleeps), a mixer with
  sample-rate conversion, low-latency mode for games.
- **Verdict:** 🔧 covered by **item 73** (interrupts) and Phase 5/8 audio
  work.

### CPLD

- **Today:** our shadow starts at zero and sets the bits we know.
- **Better ways:** start from Windows Mobile's own shadow copies at hand-
  over (found in item 64: RAM from `0xA8001320`), so nothing WM left on is
  switched off by accident, and map the remaining slots.
- **Verdict:** 🔧 **item 77** (small, removes a class of bugs: the
  backlight blackout of 2026-10-12 was this).

### Boot and hand-over

- **Today:** HaRET from WM's StartUp folder; the loader adopts WM's state.
- **Limit or choice:** starting from WM is a development choice (and the
  owner's "soft reset returns to WM" principle); adopting its *state* is
  not required.
- **Better ways:** at start, take ownership of every block we use (clock,
  memory timings, display, DMA, GPIO edges) into known values, so nothing
  depends on what WM happened to leave. A native launcher replaces HaRET
  later (planned).
- **Verdict:** 🔧 part of **items 67, 68, 72, 77**; HaRET itself ⚖️ for
  now (development path; flash install is Phase 9).

### Memory model (ADR-0003, 0005, 0006, 0008, 0011, 0019)

- **Today:** CE5's 32 MB slots with FCSE, plus per-process protection,
  W^X, demand paging, compressed RAM, region records (all better than
  CE5).
- **Limit or choice:** FCSE is forced in practice (VIVT caches, no ASIDs
  on ARMv5: without it every switch flushes the caches). The CE5 *layout
  numbers* are a compatibility choice: the 31-process limit comes from
  CE5's shared-file area above slot 32, not from FCSE (which allows 64
  PIDs); slot 1 is reserved for "XIP DLL code", which this device can't
  execute in place.
- **Better ways:** keep the layout WM apps see, but check with the API
  scanner whether apps actually use the shared-file area and how; native
  processes may not need the 31 limit; slot 1's space could serve the
  shared page cache. ADR-0006's rules (page tables in SRAM measured 5.2×
  faster TLB misses on the device, DMA pre-zeroing, large pages) are
  decided but mostly unbuilt.
- **Verdict:** ✅ overall; 🔧 **item 78** (build ADR-0006's measured wins;
  re-examine the CE5 layout limits with data).

### Scheduler and system calls (ADR-0010)

- **Today:** O(1) CLZ scheduler, tickless, lazy iWMMXt; CE's priority
  numbering, 100 ms quantum, KData layout.
- **Limit or choice:** KData's layout and the priority numbers are forced
  by WM binaries (measured on 500 ROM modules). The 100 ms quantum is
  CE's default, a choice.
- **Verdict:** ✅ (tickless beats CE's 1 ms tick); ⚖️ KData and numbering
  justified by legacy binaries; the quantum to be measured for
  interactivity when the UI exists (item 67's work measures switch costs
  first).

### IPC, processes, driver hosts (ADR-0016, item 46)

- **Verdict:** ✅ (supervised per-class hosts, grants, restarts that keep
  device state: CE's single `device.exe` had none of it).

### File systems and storage

- **Today:** FAT32/exFAT as Windows writes them; crash-safe ordering,
  power-loss tests, guard, check; packs planned. No design yet for the
  registry, the flash file system or CE's object store.
- **Limit or choice:** the on-card format is an interoperability
  requirement (cards go into PCs and cameras).
- **Better ways:** when they are designed, a transactional registry and a
  flash file system made for this flash (log-structured, wear-levelled,
  crash-safe), presenting CE's semantics to old apps without copying
  IMGFS/TFAT.
- **Verdict:** ⚖️ card format (interop); ✅ crash safety and exFAT beyond
  WM; the registry and flash design must go through this rule when their
  ADRs are written (noted in the ROADMAP).

### UI structure and the CE Personality

- **Verdict:** ⚖️ the WM structure (Today, Start, softkeys) is the owner's
  decision for familiarity, with our own look; ✅ the personality
  reimplements DLLs natively rather than emulating. 🔧 "GDI on the
  software backend" (ROADMAP Phase 4) becomes "GDI on the CPU first, on
  the MBX when item 69's work lands", so legacy apps get the GPU too.

### Development tools that shape the product

- **Verdict:** ✅ (emulator fork that follows the device, measured
  budgets, mutation-checked tests). One addition from this audit: read
  cheap timestamps from the XScale's CP14 cycle counter instead of the OS
  timer (846 ns per read over the bus), for benchmarks and the trace
  ring: part of **item 67**.

## Documents that disagree with the code (to fix)

- DESIGN.md's memory table (kernel 4 MB, ~40 MB apps) vs memory-budget.md
  (OS 12 MiB, apps ≥ 52 MiB).
- DESIGN.md "shared memory for bulk data" vs ADR-0016's page moves.
- DESIGN.md "within one scheduler tick" vs the tickless scheduler.
- ADR-0011's reclaim tiers vs the second-chance clock that was built.
- `kernel/mm/src/layout.rs` "CE5 allows 32 processes" vs `domains.rs` 31.
- `drivers/cpld/src/lib.rs` "whether they read back isn't known yet"
  (they don't, cpld.md).
- `kernel/fat/src/lib.rs` "not here yet: subdirectories, deleting" (both
  exist); ADR-0013 "not yet: creating files".
- docs/ui/README.md's 600 KB RGB565 desktop cache vs the two-plane
  ARGB8888 design (item 60).
- `drivers/pxa-keypad` "polling now, interrupts later" (interrupts exist).
- DESIGN.md "Duktape" vs §10.1 "QuickJS-class".

Queued as **item 79** (one pass; small). **Done 2026-10-13**, moved up the
queue on review (the docs are how the owner sees the project), with two
more found the same day: docs/hw/2700g.md's "WM draws 2D on the CPU"
(measured wrong) and the brief's example built on it.

## Ranked: what to do next

| Rank | Item | Why first |
|---|---|---|
| 0 | The "know the silicon" track: 69 (MBX) and 80 (CPU deep-dive), before the UI and GDI designs | The OS is built around what the chips do best, not retrofitted |
| 1 | 67 Own the clocks and power states | Every other number improves with it; battery |
| 2 | 69 MBX bring-up (clear the screen), then triangle, texture, blend | Opens GPU drawing and composition |
| 3 | 70 DMA flush, double buffering at vsync, hardware scroll | Tear-free, CPU free, no MBX needed |
| 4 | 68 Memory and bus timings | Every memory access and every flush |
| 5 | 73 Interrupts and DMA in every OS driver | CPU sleeps; latency; battery |
| 6 | 74 USB double buffering and DMA | The development loop's speed, and the OS's |
| 7 | 76 Touch quality (and backlight low steps) | How the device *feels* |
| 8 | 71 iWMMXt pixel paths | What stays on the CPU, faster |
| 9 | 72 Own the display set-up | Removes the 500 ms settle and WM's state |
| 10 | 75 CF tuned timings, multiple-sector, interrupts (measured: docs/research/storage-speed.md) | The CF card's speed for the OS (not the USB drive's) |
| 11 | 77 CPLD shadow from WM's | Removes a bug class |
| 12 | 78 ADR-0006's measured wins; CE5 layout limits | Memory speed; process limits |
| 13 | 79 Fix the documents above | Truth in the docs |

Most of 67, 68, 72, 75, 76 and 77 need the owner at the Axim for a device
run each; 69 and 70 begin in the emulator where the model allows (the MBX
core is not modelled yet: its model grows with the reverse engineering).
