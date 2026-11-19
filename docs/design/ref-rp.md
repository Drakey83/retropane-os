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
| 4a | Skins in the atlas at load; each stored command as one TA strip or fan from the existing per-vertex numbers (the picture first) | **done:** run 12, 92 ms (10.8 a second); the models in the X51v's picture; the TA's fan proven |
| 4b | Each vertex projected once, each normal lit once (tables a model a frame) | **done with 4d's projection:** run 13, 66.5 ms (15.0 a second) |
| 4c | Sprites, beams, particles, the sky, water and conveyors; 2D (the HUD, the console) as rectangles | the whole frame against the software renderer's picture |
| 4d | Fixed point for the bones and the projection (one 3 x 4 integer transform a vertex), the emitter's world corners transformed once and their texture coordinates made at map load | **done:** projection run 13 (66.5 ms); the bones and model lighting run 14, 62.8 ms (15.9 a second); brush entities by the tables run 15, 56.6 ms (17.7) |
| 4e | The 2700G's clock while the game draws (ADR-0035's ladder, `-rpclock`) | **108.33 / 1:** run 16, 54.9 ms (18.2 a second), the render 26.2 to 13.2 ms, the end frame pixel for pixel run 15's |

**4a built and run (2026-11-16).** `r_studio.c` hands each mesh to the
emitter under `rp_emit 2` (`RP_StudioMesh`, the lighting
`R_StudioSetColorBegin` would do, in a command's array, then
`RP_StudioCmd` once a command); `r_emit.c` writes each skin at its first
use into the atlas (`RP_StudioSkin`: RGB565, or ARGB1555 with the alpha
test where a skin has the software renderer's see-through texel; kept
against the page's generation, written again if its page was dropped) and
each command as one strip or one TA fan (`RP_GFX_FAN`, new in the port
layer's check: `ports/sys/src/gfx.rs` `FAN`, 3-255 corners as a strip's),
pieces of 255 corners at most, corners turned as `RP_ChipFace`'s, the
light's own colour Gouraud shaded and multiplied into the texel (the
software renderer kept a grey in 32 steps; `ref_gl`'s look). `rp_fan 0`
sends fans as strips, unused. A command with a corner behind the near
plane was not drawn (counted: 0 in c1a0's start view, 394-467 a frame up
close in run 35); since run 36 (R299) it is clipped: its triangles wholly
in front go on as runs, a triangle across the plane is cut in 16.16
integers into a 3- or 4-corner strip. Blended render modes are 4c's.
**Chrome is drawn (the owner's ruling, 2026-11-16: "Dont do that", no mesh
skipped):** the guards' helmet tops (barneyt.mdl `BX_Chrome1`, chrome and
flat shade, 40 triangles) were missing under the integer path. A chrome
mesh now goes as every mesh, its texels `R_StudioSetupChrome`'s in
integers (each bone's vectors 2.30 once an entity a frame, two integer dot
products a normal: `RP_StudioChromeFixed`), run 37. **Proven by the owner's
eye on run 37: "the helmet looks correct now".** The owner's ruling the
same day: **draw everything, nothing skipped**: blended models drawn
since (see-through by the model's alpha, additive 1 + 1, depth not
written: the measured `blend`/`depth_test` words).

**Kit 40, toward 45 a second (the owner, after run 39: "work longer on
these big changes"):** run 39's samples (`local/half-life/run39-profile.txt`)
named the costs; each lever's rule 0:

- **The models' culling** (`RP_StudioClipCmd`): models are 85% of the
  frame's words and the bus is the feed's cost. ref_gl leaves back faces
  to the GPU (GL_FRONT culled), the software renderer draws them; the
  2700G culls only after the bus carried them. Ours drops them on the
  CPU from the TA's own integer x, y, Half-Life's GL winding, plus
  triangles wholly off the screen, keeping gaps of one or two triangles
  inside a strip (cheaper than a restart). `rp_cull` 0 / -1 for checks.
- **The feed's software per word** (`Feed::put_words`, `send_runs`): run
  39 paid 234 ns a word against the bus's ~133; now a run of words is one
  tight loop, the FIFO's room read once a chunk. DMA was weighed and set
  aside (R292: no faster than the CPU's stores, and the CPU would have to
  start a piece every ~70 us).
- **No double physics**: `cl_nopred 1`, `cl_lw 0` (one device, the server
  runs the input in the same host frame: Host_Frame's order).
- **The mip hold** (`RP_MipHold`): a face keeps its level until a quarter
  past the line, so the atlas's churn (run 39: `RP_SurfacePlace` with its
  uploads 4.2%, the CPU lighting 2.4%) stops while standing or strafing.
- **The float compares that weren't** (`rp_fixq`, `RP_Fix16`, `RP_Fix30`):
  the compiler turned bit tests back into `__aeabi_fcmpeq` calls (1.6%,
  seen in the disassembly); an empty asm hides the bits' origin.
- **The client's skeleton** (`ConcatTransforms`, hlsdk patch): the SDK's
  floats (NEON where there is one) as integers, rotation 4.28 and offset
  16.16, floats made back from their bits; the PC's check over 200,000
  random bone pairs: within 2.4e-7 and 0.0005 units, out-of-range values
  back to the floats.
- **The black walls' witness** (`RP_HoleCheck`): the clear colour one step
  off black, sampled every report, the first three frames with holes kept.


In the emulator (`emu-xash-kit`, rp-v0.33.0): the 2700G's end shot has
the scientists behind the desk as the software renderer's does, 95% of
the pixels within a few levels. **Run 12 on the X51v (port-runner-0012,
c1a0, 120 s, `+rp_emit 2`): 92 ms a frame, 10.8 a second** (run 11: 95
ms, 10.5, the models not drawn). 63 meshes a frame as 489 strips and 70
TA fans: **the TA's fan draws on the X51v** (the first ever sent: no
frame refused, no TA or render hang); the picture of the end frame has
the models (`local/half-life/run12-frame-gpu.png`). The frame's words
42,900 (18,300), the CPU feeding 10.0 ms (4.8), a render 26.1 ms (24.4),
the CPU never waiting for it. Samples of the play (58,173): the
software-float helpers 59%, `RP_EmitFace` 4.4%, `RP_StudioCmd` 3.2%,
`TransformVector` 1.7%; the models' lighting and projection are still
done again for every use of a vertex, 4b's to remove.

**4b with 4d's projection built and run (2026-11-16).** Rule 0 in
`r_emit.c` ("Steps 4b and 4d"): the view's rotation as 2.30 integers once
a frame, vertices as 16.16 (the world's at map load, a model's from its
float by bit operations), moved into view space with 64-bit multiply-adds
and projected with one reciprocal (a 128-entry table and two Newton steps,
no divide), the TA's floats by `clz` and 1/z's float straight from the
reciprocal's bits; each world vertex and each model vertex worked once a
frame into a table the faces and commands index; the world's texture
coordinates made at map load (24.8 by surfedge); each model normal's
colour once a part when no local light is on. A corner nearer than a unit
takes the float path (`rp_fixed 0` takes it everywhere). Checked on the PC
against the float path over 100,000 random points: at most 0.07 pixel off
on the screen, 1/z within 8 parts in 100,000, the reciprocal within one
part in a billion. **Run 13 on the X51v (port-runner-0013, c1a0, 120 s):
66.5 ms a frame, 15.0 a second** (run 12: 92 ms, 10.8); the end frame's
picture as run 12's (`local/half-life/run13-frame-gpu.png`); 372 world
faces a frame by the tables, 95 by floats (brush entities and near
corners); model vertices 1,199 worked and 2,115 used again a frame. The
software-float helpers fell from 59% of the play to 46%; what is left of
them is mostly the models' bones and lighting (`R_StudioDrawPoints`,
`R_StudioLighting`, `R_LightLambert`) and the client's skeleton
(`ConcatTransforms`, quaternions): 4d's next part. Render 26.2 ms, the CPU
feeding 10.0 ms, sound 5.3%.

**4d's rest, brush entities and the clock (2026-11-16, runs 14-16).**

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| Bones | `ref_soft`, `ref_gl` (and 4b): each vertex by its bone's float 3 x 4 into world space, then by the view again; on the PXA270 each multiply and add a software-float call | each bone **joined with the view once a part** (`ports/xash/ref_rp/rp_fixed.h`: 4.28 rotation, 16.16 offset in view space), a vertex from the model's file straight to view space in nine `smull` into the table RP_StudioCmd indexes | one integer transform a vertex where there were two float ones, no world space at all |
| Model light | `R_StudioLighting`: the hemispherical Lambert in floats with a divide, the gamma table, then `R_LightLambert`'s float scale to bytes | an integer dot (2.30 x 4.28), two multiplies by the Lambert's constants, the engine's own gamma table, straight to the colour bytes (no local light; with one, the float path) | no float and no divide a normal |
| Brush entities | `ref_soft`: a door or lift is a model with the view turned into its space, every corner by floats every face | the world's own tables (a submodel shares the world's vertices and corners), in the entity's view (R_RotateBmodel's rows as 2.30, the eye at `tr.modelorg`), a vertex worked once a view (stamps by view serial) | the 82 brush faces a frame (c1a0) cost what the world's do |
| The 2700G's clock | Windows Mobile: 99.67 MHz, the cores at divide 2 (49.83 MHz), always | the screen's open steps to a rung of ADR-0035's ladder with its own step code, the close puts Windows Mobile's back, the underrun flag checked both times (rule 8); a rung past the owner's game step (136.50 / 1) is refused | the render's time halves at 108.33 / 1 (2.17 x the cores' clock) |

PC check (gcc, `rp_fixed.h` itself against doubles over 100,000 random
bones, points and normals): view space within 0.0003 units, the screen
within 0.008 pixel, the light's gamma index one step off in 3 of 100,000.
**Run 14 (port-runner-0014): 62.8 ms a frame, 15.9 a second** (run 13:
66.5). Its samples showed `memcpy` at 1.8%: the build is `-ffreestanding`,
so the float-to-bits `memcpy` in every fixed-point helper was a call; now
`__builtin_memcpy` (inline) in all four. They also showed parts with a
chrome mesh kept on floats; chrome was then skipped under the integer
path (undone 2026-11-16 by the owner's ruling: chrome is drawn, above). **Run 15 (port-runner-0015): 56.6 ms, 17.7 a
second**: 454 of 467 world faces a frame by the tables (82 of them brush
entities'), 13 by floats (corners nearer than a unit); 8 model parts, 67
bones and 1,806 normals a frame in integers. The software-float helpers
46% of the play in run 13, 37% now; what is left of them is the player's
traces (`PM_RecursiveHullCheck`, `PM_PlayerTraceExt`), the client's
skeleton (`ConcatTransforms`, quaternions), the doubles and the sound.
**Run 16 (port-runner-0016, `-rpclock 25 3 0 1`): 54.9 ms, 18.2 a
second**; the 2700G at 108.33 / 1 (the PLL locked at once, no write
rewritten, the underrun flag clear before and after both changes); **a
render 13.2 ms** (26.2 at Windows Mobile's), the TA done 9.8 ms after its
start (10.7), the CPU feeding 9.4 ms (10.0); no frame refused, no TA or
render hang; the end frame's picture **identical pixel for pixel** to run
15's at Windows Mobile's clock (one frame: R41's question for the TA path
at divide 1 is answered for this rung on this frame only). The render is
now well inside the 22.2 ms budget: the frame is the CPU's.

**Run 17 (port-runner-0017, `-rpclock 21 2 0 1`, the owner's game step
136.50 / 1): glitched to the owner's eye.** A render that finishes takes
10.7 ms, but about one in four never finishes (26 render hangs in ~90
frames, the core reset after 200 ms each; no TA hang, no frame refused),
so the frames ran 84-190 ms; the end frame that drew is 99.8% run 16's.
The game step was proven on the CPU binner's path with the divide-1 cures
(ADR-0035 D); the TA path has none, and its divide-1 behaviour past 108.33
was never measured (R41). The kit's default is 108.33 / 1 (clean, run 16)
until the TA path runs 136.50 cleanly (R295, the owner: "we need to make
it so 136, can be ran cleanly").

**Runs 18 and 19 (R295, the owner: "we need to make it so 136, can be
ran cleanly"; `-rpladder`, [the sheet](../research/sheets/r295-ta-path-136.md)).**
One run each, the runner stepping the 2700G's clock and the TA path's
layout and render words cell by cell, every render waited for and
counted. **The TA path's edge lies between 133.25 / 1 (1 hang in 240
renders) and 136.50 / 1 (a third of renders, in bursts that follow the
view); 108.33 and 117.00 / 1 drew every render.** No lever cured
136.50: the region array, manager block and tail on the die, Dell's GLES
render words (all, or all but any one), the parameter fetch's settings
(`0x61C`). The memory timing word is the same at 133.25 and 136.50; the
suspect left is the core's reads of the TA's lists in SDRAM (the game
step was proven with the CPU binner's lists on the die). The clock does
not set the frame rate today (the CPU's ~55 ms against a 13.2 ms
render); the options are in the sheet for the owner.

**Levers 2 and 3, the maths and the sound's loops (runs 20 and 21,
2026-11-16, 108.33 / 1).**

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| `sin`, `cos`, `sinf`, `cosf` | musl's (Rust's `libm` in `compiler_builtins`): `sinf` widened to doubles, `sin` correctly rounded doubles, every step a software-float call | `hal/mem/src/trig.rs`: the argument times a 96-bit window of 2/pi's bits (one `u128` product, exact at every exponent), the quarter turn split off in integers, two degree-4 Q30 polynomials fitted offline to this exact evaluation (`tools/numerics/fit_trig.py`, at most 1.2e-9 off before rounding), the other function of the same argument kept for the next call | no software double at all; the game's `sin(a)` then `cos(a)` costs one reduction |
| Literals | C: `0.5` is a double, so every float expression holding one is widened term by term | `-fsingle-precision-constant` on the game's sources (no literal needs a double's range: checked) | the client's and engine's doubles from literals gone |
| Float compares | `compiler_builtins`' generic `__eqsf2`/`__lesf2` through every case | `float::compare`: two shifts for the NaNs, one signed compare on the bits (host test: every pair of 6,015 values against the PC's own compare) | a compare is a handful of instructions |
| The sound's resampling loops (`s_mix.c`) | Xash: a double phase stepped every sample (Quake's own mixer used integers) | a 32-bit phase with one carry a sample, the neighbours blended in Q16; `chan->sample` still advances in doubles once a call, so nothing drifts | no software double a sample |

**Run 20 (sin/cos, literals): 49.7 ms, 20.1 a second** (run 16: 55.0,
18.2); the end frame 99.97% run 16's (78 pixels: the view's maths now at
a float's precision). The client's `__muldf3`/`__adddf3` left the
samples. **Run 21 (+ the sound's loops, the compares): 48.0 ms, 20.8 a
second**; the engine's `__adddf3` left the top of the samples. Left on
top (run 21): `rp_fadd_asm` 6.5% (the player's traces, the emitter),
`RP_ChipFaceFixed` 5.6%, `rp_snd_write` 5.3% (the OS mixer's general
path for one stream, R296), `rp_fmul_asm` 3.9%, the feed (R292).

**Step 4f, the feed by DMA: designed, not built.** The CPU spends 9.4 ms
a frame writing 42,900 words into the TA window. `rp-tabench` (P4) moved
one unbroken list by DMA in pieces the FIFO had room for, the CPU polling
the FIFO between pieces: faster than the CPU's stores but not free. Our
frame ends every strip with a write to the end window (1,026 a frame), so
pieces would be one strip long unless the ends ride in a descriptor chain
(PXA27x DMA: a descriptor for each strip's words, one for its end);
and a DMA write into a full FIFO holds the shared bus (the CPU's own
memory then waits), which is why tabench paced by room. The win is the
CPU free while the chip is fed, which needs the frame's words double
buffered (the emitter making frame n+1 while frame n is fed) and the feed
started rather than waited for: ADR-0038 G4's second half. Questions for
the device before building: whether the TA takes a frame's strips with
their ends in a chain at the bus's pace without a FIFO check (Dell's GLES
lets the bus stall), and what the CPU loses to the stalls meanwhile.

**Kit 53 (2026-11-17, built, emulator-run, not on the X51v).** Picked
from run 51's samples, all exact (the same words, the same bits):

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| A lit part's corners (`RP_StudioCmdLit`) | ref_soft and ref_gl: a triangle-interface call a corner; kits 49-52: each corner staged in a table (`RP_StudioMesh`'s `v[]`) and read back by `RP_StudioCmd` | one pass from the model's four shorts a corner (vertex, normal, s, t) to the TA's seven words; the near plane's new corner takes its colour from the corners' colour words | no ~24-byte staging write and read a corner on a cache that allocates on reads only (run 51: `RP_StudioMesh` 3.0%) |
| `rp_mv`'s records | Linux and WM leave the PXA270's read-allocate cache to the hardware | `RP_StudioVertFixed` asks for each record's line four vertices before writing it (PLD), the clip's `rp_mq` copy a few corners ahead | the stores land in the cache and the corners read them there; the fills overlap the transform (run 51: ~600 samples on these misses) |
| The client's light transforms (`StudioSetupBones`) | the SDK: a second `ConcatTransforms` a child bone for the light transform | copied from the bone transform when the bone's root came through `StudioFxTransform` unchanged (a byte compare) | Xash's hardware path makes them the same bits; half the concatenations go (run 51: 1.7%) |

Emulator (c1a0, 70 s, `+rp_lit 2`): 984,058 corners built both ways, 0
differed in a word; the near-plane cut was not exercised (no command
clipped there in c1a0). Estimate from run 51's samples: ~2,100 of 57,227 samples (~3.7% of the
sampled play) on top of kit 52's ~9-10%. **How to read the samples
(found for kit 53):** the runner's port-layer calls run with interrupts
masked (`with()`), so a masked call collapses to about one sample: the
TA feed (`rp_gfx_frame_parts`) shows as 3.0% while the log measures 4.2
ms of a 43 ms frame (~10%), about 1.3 samples a frame. The mixer's own
meter agrees with its samples (3.2-3.7%: its calls are short). So the
sampled shares of unmasked code are ~7% high against the wall clock, and
the feed is the largest single cost left on the CPU: step 4f (above)
would take most of it off.

**Kit 54 (2026-11-17, the owner: "keep going with 1-4"; built,
emulator-run, not on the X51v).** Three switches for one batched device
run, each measured against the default in the same run:

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins (or what the run decides) |
|---|---|---|---|
| Write-allocate (`-rpwalloc`) | Linux on the PXA27x and Dell's WM: write-back, read-allocate (Linux's `cachepolicy=writealloc` is opt-in) | the game modules' 14 MiB (0xA8C00000-0xA9A00000: code, data and every ref_rp table) remapped X=1, C=1, B=1 (`rt::write_allocate_sections`) | a table one stage writes and the next reads stays in the cache; the cost is a line fill on a first write: the run decides |
| The OS mixer's stages (always on) | none: the samples can't see inside a masked call | the core's cycle counter around each stage (`Stats::cyc_*`), a log line beside the mixer's counts | the ~520 cycles a frame split into adding, the chain and the write-back, so the next change is aimed |
| The engine mix at 22,050 Hz (`-rpmix22`) | Xash mixes at 44.1 kHz; its room effects still offer 22 kHz (`room_hires` 1, from the older engines' lower mix rates) | the engine's mixer and room effects at 22,050 Hz (the room's delays at `room_hires` 1's sizes), the OS mixer upsampling 2x to the codec | half the engine's mixing (S_PaintChannels, S_MixChannelToBuffer, SX_RoomFX: ~4.5% of the play) against the OS mixer's filter path: the run decides; a 44.1 kHz sound loses its top octave (most of Half-Life's are 22 or 11 kHz) |

**Step 4f's probe before the build (kit 54's finding).** The feed by DMA
pays only if the chip is fed while the CPU makes the next frame, which
needs the frame's words in two buffers (+512 KiB while R310 shows the
heap tight) and the render started from a pump; and an unpaced DMA write
into a full TA FIFO may hold the shared bus (the CPU's SDRAM then waits,
and a TA that never drains would freeze the X51v). So the next run asks
first, last in its batch with the owner told: (a) a frame's runs sent by
DMA with no room check, the strip ends by the CPU: does the TA take them
(TA done, no hang, the same picture); (b) a cache-missing loop's progress
during those pieces against idle: what the CPU keeps; (c) the paced way
(P4's) on the game's real frames: CPU free time a frame. Then 4f is built
on the answers.

**Step 4f's probe, built (2026-11-17, `-rp4f <seconds>`,
`ports/runner/src/gfx/probe4f.rs`; emulator-run, not on the X51v).**

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| Whether DMA may feed the TA | Dell's GLES: the CPU writes every word, a full FIFO stalls the bus; P4's `rp-tabench`: one unbroken list by DMA in pieces the FIFO has room for, timed on quads | the game's own frames fed twice each, by the CPU and by DMA (each run's whole cache lines by DMA, its ragged ends, the strip word and the strip's end by the CPU), both rendered and the pictures hashed against each other; 16 frames paced (phase A), then 16 unpaced (phase B, last: it may freeze the X51v) | the answer comes from the frames 4f would feed, judged by the chip's own pixels, not a synthetic list |
| What the CPU keeps meanwhile | a stall is invisible to the samples and to a loop that only polls | while each piece runs the CPU walks 128 KiB (four times the D-cache) a line a load, against the same walk with the bus idle, measured before each phase | the share of the CPU's memory speed left while the chip is fed: what a double-buffered 4f would really get back |
| The run's safety | a hung bus is found by the owner's eye | a piece not done in 200 ms aborted, a TA or render hang reset by the runner's recovery, phase B's first line on the card before its first piece, any failure ends the probe | a freeze, if it comes, is named in the log up to its moment |

The walk's buffer is the last 128 KiB of the samples' room (sampling then
covers ~48 s), the DMA channel is the CompactFlash reads' (the runner reads
no CF card; the channel table's chains are at their budget). Probed frames
are not counted in the `rp-gfx:` lines.

Emulator (c1a0, 70 s, `+rp_emit 2 -rp4f 20`): both phases 16 of 16
pictures the same as the CPU's, no TA, render or DMA failure; ~1,500
pieces a frame (~26 words each: the runs are short). The emulator's DMA
finishes at once, so the walk's share and the free time are the X51v's to
say. What would overturn the probe's plan for 4f: phase B hanging or
differing (then 4f stays paced), the walk keeping little of its speed
during pieces (then the bus is the cost, not the CPU's stores), or the
per-piece cost (arm, clean, poll ~1,500 times a frame) eating the free
time: then 4f's design is one descriptor chain a frame (the strip words
and ends from a side table), not a piece a run.

**Run 52's batch (planned 2026-11-17; kit 54's build, only `args.txt`
changes between runs; c1a0's walk, `-rpend 120`, compared with run 51's
first 120 s: 22.9 fps).** A: the default (kits 52-54). B: `-rpwalloc`.
C: `-rpmix22`. A2: the default again (the drift between A and A2 bounds
what B and C can claim). D: c2a5 with the tour to c2a4b (`-rpend 150`:
R309's water budget and sky, the conveyors, R310's heap fix across a
level change). E, last: `-rp4f 20`, `-rpend 45`, the owner told it may
freeze the X51v. Each run's log, samples and the mixer's stage cycles
read against A.

**Run 52 (2026-11-17, kit 54, logs in `local/half-life/run52`).** The
walk does not see the same views each run (where it looks depends on
timing), so the runs are read over lines 3-25 (after the map's start)
with each run's load beside it, as frame time against words a frame (A
and A2 give ~25.6 ms + 0.67 ms a thousand words):

| Run | fps | Words a frame | Against the default at the same load |
|---|---|---|---|
| run 51 (kit 51) | 24.6 | 15,400 | kit 54 ~12% faster |
| A, the default | 28.2 | 14,700 | |
| A2, the default | 26.2 | 18,700 | |
| B, `-rpwalloc` | **29.3** | 20,300 | **~13% faster** |
| C, `-rpmix22` | 24.8 | 17,500 | ~8% slower: the OS mixer's 2x upsampling took ~600 cycles a sound frame adding the stream (A: ~125), its wakes ~950 (A: ~490) |

A ended in a data abort in the end picture's software render (R312:
`-rpendshot` left out after it); B, C, A2 and E returned. D (c2a5) drew
the sky (4 sides) and water (7 faces, 57 cells, nothing cut) at 14-26
fps, then hung in `R_CreateDecalList` at the tour's reload of c2a5 (R313).

**The probe (E, c1a0's walk, nothing froze):**

| Phase | Words a frame | Feed by DMA / by CPU | CPU free while pieces ran | CPU's memory walk meanwhile | Pictures | Hangs |
|---|---|---|---|---|---|---|
| A, paced | 26,876 (19,402 by DMA in 738 pieces) | 9.2 / 5.0 ms | 3.3 ms | 65.9% of idle | 16 of 16 the same | none |
| B, unpaced | 9,715 (6,714 by DMA in 265 pieces) | 3.6 / 2.3 ms | 1.2 ms | 64.6% of idle | 16 of 16 the same | none |

The answers: (a) the TA takes the game's runs by DMA with no room check,
the FIFO full (510 words) without a hang, the pictures exact; (b) the CPU
keeps ~65% of its memory speed while the DMA writes, paced or not, so the
bus share is the cost, not a stall; (c) a piece a run costs more than it
frees: arming, cleaning and polling ~740 pieces took 9.2 ms against the
CPU's 5.0. So 4f is built as **one descriptor chain a frame, unpaced**:
each strip's corners straight from the emitter's buffer, its strip word
and end window write from a side table of constants, the CPU armed once
and free while the chain runs (~65% of its memory speed, all of its
cached work). On run B's load (~5 ms of feed a frame) that is ~3-4 ms back
a 34 ms frame: ~29 -> ~32 fps, with write-allocate on.

**Run 52 corrected (2026-11-17, found reading run 53's samples):** run 52 B
(`-rpwalloc`, 29.3 fps) had **no sound** (`sound: none (LinkRefused)`, the
run after A's crash: R301), so its engine mixing and the OS mixer (~8-10%
of the play) were missing. Its "~13% faster" is void.

**Run 53 (2026-11-17, kit 55, sound up in every run, c1a0's walk, lines
3-25):** N (read-allocate) 37.2 ms a frame at 16,100 words; NB (with
`-rpchain`) 37.6 ms at 15,700; A and A2 (write-allocate) 40.8 and 40.3 ms
at 17,500 and 16,400; B and B2 (write-allocate, chain) 38.1 and 39.5 ms.
**Write-allocate is ~8% slower** (the default again from 2026-11-17, kit
56); **the chain gains nothing measurable yet** (NB = N within the runs'
spread): every frame chained (830-2,300 descriptors), no hang, never waited
for, but its CPU side (the check walk and the descriptors) still ~3% of the
samples and the TA done ~12 ms after the arm, so the DMA shares the bus
with the CPU (~65% of its memory speed, run 52 E) for about a third of the
frame. Next for 4f: the chain's own time measured (when the DMA stops, not
when a pump looks), fewer and larger descriptors (the strip word kept in
the frame where the TA can take it, so a state block, its strip and its
corners are one descriptor), and the check and the build in one walk.
Kit 55 against kit 54 at the same load: the same within the spread.

**Kit 55 (2026-11-17, the owner: "Yes, do both"): write-allocate the
default (reverted after run 53, above), step 4f built behind `-rpchain`.**

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| The game's memory | read-allocate (Dell's WM, Linux's default) | write-allocate on the modules' 14 MiB by default (`-rpnowalloc` the control) | run 52: ~13% faster at the same load |
| The feed (`ports/runner/src/gfx/chain.rs`) | Dell: the CPU writes every word, a full FIFO stalls the bus; P4: DMA pieces sized by the FIFO's room, the CPU polling | one unpaced descriptor chain a frame over the frame's words where they lie (no copy); the strip word without its count, each strip's end, the grid word with the scene's end and the end word from a table of constants, so the TA says done with no CPU; the scene's start and the background by the CPU, the chain armed, the call returns | the CPU's ~3-5 ms of feeding a frame comes back; no piece a run (run 52's probe: dearer than the CPU) |
| The render's start | the CPU waits for the TA, then starts the render | a pump in the program's clock reads (`Screen::pump`, from `now_us`: a game reads the clock many times a frame) looks at the DMA and the TA's done bit and starts the render; the next frame's call waits for whatever is left (counted: "waiting at a frame's start for the last chain") | no interrupt, no new port call; the render starts within a clock read of the TA's done |
| The frame's words | one buffer, reused as soon as the call returns | `RP_GFX_KEEP` (`struct rp_gfx`'s new `flags`): ref_rp writes the next frame into a second `rp_buf` and `rp_lm_buf` (640 KiB from the heap, only when the port asks; without room it waits for the last frame each frame) | zero copy; the DMA reads frame n while the emitter writes n+1 |
| Alignment | 8-byte DMA addresses (the PXA27x's default) | the channel's `DALGN` bit (Linux's `pxa_dma` does the same for a misaligned transfer: a finding, to prove on the X51v) | the corners start on any word |

The chain's channel is the USB controller's (3; the probe keeps CF's), its
table and descriptors 192 KiB of the scratch RAM (12,284 descriptors, ~4
a strip; a frame needing more goes by the CPU, counted). Off by default
until the X51v draws it: its first run measures `-rpchain` against the
CPU's feed on the same walk.

**Kit 57 (2026-11-17, the owner: "Start the next batch toward 30, biggest
levers first": yes), all exact:**

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| The traces' float-bit copies | `memcpy` under `-ffreestanding` (a call each: builtins off) | `__builtin_memcpy` from the aligned local (`RP_FloatBits` in pm_trace.c, ref_common.h), inline at any flags | ~2% of run 53 N's play in those calls. **Kit 57 first built all the game hosted (builtins on): the X51v faulted at once** (run 54: the menu's `MurmurHash3_32`, a `memcpy` from a misaligned `uint32_t *` inlined as a word load; the emulator does not trap misaligned loads, R280), so freestanding stays and only copies from aligned locals are builtins |
| The OS mixer's add (one stereo stream at 44.1 kHz, full volume: Half-Life's) | a general per-sample add with a saturating sum, a gain test and a channel test | the block's first stream written into the empty mix a run at a time | run 53: 128 cycles a frame for the add; the same values (`0 + v` never saturates; the test holds the fast and the reference paths equal both ways) |
| The speaker's high-pass | the RBJ cookbook's direct form, five products a sample | its numerator is `b0 (1, -2, 1)` exactly in the stored Q30 words (asserted at build): three products | bit for bit the same (200,000 samples compared) |
| The test walk | `impulse 100` without the suit (ignored by the game: no flashlight, no HUD in runs to 53) | `give item_suit` first | every run measures what the player sees: HUD and flashlight light |
| The touch keys (LIGHT, FIRE 2) | shown whenever a map is played | only with the HEV suit (the owner, 2026-11-17), as the game's own HUD: `WEAPON_SUIT`, bit 31 of the weapons the server sends | no flashlight key before the suit has a flashlight |

**Run 54 (2026-11-17, kit 57 with the fix, the suit on: the HUD and the
flashlight in every run, sound up in every run; c1a0's walk, lines 3-25):**
A 41.0 ms a frame at 17,300 words, A2 37.4 at 16,200; with `-rpchain` B
34.7 at 15,100, B2 39.5 at 18,200. The OS mixer, by its own meter: the
add 128 -> 76 cycles a frame, the chain 221 -> 191, the whole wake 487 ->
410; its share of the time 3.8% -> 3.2%. **Two runs of the same kit differ
by up to ~10% (A against A2): the walk's views and timing differ run to
run, so changes of a few percent cannot be told apart this way.** Next:
a fixed measurement, the engine's `timedemo` of one recorded demo (the
same frames every run), before the next levers are judged.

**The fixed-step walk (2026-11-17, the owner chose it over `timedemo`,
which replays the client only).** `-rpwalk` now sets the engine's
`host_framerate` to 1/30 s and the random seed on the run's first frame,
before the map loads, and the walk's timers read the game's clock: every
run plays the same frames. The score: `rp-walk: frames a-b took N ms`
every 250 frames, with the player's position (the emulator: two runs at
the same place at every mark). Kits compare window by window.

**Run 55 (2026-11-17, kit 59, the X51v, two runs of the same kit, suit,
HUD, flashlight and sound on):** frames 1-2,750 took **111,156 ms in both**
(24.7 fps), every 250-frame window within 1 ms of the other, the player
at the same place at every mark (and the emulator's). The yardstick for
every lever from here: kit against kit, window by window.

**Run 56 (2026-11-17, the fixed-step walk, frames 1-2,750, sound on):**
the CPU's feed 111,156 ms (run 55); **the DMA chain 103,811 ms: 6.6% less
time, 24.7 -> 26.4 fps**; write-allocate 113,209 ms (1.8% slower: stays
off). **The chain is the default from kit 60** (`-rpnochain` the control;
kit 60's default run: 103,808 ms). Run 56's samples with the chain (the
play): soft floats `rp_fadd` 6.8% and `rp_fmul` 3.7% (from the traces,
`R_MarkLights` for the flashlight, `R_AddDynamicLights`), `RP_StudioCmdLit`
4.5%, `RP_ChipFaceFixed` 3.4%, `RP_StudioLightFixed` 3.0%, the OS mixer
3.0%, `rp_gfx_frame_parts` 2.9%, `RP_StudioCmdSend` 2.9%: the next
levers, each judged on this walk.

**Kit 61 (2026-11-17, the next exact levers from run 56's samples, biggest
first that are exact; each checked in the emulator, `rp_lit 2` counting
every difference):**

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| The traces' plane sides (`PM_RecursiveHullCheck`, `PM_HullPointContents`) | Quake's and Xash's `PlaneDiff`: the point's coordinate (or its dot with the normal) minus the plane's distance, a software subtraction twice a node, the sign then tested | the two sides compared on their bits (`RP_LtSide`: the floats' order as integers); the subtraction made only for a node the line crosses, where its value is used; a NaN or an infinity takes the old way | for finite floats `a - d` rounded is `< 0` exactly when `a < d` (the sign survives rounding, zero only at `a == d`, subnormals kept): 200 million pairs on the PC (edges, neighbours, subnormals), none differ. Run 56: the subtractions ~1% of the play; a box hull (all axial planes) now walks with no float at all |
| The flashlight's marks (`R_MarkLights`) | the light's foot on the plane made for every surface of a node | made once a plane and kept while the next surface has the same plane (a node's surfaces lie on its plane) | the same operations on the same values; the hoped-for ~1/4 of the marks' floats not seen on the X51v (run 57: a node's faces carry plane entries of their own) |
| The models' light and vertices (`RP_StudioLightFixed`, `RP_StudioVertFixed`) | ref_soft and ref_gl: a call a normal (`R_StudioLighting`) and a vertex (`Matrix3x4_VectorTransform`); kit 49 kept that shape | one call a mesh (`RP_StudioLightMesh`) and one a part (`RP_StudioVertsFixed`): flags tested once, full-bright and flat meshes made once and copied, the light in registers | run 56: ~320 cycles a normal, ~340 a vertex, much of it the call's frame and the globals' loads; the same integer maths, so the same bytes (`rp_lit 2` remakes each the old way and counts differences) |
| The server's entity walk (`SV_Physics`) | each edict read as the loop reaches it (the CPU's own prefetch does it on a desktop) | the edict two on and its movetype's line asked for with `PLD` | run 56: ~300 samples waiting on those first loads; a hint only, nothing read or written changes |

**Run 57 (2026-11-17, kit 61, the X51v, the fixed-step walk, kit 60's
default arguments, sound up the whole run):** frames 1-2,750 took
**101,583 ms (27.0 fps) against kit 60's 103,808 ms (26.4 fps): 2.1% less
time**, every one of the twelve marks at the same place as kit 60's (the
traces' compares change nothing the game sees). The emulator first: `rp_lit
2` made 948,617 normals and vertices both ways, 0 differed. Run 57's
samples against run 56's: `PM_RecursiveHullCheck`'s float adds 780 -> 283,
`SV_Physics` 758 -> 591, the models' light 1,752 -> 1,531 (the call's cost
was the smaller part: the maths and the normals' misses stay), the
vertices 1,220 -> 1,244 (no gain), `R_MarkLights`' adds unchanged (Xash's
loader gives a node's faces plane entries of their own, so the kept foot is
made again for nearly every surface; still exact). Next, biggest first:
`RP_StudioCmdLit` 4.7%, the soft floats 9.4% (`R_MarkLights`,
`PM_PlayerTraceExt`'s per-entity offsets), `RP_ChipFaceFixed` 3.4%,
`rp_gfx_frame_parts` 3.0% (all its samples on the instruction after it
turns interrupts back on: the chain's build with interrupts off, so the
sampler cannot see inside it), the OS mixer 3.1%.

**Kit 62 (2026-11-17, from run 57's samples, all exact):**

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| A model's float normals and points to fixed point | a C cast (a software call), or kit 49's `rp_fixq`: the exponent tested for zero, underflow and overflow, a branch each, a shift left or right | `rp_fixq3_fast`: when all three exponents are at most 157 - q (every normal and model point), the significand kept 7 bits up and shifted right once; a shift of 32 or more is 0; anything else goes to `rp_fixq` | run 57: ~22% of the lighting loop; the same integers on every one of the 2^32 floats for q = 16, 28 and 30 (6.7 billion on the fast path, none differ) |
| The gamma table and the colour bytes | `LightToTexGamma` tests the engine's linear-gamma bit for every normal; `rp_light_byte` a 64-bit product | the table chosen once a mesh; 32-bit products when every light colour is below 2^21 (16.16), so `g16 x 1023` fits | the same values; a load, a test and three 64-bit multiplies a normal gone |
| The colour bytes' lines | written, then read by `RP_StudioCmdLit` (the PXA270 allocates on reads only, so the reads missed: run 57's hottest instruction there) | asked for with `PLD` sixteen normals ahead of their stores | a hint only |
| `PM_PlayerTraceExt`'s per-entity result | Quake's and Xash's: every entity's end point (a lerp) and plane distance (a dot product) made, then dropped unless that entity is the nearest so far | made only for an entity whose trace is kept | the same kept values; ~10 float operations an entity a trace gone (most traces miss most entities) |

**Run 58 (2026-11-17, kit 62, the X51v, the fixed-step walk, sound up):**
frames 1-2,750 took **100,359 ms (27.4 fps) against kit 61's 101,583 ms
(27.0 fps): 1.2% less time**, every 250-frame window faster (by 84-130 ms)
and every mark at the same place; the emulator first: 948,617 normals and
vertices both ways, 0 differed. From kit 60: 103,808 -> 100,359 ms (3.3%).
30 fps is ~91,700 ms: 8.6% still to go. Run 58's samples, the play:
`rp_fadd_asm` 5.6%, `RP_StudioCmdLit` 4.5%, `RP_ChipFaceFixed` 3.5%,
`rp_fmul_asm` 3.5%, `RP_StudioCmdSend` 3.1%, the OS mixer 3.1%,
`rp_gfx_frame_parts` 3.0% (interrupts off), `RP_StudioLightMesh` 2.5%
(1,531 -> 1,453: its misses, not its maths, are most of what is left).

**Kit 63 (2026-11-17, all exact):**

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| The TA's tail buffer and region array cleared each frame | Dell's driver and `ta::clear`: the CPU zeroes them in the 2700G's memory, ~21,000 single stores across the bus (84,744 bytes), then writes the start table and the background strip | the chain does all of the scene's start: twelve descriptors from a block of zeros in the chain's own memory, nine for the start table's registers, the background strip and its end, then the frame; the CPU only arms the done bit and the chain | the CPU's feed 2.2-3.0 -> ~1.1 ms a frame; the same words in the same order: `-rpchaincheck` fed 32 frames both ways and hashed the pictures, the same 32 of 32 in the emulator and on the X51v (run 59) |
| The network's send buffer | Xash (and GoldSrc's netchan): a 32 KiB stack buffer zeroed for every packet the client and the server send | zeroed only as far as the packet can reach, bounded before its first write (headers, the reliable and unreliable bits, the pad, a dword of slack); a guard counts any packet past it (0 in every run) | run 59's samples (the sampler's fix below): this memset 1.0% of the play; the bit writer keeps the bits around each field, so zero is needed where it writes and sends, and only there: the same bytes on the wire |
| The sampler's caller of a helper that uses `lr` as data (`memset`, `__aeabi_memcpy4`) | gprof and perf unwind with frame pointers or DWARF tables the build does not keep | when the interrupted `lr` is not a return address (a word after a `bl`/`blx`), the first one in the stack's top 16 words (`rt::set_sample_window`) | ~3% of the samples had a caller of 0; now named (run 59: `Netchan_TransmitBits` 1.0%, `CL_ProcessPacket` 0.4%, `CL_RunUsercmd` 0.4%, `CL_PredictMovement` 0.4%) |

**Run 60 (2026-11-17, kit 63, the X51v, the fixed-step walk, sound up):**
frames 1-2,750 took **98,686 ms (27.8 fps) against kit 62's 100,359 ms
(27.4 fps): 1.7% less time**, every window faster, every mark the same.
(Run 59, with the chain alone and `-rpchaincheck` on its first frames:
frames 251-2,750 0.95% faster than kit 62; the check's frames made its
first 250 slower.) From kit 60: 103,808 -> 98,686 ms (4.9%); 30 fps is
~91,700 ms, 7.1% still to go. The chain's gain is less than the CPU time
it frees: the DMA now holds the bus ~2-3 ms longer a frame while the game
runs.

**Kit 64 (2026-11-17, exact):**

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| A world face's corners (`RP_ChipFaceFixed`) | each corner's vertex record read as the loop reaches it (a desktop CPU's prefetcher hides the misses) | every corner's record asked for with `PLD` before the loop | run 60: ~450 samples waiting on those misses one after another; run 62: 2,008 -> ~1,800 |
| The client's physics entities (`CL_CopyEntityToPhysEnt`) | Xash: `pe->player` written, then read back (the compiler must: `pe` and `state` may alias), the state read as it comes | the value kept in a register; the state's lines asked for first | run 60: its hottest load was that read-back (the PXA270 does not allocate on a store, so it waited on memory); 815 -> ~490 samples |
| R316, the fused dot product in C (`ports/xash/include/rp_fdot.h`) | C's five software-float calls | one call, every product and sum rounded as C rounds them (1,000,000,000 vectors on the PC, none differ; run 61's marks all the same) | **measured slower, not used**: run 61 (kit 64 with it) 98,804 ms against kit 63's 98,686; `rp_fdot3` took 1,591 samples where the five assembly calls it replaced cost ~850. The compiled C loses to the hand-written assembly routines; an assembly version is the next try (R316) |

**Run 62 (2026-11-17, kit 64 without the fused dot, the X51v, the
fixed-step walk, sound up):** frames 1-2,750 took **98,191 ms (28.0 fps)
against kit 63's 98,686 ms (27.8 fps): 0.5% less time**, every window
faster, every mark the same. From kit 60: 103,808 -> 98,191 ms (5.4%);
30 fps is ~91,700 ms, 6.6% still to go.

**Kit 65 (2026-11-17, exact, from run 62's samples):**

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| The server's physics entities (`SV_CopyEdictToPhysEnt`) | Xash: the edict's fields read as the copy reaches them | the edict's lines asked for with `PLD` first (kit 64's client fix, server side) | run 62: 648 samples, most of them waiting on those loads; a hint only |
| R316, the fused dot product, in ARM assembly (`rp_fdot3_asm`, `hal/mem/src/float_arm.rs`) | C's five software-float calls (three `rp_fmul_asm`, two `rp_fadd_asm`), each unpacking its operands, testing its rare cases, rounding and packing, with a call, a return and two saved registers each (a desktop does the dot in an FPU; a fused multiply-add rounds once and gives other bits); kit 64's C version of ours lost to the calls (1,591 samples against ~850) | one call: the six operands tested for the common case at once per pair, the three products made with `umull` and rounded at the top of a word, kept unpacked (exponent, significand, sign) into the two sums, the sums' alignment, add or subtract and normalisation without a branch (`clz` covers a carry and a cancellation alike, the smaller operand chosen by conditional moves), one pack at the end; a zero product made in place (axial normals); every rare case (subnormal, infinity, NaN, a result past 2^126 or under 2^-126) takes the five calls in C's order | the same bits as C by construction (each step rounds as `rp_fmul_asm`/`rp_fadd_asm`, the sums as the C reference proven on 10^9 vectors), checked on the X51v and in the emulator by `-rpfloat` against the compiler's generic routines (six kinds of vectors: the game's range, zeros, cancellations, any exponent, any bits, the edges); `-rpfloat` also times the generic calls, our five calls and the fused one on the X51v |
| The light styles' clocks (`CL_RunLightStyles`, `SV_RunLightStyles`) | Quake and Xash: every one of the 64 styles' clocks advanced every frame on the client and the server, and the server's `time * 10` made for each, used or not; each style is a 1,296-byte record, so each is a cache miss | a style with no pattern (most of the 64) gets its value and nothing else: its clock is never read (not sent, not saved; set anew with its pattern), and the client's single-letter styles likewise; the server's `time * 10` made only where it picks a letter; each record's line asked for with `PLD` four styles ahead | run 62: 156 of `CL_RunLightStyles`' 198 add samples sit on the add's first instruction, the stall of the clock's miss; the same values for every style that has a pattern (the game sees nothing else) |

**Run 63 (2026-11-17, kit 65, the X51v, the fixed-step walk, sound up):**
frames 1-2,750 took **97,389 ms (28.2 fps) against kit 64's 98,191 ms
(28.0 fps): 0.8% less time**, every 250-frame window faster (by 30-94
ms) and every one of the thirteen marks (to frame 3,250) at the same
place. The emulator first: `-rpfloat` compared
12,582,912 results (6,291,456 vectors, six kinds, each against the fused
routine and the five calls), 0 differ; the X51v the same, 0 differ. On
the X51v the dot product costs **208.5 cycles fused against 267.9 for our
five assembly calls** (553.6 for the compiler's generic ones), 22% less;
in the play `rp_fdot3_asm` took 1,165 samples where the calls it replaced
had cost ~1,280 (`rp_fadd_asm` 3,459 -> 2,698, `rp_fmul_asm` 2,168 ->
1,649; part of the routine's samples are its operands' misses, which the
callers carried before). The light styles: the client's 300 -> 154
samples, the server's (inside `SV_Physics`) 810 -> 694;
`SV_CopyEdictToPhysEnt` 648 -> 429. From kit 60: 103,808 -> 97,389 ms
(6.2%); 30 fps is ~91,700 ms, 5.8% still to go. Run 63's play, biggest
first: `RP_StudioCmdLit` 4.4%, `rp_fadd_asm` 4.2%, `RP_ChipFaceFixed`
3.1%, `RP_StudioCmdSend` 3.0%, `rp_snd_write` 2.8%, `RP_StudioLightMesh`
2.6%, `rp_fmul_asm` 2.5%, `rp_gfx_frame_parts` 2.2%, `rp_fdot3_asm` 2.0%.

**Kit 66 (2026-11-17, exact, from run 63's hottest single instructions):**
run 63's samples sit, more than anywhere else in the game, on the first
use of a load that missed the cache: the PXA270 has no hardware
prefetcher and its misses cost far more than its maths here. Each lever
below is a `PLD` hint, nothing read or written changes.

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| The server's entity walk (`SV_Physics` -> `SV_UpdateBaseVelocity`) | each edict's flags read when its turn comes (a desktop's prefetcher sees the stride) | kit 61's `PLD` of the edict two on now also takes its flags' line | run 63: 195 samples on the flags' first use |
| The entities sent to the client (`SV_AddEntitiesToPacket`, the game's `AddToFullPack`) | the same, edict by edict | the edict two on: its first line, its effects' and its model index's lines | run 63: 138 + 128 samples on those first uses |
| An entity's delta (`MSG_WriteDeltaEntity` -> `Delta_CompareField`) | Xash and GoldSrc: field by field, each field's bytes in both states read as the compare reaches them | every line of both states asked for before the fields are walked, and the field table four entries ahead | run 63: ~450 samples on the states' and the table's first loads |
| The client's physics entities (`CL_AddLinksToPmove`) | state by state | the state two on asked for | run 63: 136 samples on a state's first use |
| A model's entity lights (`R_StudioEntityLight`) | all 128 entity-light records (40 bytes each) read for every model drawn, nearly all dead | the record four on asked for as each is tested | run 63: 320 samples on the die time's first use |
| The flashlight's BSP walk (`R_MarkLights`) | each node's children and each surface read when reached | both children asked for on arrival at a node (while its distance is made), the surface two on in the surface loop | run 63: ~300 samples on nodes' and planes' first loads |

**Run 64 (2026-11-17, kit 66, the X51v, the fixed-step walk, sound up):**
frames 1-2,750 took **96,296 ms (28.5 fps) against kit 65's 97,389 ms
(28.2 fps): 1.1% less time**, every 250-frame window faster (by 34-143
ms) and every one of the
thirteen marks at the same place; the emulator first (its marks the
same, the dot's check 0 differ). By spot, run 63 -> run 64 samples:
`SV_UpdateBaseVelocity` 215 -> 35, `Delta_CompareField` 712 -> 406,
`R_StudioEntityLight` 468 -> 211, the game's `AddToFullPack` 386 -> 321,
`SV_AddEntitiesToPacket` 233 -> 166; but `SV_Physics` 694 -> 784 and
`Delta_WriteField` 244 -> 297 (part of the misses now wait on the hints
themselves: the PXA270 has four fill buffers), `R_MarkLights` 624 -> 598
(little), and `CL_SetSolidEntities` (where `CL_AddLinksToPmove` is
inlined) 382 -> 430: that hint gave nothing and is taken out in the next
kit (R317). From kit 60: 103,808 -> 96,296 ms (7.2%); 30 fps is ~91,700
ms, 4.8% still to go.

**Kit 67 (2026-11-17, exact, from run 64's samples):** the copy routines
are now the largest cluster (~1,800 samples), and part of it is copies of
two to twelve bytes made as calls: the game modules are built freestanding
(kit 58: builtins off, because a hosted build inlined a misaligned word
load that faulted on the X51v), so every `memcpy` in their source is a
call to the general routine.

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| A light's die time against the frame's time (`RP_FloatLess`, `RP_TimeCeil`, ours since run 51) | the float's bits read with `memcpy` (the portable C idiom): here a call each, two a light, 128 lights a model | `__builtin_memcpy` between aligned locals: one register move | run 64: 130 samples in `memcpy` from `R_StudioEntityLight` alone; the same bits |
| An animation value (`UnalignedShort` in the engine, `Unaligned<short>` in the game's client) | Xash's: `memcpy` of two bytes (a call here), safe for a value at an odd address | two byte loads in a line of assembly: never a halfword load (alignment unknown, R280), never a call | run 64: ~250 samples of 2-byte copies (the bones' angles and positions); the same short for any address |
| The game's vectors into an entity's state (`AddToFullPack`) | the SDK's `memcpy( state->origin, ent->v.origin, 3 * sizeof(float) )`, eight a visible entity | `__builtin_memcpy`: three word moves (both float arrays aligned) | run 64: 201 samples of `memcpy` from it |
| R317 | kit 66's hints | `CL_AddLinksToPmove`'s hint out (no gain), `SV_Physics` back to three hints a step | run 64: the PXA270's four fill buffers |

**Run 65 (2026-11-17, kit 67, the X51v, the fixed-step walk, sound up):**
frames 1-2,750 took **95,736 ms (28.7 fps) against kit 66's 96,296 ms
(28.5 fps): 0.6% less time**, every 250-frame window faster (by 18-88
ms) and every one of the thirteen marks at the same place; the emulator
first (the marks the same, no `memcpy` call left in the bones' code or
`R_StudioEntityLight`). Run 64 -> run 65 samples: `memcpy` 1,288 -> 771;
part of what it waited on now waits in the callers, where the copies
are inlined (`AddToFullPack` 321 -> 439, `StudioCalcBoneQuaterion` 365
-> 476, `StudioCalcBonePosition` 116 -> 149, `R_StudioCalcBones` 204 ->
234), `R_StudioEntityLight` 211 -> 157, `CL_SetSolidEntities` 430 -> 392,
`SV_Physics` 784 -> 762: about 340 samples less in all. From kit 60:
103,808 -> 95,736 ms (7.8%); 30 fps is ~91,700 ms, 4.2% still to go.

**Kit 68 (2026-11-17, exact):**

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| A model command's corners (`RP_StudioCmdLit`, `RP_StudioClipCmd`) | GL and ref_soft light and set up every vertex before culling; kit 53's one pass made every corner's texels (two float-bit conversions, the wrap test) and colour (the normal's colour bytes, a miss) before the sort dropped any | the positions first; the sort (near plane, facing, off screen) decides which triangles are sent; only their corners get texels and colour (`RP_LitFill`, once a corner); a command wholly behind the near plane skips the copy of its view-space points | run 65: of a frame's ~3,500 model triangles ~1,050 face away or leave the screen and ~1,100 lie wholly behind the near plane; 4.4% + 3.2% of the play in the two functions. The same words for every corner sent (`rp_lit 2` builds every corner both ways; a guard counts any corner sent unfilled, 0) |
| The engine's sound mixing loops (`S_MixMono*`, `S_MixStereo*`, `*Pitch*`, `*Lerp*`) | Xash's C: `data[i] * volume[0]` in the loop; the source bytes may alias the paint buffer, so the compiler loads each byte twice and a volume from the stack each sample | the volumes and each source sample read once into locals | run 65: `S_MixChannelToBuffer` 971 samples, the 8-bit loop's load-use stalls; the same integer sums |

**Run 66 (2026-11-17, kit 68, the X51v, the fixed-step walk, sound up):**
frames 1-2,750 took **94,204 ms (29.1 fps) against kit 67's 95,736 ms
(28.7 fps): 1.6% less time**, ten of the eleven 250-frame windows
faster (by 41-277 ms; frames 1,001-1,250 2 ms slower) and every one of the thirteen marks at the same place. The emulator
first, twice: with `rp_lit 2`, 1,072,366 corners built both ways, 0
differed in a word; plain, the guard's corners sent unfilled 0 (0 on the
X51v too). Run 65 -> run 66 samples: `RP_StudioCmdLit` 2,500 -> 1,564,
`RP_StudioCmdSend` 1,840 -> 1,634, `S_MixChannelToBuffer` 971 -> 896.
From kit 60: 103,808 -> 94,204 ms (9.3%); 30 fps is ~91,700 ms, 2.7%
still to go.

**Kit 69 (2026-11-17, exact):** run 66's log says what its samples could
not: `rp_gfx_frame_parts` (2.5% of the play, all of it seen where it turns
interrupts back on) is mostly the CPU **waiting for the last frame's
render**, 0.6-2.9 ms a frame, at the start of each frame call. Step 4f's
render starts when a pump sees the TA done, and the pump ran only at the
program's clock reads: "the TA done 15-17 ms after the arm" is when the
pump looked, not when the chip finished (the descriptor count barely
moves it: 1,681 descriptors 14.8 ms, 2,904 16.6 ms).

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| Starting a frame's render after the TA took it (`Screen::pump`) | Dell's GLES driver starts it from the 2700G's interrupt or at the next API call (eglSwapBuffers); step 4f's pump at the program's clock reads | the runner's OS-timer tick (1 kHz; the sampler's when sampling) runs the screen's pump alone (`rt::set_tick_hook`, `rp_hal_cabi::pump_screen`): the render starts within a millisecond of the chip being done; the system is held only with interrupts masked, so a tick never lands inside a call | no unknown hardware (the 2700G's interrupt line on the X51v is not mapped yet), a register read a millisecond while a frame is in flight; the same frames drawn, sooner, so the next frame's start waits less |
| The OS mixer's wake (R306's ~130 cycles a frame outside the measured stages) | measured as a whole | timed by part: the clock and the clearing of what played, the mixing whole, the ring's end moved, the rest, and the programs' samples copied in; the copy itself in two plain loops (a chained iterator tested which part of the ring every sample went to) | the next lever named by a run, not guessed; the same bytes |
| A world face's corner table (`RP_ChipFaceFixed`) | read when the face is drawn | the next face's line asked for while this one is drawn (`RP_FaceAhead`) | run 66: 271 samples on that first load; a hint only |

**Run 67 (2026-11-17, kit 69, the X51v, the fixed-step walk, sound up):**
frames 1-2,750 took **91,856 ms (29.9 fps) against kit 68's 94,204 ms
(29.1 fps): 2.5% less time**, ten of eleven 250-frame windows faster (by
4-711 ms; frames 2,001-2,250 8 ms slower) and every one
of the thirteen marks at the same place (the emulator first: its marks
the same). The wait for the last render, 0.6-2.9 ms a frame in run 66,
is 0.0-0.6 ms; `rp_gfx_frame_parts` 1,436 -> 1,020 samples,
`RP_ChipFaceFixed` 1,837 -> 1,718. The mixer's wake on the X51v, cycles
a frame mixed: the clock and the clearing of what played 16, the mixing
whole 306 (its stages 74 adding, 189 the chain, 6 writing back), the
ring's end 38, the rest 53, the programs' samples copied in 10: the
chain (speaker filter, limiter, dither) is the mixer's cost, not its
book-keeping. From kit 60: 103,808 -> 91,856 ms (11.5%); 30 fps is
91,667 ms: 189 ms (0.2%) still to go.

**Kit 70 (2026-11-17, exact):**

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| The float compares (`__aeabi_fcmp{eq,lt,le,gt,ge}`) | compiler-rt and libgcc: unpack, test the special cases one by one; ours since R282 a Rust wrapper round `float::compare` | ARM assembly (`rp_fcmp*_asm`), the AEABI names aliased straight to it: a NaN out and both zeros equal in four instructions, then each float's bits made an ordered integer (a negative's magnitude bits flipped) and one signed compare, no branch but the rare exits | run 67: the five compares 1,255 samples, 2.2% of the play (`VectorIsNull`, the traces' plane tests, lighting); the same answer for every pair (IEEE's order is the integers' order on sign-and-magnitude bits): `-rpfloat` checks every edge pair in every sign and 2^21 random and neighbouring pairs against the compiler's own compares |
| The OS mixer's divisions | the frame counters are 64-bit (a ring never wraps them in a lifetime), so `% RING_FRAMES`, `% cap`, a piece's multiple and `Ratio::locate`'s `/ 44100` were 64-bit divides: library calls on the PXA270, about five a 32-frame block | a 32-bit divide whenever both numbers fit (the first 27 hours of sound); by a constant it is a multiply | the same quotient and remainder for every value (the 64-bit way stays past 2^32) |

**Run 68 (2026-11-17, kit 70, the X51v, the fixed-step walk, sound up):**
frames 1-2,750 took **91,692 ms (29.99 fps) against kit 69's 91,856 ms
(29.9 fps): 0.2% less time**, every mark the same; `-rpfloat`'s compares
2,098,752 results, 0 differ (the emulator and the X51v). The compares
gave less than their share promised (1,255 -> 1,166 samples): most of
their samples are the callers' argument loads missing, which land on the
routine's first instruction. 30 fps is 91,667 ms: 25 ms still to go.

**Kit 71 (2026-11-17, exact, from run 68's hottest instructions):**

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| The bulk copy (`rp-hal-mem`'s `memcpy`, `__aeabi_memcpy4`) | newlib and compiler-rt: no prefetch; ours since C7 a `PLD` 128 bytes ahead in the loop (the X51v's sweep: 47 -> 109 MB/s on 256 KiB) | also the first four lines asked for before the loop (the game modules' build only: the OS images keep their size budgets) | run 68: 551 samples on the copy loop's first use of a load, most copies 340 to 2,352 bytes (entity states, the client's prediction state): their first lines missed one after another; a hint only |
| A model command's vertex records (`RP_StudioCmdLit`) | kit 53's `PLD` four corners ahead | eight ahead in this loop | kit 68 left the loop a third as long, so four corners is no longer a miss's time; run 68: 218 samples on the record's first load |
| The server's entity walk (`SV_Physics`) | kit 66's hints (the edict, its movetype's and flags' lines) | the velocity's line too | run 68: 250 samples on its first load (the physics' first test) |

**Run 69 (2026-11-17, kit 71, the X51v, the fixed-step walk, sound up):**
frames 1-2,750 took **91,454 ms (30.07 fps) against kit 70's 91,692 ms
(29.99 fps): 0.3% less time, over 30 fps**, every 250-frame window faster
(by 10-30 ms) and every one of the thirteen marks at the same place; the
emulator first, its marks the same, `-rpfloat`'s checks 0 differ on both.
Run 68 -> run 69 samples: `__aeabi_memcpy4` 873 -> 828, `memcpy` 824 ->
717, `RP_StudioCmdLit` 1,528 -> 1,508, `SV_Physics` 767 -> 758. From kit
60: 103,808 -> 91,454 ms (11.9%); from kit 64 (28.0 fps) seven kits.
The margin over 30 fps is 213 ms (0.2%) on this walk; the 5-second
frame-rate lines over the whole run (the map's start and the corner where
the walk turns included) average 27.85.

**Kit 72 (2026-11-17, exact, widening the margin):**

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| A model's normals lit (`RP_StudioLightMesh`) | GL and ref_soft light every vertex before culling; kit 61 lit a mesh's normals in one call, all of them | a mesh is recorded and each normal lit when a sent corner first reads its colour (`RP_StudioNCol`), with the same per-normal integer maths (`RP_LitNormal`, shared by both ways); full-bright and flat meshes made at once | run 69: 1,486 samples while ~60% of the model triangles are dropped before drawing (kit 68); `rp_lit 3` lights lazily and makes each such normal the old way too, counting differences |
| The traces' BSP walk (`PM_RecursiveHullCheck`, `PM_HullPointContents`) | each level's clip node read when reached, then its plane: two misses one after the other a level | both children's clip nodes asked for while this node's plane is tested (a leaf's negative number skipped) | run 69: ~180 samples on the node's first load and ~230 on the plane's; a hint only |

**Run 70 (2026-11-17, kit 72, the X51v, the fixed-step walk, sound up):**
frames 1-2,750 took **90,161 ms (30.5 fps) against kit 71's 91,454 ms
(30.07 fps): 1.4% less time**, every 250-frame window faster (by 17-212
ms) and every one of the thirteen marks at the same place. The emulator
first, twice: `rp_lit 3` lit 164,075 normals lazily and made each the
old way too, 0 differed; plain, the corners' guard 0 (0 on the X51v
too). On the X51v 208-563 of a frame's 2,200-2,900 recorded normals are
lit (the rest belong only to dropped triangles): `RP_StudioLightMesh`
1,486 -> 58 samples plus `RP_StudioNCol` 406. The traces' child hint
cost more than it gave (`PM_RecursiveHullCheck` 1,215 -> 1,301,
`PM_HullPointContents` 473 -> 548: one of the two children is always a
fill wasted on the PXA270's four): out in the next kit (R318). From kit
60: 103,808 -> 90,161 ms (13.1%); 30 fps is 91,667 ms: the margin is
1,506 ms (1.6%).

**Kit 73 (2026-11-17, exact):**

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| The server's datagram and the client's command packet (`SV_SendClientDatagram`, `CL_WritePacket`) | Xash and GoldSrc: the whole buffer zeroed before each message (16 KiB and 8,000 bytes a frame), because the bit writer reads and masks whole dwords | `MSG_InitLazy`: the writer zeroes each dword the first time a write reaches it (64 bytes at a time, a frontier that only moves up); every other buffer as before | run 70: ~280 samples of `memset`/`__aeabi_memclr4` from these two; below the frontier everything was zeroed then written, so every byte a message holds is the cleared buffer's (kit 63 bounded the network's own buffer the same way) |
| The engine's output peak (`SNDDMA_Submit`) | run 37's diagnostic: every output sample read for a once-a-minute loudness line | measured only when a test run asks (`-rpsndpeak`) | run 70: ~150 samples; the sound unchanged |
| R318 | kit 72's clip-node hints | out | run 70: they cost more than they gave |

**Run 71 (2026-11-17, kit 73, the X51v, the fixed-step walk, sound up):**
frames 1-2,750 took **89,772 ms (30.6 fps) against kit 72's 90,161 ms
(30.5 fps): 0.4% less time**, every 250-frame window faster (by 29-52
ms) and every one of the thirteen marks at the same place (a message
holding other bytes would have moved them: the client and server talk
through these buffers every frame); the network guard 0. From kit 60:
103,808 -> 89,772 ms (13.5%); the margin over 30 fps is 1,895 ms (2.1%).
The owner, watching it: "looks glitchy" (things jump or stutter, frame
pacing hitches). Not yet measured: the frame-rate lines are 5-second
averages, so a single long frame does not show; the walk's own motion
(game time a fixed 1/30 s a frame, ~1,500 frames walking into one
corner) is the same since kit 60. R319: the worst frame of each 5-second
line, and a check of the hitches against the kits before today's.

**Kit 74 (2026-11-17, R320: the effects drawn).** The owner's play run on
kit 72 (port-runner-0092): c1a0e's test-chamber electricity did not show.
In the code, with `rp_emit 2` every triangle of the engine's triangle
interface (sprites, beams, particles, the client's own TriAPI drawing)
reached `RP_EmitTriangle`, which only counted it: step 4's "later
sub-steps" for them were never built, against the owner's
draw-everything rule.

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| Sprites, beams, particles (TriAPI) | ref_soft rasterises each triangle on the CPU through span tables (a few blend levels, no filtering); ref_gl makes a GL state a call and immediate-mode vertices; Dell's GLES driver writes each vertex to the TA by the CPU | each triangle the software renderer would rasterise becomes three corners of TA words: the bound frame placed in the atlas as the HUD's pictures are (`RP_PicPlace`: RGB565, or ARGB4444 where the frame has alpha), the TriAPI colour each corner's colour, the render mode the models' proven blends (alpha over, one plus one for additive, glow additive with no depth test as GoldSrc hides glows by a trace), depth compared with the world and not written; a frame with no picture drawn plainly in its colour, never dropped | the chip draws them (filtered, true blends) for ~3 words of CPU a corner; the same triangles in the same order the software renderer draws, so every effect the game makes appears |

**The emulator (2026-11-17, kit 74, c1a0 with the walk and `-rpfx`, a test
word: the gluon gun held firing, its ammunition topped up every 4 s):**
34-45 effect triangles a frame drawn, none plain, none dropped; the end
frame on the 2700G shows the gluon beam, its spiral and the blood
particles where the software renderer's picture of the same frame has
them (`local/half-life/kit-74`). Not yet on the X51v.

**Kit 75 (2026-11-17, R321: c1a0e's ladder).** Found with the emulator and
a new test word, `-rpcam x,y,z,pitch,yaw` (the player put at one spot,
noclip, through a cheat command `rp_setpos`): the ladder (`func_wall` *18,
render mode 4, `{ladder2`) drew as a solid black strip. Measured, not
guessed: 63,476 of the 103,040 see-through texels uploaded were the key,
written as ARGB1555 with alpha 0, and still drew black. Two causes, both
fixed: (1) ref_soft keeps the see-through key only in a texture's mip 0
and in three of its five surface block drawers, while ref_rp picks its
own mip a face (the mip hold), so a face at mip 1-3 had its holes lit
black: every level now keeps the key by the same rule on its averaged
alpha, and every drawer keeps it unlit; (2) the 2700G's alpha test acts
only on a punch-through object, TSP bit 14 (2700g.md "Alpha test": Dell's
GLES sets it with `GL_ALPHA_TEST`), which ref_rp never set for
see-through faces or masked model skins: set now. The emulator's model
applies the alpha test from the object word alone and still drew the
strip black, so the X51v is the check.

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| See-through ('{') textures at every distance | ref_soft: the key at mip 0 only (it draws the world at its own mip, mostly 0 near); GL: alpha in every level, alpha-tested | the key at every level by the same alpha rule, kept unlit by every block drawer; the punch-through bit with the alpha test, as Dell's driver does | ladders, rails, grates and fences see-through at any level ref_rp's mip hold picks |

**c1a0e to 30 fps (R324, the owner 2026-11-17: "how are we going to get it
to 30?", "Rule 0 it").** c1a0e plays at 13-17 fps (~65 ms a frame) where
c1a0's walk does 30: half the frame has to go. Its frames draw no more
than c1a0's and the 2700G is never waited on, so the time is the game's
own work. The levers below are the candidates; the sampled run on c1a0e
picks which, by measurement, before any is built.

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| How it is measured | GoldSrc's and our own so far: one fixed scene (c1a0's walk) as the yardstick | every kit timed on three: c1a0's walk, c1a0e's start and its cascade (a fixed-step script each), so "30 fps" is the game's worst, not one map | a lever that helps c1a0 and not c1a0e is seen as such; the worst scene sets the goal |
| Single player's client and server (loopback) | GoldSrc and Xash run a listen server even alone: every frame the server packs every visible entity (`AddToFullPack`), delta-encodes each field against the last frame into a bit stream (`Delta_WriteField`), the client reads it back, decodes field by field and copies it into its entities and prediction; Quake 3 and Source keep that path in single player too (one code path); id Tech 4 and Unreal hand a local client the state directly | the same server and client, the transport short-circuited for the local client: the server's entity states handed over as they are, each field put through the same quantization the encoder applies (the bits and scale per field from the delta tables), so the client holds exactly what decoding gave, without the bit stream, the field compares or the copies | exact by construction (the same quantized values, checked field by field against the encoder in a test mode); c1a0 already spends several percent here (`Delta_CompareField`, `AddToFullPack`, the packet copies) and c1a0e sends far more entities |
| The server's per-frame work (thinks, triggers, scripted scenes, sounds) | GoldSrc runs every entity's think at the drawn frame rate; Quake's server ran at a fixed 10-20 Hz with client interpolation, Source at a fixed tick | first the exact way: the samples name the costly entities and their work is made cheaper without changing what it computes (as the traces and the lighting were); a fixed server tick only as the owner's ruling (it changes the game's timing slightly) | exact first; the timing change is the owner's to weigh, with numbers |
| The CPU's clock | Dell runs the PXA270 at 624 MHz | the runner's past-rating step (`-rpcpu`), the divide-1 way: measured per device by the self-test | lifts everything at once; the owner's decision, kept as the last margin, not the plan. **Tried 2026-11-18 (run 88): 682.5 MHz with the bus at 195 ran clean and drew the same picture, but the cascade stayed at 9.6 / 23.3 fps (kit 86: 9.7 / 23.4 at 624): the frames are bus-bound as much as core-bound, so fewer bytes a frame is the lever, not the clock** |

**Kit 76 (2026-11-17).** (1) The owner on the X51v, kit 75: "lines next to
LOADING and some other HUD elements". The 2D pictures and the effects'
frames are tiles in the shared atlas; the filter read the texels beside a
tile at its edges (since kit 74 those neighbours are sprite frames, not
blank). Rule 0: Direct3D 9's half-texel rule, an atlas's gutter: the
texels sampled are kept half a texel inside each tile (the positions as
they were), the cheapest of the two and enough for 1:1 HUD pictures.
(2) `-rpsuit`: a map started on its own (c1a0e) gives no HEV suit and its
scripted doors stay shut; the test word gives the suit once the map plays.

**Kit 77 (2026-11-17, R324: the triangle interface).** The sampled run on
c1a0e's start (play75) put ~12% of a frame in the triangle interface's
per-corner floats: `R_SetupFinalVert` (a float transform and projection a
corner), `TriTexCoord2f` (`fmodf`, double compares and loops a corner),
`_TriColor4f` (packing a colour for the software spans the 2700G never
reads). The beams and sprites of the test chamber are made of these.

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| A corner's place | ref_soft: a float 3x4 transform, a float divide and the screen-edge clip flags for its rasteriser; ref_gl: floats to GL immediate mode, the driver transforms | the corner into view space and onto the screen in fixed point by the world's own tables (`RP_ViewPoint`, `RP_ProjectS`), so a beam meets a wall in the same numbers; only a triangle crossing the near plane goes the software's way (it clips); the 2700G clips the screen's sides | no float call a corner; a cull of wholly off-screen triangles as the software's |
| A corner's texel | `fmodf`, then loops in doubles, then a bound in doubles | the coordinate's fraction in 16.16 (the same `fmodf`-by-10 rule for whole numbers), clamped to the same 0.01-0.99, times the size | the same texel (within one, the float rounding); integers only |
| The colour | packed into 5-6-5 and the software's dither bytes each call | the 8-bit colour only (`RP_Emit2DColour`), the packing skipped while the 2700G draws | work nobody reads, gone |

What would overturn it: an effect visibly off by a pixel against kit 76
in the emulator's `-rpfx` frames, or a triangle crossing the screen's
edge that the TA does not clip (then those go the software's way too).

**Kit 78 (2026-11-17, R324: the beams and the cascade's yardstick; the
owner: "I want the cascade to still run at 30fps").** The emulator on
c1a0e's start: 562 beam triangles a frame made in full, every one off the
screen (the chamber's beams behind the antechamber's walls), the float
walk ~11% of the device's frame there (kit 75's samples).

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| Which beams are made | ref_gl culls a beam by the visible set and the frustum from its two ends (`R_BeamCull`); ref_soft culls none | the visible set (`Mod_BoxVisible`) with the box grown by the beam's width and its noise's whole reach (the fractal noise stays within 16 x its scale) | the beams behind walls cost nothing; unlike GL's ends-only box, a noisy beam's edge never pops; the noise is made before the test, so the game's random numbers are the same |
| A beam's segments | ref_soft and ref_gl: per segment a float point, noise, two cross products and normalizations, corners, then `TriTexCoord2f` and `TriBrightness` a corner | the beam's constants once in floats, the segments in 16.16 integers (normals by an integer square root and one divide), the corners straight to kit 77's fixed-point corners, the colour word from integers; sine-noise beams keep the float walk | no float call a segment; the same corners within a sub-pixel (an A/B test word, `-rpbeamfloat`, draws the float walk in the same kit) |
| Finding an entity by name | GoldSrc and Xash: a `strcmp` down the field table, then every edict in order, ~1 KiB apart, read cold | the last field's description kept (one `strcmp`), the edicts four ahead prefetched (`PLD`); the same scan, the same first match | exact by construction; the XScale has no prefetcher, so the scan stalled on every edict |
| The yardstick | a hand play (the owner's runs: 12 fps, 4.6 at worst) | `-rpcascade`: the walk's fixed step and seed, god mode and the suit, then the map's own cascade fired (`fire probe_arm_mm`, the SDK's cheat) at 4 s of game time; scored window by window (`rp-cascade: frames ...`) | the same frames every run, so kits compare on the scene the owner named |

What would overturn it: a beam that pops at a wall's edge (the visible
set's leaves are coarse: then a larger margin), or a cascade frame that
differs between `-rpbeamfloat` and the fixed walk by more than a pixel.

**Kit 79 (2026-11-17, R324: the light probes).** Kit 78 on the X51v,
`-rpcascade`: frames 1-250 21.1 fps, 251-500 6.9 fps (the debris: ~200
model meshes a frame, 123 model parts); the light probes ~15% of the run
(`R_RecursiveLightPoint`, `R_LightVecInternal`, their float calls): each
moving model five floor traces a frame (one for the light, four for its
direction), each through every brush entity near the player too.

| Part | The vendor's or textbook way (a finding) | Ours | Why ours wins |
|---|---|---|---|
| The trace down the BSP | GoldSrc, Quake and Xash: floats, a plane test, a divide and a lerp a node, two dot products a surface | the same nodes in the same order in 16.16 integers, each plane's numbers read from their float bits as met (no table, no memory), one integer divide a crossing, the luxel from the same lightmap vectors; deluxemaps (none in Half-Life) keep the float walk; `-rplightfloat` the A/B | no float call a node; the emulator's cascade frame pixel-identical to the float walk's |
| Which brush models a trace visits | Xash's extended lighting (on by default): every brush physent, each set up and walked | a brush model whose bounds (a unit wider; its radius if turned) the ray's box misses is passed by: it cannot be hit | exact by construction; most brush entities are nowhere near a given floor trace |

Measured (kit 79 on the X51v): frames 1-250 22.4 fps, 251-500 7.8 fps
(kit 78: 21.1 and 6.9).

**Kits 80-86 (2026-11-17, R324: the cascade's debris and the search by
name).** Kit 80 counted the heavy half on the X51v: 110 debris tempents a
frame, 109 model light setups, 525 light traces, 201 physics traces, about
a millisecond of CPU a piece; the calm frames 43 ms. Each kit below was
measured on the X51v with `-rpcascade`; each run's last 2700G picture
(the same game frame) was bit-for-bit the previous kit's.

| Kit | The vendor's or textbook way (a finding) | Ours | Calm / heavy fps |
|---|---|---|---|
| 81 | the HL SDK's studio renderer rebuilds every entity's bones from the animation each frame | a model with no blends, controllers, sequence blending or gait keeps its local bone matrices by (model, sequence, frame); a hundred identical debris pieces share one set and join their own transform | 22.6 / 8.3 (from 22.4 / 7.7) |
| 82 | Xash's sky check traces 8192 units world-only, then looks up the world's surface along the same line whatever the trace found | the surface looked up alone (with PM_WORLD_ONLY the trace's entity is 0 or -1, both the world); the brush boxes made once a frame in 16.16 | 22.9 / 9.1; physics traces 201 to 97 a frame |
| 83 | PM_RecursiveSurfCheck in floats | the same walk in 16.16, bevels in integers; see-through faces keep the float walk | 22.9 / 9.3 |
| 84 | | a model's five light traces sharing one brush list: no gain, taken out | 22.9 / 9.2 |
| 85 | Xash's translucent sort measures both entities' distances at every comparison | each entity measured once by the comparator's rules (its quirk kept), the keys sorted with the same outcomes | 23.0 / 9.4 |
| 86 | GoldSrc and Xash strcmp the wanted name against every edict | each edict's targetname hash kept beside the string handle it came from (cleared at each new server), strcmp only on a hash match | 23.4 / 9.7 |

Measured and not built: `r_lighting_extended 0` (GoldSrc's world-only
model light) 9.6 against 9.4, the same last picture: too small to ask for.

**The light grid (the owner's pick, 2026-11-17: "Light grid first"),
built and measured, taken out.** Quake 3 and Source light moving models
from a grid; ours kept Half-Life's floor sample but took it from the
centre of an 8-unit cell and remembered the luxel per cell (reused from no
higher up while above the floor hit; the styles combined anew; world hits
only). On the X51v the cascade's light traces fell from 520 to 280 a
frame and the light functions from ~15% to ~12% of the heavy half, but
the heavy half ran 9.5 fps against kit 86's 9.7 (the last picture the
same): the debris fly across cells faster than cells are reused, and
every miss still traces. Kits 87's other tries (a single-frame shortcut in
StudioEstimateFrame, an axis test before each dynamic light's square
root): no gain, taken out.

**The clock, then the cache (the owner, 2026-11-18: "Yes, try the clock",
then "Cut memory traffic").** Run 88: 682.5 MHz with the bus and memory at
195 ran clean but the cascade stayed at 9.6 / 23.3 (C2 sheet section 10),
so the frames wait on the bus. To find on what, the runner gained an
**event sampler** (`-rpsampleon <event>`: a sample on each overflow of a
performance counter, the PMU's interrupt, source 12, proven on the X51v
in run 89) and per-step totals of the bus waits. Runs 89-92 (cycles of
the heavy half): **the I-cache failing to deliver 44.5%**, loads in
flight 19.1%, the write buffer full 3.7%; the calm half 22% / 33% / 8%.
The debris is code-heavy, not data-heavy: each of ~110 pieces runs the
whole model path (light, bones, emit, physics), far past the 32 KiB
I-cache, so every piece fetches it again from SDRAM, at the bus's speed.
`-Os` (run 91: the same picture, I-cache 41.1%) gained 0.7%: the path is
too far past the cache for smaller code to fit it.

| Kit | The original way | Ours | Cascade fps (calm / heavy) |
|---|---|---|---|
| 88 | GoldSrc and Xash leave the I-cache to hardware replacement (round robin over 32 KiB on this core) | the runner locks the model path's hottest code (`ports/xash/icache-lock.txt`: functions ranked by the X51v's I-cache-stall samples per byte, run 92) into the I-cache, 8 ways of every set left free, **only while the engine says its frames are model-heavy** (`rp_code_hint`: from 40 models drawn, off below 24) | **23.4 / 10.8** (run 96, the kit as built: 24 KiB, no profiling; with the event sampler on: 24 KiB 10.5, 16 KiB 10.4; held throughout, 16 KiB: 22.7 / 10.6; none, the same profiling: 23.3 / 9.6) |

| 89 | GoldSrc and Xash light each model as it is drawn, the light code between every piece's draw code | **the lighting as a stage of its own** (the owner, 2026-11-18: "Debris stage by stage"): `RP_LightPrepass` runs `R_EntityDynamicLight` for every studio model of the draw list before any is drawn, so its code stays cached across all of them; each draw takes the stored result only when every input is bit for bit what the pass saw (origin, model, effects, render fx, switches, time, the world's light key, every dynamic light) and replays exactly what the function wrote; `-rpnopre` off. With it the I-cache lock no longer pays (24 KiB: 11.8; 8 KiB re-ranked: 14.4; none: 14.7), so the kit locks nothing by default | **24.3 / 14.7** (run 100; 108 of 108 debris lights from the pass, the heavy half's I-cache share 41% -> 22%) |
| 90 | GoldSrc and Xash walk the light ray down `mnode_t` (52 bytes) and its float plane (20 more, elsewhere); kit 79 read those floats' bits into 16.16 at every visit (4 conversions a turned plane, 8 a surface tried) | **the light walk on packed fixed-point nodes** (the owner, 2026-11-18: "Cheaper light traces"): once a map, 24-byte light nodes (16.16 distance, 2^30 normal, children by number, surface range) and each texinfo's lightmap vectors in 16.16 (checked surface by surface), ~90 KiB on c1a0e, freed at the next map; the same integer sums in the same order, so the same nodes, surfaces and luxels; `-rpnolnode` off. With it, kit 89's dlight check compares only the dynamic lights live in the snapshot or now (one out in both adds nothing) instead of all 1,280 bytes a model | **24.4 / 15.1** (run 102; the light walk 6.8% -> 2.8% of the play, memcmp 1.5% -> gone; 9,732 light nodes and 2,944 surfaces a heavy frame) |
| 91 | GoldSrc and Xash find an entity by name testing each edict's validity, then its name (kit 86 added a hash before strcmp): two cache lines an edict ~1 KiB apart | **the name's handle first** (the owner, 2026-11-18: "Index entity searches"): when the edict's targetname handle is the one its cached hash was made from and the hash is not the wanted name's, it cannot match valid or not, so it is passed on that one line; only a new handle or a matching hash reads `free` and goes the old way; the same order and first match, proven by strcmp | **24.9 / 15.5** (run 103; the search 3.0% -> 1.4% of the play) |
| 92 | Xash compares every field of every entity the client has, every frame, to find the few that changed, and the client rebuilds its physics lists from every entity three times a frame | **ADR-0040 step 7a, the direct link's first part:** a state equal byte for byte to the client's is passed on one block compare (Xash would compare each field, find none changed and take back what it wrote, so the message is the same), and the client's physics lists are kept while the frame they were built from is the same (three builds a frame -> one a frame parsed); `-rpnodirect` off | **25.6 / 15.6** (run 117; 96% of the states passed on the compare, 2 of 3 list builds kept; the calm half 1.7 ms a frame faster; the debris are client temp entities, so the heavy half has none of this work) |
| 93 | GoldSrc and Xash run all their code from SDRAM, the I-cache refilled by round-robin replacement; the consoles of the time hand-placed overlays in a scratchpad; today's tools lay code out by profile | **ADR-0040 step 7b, the hottest code on the PXA270's 256 KiB on-chip SRAM:** the X51v's own profile (run 118) ranks the functions by samples a byte (`ports/xash/sram-hot.txt`, the float and division routines' sections included), the linker gathers them into whole 64 KiB pages a module (`.rp_hot`: the engine 192 KiB, the client 64), and the runner maps those pages onto the SRAM by the page table (`hot_code_on_sram`: the same addresses, no veneers), taking them off before a module load and at the end; `sram.bin` absent or `RETROPANE_XASH_SRAM=0`: off | **27.4 / 17.7** (run 119, against the same kit without it, run 118: 25.6 / 15.5; the code of ~81% of the play's samples on the SRAM) |
| 94 | GoldSrc and Xash trace the floor under every model every frame, plus four traces around it for the light's direction (~550 a frame for the cascade's ~110 pieces); Quake 3 lights models from a grid its map compiler bakes | **option C, behind `-rplightgrid` for the owner to see first:** a lazy floor-light grid (32 x 32 x 64 units, points made by the game's own trace on demand, kept while the light key holds), temp-entity models lit by the eight points around them and their direction from the grid at the game's four offsets; monsters and the view model keep the trace. Off by default | off: **27.5 / 17.7** (run 126, the picture run 86's); on: 27.6 / 16.4 (run 124: slower as built, 312 points made a frame) |
| 95 | (kit 94's grid kept each point's colour, so every light-style change traced it again) | **the grid's points keep their world hit** (surface and luxel, read back from both surface samplers into `rp_cap`) and re-read the luxel when a style changes; blends in integers; brush-model hits keep their colour while the light key holds; a world hit is traced again after 30 frames | grid on 27.6 / 17.7 (run 127; traces 519 -> 132 a frame), off 27.5 / 17.6 (run 128, the picture run 86's): **no gain**, the traces were already cheap after kit 90 |

The picture of every run above byte for byte kit 86's. What is left in
the I-cache share (41% of the heavy half with the lock) is the path's
breadth: the next lever is running the debris stage by stage (all pieces
through one stage's code, then the next), not more locking (16 to 24 KiB
gave 1%).

What would overturn it: a TA fan that does not draw as the strip does
(then fans go as strips: a fan of n corners is n - 2 triangles either
way, a few more words); skins that wrap (s, t past the skin: then that
skin gets a power-of-two texture of its own with wrap).

## Step 7c: the map cooked on the X51v (ADR-0040 parts 2 and 3, kit 103, 2026-11-18)

**Where the world's time went (run 146, kit 102, the cascade's calm
window):** 9.7 ms of the frame in the world and brush-entity bucket. Of
it, the BSP walk 1.03 ms (`R_RecursiveWorldNode` 0.67, `RP_WalkPlane`
0.36: a 52-byte float node a visit, its box's floats turned into 16.16
again every frame), `R_BeginEdgeFrame` 0.72 ms (clearing the edge
renderer's 480-line tables once for the world and once per brush entity,
tables the 2700G path never reads), `R_TransformFrustum` 0.42 ms (software
floats, made again after every brush entity from the same inputs), and
the faces' own path (`RP_ChipFaceFixed` 1.74, `RP_ProjectS` 0.65,
`RP_EmitFace` 0.36): about 300 faces a frame, the first touch of each
face's corner run a cache miss. `R_DrawSegs` and `RP_FracNoiseFix` (~1.9
ms) are the chamber's beams, not the world.

**Rule 0, written before building.**

| Part | GoldSrc / Xash / console ports (findings) | Ours | Why ours wins |
|---|---|---|---|
| The map's format | Xash loads the BSP as stored, floats everywhere, and recomputes integers a frame; the PS2 port and fixed-point Quakes converted maps on a PC; modern engines cook in a studio's build | cooked on the X51v at the map's first load into `maps/<map>.rpw` beside the game (the writable game folder): integer planes (only the 793-1,198 the world's nodes use, of 11,746-18,972), the world's nodes in preorder as 32-byte records (a cache line), leaves, 16-bit leaf face lists, a byte of fixed facts a face, face boxes, and the vertex and corner tables | any map or mod, nothing prepared on a PC (turnkey); the walk reads one line a node and no floats |
| Staleness and trust | (Valve's own checksums are CRC32 for multiplayer consistency) | the BSP's MD5 from the buffer the engine already hands the renderer (no second read, 62-73 ms) plus its length and counts and the cook's version; the file is untrusted (ADR-0039 rule 10): every index bounds-checked, a child always after its parent (no cycle can be written), the walk's array capped at the map's records; anything wrong: cooked again | a stale or damaged file can only cost a re-cook, never a wrong index |
| The walk | every frame from the root over every node in the PVS | the PVS's nodes and leaves copied into one compact array only when the view enters another cluster (`tr.visframecount`); each frame walks it with the engine walk's own integer tests in its own order (axial planes inline, both children's lines asked for ahead, the leaves holding static entities flagged in the array and made again when any leaf's efrags change: `rp_efrag_gen`) | the same faces in the same order (word for word), from a few hundred cache lines |
| The edge tables | `R_BeginEdgeFrame` clears them for the world and every brush entity | not cleared while the 2700G draws alone (`rp_emit 2`, no frame drawn both ways), where nothing reads them | 0.72 ms of nothing |
| The frustum | `R_TransformFrustum` in floats after every brush entity | two remembered slots (the world's view, the last entity's), keyed by the inputs' bits; the same rows give the same normals, a new eye only the four dot products | exact; the floats once a frame for the world |
| A face's fixed facts | three structures read and a call into the engine (`Mod_SampleSizeForFace`) a face a frame to decide R297 B | one cooked byte | fewer misses a face |

**Measured on the X51v (kit 103, against kit 102's cascade 28.7 / 18.7
and fight 19.6 / 21.9 fps):**

| Run | What | Cascade fps (calm / heavy) | Fight fps (first / second half) | Picture |
|---|---|---|---|---|
| 149 | the edge clear and frustum memo only (the cook failed: the world reaches 1,323 of c1a0e's 2,169 nodes, the rest are the brush entities') | 28.9 / 19.1 | | |
| 150, 151 | the cooked walk (first load cooks 11.5 ms, writes 0.4-0.5 s) | 29.0 / 19.2, 29.1 / 19.2 | | byte for byte run 146's |
| 152 | `-rpfacecull` (faces wholly off the screen not sent: 52-59 of ~197 a frame) | **29.6 / 19.6** | | **4,458 pixels differ**: the mip hold remembers when a face was last drawn, so skipping off-screen faces changes later texel choices |
| 153, 154 | the fight, every cooked array kept in RAM (~750 KiB on c2a5f) | | 20.6 / -- | **out of memory** (the heap had ~50 KiB in holes) |
| 155 | `-rpnocook` (the control) | | 19.5 / 21.8 | |
| 156, 157 | the lean cook (only the world's planes and nodes, face boxes kept only for `-rpfacecull`, the file read a section at a time, the walk's array grown on demand): 168 KiB on c2a5f, 84 on c1a0e | | **19.8 / 22.2, 19.7 / 22.1** | fight pictures differ between runs of one kit (156 vs 157: 3,763 pixels; 148 vs 155, both kit 102's walk: 358), so not comparable |
| 158 | the same kit, the cascade (cooked again for file version 3) | **29.1 / 19.2** | | **byte for byte run 146's** |
| 159, 160 | **kit 104: `-rpfacecull` the default** (the owner, 2026-11-18, after seeing runs 146 and 152 side by side: "Yes, turn it on"; `-rpnofacecull` the control; the face boxes now kept in RAM) | **29.6 / 19.6** | **19.9 / 22.4** (48-56 of ~110 world faces a fight frame off the screen) | run 152's, the new reference |

The world's bucket of the calm window: 9.7 -> 8.6 ms (run 151: the walk
0.44 ms against the walk plus the edge clear's 1.75; the frustum's floats
gone). Reading the cooked file (104-185 ms) is slower than cooking it
(9-19 ms): the cook itself is cheap because the engine has already parsed
the BSP. What the file buys is a place for the cooked parts that are dear
to make (7d's integer hulls, textures in the 2700G's layout: the map's
start spends ~1.2 s in `GL_UploadTexture` and `RP_TexPlace`), so it stays.

**Left in the world (the next levers, each with its own run):** the
faces' corners (`RP_ChipFaceFixed` + `RP_ProjectS` ~2.3 ms: iWMMXt
transforms, ADR-0040 part 3), the brush entities' float path
(`R_RotateBmodel`, the float side test a face), and `-rpfacecull`, ruled on by the owner and the default from kit 104
(its changed texel choices: median 4 of 255 in one channel, not visible
at 6x; `local/half-life/facecull/`).

**Kit 105, the corners on iWMMXt (the owner, 2026-11-18: "do the corners
on iWMMXt next"), measured and not kept.** Rule 0 first: Dell's GLES
driver transforms with TMIA (`0x22780FC`), the Dreamcast's SH-4 with
FTRV; iWMMXt's SIMD lanes multiply 16 bits, so 32-bit corners take TMIA
(32 x 32 into a 64-bit wR register, one at a time), not the lanes, and the
16-bit lanes would change the picture. Run 151's samples first:
`RP_ProjectS` (0.57 ms) is arithmetic, but 64-bit products TMIA cannot
pair; `RP_ChipFaceFixed` (1.8 ms) waits on loads, its kit-69 hint itself
reading the next face's record. Built: the 3 x 3 sums of `RP_ViewPoint`
and `rp_bone_point` by TMIA (wR0-wR2, read back by TMRRC), the projection's
64 x 32 products as 32 x 32 where x / z fits, and a look-ahead from a
cooked table of each face's corner run (no record read). On the X51v:
**exact** (run 161, `-rpwmmxcheck`: 1,352,707 sums both ways, 0 differed)
but **no faster**: TMIA on 29.5 / 19.4 (run 162) and off 29.5 / 19.4 (run
163, the same kit), the kit as a whole 29.5 / 19.4 and fight 19.7 / 22.2
(run 164) against kit 104's 29.6 / 19.6 and 19.9 / 22.4; its picture 1
pixel (one green step) from kit 104's with TMIA on or off. Reverted to
kit 104. What it shows: the world's and models' corners are a few
thousand a frame and their multiplies are not where the frame waits;
iWMMXt pays where 16-bit data streams (the mixer, kit 99), not here.

**Kit 106, the owner's rulings (2026-11-18):** the debris lighting
options A and B (measured on kit 91, runs 109-112, watched by the owner)
and a 20 Hz server tick with client prediction (runs 113-116) are the
defaults; `-rpnolighthalf`, `-rpnolightone` and `+sv_fps 0` the controls.
The yardsticks now run with `+cl_nopred 0`, the engine's own default
(`local/half-life/yardsticks/`). Runs 165-166: cascade **29.8 / 22.9**,
fight **21.6 / 23.8** fps against kit 104's 29.6 / 19.6 and 19.9 / 22.4;
run 165's end picture is the new reference. The calm frames are at 30;
the heavy half (44 ms against 33) is the debris' own path: the next
lever (roadmap 7e).

**7e, where the debris' time goes now (kit 106, 2026-11-18).** Run 165
(the 1 kHz sampler): the heavy half costs 9.3 ms a frame more than the
calm one for ~110 pieces, ~85 us (53,000 cycles) a piece: lighting ~3.4
ms (`R_EntityDynamicLight` +0.86, `R_StudioEntityLight` +0.83, the sky
trace +0.40, `RP_PreDlSame` +0.38, `RP_LPlane` +0.31, a 64-bit division
+0.22), drawing ~3.0 ms (`RP_StudioCmdSend` +1.02, `RP_StudioCmdLit`
+0.80, `RP_StudioVertsFixed` +0.50; no instruction above 14 of their 696
samples: the cost is spread, not a hot spot), bones ~1 ms (the client's
float matrices: game code, unchanged). Run 168 (`-rpsampleon 1`): the
I-cache now takes 14.6% of the heavy half's cycles (44.5% before kits
89-93), so staging the draw no longer has a big share to win. Runs 169-172:
the yardstick without any sampler gives the same 29.8 / 23.0 as with the 1
kHz sampler (the sampler costs nothing measurable); run 168's 30.5 / 24.4
did not repeat (run 171: 29.8 / 22.9), a one-off, closed by the stop rule.
What it means: the exact per-piece levers left are each 0.3-0.9 ms; all of
them together are worth ~3 ms (heavy ~23 -> ~25 fps). 30 on the heavy half
needs ~10 ms, about the debris' whole cost, so it needs a change to what the
debris does (fewer pieces drawn, lit more simply), which is the owner's
call.

**Kit 107, 7e's exact fixes (the owner: "Exact fixes, then options").**
The dynamic and entity lights change only through the engine's writers
(allocate, decay, clear, the test lights), which now give `rp_light_gen`
a new number; each frame's live lights are listed once and kept while it
holds (`R_StudioEntityLight` no longer scans 128 slots for every piece,
`R_EntityDynamicLight` 32, and the pre-pass's copy is known equal while
the number has not moved). The light walk's 64-bit division is 17 steps of
shift and subtract (checked on the PC against C's over 2 million pairs).
Run 173 (`-rplightcheck`): 80,414 lists and answers checked against the
scans on the X51v, 0 differed. Runs 176-177: cascade **29.9 / 23.8**,
fight **21.8 / 24.0** (kit 106: 29.8 / 23.0 and 21.7 / 23.9). **The
cascade's end picture varies between runs of one kit** in a 7-pixel spot
at (546, 235) (runs 165 and 169 one way, 172 the other, kit 106 both):
kit 107's runs match one or the other exactly, and kit 105's one-pixel
"difference" was likely this spot.

**Option (a), shown to the owner, off by default: `-rpdebrisN`** draws at
most the N nearest debris pieces a frame (studio temp entities); the rest
still exist and move (their physics is the game's) but are not lit or
drawn. Runs 188-189 (full cascade, calm / heavy): 55 nearest **29.9 /
27.5**, 35 nearest **29.9 / 28.8** (all: 29.9 / 23.8). Pictures at cascade
frame 300 (runs 185-187, `local/half-life/debriscap/`): 1,652 and 2,121
pixels differ from all pieces drawn; the far clump on the lit wall
vanishes. That vanishing is against the owner's standing rule that
nothing vanishes (a missing feature draws plainly), so the next variant
draws the far pieces plainly (unlit) instead of leaving them out.

**Option (a), drawn plainly (the owner: "Yes, build it"), measured and
not kept: `-rpdebrisplainN`.** The N nearest pieces lit and drawn as now,
the rest kept on the screen: their light one sample a 64-unit cell (the
cell's first far piece samples it the game's way; vendor: every model
sampled at its origin every frame; modern: light probes; ours: probes made
only where far pieces are, this frame only), no entity lights, one flat
shade a mesh (the game's own `STUDIO_NF_FLATSHADE`). Run 190 (frame 300,
`local/half-life/debrisplain/`): 1,312 pixels differ from all lit (the
cap: 2,121), the far clump still there, a little flatter. **But slower:
runs 191-192 (full cascade) 35 lit 29.8 / 22.6, 55 lit 29.8 / 22.7,
against all lit 29.9 / 23.8.** Why (run 191's samples, heavy minus calm a
frame): the far pieces' cost is their meshes (`RP_StudioCmdSend` +0.98,
`RP_StudioCmdLit` +1.03, `RP_StudioVertsFixed` +0.61 ms) and the client's
bones (`ConcatTransforms` +0.48), which a flat shade does not touch; the
cells paired only 71 far pieces into 43 samples, and a far piece's sample
skips option A's every-other-frame light, so the lighting got no cheaper
(`R_EntityDynamicLight` +0.59, `RP_ProbeLight` +0.34). What it means: the
cap's gain is the meshes and bones it leaves out; drawing every piece
plainly and fast needs a cheaper shape for the far ones (a flat stand-in
of a few triangles, no bones), not cheaper light. Off by default.

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
| 3 | The runner's `rp_gfx_*`, the world on the 2700G with the surface cache | **done:** run 11, 95 ms (10.5 a second); the picture right on the X51v (models: 4a, run 12) |
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
