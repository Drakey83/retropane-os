# Sheet: R184 step 2, the radar's timeline and rain loop (rule 0, before code)

Decided (docs/design/radar-maps.md M6, build order step 2; R177): the past
2 hours only, ending at now; the newest frame fetched first and shown
still, the rest in the background, the loop only once all are in; frames
kept between visits, only the newest fetched again; each frame composed
over the map once; the rain a little stronger on Paper and Ink than on
Night. This sheet is how, checked against rule 0 before any code.

## What a loop step costs now (measured, not guessed)

From the X51v run today-0053 (`local/rp-today-run-2026-11-04-r184.txt`,
lines 739-1023), every time the loop moves to the next frame:

| | Where | Cost |
|---|---|---|
| 1 | The CPU blends the rain over the map, pixel by pixel (`rp_today::show_radar`) | ~8 ms (a rain frame 34.3-36.1 ms, against the bare map 26.6-27.3 ms, lines 744 and 757) |
| 2 | The CPU converts all 222,000 pixels to the screen's 16 bits and writes them into the wallpaper in video memory (`wallpaper_row`, 443 KB) | **~27 ms** (the bare map's line: no rain, same cost) |
| 3 | The renderer, told the wallpaper changed, rebuilds its whole background and draws the whole screen afresh | **~34-41 ms** a step ("slow frame ... background true", e.g. lines 758, 764, 770) |

So a step is **~70 ms**, not 35 ms, and **the blend is the smallest
part**. The plan as first written ("compose each frame once into a picture
that is kept", R177) removes only part 1 (~8 ms): it would leave ~62 ms a
step. That is the finding this sheet turns on.

Also found: the relay keeps only RainViewer's **last eight** frames
(`xtask/src/usb.rs`, `Select-Object -Last 8`: 70 minutes, the timeline
says "past hour"); two hours is 13 frames. The frames are fetched oldest
first, and every frame is a full 32-bit copy in RAM (8 x 887 KB = 7.1 MB).
Kept-by-time on a refresh already works (P7, `take_radar_moves`).

## What others do (findings, not the answer)

- **Windows Mobile 5 / Dell:** no radar.
- **RainViewer's own web example and most web radars** (Leaflet,
  MapLibre): every frame is a layer of tiles over the map; all are loaded
  with their opacity at 0, and the loop sets one layer's opacity at a
  time. The GPU composites the map and the rain layer every screen
  refresh.
- **Phone apps** (Apple Weather, MyRadar): frames as GPU textures, blended
  over a vector map every refresh, with a progress line while frames load.
- What they share: **the frames sit where the display hardware reads
  them, and showing one costs no CPU.** That is the part worth beating on
  the X51v, whose CPU writes into video memory slowly (part 2).

## The ways for the X51v

| | Way | Step cost (estimate, to measure) | For | Against |
|---|---|---|---|---|
| A | **Compose once into RAM** (R177 as written): each frame map+rain kept as 32-bit pixels; a step copies it into the wallpaper | ~27 + ~36 = **~62 ms** | simplest | removes only the blend; 13 frames x 887 KB = 11.5 MB of RAM |
| B | **Compose once into video memory**: each frame map+rain written once, at 16 bits, into its own wallpaper picture in video memory (13 x 443 KB = 5.8 MB of the graphics pool's ~8.4 MB); a step only tells the renderer which picture is the wallpaper (an address) | CPU **~0 ms** + the renderer's rebuild ~36 ms | parts 1 and 2 gone; the same thing the web and phones do (the frames where the hardware reads them), done once instead of every refresh | part 3 stays; 5.8 MB of video memory while Radar is open (given back when it closes) |
| C | **B, and the renderer redraws only what changed**: when each frame is composed, the box where its rain differs from the frame before is kept (and the last frame's against the first, for the loop's wrap); a step tells the renderer that box only, not "the whole wallpaper changed" | CPU ~0 + the rebuild for the rain's box only: rain over a fifth of the view, **~7 ms**; rain everywhere, B's ~36 ms at worst | the cheapest step on this hardware by far, because rain moves over a small part of the map from one ten-minute frame to the next | one new renderer call (a wallpaper change limited to a box), with tests that a box-limited change draws the same screen as a whole one |
| D | **The 2D engine blends the rain every step**: the map as the wallpaper, each rain frame a see-through picture (16-bit, 4 bits of alpha) drawn over it | GPU blend of the view ~10-15 ms + rebuild | the map in one picture; frames smaller | rain colours and edges cut to 4 bits a channel (banding on RainViewer's smooth gradients); still rebuilds the screen each step. Rejected |

**Proposed: C** (B with the changed box). It beats the web's way here:
they composite the map and rain on every screen refresh; we composite
each frame once, when it arrives, and a step then costs only the part of
the screen where the rain moved. Target in the log: a step's CPU under
1 ms and its renderer frame under 15 ms in typical rain (from ~70 ms).

## How the rain is kept (for a look change, and between visits)

A look change (Paper/Ink/Night, or Auto at sunset) must compose every
frame again, so the rain itself is kept. Three ways:

| | Keep | RAM | A look change |
|---|---|---|---|
| 1 | each frame as 32-bit pixels (now) | 11.5 MB for 13 | re-blend only (~0.5 s for 13, est.) |
| 2 | each frame as one byte a pixel (RainViewer's colours are a small palette; R154) | 2.9 MB | re-blend only |
| 3 | **the PNG answers as they came** (RainViewer's tiles: 300-8,000 bytes each, 9 a frame) | **~0.1-0.4 MB** for 13 frames | decode again, then blend (~1 s for 13, est.; the frame shown first, the rest after, the loop pausing meanwhile) |

**Proposed: 3**, measured on the device: a look change is rare and the
newest frame is redone first, so the screen changes at once; 2 if the
13-frame redo is measured over ~1.5 s. RAM drops from 7.1 MB to under
half a megabyte, and the 32-bit frame buffers go.

## The rest of step 2

- **Two hours:** the relay sends RainViewer's whole past list (13 frames,
  every 10 minutes); the device's limit goes from 8 to 13; the timeline's
  right end reads "now", its left "2 h ago", and the frame shown says
  "latest" or "40 min ago" as today.
- **Newest first:** the map, then the newest frame (shown still, at once),
  then the others newest to oldest. While they come, **the timeline's
  ticks fill in as each frame lands** (the progress is the timeline
  itself, no separate spinner), and the play control stays dim; the loop
  starts by itself once all 13 are in, unless the person paused it.
- **Kept between visits:** leaving Radar and coming back shows the kept
  frames at once; after 10 minutes only the new frame is asked (P7, as
  now, newest first). The composed pictures in video memory are given
  back when Radar closes and made again from the kept PNGs on return
  (~0.5 s for 13, est., newest first, so the screen is never empty).
- **Opacity per look, computed, not picked by eye:** the rain's own alpha
  times a factor per look, fitted so the lightest rain step stands out
  from the land by the same colour difference (CIE ΔE 2000) on Paper and
  Ink as it does on Night; a test holds the fitted factors. Shown in
  ui-preview (`RP_PREVIEW_MAPS`) for the owner before the run.

## Build (if C and 3)

1. `rp-mbx-render`: the wallpaper's address settable, and a wallpaper
   change limited to a box (tests: a box change draws the same screen as
   a whole change; a box outside the screen is clipped).
2. `rp_weather`: the per-look rain factor and its fit test; the changed
   box between two frames (tests: no rain, rain at the edges, the loop's
   wrap).
3. `rp_shell` (`weather_net`): up to 13 frames; the newest asked first,
   then newest to oldest; the loop held until all are in; the timeline's
   ticks show which have landed; "2 h ago ... now".
4. `rp-today`: frames kept as PNG answers; each composed once into its
   picture from the graphics pool when it lands (and again on a look
   change, newest first); a step switches the wallpaper with the box;
   the log says per step: CPU us, the renderer's frame us and pixels.
5. The relay: the whole past list.
6. ui-preview, emu-today, `cargo xtask ci`, then the owner's run.

## What would overturn it

- The graphics pool can't spare 5.8 MB beside the plane path's own
  pictures (logged at the start): then B/C's pictures at 8 frames shown
  in the loop of the newest (the timeline still 2 h, the oldest five
  stepped through by hand), or way A with the 16-bit pictures in RAM
  copied by the DMA engine (the CPU free while it copies).
- The box-limited rebuild measured no faster than the whole one: then B.
- Decoding 13 frames' PNGs again on a look change over ~1.5 s: then 2.

## As built (2026-11-04, the owner's "Go, way C")

- **Way C**, with one change found while building: 13 pictures (one per
  frame) would have taken 13 of the graphics service's picture slots and
  grown it past its size bound, and allocating and freeing a picture per
  frame fragments the pool. Instead the radar takes up to three large
  RGB565 pictures when Radar opens (48 bands of 42 rows each, halved if the
  pool can't give that) and hands out **band slots** inside them; a frame
  takes a slot only for each band where it rains. With fewer slots than
  bands, frames not on screen give theirs way and are composed again when
  the loop reaches them (slower, never wrong; logged). The pictures go back
  to the pool when Radar closes.
- The renderer (`Mbx2dRenderer::wallpaper_bands`) reads each band of the
  wallpaper from its slot and makes the background and the screen again
  only in the box where the rain changed (tests: the screen equals a
  rewritten wallpaper's, the box only). The graphics service checks every
  slot is the radar's own RGB565 picture (`Graphics::wallpaper_bands`), and
  a picture a band reads can't be given back.
- **The shell** asks the newest frame first, shows it still, holds the loop
  until all 13 are in (the timeline's ticks fill in, "5 of 13 in", then
  "past 2 hours"); a loop step dirties only the timeline, the rain's box is
  the device's.
- **The rain's PNGs** kept in a 2 MB store (compacted when full), moved by
  record on a refresh; the 7.1 MB of 32-bit frames gone.
- **Opacity, as fitted** (`tools/maps/rain_opacity.py`, held by
  `the_rain_factors_are_fitted`): equal DeltaE to Night is out of reach on
  the light maps (RainViewer's light greens and yellows: even opaque rain
  on Paper reaches 26.7 against Night's 26.3 of 42.1), so the fit matches
  the **share** of what each map allows instead (0.625): Paper x1.35
  (345/256), Ink x1.44 (369/256), Night unchanged.
- Shortcut on the audit: C97 (a tile not kept or not readable shows as no
  rain, logged).

## First X51v run (today-0054, 2026-11-06): froze; what the log shows

`local/cardlog/today-0054.txt` (the serial log was lost to the watchdog
reset; the card log has the run):

1. **The relay refused frames 10-12** ("RPNET 404 0"): its TILE check took
   a one-digit frame number (`'^\d$'`), fine for 8 frames, not 13. The
   device took each refusal as a dry tile, so the three newest frames were
   composed empty (0 bands of rain) and the newest frame showed no rain.
2. **Composing a frame took 0.56 s** (9-10 of 11 bands raining), every
   frame alike, in one pass of the main loop (the empty frames 18 ms). The
   sheet estimated ~50-100 ms; decoding the nine PNGs again is most of it
   (the step-1 run decoded a tile in ~10-60 ms on arrival). Split to be
   logged.
3. **The pool gave 84 band slots of 143** (3.4 MB); today's rain covered
   10 of 11 bands in every frame, so the loop needed 130. The background
   composer then made frame 1 by evicting frame 2 and frame 2 by evicting
   frame 1, 83 times, 1.1 s each (the "1.1 s" lines are two attempts in
   one), starving the loop until the watchdog reset the X51v.

Fixes that need no choice: the relay takes two-digit frames; a refused
tile marks its frame missing (never shown dry) and asks it again once;
composing spread over passes (a tile decoded or a band written per pass,
no pass over ~60 ms); the background composer never evicts; the log splits
decode and compose time.

The choice (room for widespread rain: 13 x 10 bands = 5.4 MB, the pool
gave 3.4 MB beside the pages' pictures):

| | Way | For | Against |
|---|---|---|---|
| A | **Give Hourly's picture (2.3 MB) back while Radar is open**, made again on the way back (measured 96 + 110 ms; the swipe through 10 Day hides most of it) | all 13 frames fit in any rain; a step stays a pointer switch | Hourly redrawn after Radar (~0.2 s of CPU) |
| B | Composed frames kept in RAM (30 MB free), the shown one copied into the chip by DMA each step | no page touched; any number of frames | a ~15 ms DMA copy a step in heavy rain (CPU free meanwhile), new DMA code |
| C | Fewer frames in heavy rain (the newest that fit) | nothing else changes | the loop shorter than 2 hours exactly when rain is widespread |

Proposed: **A**.

### Rule 0 on the room choice (the owner, 2026-11-06: "is this using rule 0?")

The A/B/C table above was three workarounds weighed against each other,
not rule 0. The check:

- **What others do (findings):** phones and browsers keep radar frames
  as GPU textures and, when memory runs short, drop what isn't on screen
  (iOS discards off-screen layer contents, Android's trim-memory calls,
  browsers' off-screen tiles) and redraw it on return. Windows Mobile:
  no radar.
- **Can we need less, instead of finding more?** Two ways checked against
  the 2D engine's measured facts (docs/hw/2700g.md):
  - **D. Rain alone at 8 bits a pixel with its own palette** (RainViewer's
    colours with their alpha: 38 in the test tile, under 256), blended by
    the engine over the map already in the wallpaper: half the memory of
    16-bit, the map never copied per frame, colours exact. But the engine
    reads 8-bit sources at a twelfth of 16-bit's speed (a full screen
    63.3 ms against 5.0 ms, B9): a heavy-rain step would cost ~40-50 ms
    of engine against the ~28 ms measured now.
  - **Smaller cells than bands** (only where it rains): no help when rain
    covers the view, which is when room runs out.
- **Measured this run:** a loop step's frame in heavy rain 28.3 ms median
  (61 steps; 90.6 ms worst), against ~70 ms before step 2; the step's CPU
  24-32 us. Above the 15 ms target in rain this widespread.

**Proposed: A, done the modern OS way**: the pages' pictures not on
screen are given back while Radar needs the room (Hourly's first, the
largest) and made again when their page comes back, by their own client
(no service eviction of another client's pictures: ADR-0027's watch
item). It beats D on step speed (measured engine rates), B on CPU and
copies, C on data. D stays the fallback if a page can't give room.
Owed with it: the 28 ms heavy-rain step (redraw only the bands whose
pixels differ, not the box of both frames' rain).
