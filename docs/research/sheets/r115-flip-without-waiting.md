# Sheet: R115, the 2D flip without the CPU waiting, 2026-10-30

**What it decides.** How a 2D flip reaches the vertical blanking without
the CPU spinning for it. Since R113's fix (flips written only in lines
640-644, because a `GSCADR` write shows at once and tore mid-scan),
`Mbx2dRenderer::present` polls `DVLNUM` until the blanking, up to a whole
refresh (16.6 ms). Measured on the X51v (`cargo xtask perf`, today-0012,
`local/perf-2026-10-30-R115b.txt`):
- touch to flush: median 10,388 -> 19,924 us (+92%), worst 42,804 ->
  49,384 us;
- pen-down sampling period: worst 12,664 -> 23,336 us (+84%), because the
  pen isn't read while the CPU waits;
- queue to flip unchanged (5,593 -> 5,585 us).

The owner, 2026-10-30: "knock out" the reminders that came due. R115's
two ways were new floors or this fix, so this is the fix, a sheet first.

## 1. Original way (Windows Mobile / Dell's product)
Flips happen only in the blanking, from the VEVENT0 interrupt. W2
measured 1,534 flips, all on line 643. The CPU never waits for a flip.

## 2. Dell's way (REA, R113 sheet, log #3-#8)
`ddi_2700g`'s display interrupt (0x0211d284, SYSINTR 27 on GPIO85):
1. It clears `DINTRS` bit 2.
2. With flips queued, it writes `GSCADR` (0x0211d30c), then `DUCTRL` = 1.
3. VEVENT0 is enabled in `DINTRE` only while flips are queued.
4. `DVECTRL` is never written, so VEVENT0 stays on line 643.

GPIO85 is shared with the 3D core's render done.

## 3. Modern way
Display registers are double-buffered and latched at vblank, so a driver
writes them at any time (DRM atomic commits; training knowledge, a claim).
R113 tried the 2700G's candidates for a latch (`DSCTRL` update bits with
`DUCTRL`): no latch on this panel. Where no latch exists, a compositor
schedules the commit against the known scan timing instead of blocking
(a frame deadline, the way our 3D service already starts frames behind
the scan).

## 4. The alternatives, and why ours wins
| | What | Wins | Loses |
|---|---|---|---|
| A. New floors | Keep the wait, approve the slower numbers | No work | Touch twice as slow at the median; the pen sampled unevenly |
| B. Dell's | The VEVENT0 interrupt on GPIO85 wakes the display host, which writes the queued flip | The CPU never waits; proven by Dell | GPIO85 is shared with render done, so the 3D driver and the display need one owner of the line. Our runtime wakes the owner (ADR-0007: signalled, then serviced), so its wake latency must land inside lines 643-644 (~51 us) unless VEVENT0 is moved earlier with `DVECTRL` (Dell never does) |
| C. Ours: the flip as a deadline | The renderer queues the landed frame's flip and returns. `flip_due()` gives the blanking's time from `DVLNUM` and R111's measured line period (25,687 ns). The main loop keeps sampling the pen and serving input until then, then spins only from ~line 636 (~0.1-0.2 ms) and writes in 640-644 | No interrupt plumbing; works in today's polled loop; the CPU waits ~0.2 ms instead of up to 16.6; the same scan model the 3D service starts frames with (one design, not two) | A loop busy past the deadline (a long draw) lets the flip slip a refresh. Each slip is counted (`late_flips`, as now) and measured |
| C + B later | C now; when the display host owns GPIO85, the VEVENT0 interrupt becomes C's wake source | Interrupt precision with C's design; nothing rewritten | The interrupt half waits for the line's single owner (the 3D service, ADR-0026 rule 5) |

**Recommended: C now, B as its wake source later.** It fixes the measured
cost where it arises (the spin) with the timing model already proven on
the device, and keeps every flip in the blanking.

## 5. The plan
1. `rp-mbx-render`:
   - `present` splits into a queued flip (`pending_flip`, set where the
     fence lands) and `commit_flip(now)`, which writes only inside
     `BLANK_LINES`.
   - `flip_due()` gives the time to line 636 from `DVLNUM` and the scan
     model.
   - A flip still waits at most a refresh: past that it's written and
     counted late, as now.
2. The callers (`rp-today`'s loop, the graphics service's `poll`): wait
   for `flip_due` the way they wait for a 3D `Step::Wait`, reading the pen
   meanwhile.
3. Host tests on the model display:
   - every flip written in 640-644;
   - none lost;
   - the loop's pen reads continue while a flip is queued;
   - a slipped deadline is counted;
   - mutants for each.
4. **The device check (the owner's go):** `cargo xtask perf` on
   `rp-today`, judged against R115b and the pre-R113 numbers; R113's
   `DSIG` tear check on the flips (no torn refresh).

## 6. What would overturn it
- **Touch to flush not back within ~1 ms of 10.4 ms (median), or the pen
  period's worst not back near 12.7 ms:** the wait wasn't the whole cost.
  Measure where the rest goes before anything else.
- **Any torn refresh:** a deadline-timed write landed outside the
  blanking. Widen the spin's start or go to B.
- **Late flips above 1 in 100 in the perf run:** the loop's load makes
  deadlines unreliable. B, the interrupt as the wake, comes first.

**Asked of the owner:** the go to build C (steps 1-3); a separate go for
the device check (step 4).

## Built (the owner's go 2026-10-30, "Yes, build it")
- **Where the wait was:** the graphics service, not the renderer's
  callers. With no scene, the service flipped the landed 2D frame from
  its `poll` (the renderer's `present` spun there).
- **`Job::Flip`:** the service now returns a wait until the scan is
  `FLIP_LEAD_US` (2.2 ms, 86 lines at 60 Hz) from the blanking, then
  flips. The renderer's own wait (in the blanking only) covers the rest.
  `Starts::flips_waited` and `flips_slipped` count each flip, and each
  one that came more than a refresh late.
- **`Graphics::busy`:** `rp-today` lets a new frame wait a pass while one
  is in flight, instead of spinning on `poll`. It wakes every 1 ms
  meanwhile.
- Host test plus two mutants; `emu-today` passes.
- The interrupt (B) is left for later, as recommended.

## The device check (2026-10-30 7:47 AM, `cargo xtask perf`, today-0013, `local/perf-2026-10-30-R115c.txt`)
| | before R113 (tearing flips) | R115b (blocking wait) | R115c (this build) |
|---|---|---|---|
| pen-down period, worst | 12,664 us | 23,336 us | **12,644 us** |
| pen-down period, median | 6,952 | 6,952 | 6,952 |
| touch to flush, median | ~10.4-13.3 ms | 19,924 us | **24,208 us** |
| touch to flush, worst | 42,804-44,260 | 49,384 | **57,972 (over the 50 ms promise)** |
| queue to flip, median | 5,593 | 5,585 | 5,574 |

**Reading it (results, not decisions):**
- **The pen is sampled evenly again:** the worst gap is back to before
  R113.
- **Touch to flush got worse**, by 4.3 ms at the median and 8.6 ms at
  the worst. This build doesn't log its slip counter in `rp-today`, so the
  cause isn't measured yet. The likely one is a pass of the loop (a pen
  sample is ~2 ms) running past the 2.2 ms lead, so the flip misses that
  blanking and waits a whole refresh.
- **This sheet's expectation was wrong.** The pre-R113 10.4-13.3 ms was a
  flip written at once, mid-scan, which tore. A flip that waits for the
  blanking adds half a refresh on average (~8 ms) however the CPU waits.
  So R115b's 19.9 ms was mostly the blanking itself, not the spin. The
  overturn criterion ("back within ~1 ms of 10.4 ms") can't be met by any
  flip timed to the blanking.

## The follow-up's device check (2026-10-30, perf, today-0014, `local/perf-2026-10-30-R115d.txt`)
The flip polled first after waking, at its own deadline, with slips
counted (the owner's "Yes, fix and re-run"):

| | R115b (blocking) | R115c | **R115d (this build)** |
|---|---|---|---|
| pen-down period, worst | 23,336 us | 12,644 | **12,648** |
| touch to flush, median | 19,924 | 24,208 | **22,212** |
| touch to flush, worst | 49,384 | 57,972 | **50,712** |
| queue to flip, median | 5,585 | 5,574 | **4,950** |
| flips waited / slipped | - | not counted | **54 / 0** |

**Reading it (results, not decisions):**
- No flip missed its blanking (0 of 54), so slips don't explain this
  build's cost.
- Against the blocking wait, the pen is sampled evenly (worst gap halved)
  and the engine's part is 0.6 ms shorter. But touch to flush is 2.3 ms
  longer at the median, and its worst is 1.3 ms longer (0.7 ms over the
  50 ms promise).
- **Not measured, a hypothesis:** the blocking wait left the loop
  phase-locked to the blanking, so frames landed at a steady place in the
  scan; free-running, they land anywhere.
- **Either way, the blanking itself is most of the cost:** landing to
  verdict 8.9 ms + frame 0.3 + engine 5.0 leaves ~8 ms waiting for the
  blanking. Only a 2D change made without a flip (copied into the shown
  screen behind the scan, as the 3D service already does for scenes)
  removes it.
