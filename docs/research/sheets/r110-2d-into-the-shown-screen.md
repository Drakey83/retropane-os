# Sheet: 2D's changes copied into the shown screen over the scene (fix), 2026-10-29

**What it decides.** How 2D's changes reach screen 0 (shown) in a frame
with 3D work. Today (`services/graphics`, `Job::TwoD`): 2D draws into screen
1, the service copies 2D's drawn rectangles into screen 0 at once, whatever
the scan line, then starts the scene behind the scan. R110 run 3 on the X51v
(scene-0003) measured the cost: with only 2D changing, **every frame tore**
(1 refresh at 149.50 / 2, 2 at Windows Mobile's clock; 120 of 80 frames),
and the scene re-rendered fully each time (5.8-8.8 ms) though it hadn't
changed; with only the scene changing, **0 torn in 80 frames** (the scan
race itself is right: each render's real time, seen by its done bit, within
1-2% of the time the service planned with). The host model shows why: a
fill's colour change rebuilds `rp-mbx-render`'s background, so 2D's drawn
rectangle is the whole screen; copied into screen 0 it lays 2D's layer
(wallpaper) over the scene's area mid-scan until the re-render lands. The
service's own comment says so: "R20 covers neither yet". Any shell change
under or across a 3D scene (a panel, the wallpaper, a whole-screen redraw)
does it.

## 1. Original way (Windows Mobile / Dell's product)
GDI draws straight into the shown primary; a windowed GLES surface with no
back buffer renders into the shown screen too (REA #15, #19): both visible
as they land, no ordering between them.

## 2. Dell's way (REA)
No composition of 2D and 3D: `ddi_2700g` command type 5 (a windowed
present) only counts a sync object (REA #19); nothing keeps 2D off a 3D
window's area or times 2D's writes against the scan (no DSIG or scan-line
use in GLES or D3D, REA #13).

## 3. Modern way
A compositor builds each frame off screen from layers and presents it whole
at vblank (Wayland, DRM atomic); a layer's damage outside another's opaque
area is all that is recomposed. Training knowledge, a claim.

## 4. The alternatives, and why ours wins
| | What | Wins | Loses |
|---|---|---|---|
| A. Do nothing | As today | No work | A torn refresh or two on every 2D change while a scene is open, and a wasted full re-render (~6-9 ms of the 3D core) |
| B. The test's bar as a non-fill | Change rp-scene only | Hides it in the test | The shell still tears; the class stays |
| C. Modern: compose off screen, present whole | Every frame built in screen 1 and flipped | Never tears | Screen 1 is outside the safe region for 3D (C71 R-B); a whole-screen copy or flip per frame (2-6 ms, R113's flip timing unproven) |
| **Ours** | In a frame with 3D work, 2D's changes reach screen 0 **only outside the scene's area** (the service's `outside()`, as its held path already does) and **by a copy planned behind the scan** (`scan::copy_window`, the copy fallback's own, 0 torn in 783 refreshes on the X51v); the scene is redrawn only where its own content changed, or 2D changed under a non-opaque scene; screen 1's area made current by the area copy it already gets | Fixes the class in the service (any 2D change, any scene); no 2D pixel ever lands in the shown area; saves the wasted re-render; reuses proven parts | The copy waits for its window (up to a refresh), so a 2D change beside a scene shows up to ~17 ms later; the bookkeeping for "2D drew under the area" |

## 5. What would overturn each, and the test
- **The test:** R110 run 4 (rp-scene as run 3): the 2D-alone runs torn 0
  and no re-render (start `-`), the 3D-alone runs still 0, back to back
  torn 0 or only refreshes the copy's window can't avoid; host tests (2D
  changes over and beside the area, opaque and translucent scenes, the
  held path, stalls) with mutants.
- **Ours overturned** if 2D's change outside the area still tears with the
  copy behind the scan (then the flip latch, R113, is the way).
- **A's risk** is measured already (run 3).

**Asked of the owner:** the go to build "Ours"; a separate go for run 4.

## Built (2026-10-29, the owner's "Yes, build it")
`services/graphics` `Job::TwoD` and the new `Job::Copy2d`: with a scene
open, 2D's changes in screen 1 reach screen 0 (shown) only by a copy
planned behind the scan (`scan::copy_window`, as the fallback's copy);
where 2D drew under the area, screen 1 first takes the scene's area from
screen 0 (not shown), so the copy changes nothing there (simpler than
copying around the area: the pieces around a whole-screen change made the
copy too long for any window). 2D under an opaque scene no longer makes it
draw again; under a translucent one it does, and a frame by the copy
fallback redraws the tiles 2D drew under in screen 1. 2D into screen 0
while screen 1 is shown (only after a stalled copy) puts the area back
from screen 1 before screen 0 is shown. With no copy timed yet the change
goes in at once, counted (`Starts::two_d_now`; `two_d_behind` the others).
Host tests 31 (four new: beside an opaque scene, under it, under a
translucent one, after a stall), 5 of 5 mutants caught. Emulator (`emu-scene`):
2D alone draws the scene 0 times (run 3 on the X51v: every frame), every 2D
change behind the scan.

## Run 4 on the X51v, 2026-10-29 (card log scene-0004, `local/rp-scene-run4.txt`)
**No real tear left anywhere** (torn refreshes other than a 2D/3D hand-off
mix: 0 in 2,500+ judged): 2D alone 0 torn (run 3: 80 in 40 frames and 40
in 40) and the scene never drawn again; 3D alone 0; the checkerboard (every
frame by the copy) 0 back to back and one-off (run 3: 2). Every 2D change
behind the scan, none at once; held 0, copy stalls 0, failed 0, recovered
0; stack 11,480 of 16,384.

**What's left, for the owner:** frames where both 2D and the scene change
in rows order show **one mixed refresh each** (the new bar with the last
scene: 150 in 150 back to back at 149.50 / 2, 113 at Windows Mobile's
clock), because 2D's copy and the scene's render now land in two windows a
refresh apart. Whole pictures, never split, but a refresh where 2D and 3D
disagree in time. The two windows also cost speed: 15 frames a second back
to back at 149.50 / 2 (run 3: 18), and at Windows Mobile's clock 26 of 50
one-off frames weren't shown whole within six refreshes (run 3: 3). The
checkerboard phase, where 2D's change and the scene go together in the
copy fallback's one copy, had 0 mixed and 18 frames a second.
