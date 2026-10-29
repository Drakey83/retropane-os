# Overnight log, 2026-10-28 to 29

Reid's overnight order: 13 items in order, device tests only at Windows
Mobile's clock, 74.75 / 1 and divide 2; no divide-1 work. Each item: a
plain note first, then the detail. The morning report is at the end.

## 1. R89: exact colours for the smooth UYVY route

**In plain English:** the GPU's colour conversion now stores the numbers
the video plane's converter expects, and on the X51v every word of the
test card came out exactly as designed (38,400 of 38,400). With the
converter set to our own coefficients, the colours should come back exact
to the panel's precision, but the screen itself can't be read back: that
is your eye check in the morning. Until then, smooth 2x is **eye-right
only, not exact**.

**Detail:**

- **Design (rule 0):** the textbook (and GC2CS's CPU way) stores BT.601
  YUV, offsets included, and lets the converter's standard matrix invert
  it; our GPU passes only multiply, so they can't add +16 / +128. Ours:
  the offsets come from a flat quad drawn first and the pass added to it
  one-to-one (the blend, measured exact), and each value is halved so it
  never overflows: V = 128 + floor(128 R / 255), U the same of B, Y = 16 +
  floor(128 G / 255) (green alone carries Y: the pixel pair's odd pixel
  then keeps its own green, where a luma Y would mix in its neighbour's red
  and blue). The converter: `CSC02` = 512 / 512 (green from Y, red from
  V), `CSC04` = 512 (blue from U), the rest 0. Every division in Intel's
  formula is then exact whatever its rounding; host-checked: every 5-bit
  red and blue and every 6-bit green comes back exact (8-bit: the last bit
  lost, which the panel doesn't show).
- **GC2CW (`local/rp-gpubench-runGC2CW.txt`, WM clock, 64 ms):** the
  arithmetic measured on a 0-255 ramp, every value. DOT3 at green tint 32
  = `floor(128 v / 255)` exactly (0 of 256 off); at tint 64 = v exactly;
  the plain textured word gives v whatever the tint (it is not a multiply);
  the additive blend over a first quad exact (offset + value, clamped at
  255), alpha byte included.
- **GC2CX (`local/rp-gpubench-runGC2CX.txt`, then `...CX2.txt`, WM
  clock):** GC2CM's conversion with the offsets. First run: Y0 and Y1
  exact in every word, U and V not halved (the plain word ignores the
  tint). Pass A then moved to Dell's DOT3 word without TEX0 bit 13 (GC2CJ's
  per-channel 4x multiply) at red and blue tint 32. Second run: **the
  RGB565 card exact in all 38,400 words, every byte**. The ARGB8888 card:
  Y exact (61 of 38,400 one step low in Y0), U and V low by 1-3 steps: the
  per-channel multiply reads an 8888 texel at 5 bits (14 read as 8). The
  game frames are RGB565, so the route is exact; an 8888 source would need
  DOT3 passes for U and V too.
- **Cost:** 17.46 ms a frame serial at WM clock (passes 4.39-4.46 ms, the
  two 2D placements 2.11 ms): the same as GC2CM's 16.45-17.4 (the offsets
  quad adds nothing measurable).
- **For the morning:** `target/variants/gc2cx-look/rp-gpubench.bin` (from
  this commit): converts the card, then shows **photo 1: the GPU's exact
  UYVY with our coefficients** and **photo 2: GC2CS's CPU-made UYVY of the
  same card with Intel's standard coefficients**, each 2x on the video
  plane until a button. The look part reuses GC2CS's display calls; it
  was not run in the emulator (it waits for buttons).
- **Not done:** the route in the game path's driver (R90's build, after
  the ADR vote).

## 2. R90: the game path ADR, drafted

**In plain English:** a draft decision for your vote, nothing built:
[ADR-0038](../adr/0038-game-render-path.md). All three routes kept; the
driver picks a route each frame from how long recent frames took (drops a
level when slow, climbs back only after a long run of fast frames, so it
never flickers between them), switches only in the screen's blanking, a
per-game override, two frames in flight for the reduced routes, and each
route's own clock step (values held for the divide-1 work). Six questions
(G1-G6), each with a recommendation.

**Detail:** the measured table is GC2CP's test frames, marked as such (not
game performance); the 240 x 320 route's on-screen colour waits on R89's
eye check; G4 (two in flight) waits on item 8's stray-word check.

## 3. R80 at Windows Mobile's clock and 74.75 / 1

**In plain English:** the rare one-pixel errors on the smaller game
frames are real at 74.75 / 1 (Dell's own divide-1 speed) and absent at
Windows Mobile's clock: up to 7% of renders on the flat frames have one
wrong pixel, the textured frame none. And the time a frame spends outside
the GPU's drawing is now all the CPU preparing it (about 1.9 ms); R3's
faster bus timing (7/3/1) doesn't shorten it at all. The small-frame path
isn't shipped on these numbers (and needs this fault answered first).

**Detail** (`local/rp-gpubench-runGC2CY.txt`, run GC2CY, 185 s, stack 60%):

- **Part A: 250 renders of each game frame at 360 x 480 and 240 x 320,
  shown scaled as GC2CA showed them, against WM-clock references.** WM
  clock: **0 of 2,000 wrong**. 74.75 / 1:

  | Frame | 360 x 480 | 240 x 320 |
  |---|---|---|
  | flat | 17 of 250 | 6 of 250 |
  | over a layer | 15 of 250 | 6 of 250 |
  | textured | 0 of 250 | 0 of 250 |
  | depth tested | 7 of 250 | 3 of 250 |

  Every wrong render logged was **one pixel, all eight neighbours right**,
  the wrong value another colour of the scene (e.g. `0x4F4A` for
  `0x679F`), at every x mod 8 (0-7, not only 5 as GC2CA's two cases
  suggested), anywhere in the frame. The renderer now draws quads (the
  default since 55e6e220). Divide-1 analysis held for later (the owner's
  order); the 240 x 320 fault at divide-1 speeds is out of scope.
- **Part B: the time outside the core's render, native, the region array
  written whole each frame (4,800 words):** CPU set-up 1,880-1,900 us
  fastest at both clocks; **the poll sees the end 0-1 us after the
  render** (the learned pace, a256a2a4: the schedule costs nothing now);
  so the time outside the core is the CPU's set-up alone. **CS3 at R3's
  7 / 3 / 1 against WM's 8 / 4 / 2: set-up 1,892 against 1,892 us (WM
  clock), 1,900 against 1,900 (74.75 / 1): no change**; core and whole
  within 1-2% either way (noise). With the region array kept (22 words,
  GC2CP run 4) the set-up was 1,860 us: 4,800 posted writes cost ~30 us,
  so the bus isn't the limit for these writes; R3's timing would matter
  for reads (the 8 MB/s read-back), not this.
- **A finding for item 10:** GC2CA's own emulator check now peaks at 89%
  of its stack (it was under 80% before today's renderer growth); GC2CY
  runs standalone with its renderer in `.bss`.

## 4. R20: the tearing scheduler, built and tested

**In plain English:** the scheduler that decides, each frame, when to
start drawing so the picture never tears is built and works on the X51v:
120 of 120 frames it drew straight into the shown picture were tear-free,
at Windows Mobile's clock and 74.75 / 1, including right after sudden
slow frames. It falls back to the safe two-picture way (A) for the first
frame and right after each spike. 120 clean frames isn't proof of zero
tears (it bounds the rate under about 2.5%). The games path stays held
until you judge GC2CA's picture.

**Detail:**

- **Built:** `rp_mbx3d_render::scan` (pure, host-tested): the start line
  from a band of recent render times, the fastest x 4/5 to never overtake
  the scan, twice the last frame (never under the slowest x 11/10) to
  finish before the scan comes round, a 16-line margin each side, the
  earliest safe line taken; A for no history, a band that doesn't fit,
  frames past ~35 ms, or a non-top-down order (the checkerboard). Seven
  host tests against a tile-row model of the scan (stricter than the
  device by a tile row: it counts a tile row the scan passes through while
  it lands as torn); every plan checked tear-free across its whole band.
- **GC2CZ on the X51v** (`local/rp-gpubench-runGC2CZ2.txt`; GC2BZ's
  watched renders, 40 frames a setting with textured spikes at about 2x):

  | Clock | Order | Behind the scan | Tear-free | By A | Start line |
  |---|---|---|---|---|---|
  | WM 99.67 / 2 | interleaved | 30 | 30 | 10 | 290 |
  | WM 99.67 / 2 | rows | 30 | 30 | 10 | 290 |
  | 74.75 / 1 | interleaved | 30 | 30 | 10 | 412 |
  | 74.75 / 1 | rows | 30 | 30 | 10 | 412 |

  Every picture right. The first run (`...GC2CZ.txt`) doubled the slowest
  recent frame: 20 of 20 tear-free but A for 35 of 40 (a spike kept A for
  eight frames); the band was changed to twice the last frame.
- **Not done:** the scheduler wired into the graphics service's shell path
  (it is the shell's and apps' design, ADR-0034 P1 = C; the service's
  frame loop isn't built yet); the games path held (R20).

## 5. R71: the CS3 bus check, built and run

**In plain English:** the part of the first-docking self-test that
decides each Axim's bus speed is built and works: it writes known
patterns to the graphics chip and reads every word back, over 1.5 million
transfers a check, checks again after five minutes of heavy traffic, and
moves a unit to the faster setting only if every check is clean. On your
X51v it passed everything: this unit would run 7/2/0. Every unit still
starts at 7/3/1, and one that fails stays there. The divide-1 parts of
the self-test are not touched.

**Detail:**

- **Built:** `rp_hal_pxa27x::buscheck` (pure, host-tested: a fault of 1
  in 1,000 transfers caught and named; a check under 100,000 transfers
  never counts as a pass; 7/2/0 only with the default, cold and warm all
  clean). Six patterns (address and its complement, alternating bits,
  walking ones and zeros, hashed noise) over 512 KiB: 1,572,864 transfers
  a check. `rp-cs3bus` gained the self-test mode (`RP_CS3BUS_SELFTEST`;
  the grid's patterns now come from the shared module); `emu-cs3bus`
  checks both modes.
- **On the X51v** (`local/rp-cs3bus-selftest.txt`): MSC1 found as Windows
  Mobile's (`0x24842481`); 7/3/1 (`0x13742481`) 0 misses in 1,572,864;
  7/2/0 (`0x02742481`) cold 0 misses; 300 s of traffic at 7/2/0 (1,674
  rounds of a 1.2 MB flush and a 2D fill, every word checked) 0 misses;
  7/2/0 warm 0 misses; **pick 7/2/0**; MSC1 put back. This is one unit and
  one run: it shows the check works and this unit earns 7/2/0, not that
  every unit will.
- **Not done (held, divide-1):** the self-test's ladder, sentinel scenes
  and corner sweeps (R71's other parts).
