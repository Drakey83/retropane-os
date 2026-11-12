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
