# Decisions for the owner, 2026-11-12 evening

From the owner's remaining list of 2026-11-12 (branch
`fix/remaining-2026-11-12`). Each box is one decision; the evidence is
linked. Nothing here is built until its box is ticked, and no box here is
decided by me. Item 5 of the 11-11 list (the X50v and cold-chip runs)
stays prepared exactly as written in
[decisions-2026-11-11.md](decisions-2026-11-11.md): not run, not asked.

## Item 1: R191's Start menu (the owner's ruling (b)): applied, nothing to decide

Done and proven on the X51v (run [today-0141](runs/2026-11-12-today-0141.md)):
the Start menu opens in **18,176 us** (was 38,552), under its 33 ms;
`look-judge` verdict OK, 0 wrong, and every frame of the look route inside
the 33 / 45 ms rule. How, in one line: the glass shows the wallpaper, not
the screen, so the renderer draws nothing under a panel's body and draws
Today only through the panel's four corners (pixel-identical to drawing
everything). It is the ruling's aim (Today's commands not run under the
glass) by a different means than its words (copying Today's pixels): the
copy would only have been read at the corners. If you want the copy
itself instead, say so; nothing else waits on it.

## Item 2: the temperature guard (R76): built; R71 the remaining dependency

Built and proven on the X51v ([run GC2EA](runs/2026-11-12-gc2ea-heat-guard.md)):
the guard stepped 136.50 / 1 down to 133.25 / 1 at the run's peak heat
and back up as it cooled, every render right on both rungs, the display's
underrun flag clear after each change. Its design is in ADR-0035 ("The
temperature guard for games").

- [ ] **R71, the per-device self-test, is what sets each unit's
  thresholds** (recorded as the list asks; no box for you to tick now).
  Until R71 runs, every unit ships the defaults measured on your X51v:
  **step down at `TEMP0` 866, back up at 868** (the hottest reading of
  every run that proved the game step was 867; one count of the
  reading's spread each side), one reading every 10 s. No threshold was
  invented: each is a measurement. **What is not proven:** neither rung
  has ever run hotter than 867, so past the trip 133.25 / 1 is your ruled
  peak-heat rung, not a measured one; R71 measures both on each unit.

## A question from item 5's sky run (R183)

- [ ] **Should a clear sky drift clouds?** **Why:** run today-0141 ran
  the living sky under a real forecast: WMO 0 (clear), 45% cloud cover,
  the hero's words "Clear". The words match what is drawn for rain, snow
  and storms (R183's fix holds), but the sky always draws two or three
  clouds (`weather_of`: 2 under 50% cover, 3 above), from your 2026-11-01
  ask for "two or 3, larger, clouds on the screen at once". So a "Clear"
  card drifts two clouds. **Options:** (a) keep it: the clouds are the
  living sky's motion, and 45% cover is partly cloudy whatever the code
  says; (b) no clouds at WMO 0-1 (a still sky under "Clear"; the sky then
  has nothing that moves, so it stops stepping and saves its redraws);
  (c) clouds by cover alone, 0 below some cover (which cover would need a
  look at real forecasts first). Nothing changes until you pick.
