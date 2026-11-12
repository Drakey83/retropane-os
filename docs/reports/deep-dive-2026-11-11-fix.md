# Deep dive fix, 2026-11-11: the owner's six items

Branch `fix/deep-dive-2026-11-11`, from main at 79b0536f. **Not pushed**
(the list says not to; checked last, below). Device runs only on the X51v
already on the loader, unattended. Decisions left to the owner:
[decisions-2026-11-11.md](decisions-2026-11-11.md). Gate: 30 of 30
architectural lines for phases 2, 4, 5 (`cargo xtask gate`).

| # | Item | Outcome | Commit (`cargo xtask ci`) | Proof |
|---|---|---|---|---|
| 1 | R191: the frames the frame rule fails | **(a) and (b) fixed, proven on the X51v; (c) and (d) found, not fixed: a tick-box.** Causes proven in today-0115 first: every icon decoded marked the whole screen (Today drawn whole 7 times at start-up), and every forecast and rain-frame answer did too, on any screen (Settings drawn whole, 52.3 and 46.1 ms, showing neither). Now a frame draws only where its draw list changed (`rp_draw::changes`: 64-bit keys per command, the last list kept in 8 KB of the RAM pool). today-0137: no start-up frame slow; the forecast on Settings 302 us of CPU, 0 rects; the second late Settings frame gone. (c) the return to Today 55.5 ms and (d) the Start menu 38.5 ms: Today's list is 706 and 610 blits, ~278k blended px, the code's cost; the rule unchanged, options as a tick-box. Found: the first Light look switch on Settings, 45,088 us (88 us over), in the same box | 6c196a7f (passed) | tests `data_that_shows_little_or_nowhere_marks_only_that`, restatements pixel-checked in `incremental_frames_match_full_redraws`, three in `rp_draw::changes`, two mutants (each fails its test); run today-0137, `cargo xtask look-judge` (fails on frame 93, then 179 and 193 by reading); [run report](runs/2026-11-11-today-0137.md) |
| 2 | The Hourly tick-box | **Prepared, a tick-box with numbers.** (a) accept: 87,120 bytes, 2.6-2.9 ms, only on a return from Hourly drawn on screen 0 (1 picture on the X51v's route); (b) rebalance: both sets need ~567 KiB, leaving the glyphs ~201 of the 343 KiB they need, and reopens the 384 / 384 ruling; (c) no graph cache: 6.5-7.1 ms on every Hourly frame drawn on screen 0. The reviewer's recommendation (a), written as his; not decided | edd9b5d7 (passed) | ADR-0023 rule 4's measurements, run today-0115, the host route test |
| 3 | The bisect remainder | **Prepared, a tick-box with the evidence.** 508 to 752 us in 60-85 us steps, f4ffea28 the first over the line, the 11-09 "~2 ms" mostly spread; options accept / time by feature / both; the reviewer's recommendation (accept as unattributed), written as his; no device time spent re-bisecting | 9606470d (passed) | runs today-0119 to today-0136 (docs/reports/runs/2026-11-10-r188-compose-rule.md) |
| 4 | Push-state staleness | **Fixed, and the rule written where it is read.** deep-dive-2026-11-10-fix.md's header and push section now record the merge (main 04d3d142 -> 79b0536f on the owner's "Yes, merge", pull request 13 merged 2026-11-12 01:08 UTC), the old state kept in the past tense; decisions-2026-11-10.md's push box names it; five dated push lines get a "since" line. Every commit any report cites is on GitHub `main` (79 of 79). Rule: [docs/reports/README.md](README.md), linked from CLAUDE.md rule 2: a push or merge after a report gets a dated merge record in that report in the same session, and a push claim is settled only when the merge is recorded | 83729a1b (passed) | `git ls-remote origin main` = 79b0536f; `git merge-base --is-ancestor` on every cited commit |
| 5 | Item 5 stays prepared | **Prepared.** The X50v and cold-chip steps copied word for word (checked by string match); neither runs until the owner says the device is up | a21fb016 (passed) | decisions-2026-11-11.md, item 5 |
| 6 | GitHub run 43132345975 | **Recorded: success.** [Run 43132345975](https://github.com/Drakey83/retropane-os/actions/runs/43132345975), the push to main of 79b0536f, completed 2026-11-12 03:23 UTC, conclusion **success**, all five jobs (fmt, clippy, lint and test; the X51v cross-build; the emulator boot; the benchmarks; coverage) | this report's commit | `gh run view 43132345975` |

Reminders that came due and what happened: R191 (items (1) and (2) fixed by
today-0137, (3) and (4) a tick-box; stays open). The other device-run
reminders (R3, R4, R12, R22 and others) need the owner watching or other
images, so none was folded into this unattended run.

## CI

- **GitHub's workflow:** the newest run on `main`,
  [43132345975](https://github.com/Drakey83/retropane-os/actions/runs/43132345975)
  on 79b0536f, **success**; the two before it on 79b0536f and a4caf626
  (43061475333, 42950562626) also succeeded. No GitHub run exists for this
  branch: the workflow runs on a push to main or a pull request, and the
  list says not to push.
- **`cargo xtask ci` (this PC, Windows):** passed on every commit of the
  branch, each in a clean second checkout (`../rp-ci`), the last commit
  included (its line is written after this report's commit runs).

## Push state (settled last, then checked)

Not pushed: `git ls-remote origin` shows no `fix/deep-dive-2026-11-11`, and
GitHub `main` is at 79b0536f, this branch's start. Nothing force-pushed, no
history rewritten. If the owner later pushes or merges this branch, the
merge is recorded here as a dated merge record ([README.md](README.md),
rule 2).
