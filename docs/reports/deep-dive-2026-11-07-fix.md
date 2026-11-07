# Deep dive fix, 2026-11-07: the owner's rulings, fixes a-f, what proves them

Branch `fix/deep-dive-2026-11-07`, from main at 3b8e02f1 (the 2026-11-06
branch, merged). **Not pushed** (the owner's list: do not push). Each commit
named below; CI (`cargo xtask ci`) run in a separate checkout as noted. Gate:
30 of 30 architectural lines for phases 2, 4 and 5 (`cargo xtask gate`).

## The rulings (docs/reports/decisions-2026-11-06.md, every answered box ticked)

| # | Ruling | What changed | Commit | Proved by |
|---|---|---|---|---|
| 1 | ADR-0035 amendment E accepted | ADR-0035 marks E ACCEPTED 2026-11-07; its device check then ran and passed (f) | d56a5ec7, this report's commit | `cargo xtask lint`; the P8 run (f) |
| 2 | Loader zeroed RAM 36 -> 42 KiB approved | `xtask/src/size.rs` marks it approved by the owner 2026-11-07 | 8f1dd72a | the size table's tests; `cargo xtask cross` |
| 3 | Four slower floors approved | `docs/perf-floors.md`: cache_walk_40k 295100 -> 298534, cache_walk_48k 353757 -> 357904, pen_down_period_worst 12664 -> 12724, landing_to_verdict_median 8920 -> 8924 (the last session's medians), each marked approved by the owner 2026-11-07 | d56a5ec7 | `cargo xtask perf sessions` (the last session's medians) |
| 4 | Re-measure: yes, not now | Run sheet with the exact commands, not run: [r109-perf-remeasure.md](../research/sheets/r109-perf-remeasure.md) | d56a5ec7, 2cc5fab3 | lint (CI on 2cc5fab3 passed) |
| 5 | Game Mode sheet A, alternative 3 | Sheet A marked approved; design B drafted, design only: [game-mode-b.md](../design/game-mode-b.md) (its open questions: the slot limit of ADR-0003, the process model not yet on the X51v, the reserve, the forced-way-back keys) | d56a5ec7 | docs only |
| 6 | No fresh R90 draft | Recorded on R90 and the decisions file; ADR-0038 not reopened | d56a5ec7 | docs only |

## The fixes

| | What changed | Commit | Proved by | Device evidence owed |
|---|---|---|---|---|
| a | Both 2026-11-06 reports say the branch merged to main at 3b8e02f1 | d56a5ec7, this report's commit | lint | none |
| b | **The 43-47 ms "Radar slide-in" was the emulator's host clock, not work** (the same still frame 20 ms on an idle PC, 68 ms beside a build; the quiet run itself failed the budget). Under `-icount` the slide-in frame was 22.4 ms (5.8 ms CPU, 16.5 ms waiting for the scan). The look check now runs under `-icount shift=1` and fails any frame over 33 ms, moving ones too. New numbers: worst moving 13,984 us, no still frame over 10 ms, the Radar page's frames at most 10.1 ms | 389d5014 | the budget test (a moving frame over 33 ms fails); `emu-today-look` passes | the X51v's own frame times (R185 runs logged a worst draw of 72-115 ms; the emulator models no bus timing) |
| c | Glyph atlases evicted one at a time, oldest first, the frame's own pinned by looking ahead at its text; the whole flush only when the frame's own atlases fill the memory; the gap search sorts nothing (the sort cost 5 KB an image). New numbers (emulator, `-icount`): the first look switch uploads 5 atlases, not 10; the return to Today 2, not 9 | 6afe4b6d, dfbc4490 (6afe4b6d alone put rp-scene over budget; dfbc4490 passed CI), 8f1dd72a (rp-scene's divide-1 variant budget 174 -> 177 KiB) | `a_look_switch_uploads_only_its_new_atlases` (2 uploaded not 5, pixels equal to the reference); two mutants; `emu-today-look` | a look switch on the X51v |
| d | **Both R186 failures, one cause:** since 55e6e220 (2026-10-28) the renderer draws quads as four-corner objects, which the fork's 3D model at rp-v0.31.0 doesn't read (its list walk drops the tile's list at 0x60A00000, so only the background is drawn; G4a's "row 132" is the background grey decoded). `emu-mbx3d-api` passes at 55e6e220^ and fails at 55e6e220 with R186's BAD. The fork's existing commit 7fad420638, alone on rp-v0.31.0 (local branch `quad-objects`, cbba0bae4f, not pushed), fixes both. A second cause in our xtask: the R69 gpubench runs (from 38c2a6fd) were checked as if the fork started Windows Mobile's clocks; a clean refusal now counts when nothing else ran | 020ac9d0 (xtask); the fork commit is local | `emu-mbx3d-api` every check and `emu-gpubench` all runs (177 OK, 134 of them clean refusals) on the four-corner build; `a_gpubench_clock_refusal_counts_only_when_nothing_else_ran` | none; **left:** publishing the fork as rp-v0.31.1 and bumping `QEMU_RELEASE` needs a push to the fork (the owner's go) |
| e | `xtask/src/emu/mod.rs` 18,092 -> 3,512 lines: gpubench.rs, gpu.rs, loader.rs, card.rs, audio.rs, drive.rs, display.rs, usblink.rs; moves only | 2a48e962 | `cargo test -p xtask` 168 pass, clippy clean; **the full emulator suite after the split:** on the four-corner build every check to `glesfeat` G passes (94), G2 stops as expected there (its judgement encodes the release's missing quads); on the release, from `glesfeat` to the end every check passes but `emu-mbx3d-api` and `emu-gpubench` run A, red as R186 records and green on the four-corner build | none |
| f | **The P8 device check ran and passed** (the X51v on the loader, unattended): every screen-1 render at 143.00 / 1 took the cures by the clock (150 of 150, 50 of 50), torn 0 of 152 and 0 of 173, DSIG equal to the reference on all 8 one-offs, failed 0; the control P7 (cures off) tore every refresh | this report's commit | [run report](runs/2026-11-07-scene-p8.md) | none for amendment E |

## Notes

- **Due reminders at the device run:** none folded in: the run was one
  fixed image, unattended, with the owner away; the others need another
  image or him there.
- **Device evidence still owed:** the X51v's frame times for b and c; the
  re-measure (ruling 4, on his go); C97/C98, the lent pages, C21, R181-R183
  from the 2026-11-06 report.
- **Stopped there.** No new feature started; no settled ruling reopened.
