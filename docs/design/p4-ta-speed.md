# P4: the tile accelerator against CPU-built lists (decided 2026-10-24)

**Status: decided by the owner 2026-10-24 (Q1 A, Q2 A, Q3 the synthetic
quads); the owner's go to build it now, 2026-10-25 (the divide-1 work
moved to `rp-gpubench`, so moving the TA code out of `rp-glesfeat` no
longer collides). Rule 0's design below, before any code.** The gate's P4 (phase 5):
"the TA's speed against the CPU-built lists at 100 / 1,000 / 5,000 quads".
ADR-0026 D1 already chose **both** (C): the interface on our CPU-built
objects and lists, games on the tile accelerator. P4 measures where the
line between them falls, and how much CPU each costs a frame. It isn't a
"which path" vote.

## What exists, and what stands in the way

- **CPU-built lists:** `rp-mbx3d-render` keeps a retained scene of at most
  **64 quads** (`MAX_SLOTS`), the interface's need. 1,000 and 5,000 quads
  are past it by design, so a measurement needs a test-only CPU binner: a
  quad's tiles worked out, objects written, a list per tile. `rp-glesfeat`
  G2's "per-tile lists" way is a small one of these.
- **The TA path:** `rp-glesfeat`'s `through_ta` (proven at Windows Mobile's
  clock: G, G2 ways 7-8, M) sends at most 4 draws of 96 words, with
  `rp-glesfeat`'s own memory layout. 5,000 quads means streaming ~100 KiB
  of vertices through the 4 KiB TA window, which shares the 2D engine's
  FIFO (ADR-0030 T8), waiting on its level.
- **C53:** after a TA hang a core reset doesn't bring the TA back. An
  unattended run that hangs the TA leaves the rest of it blind.
- **The divide-1 work** is in `rp-glesfeat` now. Moving its TA code out
  would collide with it.

## The run, as proposed (`rp-tabench`, a new test image)

At Windows Mobile's clock, into a 16-bit target, each count drawn 5 times
and every picture checked against the reference:

| Way | 100 | 1,000 | 5,000 quads |
|---|---|---|---|
| CPU binning (test-only binner, objects and lists by the CPU) | CPU time, render time | same | same |
| The TA (vertices streamed, the TA bins) | CPU time, TA time, render time | same | same |

Quads 16 x 16, textured (a game's sprite or wall-strip size), scattered,
plus a second content set of 64 x 64 quads (the interface's size). Logged
for each: the CPU's time building the frame, the TA's or the binner's
time, the render time, the parameter memory used. A TA that hangs is
recorded as such, and the run moves to the next way (C53: the TA counts
stay as far as they got).

## Rule 0: what Dell does, and the better ways this run measures (2026-10-25)

**Dell's way (a finding):** the GLES driver's transform stage writes every
vertex into the TA window with the CPU, word by word, 7 words a textured,
coloured vertex (`TEX_VERTEX_WORDS`), a state block before each draw. The
bus into the 2700G is measured at 30-33 MB/s for CPU stores and the PXA's
DMA alike (2700g.md: a 600 KiB screen in 18.5 and 20.0 ms). A frame of
5,000 textured quads as Dell feeds it (a strip of 4 vertices, 29 words a
quad, plus state) is ~580 KB: **~17 ms of bus a frame, all of it CPU
time**, before any binning or rendering: at 30 fps that's half the frame
budget (the port-fps rule) gone to copying.

**The ways measured beside it, each a lever Dell never pulled:**

1. **Feed by DMA, the CPU free:** vertices built in SDRAM (or the PXA's
   internal SRAM, as E1e staged textures) and moved by the DMA controller
   in pieces no larger than the FIFO takes, the FIFO level (ADR-0026 T8:
   shared with the 2D engine) checked between pieces. The CPU builds the
   next batch meanwhile. Unknown: whether the TA window accepts DMA bursts
   (it's a register window; 32-byte bursts reached local memory in BX).
2. **Fewer words a vertex:** the state block's mask chooses the vertex
   fields (`0x10000053` untextured, `0x10000057` textured with a colour);
   a sprite tinted white needs no colour word, so 6 words instead of 7
   (-14%); x and y as the only changing words across a strip of quads
   that share z, 1/w is a further test of which fields the TA defaults.
   Measured: the bytes a frame and the picture right.
3. **State once per batch, not per draw** (ADR-0026 rule 3): one state
   block for every quad sharing a texture, the strips back to back.
4. **Our CPU binner where it wins** (ADR-0026 D1 C, rule 4): a static
   scene's objects and lists are written once and reused (rule 1: only
   changed tiles re-rendered), so its bus cost a frame can be ~0; the run
   logs the first frame and a repeated frame separately, which the TA (a
   fresh stream every frame) can't match.

**Why ours should win:** at 5,000 quads the TA path's cost is the bus, not
the binning; DMA takes it off the CPU entirely and the smaller format cuts
it, so the CPU time a frame should fall from ~17 ms (Dell's) toward the
time to build the vertex batch in SDRAM; the crossover against the CPU
binner is then where the binner's per-tile writes outgrow the TA's
stream, measured, not assumed. Each way's picture is checked against the
reference: a faster feed that draws wrong is no win.

**ADRs checked (ADR before code):** ADR-0026 rule 2 (the CPU sleeps while
the GPU works: the run times the CPU's own work separately from the TA's
and the render's); rule 3 (state once: way 3); rule 4 (both paths, each
where it wins: this run finds where); rule 5 (one owner: a test image,
under the loader, as every 3D test); D3 (no interrupt from the 3D core,
T2: the TA's done bit is polled with a timeout, as `through_ta` does);
T8 (one FIFO with the 2D engine: the 2D engine idle during the run, the
level checked before each piece). ADR-0035 (the 3D clock: Windows
Mobile's, 99.67 / 2, as decided for P4's first run).

## The build, in order

1. **Move the TA path into `rp-mbx3d`** (Q1 A): `through_ta`'s steps (the
   buffers zeroed, the start table, the stream and end-window writes with
   their offsets counting on, the scene end, the done poll, the region
   array handed to the 3D stage) as a driver function taking the stream
   as a slice of any length, sent in FIFO-sized pieces; `rp-glesfeat`
   then calls it (one copy). Host-tested against `rp-glesfeat`'s proven
   word sequence (the same writes for its four draws).
   **Built 2026-10-25:** `rp-mbx3d`'s `ta::feed` (`drivers/mbx3d/src/ta_feed.rs`),
   with `ta::clear` and `ta::Layout`; `rp-glesfeat`'s `through_ta` calls
   it, its 96-word and 4-draw limits gone. Rule 0 for the feed: Dell's
   GLES driver writes word after word and lets a full FIFO stall the bus;
   ours reads the FIFO's room once a piece and writes that many words (a
   credit: one status read per up to 510 words, each read crossing the
   same slow bus), and a FIFO full for the limit ends the scene with its
   status instead of hanging the CPU (C53's evidence kept). It records the
   status reads, the deepest FIFO level seen (ADR-0026 T8's evidence: do
   TA words show in the 2D engine's level?), the feeding time and the
   TA's time. Host tests: the exact writes of `through_ta` for four draws,
   both windows wrapping at 4 KiB, a full FIFO waited out with its room
   used whole, a FIFO that stays full, a TA that never says done.
   Found: the TA start table's `0x85C` word (`0x000400E1`, Dell's)
   already has bit 7, the scene-end bit, on, so the scene end's write
   repeats it; whether Dell's GLES driver clears bit 7 for a scene's
   body is open (an audit question, C78). Not yet on the device: the
   status reads between window writes are new there.
2. **The streams:** textured quads as strips, state once per batch, the
   6-word vertex (no colour) as a second form; host-tested word by word.
   **Built 2026-10-25:** `rp-mbx3d`'s `ta::quads` (`ta_quads.rs`), a
   lazy iterator: each word is worked out when `ta::feed` asks for it, so
   a frame goes from the quads (40 bytes each) straight into the TA window
   with no 150-190 KiB copy built first (Dell's transform stage builds
   vertices in a buffer and copies them; `ta::feed` now takes any stream
   of words and primitive ends, `ta::draws` the slice form). Forms:
   `Coloured` (Dell's 7 words), `White` (6, no colour); batching
   `PerQuad` (Dell's: 38 words a quad, 190,000 for 5,000 quads) or
   `Once` (state once: 26 words a quad with `White`, -32%). Host tests:
   the state block and strip word equal the proven `textured_stream`'s,
   the corners in G2's strip order, the white form only dropping the
   colour, the counts, a 40-quad stream through the feed.
   **Open (static reading, time-boxed 2026-10-25):** the GLES driver's
   transform stage sizes a vertex on the CPU side (`0x02272064`: 16
   bytes; +4 when context `+0x5E0` is set (the colour), +8 a texture
   unit), and the state-word builder (`0x02267700`) shows no bit for it;
   how the TA learns a vertex has no colour isn't read yet (the clear's
   TSP `0x22206F80` gives 3-word vertices with the same mask, so the TSP
   likely selects the fields). The run's `White` way carries the
   candidate TSP and checks the picture (reminder R38). Also read there:
   Dell's emitters jump back to the window's start rather than split a
   vertex across its end; word stores don't care, but step 4's DMA pieces
   must end at the window's end.
3. **The CPU binner** (test-only): a quad's tiles from its bounding box
   (8 x 16 tiles), objects through `rp-mbx3d-render`'s encoder, a list per
   tile, the region array; host-tested against the model of the 3D core
   (`rp-mbx3d-render::model`): its pictures equal the TA reference's.
   **Rule 0 for the binner (2026-10-25, before code).** *Dell's way:* his
   driver's flat scenes give every tile the one object list (no binning:
   each tile reads every object); *the textbook's / the TA's:* a list per
   tile grown as objects arrive, in linked blocks (the TA's 4-byte tail
   pointer a tile), so list words are scattered over memory and each
   block costs a link word. *Ours:* two passes, a counting sort: pass 1
   counts each tile's quads from their bounding boxes, a running sum
   gives every tile's list its exact place; pass 2 writes each quad's
   index into a scratch array in SDRAM (2 bytes an entry); then every
   tile's list is written in tile order, one block, end to end, so the
   CPU's stores to the 2700G go out in address order (the write buffer
   merges them) and the core reads each list straight through. No link
   words, no overflow, memory exactly the entries (5,000 16 x 16 quads:
   about 20,000 entries, 40 KiB of scratch); quads in a tile in draw
   order (the sort is stable). Tiles with no quad share one list (the
   background alone). Objects: each quad's two one-triangle objects
   through the renderer's own encoder (one copy, proven on the X51v and
   modelled), written once, in order. *Why it wins:* the TA must take
   every vertex every frame (Dell: ~17 ms of bus at 5,000 quads); a
   scene binned once is re-rendered with no words written at all (the
   repeated frame), and a first frame writes each object once plus about
   4 bytes a tile it touches. *Measured, not assumed:* the first frame's
   CPU and bus time against the TA's in `rp-tabench`. *Later, if it pays:*
   one 4-corner textured object a quad instead of two triangles (33 words
   instead of 52, half the list entries) once the TA's own textured quad
   object is logged in `rp-tabench` (its layout isn't modelled or
   proven).
   **Built 2026-10-25:** `rp-mbx3d-render`'s `binner` (`bin`,
   `start_table`, `Layout`, `Scratch`, `Binned`), the caller's SDRAM as
   scratch, no allocation; the quad's tiles from the renderer's own
   bounding-box helper (one copy). Host tests on the model of the 3D core:
   60 mixed quads (flat, Gouraud, textured, blended, partly off screen)
   draw exactly the renderer's picture; 250 16 x 16 quads binned draw
   exactly what every tile reading every object draws (Dell's way, the
   reference); draw order kept in every tile; a repeated frame stores
   nothing; no background leaves empty tiles white; scratch or parameter
   memory too small refused, not overrun. Mutant: pass 2 in reverse draw
   order fails three tests.
4. **`rp-tabench`:** at 99.67 / 2 into a 16-bit target, 100 / 1,000 /
   5,000 quads of 16 x 16 and of 64 x 64, five frames each: the CPU
   binner (first frame and a repeated frame), the TA fed by CPU stores
   with 7-word vertices (Dell's), with 6-word vertices, and by DMA; CPU
   time, TA time, render time, bytes, parameter memory, each picture
   checked against the reference; a TA hang recorded and the run moved on
   (Q2 A); the step on screen from the first moment; the log on the card.
5. **Emulator check and mutants**, then the owner's go (attended for its
   first minutes: a TA hang may need a reset).

## The choices for the owner

**Q1. Where the TA code lives.**
- **A (recommended): moved into `rp-mbx3d` (the driver) when the divide-1
  work is done with `rp-glesfeat`.** One copy, shared by the test images
  and the games' GLES layer later. Minus: waits for the divide-1 work.
- B: copied into `rp-tabench` now. Starts at once. Minus: two copies to
  keep in step until A happens anyway.

**Q2. C53 first?**
- **A (recommended): no, the run records a hang and moves on.** It's
  attended for its first minutes (a TA hang may need a reset). Minus: a
  hang ends the TA's numbers early.
- B: fix C53 first (a TA that survives a core reset). Minus: unknown size:
  it needs the TA's reset path, unread.

**Q3. Content.** 16 x 16 and 64 x 64 textured quads, as above. Or your
game's real mix, once a port (Doom, Heretic: apps-and-packs §6) gives one.

## The owner's votes (2026-10-24)

- **Q1: A.** Move the TA code into `rp-mbx3d` once, after the divide-1
  work: two copies would drift.
- **Q2: A.** No C53 fix first: measure; a hang that ends the numbers early
  is the evidence that C53 matters, and then it's fixed with that evidence.
- **Q3: the synthetic quads** (16 x 16 and 64 x 64 textured): no port
  exists yet; the two sizes bracket real work.

## What it decides

Nothing by itself (results, not decisions). It gives the owner the
crossover count (where the TA's lower CPU cost outweighs its set-up), and
the numbers the GLES layer's design needs.
