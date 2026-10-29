# Sheet: the graphics service on the X51v (R110, test), 2026-10-29

**What it decides.** Whether the graphics service itself (not a bench
harness) drives a 3D scene tear-free on the device: the scan race, the
copy fallback, the bottom-row start, with its counters (`starts()`,
`copy_stalls()`, `last_copy_line`) logged. No image drives a 3D scene
through `rp-graphics` today (`rp-today` is 2D only), so it needs one.

## 1. Original way (Windows Mobile / Dell's product)
No such test in Dell's code: no `DSIG` read anywhere (REA log #13, #14,
#21). Windows Mobile was tested by use.

## 2. Dell's way (REA)
Searched local/re for a self-test of the display or 3D path:
`0x2184|,0x184[,)]` (the signature) in `ddi_2700g`, `libGLES_CL`,
`d3d_2700g`: no Dell function found. The only frame-timing reads are
`DVLNUM` in GetScanLine 0x02125b64, the plane-off wait 0x0211fe70 and the
mode switch 0x021273d4 (REA log #7).

## 3. Modern way
An on-device integration test of the compositor with per-refresh scan-out
CRCs and the driver's own counters (DRM/IGT kms tests). Training
knowledge, a claim.

## 4. The alternatives, and why ours wins
| | What | Wins | Loses |
|---|---|---|---|
| A. Do nothing | Trust the proof run (rp-gpubench) for the policy | No new image | The service's own code (its jobs, the 2D/3D hand-offs, the held 2D frame) never runs on the device: "proven on hardware" unmet |
| B. Fold it into the proof run | rp-gpubench calls `rp-graphics` directly | One image | gpubench's harness and the service's frame loop mixed; a fault is harder to place |
| **C. Ours** | A small image (`rp-scene`): a 3D scene through `rp-graphics` with 2D changes beside it, at 149.50 / 2, the four game frames' meshes, every refresh's `DSIG` against references, the counters logged on the card (rp-cardlog) | The shipped code proven as shipped | A new image to build and keep |

## 5. What would overturn each, and the test
- **C overturned** if the proof run already fails the copy (C93): this run
  waits for it.
- **A's risk shown** if C finds a tear or stall the proof run didn't.
- **The test:** 600 frames, each policy's share counted (`behind`,
  `after_area`, `screen1`), torn refreshes by `DSIG`, `copy_stalls`, R96's
  failed/hung/undrawn counts.

**Asked of the owner:** the go to build the image after the proof run's
result; a separate go for the device run.

## Built (2026-10-29, the owner's go: "R110, R113, R114, in this order")
`rp-scene` (`shell/scene`): the service opens a 480 x 448 scene under a
label, four scenes through its quads (flat and Gouraud; the 8 x 8 texture
magnified bilinear; three translucent quads over a Gouraud field; three
depth-tested quads) and a 2D bar whose colour follows the frame. Three
phases: Windows Mobile's clock (rows), then 149.50 / 2 (`clock::step`,
`Graphics::clock_changed`), then the checkerboard (`Graphics::set_tile_order`:
no frame the scan can lead, every one by the copy). Each phase measures
12 references settled (the four whole pictures, and the two hand-off
mixes per picture: 2D's change shown before the scene renders, and the
reverse, which the service's comments say R20 doesn't cover yet), then 150
frames back to back and 50 one-off (asked at lines 0, 213, 426), every
refresh judged (`rp_drv_mbx_display::sig::Watch`): torn split into the two
mixes and other, back, unread; `starts()`, `copy_times()`, `copy_stalls()`,
failed, recovered, the 2D engine's stalls, the one-off frames' time to their
first whole refresh and those never shown whole. 600 frames.

Fixed the class on the way (no copies): the clock rung step moved into
`rp_drv_mbx_display::clock` (`Rung`, `step`, `begin`, `finish`,
`put_back`, `CS3_WM`, `CS3_LONG`), used by `rp-gpubench`, `rp-glesfeat`,
`rp-mbx3d`'s put-back (which skipped the PLL rewrite when the registers
read as Windows Mobile's: run G1's lesson broken) and `rp-scene`; the
refresh judge, the watch and the settled signature read moved from
`rp-gpubench` into `rp_drv_mbx_display::sig`.

**Expect** (audit C94, the owner's rulings): held 0 after the start, torn
other than the hand-off mixes 0, copy stalls 0. The mixes are counted, not
expected 0: they measure the 2D/3D hand-off the service doesn't yet keep
whole. The device run waits for the owner's "run it".

## Ran on the X51v, 2026-10-29 (card log scene-0001, `local/rp-scene-run1.txt`)
The service's side, proven: 600 frames, **held 0 in every phase** (audit
C94's start-up copy timing works on the device), copy stalls 0, failed 0,
recovered 0, the 2D engine's stalls 0, no underrun, the clock to 149.50 / 2
(lock 10 us) and back clean, stack 8,920 of 16,384 bytes.

| Phase | Behind the scan | By the copy | Back to back | Copy time a whole screen |
|---|---|---|---|---|
| Windows Mobile's clock, rows | 113 + 37 | 37 + 13 | 12.1 frames a second | 3.2-9.5 ms |
| 149.50 / 2, rows | 150 + 50 | 0 | 18.6 frames a second | 2.2-5.5 ms |
| 149.50 / 2, checkerboard | 0 | 150 + 50 | 18.6 frames a second | 2.5-6.3 ms |

**The tear judging did not work (the test's fault, not a result):** the
references were steady and distinct per scene, but almost every refresh in
the run read a signature unlike all of them (750 of 753 back to back, 367 of
367 one-off, also while the picture stood still), so the "torn" counts are
not tears. The cause, found in the emulator the same night: the references never had
the bar. The step that gets a held scene drawn sent an empty 2D frame, and
the 2D renderer takes each frame as the whole 2D scene, so its catch-up
painted the bar away (its pixels read as the wallpaper's grey). So the
first reading here, that the bar's rows don't change `DSIG`, was wrong:
withdrawn. Fixed: that step's frames carry the bar.

**Found while fixing it (emulator, rp-next-032; to be confirmed on the
X51v):** when the scene renders into screen 0, the whole of row 544, the
first row under the scene's area, is overwritten with 240 32-bit colour
words (`0xff808080`, `0xff301010`: 960 bytes, a write outside the scene's
area). Run 2 logs it from the device (the screens compared band by band,
the first and last differing words).
Next (not run): log the signatures the run reads, and compare screen 0 and
screen 1 word for word after a frame, to find whether the shown picture
differs from the reference's (a stale part of one screen) or the reading does.

## Run 2 on the X51v, 2026-10-29 (card log scene-0002, `local/rp-scene-run2.txt`)
The judging works now: every compared one-off frame showed exactly its
reference (24 of 24, band by band, both screens equal), each read its
reference's `DSIG`, and the four whole pictures are distinct. **Row 544's
stray write is the emulator's only** (the X51v's screens never differed:
`docs/hw/emulator.md`). The service's side as in run 1: held 0, copy stalls
0, failed 0, recovered 0, the clock step clean, stack 9,792 of 16,384.

| Phase | Frames | Behind / by the copy | Refreshes judged | Torn | Of them 2D-first mixes | Never shown whole | Ask to first whole refresh |
|---|---|---|---|---|---|---|---|
| WM clock, rows, back to back | 150 | 113 / 37 | 753 | 226 | 37 | | |
| WM clock, rows, one-off | 50 | 37 / 13 | 353 | 50 | 0 | 3 | median 77.4 ms, worst 85.3 |
| 149.50 / 2, rows, back to back | 150 | 150 / 0 | 491 | 151 | 0 | | |
| 149.50 / 2, rows, one-off | 50 | 50 / 0 | 281 | 55 | 1 | 0 | median 51.1 ms, worst 61.8 |
| 149.50 / 2, checkerboard, back to back | 150 | 0 / 150 | 491 | **2** | 0 | | |
| 149.50 / 2, checkerboard, one-off | 50 | 0 / 50 | 292 | **0** | 0 | 0 | median 56.3 ms, worst 61.8 |

**What it means.** Frames that go by the copy (render into screen 1, copy
into screen 0 behind the scan) are tear-free through the service: 2 torn
refreshes in 783, as GC2DF found in the bench. **Frames started behind the
scan on screen 0 tear about one refresh each** (151 in 150 frames back to
back, 55 in 50 one-off at 149.50 / 2; 50 in 50 one-off at Windows Mobile's
clock), and nearly all of those refreshes are unlike any whole picture or
hand-off mix (each torn one a different signature), so they are real tears,
not the 2D-first hand-off. GC2CZ in the bench had 120 of 120 such frames
tear-free, so the difference is in the service's path (its start line, its
window, or 2D's change copied into screen 0 while the scan reads it). At
Windows Mobile's clock, 37 back-to-back frames went by the copy and showed
exactly 37 2D-first mixes: there the bar reached the screen a refresh
before the scene. Open, for the owner: where the behind-the-scan tear
comes from (the next step is logging each frame's start line, window and
render time against the refresh it tore), and the hand-off at that clock.

## Run 3 on the X51v, 2026-10-29 (card log scene-0003, `local/rp-scene-run3.txt`): the tear found
- **Not the schedule:** each render's real time (the 3D core's done bit,
  watched by the image) is within 1-2% of the time the service plans with
  (WM clock 8,765 against 8,769 us; 149.50 / 2 5,838 against 5,852).
- **3D alone (only the scene changes): 0 torn in 80 frames** at both
  clocks: the start behind the scan is tear-free in the service too.
- **2D alone (only the bar changes): every frame tears** (80 torn refreshes
  in 40 frames at Windows Mobile's clock, 40 in 40 at 149.50 / 2), and the
  scene re-renders fully every frame though unchanged.
- **Why** (the host model): a fill whose colour changes rebuilds
  `rp-mbx-render`'s cached background, so 2D's drawn rectangle is the whole
  screen; the service copies it into screen 0, shown, at once, laying 2D's
  layer (wallpaper) over the scene's area mid-scan until the re-render
  lands: one torn refresh, two when the render starts in the next refresh.
  The fix's alternatives: `r110-2d-into-the-shown-screen.md` (waits for the
  owner's go).
