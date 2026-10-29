# Sheet: a copy's time known from the start (fix, R20 / audit C94), 2026-10-29

**What it fixes.** The owner's ruling after GC2DF: before any copy has been
timed, a fallback frame is held. The service learns a copy's time only
from its own copies of an eighth of the screen or more, so a scene whose
frames never fit behind the scan, with no such copy made, stays held (audit
C94): nothing tears, but nothing draws.

## 1. Original way (Windows Mobile / Dell's product)
No copy is ever timed: full-screen 3D flips at VEVENT0 from a back buffer,
windowed GLES draws straight into the screen, GDI blits are fire-and-wait.

## 2. Dell's way (REA)
`ddi_2700g` imports only `Sleep` and `GetTickCount` for time (REA log #22).
`GetTickCount` is read in one place, 0x0211d7f8 (x 1000: a microsecond
clock in 1 ms steps), used only for 500 ms time-outs around waits
(0x02101508, 0x02111984, 0x0211ef24, 0x0211f2dc; log #23-#24). No timing
or calibration of the 2D engine anywhere: Dell never needs one, because it
never copies into the picture being shown.

## 3. Modern way
Drivers measure the engine instead of assuming it: GPU timestamp queries
around each job, and the job's own completion time fed back into
scheduling (frame pacing). Training knowledge, a claim.

## 4. The alternatives, and why ours wins
| | What | Wins | Loses |
|---|---|---|---|
| A. Do nothing (today) | Hold until a large copy happens to be timed | No code | C94: a scene can stay held |
| B. A built-in time | GC2DE/GC2DF's 3,149-3,356 us at 149.50 / 2 as a constant | Instant | A copied value: one unit, one clock (the any-Axim rule); wrong at another rung of ADR-0035's ladder |
| C. Dell's | Never copy into the shown picture: flip from a back buffer | No timing needed | GC2DF: option 1's flips tore 48 of 48 and freed the core 15.7 ms later |
| **Ours** | When the service starts, and whenever the clock changes, time one whole-screen 2D copy of screen 0 into screen 1 (screen 1 is never shown while the 3D core can draw, and holds the same frame when idle, so the copy changes nothing visible) into `Graphics::copies`; the copies the service makes afterwards keep the band current, as now | A measured time on this unit at this clock from the first frame; no constant; ~3.4 ms once | One copy at start; a clock change must tell the service (a call that resets the render and copy histories and re-times) |

## 5. What would overturn each, and the test
- **Ours overturned** if the start-up copy's time falls outside the band of
  the copies timed during the run (the plan takes 4/5 of the fastest and
  twice the last): then a start-up time can't stand in for a running one.
  Test: the service's device run (R110) logs the start-up copy's time
  beside every timed copy after it, and counts held frames (should be 0
  after start) and torn refreshes by `DSIG` (should be 0).
- **B overturned** by the same log at a second clock (133.25 / 2 or
  Windows Mobile's): a different time.
- **A's risk shown** by `Starts::held` above 0 in a scene that never
  fits behind the scan.
- Host tests: a new service has a copy's time; a clock change resets the
  histories and re-times; a held frame never happens once started.

**Asked of the owner:** the go to build ours (the service only: start-up
and clock-change timing, its host tests, mutants); its device proof
folds into R110's run, whose own sheet also waits for your go.
