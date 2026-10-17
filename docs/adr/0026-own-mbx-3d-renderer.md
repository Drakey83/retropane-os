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
| T5 | TSP bits: blend (bits 31-26, as the PowerVR2 has them?), alpha, Gouraud with texture | **passed** (version 23: factors in 31-26, bit 13 on; vertex and texture alpha) |
| T6 | TEX0 bits 31-28 (filtering, clamping: version 18 saw them change sampling) | texture quality (version 21, on a hand-written textured object) |
| T7 | Depth: `0x674` and `0x610` with depth-writing objects (ISP bit 26) | **passed within a render** (version 23: TSP bits 11-9 mode, bit 7 no-write; larger z nearer); storing depth across renders (`0x674`) still open |
| T8 | 2D engine and TA in the same FIFO, interleaved | one queue for both |
| T9 | 3D clock off and on again (`MBXCLK` 2 -> 1 -> 2): does the core keep the parameter memory set-up and its registers? | D8: **kept** (`rp-mbx3d-api` run 4: nothing redone, 1 ms and 300 ms off, drew right); the renderer redoes nothing (`ClockPolicy::KeepAll`), a clock-on frame ~1.0 ms instead of ~5.6 ms |

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

**Version 23 on the device (2026-10-15, `local/rp-mbx3d-run23.txt`):
alpha blending and depth work**, with the forms read from the GLES
driver (`0x02267700` builds the TSP; `glBlendFunc`, `glDepthFunc`,
`glDepthMask`; TEX2 table `0x0225D3A0`):

- **TSP bit 13 turns blending on** with the factors in bits 31-26
  (source 31-29, destination 28-26, GL's codes: 0 zero, 1 one, 2/3
  destination colour and inverse, 4/5 source alpha and inverse, 6/7
  destination alpha and inverse). A flat square at alpha `0x80` over red:
  `0xBF7F007F`, half and half exactly; an ARGB4444 texture at alpha 8/15:
  `0xBF770087` (R `0xFF` x 7/15, B `0xFF` x 8/15). Without bit 13 the
  texture drew opaque. Clearing TEX2 bit 13 made no difference here (it
  may matter for other combine modes).
- **Depth compare and write live in the TSP's low bits** (as
  `glDepthFunc`/`glDepthMask` store them): bits 11-9 the mode (PowerVR2
  numbering: 1 less, 4 greater, 6 greater or equal, 7 always), bit 7 set =
  don't write. With writes on, blue at z 0.8 "greater" covered red at
  0.5, blue at 0.3 stayed behind, "less" never passed against the
  background at 0.1: **larger z is nearer**. Dell's `0x0E80` (always, no
  write) is why version 22 saw no writes. The ISP's top bits were set to
  the same mode here; which of the two fields the core obeys is a detail
  for the renderer's tests.

## Implementation (2026-10-15, milestones 1-2: `drivers/mbx3d-render`)

`rp-mbx3d-render` builds D1 (the direct path; the TA path for games comes
later), D2, D3 (the timer fallback), D4, D5 and D8, host-tested against a
model of the 3D core (its `model` feature, every behaviour above that the
device showed; anything unmeasured panics):

- **A retained scene of up to 64 quads** (flat, Gouraud, textured in the
  four decoded formats with wrap/mirror/clamp, blended with bit 13, depth),
  each written into the parameter memory only when its words change (a
  hash per slot); slot order is draw order.
- **Per-tile lists, shared:** each rendered tile's list holds the
  background object and only the quads touching it; tiles with the same
  quads share one list, kept across frames (128 kept; a quad changing
  kind drops the lists naming it; the space is reclaimed between frames,
  when nothing references it). A full screen of large quads needs a
  handful of lists, not 2400.
- **Buffer age:** a frame into the picture rendered two frames ago adds
  the tiles it missed; an unknown picture gets everything.
- **Costs (host model):** a frame after the first is the start write, one
  status read at the predicted time and its clear; a moved 200 x 200 square
  costs 34 quad words and 2 words a tile (351 tiles), no new lists.
- **No floats:** positions in 1/16 pixel, depth and texture coordinates
  fixed point, the core's float words built with integer operations,
  rounded exactly as a conversion would (checked against it).
- **Rules the code keeps:** a rendered tile is written whole, so the scene
  has a background (on by default) or must cover what it renders; one
  renderer drives the core at a time (a new one resets the core first); its
  resets reset the whole MBX, so the 2D engine is idle then and restarts
  its stream (`Engine::after_core_reset`, `core_resets()`).

New on the device with the first run of `rp-mbx3d-api` (owner's go):
tiles pointing at different lists, objects and lists beyond the parameter
memory's first 4 KiB, the ADR-0027 placements (parameter memory at 1 MiB,
the manager's block at `0xFC000`, the region array and textures in the
pool), and the 3D clock off and on again between renders.

**`rp-mbx3d-api` run 1 on the device (2026-10-15,
`local/rp-mbx3d-api-run1.txt`): 92 checks, 88 passed.** S1-S4, S6, S7 as
version 19-23 found them, through the API: dirty tiles (unlisted ones
untouched), every sub-rectangle offset, both alpha results, textures
texel-exact with all three address modes, the clock gate off and on;
renders of plain tiles matched the prediction to the microsecond, a frame
after the first cost one register write. **The 4 failures were depth:** the
background didn't write depth, and "greater" failed wherever only it lay
under a quad: **a tile's depth is not cleared by `0x6DC = 0`** (version
23's background wrote depth, so it never showed). Fix: the background
writes depth (z 0); the model now starts tiles' depth undefined while
`0x6DC` bit 0 is clear, and a test guards it. Also: with real objects the
render took longer than 16 us + 5.2 us a tile (a 10 x 10 block under one
quad: 541-678 us), so the prediction adds ~1 us a quad a tile and the poll
step is a sixteenth of it; run 2 logs the exact times to calibrate.

**Run 2 (`local/rp-mbx3d-api-run2.txt`): 92 checks, 91 passed** (the
depth fix holds: all three "greater" cases right). Exact render times: ~10
us + 5.25 us a tile, plus per quad a tile ~0.3-0.45 us flat, ~1.1 us
textured and **~4.9 us blended** (a blended quad costs about as much as a
tile: keep glass and fades to the tiles they cover). `predict_us` and
`quad_cost` now fit all eleven frames within 6%, pinned by a test. The one
failure is "less": blue "less" at 0.3 over red at 0.5 showed the floor
(the background, z 0), where version 23 (its floor at z 0.1) showed red. No
compare explains that the floor's z decides it; "less" and "less or equal"
are marked unreliable (the model refuses them; the interface needs only
"greater"), and run 3's stage S8 varies the floor's z, red, blue's write
and a partial render to find out.

**Run 3 (`local/rp-mbx3d-api-run3.txt`): 89 checks, 0 BAD; S8 answered
"less".** Over whole-screen renders "less" and "less or equal" **never
passed**, whatever the floor's z (0, 0.1, 0.6: even blue at 0.3 over a
0.6 floor stayed hidden) and with or without depth writes; red under them
always stayed. The floor's z was a red herring: what hid red in runs 1-2
was the **partial render** (only blue's tiles), the one S8 variant that
lost red again; version 23 rendered whole screens. Conclusion: the two
modes don't work as we drive the core (the PowerVR2 family uses "greater"
for nearer; whatever "less" needs is unknown), and they can damage what's
under them. The renderer now **refuses every depth mode the device hasn't
shown working** (`DEPTH_MODES`: greater, greater or equal, always;
`Error::Unsupported`), so no client can reach the failure; the interface
needs only "greater". S8 is removed; S5 checks the refusal.

**Open, found while modelling:** "less" never passed on the device, not
even blue at 0.3 over red at 0.5 with writes on (version 23, T7b), where it
should have; "greater" behaved exactly. The model keeps it as measured.

## Next device tests (static reading 2026-10-15, `docs/hw/2700g.md` "The 3D core's interrupt, filtering, mipmaps and texture formats")

The GPU facts that shape the OS's design, before any interface design
(owner, 2026-10-15). One `rp-mbx3d` version (24) runs them all, on the
owner's go:

| Test | Question | Decides |
|---|---|---|
| T10 | Render done on GPIO85 with `0x130` bit 15 set (Dell's `0x87F5`; T2 had bit 2 alone after a reset), every other source masked and cleared first; which `MINTENABLE` bit 0-3 routes the MBX core | D3: sleeping waits instead of the timer |
| T11 | Depth kept across renders: `0x674` a buffer, `0x610 = (tiles - 1) << 7 \| 2` (store), then `\| 1` (load) on the next render | depth for partial renders and games |
| T12 | Bilinear filtering (TEX1 bit 31) and mipmaps (TEX1 bits 30-29, LOD bits 28-25), the level layout (smallest or largest first) | interface image quality when scaled |
| T13 | PVRTC 4 bpp (TEX0 code 28) and 2 bpp (code 16) from known blocks; 32-bit texels (code 12) | the 64 MB budget: 4-8x less texture memory |

**`rp-mbx3d` version 24 on the device (2026-10-15,
`local/rp-mbx3d-run24.txt`; the image as first built, `6f6557e`: T10
counted edges through GPIO85's interrupt, the reviewed polling-only form
came after the run).**

- **T10, render done reaches GPIO85: yes.** With the line quieted
  (`0x130` 0, all status cleared, the display's events off), a full-screen
  render set status bit 2 at 12,484-12,492 us and GPIO85 edged **at the
  same microsecond**, once, with `0x130` = `0x8004` or Dell's `0x87F5`;
  **none with `0x0004`** (T2's form: bit 15 is the MBX core's master
  enable). `MINTENABLE` `0x8004` and `0x800F` edged, `0x8001`, `0x8002`,
  `0x8008` and `0x000F` didn't: **bit 2 routes the MBX core, bit 15 is the
  master**. The level never read low between samples (a pulse) and was
  high after the clear. **3D waits can sleep on the interrupt**: built
  (`Renderer::set_interrupt`).
- **T11, depth across renders: not stored as tried.** With `0x674` a
  buffer and `0x610 = 15 << 7 | 2`, render A left all 2048 words at the
  fill (`0xAAAAAAAA`); render B drew blue everywhere, with and without the
  load bit. Open (static reading next: what else the GLES scene routine
  sets with a depth buffer).
- **T12, bilinear: works.** TEX1 bit 31 turned the checker's step into a
  ramp `0xFF3F3F`, `0x7F7F`, `0xBFBF` at pixels 106, 108, 110 (texel 0's
  centre at 104, texel 1's at 112): **weights linear between texel
  centres, the pixel sampled at its corner (not its centre), each channel
  truncated**. Built (`Texture::with_linear`; the model matches the run's
  pixels exactly). **Mipmaps: not engaged.** Bit 29 drew the base level at
  every size (64 to 8 px), in both layouts, at every LOD value 0-15;
  **bit 30 alone hung the core** (recovered). Open (static reading next:
  the upload's level layout and what else enables levels).
- **T13, 32-bit and PVRTC: work.** Code 12 decoded ARGB8888 (red, green,
  blue exact; drawn opaque, its alpha in blending untested). **PVRTC
  4 bpp (code 28) and 2 bpp (code 16) decoded** solid red blocks exactly;
  the words swapped drew black (modulation word first, confirmed);
  half-red, half-green blocks drew red left, green right (the samples fit
  both twiddled and row block order: open). Built (`upload32`,
  `upload_pvrtc` for blocks compressed on the PC).

**Next: `rp-mbx3d` version 25** (static reading in `docs/hw/2700g.md`,
"Depth buffers, mipmap levels and the target format"): T11b depth stored
with the GLES driver's full register set (`0x648` and `0x6D0` 1, `n` 16
and 2400); T12b mipmaps with TEX0 bits 12-9 set; T13b PVRTC's block order.

**`rp-mbx3d` version 25 on the device (2026-10-15,
`local/rp-mbx3d-run25.txt`).**

- **T12b, mipmaps: work.** With TEX0 bits 12-9 non-zero (3, 6, 7 or
  `0xB`: all alike) and TEX1 nearest-mipmap (bit 29), **layout B** (the
  base first, each level after the larger one) drew the 16 x 16 level on a
  16 px quad and the 8 x 8 on an 8 px quad (a 64 x 64 texture); layout A
  (the PowerVR2's, smallest first) always drew the base. Version 24 left
  bits 12-9 at 0 and drew only the base: **bits 12-9 are the highest level
  index allowed**. Built (`upload_mipmapped`: levels made on the CPU by
  2 x 2 averages, integer only; both models select levels as measured).
- **T13b, PVRTC's block order: twiddled like texels** (x in the odd bits):
  each block's centre showed row 0 = blocks 0, 2, 8, 10, row 1 = 1, 3, 9,
  11. Both models had assumed it; now measured.
- **T11b, depth stored across renders: still nothing** with the GLES
  driver's full register set (`0x648`, `0x6D0`, the rest), `n` 16 or
  2400, load bit on or off: the buffer untouched, B drew everywhere. The
  GLES scene routine (`0x0225e5c4`), with a depth buffer, also sets **bit
  19 of the tile accelerator's configuration word** (`0x85C`, ours never
  written on the direct path) and `0x610`'s store bit: depth storage may
  belong to the TA path. **Parked** until the TA path is built for games
  (ADR-0026 D7); then one try with `0x85C` bit 19 settles it. The
  interface doesn't need it (each render's depth starts from the
  background).

**Next: `rp-mbx3d` version 26** (static reading in `docs/hw/2700g.md`,
"The MBX cores' clock"): S26a render and fill times at `MBXCLK` divide-by
2 (WM's), 3 and 4 (all within Dell's 75 MHz cap; divide-by-1 exceeds it
and waits for the owner), S26b (T4) a 16-bit target (`0x650` 1 or 3,
`0x660` in pixels or words), S26c the PLLs read (last).

**`rp-mbx3d` version 26 on the device (2026-10-15,
`local/rp-mbx3d-run26.txt`).**

- **S26a, the cores are clock-bound.** At `MBXCLK` divide by 2, 3 and 4:
  2400 tiles 12,484 / 18,724 / 24,968 us, 600 tiles 3,132 / 4,692 / 6,256
  us, a full-screen 2D fill 6,180 / 9,268 / 12,360 us: **exactly in
  proportion to the divider**, 3D and 2D alike. Every MHz of the cores'
  clock is speed.
- **S26c, the clocks:** `COREPLL` `0xBB1` = 13 MHz x 23 / 3 = **99.67 MHz**
  (mbxfb's own "100 MHz" setting), `DISPPLL` `0xEC5` = 23.56 MHz (the
  pixel clock), `PLLSTAT` 7 (both locked), `SYSCLKSRC` 2, `MEMCLK` 1,
  `M24CLK` 2. So **Windows Mobile runs the MBX cores at 49.8 MHz**, a
  third under the 75 MHz Dell's driver allows them; **Dell's default
  "CoreClock" is 133 MHz** (`0x021220f4`), which at divide by 2 gives 66.5
  MHz. Dell's driver sets the PXA's bus timing (`MSC1`) for the 2700G by
  the same clock: one setting for 99 MHz and up, which 133 MHz shares.
  Next: the core PLL stepped up, `rp-mbx3d` version 27.
- **S26b (T4), a 16-bit target: works.** `0x650` 1 wrote RGB565 (red
  `0xF800`, green `0x07E0`, blue `0x001F`, white `0xFFFF`), 3 wrote
  ARGB1555 with the alpha bit set (`0xFC00`, `0x83E0`, `0x801F`); tiles
  stay 8 x 16; **`0x660` is the stride in bytes / 4** in every format (240
  for 960 bytes; 480 skipped every other line). The 16-tile block took 120
  us, no faster than 32 bits as measured (a cost per tile, not per
  byte). Built: `PixelFormat` on the renderer's `Picture` (both models
  write 16-bit targets as measured). The interface's screen pictures stay
  32-bit (ADR-0023's choice, for exact 2D blending and dithering); making
  them RGB565 is an ADR-0023 change for the owner (half the memory and
  bus traffic of every picture; the 2D engine's blends onto 565 need
  their own device check).

**Next: `rp-mbx3d` version 27:** the core PLL stepped from 99.67 MHz to
110.5, 121.3 and 133.25 MHz (Dell's default "CoreClock"; the cores at
half, at most 66.6 MHz, under Dell's 75 MHz cap; Dell's bus-timing tier
unchanged), changed as Dell's driver does it (system clock on the 13 MHz
reference, PLL written, lock awaited), each step checked (a 256 KiB
memory pattern, a 2D fill, ten 3D renders matching WM's pixels) and
timed; WM's PLL back on any failure and at the end. 149.5 MHz waits for
the owner.

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
