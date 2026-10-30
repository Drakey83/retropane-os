# Sheet: the Weather app's drag and entrance past 30 fps (step 0, round 2), 2026-10-30

**What it decides.** The next optimization after round 1
(`weather-frame-speed.md`), from the second device run
(`local/rp-today-weather-run2.txt`, the owner driving).

**Measured (run 2, one line per movement):**

| Movement | fps (median period) | CPU median | queue to flip | background rebuilt a frame |
|---|---|---|---|---|
| Page entrances (Stagger) | ~20 (48-50 ms) | 10-24 ms | 3-29 ms | ~60-70k px |
| Hourly dragged or flung | ~13.5-15 (66-74 ms) | 37-40 ms | 21-26 ms | ~190k px (the whole page) |
| Radar loop frames | ~6-15 | ~40 ms | ~22-29 ms | 200-300k px |
| Run 1, any Weather frame | ~10 (85-100 ms) | 53-58 ms | 29 ms | 307k px (the screen) |

Round 1 halved entrance frames; a drag is still under 30 fps. Start-up
costs (new): wallpaper unpacked 107 ms, radar map 155 ms, rain cells 42
ms; the map under the page 38 ms, the wallpaper back 161 ms. The run log
filled at 64 KiB again (1,150 touch-recorder lines), so the end summary
was lost.

**Why (from the code and these numbers):** dragging moves every row's
glass, and moved glass is still *hoisted*: each frame the background is
rebuilt over the whole page (wallpaper copy plus every row's gradient),
then copied again into the work picture before the text is drawn. The
page is composed twice a frame. Where the remaining CPU goes (glyph
blits, waiting on the engine's FIFO, the per-command key loops) is not
measured: the profile build has never run on a Weather drag.

## 1. Original way (Windows Mobile)
Lists repaint the strip a scroll uncovers after `ScrollWindowEx` copies the
rest, on opaque backgrounds; no movement under glass.

## 2. Dell's way (REA)
Search #36 holds: GDI's SRCCOPY blits, no glass, no retained layers.
Nothing new to search for this question.

## 3. Modern way
Retained layers composited at an offset (ADR-0034's option D: a panned
page on the video plane, a free repoint at 60 Hz).

## 4. The alternatives
| | What | Wins | Loses |
|---|---|---|---|
| A. Keep round 1 | | Nothing to build | Drag at ~14 fps |
| B. Drop the movement | Pages appear, lists jump | | The owner's rule: only if it can't be made faster |
| C. Option D now (panned page on the video plane) | The list drawn once tall, dragging repoints the plane | 60 fps drags | The overlay path in the shell image, ~1 MiB a page, the wallpaper moving with the list; a large build |
| **Ours (E2): measure, then compose moving glass once** | (1) Glass that is new or moved **and** inside this frame's changed area is drawn straight into the frame, not hoisted, so a drag stops rebuilding the background: the page is composed once a frame (glass already in the background stays cached). (2) A **profile run** (`RP_TODAY_PROFILE` zones around the renderer's key loops, its glyph blits and its FIFO waits) on a scripted Weather drag, so the next cut is measured, not guessed. (3) The touch recorder off in this image (the log keeps the movement lines) | Predicted: a drag's background work goes from ~190k px rebuilt plus a copy to nothing; ~10-15 ms of engine and CPU saved a frame, ~25 fps; the profile names what is left | Still not a sure 30: C stays next if E2 measures short |

## 5. What overturns it, and the test
- **E2's drag still under 25 fps:** the profile decides between cutting
  the CPU's part (keys, glyphs) and going to C.
- **Pixels wrong** (glass drawn twice where it is both hoisted and
  direct): the model's tests against the CPU renderer fail first.
- **The test:** host tests (a dragged list rebuilds no background; a
  still list is hoisted again; pixels equal the CPU renderer's), then one
  short device run: the owner drags Hourly and flicks it, about a minute.
