# Decisions for the owner, 2026-11-09

From the owner's list of 2026-11-09 (branch `fix/deep-dive-2026-11-09`).
Each box is one decision; the evidence is linked. Nothing here is built
until its box is ticked.

## Item 3: the device frame budget

- [ ] **Give the 2D renderer's picture cache room for Today's pictures**
  (ADR-0023 rule 4's memory table). **Why:** on the X51v the return to
  Today still takes 53.5 ms, and 9 pictures are uploaded again in that
  frame ([today-0082](runs/2026-11-09-today-0082.md)). Today draws a
  250 x 176 hero (176,000 bytes) and eight 66 x 66 cards (17,424 bytes
  each), 315 KB in all. The picture cache is 256 KiB and flushes whole
  when full, so Today's own set never fits. **Proposed:** move the boundary
  between the picture cache and the glyph memory, which sit side by side
  (`0x516000`-`0x5D6000`, 768 KiB), from 256 / 512 KiB to 384 / 384 KiB,
  and evict pictures one at a time (the oldest not drawn this frame, as
  the glyphs now are), not the whole cache. The glyphs need 279 KiB for
  every pair on the look route since today's fix, plus at most 64 KiB of
  part-filled chunks: 343 of 384 KiB. Nothing else in the memory map
  moves. **Cost:** less room for glyphs if a later screen draws many new
  font and colour pairs (the cache then evicts the oldest, as now).
  **What overturns it:** a screen whose glyph pairs need more than 384
  KiB, measured with the route test.
- [ ] **Whole-screen frames and the 33 ms budget.** **Why:** after today's
  fixes the look switch is 39.5-42.6 ms and the return 53.5 ms on the
  X51v. A whole-screen frame costs about 40 ms of 2D engine work: the
  4 x 4 dither alone is 18.9 ms a screen (perf row
  `b8_dither_add_full_screen`), and copies and blends the rest. So a
  frame that changes the whole screen can't meet 33 ms on this engine
  whatever the caches do. The budget is yours: (a) keep 33 ms for every
  frame and treat whole-screen changes as work to cut (for example, a
  pre-dithered copy of the background for areas no command draws over);
  (b) hold 33 ms for frames that move (scrolls, slides) and allow a single
  whole-screen change (a look switch, a screen change) up to ~45 ms, as
  one late frame.
