# Decisions waiting for the owner (deep-dive list, 2026-11-06)

Written while the owner was away; **the owner ruled on every box
2026-11-07** (his answers under each). Each item's code is built on branch
`fix/deep-dive-2026-11-06`, **merged to main at 3b8e02f1** on 2026-11-07
(GitHub main fast-forwarded to it on the owner's "push to live"; the
branch and `backup/main-2026-11-07` pushed with it). The follow-up work is
on `fix/deep-dive-2026-11-07` ([its report](deep-dive-2026-11-07-fix.md)).

## Item 1: the divide-1 cures

- [x] **Accept ADR-0035 amendment E** (the cures follow the clock: on for
  every render into screen 1 at divide 1 above 74.75 MHz, off on die and
  at divide 2; R120's switch stays dropped).
  [docs/adr/0035-2700g-clocks.md](../adr/0035-2700g-clocks.md), "Amendment E".
  **Owner 2026-11-07: "yes, accepted. The cures follow the clock as
  built."** He also said not to merge the branch to main until the P8
  device check has run; GitHub main already held 3b8e02f1 from his "push
  to live" earlier the same day, so main carries the rule ahead of its
  device check (nothing rewritten). The check then passed on the X51v
  2026-11-07 (run scene-0001, P8 judged by DSIG; corrected 2026-11-09:
  this line said it stayed owed).
- [x] **Run the device check** (**passed on the X51v 2026-11-07**, run scene-0001: [report](runs/2026-11-07-scene-p8.md)) ([run sheet](../research/sheets/divide1-cures-by-clock.md)):
  `rp-scene` divide-1 variant, P8 judged by DSIG. Made and passed (scene-0001).
- [ ] **Which divide-1 rungs ship** stays amendment D's vote (Q1-Q3),
  unchanged by E.

## Item 5: the stack

- [x] **Approve the loader's zeroed-RAM budget 36 -> 42 KiB** (`xtask/src/size.rs`):
  the two card LUNs moved from `main`'s frame into static slots, the
  loader's deepest stack path 87% -> 67% of its 32 KiB. The same bytes,
  counted in the zeroed RAM instead of the stack. Unticked, the LUNs go
  back on the stack and the loader stays on R149's list.
  **Owner 2026-11-07: "yes, approved. The bytes moved off the stack, they
  are not new."

## Item 8: the ratchet's floors

Fourteen drift-type tolerances (R109's list) were widened to four standard
deviations of their 11 session medians in `local/perf` (`cargo xtask perf
sessions`); no floor value changed. These floors' latest session is slower
than the floor allows, so each needs your call (a slower floor is yours):

- [x] **`probe.cache_walk_40k`**: last session +1.16%, tolerance 0.21% (R115
  saw +1.2% there, unrelated to its change). Approve a slower floor, or a
  chase first.
- [x] **`probe.cache_walk_48k`**: last session +1.17%, tolerance 0.17% (the same).
- [x] **`tr_touch_replay.pen_down_period_worst`**: last session +0.47%, tolerance 0.38%.
- [x] **`tr_touch_replay.landing_to_verdict_median`**: last session +0.045%,
  tolerance 0.0448% (on the line).
- **Owner 2026-11-07: all four approved as slower floors.** New floors (the
  last session's medians, run-1793368325 to run-1793371107): 40k 295100 ->
  298534, 48k 353757 -> 357904, pen-down worst 12664 -> 12724, landing to
  verdict 8920 -> 8924; each row in `docs/perf-floors.md` marked approved
  by the owner, 2026-11-07.
- [x] **Re-measure the floors on the X51v** (`cargo xtask perf noise 5` in
  three sessions, a soft reset between, the same build): now that every run
  file records its build, the spread can be taken build for build and the
  other 34 numbers whose sessions differ (most of them real code changes,
  e.g. `probe.chase_sdram_256k` after its benchmark changed) re-floored.
  **Owner 2026-11-07: "yes, but not now"** (it needs him and the device).
  Ready, not run: [run sheet](../research/sheets/r109-perf-remeasure.md).

## Item 9: games, design only

- [x] **Game Mode sheet A** ([DRAFT](../research/sheets/game-mode-alternatives-a.md)):
  go on alternative 3 (one kernel, two user-space profiles; a fast tier
  under 2 s that keeps frozen programs in RAM, a full tier up to ~15 s that
  writes them to flash or SD), or a change. Design B waits for this.
  **Owner 2026-11-07: "yes, go"** on alternative 3. Design B drafted,
  design only: [game-mode-b.md](../design/game-mode-b.md).
- [x] **The R90 game-path ADR:** no new draft was written. It already exists
  as [ADR-0038](../adr/0038-game-render-path.md), accepted by you on
  2026-10-29 (G1-G6: the three display routes ranked by measured time,
  dynamic resolution per frame, the per-game override). Writing a new draft
  would reopen a settled ruling, which the list forbade. Tick if you want a
  fresh draft anyway; what is left of R90 is the driver itself.
  **Owner 2026-11-07: no new draft;** ADR-0038 stands, not reopened; what is
  left of R90 is the driver itself.

