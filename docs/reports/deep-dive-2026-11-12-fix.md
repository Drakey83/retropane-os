# Deep dive fix, 2026-11-12: the owner's rulings on the 11-11 boxes

Branch `fix/deep-dive-2026-11-12`, from main at 1bb345a9. **Not pushed**
(the list says not to; checked last, below). Device runs only on the X51v
already on the loader, unattended (three look-route runs, today-0138 to
-0140). Decisions left to the owner:
[decisions-2026-11-12.md](decisions-2026-11-12.md). Item 5 of the 11-11
list (the X50v and cold-chip runs) is not in this list: it stays prepared
as written in [decisions-2026-11-11.md](decisions-2026-11-11.md), not run.
Gate: 30 of 30 architectural lines for phases 2, 4, 5 (`cargo xtask gate`).

| # | Item | Outcome | Commit (`cargo xtask ci`) | Proof |
|---|---|---|---|---|
| 1 | R191: keep Today's last picture | **Applied, proven on the X51v.** Today's screen kept in the 2700G's spare 600 KiB at `0x200000` (screen 0's old place: the pool unchanged), composed ahead while another screen is up and still a second (the Reveal pattern), the return copying it in and drawing only where Today's list differs. **The return to Today: 1,172 us of CPU** (was 55.5 ms), under the 45 ms allowance; look check 0 wrong; `look-judge` verdict OK. Two runs on the way, each a cause found in its log: today-0138 refused the kept screen (the radar's map written into the wallpaper and put back changed its generation: fixed by naming the wallpaper), today-0139 started from it but kept light and came back dark (60.3 ms: fixed by composing ahead). **Re-measured:** the Start menu 38,552 us, still over its 33 ms: a new tick-box with the numbers, not redesigned; the first Light switch 45,068 us is a one-time atlas build per look (the switch back 41,756 us, 0 atlases): recorded as such. ADR-0023 rules 2 and 4 amended | d5782096 (passed) | tests: the renderer's return pixel-checked against the reference, its refusals; the service's keeper-only, one-call, compose-ahead; the shell's every pixel changed while away inside what the return marks; 3 mutants, all killed; [run report](runs/2026-11-12-today-0140.md) |
| 2 | Hourly: accept | **Ticked and recorded.** 87,120 bytes, about 2.6-2.9 ms, only on a return from Hourly drawn on screen 0; 1 picture on the X51v's route; (b) and (c) rejected for the box's reasons. Nothing in code | e22ded19 (passed) | decisions-2026-11-12.md item 2 |
| 3 | Bisect remainder: accept, floors re-set | **Ticked, floors re-set** in docs/perf-floors.md: frame and queueing median 784 us (was 380), touch to flush median 16,912 us at 10.0993% (was 16,204 at 0.0494%), runs today-0116 and -0117; both rows name the ruling. No re-bisect | a38a40b7 (passed) | decisions-2026-11-12.md item 3 |
| 4 | The two CI runs in progress at the last review | **Both success**, all five jobs: [43298504569](https://github.com/Drakey83/retropane-os/actions/runs/43298504569) on 1bb345a9 (push to main, completed 2026-11-12 22:24 UTC) and [43297706032](https://github.com/Drakey83/retropane-os/actions/runs/43297706032) on 37c815c9 (push, completed 22:39 UTC) | this report's commit | `gh run view` on each |

Reminders that came due: R191 (its (3), the return, fixed by today-0140;
(4), the Start menu, a new tick-box; it stays open). The other device-run
reminders need the owner watching or other images, so none was folded
into these unattended runs.

## CI

- **GitHub's workflow:** the newest runs on `main`,
  [43298504569](https://github.com/Drakey83/retropane-os/actions/runs/43298504569)
  (1bb345a9) and
  [43297706032](https://github.com/Drakey83/retropane-os/actions/runs/43297706032)
  (37c815c9): conclusion **success**, both. No GitHub run exists for this
  branch: the workflow runs on a push to main or a pull request, and the
  list says not to push.
- **`cargo xtask ci` (this PC, Windows):** run on every commit of the
  branch in a clean second checkout (`../rp-ci`); each commit's result is
  in the table, the last commit's written after this report's commit runs.

## Push state (settled last, then checked)

Not pushed: `git ls-remote origin` shows no `fix/deep-dive-2026-11-12`,
and GitHub `main` is at 1bb345a9, this branch's start. Nothing
force-pushed, no history rewritten. If the owner later pushes or merges
this branch, the merge is recorded here as a dated merge record
([README.md](README.md), rule 2).
