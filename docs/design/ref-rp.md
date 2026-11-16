# ref_rp: Half-Life's renderer on our 2700G driver (design, 2026-11-16)

Readiness line 8 (docs/design/half-life-readiness.md), layer 1 of the
float plan (docs/design/apps-and-packs.md, "Floating point in the
ports"), R282. ADRs it builds inside: ADR-0026 (our own 3D renderer: D1 C,
games through the tile accelerator; D6 one GPU owner; D7 layer 3, native
ports straight on our driver), ADR-0038 (the three game routes), ADR-0039
(the renderer is inside the engine module, rule 5; the port layer owns
the hardware, rule 1), ADR-0022 (the graphics stack). A design, not a
result: nothing here works until the X51v shows it.

**Ruled by the owner, 2026-11-16: "Yes, as recommended"** (R287): choice 1
B (grow from the software renderer's scene code), choice 2 A first (the
surface cache on the 2700G, 2 B and 2 C measured beside it, the faster
kept), choice 3 (the tile accelerator, modelled in the emulator: R288).

## Where the frame goes today (run 7 on the X51v, 240 ms, 4.1 a second)

Run 7's play samples (`target/xash-kit/samples.bin`, 58,201 of them),
each float helper's sample charged to the function that called it (our
assembly helpers keep the return address), split by where the code lives:

| Part | Share of the frame | About |
|---|---|---|
| The software renderer (`ref/soft`, inside the engine module) | **71.2%** | 171 ms |
| The engine (game physics, sound mixing, network code, the client's frame) | 18.2% | 44 ms |
| The runner (the sound stream's writes and waits) | 5.3% | 13 ms |
| The game's client module | 3.4% | 8 ms |
| The game's server module | 1.8% | 4 ms |

Inside the renderer: the span drawing `D_DrawSpans16` with its floats
27.5%, the blit `R_BlitScreen` 7.0%, the edge clipping (`R_ClipEdge` 3.7%,
`R_RecursiveClipBPoly` 3.6%), the depth spans 3.5%, the world's vertex
projection (`R_SetupFinalVert` 3.0%, `TransformVector` 2.3%), the models'
points and lighting (`R_StudioDrawPoints` 2.3%, `R_StudioLighting` 1.8%,
`R_AliasProjectAndClipTestFinalVert` 2.0%), the triangle interface's
per-vertex calls (`TriTexCoord2f`, `_TriColor4f`, `TriColor4ub`,
`TriVertex3f`: 4.4%) and its triangle rasteriser (`R_DrawTriangle` 1.7%).

**What that means for 60 frames a second, said plainly.** Read as shares
of a 240 ms frame, a renderer that cost nothing would leave about 69 ms.
**Run 8 measured it instead, and it is much less:** with the 3D view
switched off (`+r_norefresh 1`) the frame is **32.4 ms, 30.8 a second**,
and **49% of that is still the software renderer's blit** (`R_BlitScreen`,
which copies the HUD's picture). The game, the engine and the sound take
**about 16.5 ms a frame**: a 60 a second budget. Much of the frame's
"other 29%" was work done a second (the sound, the network code), not a
frame, so it shrinks per frame as frames come faster. ref_rp has no blit.
So layer 1 alone can take the port to about 60 a second **if ref_rp's own
CPU work is near nothing**: that, not the game, is now the frame's limit.

## Step 1 on the X51v (runs 8 and 9, 2026-11-16)

| Run (c1a0, 120 s of play) | Frame | A second |
|---|---|---|
| 7: the software renderer | 240 ms | 4.1 |
| 8: no 3D view (`r_norefresh 1`), the blit kept | 32.4 ms | 30.8 |
| 9: ref_rp step 1 (`rp_emit 1`: the scene code kept, the emitter's words into RAM, no rasteriser, no blit) | **154 ms** | **6.4** |
| 10: as 9, asking for an 8 MiB surface cache (not applied: see below) | 154 ms | 6.4 |

Run 9's view: 467 world faces, about 3,600 triangles, 9,000 vertices and
**69,000 words a frame** (the same counts in the emulator), 86 state
blocks, 5 faces cut by the near plane. At the bus's ~120 ns a word that is
about 8.3 ms of transfer a frame (by DMA, the CPU free meanwhile).

Where run 9's 154 ms goes (its samples, helpers charged to callers): the
**surface cache rebuilt every frame, 37%** (`R_DrawSurfaceBlock8_mip0`
23.2%, `_mip1` 10.9%, `D_CacheSurface` 2.5%): the emitter asks for the lit
texture of every face the BSP walk finds, where the edge renderer asked
only for the faces left in view after its spans hid the rest, and the
cache (3.2 MB by the engine's formula) thrashes; the models' path 15%
(`R_StudioDrawPoints`, `R_AliasProjectAndClipTestFinalVert`,
`R_StudioLighting`, the triangle interface's calls one a vertex,
`R_SetupFinalVert`); the emitter's own world faces 6.2%; the game, engine
and sound about 30%.

**The cause, proven (2026-11-16):** run 10 on the X51v asked for an 8 MiB
cache and measured the same 154 ms, because the software renderer sizes
its cache once at its start, before the command line's `+` settings run
(the override never applied: run 10 tested nothing, said plainly). With
the size now applied when it changes, the runner's heap refuses 8 MiB and
6 MiB (35,968 KiB, 34,114 in use at the map's start). So the emitter now
counts the cache's work instead, which is the same on any clock: in the
emulator **the cache builds 464 of the 467 faces every frame, none of
them because a light changed**: a total thrash, the view's lit surfaces
larger than the 3.2 MB cache.

What it says (results, not decisions): the rasterisers and the blit are
gone from the samples as designed (240 ms to 154 ms); **choice 2 A costs
more than it saves while the cache thrashes**, and in RAM it cannot grow
in this build. Choice 2 A's own design already puts the cache in the
2700G's 16 MB, built once a surface: if that holds the view, the 37%
(about 57 ms) goes, which projects run 9 to about 97 ms, 10 a second (a
projection from the samples, not a measurement). What the view's lit
surfaces take in bytes at their mip levels is the next count, and decides
between 2 A in video memory and 2 B (texture and lightmap, nothing lit on
the CPU) before step 3. The models' path (15%) is next after that, step
4's (vertices as arrays, in fixed point).

## Rule 0

**Xash's own renderers (findings, not the answer):**

- `ref_gl`: OpenGL 1.x. The world's surfaces chained by texture, drawn
  with the texture and its lightmap (two texture units, or two passes
  blended), the lightmaps packed in pages and re-sent when a light
  changes; the models' bones and lighting on the CPU, their vertices
  handed to GL in model space; **the GPU transforms every vertex** (on a
  PC its T&L, on GLES1 phones the driver). Its GLES road on phones goes
  through a GL-to-GLES layer (gl4es, NanoGL).
- `ref_soft` (what runs today): Quake 2's software renderer. The world as
  edges sorted into spans (no pixel drawn twice), each surface's texture
  lit once into a cache (the surface cache) and drawn from there; models,
  sprites, beams and particles through a GL-like triangle interface
  (TriAPI) onto a CPU triangle rasteriser. Floats throughout on a CPU
  without them.
- The Nokia N95 port (a PowerVR MBX like ours, with an FPU) and the
  Dreamcast port (PowerVR2, the same family's ancestor) both run `ref_gl`
  over a GL layer.

**The best way on the 2700G and our driver:**

- The 2700G is a tile-based deferred renderer: it sorts every opaque
  pixel's depth itself before texturing, so drawing order costs nothing
  for opaque surfaces and no pixel is textured twice. The edge sorting
  `ref_soft` spends its time on is what the chip does for free.
- It has no vertex transform: it takes vertices already on the screen
  (x, y, z, 1/w, colour, u, v, floats). The CPU transforms either way;
  what matters is that it does it in integers (layer 1: the driver
  speaks fixed point) and only once a vertex.
- Its limit is fill (a native frame's textured layer measured at 10-12
  ms at Windows Mobile's clock, ADR-0038), and the bus to it (about 120
  ns a word, ADR-0026). A design that draws each pixel once and writes
  fewer words wins.
- The second texture unit (one pass for texture times lightmap) hangs
  the 3D stage today (`rp-glesfeat` M; 2700g.md "the second texture
  unit"): open, not ruled out.

**Ours (the three choices below, ruled by the owner 2026-11-16):**

### Choice 1: which of Xash's renderers ref_rp grows from

| | A: `ref_gl`'s scene code, its GL calls rewritten to our driver | **B: `ref_soft`'s scene code, its rasterisers replaced by the 2700G (recommended)** | C: from nothing |
|---|---|---|---|
| What it already does on the CPU | culling, texture chains, model bones; leaves the transform to a GPU we lack | culling, the world's vertex projection, the models' transform and lighting, and every sprite, beam, particle and decal as screen triangles: the 2700G's input | nothing |
| What runs today | not built for the X51v | built, linked, run 7's exact frame on the X51v | nothing |
| Lighting | texture + lightmap: two passes on the 2700G until the second unit works | the surface cache: one texture a surface, one pass (choice 2) | to design |
| Work | 24,000 lines, every GL call site | about 15,000 lines kept; the rasterisers (spans, edges, polysets, the blit) replaced by one emitter | all of it |

Why B: the 2700G wants screen triangles from the CPU, and `ref_soft` is the
renderer that already makes them on the CPU; what it adds on top (edge
lists, spans, the blit: about half the frame) is exactly what the chip
does itself. `ref_gl` was written for a GPU that transforms, so its CPU
side would have to grow a transform stage anyway. And B keeps run 7's
exact frame as the reference every step is compared against.

### Choice 2: how the world is lit

| | **A: the surface cache on the 2700G (recommended first)** | B: texture and lightmap, two passes (`ref_gl`'s fallback) | C: the second texture unit, one pass |
|---|---|---|---|
| The 2700G's fill | one textured layer | two layers on every lit pixel | one layer |
| Bus | a surface's lit texels sent when it first shows or its light changes | lightmaps only (small), when they change | lightmaps only |
| CPU | lights each new surface once (`ref_soft`'s own code) | none for lighting | none |
| Looks | the software renderer's look, as today | the GL look | the GL look |
| Known risk | the flashlight relights the surfaces it touches every frame | none new | hangs today (open) |

A is first because fill is the chip's limit. Each surface's lit texture
goes into a power-of-two block of a twiddled atlas page in 2700G memory
(a buddy allocator: a power-of-two block of a twiddled page is one run of
memory, so a surface goes over the bus in one write, the CPU's or a
DMA's). B and C are measured beside it on the same map (fill time, bus
words, CPU), and the winner stays; C first needs the 3D stage's two-unit
words found (static reading, then a test image with the owner's go).

### Choice 3: the path to the chip, and the emulator

The tile accelerator (TA), as ADR-0026 D1 C decided for games: the CPU
streams triangles, the chip bins them (P4 on the X51v: 1,000 small quads
as one list by DMA in 7.1 ms with 5.2 ms of it free for the CPU; 0 hangs
in 150 renders since `ta::set_up`). Our CPU binner draws only flat
2D quads (one depth a quad, 1/w always 1.0), so it is not a game path.

**The emulator does not model the TA** (docs/hw/emulator.md: its TA never
says done). For `emu-xash-kit` to check ref_rp's pictures, the QEMU fork
gains the TA: the vertex stream binned into the same objects and lists
the fork's 3D model already draws (the words the TA writes are known from
`rp-glesfeat` and P4). That is the emulator-tracks-device rule, and it
also lets every later port's renderer be checked before the X51v.

**Step 1b (emulator, 2026-11-16; byte counts are the same on any clock):**
c1a0's view needs **4,655 KiB of lit surfaces a frame** (16-bit texels at
the software renderer's mip levels: 203 faces at level 0, 241 at 1, 23 at
2), **7,053 KiB as power-of-two blocks**: half again as much, the blocks'
waste. The 3.2 MB cache cannot hold one view, which is the thrash. In the
2700G's 16 MiB beside two 640 x 480 pictures (1.2 MiB), the tile
accelerator's parameter memory and the models' textures, 4.6 MB fits;
7 MB is tight, and walking through a map needs room beyond one view. So
the atlas wants blocks packed tighter than powers of two (rectangles of
whole twiddled 8 x 8 squares, or shelves of equal heights), and the
level-0 faces (443 of 467 at levels 0 and 1) are where the bytes are. To
measure next with the chip: what the bus's 33 MB/s costs when the view
turns (a new view's 4.6 MB is about 140 ms to send whole; only the newly
seen surfaces go, spread over frames) against choice 2 B's lightmaps.

## What ref_rp is

- **One emitter replaces the rasterisers.** Every surface, triangle,
  sprite, beam, particle and decal `ref_soft` would rasterise is written
  as TA words into a buffer in RAM, state words only when the state
  changes (ADR-0026 D4), and the buffer handed to the TA by DMA in pieces
  the FIFO has room for (P4's fastest way), the CPU going on with the
  next frame meanwhile (ADR-0038 G4: two frames in flight).
- **The world:** `ref_soft`'s BSP walk, culling and vertex projection
  kept; each visible face's projected corners clipped only against the
  near plane (the chip clips the screen edges itself) and written as a
  fan with its surface-cache block's coordinates. Its edge lists, spans
  and blit go.
- **Models, sprites, beams, particles:** the triangle interface's calls
  become array writes into the emitter (no call a vertex), and its CPU
  rasteriser goes.
- **2D (the menu, the console, the HUD):** rectangles through the same
  emitter.
- **Vertices in integers:** the view and each model's matrices made once
  a frame (or an entity) in float, turned to fixed point, the vertices
  transformed with `smull` (layer 1), the TA's floats built from them with
  `clz` (as `rp-mbx3d-render` already does exactly).
- **The picture:** 640 x 480 landscape drawn rotated into the safe
  region's picture (the panel is 480 x 640 upright), through ADR-0038's
  routes: native first, the lines route (half the pixels) and the
  smooth route measured on this game for G2's dynamic resolution.
- **The C face:** the runner (the port layer, ADR-0039 rule 1) owns the
  2700G and gives the engine a handful of calls (`rp_gfx_*`: set up, a
  texture block's room and upload, a frame's buffer handed over, the
  frame shown); ref_rp is C inside the engine module (rule 5) and never
  touches a register (ADR-0026 D6).

## What the device must show first (unknown today)

1. **Perspective through the TA:** every device run so far drew with 1/w
   = 1.0. Whether the TA takes u, v plain or divided by w, and how its
   1/w and z are ranged, is read first from Dell's GLES driver's transform
   stage (static), then shown by a test image (owner's go).
2. **Depth precision** over a Half-Life view (near 4 units, far 4,096).
3. **The TA at the game step** (136.50 / 1, ADR-0035 D): R41.

## The steps, each checked in the emulator, then measured on the X51v

| Step | What | Measured by |
|---|---|---|
| 0 | The floor: the run with no 3D view (`+r_norefresh 1`) | **done:** run 8, 32.4 ms (16 ms of it the blit) |
| 1 | ref_rp's CPU side: `ref_soft` with its rasterisers replaced by the emitter writing TA words into RAM (no chip yet) | **done:** run 9, 154 ms; the surface cache's thrash 37% (proven) |
| 1b | The view's lit surfaces in bytes at their mip levels; choice 2 A (cache in video memory) against 2 B (texture and lightmap) | counts in the emulator, then the frame on the X51v |
| 2 | The TA in the QEMU fork; the facts above on the X51v | `emu-tabench` drawing; a test image |
| 3 | The runner's `rp_gfx_*`, the world on the 2700G with the surface cache | the first picture; frame ms |
| 4 | Models, sprites, beams, particles, 2D | the whole frame on the 2700G against run 7's picture |
| 5 | Fixed-point transforms, choice 2's B and C measured, the routes | frame ms per change |

**The measurement that proves ours wins:** the same map (c1a0), the same
120 s of play, the frame time and the samples against run 7's 240 ms with
the software renderer; every step's picture against the software
renderer's own frame of the same view (the end shot).

## What would overturn this

A TA that cannot draw perspective-correct textures (then the CPU binner
grows 3D objects, measured against it); a surface cache whose bus traffic
on the X51v costs more than the second pass's fill (choice 2 B then wins);
the second unit made to work (C then likely wins for the world).
