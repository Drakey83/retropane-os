# Run GC2EA: the temperature guard on the X51v (R76, the owner's remaining list of 2026-11-12, item 2)

- **Date:** 2026-11-12, 9:26-9:29 PM
- **Device:** the owner's Dell Axim X51v, already on the loader, unattended
  (idle about ten minutes after run today-0141)
- **Card log:** gpubench-0132 (`local/cardlog/`, not in git)
- **Command:** `RP_GPUBENCH_GC2_TEMPGUARD=1 RETROPANE_LOG_WAIT_SECS=3600
  cargo xtask load rp-gpubench`
- **What it is:** the guard's code (`rp_drv_mbx_display::heat::HeatGuard`,
  ADR-0035 "The temperature guard for games") driving the 2700G's clock
  while the core draws GC2DT's four moved game frames, every render checked
  against its reference made at Windows Mobile's clock, `TEMP0` read every
  10 s.

## Why this run's thresholds aren't the shipped ones

The shipped trip is 866 (the first reading hotter than every run that
proved the game step; their hottest read 867). GC2DV held 867 through 4,800
renders at 137.80 / 1, the chip at its hottest in a room: a run can't be
counted on to reach 866 on a cool night. So this run trips **3 counts
warmer than its own start** and clears 2 counts cooler than that: the
guard's mechanism on the device (readings in, the clock stepped between
frames, the display unharmed, every frame right on both rungs), the
shipped numbers in the host tests (`heat.rs`, seven tests).

## Result

| | |
|---|---|
| TEMP0 at the start | 882 (idle); this run's trip 879, clear 881 |
| Before heating, 136.50 / 1 | the clock set (underrun flag after: clear); 100 of 100 renders right |
| Heating, drawing at 136.50 / 1 | TEMP0 881, 880, 880, 879 at 11-41 s; 280 of 280 renders right |
| **Step down** at 41 s, TEMP0 879 | to 133.25 / 1, the underrun flag clear; the check after: **100 of 100 right** |
| Cooling, nothing drawn, the 3D clock off | TEMP0 879, 880 (x 5), 881 at 20-80 s |
| **Step up** at 80 s of cooling, TEMP0 881 | to 136.50 / 1, the underrun flag clear; the check after: **100 of 100 right** |
| After | Windows Mobile's clock put back; its controls 4 of 4 right; the pool given back whole; 0 hung |

The guard stepped down once at the trip and up once past the band, never
inside it (TEMP0 880 five times while cooling: no step, the hysteresis
holding). Each change took the clock through `clock::begin` (the writes
read back) and left the display's underrun flag clear.

## Run again on the committed code (9:34 PM, gpubench-0133)

The first run idled the core by writing `MBXCLK` itself; the policy lint
refuses that outside the driver (audit C43), so the cooling phase now uses
the driver's own gate (`clock::Gate`, as the OS idles the core: the same
register value). Run again on exactly that code: TEMP0 881 at the start
(trip 878, clear 880); **step down at 31 s** (878), 100 of 100 right at
133.25 / 1; **step up after 60 s cooling** (880), 100 of 100 right at
136.50 / 1; 188 of 188 while heating; the underrun flag clear after each
change; Windows Mobile's controls 4 of 4; 0 hung.

## What it doesn't show

- **The shipped trip itself:** the chip never read hotter than 879 here;
  866 is past every run so far, and neither rung has run there (R71's
  self-test measures each unit's envelope; decisions-2026-11-12-evening.md
  item 2).
- **A game:** the frames are GC2DT's test frames; the guard is wired to
  games with the game path (R90).
