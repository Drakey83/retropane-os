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

**Built (R288, 2026-11-16, QEMU fork rp-v0.32.0):** the fork's TA parses
the stream as fed and writes the X51v's own objects, 16-word list blocks
and region array (their forms read from Dell's captured frame:
docs/hw/2700g.md "The TA's output"), which the fork's 3D core then draws
from `0x608` as the chip does. Strips of up to six corners become one
object with a per-tile triangle mask, so ref_rp's faces as strips go in
as the chip takes them. `emu-tabench`'s 30 TA ways now draw exactly the
binner's pictures, as on the X51v (run 9). Still affine: u, v as fed and
1/w unused until R289 measures the TA's perspective.

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

## The 45 a second budget (the owner, 2026-11-16)

"I want us to be clever enough to get at least 45 fps on average": 22.2 ms
a frame, averaged over the run. Run 8 put the game, the engine and the
sound at about 16.5 ms, so both sides shrink: ref_rp's CPU work to about
6 ms (from run 9's ~50 ms once the cache stops thrashing), and the game's
to about 13 ms, while the 2700G draws the last frame beside them (ADR-0038
G4: two in flight). Each line is measured on the X51v when built; none is
a result yet.

**ref_rp's CPU, ~50 ms to ~6:**

- **Nothing a frame that is the same every frame.** A world face's
  texture coordinates depend only on its corners and its texture's axes:
  worked out once when the map loads (per mip level), not per frame as the
  software renderer and step 1 do. A still camera reuses the whole last
  frame's buffer (no CPU at all).
- **A vertex transformed once, not once a face.** The world's corners are
  shared by three or four faces each: transformed once a frame into a
  table (fixed point, `smull`), the faces then only index it.
- **Models as the strips they are stored as.** Half-Life's model meshes
  are already strips and fans; the triangle interface breaks them into
  loose triangles (run 9: 9,000 vertices for 3,600 triangles). Sent as
  strips, about one vertex a triangle: the words a frame roughly halve
  (69,000 to about 35,000, the bus 8.3 ms to about 4).
- **One matrix a bone, in fixed point.** Each bone's matrix times the
  view's, once a frame; each vertex then one 3 x 4 integer transform
  (iWMMXt multiply-adds, layer 3). **Lighting per normal, not per
  vertex:** a model's vertices share a small set of normals; each is lit
  once and the vertices look it up.
- **No call a vertex, no float a vertex:** the perspective divide by a
  reciprocal table and one Newton step (as `hal/mem/src/float.rs`'s
  divide), the TA's floats built by `clz`.

**The game's CPU, ~16.5 ms to ~13** (from run 8's samples):

- **The local game skips the network's delta coding.** In single player
  the server and the client are one program, yet every frame the server
  delta-codes each entity for the client and the client decodes it
  (`Delta_CompareField`, `Delta_WriteField`, `CL_DeltaEntity`): handed over
  as they are when both sides are local (the loopback), exactly the same
  state.
- **The player's collision traces** (`PM_RecursiveHullCheck`,
  `PM_PlayerTraceExt`, `SV_ClipToEntity`) and the doubles (`__adddf3`,
  `__muldf3`: the engine's and the client's) through layer 2's relaxed
  float, each checked against the float build.
- **The sound's mixing** (`S_MixChannelToBuffer`, `S_PaintChannels`,
  `rp_snd_write`) in iWMMXt, or handed to the OS mixer's stream.

## Step 3: the world on the 2700G (design, 2026-11-16)

**Rule 0, written before building.**

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| Lit surfaces | `ref_soft`: a 3.2 MB ring in RAM, rebuilt whenever pushed out (run 9: 464 of 467 faces a frame); `ref_gl`: texture and lightmap, two layers of fill; Dell's GLES: one `glTexImage2D` a texture, copied in rows then twiddled by the CPU (`libGLES_CL.dll`, 2700g.md "No row-order texture mode") | each surface lit once into a scratch buffer and written **straight into a twiddled atlas page in video memory** (512 x 512 RGB565, 512 KiB), packed on shelves of whole 8 x 8 squares, the colour conversion and the twiddle done in that one pass (a 64-entry table a square); kept until its light changes; a full atlas drops its least recently drawn page | the view (4,655 KiB) fits in 14 pages (7 MiB), so a still or slowly turning view lights nothing; shelves of whole squares waste only the rounding to 8 texels where powers of two took half again (step 1b's 7,053 KiB); one pass over each texel, no second copy in RAM |
| Depth | Dell's GLES: z = GL window depth (`1.00029 - 0.96360 / w`, a multiply-add a vertex) compared "less" | z = 1/w itself, compared "greater or equal" (the PowerVR2 and Dreamcast way, now on the MBX) | no work a vertex (the 1/w word is the z word), and a float 1/w keeps its precision far away where window depth crowds it near 1 |
| The screen | ADR-0034 A: draw off screen, the 2D engine copies the picture in (~5 ms) | the 3D core draws **straight into the visible screen at `0x0`** (ADR-0034 option C, sound at every clock in G21: 20 of 20 where off screen failed above 74.75 / 1) | no copy (5 ms of a 22 ms budget); tearing is the known cost, to be seen on the device and moved behind the scan line later |
| Landscape | a rotation pass over the picture (the software renderer's rotated blit, 7% of run 7) | the emitter writes each vertex already turned (`x = sy`, `y = 640 - sx`, the input's `RP_TURN_RIGHT`) | the turn costs nothing: two words swapped a vertex |
| The C face | GL: hundreds of calls, a state machine, a call a vertex | four calls (`rp_gfx_open`, `rp_gfx_frame`, `rp_gfx_wait`, `rp_gfx_close`): the frame's words in one buffer, **checked by the port layer** (only the state block and strips it knows, a strip's corner count in its own word, every texture inside the game's texture memory) before any reaches the TA | one call a frame; a wrong word stops at the check instead of hanging the chip |
| The frame | Dell: the CPU waits for each render | the render runs while the CPU makes the next frame; the CPU waits only if it is still drawing at the next frame's feed (ADR-0038 G4's first half) | the CPU's and the chip's times overlap instead of adding |

**Built and checked in the emulator (2026-11-16, QEMU fork rp-v0.33.0,
`emu-xash-kit` with `+rp_emit 2`):** the 2700G drew c1a0's world every
frame, no frame refused, no TA or render hang. The end shot draws one
frame both ways (the software renderer's picture and the 2700G's of the
same view, `target/xash-kit/emu-last-frame.png` and `emu-gpu-frame.png`):
the 2700G drew 306,297 of 307,200 pixels, **95% of them within a few
levels of the software renderer's**, the rest the models, sprites and HUD
step 4 adds (the scientists behind the desk) and texel choices at faces'
edges. The video-memory cache built 5 surfaces a frame once the view had
settled (24 in the first seconds), against 464 in step 1: the thrash is
gone. Words a frame 18,300 (step 1: 69,000-83,000, the models' triangles
not sent yet). The emulator's frame times are the PC's, not the X51v's.

**Run 11 on the X51v (port-runner-0011, 2026-11-16, c1a0, 120 s of
play, `+rp_emit 2`): 95 ms a frame, 10.5 a second** (run 7, the software
renderer: 240 ms, 4.1; run 9, step 1: 154 ms, 6.4). The 2700G's picture
of the end frame is the world as the software renderer draws it
(`local/half-life/run11-frame-gpu.png`); no frame refused, no TA or render
hang in 120 s. The chip's side: the CPU feeding the TA 4.8 ms a frame
(18,300 words), the TA done 5.5 ms after its start, **a render 24.4 ms**
(at the loader's 2700G clock, `MBXCLK` 0x5), and **the CPU never waited
for a render** (0.0 ms): the 2700G is not the frame's limit yet, the CPU
is. The video-memory cache built 5 surfaces a frame (0.5 ms). Where the
95 ms go (the play's 58,180 samples): the software-float helpers 59%; the
models' path (`R_StudioDrawPoints`, `R_SetupFinalVert`, the lighting and
the triangle interface, with their floats) about a quarter of the frame,
**for triangles step 3 does not draw yet**; the emitter's world faces with
their floats about 11%; the doubles in the engine and client, the sound
(`rp_snd_write` 3.7%) and the game the rest. Read with the 45 a second
budget: the render's 24.4 ms alone is past 22.2 ms at this clock, so the
2700G's faster clock (divide 1 at 108.33 MHz, measured 2.17x Windows
Mobile's) belongs in the plan beside the CPU's lines.

**A correction to step 1b, found here:** the software renderer's cache
stores a surface's height as its bytes over its width (`D_SCAlloc`:
twice the rows of 16-bit texels), so step 1b's sums were doubled: **the
view's lit surfaces are about 2,327 KiB (3,526 KiB as power-of-two
blocks), not 4,655 (7,053).** Counted now from the width and height
themselves. The atlas's 7 MiB holds three such views.

What step 3 leaves for later (its own lines in the steps table): models,
sprites, particles, the sky, water and 2D (step 4); the feed by DMA in
pieces (P4's fastest way; the CPU's stores first, timed); depth precision
at the far plane, seen on the device first.

## Step 4: the models on the 2700G (design, 2026-11-16, the owner: "Yes, start step 4")

Run 11's samples put about a quarter of the 95 ms frame in the models'
path, for triangles step 3 throws away. Read in `ref/soft/r_studio.c`:
the bones move each vertex once into `g_studio.verts` (world space);
`R_StudioDrawNormalMesh` then walks the mesh's stored commands (each a
strip or a fan, `ptricmds`: vertex, normal, s, t) and hands every vertex
to the triangle interface: `R_StudioSetColorBegin` lights it again for
every command it is in (`R_LightLambert`, floats), `TriTexCoord2f`,
`TriVertex3fv`, then `R_SetupFinalVert` projects it, again per use, and
the triangle rasteriser draws loose triangles.

**Rule 0.**

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| Geometry | `ref_gl`: one GL call a vertex, the GPU transforms; `ref_soft`: the triangle interface, one call a vertex, loose triangles (run 9: 9,000 vertices for 3,600 triangles) | each stored command sent as **one TA strip or fan** (the TA's own fan, `0x5`, beside the strip): a vertex word a corner, no call a vertex | about one vertex a triangle instead of three: the words and the projection work roughly a third |
| Projection | per use, in float (`R_SetupFinalVert`) | each of the model's vertices **projected once a frame** into a table (view space, 1/z, the turn), the commands then only index it | a vertex shared by 4-6 triangles is worked once |
| Lighting | per command vertex (`R_LightLambert` again for every use) | **once per normal index** a frame (a model's few hundred normals), looked up by the commands | the lighting's floats fall by the ratio of uses to normals |
| Skins | `ref_soft`: 8-bit skins in RAM read through a palette per pixel; `ref_gl`: uploaded at load as power-of-two textures, resampled | each skin **written once at load into the same video-memory atlas** (its own size rounded to 8 x 8 squares, RGB565 through the palette, twiddled in that pass), the skin's s, t mapped into its page | no resampling (the skin's own texels), no waste to powers of two, the atlas's one allocator; gouraud light from the vertex colour (TSP bit 22) |

**The sub-steps, each checked in the emulator (the end shot drawn both
ways), then a run on the X51v:**

| Step | What | Measured by |
|---|---|---|
| 4a | Skins in the atlas at load; each stored command as one TA strip or fan from the existing per-vertex numbers (the picture first) | the models in the 2700G's picture; frame ms |
| 4b | Each vertex projected once, each normal lit once (tables a model a frame) | frame ms, the models' share of the samples |
| 4c | Sprites, beams, particles, the sky, water and conveyors; 2D (the HUD, the console) as rectangles | the whole frame against the software renderer's picture |
| 4d | Fixed point for the bones and the projection (one 3 x 4 integer transform a vertex), the emitter's world corners transformed once and their texture coordinates made at map load | frame ms against the 45 a second budget |

What would overturn it: a TA fan that does not draw as the strip does
(then fans go as strips: a fan of n corners is n - 2 triangles either
way, a few more words); skins that wrap (s, t past the skin: then that
skin gets a power-of-two texture of its own with wrap).

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
| 3 | The runner's `rp_gfx_*`, the world on the 2700G with the surface cache | **done:** run 11, 95 ms (10.5 a second); the picture right on the X51v (models to come) |
| 4 | Models, sprites, beams, particles, 2D (sub-steps 4a-4d above) | the whole frame on the 2700G against run 7's picture |
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
