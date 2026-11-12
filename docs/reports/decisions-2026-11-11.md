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

## Item 3: the touch replay's rise no single commit explains (carried from decisions-2026-11-10.md, item 2)

- [ ] **What to do about the rise R188's second bisect couldn't pin to
  one commit.** **Why:** with the compose held off in every build, the
  touch replay's frame-and-queueing median climbs from 508 us at 4fa4dbad
  to 752 at main, in 60-85 us steps over the features of 2026-10-30 to
  11-01; the first commit over the line (680 us) is f4ffea28 (the card's
  word shadows, whole clouds, 16 glyph atlases: +60 us, about a quarter
  of the rise). Touch to flush barely moves with it (16,472 against 16,704
  us with the compose off), so 11-09's "~2 ms" was mostly run-to-run
  spread (two identical runs differed by 1.7 ms). This branch's starting
  point (main at 79b0536f) measured 784 us with the 1 s compose rule.
  Evidence: [the bisect's table](runs/2026-11-10-r188-compose-rule.md)
  (card logs today-0119 to today-0136). **Options:** (a) accept it as
  unattributed and re-set the touch floors to the measured numbers (frame
  and queueing ~784 us, touch to flush 16.9-18.6 ms); (b) time Today's
  frame by feature on the X51v (the word shadows, the clouds, the glass)
  and cut the costliest first, floors kept at 384 / 16,248; (c) both. No
  device time goes into re-bisecting 60 us steps unless you say so.
  **The reviewer's recommendation, for you to rule on:** accept it as
  unattributed: gradual drift, no single cause, and the floors hold it.

## Item 5: the divide-1 runs that stay prepared (R189; carried from decisions-2026-11-10.md, item 3, word for word)

The owner, 2026-11-11: "Item 5 stays prepared, not forced." Neither run
happens until he says the device is up; no new code is needed for either.
The steps, as written on 2026-11-09:

Prepared; nothing here needs your decision, only your hands, when you're
well. No new code is needed for either run: both use runs already in
`rp-gpubench`.

- [ ] **A second 2700G: your X50v.** **Why:** every divide-1 number so far
  is one chip's (the X51v's); the margin is per chip, and the X50v is a
  second 2700G. **What you'd do:** put the same SD card in the X50v, boot
  Windows Mobile, start HaRET as on the X51v, and tell me "X50v loader is
  up". RetroPane's loader has never run on the X50v, so the first step is
  only to see that it comes up (it may not: the board differs). **What I'd
  run then:** GC2DR (`RP_GPUBENCH_GC2_FINE=1 cargo xtask load
  rp-gpubench`, every locked clock from 133.25 to 139.75 / 1 on the four
  game frames moved, about 10 minutes), then GC2DT's 137.80 / 1 check.
  Each unit then ships only up to its own proven step (your ruling, item 1).
- [ ] **A cold chip.** **Why:** every run past the line was warm. **What
  you'd do:** nothing beyond leaving the X51v off for an hour or more, then
  booting it to the loader and saying "cold, loader is up" before
  anything else runs. **What I'd run:** GC2DR first, its TEMP0 reading
  logged before and after each setting, against the warm GC2DR of
  2026-11-08. (An X51v sitting idle on the loader for an hour is cooler
  but not cold: the display and the 2700G's clocks stay on. If one comes
  up this session it is run and reported as "idle an hour", not as cold.)
