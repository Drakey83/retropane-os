# Sheet: divide 1's copy path with the display on (R121), 2026-10-30

**What it decides.** Whether a frame drawn into SDRAM at 143.00 / 1 can be
made right while the display scans out. If it can, copy-path frames keep
full divide-1 speed instead of R120's switch to divide 2. The owner,
2026-10-30: "take 3 more test to figure it out", then "you have my
permission to do all tests that you want". **Stop rule (written
first):** three device runs. If none finds a cure, R121 closes and
R120's speed switch stands.

## What is known
- **Wrong:** at 143.00 / 1 with the display scanning out screen 0 (on die,
  `0x0`), a game frame drawn into SDRAM (`0x296000`), even with the
  underlay. The flat frame was 18 of 20 wrong (R23E run 3).
- **Right in each of these cases:**
  - the same render with the fetch off (R118C);
  - the same render at 143.00 / 2, the same PLL and memory timing, only
    the cores' divider changed (R117S);
  - a render into screen 0 itself, the memory the display reads (GC2BY,
    R117S).
- **So** the fault needs three things together: the cores at 143 MHz, the
  display fetching, and an SDRAM target. The cores' write rate into SDRAM
  is the suspect, not the memory timing.
- **Already ruled out**, for R69's display-triggered fault at divide 1
  (GC2BC, GC2BS, GC2BT): `DMCTRL`'s fields (each byte halved and doubled,
  Linux's value), the scan line a render starts at, and the CPU kept off
  the bus. Not repeated here.

## 1. Original way (Windows Mobile / Dell)
Dell never runs the cores at divide 1 above 75 MHz (REA #34), so it never
meets this fault.

## 2. Dell's way (REA)
Nothing more. `DMCTRL` is the only display memory control (Linux names
its fields: burst length, FIFO thresholds, refresh window), and R69
measured it.

## 3. Modern way
A tile GPU's write-out is a burst stream that the memory arbiter must
interleave with scan-out. Modern parts pace or prioritise it in
hardware. Ours can only pace it from the work per tile (the textured
frame, with the most work per tile, was never wrong) or lighten the
display's reads.

## 4. The tests
| Test | What | Reads |
|---|---|---|
| **1 (R121A)** | 143.00 / 1, the four game frames into SDRAM, lists on die, the underlay on, 20 renders each a way, each against its own 74.75 / 1 references. Ways: as in use (the control); **a quarter of the display's reads** (a 240 x 320 picture doubled); **checker tile order**; **an alpha-0 quad blended over the frame** (more work per tile, the write-out paced); the fetch off (the control) | Rate-bound if a quarter of the reads cures it; pacing if the overlay does; nothing if all stay wrong |
| 2 | Decided by test 1: the cheapest form of what moved (a smaller overlay, only over light tiles, its cost in ms), or the next lead if nothing moved | |
| 3 | The proof: the cure in the renderer, 100 renders each at 143.00 / 1 with the display on, the cost measured | |

## 5. What would overturn it
- The as-in-use control comes out right: the fault isn't reproduced, so
  the run reads nothing (rerun).
- The fetch-off control comes out wrong: something else changed.
- A cure that costs more per frame than R120's switch saves isn't one:
  the measured times go to the owner.

## Results (2026-10-30 night, gpubench-0237 to -0239; 2700g.md "R121 test 1-3")
- **Test 1:** the shipped layout was wrong once in 80; every other way 0
  of 80. R23E run 3's 18 of 20 had the lists in SDRAM.
- **Test 2:** the shipped layout's flat frame was wrong in 2 of 100 (one
  pixel, a tile's last row). Without the underlay it was 100 of 100;
  with the lists in SDRAM, 78 of 100.
- **Test 3:**
  - Both cures were 0 of 400.
  - The alpha-0 overlay at 143.00 / 1 took 8.70 / 8.73 / 15.30 / 8.73
    ms (flat, layered, textured, depth).
  - R120's switch, at 143.00 / 2, took 9.18 / 11.28 / 15.53 / 11.34 ms.
  - **The overlay is faster on every frame and changes no clock.**
- **Soaks, an hour each:** the switch (R117T, gpubench-0240) 0 of
  16,572 round trips wrong; the overlay (R121S, gpubench-0241) 0 of
  32,592 renders wrong, no underruns.
- The owner picks which ships.

## The owner's pick (2026-10-30 morning): "Invisible layer"
Built (knob off until a divide-1 rung ships, ADR-0035 amendment D):
- `Renderer::set_overlay(on)`: the alpha-0 full-screen quad, drawn over,
  listed last in every tile (after the underlay and the scene), drawn
  with or without a background, its blend counted in the prediction.
  Switching it (and now the underlay) rewrites the lists and redraws
  nothing: the picture is the same, and ADR-0026 rule 1 renders only
  changed tiles.
- `Graphics::sdram_cures(on)`: both cures on for each render into screen
  1 (SDRAM), off for each into screen 0 (on die, where the overlay would
  cost ~3 ms for nothing).
- **Owed:** the device check through the service at 143.00 / 1 with
  fallback frames forced, every frame checked (the proven-on-hardware
  rule); it needs the owner's go.

## The device check through the service (proposed; waits for the owner's go)
- **What R110 did (the finding):** `rp-scene` drives the shipped graphics
  service on the X51v and judges every refresh by the display's own
  signature (`DSIG`). It ran at Windows Mobile's clock and 149.50 / 2.
- **Ours:** an `rp-scene` variant (`RP_SCENE_DIVIDE1`, the default image
  unchanged; the owner's go 2026-10-30, "build the next thing"): R110's P3
  (149.50 / 2, every frame by the copy; P1 and P2 are proven by R110's
  runs, and the emulator showed six phases overflow the run log), the 2D
  flips (P4), then three phases at 143.00 / 1 (P5-P7):
  - checkerboard order, `sdram_cures` on: every frame goes by the copy
    path into SDRAM, the case R121 cured;
  - rows, cures on: frames led behind the scan into screen 0 (cures off)
    mixed with copy-path frames (cures on), so the switching is proven;
  - checkerboard, cures off: the control. Some frames should come out
    wrong, which shows the check can see the fault.
  - Each phase's references are compared with the phase before's (P5's
    with 149.50 / 2's), so a wrong
    reference can't hide a fault.
- **Why it wins over a bench run:** the code that ships is what's tested,
  every refresh judged, on the real display.
- **Length:** about 10 minutes, unattended once loaded.
- **What would overturn it:** any wrongly judged refresh in the first two
  phases. The cure isn't enough through the service: back to the sheet.
