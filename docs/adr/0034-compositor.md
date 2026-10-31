# ADR-0034: The compositor: layers, frame modes and the frame budget

- **Status:** **Accepted (owner, 2026-10-19): C1 A, C2 A (with draw-time
  dithering), C3 A, C4 A, C5 A**; **C4 changed for glass-over-wallpaper pages: L2 (owner, 2026-10-30, below C5)**; with the owner's review fixes (Gap 1,
  the pan-to-partial seam; Gap 2, the price table bounded) written in
  below and in [docs/design/compositor.md](../design/compositor.md).
  **Amends ADR-0023** (C2: 16-bit opaque pictures) **and ADR-0030 G1**
  (C5: layers replace areas). Implements ADR-0022 point 3 as amended.
  **Amendment P (screen placement): accepted (owner, 2026-10-24): P1 A,
  P2 yes, P3 measured per device in the self-test** (below); **P1 changed
  to C by the owner later on 2026-10-24** (below).
- **Date:** 2026-10-19

## In plain words

The screen is a stack of layers; every frame the compositor picks the
cheapest way to show it: nothing (still), a repoint of the display (a
whole page moving), the display blending its two planes (glass over a
moving page), or the 3D core redrawing only the changed tiles (sheets,
fades, clouds, the carousel). A frame budget priced from the X51v's
measured costs keeps every frame on time, and effects that don't fit
degrade in an order chosen in advance. The design, part by part, is
[docs/design/compositor.md](../design/compositor.md); this ADR records the
decisions.

## Decisions (owner, 2026-10-19)

| | Chosen | Why (the owner's review) |
|---|---|---|
| C1. Pictures for layers the 3D core moves | **A: drawn by the CPU in SDRAM, reordered there, copied by DMA** (a strip 3.0 ms, a whole layer ~71 ms once) | The direct write is 14 ms quicker but holds the CPU the whole time; DMA leaves it 79% free, and responsiveness is the priority |
| C2. Depth | **A: 16-bit for opaque pictures (RGB565), 32-bit only for glass with smooth gradients; opaque gradients dithered when the CPU draws them** (once, so free: a weather sky in RGB565 without banding) | Half the memory and uploads; the 3D core's 16-bit target saves 0-3 ms |
| C3. Full-screen 3D effects at today's clock | **A: designed as moving panels or short (~0.3 s) effects at ~20-35 fps**; 60 for everything else | Honest to the measured cost (a full-screen textured layer 25-29 ms); SPB's own effects weren't all 60 fps either |
| C4. The video plane | **A: the panned page, its wallpaper drawn into it**; the graphics plane the glass and chrome | Every pan stays a free repoint at 60 Hz; a wallpaper change redraws the page pictures once |
| C5. Text and glass over 3D content | **A: yes, as layers** (replaces ADR-0030 G1's areas) | Sheets over the carousel and the clock over the weather sky are the SPB look's defining moments; the GPU's single owner and the per-client quotas don't change; translucency is priced per tile, so it's bounded |

**C4 changed for pages with glass over a still wallpaper (owner,
2026-10-30: "Yes, L2"; sheet
[weather-gpu-movement.md](../research/sheets/weather-gpu-movement.md)).**
The planes are swapped for such pages: screen 0 (RGB565, the graphics
plane, the bottom slot) holds the wallpaper and the chrome and stays
still; the page's glass and text are drawn once into a per-pixel-alpha
picture on the **video plane** (the slot above, its window the page's
rectangle), panned and turned by repointing it in the blanking. The
wallpaper no longer scrolls with the page and no page holds its own copy
of it. Rests on device tests M1-M4 of the sheet (the video plane's
per-pixel alpha, a partial window, horizontal steps, hand-offs); if M1
fails, C4 as first voted (the wallpaper in an opaque RGB565 page) is the
fallback. Its depth (ARGB8888 or ARGB4444 + dither, C2) is the owner's
after M1's look. Every rule of amendment P holds: the video plane is off
in every frame the 3D core draws.

## The review's fixes

**Gap 1, the seam between a pan and a partial render.** The 3D core
writes each tile whole from its own inputs and can sample only textures;
it can't draw over a row-order picture (a panned page) as a background.
So:

- **While a page pans, changes over it go through the graphics plane**
  (mode 3, pan under glass): a sheet, a dim, glass. A sheet sliding up or
  down is a repoint of the graphics plane over a taller glass picture
  (free); one appearing or changing is drawn into the glass picture by the
  CPU or the 2D engine. One translucent layer this way; a second drops its
  translucency for the pan (ADR-0022 point 3).
- **The 3D core composes only over content that exists as textures** (the
  carousel's panels, the weather sky, a page prepared as a texture). A
  page that should take a 3D effect after it stops is uploaded as a
  texture first (~71 ms, a static cost: the effect starts after it, or is
  prepared ahead when it can be foreseen), or the effect stays on the
  planes.
- The mode for each frame follows from where the changed layers live:
  plane pictures → modes 2-3; textures → mode 4; never a 3D render over a
  plane picture.

**Gap 2, the price table bounded.** For v1 the table is **fixed** (refit
from `rp-gpubench`'s measurements: quality audit C20) and the measured
render time of every frame is **logged only**. A drift is reported, never
acted on, until there is evidence that the fixed table is wrong; any later
self-correction acts only on sustained deviation, with its range clamped.
One slow frame (a stall from elsewhere) can't inflate the prices.

**C2's dithering, one more case.** Gradients the **3D core** makes on a
16-bit target (a fade, Gouraud colours) may band too, and whether the core
dithers is unmeasured: until measured, 3D-made gradients target 32-bit
pictures (or are drawn by the CPU, dithered, as layers).

**C4's pages:** each tall page holds its own copy of the wallpaper, so
pages are **built on demand and dropped** when they're off screen and not
about to be shown (ADR-0027's pool, the client's quota).

## Measured since (`rp-gpubench` run E, 2026-10-20)

A full-screen layer into a texture now takes **34.8 ms** with the reorder
by table and the CPU writing straight in (was 71 by C1's path), or 41.0 ms
reordered beside the DMA (the CPU free only 11.6 ms of it: the reorder and
the DMA share SDRAM). That narrows C1's trade: the DMA path no longer
leaves the CPU 79% free. **Not re-decided:** C1 stands until the owner
looks at it again with these numbers. 8-bit layers take half the upload
and draw faster than RGB565 (22.7 ms vs 27.2 full screen); PVRTC a quarter
or an eighth; text as one 2D command a glyph moves 11x fewer bytes but
takes as long as pixels. Run F: text drawn by the CPU as 4 bpp strips
and coloured by the 2D engine takes as long in all (23.6 ms a screen
against 22.4 as pixels: the engine's blend runs at ~16 Mpx/s), but the
CPU only 6.2 ms of it (3.6x less) and the bus 4.3x fewer bytes: a choice
of CPU time against nothing, for the renderer to weigh. Run F's second
run: **text on a solid background as plain 2D copies from a pre-coloured
glyph atlas: 12.5 ms a screen, 1.8x faster than pixels**, exact (the CPU
queueing throughout); writing only the ink pixels is slower (25 ms).

## Process

- The pan path is **proven** when its device run passes, and **done** only
  when the emulator models the repoint (ADR-0022 point 3): the fork models
  it from rp-v0.29.0 (`plane-repoint`, `emu-gpubench`'s W4 check),
  released and pinned 2026-10-20: the build order's step 1 is done once
  its device run passes.
- C2 amends ADR-0023 (its screen pictures were 32-bit) and C5 amends
  ADR-0030 G1 (areas): both ADRs carry a pointer here.

## Consequences

- The graphics service (ADR-0030) gains the layer list, the frame loop on
  VEVENT0 and the price table; the 3D renderer (ADR-0026) gains band
  rendering behind the scan line and rectangular textures (measured).
- The 2D renderer (ADR-0023) draws opaque layers in RGB565 with ordered
  dither at draw time, glass in ARGB4444 + dither or ARGB8888.
- Open measurements that feed the design: the GPU bus (`rp-cs3bus`, approved),
  whether the 3D core dithers on 16-bit targets, an RGB565 page panned on
  the video plane under shaped glass (the combination not yet run).

## Build order

As in [compositor.md](../design/compositor.md) "How it would be built":
the frame loop with still and pan (device-proven, emulator-modelled), pan
under glass, the price table and partial mode, the upload path, then the
first showpiece. Each step: a design note, a host test, an emulator
check, a device run on the owner's go.

## Ruled since (the owner, 2026-10-24, on the compositor harness's runs)

From `rp-gpubench` run CH (docs/design/compositor-harness.md; the gate's
T1, T3, T4, ticked):

- **T1: the pipeline.** The CPU prepares the next frame while the 3D core
  draws this one (frames 9-10% shorter; the worst task-switcher frame 29
  to 22 ms). The 2D engine and the 3D core still take turns (ADR-0030 G2:
  P8 open). Measured with the scan-out contention of quality audit C71
  still present, so the render's ~1.5 ms slowdown beside the CPU may
  shrink when that is fixed.
- **T4: the routing table as measured.** The planes for pans, sheets and
  fades (free); the 2D engine for fills, copies, list rows, keys and text;
  the 3D core for layered effects, scaling and turning the screen (26 ms
  against the 2D engine's 42); the CPU draws content once, into caches.
- **T3: the split.** The CPU writes small 2D command batches; DMA moves
  pixels and long batches. DMA for everything rejected (no gain).
- **The video plane's scaler (C70, P9):** Windows Mobile's `SPOCTRL` bit
  27 is kept for scaling **if the owner's look at P9 shows the picture
  right**, dropped if it tears; free scaling is then the app-opening
  animation's way (W3 missed its 250 ms by 105 ms in 3D). **Its filter
  (the owner's rulings):** shrinking takes the designed tables (P9e,
  2026-10-25); an exact 2x enlargement takes R79's set fitted to game
  frames (2026-10-29, from the labelled photos `gpubench-0218`); any
  other enlargement keeps mbxfb's set (`Filter::designed`).

## Amendment P: where the display may read while the 3D core draws (accepted, owner, 2026-10-24)

**The owner's votes (2026-10-24):**

- **P1: C (changed by the owner, 2026-10-24, after G21, GC2I and GC2J):**
  "I'm committed to C now." The 3D core draws straight into the screen
  inside the safe region, which is what lets it run at divide 1 above
  74.75 MHz (G21: A's off-screen target fails there; GC2I: every 16-bit
  kind right to 133.25 / 1 inside the region; GC2J: 108.33 / 1 soaked
  clean, 2.17x Windows Mobile's speed for Gouraud). Still owed before
  anything ships on it, each proven on the X51v: the tearing design (the
  render started as the display begins at the top, so it stays ahead of
  the scan line; to measure first, the order the core writes its tiles
  in), what frames longer than one refresh do (design owed, the owner's to
  rule), every divide-1 step above 74.75 entering ADR-0035 by its own
  rules, and P3's per-device self-test. A stays what it was proven to be:
  tear-free at 74.75 / 1 and divide 2.
- **P1 as first voted: A.** "It's the only option that's proven, doesn't flicker, doesn't
  tear, and doesn't corrupt." B flickers at 60 fps; D corrupts; C is
  unproven and might reintroduce the collision in a new form. A's costs
  (up to a 5 ms copy, no panned page during 3D frames) are affordable, the
  ~2 ms render speedup partly offsetting the copy.
- **P2: yes**, C's test runs anyway: if the 3D core can draw straight into
  the safe region, dropping a 5 ms copy from every 3D frame matters for
  60 fps games. It doesn't block A: A now, C a later optimization if its
  test passes.
- **P3: measured per device in the self-test** (ADR-0035 rule 1). The
  ~700 KiB comes from one X51v (the owner's); the any-Axim rule needs it on
  every unit, and whether the region is on-die is still owed. The
  self-test's measurement answers the size, the on-die question and the
  X50v together. (The owner first said "a fixed constant for now", then
  this: this stands.) **Until a device's self-test has measured it** (a
  first boot, a fingerprint mismatch), the layout is the smallest proven:
  the screen alone at `0x0` (600 KiB, harmless on this X51v up to
  `0xAE000`), the video band off while the 3D core draws; a measured
  region smaller than the screen is reported and the 3D core's renders
  kept off while the display reads (G16's way) until resolved, never run
  blind.
- **The smallest proven end raised to `0xB0000` (owner, 2026-10-30: "Yes,
  raise it").** R23E (gpubench-0226, -0227, -0233; sheet
  [r23-safe-region-edge.md](../research/sheets/r23-safe-region-edge.md))
  measured the edge three times on the X51v: a shown screen ending at
  `0xB0000` drew every render right at 74.75 / 1 and 149.50 / 2, one
  ending at `0xB0400` didn't (74.75 / 1). The edge is the 2700G5's on-die
  memory (R23T: the same `0xB0000` is where render targets turn slower),
  so the region is one fact, not two. `rp_vram_map::SAFE_END` = `0xB0000`
  (704 KiB); the 3D lists on die (R116) grow 96 -> 104 KiB. Ruled for
  divide 2, which ships; at 143.00 / 1 the edge couldn't be read (every
  place wrong with the display fetching, R120), so divide 1 revisits it
  when R120 is answered. P3's per-device measurement still replaces it.
- **Rule 5 under P1 = C ruled: R-B (owner, 2026-10-29, ruling 2 of
  [rulings-2026-10-29.md](../reports/rulings-2026-10-29.md);
  [c71-screen-place.md](../design/c71-screen-place.md)).** Screen 0 at
  `0x0`, inside the region, is the only picture shown while the 3D core
  works; screen 1 stays outside as 2D's back picture (2D-only frames
  flip, rule 4). In 3D frames 2D's frame reaches screen 0 first (a flip,
  or its changed rectangles copied in by the 2D engine), the 3D core
  draws into screen 0, and its area is copied back into screen 1. R-A
  (one screen, +600 KiB) later only if R20's scheduling is proven on 2D
  frames.
- **R20's fallback under R-B (owner, 2026-10-29: "Option 1"):** a 3D
  frame the scheduler can't start behind the scan (no history yet, right
  after a spike, a checkerboard order, or too long for one refresh)
  renders into screen 1 (2D's back picture, already holding the same
  frame) while screen 0 stays on screen, is flipped to in the blanking
  once done (the 3D core idle: rule 4), and its area is copied back into
  screen 0. Screen 1 lies outside the safe region, so this needs its own
  soak first: 3D targets at screen 1's place at 149.50 / 2 with the
  display on screen 0, every render checked (ADR-0035: a target outside
  the region runs 133.25 / 2 until it earns its soak). Until that soak
  passes, such frames start at the blanking (line 640) on screen 0. The
  scheduler plans from the line period measured on the device (R111:
  25,687 ns under our 60 Hz set-up against the 27,460 ns built in).
- **Option 1 replaced by the copy (owner, 2026-10-29, "Yes, copy
  variant", after the REA survey of Dell's flip path):** a frame the
  scheduler can't start behind the scan renders into screen 1 (off
  screen, screen 0 shown: rules 1-2 hold), then the 2D engine copies its
  area (and 2D's changes held in screen 1) into screen 0 behind the scan,
  the copy planned like a render (`Schedule::window` on the copy's own
  measured time: a whole screen from about line 557). No flip and no flip
  back, so the 3D core is free once the copy ends (projected from GC2DE:
  shown as soon as option 1's flip, free ~13 ms sooner, not yet
  measured); the per-frame one-off or back-to-back choice is dropped. With
  no copy time measured yet, or the screen-1 soak not passed, the frame
  starts on screen 0 once the scan has passed the scene's bottom row,
  capped at the refresh's last line.
- **The copy switched on, the blanking start dropped (owner, 2026-10-29,
  after GC2DF on the X51v, gpubench-0220: the copy 0 torn refreshes, the
  blanking start and option 1 48 of 48):** `allow_screen1_3d` on; option 1
  stays off; the blanking start is not kept. Before any copy has been
  timed, a fallback frame is **held** (no tear-free start is claimed)
  rather than started: "correctness, no tear, outranks speed". Timing a
  whole-screen copy at start-up is the direction, as a new fix with its
  own rule 0 and REA sheet first.

### In plain words

The display and the 3D core share the 2700G's memory. On this X51v, while
the 3D core is drawing, **any display plane reading memory past roughly the
first 700 KiB corrupts what the 3D core draws**: badly at the cores'
divide 1, now and then at Windows Mobile's own clock. Reading the first
~700 KiB is harmless. The 2D engine isn't affected. Windows Mobile never
showed it because its screen sits at the start of memory. The proposal:
**our one visible screen moves to the start of memory, the 3D core draws
off screen, and the 2D engine copies its results onto the screen.**

### The evidence (docs/hw/2700g.md, tests G16-G20; quality audit C71)

| Run | Finding |
|---|---|
| G16 | Dell's frame at 74.75 MHz / 1: 0 of 20 right with the display reading, **20 of 20 with its fetch off during each render** |
| G17 | The screen at Windows Mobile's place (`0x0`): **20 of 20** with the display reading normally; at ours (`0x100000`) 0 of 20; Linux's `DMCTRL` no help |
| G18, G19 | Right while the screen ends by **`0xAE000`**, wrong from one ending at `0xB6000`; the wrong words scale with the share past ~`0xB0000` (~704 KiB): likely the 2700G's on-die frame buffer (a hypothesis, not proven) |
| G20 | **The video plane obeys the same rule**: reading outside, Dell's frame wasn't even repeatable at Windows Mobile's clock (89 words full screen, 27 a 96-row band); a band inside harmless. **The 2D engine is unaffected**: a 600 KiB copy exact with the screen outside, ~0.8 ms slower |
| C71 (the other session) | At Windows Mobile's 99.67 / 2, five multi-quad scenes: up to 20 of 20 renders differ with the screen at `0x9D6000`; **0 of 100 with it at `0x0`**, and each render ~2 ms faster |
| GC2S (the other session) | The screen at `0x0`: every kind right at 133.25 and 149.50 / 2 (9 of 9, were 7), 4-5% faster; divide 1 above 74.75 still wrong in 6 of 9 kinds: **a second, separate problem** (the divide-1 hunt continues) |

What the current design does that this rule hits: C4 puts the panned page
(1.2 MiB) on the video plane; the glass layer and chrome on the graphics
plane (600 KiB, or 1.2 MiB at 32 bits); mode 4 renders "into the visible
picture in bands behind the scan line or into a second picture flipped in
the blanking"; `rp-vram-map` keeps two screens at `0x200000` and
`0x296000`, both outside.

### The rules proposed (option A)

1. **The safe region** is local memory `0x0`-`0xAE000` (696 KiB: the
   largest measured harmless; G19; raised to `0xB0000`, the on-die
   memory's end, 2026-10-30: see amendment P). It holds **the visible screen**, one
   480 x 640 RGB565 picture (600 KiB) at `0x0`, and at most ~96 KiB more
   for a small video-plane band (G20's way 3).
2. **While the 3D core draws, the display reads only the safe region**:
   the graphics plane shows the screen at `0x0`; the video plane is off or
   inside the safe region. This holds at every clock (G20, C71).
3. **The 3D core draws off screen**, into targets in ordinary local
   memory, and **the 2D engine copies** the finished tiles or bands onto
   the screen (exact under any display reads, G20; ~5 ms for a whole
   screen, a band or a few tiles in proportion). The copy replaces mode
   4's flip and its bands behind the scan line keep their timing, now as
   copies.
4. **Planes reading outside the safe region** (C4's panned page on the
   video plane, a tall glass picture, a 32-bit screen) **are used only in
   frames where the 3D core is idle** (modes 1-3: still, pan, pan under
   glass). Gap 1 already keeps the 3D core from drawing over a panned
   page; the compositor now also holds any 3D work (an animation
   elsewhere) until the pan ends, or shows that frame's 3D result in the
   safe screen.
5. **`rp-vram-map`** (after the vote): `SCREENS` becomes one screen at
   `0x0`; the second becomes the 3D core's off-screen target in ordinary
   memory; the loader already keeps Windows Mobile's screen at `0x0`. A
   build-time check refuses any plane picture in the safe region's place
   or any region overlapping it but the screen and the band.
6. **The divide-1 problem above 74.75 MHz** is separate (GC2S) and stays
   with the divide-1 hunt; this amendment doesn't depend on it.

### Pros and cons

| | Pros | Cons |
|---|---|---|
| **A, as above (my recommendation)** | Fixes C71 at every clock and Dell's frame at 74.75 / 1 (G17-G19); renders ~2 ms faster (C71); keeps every plane trick (pans, glass) for frames without 3D; the 2D engine is already the screen's writer (ADR-0023), so the copy fits | One visible screen only: no flip between two screens (a copy instead, ~5 ms worst case for a whole screen); 3D frames can't show the video plane's panned page at the same time; the safe region's size is measured on one X51v (the any-Axim rule: verify on another unit, and the X50v) |
| B: the display's fetch paused during each render (G16) | No placement rule at all; any layout | The screen goes blank while the 3D core draws (up to ~20 ms a frame): visible flicker at 60 fps: not usable for the shell |
| C: A, but the 3D core draws straight into the screen at `0x0` (no copy) | Saves the copy (~5 ms full screen) | **Unmeasured**: whether the 3D core's target inside the safe region is itself sound; tearing unless it draws behind the scan line. Needs a test before a vote |
| D: keep the design, avoid the content that shows it | No change | Corrupted renders at divide 2 on ordinary scenes (C71): not acceptable |

### Measurements still owed (before or after the vote, the owner's call)

- **C's test:** the 3D core rendering into a target inside the safe region
  (Dell's frame and C71's scenes), right and how fast.
- **The copy's real cost** on the shell's frames: tiles and bands into the
  screen (the full screen measured: 5.0 ms with the display reading the
  safe region, G20 way 5).
- **The region:** whether it is on-die memory (it behaves so), its exact
  end between `0xAE000` and `0xB6000`, and the same on a second X51v and
  the X50v (the any-Axim rule), with `ODFBPWR`'s modes (Dell sleeps it
  across clock changes).

### Votes needed

- **P1:** option A (my recommendation), B, C (after its test) or D.
- **P2:** if A: run C's test anyway, to see whether the copy can be
  skipped later (yes / no).
- **P3:** whether the safe region's size goes into the per-device
  self-test (ADR-0035) or stays a fixed constant from these runs.

