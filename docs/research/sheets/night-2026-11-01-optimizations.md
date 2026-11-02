# Sheet: the night's optimization passes (2026-11-01, the owner's go)

The owner, 2026-11-01: "I want you to do all of the passes and everything
you need to do before i test anything else", then "its fine if it takes
you a while, loader is up feel free to do any test you need". Each pass:
its cause proven first, rule 0, what was built, how it is held, and what
would overturn it. R176's panel pass has its own sheet
([r176-panel-cost.md](r176-panel-cost.md)).

## 1. The living sky drawn only where it changes

- **Cause (the X51v, today-0048 and the panel run):** a sky step redrew
  the whole weather card (glass, words, picture, sky) 30 times a second,
  ~14 ms of CPU each (~40% of the CPU on Today).
- **Rule 0.** Windows Mobile had no animated weather; Android's live
  wallpapers redraw the whole surface each frame on a GPU. Ours computes
  each step's change from the step count (clouds move whole pixels on some
  steps; rain and snow every step), so it draws only that.
- **Built:** a dry sky's step marks only where each moved cloud was and
  is (`sky_step`, `Cloud::rect`), nothing on a step where none moved; the
  clouds spaced in whole paces so all move on the same step (3 steps in 4
  draw nothing). Rain, snow and storms still draw the whole card a step.
- **Held by:** `the_sky_drawn_where_it_changes_matches_it_drawn_whole`
  (1,200 steps drawn in parts equal the card drawn whole, pixel for pixel;
  fewer than half the steps draw; none the whole card).
- **Overturns it:** the sky's frames on the X51v not down from ~14 ms of
  CPU, or the owner seeing any difference in the clouds' motion.

## 2. Living sky On / Off (the owner's ask)

A row in Weather settings, LOOK: on by default, kept in the settings
record's appearance byte (no new version or length; an older record reads
as on). Off, nothing of the sky is drawn or woken for (a test). Rows 38
-> 36 px so twelve rows fit above the bar.

## 3. A chart's line in its own picture

- **Cause:** the chart's line was painted into Hourly's graph slot:
  opening a chart repainted it, Hourly's page key changed, and Hourly's
  2.3 MB picture was rebuilt (~190 ms of CPU) with the chart's line in
  it; closing, again (11 times in 22 s in the panel run).
- **Built:** `chart_pic` apart (`keep_chart`, one more store slot,
  ~200 KB of the 29 MB free); Hourly's key never moves with a chart.
- **Held by:** the chart tests (the line in its own picture, Hourly's key
  unchanged with a chart open).

## 4. Icons at start: the inflate and the Paeth undo faster

- **Cause (local/rp-today-run-2026-11-01-press.txt):** 20 pictures, 1.44 s at start, each inflated from
  its 250 px master (70-100 ms) and shrunk; on the PC ~3/4 of a picture's
  making is the inflate, ~1/4 the Paeth undo.
- **Rule 0.** zlib's own inflate decodes a code's first 9 bits in one
  table read; the RFC's way (and ours, from puff) walks a bit at a time.
  Ours now does as zlib does, each table sized to its code (the stack
  rule's 4 KB frame held), the slow walk only for the rare long codes;
  the Paeth undo's loop without edge tests.
- **Built:** `rp-inflate`'s first-look tables; `align` gives back bytes
  read ahead (a stored block after a short end-of-block code); a fixture
  of dynamic blocks between flushes' stored blocks.
- **PC:** a picture 1,149 -> 770 us (inflate 836 -> 486, Paeth 297 ->
  284). The X51v's number comes from the next run.
- **Not built (the owner's call):** packs holding each size the screens
  show (inflate only what's shown, ~10x less for the small ones): option
  C (weather-rule0.md) chose big pictures only; changing it is his ruling.

## 5. A frame budget the build holds (R171)

emu-today now fails if any frame of its replay takes longer than 33 ms
(30 fps) to prepare, in the emulator's instruction time: a change that
makes a screen's frame heavier is caught before it reaches the X51v.
Today's worst is ~24 ms there.
