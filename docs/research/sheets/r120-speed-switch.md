# Sheet: the speed switch for divide-1 copy-path frames (R120), 2026-10-30

**What it decides.** How divide 1 (143.00 / 1, the fastest step game
frames draw right at, GC2BY) keeps the graphics service's copy path. The
copy path is R20's fallback: a frame drawn into screen 1 in SDRAM, then
copied (ADR-0034). R23E run 3 (gpubench-0233) found that at 143.00 / 1,
with the display scanning out, a frame drawn into SDRAM comes out wrong
even with R118's underlay. Frames drawn into screen 0 (on die) are right.
R117S (gpubench-0234) measured the divider-only switch 1 -> 2 -> 1 with
the display on: 1 us a switch, clean at every wait down to none, 280
renders right. The owner, 2026-10-30, on "the speed switch (my pick),
with a long test run of the no-pause switch before it ships": "keep going
with this".

## 1. Original way (Windows Mobile / Dell's product)
Dell runs the cores at divide 1 only up to 75 MHz. Its rule is divide 1
up to 75 MHz, else the smallest of 2-4 that keeps the cores at or under
75 MHz (`0x02120370`, 2700g.md). So it never meets this fault.

## 2. Dell's way (REA)
REA #34 (logged): Dell writes `MBXCLK` only at init
(`0x02120730`-`0x021207FC`) and in its clock change (`0x02120370`). The
clock change first writes 0 (both clocks off), then the divider and the
enables. **There is no per-frame divider switch**, and Dell has no
copy-path problem to solve. This is a finding, not an answer.

## 3. Modern way
Today's phone GPUs change their clock per workload (DVFS: Linux devfreq
governors, a frame's demand picking the operating point), and a
glitch-free divider is how they do it cheaply. This is training
knowledge, a claim. Ours uses the same idea per render target, not per
load: on die gets full speed, SDRAM gets half speed, for one frame.

## 4. The alternatives, and why ours wins
| | What | Wins | Loses |
|---|---|---|---|
| **A. Ours: the speed switch** | The cores drop to divide 2 for a frame drawn into screen 1 (SDRAM) and return to divide 1 when it's done. The switch is the divider alone, no PLL, no wait (R117S) | Every frame behind the scan at full divide-1 speed. Copy-path frames right, at 71.5 MHz cores (4% under today's shipped 74.75). 1 us a switch. The underlay (R119) is no longer needed: SDRAM renders happen at divide 2, where none were wrong | Copy-path frames draw at half speed. These are the rare ones (no history yet, a spike, a frame too long for one refresh), but the last kind is heavy, so its render about doubles. The soak and ADR-0035 rule 8's amendment come first |
| B. Keep digging (why the display's reads disturb SDRAM writes at 143 / 1) | The display's fetch timing, priority, bursts | Full speed for copy-path frames too, if a cure exists | Unknown runs and odds. The owner's 3-test rule already fell back to A |
| C. No copy path at divide 1 | Frames that can't lead the scan wait a refresh | Nothing to build | A visible stutter on exactly the heavy frames |
| D. Copy-path target on die | Render the fallback on die instead of SDRAM | Full speed | No room: screen 0 (600 KiB) and the lists (104 KiB) fill the 704 KiB. Only small scene areas could fit, and only by moving the lists |

## 5. The plan (A), in order
1. **The soak (`rp-gpubench` run R117T, a test image).** At 143.00 / 1
   with the display scanning out screen 0 (as in use), the zero-wait
   divider switch:
   - Each round trip: down to divide 2, one game frame drawn into SDRAM
     and checked; up to divide 1, one drawn into screen 0 and checked.
   - The checks compare against 74.75 / 1's references, hash first, word
     for word only on a mismatch (the fast-tests rule).
   - The four game frames take turns. The underrun flag is checked every
     100 round trips, and every lost render is counted.
   - A short day run of **10 minutes** (estimated 5,000-10,000 round
     trips; the count is logged), owner watching. Then **an hour
     overnight** (the day/night rule).
   - Each run has an emulator check.
2. **The driver:** `clock::set_divider(bus, div)`: the divider alone (the
   enable kept, no wait), read back once. A wrong read is a failure, as in
   ADR-0035 rule 4.
3. **The graphics service:** `Graphics::copy_divider(Option<u32>)`, off
   until divide 1 ships (ADR-0035 holds it).
   - When it's on, `aside()` (the render into screen 1) writes divide 2
     before starting the render, and `poll()` writes divide 1 back once
     that render is done, before the next render into screen 0. The
     divider never changes during a render.
   - The scheduler learns the fallback's render time at divide 2 apart
     from the behind-the-scan time.
   - The underrun flag is checked after each switch (ADR-0035 rule 6).
   - Host tests use a mock bus: the order of the writes, no write while
     a render runs, the two times learned apart, off by default. The
     tests also come with mutants.
4. **The device check:** R110's service run at 143.00 / 1 with fallback
   frames forced, every frame checked.
5. **ADR-0035 amendment S (proposed; the owner's vote):** rule 8 keeps
   PLL changes where the display can take them. The cores' divider alone
   (no PLL, no `SYSCLKSRC`) may change per frame, because R117S and the
   soak measured it clean with the display on. Each switch is followed by
   the underrun check. The self-test (rule 1) runs the round trip on each
   device as part of each divide-1 step (the any-Axim rule).

Steps 1-4 change no shipped behaviour (the knob is off). Step 5 is the
owner's vote before the knob turns on.

## 6. What would overturn it
- **Any wrong render, lost render or underrun in the soak:** put back
  R117S's smallest clean wait above zero (3 us) and soak again. If that
  is still wrong, the switch isn't safe: C or B, the owner's call.
- **The forced-fallback device check shows copy-path frames too slow for
  a refresh at divide 2:** the measured times go to the owner (B for
  those frames, or the PLL's 149.50 / 2 for whole games).
- **The self-test finds a unit whose switch isn't clean:** that unit
  keeps divide 2 (rule 2's fallback). Nothing is locked out.
