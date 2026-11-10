# Decisions for the owner, 2026-11-09

From the owner's list of 2026-11-09 (branch `fix/deep-dive-2026-11-09`).
Each box is one decision; the evidence is linked. Nothing here is built
until its box is ticked.

## Item 3: the device frame budget

- [x] **Give the 2D renderer's picture cache room for Today's pictures** *(ruled yes by the owner, 2026-11-10; built on fix/deep-dive-2026-11-10, run today-0115)*
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
- [x] **Whole-screen frames and the 33 ms budget.** *(the owner, 2026-11-10: (b), 33 ms for frames that move, a single whole-screen change up to ~45 ms as one late frame)* **Why:** after today's
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

## Item 5: the divide-1 runs that stay untried (R189)

Prepared; nothing here needs your decision, only your hands, when you're
well. No new code is needed for either run: both use runs already in
`rp-gpubench`.

- [ ] **A second 2700G: your X50v.** *(the owner, 2026-11-10: no decision; stays prepared, carried to decisions-2026-11-10.md)* **Why:** every divide-1 number so far
  is one chip's (the X51v's); the margin is per chip, and the X50v is a
  second 2700G. **What you'd do:** put the same SD card in the X50v, boot
  Windows Mobile, start HaRET as on the X51v, and tell me "X50v loader is
  up". RetroPane's loader has never run on the X50v, so the first step is
  only to see that it comes up (it may not: the board differs). **What I'd
  run then:** GC2DR (`RP_GPUBENCH_GC2_FINE=1 cargo xtask load
  rp-gpubench`, every locked clock from 133.25 to 139.75 / 1 on the four
  game frames moved, about 10 minutes), then GC2DT's 137.80 / 1 check.
  Each unit then ships only up to its own proven step (your ruling, item 1).
- [ ] **A cold chip.** *(the owner, 2026-11-10: no decision; stays prepared, carried to decisions-2026-11-10.md)* **Why:** every run past the line was warm. **What
  you'd do:** nothing beyond leaving the X51v off for an hour or more, then
  booting it to the loader and saying "cold, loader is up" before
  anything else runs. **What I'd run:** GC2DR first, its TEMP0 reading
  logged before and after each setting, against the warm GC2DR of
  2026-11-08. (An X51v sitting idle on the loader for an hour is cooler
  but not cold: the display and the 2700G's clocks stay on. If one comes
  up this session it is run and reported as "idle an hour", not as cold.)

## Item 4: the touch replay's slowdown (R188)

- [x] **The compose-ahead's cost (proven: about half the rise).** *(the owner, 2026-11-10: (a), compose ahead only after Today has been still for 1 s)* On the
  X51v the touch replay's touch-to-flush median is 19.2-20.3 ms against
  the 16.2 ms floor; with the Weather app not composed ahead it is
  18.1-18.4 ms ([report](runs/2026-11-09-r188-touch-replay.md); the
  bisect's first slow commit, 4fa4dbad, made that composing). Composing
  the app replaces the 2D renderer's background and pushes Today's
  pictures out of its cache, so the next Today frame redoes both.
  **Options:**
  - (a) Compose ahead only after Today has been still for 1 s. Plus: no
    cost while you tap about. Minus: a tap on the hero within 1 s of a
    change opens Reveal the old way (the app composed on the tap: the 99 ms
    freeze you called clunky, now only in that case).
  - (b) Keep Today's background and pictures through a compose: the
    compose given its own background picture (1.2 MiB of 2700G memory, a
    change to ADR-0023 rule 4's map) and the bigger picture cache of item 3.
    Plus: Reveal unchanged, no cost. Minus: memory taken from the pool.
  - (c) Keep it, and take the touch floors as they are now (the
    compose's 1.5 ms is Reveal's price).
- [x] **The other ~2 ms (not yet attributed).** *(the owner, 2026-11-10: yes, run it next time the X51v is on the loader)* It needs a second bisect
  with the compose held off in every build. Old commits can't switch it
  off, so the way is to bisect with a patch applied per commit. That's
  about an hour of unattended device runs. Tick to have it run next time
  the X51v is on the loader.
