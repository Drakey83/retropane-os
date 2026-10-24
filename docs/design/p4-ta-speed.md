# P4: the tile accelerator against CPU-built lists (a plan for the owner's vote)

**Status: proposed 2026-10-24, not built.** The gate's P4 (phase 5):
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

## What it decides

Nothing by itself (results, not decisions). It gives the owner the
crossover count (where the TA's lower CPU cost outweighs its set-up), and
the numbers the GLES layer's design needs.
