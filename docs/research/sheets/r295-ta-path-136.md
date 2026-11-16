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

## Run 22 (2026-11-16, 3:01 PM): the hangs are real stalls

Marker pixels (magenta words at five places down the screen) written
before each ladder render and read when a render is called hung: **in
no hang were all five drawn over**, so the core stops part-way through
the tiles (rows, top first) and does not finish; the done bit is not
lost. Where it stops varies (0 to 4 of 5 marks drawn; most often between
the second and third mark). No cure: the TA's parameter memory from the
die's first free byte on (38 of 100 against controls 33 and 38), looser
memory timing at 136.50 (`LMTIM` `0x955D2`: tRP, tRCD 5, tRC 13; 34 of
100). 108.33 / 1: 0 of 80. Next (run 23): the frame split by kind of work
(every texture 8 x 8, no models, no world) to find the path that stalls.

## Run 23 (2026-11-16, 3:06 PM): the models stall the core

The same 136.50 / 1 frames with part of the work left out (the runner
rewriting the checked words on their way to the TA):

| Cell | Hangs |
|---|---|
| control | 24 of 80, 34 of 80 |
| every texture 8 x 8 (almost no texel reads) | 35 of 80 |
| **no models** | **5 of 80** (every 16th frame exactly: something periodic, not yet named) |
| no world (models only) | 12 of 80 |
| 108.33 / 1 | 0 of 40, 0 of 40 |

Texel reads are not the cause. The models are: only they carry Gouraud
(TSP bit 22), the alpha test (see-through skins, ISP `0x80000000`) and
the TA's fans. Run 24 splits those.

## Run 24 (2026-11-16, 3:10 PM): the models split — log not yet read

Cells at 136.50 / 1: the models' fans left out, their strips left out,
their alpha test off, the models flat. The run returned (45,849 bytes of
log on the loader), but the PC's USB link failed afterwards ("Device
Descriptor Request Failed"); the loader's copy was lost to the soft
reset. Its card log is to be read with `cargo xtask cardlog` once the
link is back, or the run repeated.

**Run 24 read (card log, after the PC restart).** At 136.50 / 1, of 80
renders: control 25 and 42; the models' fans left out 45; their strips
left out 11; their alpha test off 17; flat 31. The strips matter, the
fans do not. But the two controls 25 and 42 show the confound: the view
moves through the run, so cells run at different moments see different
scenes. Run 25 alternates each change with its control frame by frame.

## Run 25 (interleaved: each change on every other frame, the frames between it its control)

Hangs on a frame that followed a good one (a hang's own successor
counted apart, below):

| Change (136.50 / 1) | With it | Without (same cell) |
|---|---|---|
| no models | 2 of 45 | 10 of 52 |
| **no model strips (fans kept)** | **2 of 38** | **15 of 49** |
| the models' alpha test off | 5 of 51 | 9 of 54 |
| no model fans | 9 of 50 | 6 of 47 |

On a frame right after a hang: 5/7, 7/10, 4/12 (control frames) and
5/15, 8/22, 3/10 (with the change) hung again, whatever was drawn: **a
hang makes the next render likely to hang** (the bursts). Two findings:
the models' strips start the hangs (fans, alpha test: little or none),
and the recovery (3D clock cycled, core reset, manager set up) leaves the
core fragile at 136.50. Run 26: tiny strips and slivers apart, and a
background-only render after each hang.

## Run 26 and where it stands (2026-11-16)

| Cell (136.50 / 1, interleaved) | Hang after a good frame: with / without | After a hang: with / without |
|---|---|---|
| tiny model strips out (372 a frame) | 4 of 47 / 9 of 51 | 4/13 / 4/8 |
| strips with a sliver out (411 a frame) | 6 of 42 / 16 of 52 | 1/18 / 2/7 |
| **a background-only render after each hang** (every frame) | 6 of 44 / 14 of 50 | **3/16 / 1/9** |
| no model strips (repeat of run 25) | 8 of 48 / 10 of 48 | 3/12 / 2/11 |

- **The warm-up render cures the bursts:** after a hang the next render
  hung 4 times in 25 (16%), against about half in run 25.
- **No change to the frame cures the fresh hangs.** Less model work
  lowers the rate (to roughly 10% a render), but run 26's repeat of "no
  model strips" (8 of 48 against 10 of 48) did not reproduce run 25's
  2 of 38: the rate follows the amount of work in the frame, not one
  kind of draw. That is a margin of the core at this clock on the TA's
  lists, not a word we send wrong.
- The CPU binner's path is the one proven clean at 136.50 / 1 (GC2DT,
  1,200 of 1,200 game frames, lists on the die): ports drawn that way
  get the game step already. The TA path's clean top is 133.25 / 1
  (1 hang in 240 in runs 18-19, 2 in 100 with Dell's words).

### Options for the owner (results, not a decision)

- **A. The TA path at 133.25 / 1 with the warm-up recovery** (and a
  shorter hang limit): ~1 render in 200 or fewer stalls, each now one
  dropped frame instead of a burst. 2.2 ms a render faster than today's
  108.33.
- **B. 136.50 / 1 for ports on the CPU binner's path (proven), 133.25 /
  1 for the TA path**, each its own proven step, as the guard's design
  already allows.
- **C. Keep chasing 136.50 on the TA path** with a new lead only (stop
  rule: one decisive run each): the per-tile wait `0x664` swept by value,
  or the region array rewritten in the checker order that cured the
  binner's tile race; each a run.

## Run 27: the per-tile wait `0x664` (the owner's pick, option C): no cure

At 136.50 / 1, the wait on every other frame, a warm-up after each hang;
hangs on a frame after a good one, with the wait / without:

| `0x664` | with | without |
|---|---|---|
| 15 | 3 of 46 | 9 of 50 |
| 8 | 8 of 44 | 10 of 44 |
| 4 | 9 of 48 | 7 of 46 |
| 15 again | 5 of 52 | 6 of 54 |

All four together: 25 of 190 with the wait, 32 of 194 without: no
difference beyond the noise. The lead is closed (stop rule: one
decisive run). A render takes ~1 ms longer at 15. The leads left: the
region array rewritten in the checker order (the binner's tile-race
cure), and the warm-up's effect at 133.25 / 1 over a long run.

## Run 28: the region array in the checker order (the owner's pick): no cure

The TA's region array copied by the CPU into the binner's checker order
(even tiles from the last, then odd; the last-entry bit moved) on every
other frame, a warm-up after each hang. At 136.50 / 1: **30 of 160
hangs with the checker order, 32 of 160 in rows**; after a good frame
16/63 and 6/65 against 12/60 and 13/68. At 108.33 / 1 the checker order
drew every render (0 of 20). The copy costs 2.9 ms of CPU a frame. Lead
closed.

Every lever within the frame and the render block is now tried
(layouts, render words, `0x61C`, `0x664`, memory timing, tile order,
content), none cures 136.50 on the TA path. What is left differs from
the proven game step only in where the core reads its lists: the TA's
linked blocks in SDRAM against the binner's lists on the die, which a
TA frame's lists are too large for. 133.25 / 1 stays the TA path's
near-clean top (option A); 136.50 / 1 stands proven for the CPU
binner's path (GC2DT).

## Run 29: 10 minutes at 133.25 / 1 with the warm-up (option A's check): not clean

`RETROPANE_XASH_PLAY=600`, `-rpclock 41 4 0 1`, the warm-up render
after every hang now in normal play, the hang limit 50 ms (was 200):
**209 hangs in 11,071 frames (1 in 53, about one every 2.6 s)**, steady
over the whole 10 minutes (0-6 a 5 s window, no rise with heat). The
short cells' 1 in 240 understated it. Option A is not clean either.
The run also showed the ladder's split costing 3.8 ms a frame in normal
play (the feed 13.2 ms); fixed: the frame goes straight to the TA unless
a cell changes it. **Run 30 (108.33 / 1, the default): 47.7 ms a frame,
21.0 a second**, the feed 9.4 ms again, 0 hangs.

The owner (2026-11-16): not giving up on the faster clock. Run 31: ADR-0035
amendment E's alpha-0 overlay (the SDRAM cure never tried on the TA path).
