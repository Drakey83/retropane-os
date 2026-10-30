# Sheet: the flat game frame into SDRAM at 143.00 / 1 (R118, chase), 2026-10-29

**What it decides.** Why R117's flat game frame (frame 0: the warped
flat-coloured mesh straight over the renderer's black background object)
draws wrong into SDRAM at 143.00 / 1 (18 of 20 renders, 57 words, the
lists on die; 20 of 20 in checker order) while the layered and depth
frames (the same flat mesh over a full-screen quad first) and the
textured one are exact. Fixed, the copy path runs at full divide-1 speed;
not fixed, the speed switch (R117's divider step) carries it.

**The owner's stop rule (2026-10-29): at most three device tests; no fix
by then, build the speed-switch plan.** Each test is one run that can
close or narrow the chase; no test repeats a way.

## 1. Original way (Windows Mobile / Dell's product)
Dell's driver never runs the cores at divide 1 past 74.75 MHz, so it never
meets this; at its clocks the place doesn't matter.

## 2. Dell's way (REA, searched 2026-10-29)
What Dell's code offers for this, already read and logged (2700g.md
"Dell's colour clear", the GLES per-frame register list; REA #1-#33):
- **His GLES driver writes render registers ours doesn't:** `0x614` and
  `0x618` = `0x1E3CE508` (ours 0), `0x61C` low bits 7 (ours 4), `0x648` = 1
  (ours 0), `0x6D0` = 1. Their meanings aren't known. G4 (2026-10-23) tried
  his whole table and found no fix, **but G4 ran with the screen outside the
  safe region (G16-G19 found that later), so its "no effect" is
  confounded.** This is a live lead again.
- **His clear is a screen-sized four-corner quad at the clear depth**, not
  our one large triangle at depth 0. G2 saw our background object behind
  divide-1 errors and G3 saw no fix from changing it, both confounded the
  same way.
- **He never polls the core while it renders** (G7/G8; ours reads the
  status ~9 times a frame); G8's "no effect" is confounded the same way.

Nothing new to search: no on-die, write-out or divide-1 rule in his code
(REA #25-#33). REA's help is the three confounded leads above, ranked for
the tests.

## 3. Modern way
A tiler whose tile write-out competes with the next tile's list and
texture reads on one memory bus buffers the write-out or orders the
traffic (today's tile GPUs write tiles from on-chip tile memory with
their own bus priority). Our lever: what each tile asks of memory, and
when. Training knowledge, a claim.

## 4. The alternatives, and why ours wins
| | What | Wins | Loses |
|---|---|---|---|
| A. Speed switch only | Divide 2 for copy-path frames (R117: ~2 ms each way, clean) | Proven, buildable now | ~4 ms on every copy-path frame |
| B. Copy Dell's whole table blind | His register words at once | One change | No reason known; G4 already tried it (confounded); doesn't say which word |
| **C. Ours: three tests, each one deciding** | **Test 1** (this sheet's build): at 143.00 / 1, display fetch off, lists on die, target in SDRAM, the flat frame 20 renders a way, **every wrong pixel placed** (x, y, its row in the 16-row tile, a histogram of rows; GC2P's signature was a tile's last rows): (a) as R117 (the control); (b) a full-screen black quad under the mesh (the layered frame's form with the same picture: is it the tile's light load?); (c) at 139.75 / 1 and (d) at 133.25 / 1 (how far below 143 it's clean: a lower whole-game step instead of a switch?). **Test 2** from test 1's answer, Dell's leads (his render words one group at a time, his quad clear, no polling). **Test 3** the fix itself, the four frames proven at 143.00 / 1 | Finds the cause instead of guessing; (b) is a ready fix if it works (a black underlay costs 0.2-0.7 ms: R117's layered against flat frame, on die and into SDRAM); (c)/(d) put a price on the other way | Up to three short runs |

## 5. What would overturn it
- Test 1's control (a) clean: the fault isn't steady; then the chase
  measures how often and stops at the stop rule.
- (b) clean and (a) wrong: the tile's load is the cause; the fix is in
  the renderer (an underlay or tile order), proven by test 3.
- Wrong at (c) and (d) too: it's not a margin; Dell's leads (test 2).

**Asked of the owner:** given in his message ("chase flat frame but do it
smartly ... stop at 3 tests"); each device run still waits for his
"run it".

## Test 1 (2026-10-29, gpubench-0229): the underlay cures it
143.00 / 1 as R117: 19 of 20 wrong, every wrong pixel in a tile's rows
14-15; **a full-screen black quad under the mesh: 0 of 20** (+0.8 ms in
SDRAM); 139.75 and 133.25 / 1: 2 and 1 single last-row pixels (not a
margin). Test 2 next: Dell's clear form in place of our background
triangle (the quad instead of the extra object: the fix without the 0.8 ms,
if it holds), against the underlay, more renders, all four frames.
