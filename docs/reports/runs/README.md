# Device-run reports

The run logs the PC keeps (`local/`) are not in git, so a run number in a
sheet could not be checked from the repository. Each report here is the
committed evidence of one run on the X51v: the run's ID and card-log
number, the command, what it measured, and its verdict and measured lines
copied from the log.

## The rule (the owner's list, 2026-11-06, item 2)

- **Every new device-run claim in a sheet points at a report here**, not
  at a `local/` file. A sheet may still name its `local/` log as where the
  whole log is kept, beside the report.
- `cargo xtask lint` enforces it for every sheet not in its list of sheets
  written before the rule (`xtask/src/policy.rs`, `RUN_REPORT_EXEMPT`).
- **Before adding a report:** read it whole. No keys, tokens, passwords,
  places (coordinates, map tile numbers, the kept maps' key names), user paths or personal data. The reports here were
  made by a script that copies the verdict and measurement lines and
  redacts those patterns; every line it flagged was read (the only flags
  were the request name `RPNET TEMPEST`, which carries no key: the device
  never logs it, audit C95).

## Reports

| Run | Date | What | Report |
|---|---|---|---|
| gpubench-0237 | 2026-10-30 | R121 test 1 | [2026-10-30-gpubench-0237.md](2026-10-30-gpubench-0237.md) |
| gpubench-0238 | 2026-10-30 | R121 test 2 | [2026-10-30-gpubench-0238.md](2026-10-30-gpubench-0238.md) |
| gpubench-0239 | 2026-10-30 | R121 test 3 (the overlay) | [2026-10-30-gpubench-0239.md](2026-10-30-gpubench-0239.md) |
| gpubench-0240 | 2026-10-30 | R117T, the speed switch's hour | [2026-10-30-gpubench-0240.md](2026-10-30-gpubench-0240.md) |
| gpubench-0241 | 2026-10-30 | R121S, the overlay's hour | [2026-10-30-gpubench-0241.md](2026-10-30-gpubench-0241.md) |
| scene-0008 | 2026-10-30 | R121 through the service | [2026-10-30-scene-0008.md](2026-10-30-scene-0008.md) |
| scene-0009 | 2026-10-30 | R121 through the service, run 2 | [2026-10-30-scene-0009.md](2026-10-30-scene-0009.md) |
| today-0054 | 2026-11-06 | R184 step 2, first run (froze) | [2026-11-06-today-0054.md](2026-11-06-today-0054.md) |
| today-0055 | 2026-11-06 | R184 step 2 after way A | [2026-11-06-today-0055.md](2026-11-06-today-0055.md) |
| r184s3a | 2026-11-06 | R184 step 3, run 1 start 1 | [2026-11-06-r184s3a.md](2026-11-06-r184s3a.md) |
| r184s3b | 2026-11-06 | R184 step 3, run 1 start 2 | [2026-11-06-r184s3b.md](2026-11-06-r184s3b.md) |
| r184s3c | 2026-11-06 | R184 step 3, run 2 start 1 | [2026-11-06-r184s3c.md](2026-11-06-r184s3c.md) |
| r184s3d | 2026-11-06 | R184 step 3, run 2 start 2 | [2026-11-06-r184s3d.md](2026-11-06-r184s3d.md) |
| today-0062 | 2026-11-06 | R185 look check, run 1 | [2026-11-06-today-0062-r185-run-1.md](2026-11-06-today-0062-r185-run-1.md) |
| scene-0001 (P8) | 2026-11-07 | ADR-0035 amendment E's device check: passed | [2026-11-07-scene-p8.md](2026-11-07-scene-p8.md) |
| r185a | 2026-11-06 | R185 look check, run 2 | [2026-11-06-r185a.md](2026-11-06-r185a.md) |
| r185b | 2026-11-06 | R185 look check, run 3 (the pass) | [2026-11-06-r185b.md](2026-11-06-r185b.md) |
| today-0065 | 2026-11-08 | Look check at 4 PM: failed, two faults found (no page after Radar; a stale page on a late settings change) | [2026-11-08-today-0065.md](2026-11-08-today-0065.md) |
| today-0071 | 2026-11-08 | The fix for today-0065: 0 wrong, a hand-off after every Radar exit | [2026-11-08-today-0071.md](2026-11-08-today-0071.md) |
| today-0082 | 2026-11-09 | R172's fixes on the look route: look switch 88-95 -> 40-43 ms, no glyphs uploaded again, the return 53.5 ms (pictures left); look check 0 wrong, 0 unknown | [2026-11-09-today-0082.md](2026-11-09-today-0082.md) |
| R188 bisect, today-0083 to -0109 | 2026-11-09 | The touch replay's slowdown: first slow commit 4fa4dbad (the app composed ahead), about half the rise | [2026-11-09-r188-touch-replay.md](2026-11-09-r188-touch-replay.md) |
| glesfeat-0028, -0029 | 2026-11-09 | C53: after a TA hang the reset alone leaves the core hung; the 3D clock cycled first brings it back, 3 of 3 | [2026-11-09-glesfeat-m2.md](2026-11-09-glesfeat-m2.md) |
| gpubench-0126, -0127 | 2026-11-09 | GC2DX: the renderer's own memory guarded past the line: 0 changes in 2,000 renders at 149.50 / 1 (R189) | [2026-11-09-gc2dx.md](2026-11-09-gc2dx.md) |
| today-0110, -0111 | 2026-11-09 | The look check by day: 0110 lost its verdict to a full log (fixed), 0111 0 wrong, 0 unknown, both looks on every page (R187) | [2026-11-09-today-0110-0111.md](2026-11-09-today-0110-0111.md) |
| today-0113, -0114 | 2026-11-09 | R187 at 4 PM: the old Hourly points on the graph left 48 unknown; refitted off it: 0 unknown, dark Hourly judged | [2026-11-09-today-0110-0111.md](2026-11-09-today-0110-0111.md) |
| gpubench-0110 to -0124 (GC2DG-GC2DV) | 2026-11-08 | The divide-1 chase for games: 137.80 / 1 proven, 149.50 / 1 not reached | [2026-11-08-gc2d-chase.md](2026-11-08-gc2d-chase.md) |
| loader-0001 | 2026-11-13 | R193, the X50v's start-up log | [2026-11-13-loader-0001-x50v.md](2026-11-13-loader-0001-x50v.md) |
| loader-0002 | 2026-11-13 | R193, X50v: CPLD 0x1C bit 3 on, USB configured | [2026-11-13-loader-0002-x50v-bit3.md](2026-11-13-loader-0002-x50v-bit3.md) |
| loader-0003 | 2026-11-13 | R193, X50v: found by itself, the stale EMCE cleared | [2026-11-13-loader-0003-x50v-detect.md](2026-11-13-loader-0003-x50v-detect.md) |
| gpubench-0001 (X50v) | 2026-11-13 | GC2DR on the X50v: every game frame right to 139.75 / 1 | [2026-11-13-gpubench-0001-x50v-gc2dr.md](2026-11-13-gpubench-0001-x50v-gc2dr.md) |
| gpubench-0007 (X50v) | 2026-11-13 | GC2EH: 145.60 / 1 soak-proven on the X50v | [2026-11-13-gpubench-0007-x50v-145.md](2026-11-13-gpubench-0007-x50v-145.md) |
