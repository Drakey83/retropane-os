# Sheet: the Weather app's frames at 30-60 fps (step 0), 2026-10-30

**What it decides.** How the 2D renderer (`drivers/mbx-render`, ADR-0023)
draws a moving page (the Weather entrance, a list dragged 1:1 with the
pen) fast enough for 30-60 frames a second on the X51v.

**Measured (local/rp-today-weather-run1.txt, the first device run):**

| Frame | CPU (queueing) | Engine | Note |
|---|---|---|---|
| Weather entrance, 37 of them | 53-58 ms | ~29 ms | log: `background true` on every one |
| Weather, page still, a tap | 10.5-17 ms | | `background false` |
| Today screen, a tap | < 10 ms | | |

Frames reached the screen every 85-100 ms (about 10 fps).

**Why (read from the code, `Mbx2dRenderer::draw`):** glass (fills,
gradients, outlines that cover nothing earlier) is "hoisted" into a cached
background picture, rebuilt when a hash of all of it changes. The entrance
slides glass rows sideways, so the hash changes every frame and the
renderer (1) rebuilds the **whole screen's** background (wallpaper copy
plus every panel, each gradient row rewritten by the CPU through the slow
2700G window) and (2) sets `fresh = 2`, drawing the **next two frames
whole** as well. A dragged list would do the same on every move.

## 1. Original way (Windows Mobile)
No moving pages: GDI redraws the rectangle a program invalidates, straight
into the shown screen; list scrolling is `ScrollWindowEx` (a screen copy)
plus repainting the strip uncovered, on opaque list backgrounds.

## 2. Dell's way (REA)
Searched `local/re/ghidra-c/imgfs-a08_ddi.dll` (308 functions) for
`overlap`, `0xcc0020` (SRCCOPY), `AnyBlt`, `ScreenToScreen` (search log
#36): SRCCOPY blits in `0x2362918`, `0x2364334`, `0x2369c74`,
`0x236a378`, `0x236a990` (GDI's blit paths); **no retained layer, glass
cache or scroll cache**. GDI draws what is invalidated, into the shown
screen (REA #15, #19). A finding, not an answer.

## 3. Modern way
Android's RenderThread and iOS's Core Animation keep each view's drawing
(a display list or a backing store) and redraw only what changed; a
scroll re-composites retained content at a new offset (training
knowledge, a claim). ADR-0034 C4 gives us the same on this hardware: a
panned page on the video plane, a free repoint at 60 Hz.

## 4. The alternatives, and why ours wins
| | What | Wins | Loses |
|---|---|---|---|
| A. Keep today's | As built | Nothing to do | ~10 fps moving; a dragged list as slow |
| B. WM's way | No movement; lists jump a row at a time | Cheap | Exactly what the owner rejected ("doesn't feel polished or modern") |
| C. Never hoist moving glass | The shell marks glass that moves; it is drawn per frame, not cached | No rebuilds | Needs the shell to tell the renderer what moves: a new contract; every moving gradient row still rewritten by the CPU each frame |
| D. A panned page on the video plane (ADR-0034 C4) | The list drawn once, tall, wallpaper in it; dragging repoints the plane | Free 60 Hz scrolling | Needs the overlay path (R121, built, knob off) in the shell image, a tall page per list (~1 MiB each), the wallpaper moving with the list; much larger build, unmeasured in the shell |
| **Ours (E): redraw only what changed, everywhere** | (1) The background is rebuilt **only where its glass changed**: each hoisted command keyed (5 words), the rebuild clipped to the old and new places of the commands that differ, and no whole-screen frames after it. (2) **Gradient rows cached** in the 2700G row ring by their exact colours and span, so a panel's row is written once and reused by every row and frame. (3) A **clip command** in `rp-draw` so a list moves by the pixel inside its viewport | A sliding row costs its own rectangle, not the screen; the CPU stops rewriting identical rows; no new hardware path, same pixels (the model's tests check them against the CPU renderer) | Engine time still scales with the moving area (a dragged list redraws its ~480 x 460 viewport: predicted ~15-20 ms, so 30-50 fps, not a sure 60); D stays the way to 60 if measured short |

**Expected:** entrance and drag frames ~5-8 ms CPU (from 53-58) and
~15-20 ms engine, frames every ~17-33 ms: 30-60 fps. A prediction to
measure, not a promise.

## 5. What overturns each, and the test
- **E short of 30 fps on the device** (frame period > 33 ms during a
  drag): D next, with its own sheet.
- **E's rows wrong** (a cached row reused for a different gradient): the
  model's pixel tests against the CPU renderer fail; a device photo shows
  a wrong tint.
- **C better than E:** only if the rebuild's region copy costs more than a
  frame's drawing; the run log's `background` and per-frame engine times
  tell.
- **The test:** `rp-today` with the replay off, the owner opening Weather,
  switching pages and dragging Hourly; the run log's per-frame CPU and
  engine times and frame period (new: one summary line per movement, not
  per frame, so the 64 KiB log doesn't fill). Host: FrameStats (words,
  blits, pixels) before and after on the model, in tests.

## The owner's word on cost (2026-10-30, while this sheet was written)

"if those types of animations are too expensive then we dont have to use
them". Read with the numbers: the 53 ms was the renderer's whole-screen
rebuild (a bug in what it caches), not the cost of movement itself. After
E, the priorities are the ones he named on the device: the list
following the pen, the fling and the rebound. The entrance (rows sliding
in) stays only if it measures at 30 fps or better; otherwise it goes and
pages simply appear (his rule above). Nothing here is a ruling until he
says so.

## Room in the image (needed by steps 3-6, not part of this sheet's go)

`rp-today` loads 892 KB of its 921 KB budget (the loader's area is 928
KiB): 159 KB code, 734 KB data, of which the wallpaper 460 KB (raw
YUV 4:2:0) and the weather pictures 225 KB. Settings, the keyboard and
the radar's base map need ~50-70 KB. Proposed (its own sheet next): the
wallpaper and the radar map packed losslessly on the PC (a per-plane
predictor and deflate, as PNG does) and unpacked once at start into the
RAM pool, as R129 did for the pictures: a small decoder (~5 KB of code)
for ~200 KB back, and the same pixels the owner approved.

## 6. The plan
1. `rp-draw`: `Cmd::Clip { r }` (later commands drawn only inside `r`;
   bounds clipped); the CPU renderer (`rp-gfx`) and `rp-mbx-render`
   follow it (ADR-0021, ADR-0022, ADR-0023).
2. `rp-mbx-render`: per-command keys for hoisted glass, the rebuild
   clipped to their changes, no `fresh = 2` for it; the row ring kept
   across frames with exact keys.
3. Host tests: a slid row rebuilds only its old and new places; a frame
   after it is not whole; a cached row is reused and never for another
   gradient; clip; the model's pixels equal the CPU renderer's; mutants.
4. `rp-today`: a movement summary line (frames, median and worst CPU and
   engine, period) instead of a line per slow frame.
5. Device run on the owner's "run it".
