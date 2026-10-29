# ADR-0038: The game path: three routes, chosen per frame

- **Status:** **Accepted (owner, 2026-10-29: "Adopt all six", the ruling
  packet [rulings-2026-10-29.md](../reports/rulings-2026-10-29.md),
  ruling 1)** with the packet's changes, recorded under "The owner's
  ruling" below. Nothing built yet; nothing works until proven on the
  X51v.
- **Date:** 2026-10-29
- Reminders: R90 (this ADR), R78 item 2 (the routes measured), R89 (the
  UYVY route's exact colours), R20 (tearing), R76 (the per-route clock
  step), R80 (the one-pixel fault at reduced size), R93 (quads, now the
  default). ADRs: 0023 (screens and planes), 0026 (our own 3D renderer),
  0034 (the compositor; amendment P's C), 0035 (the 2700G's clocks).

## In plain words

A game's frame can reach the screen three ways, all three measured on the
X51v and all three kept (the owner, 2026-10-28):

1. **Native:** drawn at 480 x 640, shown as it is. The sharpest; the GPU
   draws every pixel.
2. **480 x 320 + line doubling:** drawn at half height, each line shown
   twice by the display's own doubling (`GDRCTRL` bit 29: free, nothing
   copied). Sharp across, blocky down; the GPU draws half the pixels.
3. **240 x 320 smooth 2x:** drawn at a quarter of the pixels, turned into
   UYVY on the GPU and enlarged smoothly 2x by the video plane's scaler.
   Softest; fewest pixels drawn, a conversion paid.

This ADR proposes **how a game uses them**: the driver picks a route for
each frame from how long the last frames took (dynamic resolution, so a
heavy scene drops to a lighter route instead of stuttering), switches only
in the display's blanking (no torn or half-switched frame), lets each game
be held to one route by the user, keeps two frames in flight (the CPU
building frame N + 1 while the GPU draws frame N), and runs each route at
the fastest clock step measured clean for it.

**What the measured numbers are and are not.** The times below are
`rp-gpubench`'s test frames (four meshes of 30 quads, GC2CP), not games:
they show the routes' relative cost and the fixed overheads, never a
game's frame rate. A game's frame time depends on its own scene; the
first game port measures it (R90's proof).

## Context: what is measured (X51v, 2026-10-28)

| Route | Test frame (flat mesh) at 74.75 / 1 | at 143.00 / 1 | Picture |
|---|---|---|---|
| Native 480 x 640 | 10.2 ms | 6.2 ms | exact; the reference |
| 480 x 320 + doubling | 5.2 ms | 3.2 ms; 2.3 ms two in flight | exact rows, each shown twice (GC2CT: right both ways) |
| 240 x 320 smooth 2x | 13.6 ms serial, 12.6 pipelined | 7.4 ms serial, 6.4 pipelined | converter output exact in its words (R89, GC2CX); on-screen colour awaits the owner's eye |

(GC2CP runs 3 and 4 and GC2CU run 2; whole frames: the CPU's set-up, the
render, the poll. Fastest of 10.)

- **Clocks** (ADR-0035): native clean at 143.00 / 1 (GC2BY); 480 x 320
  clean at 139.75 / 1 (GC2CR), and at 143.00 / 1 as four-corner objects
  (GC2CU run 2: the textured fault two triangles hit there is gone);
  240 x 320 has a one-pixel fault at every divide-1 step (R80, GC2CR) and
  is lossless only at divide 2 today. **Divide-1 steps above 74.75 MHz are
  held for later** (the owner, 2026-10-29: the divide-1 work waits); this
  ADR records the per-route step as a field, its values set by that work.
- **Two in flight:** `Renderer::prepare`, `start_prepared` and
  `registers_taken` exist and ran on the X51v (GC2CP (e): 2.3-5.0 ms at
  143.00 / 1, pictures right but divide 1's known texture fault); the
  pipelined UYVY way (d) left 2-24 stray words a frame with the 2D
  placements beside the render, none with them apart (d').
- **Tearing** (R20, ADR-0034 P1 = C, the owner's ruling for the shell):
  a top-down tile order started behind the scan is tear-free; the
  checkerboard order (divide 1's cure) tears. Games are held from C
  until GC2CA's picture is judged (R20).

## The owner's ruling (2026-10-29)

- **G1: C, all three routes.** Route 3 ships only after R89's eye check
  and with G2's ranking. Not claimed: "route 3 is faster" (on the test
  frames it is faster only on the textured frame at 74.75 / 1, and 1%
  on the flat one at Windows Mobile's clock; 22% slower flat at 74.75 /
  1: 12.4 against 10.2 ms).
- **G2: B, with three changes:** the routes ranked by their own measured
  times for each game, not by pixel count; the 8- and 60-frame
  thresholds provisional, fitted on the first game port; nothing ticked
  until a device run switches routes every N frames with every frame
  hashed whole (R101). **Ticked by the owner, 2026-10-29 ("Adopt C'"),
  on R101's runs (`rp-gpubench` GC2DB and GC2DC 1-4, 2700g.md "Route
  switches"):** the switch is **way C'**: the video plane is never hidden
  in a game; under an opaque graphics plane for native and the lines
  (its base pointed at the shown picture, so it reads inside the safe
  region), the graphics plane see-through over it for the smooth route;
  every switch is the graphics plane's registers and the video plane's
  base, written on scan line 0 (the registers act at once; the clean
  window is lines 643-3 of the 645-line frame). Measured: a switch every
  frame and every 8 frames, 2,400 of 2,400 frames whole by `DSIG` (which
  covers the whole picture, every bit); its cost +0.25% of a native
  render (~50 us), renders exact. Today's hide-and-show and Dell's stream
  bit each tear one frame entering the smooth route. **At 149.50 / 2
  while rendering (R108, layout B: the half routes inside the safe
  region; ticked by the owner 2026-10-29):** every switch written with
  the 3D core drawing (1,199 of 1,199 every pass, 149 of 149 every 8),
  the first frame after every switch read and whole (1,348 of 1,348),
  3,748 judged frames whole, 0 torn, 0 renders failed, pictures exact,
  no underrun (`gpubench-0214`); a pass spans 2.5 display frames with
  today's 24 ms renderer set-up after a switch. Layout C (one shared
  target, the switch every display frame) is R112, after R20's wiring.
- **G3: as written.**
- **G4: two frames in flight for route 2** (way (e): 800 of 800 frames
  right, every frame read back, at Windows Mobile's clock and 74.75 / 1,
  2026-10-29, `local/rp-gpubench-runGC2CP-every.txt`; a count, not proof
  of zero). Route 3 uses (d') only, never (d) (R30); its in-flight gain
  is measured against a serial run before it is claimed (R102).
- **G5: the field, yes; its values held** with the divide-1 work. Until
  then every route runs the divide-2 ladder's step (ADR-0035, ruling 3 of
  the packet).
- **G6: as written, still held on R20's GC2CA picture check.**

## Decisions (as proposed; ruled above)

### G1. Which routes

| | A: native only | B: native + 480 x 320 | **C: all three (recommended, and the owner's 2026-10-28 word)** |
|---|---|---|---|
| Heavy scenes | stutter | half the pixels | a quarter |
| Picture | best | blocky down | softest, smooth |
| Cost to build | none | the doubling switch | the conversion and the scaler set-up |

### G2. Choosing per frame (dynamic resolution)

| | A: the game picks once | **B: the driver picks per frame from the last frames' times (recommended)** | C: per frame from a prediction of this frame |
|---|---|---|---|
| How | fixed at start | step down when the mean of the last 8 frames passes the frame budget (33.3 ms at 30 fps, R/port-fps rule), step up only after 60 frames well under it (hysteresis: no flicker between routes) | needs the scene's cost before drawing it |
| Switch | never | in the blanking, at a frame boundary (the planes' registers latched) | same |
| Risk | stutter | one route change seen (the picture's sharpness) | mispredictions |

Rule 0: consoles' dynamic resolution (PS4/Xbox One era) scales the render
target per frame from GPU time; ours does it with the display's own
doubling and scaler, so the step costs no copy.

### G3. A per-game override

**Recommended:** the user can hold a game to one route (or to "automatic")
in the game's settings, remembered per game; the default is automatic. A
game's own request (a fixed-resolution old title) is honoured.

### G4. Two frames in flight

**Recommended:** for routes 2 and 3, the CPU builds frame N + 1 while the
GPU draws N (`prepare` / `start_prepared`), the 2D placements of the UYVY
route kept apart from the render (d', no stray words). Native stays one
frame (its set-up is already overlapped by the GPU's poll). **To prove
first:** the stray words of the overlapped way at Windows Mobile's clock
and 74.75 / 1 (tonight's item 8). **Measured 2026-10-29 (X51v, GC2CP low,
`local/rp-gpubench-runGC2CP-low-2.txt`; 2700g.md):** (d') 0 stray words at
both clocks; (e) 0 words off on each frame's last picture at both; (d),
the 2D placed beside the render, 0 at WM clock and 16-38 a frame at 74.75
/ 1. Two in flight against one at a time at 480 x 320: 12-14% less a
frame at WM clock, 15-19% at 74.75 / 1. A result for the vote, not a
ruling.

### G5. The clock step per route

**Recommended:** each route carries its own step (ADR-0035's ladder), the
driver switching the step with the route; the values come from the
divide-1 work, held for later. Until then every route runs at the shell's
step.

### G6. Tearing

**Recommended:** the game path follows R20's ruling once GC2CA's picture
is judged: C (render behind the scan, top-down order) where the route's
tile order allows; the checkerboard order (divide 1's cure) takes A's copy
path.

## Consequences

- One game path in the graphics service (ADR-0026 D6: one GPU owner), the
  route invisible to the game.
- The UYVY route needs the converter set per frame it shows (R89's words)
  and its colour judged by the owner first.
- Divide-1 values arrive later without changing the design.

## Not decided here

The clock values (divide-1 work), R80's one-pixel fault, the per-game
settings' screen (the shell's design).
