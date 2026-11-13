# Deep dive fix, 2026-11-12 evening: the owner's remaining list

Branch `fix/remaining-2026-11-12`, from main at fc2d733b. **Merged
2026-11-13 on the owner's "push it all live": main fc2d733b -> cb52aa94**
(not pushed while the list ran, as it said; the merge record is below). Device runs only on the X51v
already on the loader, unattended (look route today-0141; rp-gpubench
gpubench-0132 to -0137). Decisions left to the owner:
[decisions-2026-11-12-evening.md](decisions-2026-11-12-evening.md). Item 5
of the 11-11 list (the X50v and cold-chip runs) is not in this list: it
stays prepared exactly as written in
[decisions-2026-11-11.md](decisions-2026-11-11.md), not run, not asked.
Gate: 30 of 30 architectural lines for phases 2, 4, 5 (`cargo xtask gate`).

| # | Item | Outcome | Commit (`cargo xtask ci`) | Proof |
|---|---|---|---|---|
| 1 | R191's Start menu, ruling (b) | **Applied, proven on the X51v.** The menu's glass is a panel whose body starts from the wallpaper, so nothing under it ever shows except through its four rounded corners: the 2D renderer now draws the commands before a panel only in the pieces it leaves (the corners, anything outside it), pixel-identical to drawing everything. The ruling's aim (Today's commands not run under the glass) by a different means than its words (copying Today's pixels, which would only have been read at the corners): written in the decisions file. **The Start menu 18,176 us** (was 38,552; 191 blits, was 610), under its 33 ms; `look-judge` verdict OK, 0 wrong, every frame of the route inside the 33 / 45 ms rule; R191 closed. ADR-0023 rule 3 amended | 5204de5a (passed) | the renderer pixel-checked against the reference with the commands hidden counted; 2 mutants, both killed; [run today-0141](runs/2026-11-12-today-0141.md) |
| 2 | R76, the temperature guard | **Designed against ADR-0035 amendment D, built, proven on the X51v.** 136.50 / 1 down to 133.25 / 1 at `TEMP0` 866, back up at 868 (the hottest reading of every run that proved the game step was 867; one count of the reading's measured spread each side), a reading every 10 s, a bad reading counted hot, a failed change latching Windows Mobile's setting. Thresholds: the shipped defaults; **R71 (open) the remaining dependency**, recorded in the decisions file; no threshold invented. On the device (the run's trip 3 counts over its start; the shipped 866 is past what a room lets a run reach): the step down at peak heat and the step up as it cooled, temperatures logged every 10 s, 100 of 100 game renders right after each change, the underrun flag clear. ADR-0035 "The temperature guard for games" (rule 8 amended); R76 closed | cdbdef78, fc0b00ed (both passed) | 7 host tests, 1 mutant killed; [runs GC2EA](runs/2026-11-12-gc2ea-heat-guard.md) (gpubench-0132, and -0133 on the committed code) |
| 3 | R90, the game path's route switch | **Built; R101, R102 and R108 all passed on the X51v.** The route choice ranked by the game's measured times (8 / 60 frames, holds, two in flight on route 2, route 3 (d') only and only after R89) and way C' in the display driver, called by the runs: **R101** 1,200 / 1,200 whole switching every frame and every 8 (gpubench-0134); **R108 at 136.50 / 1** while rendering, 2,399 / 2,399 and 1,349 / 1,349 judged frames whole, 0 torn (-0135); **R102 at 136.50 / 1**, (d') every frame equal to serial (404 / 404, -0136) and 9.7-12.7% less a frame than serial in the same image (-0137); R102 closed. Left: the service's game frame entry point, with the first game port. ADR-0038 "Built" | f1893326 (passed) | 9 host tests, 1 mutant killed; [runs](runs/2026-11-12-game-path-routes.md) |
| 4 | The two CI runs in progress at the last review | **Both success, all five jobs:** [43345644916](https://github.com/Drakey83/retropane-os/actions/runs/43345644916) on fc2d733b (push to main, completed 2026-11-13 03:40 UTC) and [43344845042](https://github.com/Drakey83/retropane-os/actions/runs/43344845042) on 1aa9d746 (push to main, completed 03:23 UTC) | this report's commit | `gh run view` on each |
| 5 | Owed device evidence | **R182 and R183 proven in run today-0141 and closed:** the clock set to Thu 11/12 21:14, the PC's Thu 11/12 21:14 at the load; the sky running under the relay's real forecast, "the sky drawn WMO 0 cover 45%, the hero's words Clear" (no rain, snow or storm under Clear; the two clouds a clear sky always drifts are a question in the decisions file). **C97, C98 and R181 stay owed and recorded**, not forced: their causes can't be made to happen, and their logging is in place | e3cecd77 (passed) | [run today-0141](runs/2026-11-12-today-0141.md) |

Reminders that came due: R191 (closed, item 1), R76 (closed, item 2), R90
(built; left with the first game port), R102 (closed), R182 and R183
(closed, item 5). R101 and R108 were already closed; their runs repeated
through the driver and at the game clock. Found on the way: **R192**, the
mutant `glesfeat G8` doesn't build (it stops a whole `cargo xtask mutants`
run; untouched by this branch). The other device-run reminders need the
owner watching or other images, so none was folded into these runs.

## CI

- **GitHub's workflow:** the newest runs on `main`,
  [43345644916](https://github.com/Drakey83/retropane-os/actions/runs/43345644916)
  (fc2d733b) and
  [43344845042](https://github.com/Drakey83/retropane-os/actions/runs/43344845042)
  (1aa9d746): conclusion **success**, both, all five jobs. While the list
  ran no GitHub run existed for this branch (the workflow runs on a push
  to main or a pull request, and the list said not to push). On the
  owner's "push it all live", the pull request's run
  [43380549085](https://github.com/Drakey83/retropane-os/actions/runs/43380549085)
  on cb52aa94: conclusion **success**, all five jobs (the emulator boot
  2 h 10 min).
- **`cargo xtask ci` (this PC, Windows):** run on every commit of the
  branch in a clean second checkout (`../rp-ci`); each commit's result is
  in the table, the last commits' written after this report's commit runs.

## Push state (settled last, then checked)

When the list finished, nothing was pushed: `git ls-remote origin`
showed no `fix/remaining-2026-11-12`, and GitHub `main` was at fc2d733b,
this branch's start. Nothing force-pushed, no history rewritten.

**Merge record, 2026-11-13 08:53 UTC** ([README.md](README.md), rule 2).
On the owner's "is this all pushed live, if not please push it all
live": the branch pushed at cb52aa94 and
[pull request 16](https://github.com/Drakey83/retropane-os/pull/16)
opened; its run
[43380549085](https://github.com/Drakey83/retropane-os/actions/runs/43380549085)
on cb52aa94 concluded **success** on all five jobs (08:53:02 UTC);
`cargo xtask ci` had passed in this checkout on cb52aa94. `main`
fast-forwarded fc2d733b -> cb52aa94 (no force, no merge commit); GitHub
shows pull request 16 merged at 08:53:55 UTC with cb52aa94 as its merge
commit. This record is committed after the merge and pushed to the
branch and to `main` the same way.
