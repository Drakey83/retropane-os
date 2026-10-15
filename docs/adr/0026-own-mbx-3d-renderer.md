# ADR-0026: Our own MBX 3D renderer

- **Status:** Accepted (2026-10-15, by the owner's review: D1-D8 all as
  recommended: both scene paths, dirty tiles, interrupt waits with the
  timer fallback, register shadow with diff-only writes, 3D straight into
  the ADR-0023 picture, one GPU owner, GLES scoped to games, the 3D clock
  gate). D2 stays gated on device test T1 and D5 on T3. The device tests
  in "Before building" each need the owner's go.
- **Date:** 2026-10-15

## In plain words

The 2700G's 3D core now draws on our own terms: flat, shaded and
textured triangles, both from objects we build ourselves and through the
chip's tile accelerator (ADR-0024, versions 3-18). The owner's direction
(2026-10-15): **the GPU driver must be built for RetroPane's own work and
be as fast as the silicon allows, not a copy of Dell's driver.** Dell's
code stays a source of hardware facts only.

Dell's way, as read from its code (`docs/research/driver-inefficiencies.md`):
the CPU spins while the GPU works, every frame rewrites ~29 registers
through a function call each, and every frame renders the whole screen
(~12.5 ms at 32 bpp) however little changed. This ADR proposes a renderer
that does none of that:

1. **Only changed tiles are rendered** (the screen is 60 x 40 tiles of 8 x
   16 pixels; the region array we hand the chip can list just some).
2. **The CPU sleeps while the GPU works** (wait on the interrupt).
3. **State is written once**, not every frame.
4. **Two ways to build a scene, each where it wins**: our own objects for
   the interface (few, large, known shapes), the tile accelerator for
   games (many small triangles).
5. **One owner for the GPU**: the graphics service. Programs never touch
   its registers (Dell lets any GLES program do so: a security hole).

## Context

Measured on the X51v (ADR-0024; `local/rp-mbx3d-run*.txt`):

| Fact | Value |
|---|---|
| 3D clock | `MBXCLK` bits 1-0 = 2 (WM idles at 1, 2D only) |
| Tile | 8 x 16 pixels at 32 bpp; 60 x 40 tiles on 480 x 640 |
| Full-screen render, 32 bpp | ~12.48 ms with objects, 9.6 ms all tiles empty |
| Tile accelerator (TA), small scene | done in 1.2-1.9 ms |
| Objects | flat 17 words, textured 26 words, list pointer bits 24-21 = layout, bits 30-25 a triangle mask |
| Textures | twiddled, point sampled, sides 2^(code+3), ARGB1555 / RGB565 / ARGB4444 / RGB332 |
| TSP that draws textured | `0x22220E80` (the game's `0x22640200` never draws for us) |
| Parameter memory | must sit low (version 14: pages above 8 MiB landed elsewhere) |
| CPU writes into 2700G memory | ~33 MB/s (the bus to the chip; DMA 30 MB/s, CPU free) |
| 2D engine and TA | share one command FIFO (`0x12C` level) |
| Interrupt | the 2700G's line is GPIO85; Dell's handler takes status bits 2 (3D done) and 4 (TA done) |

The bus is the bottleneck, not the GPU: every word the CPU writes to the
chip costs ~120 ns. A design that writes fewer words wins.

## Decisions (for the owner's vote)

### D1. Who builds a scene

| | A: our own objects and region arrays (CPU) | B: the tile accelerator | **C: both, each where it wins (recommended)** |
|---|---|---|---|
| How | the CPU works out which tiles each triangle touches, writes objects, lists and the region array | the CPU streams vertices; the TA bins them and writes lists itself | interface and 2D-like work through A; games through B |
| Bus words per textured triangle | 26 (object) + 1 per tile touched + list ends | 21 (3 vertices x 7) + state when it changes | the cheaper of the two per client |
| CPU cost | binning per triangle (cheap for big quads, costly for thousands of small triangles) | none for binning | |
| Control | complete: exact tiles, static scenes can be reused frame to frame without rewriting | the TA's memory manager decides where lists go (its rules only partly known) | |
| Known risks | none new: versions 3-13 drew this way | page allocation rules, FIFO shared with the 2D engine | both paths to maintain |

Why C: the interface is a handful of large quads (glass, images, text
atlases) that rarely change, where reusing a built scene costs no bus
traffic at all; a game frame is hundreds of small triangles, where the TA
does the binning in hardware and the CPU writes fewer words.

### D2. Render only the tiles that changed

**Recommended.** The region array is ours to write: a frame that changed
three tiles lists three tiles. Expected: render time in proportion to the
area changed (12.5 ms for the whole screen, so well under 1 ms for a
button). **Unmeasured:** that tiles left out of the array are left
untouched in the target (version 3 suggests drawn tiles are filled
entirely, unlisted ones untouched). First device test (T1 below).

Alternative: always render everything (Dell's way): simple, 12.5 ms per
frame, the 2D renderer (ADR-0023) would still beat it for the interface.

**T1 on the device (2026-10-15, `rp-mbx3d` version 19,
`local/rp-mbx3d-run19.txt`): passed, D2 is no longer gated.** A blue
full-screen object over grey, the region array listing:

| Tiles listed | Render done | Unlisted tiles |
|---|---|---|
| 2400 (all) | 12,484 us | (none) |
| 600 (rows 15-24) | 3,128 us | untouched (grey) |
| 100 (10 x 10) | 528 us | untouched |
| 1 | 16 us | untouched, its neighbours too |

Linear at **~5.2 us per tile**; tile edges exact (all four corners of the
listed tile blue, the next pixel right and below grey). A button of ~8
tiles renders in ~40 us where Dell's driver renders the whole screen
(12.5 ms) every frame.

### D3. Waiting for the GPU

| | A: poll status with a timer (today's test images) | **B: interrupt (recommended)** |
|---|---|---|
| CPU while the GPU works | busy (Dell spins too) | asleep or running other threads |
| Latency | the poll interval | the interrupt's (GPIO85, already used for display events) |
| Unknown | none | whether bits 2 and 4 raise GPIO85 (Dell's handler suggests yes): test T2 |

Fallback if T2 fails: a one-shot timer at the expected finish (render time
is predictable: tiles x per-tile cost), as the 2D renderer does for the
2D engine (ADR-0022 decision 7).

**T2 on the device (2026-10-15, `rp-mbx3d` version 20,
`local/rp-mbx3d-run20.txt`): no edge.** With status bit 2 enabled in
`0x130`, `MINTENABLE` `0x800F` (WM's) and display events off, a
full-screen render set bit 2 at 12,484 us and GPIO85 never edged (nor in
the control without the enable, nor when the bit was cleared). So the
renderer uses the fallback: render time is predictable (T1: ~16 us plus
~5.2 us a tile), so it sleeps until the expected finish, then reads the
status once. Still open, by static reading first: which other enable
routes the 3D core to the line (`SINTENABLE` `0x20` / `SINTSTAT` `0x24`,
layout unknown; how Dell's interrupt thread is wired for SYSINTR 27).

### D4. State written once

**Recommended.** The renderer keeps a copy of every 3D register it has
written and writes only differences. A frame then costs the region array
address, the parameter base and the start: 3 writes, where Dell writes
~29 through function calls. Dell's 15-pair table is the proven minimum.

### D5. Where the 3D core draws

| | A: its own buffer, copied into the screen picture by the 2D engine | **B: straight into the ADR-0023 screen picture (recommended)** |
|---|---|---|
| Cost | an engine copy per frame (~9 ms full screen at 32 bpp) | none |
| Mixing with 2D | in the copy | the 2D renderer and the 3D core take turns on the same picture (one queue in the graphics service) |
| Unknown | none | the 3D core's target stride and offset set to a sub-rectangle (`0x660`, `0x65C`): test T3 |

**T3 on the device (2026-10-15, version 20): passed, D5 is no longer
gated.** The target `0x65C` moved inside the picture with the stride
kept: a 4 x 4 tile block (32 x 64 px) landed exactly at (0,0), (0,100),
(64,100), (8,100) and even (1,100), a 4-byte offset; edges exact (the
next pixel right and below grey), the picture's origin untouched. Each
render ~92-96 us for 16 tiles. So the 3D core draws straight into any
rectangle of the screen picture, no copy.

### D6. One GPU owner

**Recommended (and required by docs/security.md).** The graphics service
owns the 2D engine, the 3D core and their shared FIFO. Clients (the shell,
GLES programs, the CE personality's GDI) hand it command lists; it checks
every address against the client's own memory (ADR-0027, the 2700G memory
map, below) before anything reaches the chip. Dell maps the register
window into every GLES program: any of them can make the GPU write
anywhere in 2700G memory.

### D7. GLES is a client, scoped to games

**Recommended (owner, 2026-10-15).** GLES 1.x (the Common-Lite fixed-point
profile the WM games link, and the Common profile Quake 3's port ships)
becomes a library that turns GL calls into TA streams for the graphics
service. Its geometry stage runs on the CPU in fixed point (WMMX for the
inner loops, once profiled). The CE personality's drawing stays 2D (GDI);
GLES does not grow beyond games.

**Amended (owner, 2026-10-15): three layers, each in its place.**

1. **Our own driver for the OS** (D1-D6, D8): the interface and
   everything RetroPane draws, purpose-built, no GL in between.
2. **Binary GLES kept for legacy games**: the original Windows Mobile
   executables of closed games (Virtual Pool Mobile v1.29, Toy Golf, Geo
   Rally, and more found on the owner's card: the card is still being
   catalogued, and some GLES titles were unknown to us) run unchanged
   through the CE personality, on a GLES 1.x library over our driver.
   Sized by the import tables of the games actually owned, never
   speculatively; both DLL names (`libGLES_CL`, `libGLES_CM`).
3. **New native ports where the source is open** (DoomGLES's Doom
   engine, Quake III Arena, Quake Mobile's Quake engine): built straight
   on our driver for speed, with **extra features** the originals lack
   (first asked for: a brightness/gamma control: the owner can barely
   see in DoomGLES), RetroPane controls, and their packs (ADR-0018). Each
   port's licence is GPL-2.0, as RetroPane's.

### D8. The 3D clock

**Recommended:** on while 3D work is queued, off 100 ms after the last
render (a timer in the graphics service), so the interface on the 2D
engine pays no 3D clock. Built (2026-10-15):
`rp_drv_mbx_display::clock::Gate` (host-tested), which reports when it
switched the clock on so the caller can redo the set-up until T9 says
whether that's needed. Measuring the power saved needs the battery
gauge (Phase 9); the switch itself costs microseconds (version 3: PM ready
164 us after enabling).

## Before building: device tests (one `rp-mbx3d` version each, owner's go)

| Test | Question | Decides |
|---|---|---|
| T1 | A region array listing only some tiles: are the rest left untouched? Render time vs tiles listed | D2: **passed** (version 19: untouched; 5.2 us a tile) |
| T2 | Do status bits 2 and 4 raise GPIO85? | D3: **no, as tried** (version 20: bit 2 enabled in `0x130`, `MINTENABLE` `0x800F`: 0 edges); timer fallback |
| T3 | Target at an offset with the screen's stride: does a sub-rectangle render in place? | D5: **passed** (version 20: any pixel offset, even 4 bytes) |
| T4 | 16 bpp target: render time (half the bytes) | target format for games |
| T5 | TSP bits: blend (bits 31-26, as the PowerVR2 has them?), alpha, Gouraud with texture | the interface's glass and fades on 3D (version 21) |
| T6 | TEX0 bits 31-28 (filtering, clamping: version 18 saw them change sampling) | texture quality (version 21, on a hand-written textured object) |
| T7 | Depth: `0x674` and `0x610` with depth-writing objects (ISP bit 26) | games (version 21: compare modes within one render) |
| T8 | 2D engine and TA in the same FIFO, interleaved | one queue for both |
| T9 | 3D clock off and on again (`MBXCLK` 2 -> 1 -> 2): does the core keep the parameter memory set-up and its registers? | D8: what the gate must redo after switching on |

**Version 21 on the device (2026-10-15, `local/rp-mbx3d-run21.txt`):**

- **T5 blending, partly:** TSP bits 28-26 are the destination factor and
  31-29 the source factor, as on the PowerVR2: source 1 / destination 1
  drew additive (red + blue = `0xFFFF00FF`); source 1 / destination 0
  opaque. With source alpha / inverse source alpha (4/5, and 5/4) the
  result was as if alpha were 1 (`0x800100FC` over red): the square's
  vertex alpha (`0x80`) doesn't reach the blend, with or without bit 20.
  Where alpha comes from (another TSP bit, Gouraud, the ISP) is open.
- **T6:** the **hand-written textured path draws** with Dell's TSP (the
  8 x 8 checker landed exactly, texel boundaries at the expected pixel,
  texel (7,0) green where it belongs): D1's direct path holds for
  textures. Sampling stayed point with TSP bits 14-13 set (filtering is
  elsewhere: TEX0 bits 31-28 changed sampling in version 18). Past u = 1
  the **edge texel repeats (clamp) by default**; bit 15 changed nothing.
  **TSP bit 16 hung the 3D core**: that render and every one after it
  timed out.
- **T7 depth: void**, every render timed out after T6's bit 16. Lesson
  for the class: a test image must reset the core (and redo the memory
  manager's set-up) after any render that times out, so one bad bit can't
  void the rest of a run.

**Version 22 on the device (2026-10-15, `local/rp-mbx3d-run22.txt`):**
recovery after a timeout works (TSP bit 19 hung the core; it was reset
and the run went on).

- **Texture address modes found: TEX0 bits 31-28.** With u past 1:
  `0x2` repeats (wrap), `0x6` mirrors, `0xA` (Dell's) clamps to the edge
  texel; `0xC`/`0xE` break sampling. So bits 31-30 = U mode (0 wrap, 1
  mirror, 2 clamp), bits 29-28 = V mode (v stayed in 0-1: untested).
  Filtering is not in these bits either (point sampling throughout).
- **Depth compare (T7) likely found.** Blue drew with ISP modes 4 and 6
  (greater, greater or equal) and never with 1 and 3 (less, less or
  equal), at z 0.3 and 0.8 alike, the same over the red square as over the
  background. That fits the PowerVR2's numbering against a depth buffer
  cleared to 0 (`0x6DC` = 0 in Dell's table) with "always" objects not
  writing depth. Next: the red square with a writing mode, and `0x6DC`
  set, to confirm.
- **Alpha still doesn't reach the blend:** neither a flat square's vertex
  alpha nor an ARGB4444 texture's alpha changed the 4/5 blend (result as
  alpha 1); Gouraud, bit 20, bit 21 cleared changed nothing; bit 19 hung
  the core. Next, by static reading: the combine word TEX2 (the GLES
  driver takes it from a table by texture mode; Dell's `0xD6886610` and
  the game's `0x86086610` differ in bits 30, 28 and 23).

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Every claim above is a device test with a logged result; `docs/perf.md` gets each speed-up against Dell's numbers (12.5 ms full frame; busy-spin waits) |
| ✨ Modernize | Dirty-tile rendering and retained scenes are how modern compositors work; Dell's driver re-renders everything |
| 🗄️ Legacy support | WM games keep their GLES calls (D7); only the layer under them changes |
| ⚡ Better / faster | Fewer bus words per frame (the real bottleneck), a sleeping CPU, renders proportional to change |

## Hardware limit or choice?

- Full-screen renders every frame: **Dell's choice.** The region array is
  ours to shorten (T1 checks the silicon agrees).
- CPU spinning during renders: **Dell's choice.** The core raises status
  bits; whether they reach GPIO85 is T2.
- Register tables every frame: **Dell's choice.** The registers keep
  their values (they survive even between images: docs/hw/2700g.md).
- The 8 MiB parameter-memory placement: **likely silicon** (version 14).
- The bus speed to the chip (~33 MB/s): **silicon** (VLIO, measured).

## Alternatives considered

- **Reimplement Dell's driver faithfully:** rejected by the owner's
  direction and by the evidence log.
- **Only the TA (B in D1):** simpler, but the interface would rebuild its
  scene every frame through the FIFO where a retained region array costs
  nothing.
- **Only our own objects (A in D1):** fine for the interface, but CPU
  binning of thousands of small triangles costs more CPU and bus than the
  TA's hardware binning.
- **No 3D for the interface at all (ADR-0023's 2D renderer only):** kept
  as the default until T1-T5 say the 3D core can do glass and fades
  better; the 3D core then joins in where it wins (rotations, scaling,
  perspective, effects the 2D engine can't do).

## Consequences

- The graphics service grows a 3D queue next to the 2D one, a register
  shadow, and a 2700G memory map it enforces (ADR-0027).
- Eight device tests before building (T1-T8), each small.
- GLES becomes a library on top, not the driver's shape.
