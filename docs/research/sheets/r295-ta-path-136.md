# Sheet: why the TA path hangs at 136.50 / 1 (R295), 2026-11-16

**What it decides.** What Half-Life's TA path (ref_rp, ADR-0039; the
runner's `gfx.rs`) needs to run at the owner's game step 136.50 / 1
(ADR-0035 amendment D, "The game step") as cleanly as at 108.33 / 1. The
owner, after run 17: "we need to make it so 136, can be ran cleanly".

## The symptom, counted (run 17, `local/half-life/device-run-17-port-runner.txt`)

- 998 frames, **316 renders never done** in 200 ms (32%, one in three, not
  one in four as first read); 0 TA hangs, 0 frames refused.
- A render that finishes takes 10.7 ms (13.2 at 108.33 / 1).
- Renders the CPU waited for at once (every 16th, the CPU only polling the
  status register) hang about as often (~27%) as those drawn while the CPU
  makes the next frame: **the CPU's own traffic to the 2700G is not the
  cause.**
- Run 16 at 108.33 / 1: 0 hangs. Windows Mobile's clock: 0 hangs (runs
  11-15).

## What differs from the proven game step (GC2DT, 1,200 of 1,200 game frames right at 136.50 / 1)

| | GC2DT (CPU binner) | ref_rp (TA) |
|---|---|---|
| Object lists | on the die (36 KiB at `0xA5000`) | the TA's, in SDRAM (`0x200000`, linked 16-word blocks) |
| Region array, manager block, tail | SDRAM or die (GC2BT: no change) | SDRAM (`0x120000`, `0xFC000`, `0x100000`) |
| Tile order | checker | the TA's (rows) |
| Render words | Dell's start table (`0x61C` `0x20004`, `0x6D8` `0x01000000`) | the same |
| Render words Dell's GLES uses for TA scenes (DoomGLES, rp-wmprobe) | | `0x610` `0x30001`, `0x614`/`0x618` `0x1E3CE500`, `0x61C` `0x20007`, `0x648` 1, `0x6D8` `0x09000000`, `0x6DC` `0x3F800001` (GC2F: no effect on the binner's wrong pixels; never tried on the TA path or on a hang) |

A wrong read in a binner list gives a wrong pixel; a wrong read of a TA
list's link word sends the core off the list: **a hang**. So the leading
suspect is the core's parameter reads from SDRAM at divide 1, and the
parameter fetch's settings (`0x61C`, read as the PVR2's `FPU_PARAM_CFG`:
Dell's ISP burst threshold 0, below the PVR2's "never set below 0x4").

## Rule 0

- **Dell / Windows Mobile (a finding):** never runs the cores at divide 1;
  its GLES path renders TA scenes with the six words above at 99.67 / 2.
- **Textbook (PVR2 manual):** parameter burst thresholds at their defaults
  (`0x1F`), never below 4.
- **Ours:** measure first, one run, every lever against a control at the
  same clock: no guessing which word cures it.

## The run (run 18, one image, `-rpladder`)

Every render waited for at once (its own time, no overlap), the core
reset after 50 ms; each cell set up afresh (`ta::set_up`) after its clock
step; per cell the frames, hangs (as a pattern), render time, and on the
first hang the render block `0x600`-`0x6FC` against the same cell's last
good render.

| Cell | Clock | Lever |
|---|---|---|
| 1 | 108.33 / 1 | control |
| 2 | 117.00 / 1 | ladder |
| 3 | 133.25 / 1 | ladder |
| 4 | 136.50 / 1 | control |
| 5 | 136.50 / 1 | region array, manager block and tail on the die (`0x96000`-`0xB0000`) |
| 6 | 136.50 / 1 | `0x6D8` `0x09000000` (Dell's GLES) |
| 7 | 136.50 / 1 | Dell's GLES render words, all six |
| 8 | 136.50 / 1 | `0x61C` `0x20007` (Dell's GLES) |
| 9 | 136.50 / 1 | `0x61C` ISP threshold 4 (`0x20404`) |
| 10 | 136.50 / 1 | `0x61C` the PVR2's defaults (`0x7DF77`) |
| 11 | 136.50 / 1 | control again (heat, drift) |
| 12 | 108.33 / 1 | control again |

Cost: ~640 frames, about a minute inside a normal kit run; no new chip
words (the render block's own registers, values Dell's driver writes), no
board bits, nothing outside `rp-xash` written.

## What overturns it

- Cells 2-3 clean and 4 hung: the edge sits between 133.25 and 136.50;
  133.25 / 1 is then a clean option at once (the guard's own step).
- A lever cell at 0 hangs against its controls at ~30%: that word is the
  cure candidate (then its own run over many frames, the picture compared
  against 108.33's).
- No lever helps: the lists' SDRAM reads stay the suspect, the next run
  the memory timing (CL, tRCD) and the region array in checker order.
- Control cell 4 clean (renders waited at once): the overlap of render
  and next frame is the cause, not the clock alone.

**Fix code only with the owner's go** (R295); this run is a test under
his standing go for Half-Life kit runs (2026-11-16).

## Results (runs 18 and 19 on the X51v, 2026-11-16, under the standing go)

Logs `local/half-life/device-run-18-port-runner.txt`, `-19-`. Every
render waited for at once; a hang = not done in 50 ms (the core reset).

| Clock | Lever | Run 18 hangs | Run 19 hangs |
|---|---|---|---|
| 108.33 / 1 | control | 0 of 40, 0 of 40 | 0 of 40 |
| 108.33 / 1 | Dell's GLES words, all | | 0 of 40 |
| 117.00 / 1 | control | 0 of 40 | |
| 133.25 / 1 | control | 1 of 40 | 0 of 100 |
| 133.25 / 1 | Dell's GLES words, all | | 2 of 100 |
| 136.50 / 1 | control | 25 of 60, 19 of 60 | 33 of 100, 41 of 100 |
| 136.50 / 1 | region array, manager block, tail on the die | 16 of 60 | |
| 136.50 / 1 | `0x6D8` `0x09000000` | 13 of 60 | |
| 136.50 / 1 | Dell's GLES words, all | 5 of 60 | 25 of 100 |
| 136.50 / 1 | all but `0x610` / `0x614`-`0x618` / `0x61C` / `0x648` / `0x6D8` / `0x6DC` | | 14, 27, 32, 14, 28, 8 of 80 |
| 136.50 / 1 | `0x61C` `0x20007` / ISP threshold 4 / the PVR2's defaults | 15 / 29 / 13 of 60 | |

What it shows:
- **The TA path's edge lies between 133.25 and 136.50 / 1:** 1 hang in
  240 renders at 133.25 (both runs), a third of them at 136.50, none at
  108.33 or 117.00. The memory timing is the same word at both
  (`LMTIM` `0x944C2`, CL2): only the clock moves.
- **The hangs come in bursts** (the patterns: runs of `H` several frames
  long), so they follow what is on screen, not chance render by render;
  a 60-frame cell swings from 5 to 25 hangs on the same lever. No lever
  cell sits clear of the controls' spread: **no render word and no
  layout move cures it.** Run 18's 5 of 60 for Dell's words did not
  repeat (25 of 100 in run 19).
- The `0x61C` theory (the ISP threshold below the PVR2's minimum) is
  out: raising it to 4 was the worst cell.
- At some hangs `0x640` reads `0x00FFFFFF` (0 after every good render);
  at the rest the render block reads as a good render left it. What
  `0x640` holds is not known (a cursor of the core's, likely).
- The remaining suspect: the core's reads of the TA's lists in SDRAM
  (the proven game step had its lists on the die; a TA frame's lists
  are ~200 KiB and more, the die has ~104 KiB free beside the screen).

## Options for the owner (results, not a decision)

The 2700G's clock does not set Half-Life's frame rate today: the CPU
takes ~55 ms a frame, the render 13.2 ms at 108.33 / 1 (11.0 at 133.25,
10.8 at 136.50), and the render runs while the CPU makes the next
frame. A faster 2700G clock pays only once the CPU's frame is under
~13 ms.

- **A. Keep 108.33 / 1 (the kit's default now)**: clean in every run;
  costs no fps today.
- **B. 133.25 / 1**: ~1 hang in 240 renders (each a 200 ms stall today,
  50 ms with the ladder's limit); 2.2 ms a render faster than A, worth
  nothing until the CPU is below 13 ms.
- **C. Keep chasing 136.50 / 1 clean**: the next run puts the first part
  of the TA's parameter memory on the die (the pages it fills first) and
  tries the memory's own timing at 136.50 (tRCD and tRC a cycle longer),
  one cell each against the control; if the lists in SDRAM are the
  cause, a full cure may need the frame drawn in two halves (twice the
  TA's work, ~10 ms of CPU) or is not reachable.
