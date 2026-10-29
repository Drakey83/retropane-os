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
