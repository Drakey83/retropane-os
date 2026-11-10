# Decisions for the owner, 2026-11-10

From the owner's list of 2026-11-10 (branch `fix/deep-dive-2026-11-10`).
Each box is one decision; the evidence is linked. Nothing here is built
until its box is ticked.

## Item 1: the picture cache and the frame rule

- [ ] **A return from Hourly still uploads Today's cards.** **Why:** with
  your 384 / 384 KiB split, Today's own pictures (about 265 KB) stay, and on
  the X51v the return to Today uploaded 1 picture, not 9 (run
  [today-0115](runs/2026-11-10-today-0115.md)). But Hourly's own pictures
  (its graph 214,080 bytes, the hour's picture 102,400) and Today's set
  don't fit 384 KiB together. So a route through Hourly uploads up to five
  17,424-byte cards on the way back (the host route test: 5 pictures,
  87,120 bytes; the old cache uploaded 89,424 bytes, so this case barely
  improved). At the bus's measured 30-33 MB/s that is about 2.7 ms of the return
  frame. **Options:** (a) accept it (2.7 ms, only after Hourly); (b) find
  ~240 KiB more for pictures elsewhere in the 2700G map (your ruling
  said nothing else moves, so this reopens it); (c) draw Hourly's graph
  smaller (its 214 KB picture is 446 x 120 at 32 bits; at 16 bits it would
  be 107 KB, but the graph's soft fill needs its alpha, so this needs a
  measurement first).
