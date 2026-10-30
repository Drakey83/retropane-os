# Overnight report, 2026-10-29 evening to 2026-10-30 morning

For the owner, in plain words first. Every run was on the X51v unless
said otherwise; every result is recorded in `docs/hw/2700g.md` with its
card log. Gate: 30 of 30 (phases 2, 4, 5 lifted). Nothing was pushed to
GitHub `main`; all of it is on `backup/main-2026-10-29` after CI passed.

## The short version

1. **The safe area's edge is exactly the end of the chip's fast memory
   (0xB0000).** Measured three times at the slower clock: right up to it,
   wrong just past it. Your ruling is needed to use the extra 8 KiB
   (question 1 below).
2. **Divide 1's "backup copy" problem: found and cured, then found a
   second effect.** A frame drawn into slow memory at 143 MHz lost a few
   pixels in the bottom rows of tiles when a tile had little to draw.
   Found in your 3-test budget; the cure (a plain full-screen layer under
   the scene, built into the 3D renderer) draws every game frame right
   (400 renders, none wrong), **but only while the screen isn't being
   read**. With the display on, as in real use, drawing into slow memory
   at 143 MHz still goes slightly wrong.
3. **So, per your rule, the speed switch is the plan for those frames,
   and it turned out almost free:** dropping the 3D core to half speed
   for one frame and back takes **1 microsecond** with no pause needed
   (280 renders checked with the display on, none wrong). The 2 ms we
   thought it cost was our own caution. No micro-stutter from the switch
   itself; only that one frame draws at half speed.
4. **R115 measured:** since screen flips wait for the gap between
   refreshes (to stop tearing), touch-to-screen went from about 10 ms to
   about 20 ms (median), worst 49 ms. Still under the 50 ms target, but
   the pen is sampled less evenly while the processor waits. Your call
   (question 2).
5. **For your UI/UX work:** `cargo xtask ui-preview` draws the shell's
   screens on the PC with the device's own code, in seconds, as pictures
   you can flip through. The how-to is `docs/design/ui-workbench.md`.
6. **A tool bug fixed:** the perf ratchet was reading an old log from a
   previous card (log numbers restart when the card is wiped); it now
   picks the newest by when it was saved.

## What ran on the device tonight (in order)

| Run | Card log | What it showed |
|---|---|---|
| R23E (safe area edge) | gpubench-0226 | 0xB0000 clean, 0xB0400 / 0xB2000 / 0xB6000 wrong at 74.75 / 1; nothing visible at 149.50 / 2 |
| R23E run 2 (+143 / 1) | 0227 | Same edge; at 143 / 1 drawing into slow memory was wrong even with the screen off |
| R117 (divide 1 + copy) | 0228 | At 143 / 1 only "everything in fast memory" was clean; lists in fast memory fix 3 of 4 frames; the speed switch clean |
| R118 test 1 (flat frame) | 0229 | Every wrong pixel in a tile's last 2 rows; a black layer underneath cures it |
| R118 test 2 | 0230 | The layer cures all four game frames; Dell's way of clearing doesn't |
| R118 test 3 | 0231 | The cheapest form (background listed twice): nearly, not fully (2 of 100 wrong) |
| R118 proof of the shipped cure | 0232 | The renderer's new underlay: 0 of 100 wrong on each game frame at 143 / 1 (screen not read) |
| R23E run 3 | 0233 | Edge confirmed a third time; at 143 / 1 with the screen on, drawing into slow memory still wrong |
| R117S (switch price) | 0234 | Divider switch with the screen on: 1 us, clean at every pause down to none |
| perf (R115) | probe, gpubench-0235, mbx-blend-0050.., today-0012 | Touch-to-flush +92% (median), see question 2; a few probe numbers +0.1 to 1.2% |

The proof run of the cure was a fourth device run in the chase; it is
the shipped code's device check (the proven-on-hardware rule), not a
fourth search, and the chase's answer came inside the three tests.

## Code that landed

- `Renderer::set_underlay` (`drivers/mbx3d-render`): a full-screen quad of
  the background colour under the scene in every tile; off by default;
  host test plus two mutants; proven on the device. Not switched on
  anywhere yet: reminder R119 turns it on with divide 1 in the graphics
  service.
- `rp-gpubench` runs R23E, R117, R117S, R118, R118B, R118C, each with an
  emulator check.
- `cargo xtask ui-preview`; the perf tool's log pick fixed.
- One emulator gap logged (`docs/hw/emulator.md`): the emulator draws
  nothing for a 3D frame without a background object; the device does.

## Questions for you (nothing needs your eyes on the device)

1. **Raise the safe area to 0xB0000 for divide 2** (what ships)? Three
   runs agree; it gives the 3D lists 104 KiB instead of 96 of the fast
   memory. (R23)
2. **Touch latency after the tear-free flips (R115):** approve the new,
   slower floors, or have me write the sheet for the fix: the flip
   handed to the display's "vertical event" interrupt so the processor
   never waits (and keeps sampling the pen).
3. **Divide 1's copy path with the screen on (R120):** the speed switch
   for those frames (measured nearly free tonight), or more digging into
   why the display's reads disturb drawing into slow memory at 143. My
   pick: the speed switch, plus a long soak of the zero-pause switch
   before it ships.

## Reminders that came due and where they stand

R23 (edge measured; waits on question 1), R115 (measured; question 2),
R117 (closed by R118), R118 (done: cured with the screen off), R119
(new, parked until divide 1 ships), R120 (new, question 3), R116 D
(still waits for real shell scenes). Remote Control couldn't be turned on
for the session: your organization's policy blocks it.
