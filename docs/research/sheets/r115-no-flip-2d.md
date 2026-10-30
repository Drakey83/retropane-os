# Sheet: 2D changes without a flip (R115, part 2), 2026-10-30

**What it decides.** How a 2D change (a tap's answer, with no 3D scene
open) reaches the screen without waiting for the blanking. The owner,
2026-10-30: "design your version please", after a plain-words comparison
of three choices (blocking flip, deadline flip, no flip).

Measured on the X51v, touch to flush (`rp-today`'s replay, `cargo xtask
perf`):

| | median | worst | longest pen gap |
|---|---|---|---|
| before R113 (flips mid-scan, tearing) | 10.4-13.3 ms | 42.8-44.3 | 12.7 |
| R115b (blocking flip in the blanking) | 19.9 | 49.4 | 23.3 |
| R115d (deadline flip, today's build) | 22.2 | 50.7 | 12.6 |

Today a tap's answer takes:
- the touch filter's verdict, 8.9 ms;
- the frame and its queueing, 0.3 ms;
- the 2D engine drawing it into the hidden screen, 5.0 ms;
- the wait for the blanking to flip, **~8 ms on average**: half a refresh,
  whatever the CPU does meanwhile.

Only a change made without a flip removes that wait.

## 1. Original way (Windows Mobile / Dell's product)
GDI draws straight into the shown screen (REA #15, #19), so a change
shows as it lands, with no ordering against the scan: it can tear.
Flipping surfaces (games) flip at VEVENT0 (W2, line 643).

## 2. Dell's way (REA)
Nothing between the two: no scan-timed 2D writes, no `DVLNUM` use in
GDI's path (REA #13). A finding, not an answer.

## 3. Modern way
- Desktop compositors build each frame off screen and present it at
  vblank (Wayland, DRM atomic). That is the flip we have: it always pays
  the wait.
- Latency-first systems write the front buffer behind the beam ("beam
  racing": VR's front-buffer rendering, emulators' scan-line sync). They
  pay no vblank wait and don't tear, because each write lands where the
  scan has already passed.
- Training knowledge, a claim.

## 4. The alternatives, and why ours wins
| | What | Wins | Loses |
|---|---|---|---|
| A. Blocking flip (R115b) | As last night | 19.9 ms median, inside the promise | The pen unread up to 23 ms while the CPU waits |
| B. Deadline flip (R115d, today) | As built | The pen read evenly | 22.2 ms median, worst 50.7 (0.7 over the promise) |
| C. GDI's way | The engine draws straight into the shown screen, any time | No wait, no copy | Tears whenever the scan crosses the change while it is drawn (R113 measured mid-scan writes tearing) |
| D. Draw straight into the shown screen, timed behind the scan | The engine draws the frame into screen 0 once the scan has passed its rows | No copy | The engine's 5 ms draw (blends, the background) spans many scan lines, so a large change can't stay behind the scan; the CPU can't pace a draw already queued |
| **Ours: draw hidden, copy behind the scan** | As today, the engine draws the frame into the hidden screen. Then **only its changed rectangles** are copied into the shown screen at a scan line the copy stays behind (`scan::copy_window`). Screen 0 stays shown, and a copy with no window falls back to B's flip | No blanking wait: a tap's change (a few KiB) copies in well under a millisecond, mostly at once. No tearing (the 3D service's own path for 2D beside a scene: 0 torn in 2,500+ refreshes, scene-0004). The pen read evenly. One path whether or not a scene is open, and screen 0 always shown, so opening a scene never needs a flip back | A copy per frame (the engine's time, ~0.5 ms a 100 KiB change; a whole-screen change 2-6 ms, or the flip fallback). A change across rows the scan is about to reach waits for its window (at most its own rows' time, usually nothing) |

**Expected:** touch to flush ~14-15 ms at the median (8.9 + 0.3 + 5.0 +
the copy), against today's 22.2 and the blocking flip's 19.9, with the
pen still read evenly. That is a prediction to measure, not a promise.

## 5. The plan
1. **`services/graphics`:**
   - With no scene open, a 2D frame goes the way 2D beside a scene already
     goes (`Job::Copy2d`): drawn into screen 1, its drawn rectangles
     copied into screen 0 at a line from `copy_window`, then
     `keep_front()` (both screens hold the frame).
   - With no window (a change too large for its rows' time, or no copy
     timed yet), today's deadline flip (B) is the fallback, counted.
   - New counters in `Starts`: copied behind the scan, and flipped
     instead.
   - **Watch out:** the R110 build found whole-screen changes too long for
     a window. Those keep flipping; they are rare (a wallpaper change, the
     first frame).
2. **Host tests:**
   - a tap-sized change copies behind the scan and never flips;
   - screen 0 stays shown;
   - both screens hold the frame after;
   - a whole-screen change falls back to the flip;
   - a stalled copy takes the existing stall path;
   - opening a scene afterwards needs no flip back;
   - mutants for each.
3. **Emulator:** `emu-today` and `emu-scene` (P4, the 2D flips phase,
   becomes 2D copies).
4. **Device checks (the owner's go, one at a time):**
   - `rp-scene`'s P4: every refresh judged by `DSIG`, torn 0;
   - then `cargo xtask perf`: touch to flush and the pen gaps against the
     table above.

## 6. What would overturn it
- **Any torn refresh in P4:** a copy wasn't behind the scan. Go back to
  B, and recheck `copy_window`'s rates for small copies.
- **Touch to flush median not below 19.9 ms:** no gain over the blocking
  flip, so the copy isn't removing the wait. Measure where the time
  goes.
- **The worst case still over 50 ms:** the cause is elsewhere (the touch
  filter's 8.9 ms or the engine's 5 ms). Look there next.

**Asked of the owner:** the go to build (steps 1-3); a separate go for
each device check (step 4).

## Built (the owner's go 2026-10-30, "Yes, build it")
- **`Graphics::two_d_alone`:** a 2D frame with no scene, drawn into
  screen 1 with screen 0 shown, goes by `Job::Copy2d`. A change over a
  quarter of the screen (`COPY_MAX_PX`), or one with no window, flips
  (`Job::Flip`). Copies are timed without a scene too.
- **`reach_line`:** found by `emu-today`. A copy waiting on a wake for a
  few lines' window missed refresh after refresh, so answers took up to
  1.8 s in the emulator. Now the copy's last stretch waits on the CPU, at
  most `FLIP_LEAD_US`. With that, the emulator's interactive answers had
  a median of 8.4 ms.
- **`rp-scene`'s P4:** the bar has a dot under it, so it isn't part of
  the background. Every frame is a tap-like change: 120 of 120 copied in
  the emulator, and the P4 line counts copies and flips.
- Host tests 37 (the copy, a late wake, a large change, the flip
  fallback); 4 new mutants caught.
