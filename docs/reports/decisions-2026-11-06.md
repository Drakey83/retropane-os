# Decisions waiting for the owner (deep-dive list, 2026-11-06)

Written while the owner was away; nothing here is decided. Tick a box to
rule. Each item's code is built on branch `fix/deep-dive-2026-11-06`
(not pushed).

## Item 1: the divide-1 cures

- [ ] **Accept ADR-0035 amendment E** (the cures follow the clock: on for
  every render into screen 1 at divide 1 above 74.75 MHz, off on die and
  at divide 2; R120's switch stays dropped).
  [docs/adr/0035-2700g-clocks.md](../adr/0035-2700g-clocks.md), "Amendment E".
- [ ] **Run the device check** ([run sheet](../research/sheets/divide1-cures-by-clock.md)):
  `rp-scene` divide-1 variant, P8 judged by DSIG. Owed; not made.
- [ ] **Which divide-1 rungs ship** stays amendment D's vote (Q1-Q3),
  unchanged by E.

## Item 5: the stack

- [ ] **Approve the loader's zeroed-RAM budget 36 -> 42 KiB** (`xtask/src/size.rs`):
  the two card LUNs moved from `main`'s frame into static slots, the
  loader's deepest stack path 87% -> 67% of its 32 KiB. The same bytes,
  counted in the zeroed RAM instead of the stack. Unticked, the LUNs go
  back on the stack and the loader stays on R149's list.

## Item 8: the ratchet's floors

Fourteen drift-type tolerances (R109's list) were widened to four standard
deviations of their 11 session medians in `local/perf` (`cargo xtask perf
sessions`); no floor value changed. These floors' latest session is slower
than the floor allows, so each needs your call (a slower floor is yours):

- [ ] **`probe.cache_walk_40k`**: last session +1.16%, tolerance 0.21% (R115
  saw +1.2% there, unrelated to its change). Approve a slower floor, or a
  chase first.
- [ ] **`probe.cache_walk_48k`**: last session +1.17%, tolerance 0.17% (the same).
- [ ] **`tr_touch_replay.pen_down_period_worst`**: last session +0.47%, tolerance 0.38%.
- [ ] **`tr_touch_replay.landing_to_verdict_median`**: last session +0.045%,
  tolerance 0.0448% (on the line).
- [ ] **Re-measure the floors on the X51v** (`cargo xtask perf noise 5` in
  three sessions, a soft reset between, the same build): now that every run
  file records its build, the spread can be taken build for build and the
  other 34 numbers whose sessions differ (most of them real code changes,
  e.g. `probe.chase_sdram_256k` after its benchmark changed) re-floored.

