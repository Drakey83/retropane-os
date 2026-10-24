# The compositor harness: a benchmark for T1, T3 and T4

**Status: built for the gate (2026-10-24, overnight, the owner's go:
"the compositor harness and the step 6 batch").** A benchmark image, not the
compositor (the owner's vote, 2026-10-21, option a). It draws the frames of
[workload-brief.md](workload-brief.md) (locked by the owner, 2026-10-22)
the way [compositor.md](compositor.md) and ADR-0034 say the real
compositor will, times them, and answers three gate lines:

- **CPU T1, a pipeline or turns:** does the CPU building frame N+1 while
  the 2700G draws frame N beat the CPU and the 2700G taking turns?
- **CPU T4, each unit's job:** for each kind of work in those frames
  (fills, copies, text, glass, scaled pictures, a rotation), which unit
  (the CPU, the 2D engine, the 3D core, the display's planes) does it
  fastest, and at what CPU cost?
- **CPU T3, commands by DMA, the driver's choice:** run FD showed a DMA'd
  2D batch draws exactly as the CPU's writes; the harness prices the two
  inside real frames, so the driver's choice is made on frames, not on a
  lone batch.

Results, not decisions: the harness reports; the owner rules the lines.

## The rules it is held to (the ADR-before-code rule)

| Rule | What the harness does |
|---|---|
| ADR-0035 rule 2 and 8 (the clock is Windows Mobile's until the self-test; changes only where the display can take it) | **No clock change at all.** It runs at whatever the loader leaves (Windows Mobile's 99.67 / 2), reads `COREPLL` and `MBXCLK` at the start and end and logs them. Nothing here touches divide 1 |
| ADR-0030 G2 (one queue: a 3D render starts only when the 2D engine is idle) | Every way keeps the 2D engine and the 3D core in turns. The pipeline way overlaps **the CPU** with the 2700G, never 2D with 3D (that overlap is P8, run FT: unsafe as set up today; GC3's audit item 2) |
| ADR-0034 Gap 1 (never a 3D render over a plane picture) | Pans (W1, W2, W4, W9) live on the planes only; the 3D frames (W3, W5, W6) compose only textures |
| ADR-0034 C2 / ADR-0023 amendments (16-bit screen, glass ARGB4444 or 8888) | The page on the video plane is RGB565; the glass on the graphics plane ARGB4444; 3D targets RGB565 with the core's dither (`0x650` bit 3, P21's ruling) |
| ADR-0034 Gap 2 (the price table fixed, renders logged only) | Each 3D frame's price from the table (5.3 us a tile, +6 textured, +8 a translucent layer) is logged beside its measured time; nothing adapts |
| ADR-0023 Q1 A (the 2D engine composes at 32 bits, one converting copy writes the screen) | Where a 2D frame writes the screen picture (W2's row, W4's key, the overlay), it follows the renderer's path through `rp-graphics`/`rp-mbx-render`, not a hand-built one |
| The owner's testing rules (edges, no silent errors) | Every frame's end picture is hashed; the turns and pipeline ways of one frame must give the same hashes (else the pipeline way is reported broken, not fast) |
| The label rule | "STEP n OF m" and the frame's name on screen from the first moment (a strip the frames leave alone) |
| The shared-card rule | Writes only its own log through `rp-cardlog`, nothing else on the card |

## The frames and what each way is

All at 60 Hz, the frame event (VEVENT0) as the clock: a frame is "late"
when its work finishes after the next frame event. Each sequence runs 120
frames (2 s of motion) unless said.

| # | Frame | Ways measured | Reported |
|---|---|---|---|
| W1 | The home page panned by the finger | the video plane repointed each frame (a tall RGB565 page, the wallpaper in it: C4); the glass header held on the graphics plane | late frames, CPU time a frame, the repoint's landing line |
| W2 | A list under a glass header, a row changing mid-scroll | the pan as W1; a new row drawn into the page's off-screen part by (a) the CPU, (b) 2D commands by the CPU, (c) the same 2D batch by DMA (T3); the window re-based by one 2D copy near its end | late frames, the row's cost per way, the CPU left over |
| W4 | The keyboard sliding up, then typing | the slide as a graphics-plane repoint over a taller glass picture; a tap lights one key: (a) the CPU, (b) a 2D fill, (c) the 2D fill by DMA | late frames; tap to lit (the key's drawing done and the frame shown) |
| W9 | The notification shade pulled down | a graphics-plane repoint over a translucent sheet (the same mechanism as W4, the other direction, a full-height sheet) | late frames |
| W5 | The task switcher, one card dragged | the 3D core: four scaled card textures, the dragged one moving; dirty tiles only; **turns** (the CPU builds the scene, then the core renders, then the flip) against **pipeline** (the CPU builds frame N+1's scene while N renders) | frame times (median, worst, share over 33.3 ms), the CPU left over, price vs measured |
| W6 | The living sky behind text | the 3D core: two cloud textures drifting under a glass and text texture; dirty tiles; turns against pipeline; the degrade step 1 (the second cloud layer dropped) logged when the price says so | as W5 |
| W3 | An app opening from its icon | the 3D core: the app's picture as a texture scaled from the icon's box to the screen over 250 ms (15 frames), full mode | completion time against 250 ms; frames shown |
| W7 | A screen rotation | (a) the 2D engine's rotated copy (B11's header bits 25-26), (b) the 3D core turning a texture of the screen | completion time against 300 ms |
| extra | A toast sliding in | a small graphics-plane repoint | late frames |
| extra | The volume overlay | a 2D fill + blend of a slider into the glass picture, 30 frames | late frames, CPU time |
| extra | The screen waking | the graphics plane's global alpha ramped from 0 to full over 15 frames | late frames |

W8 (video under controls) is phase 7's and needs MVED1: listed so the
harness leaves room, not measured (the brief says so).

**The routing table (T4):** for each kind of work met above (a fill, a
copy, a row of text, a glass blend, a scaled picture, a rotation), the
time and the CPU's own time on every unit that can do it, logged as one
table at the end.

**The CPU left over:** while the 2700G works, the CPU spins a counter
until the work is done (a calibrated loop: its count over a quiet 16.6 ms
first), so "the CPU left" is measured, not assumed.

## Run on the X51v (2026-10-24, unattended, three times)

`rp-gpubench` run CH (`RP_GPUBENCH_HARNESS=1 cargo xtask codegen-variant
rp-gpubench h`; `emu-gpubench` run CH). Every result, and what it says for
T1, T3 and T4, is in docs/hw/2700g.md "The compositor harness, fewer text
operations, the 3D core's limits, the status bits". In short: the plane
frames (W1, W4, W9, toast, wake) cost nothing and are never late; a W2 row
by the 2D engine 0.62 ms with the CPU free (2.3 ms by CPU stores); the
pipeline 9-10% shorter frames than turns, the render ~1.5 ms slower beside
the CPU; the sky (W6) 34-38 fps against its 30; the app opening 355 ms
against 250; a turned screen 26 ms by the 3D core, 42 by the 2D engine.
**Found: single stale pixels after partial frames (quality audit C71)**;
a third way, every tile every frame, is exact and stays in the run as
the control.

Results, not decisions: whether the compositor pipelines, the routing
table, and what W3 and W6 do about their targets are the owner's.

## What it can't answer

- Touch-driven latency on real touches: the motion is scripted (the brief
  asks for finger motion; the scripts follow its shapes). Real taps stay
  the ratchet's milestones (perf-ratchet.md).
- 2D and 3D at once: P8's, not here.
- Times in the emulator: the emulator checks that every frame draws, the
  hashes agree between ways and the run ends; its times mean nothing
  (emulator.md).
