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

## Item 3: the divide-1 runs that stay prepared (R189; 2026-11-09's item 5, carried forward unchanged)

The owner, 2026-11-10: "Item 5 stays prepared, not forced." Neither run
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

## Item 2: the touch replay's last 0.25 ms (R188)

- [ ] **The rise the second bisect couldn't pin to one commit.** **Why:**
  with the compose held off in every build, the touch replay's frame and
  queueing goes from 508 us (4fa4dbad) to 752 (main), climbing 60-85 us at
  a time across the features of 2026-10-30 to 11-01. The first commit over
  the line is f4ffea28 (the card's word shadows, whole clouds, 16 glyph
  atlases: +60 us), about a quarter of the rise
  ([report](runs/2026-11-10-r188-compose-rule.md)). Touch to flush barely
  moves with it (16,472 against 16,704 us), and the "~2 ms" of 11-09 was
  mostly run-to-run spread. This branch: 784 us with your compose rule.
  **Options:** (a) accept it as the cost of those features and re-set the
  touch floors to this branch's numbers (frame and queueing ~784 us, touch
  to flush 16.9-18.6 ms); (b) time Today's frame by feature (the word
  shadows, the clouds, the glass) on the X51v and cut the costliest first,
  keeping the floors at 384 / 16,248; (c) both: re-set now, cut later.

## GitHub's CI (found re-verifying the list)

- [ ] **Let GitHub's CI run on this branch.** **Why:** GitHub's own
  workflow (`.github/workflows/ci.yml`, Linux) has been red on every main
  push since 2026-10-24, 04d3d142 included
  ([run 42731190233](https://github.com/Drakey83/retropane-os/actions/runs/42731190233),
  failure). Two test images named their module files through a folder
  that doesn't exist, which Windows resolves and Linux can't. Fixed on this
  branch (e18bd531), with a lint that refuses the pattern (5a66eafc).
  `cargo xtask ci` passes here on every commit, but this PC runs Windows,
  and the workflow runs only on a push to main or a pull request. Your
  list says not to push, so no GitHub run exists for this branch, and
  other Linux-only failures hidden behind this one since 2026-10-24 can't
  be ruled out from here. **What you'd do:** say "push", and I'd push the
  branch (a pull request runs the workflow without touching main), then
  read the run and fix anything red before main moves.

