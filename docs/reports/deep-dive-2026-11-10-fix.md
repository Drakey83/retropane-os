# Deep dive fix, 2026-11-10: the owner's seven items

Branch `fix/deep-dive-2026-11-10`, from main at 04d3d142. **Pushed as a
branch with a pull request, main not moved** (the owner's "Yes, push",
2026-11-10: [pull request 13](https://github.com/Drakey83/retropane-os/pull/13);
GitHub `main` still at 04d3d142; checked last, below). Device runs only on the X51v already
on the loader, unattended. Decisions left to the owner:
[decisions-2026-11-10.md](decisions-2026-11-10.md).

| # | Item | Outcome | Commits (CI) | Proof |
|---|---|---|---|---|
| 1 | Your item 3 rulings: picture cache, frame rule | **(a) Fixed, proven on the X51v; one case can't reach zero.** The cache and glyph memory split 384 / 384 KiB, pictures evicted one at a time (the oldest not drawn this frame); ADR-0023 rule 4, ADR-0027 and the memory budget updated. The return to Today uploads 1 picture, not 9, and takes 51.2 ms (53.5-57.2). On the host route through Hourly 5 cards still go up again (87 KB), because Hourly's own 316 KB and Today's set don't fit 384 KiB together: a tick-box. My 11-09 report put the 9 pictures at 315 KB; they were about 89 KB, corrected. **(b) Applied; the X51v doesn't meet it yet.** 33 ms for every frame, a single whole-screen change up to 45 ms as one late frame, the folded moving frames held to 33 ms; `cargo xtask look-judge` holds a device log to it. today-0115 fails it on four kinds of frame (the start-up burst, Settings drawn whole on network answers, the return at 51 ms, a half-screen frame), opened as R191 with each cause still to find | 7a71c003 (CI failed: clippy), 6e308400 (CI failed: a host test still sizing the glyph memory as whole fonts), 59221658 (CI passed) | tests `a_full_picture_cache_evicts_the_oldest_picture_not_the_whole_cache` (and its mutant), the route test's picture checks (fail on the old cache), `a_single_whole_screen_change_may_take_45_ms_and_a_moving_frame_33`; run today-0115 |
| 2 | Your item 4 rulings: the compose, the second bisect | **(a) Fixed, proven on the X51v.** The app is composed ahead only after Today has been still for 1 s. Touch replay: frame and queueing 784 us twice (1,260 with the compose before, 852-868 without it), touch to flush 16,912 and 18,620 us (19,184-20,268 before; floor 16,248). Reveal composed ahead after stillness (today-0115), the old way for a hero tap at 531 ms (today-0118). **(b) Run; no single cause.** The compose held off in all 170 builds: 508 us at 4fa4dbad, 752 at main, climbing 60-85 us at a time over 10-30 to 11-01's features; the first commit over the line is f4ffea28 (word shadows, whole clouds, 16 atlases: +60 us). 11-09's "~2 ms" was mostly run-to-run spread in touch to flush (16,472 against 16,704 us with the compose off). The rest is a tick-box, stated as unattributed per commit | b7a060be, 4b69acfe (CI passed on both) | test `the_app_is_composed_ahead_only_after_a_second_still` and its mutant; runs today-0115 to -0136; docs/reports/runs/2026-11-10-r188-compose-rule.md |
| 3 | Item 5 of 2026-11-09 stays prepared | **Prepared, your steps written.** The X50v and cold-chip steps carried into decisions-2026-11-10.md unchanged; no new code needed; neither runs until you say the device is up | 1a3ae09b (CI passed) | the steps, word for word from decisions-2026-11-09.md |
| 4 | Stale push lines | **Fixed.** deep-dive-2026-11-09-fix.md now says pushed (main at 04d3d142, your "push to live"); every report in docs/reports/ checked: every commit any of them cites is on GitHub `main`, and the five whose headers said "nothing pushed" carry a checked push-state line. The rule is written at the end of this report | 61a020a7 (CI passed) | `git merge-base --is-ancestor` on every cited commit against origin/main (0 of 70 not on main); the backup branches the 10-29/10-30 reports name are in main |
| 5 | The game-safe ceiling | **Recorded.** Games ship up to 137.80 / 1, about 2.77x Windows Mobile's 3D speed, not 149.50 / 1's 3.0x: divide1-149.md (the goal's note and the verdict), the 2700G page and the datasheet, performance-outlook.md (a 137.80 / 1 row: 2.77x, 2.18x, 2.28x, 2.12x by kind, the fitted kinds checked against the fit's own 149.50 figures) and quake3-on-the-x51v.md (the row and a ~55 fps full-screen projection). No port plan changed | f9b8d815 (CI passed) | GC2DT 1,200 of 1,200, GC2DV 4,800 of 4,800 (2026-11-08) |
| 6 | R190: the flip back to screen 0 | **Fixed with a test and a mutant.** Yesterday's test reached screen 0 by the 2D renderer's own catch-up (opening the scene left 2D work). The branch runs when screen 1 is on screen, screen 0 already holds the frame and 2D has nothing to draw: 2D frames with no scene, an idle frame (which catches screen 0 up without a flip), then a scene. The new test drives that; with the branch skipped the 3D frame is drawn with screen 1 on screen and the test fails | 4b833ffd (CI passed) | `a_scene_after_an_idle_frame_on_screen_1_flips_back_to_screen_0`; the mutant "graphics: the flip back to screen 0 before 3D work skipped" |
| 7 | Fresh evidence behind C71 | **Fixed with proof: the closure stands.** Run CH C71 R-B again on the X51v: 0 of 120 partial frames differ from a fresh whole render, 0 unfinished, 0 hung, as on Oct 29; recorded in the audit. A first try lacked the harness switch and judged nothing (recorded) | 0138df14 (CI passed) | run gpubench-0131, docs/reports/runs/2026-11-10-c71-rb.md |

Reminders that came due and what happened: R171 (the budget rule applied),
R172's return leg (the picture cache), R188 (both rulings, the second
bisect), R190 (closed, done 4b833ffd). Opened: R191 (the frames the new
budget fails on the X51v). Device-run reminders needing the owner watching
or other images (R3, R4, R12, R22 and others) weren't folded into these
unattended runs.

## Found while re-verifying: GitHub's CI was red

GitHub's own workflow (Linux) has failed on every main push since
2026-10-24, 04d3d142 included:
[run 42731190233](https://github.com/Drakey83/retropane-os/actions/runs/42731190233),
conclusion **failure** ("couldn't find file `boot/gpubench/src/image/../busx.rs`").
Two test images named module files through a folder that doesn't exist,
which Windows (where `cargo xtask ci` runs) resolves and Linux can't. My
report of 2026-11-09 said CI passed: true of `cargo xtask ci` here, but not
of GitHub's workflow, which I didn't check. Fixed on this branch (e18bd531),
and the lint now refuses the pattern (5a66eafc), with a scan of every
include and path in the repo finding no other. **The CI run for this
branch's last commit:** `cargo xtask ci` passed on every commit of the
branch in a clean second checkout (`../rp-ci`) except the two of item 1
named in its row. After the owner's "Yes, push", GitHub's workflow ran on
a4caf626 through pull request 13:
[run 42950562626](https://github.com/Drakey83/retropane-os/actions/runs/42950562626),
conclusion **success**, all five jobs (fmt, clippy, lint and test; the
X51v cross-build; the emulator boot; the benchmarks; coverage). The first
green Linux run since 2026-10-24: no other Linux-only failure was hiding
behind the path one.

## Push state (settled last, then checked)

The branch is on GitHub (`git ls-remote origin` shows
`fix/deep-dive-2026-11-10`, pushed on the owner's "Yes, push") with pull
request 13 open against main, not merged; GitHub `main` is at 04d3d142,
the start of this branch. Nothing was force-pushed and no history
rewritten.

**The rule, so the next report doesn't go stale:** a report's push claim
is written last, after the push state is settled. It is checked against
GitHub (`git ls-remote origin`, and `git merge-base --is-ancestor` for the
commits it cites) just before the report is finished. When the state
changes later (a "push to live"), the report that claimed otherwise is
corrected in the same session. A CI claim names which CI: `cargo xtask ci`
here, or GitHub's workflow run, with its link and conclusion.

