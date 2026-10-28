# Divide 1 at 149.50 MHz ("150 fast mode"): where it stands

**Read this page first when the chase resumes** (reminder R75, due
2026-11-03). It sums up runs GC2 through GC2BW (2026-10-23 to
2026-10-27) so no test has to be run again. Each result's full data,
logs and card-log names are in [docs/hw/2700g.md](../hw/2700g.md) under
the run's name; reminders R69 (the mechanism chase), R71 (the per-device
self-test), R73 (the night soak), R74 (games) and R75 (the pause) in
[docs/reminders.md](../reminders.md).

## GC2BX (2026-10-27, gpubench-0150): games fault at 149.50 / 1 warm

With every cure on, at peak heat, a watertight game-shaped mesh draws
wrong in **every render** (200 of 200 flat, 197 over a layer, 200
textured, 200 depth tested), in four forms at once: cracks, one-step
colour errors on flat colour, one-step texel errors, patches in another
object's colour. A one-render search of 2,563 levers on the generator
found four that clear it (TSP bits 14 and 15 at 2.9-3.7x the time, and
the vertex order rotated); none clears the meshes. **The corner scenes
were too easy a proof.** Divide 1 at 149.50 is past the core's margin
when warm for game content. **The fallback's step is not measured for
games:** 146.25 / 1 is clean warm on the corner scenes only.

## GC2BY (2026-10-27 9:02 PM, gpubench-0151): games are right at 143.00 / 1

The four game meshes, 100 rounds a step on the warm ladder (`TEMP0`
873-875 throughout): **every frame right at 143.00, 139.75, 133.25,
117.00 and 108.33 / 1 and at 149.50 / 2**; 146.25 / 1 wrong in 8 of 400
(5 flat on black, 3 textured), each a single pixel at x = 255 (the same
two pixels every time: (255, 559) and (255, 416)); 149.50 / 1 wrong in
383 of 400. **143.00 / 1 is the game-safe step measured:** 2.87x
Windows Mobile's clock on the flat frames, 2.24x on the textured one,
1.91x divide 2 at 149.50, 4.5% slower than 149.50 / 1. So the
temperature guard's step for games measures at 143.00 / 1, not 146.25
(the owner rules on R76). Full table in 2700g.md "GC2BY on the X51v".

## Status (2026-10-27): parked, the interim plan in force

The owner's last fix of this round (GC2BV's seals) didn't cure the
crack, so, by his ruling, the chase is parked until 2026-11-03 (R75) and
**the interim plan is the temperature guard**: 149.50 / 1 stepping down
to 146.25 / 1 at peak heat (R76 builds it). The chase resumes from
"Next" below.

## The goal and the owner's rulings

- **The goal:** the 2700G's 3D core at divide 1 with the system clock at
  149.50 MHz (PLL 23 / 2), lossless, at full speed, **games included**,
  on every X51v (the any-Axim rule), proven warm. 149.50 / 1 is 3.0x
  Windows Mobile's 3D speed.
- **The fallback (the owner, 2026-10-27):** a temperature guard, 149.50 /
  1 that steps down to 146.25 / 1 when `TEMP0` reads peak heat. 146.25 /
  1 was clean warm in every ladder (GC2BB, GC2BF; 300 of 300 on the worst
  scene). *Corrected after GC2BX:* those ladders ran the corner scenes;
  no game-shaped mesh has run at 146.25 or below, so the step the guard
  drops to is not yet measured for games. It costs about 2% speed, at peak heat only, and needs no
  understanding of the crack. **It is the interim plan only if GC2BV and
  GC2BW don't pan out**, and the chase resumes after the pause; it is not
  the answer while untested fixes remain.
- **Per device:** the first-docking 20-minute self-test (R71) measures
  each unit's margin; every cure here is software and universal.

## Facts every test rests on

| fact | run |
|---|---|
| Divide 1 faults at every system clock (58.5 MHz included); divide 2 at the same clock is right | GC2D |
| 149.50 / 2 and 74.75 / 1 are clean on the fault generator | GC2AF |
| The fault rate follows heat: it climbs over ~10-20 minutes of drawing and falls only when the chip cools (an idle pause at Windows Mobile's clock); setting the clock again clears nothing | GC2AY, GC2AZ, GC2BA, GC2BB |
| `TEMP0` (the touch controller's diode, lower = warmer, ~3-4 counts a degree) is the thermometer; the chip at 149.50 stays warm even idle | GC2AZ correction, GC2BB |
| **Every proof must run warm:** 10 minutes drawing scene 20 at 149.50 / 1 first (`gc2_hot_phases`) | GC2BC on |
| Render time doesn't change with heat (4.48 ms on scene 20); the core gets pixels wrong at the same speed | GC2AZ |
| The rate swings run to run at the same `TEMP0` (scene 3: 2-14%) | GC2BP-GC2BR |

## The faults and their cures

Two faults showed up at divide 1, each with several forms. "Proven"
means 0 wrong warm over the run named.

| fault | form | cure | proven | cost |
|---|---|---|---|---|
| **Tile race** (a tile's write-out held at the wrong moment) | the big reader: the object lists in SDRAM | **the lists on the die** (36 KiB at `0xA5000`, `GC2BB_PARAMS`) | the ring scene 0 of 300 (GC2BD); 18-fold cut on scene 20 (GC2BE) | 0.1-0.9% faster |
| | the small reader: the display's fetch | **the checker tile order** on top of the lists on the die | scene 20 0 of 300 (GC2BF); 0 of 27,026 over every geometry and an hour's soak (GC2BG) | 0.15% faster |
| | residue A: **the crack** on a quad's shared diagonal, at one screen place, idle pipeline, peak heat | open: GC2BV's seals | see "Open" | |
| | residue B: **the green step** (grey one green bit high over a tile's share of the top-right rectangle), with the layer | open | see "Open" | |
| **Texel coordinate error** (about +0.009 texel warm, one-sided) | a sample on a clamped texture's texel edge reads the next texel | **the half-texel shift** (`TexFix::ClampUv`, `Scene::KindUv`) | textured sweep 0 of 2,200 (GC2BG, GC2BP) | none |
| | a dyadic per-pixel step lands samples short | **the span nudged up 8 UV units, only on axes magnified 16x or more** (`gc2::nudged`, `NUDGE_MAGNIFIED`) | kinds 0 of 800 (GC2BP); blanket nudging broke kind 5 (GC2BO) | none |
| | extreme magnification (an 8x8 texture over the screen) | **the texture enlarged once to 16x16 on the CPU, then nudged** (`upsampled_words`) | kind 10 0 of 810 (GC2BO), 100 of 100 (GC2BN) | +1% on such textures only |

At peak heat in GC2BP, with every cure on: the kinds 0 of 800, the
textured sweep 0 of 2,200, the ring 0 of 300, the 20-minute soak 0 of
10,036, the flat sweep 0 of 2,200 except scene 3.

## Ruled out: don't run these again

| idea | run | result |
|---|---|---|
| A bit in the render block `0x600`-`0x6FC` (1,696 bits) | GC2AR | none cures |
| A bit in the TA and manager block `0x804`-`0x87C` (800 bits) | GC2AY, GC2AZ | none; the six "clean" bits were chance |
| The per-tile wait `0x664` | GC2AS-GC2AX, GC2BB, GC2BF | cuts one fault's words, no cure; worse on sweeps |
| A header bit (134 bits) | GC2AP | only TSP bits 14 and 15 |
| Punch-through (TSP bit 14) or bit 15 | GC2AQ, GC2BC | cures the race and the texel error, at **3.7-4.5x the time** (no use) |
| The local memory's timing `LMTIM`, drive strength `LMCFG` | GC2T, run S/W | loosening makes it worse |
| The display's memory control `DMCTRL` (each byte halved and doubled, Linux's) | GC2BC, GC2BS | nothing; bytes 0-1 doubled starve the display |
| The render begun at a scan line; bands behind the scan | GC2BC | the arbiter's wait doesn't care where the scan is |
| The display's fetch off | GC2BB, GC2BF, GC2BS | cures scene 20 (its reader) but not scene 3 (17 of 100); costs the screen |
| The video plane as scanout | GC2BS | worse (78 of 100) |
| A double buffer on the die (the display reading the other buffer) | GC2BE | no difference (267 against 267) |
| The target in SDRAM | GC2BS | +19% time, not clean |
| The region array on the die; the manager's block on the die; the array moved in SDRAM | GC2BT | no change: no reader outside the core is left |
| The CPU off the 2700G's bus during the render | GC2AF, GC2BT | no part |
| Tile orders for the residue: rows, reversed, columns, columns reversed, scrambled | GC2AN, GC2BQ | checker is the cure; none clears the residue (scrambled least, and ~2% faster) |
| The shading: the background flat, the rectangles Gouraud | GC2BR | not the cause |
| The depth path (`greater` with writes), depth writes alone | GC2BU | worse (25 and 35 against 15-24) |
| The rectangles half a pixel over | GC2BU | moves the crack, doesn't end it |
| Fixed offset compensation; splitting the primitive | GC2BI, GC2BJ | no |
| Heat as uniform (the owner's push-back) | GC2BA, GC2BB | heat sets the rate; the faults' places are fixed by geometry |

## Open (as of 2026-10-27)

1. **The crack (scene 3 without its layer):** dropped four-pixel groups
   just left of the middle-right rectangle's TR-BL diagonal, showing the
   background, at (418, 246), (403, 258), (386, 274); only in that cell
   (moved three tiles any way, clean: GC2BU's map); 15-31% of renders at
   peak heat; 10-20 times rarer with the hidden layer. **GC2BV
   (gpubench-0149): no seal cures.** The other diagonal cracks the
   bottom-right rectangle in **300 of 300** renders (a deterministic
   generator: start any new idea there); a band over the seam cuts the
   crack threefold and moves it to the next marginal edge; a copy split
   the other way cracks 300 of 300. No per-quad software seal reaches it,
   and game meshes would meet it on any marginal edge.
2. **The green step (scene 3 with its layer, 1-14%):** `0x8410` read
   `0x8430` over a tile's share of the top-right rectangle, along its
   right and bottom edges. Not shading (GC2BR), not the order (GC2BQ),
   not a port reader (GC2BS, GC2BT). Untested idea: the same edge-seal
   class as the crack.
3. **Games (R74):** a seal covers our own rectangles, not a game's
   meshes. **GC2BW** draws watertight warped meshes (flat on black, over
   a layer, textured, two depth tested) beside scene 3 as the control.
   Clean meshes with the control faulting: games need nothing more. A
   crack: the chase needs a cure that covers every shared edge. *Built
   and emulator-checked, not run (the chase parked first).*
4. **The lists' on-die budget is a real limit for games:** in GC2BW's
   emulator check a frame of two 30-quad meshes overlapping was refused
   (`TooComplex`). The cause, read from the renderer: it caches at most
   128 different per-tile object lists (`LISTS`); past that every tile
   writes a fresh list, and those overflow the 20 KiB of list space left
   on the die (36 KiB less the 16.25 KiB of object slots). A game frame
   with more objects needs a larger list cache, lists split between the
   die and SDRAM, or the frame drawn in passes; the cure that rests on
   the lists on the die must hold for it (amendment D).
5. **The night soak (R73)** of every cure, 2+ hours, only once 1-3 are
   clean, built with what GC2BV and GC2BW prove (the owner, 2026-10-27:
   one soak image, made after both, not before).
5. **ADR-0035 amendment D's renderer rules** for the owner's vote once
   proven: the lists' on-die budget (36 KiB beside a 480x640 screen), the
   checker order against tearing (R20), the nudge rule, the enlargement
   rule, any seal.

## How the tests are built and run

- Every run is a variant of `rp-gpubench` behind one flag
  (`RP_GPUBENCH_GC2_<NAME>=1`), built with
  `cargo xtask codegen-variant rp-gpubench <name>` and loaded with
  `RETROPANE_LOG_WAIT_SECS=4200 cargo xtask load target/variants/<name>/rp-gpubench.bin`
  (the device lock `local/DEVICE-IN-USE.txt` first; the log in
  `local/rp-gpubench-run<RUN>.txt`, the card log in `rplog/` on both
  cards).
- Each new flag goes in four places in `boot/gpubench/src/main.rs`: the
  run's OR list, the plan text, the patch size, the dispatch; and an
  emulator entry in `xtask/src/emu.rs` checked with
  `RETROPANE_QEMU=wsl RETROPANE_WSL_QEMU='$HOME/retropane-qemu-gpu/build/qemu-system-arm' RETROPANE_QEMU_ANY=1 RETROPANE_GPUBENCH_RUNS="run <RUN>" cargo xtask emu-gpubench`
  (the GPU fork; the stock emulator can't set the clock). Use
  `CARGO_TARGET_DIR=target-alt` while a device run holds `xtask.exe`.
- The method every run since GC2BC follows: the 10-minute warm-up;
  references at Windows Mobile's clock, repeatable; every target
  poisoned first; ways paired render by render with the baseline (the
  way first on alternate pairs, so drift lands on both); `core_taken` at
  every renderer switch (audit C91); the controls at Windows Mobile's
  clock after; each wrong render described (the place, the value, the
  tile place).
- The scene runners: `gc2_orders` (plans through `OrderPlan`; it builds
  its own scenes, so a renderer-level lever must go through the scene or
  `gc2::extra_slots`: GC2BU's flaw), `gc2_pairs`, `gc2_hot_phases`.
  Scene 3 without its layer (`Scene::Corner3(S3_AS_IS)`) is the
  strongest generator left.
- Size: the gpubench stack is 56 KiB and its loaded budget 200 KiB (test
  image; ratchet freely). At most two renderers live at once in a run.

## Next, when the chase resumes

1. **Run GC2BY: done 2026-10-27** (above): 143.00 / 1 the highest step
   with every game frame right. The single pixel at x = 255 that fails at
   146.25 / 1 (and at 149.50) is the smallest form of the game fault
   left: a lead for the chase, not yet chased.
2. Start from the deterministic generator (scene 3 split TL-BR, 300 of
   300 at peak heat) for any coverage idea: one render answers it.
   Untried ideas: the ISP's own settings for edge rules or sample
   position (if any exist in the undocumented blocks, read Dell's GLES
   for them first), the triangle set-up's precision (vertices nearer
   the tile origin: coordinates relative to a smaller box), and
   overlapping opaque triangles by a fraction of a pixel in the driver
   (a conservative edge, opaque only).
3. If a cure holds on the generator: the whole proof set (both sweeps,
   the kinds, the ring, GC2BW), then the night soak (R73, one image with
   every cure, built after the results), the renderer rules for the
   vote, the self-test (R71).
