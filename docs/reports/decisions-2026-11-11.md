# Decisions for the owner, 2026-11-11

From the owner's list of 2026-11-11 (branch `fix/deep-dive-2026-11-11`).
Each box is one decision; the evidence is linked. Nothing here is built
until its box is ticked, and no box here is decided by me: where the
reviewer recommended an answer, it is written as his recommendation.

## Item 1: the frames R191 leaves over the rule (the rule itself stays)

- [ ] **Today's whole list costs more than the 45 ms allowance.** **Why:**
  run [today-0137](runs/2026-11-11-today-0137.md): the start-up burst and
  Settings' redraws on network answers are fixed, but the return to Today
  takes 55.5 ms (706 blits, 278,442 blended px) and the Start menu over
  Today 38.5 ms (610 blits, half the screen, held to 33 ms). The cost is
  Today's list as the 2D engine draws it (glass, the word shadows, the
  living sky's clouds as glyphs), not the rule: drawn whole, Today takes
  47-55 ms on its own. I didn't change the rule. The first Light look
  switch on Settings took 45,088 us (88 us over), building 5 glyph
  atlases and 94 glyphs in that one frame. **Options:** (a) keep Today's
  last picture when leaving it, in the 2700G's spare memory or the RAM
  pool, so the return is a copy (the Reveal machinery already composes the
  Weather app ahead); (b) cut Today's blits: draw its text runs and word
  shadows as fewer, larger blits, or prebuild each look's atlases while
  still; (c) leave them over the rule, logged. Each is a design change
  (ADR-0023's renderer or Today's list), so nothing is built until you
  pick.

## Item 2: a return from Hourly re-uploads Today's cards (carried from decisions-2026-11-10.md, item 1)

- [ ] **What to do about the five cards a return from Hourly uploads.**
  **Why:** the picture cache is 384 KiB (393,216 bytes; the owner's split
  of 2026-11-10). Today's own pictures are about 265 KB (the hero 176,000
  bytes, five cards 17,424 each, small ones); Hourly's are 316,480 bytes
  (its graph 214,080, the hour's picture 102,400). Together, about 581 KB:
  they don't fit, so whichever was drawn last pushes the other out.
  **Measured so far:**
  - **On the host, the worst case** (the route test, every page drawn by
    the 2D renderer on screen 0): the return through Hourly uploads 5
    cards, 87,120 bytes; the old 256 KiB cache uploaded 9 pictures,
    89,424 bytes. At the bus's measured 30-33 MB/s (DMA), that is
    2.6-2.9 ms of the return frame. A route that skips Hourly uploads 0.
  - **On the X51v** (run today-0115, the look route, which visits Hourly):
    the return to Today uploaded 1 picture, not 5. With the plane path on,
    the Weather pages are pictures on the page layer, so Hourly's graph
    reaches the 2D renderer's cache only when Hourly is drawn on screen 0
    (the plane path off, or a hand-off).
  **Options, with their numbers:**
  - **(a) Accept the re-upload on that one route.** 87,120 bytes, about
    2.6-2.9 ms, only on a return from Hourly drawn on screen 0; 1 picture
    on the X51v's route. Nothing changes.
  - **(b) Rebalance the 384 / 384 split.** Holding both sets needs about
    581 KB (567 KiB) of pictures, leaving the glyphs about 201 KiB. The
    glyphs need 343 of their 384 KiB: 279 KiB for every font-and-colour
    pair on the look route plus up to 64 KiB part-filled (ADR-0023 rule
    4's measurement). About 142 KiB short: glyph atlases would be evicted
    and uploaded again at screen changes and look switches, the cost the
    11-09 split was made to remove (12 + 3 atlases on the return,
    today-0071). It also reopens the owner's settled 384 / 384 ruling.
  - **(c) Don't cache Hourly's graph** (draw it from a fresh upload each
    frame that shows it). 214,080 bytes at 30-33 MB/s is 6.5-7.1 ms added
    to every Hourly frame drawn on screen 0, scrolling included, to save
    2.6-2.9 ms once on the way back. (Drawing the graph at 16 bits, 107 KB,
    still leaves the two sets about 474 KB together, over 384 KiB, and
    loses the soft fill's alpha: no help here.)
  **The reviewer's recommendation, for you to rule on:** (a), accept the
  5-card re-upload on that one route; rebalancing would starve the
  glyphs, which need 343 of their 384 KiB.
