# Runs gpubench-0134 to -0137: the game path's route switch (R90; R101, R108, R102 at the game clock)

- **Date:** 2026-11-12, 9:42-9:52 PM
- **Device:** the owner's Dell Axim X51v, already on the loader, unattended
- **Card logs:** gpubench-0134 (R101), -0135 (R108), -0136 (R102 checked),
  -0137 (R102 speed) (`local/cardlog/`, not in git)
- **What changed since the October runs:** way C' is now the display
  driver's (`rp_drv_mbx_display::game_route::show`), and `rp-gpubench`'s
  way 3 calls it, so these runs prove the driver's switch; R108 and R102
  ran at the ruled game step 136.50 / 1 (ADR-0035 amendment D) in place of
  149.50 / 2 and 74.75 / 1.

## The route choice (built, host-tested)

`rp_graphics::route::Picker` (ADR-0038 G1-G4): a route per frame from the
game's own measured times. A step down after the mean of 8 frames passes
33.3 ms, to the sharpest lighter route **measured faster** than those
frames (or not measured yet); a step up after 60 frames in a row under the
budget, only to a sharper route whose own time fits; a game held to one
route never switches (G3); two frames in flight on the lines route only,
the smooth route's 2D placements apart from the render (G4); the smooth
route never chosen before the owner's colour check (R89, G1). Seven host
tests and a mutant. **Rule 0:** consoles step a fixed ladder of render
sizes by GPU time; on the X51v the lighter route isn't always faster (run
-0137 below: the smooth route slower than native on three of four
frames), so ours ranks by this game's measured times and never enters a
route its own numbers refute.

## R101: the driver's switch, every frame and every 8 (gpubench-0134)

| | |
|---|---|
| A switch every frame | 1,200 of 1,200 frames the route in place, whole by `DSIG`; 0 another route, 0 neither, 0 unread |
| A switch every 8 frames | 1,200 of 1,200 whole; 0 torn |
| Way C''s cost (native render, video plane reading the shown picture) | the core 19,512 against 19,461 us with the video off (+0.26%); 20 of 20 pictures the reference's |
| Each route's signature | native and lines as GC2DB's; the smooth route's is its own UYVY picture (the converter's model, R89) |

## R108: way C' at 136.50 / 1 while rendering (gpubench-0135)

| | |
|---|---|
| Clock | 136.50 / 1, the underrun flag clear after the change and after the change back |
| Pictures | the same hashes at 136.50 / 1 as at Windows Mobile's clock |
| A switch every pass, the next frame rendering across it | 2,399 of 2,399 judged frames whole, 0 the other route, 0 torn (600 frames passed unread during the renderer's set-up, as in October: no switch in them) |
| A switch every 8 passes | 1,349 of 1,349 whole, 0 torn (145 unread) |
| Renders | 3,853 measured, 0 hung |

## R102: route 3's two in flight against serial, same image, 136.50 / 1

**Every frame checked (gpubench-0136):** (d') 101 of 101 frames on each
of the four game frames equal to the serial (c') output word for word (0
differ; both carry the same words off the converter's model, R89's).

**Speed (gpubench-0137, the same image, fastest whole frames):**

| Frame | (c') serial | (d') two in flight | Gain | Native | (e) lines, two in flight |
|---|---|---|---|---|---|
| 0 flat | 7,628 us | 6,656 us | **12.7%** | 6,420 us | 2,308 us |
| 1 | 7,664 | 6,716 | **12.4%** | 6,673 | 2,452 |
| 2 textured | 9,112 | 8,228 | **9.7%** | 10,748 | 4,952 |
| 3 | 7,684 | 6,736 | **12.3%** | 6,762 | 2,464 |

Route 3's in-flight gain is measured against a serial run in the same
image at the game clock: 9.7-12.7% a frame (it was 6-8% at Windows
Mobile's clock and 74.75 / 1). Against native it is still slower on the
three untextured frames (0.96-1.00x) and faster on the textured one
(1.30x); the lines route with two in flight is 2.2-2.8x native. (d), the
2D beside the render, again left 12-23 words with U' or V' off: (d') only,
as G4 rules.

## What is not done

- **The graphics service's game frame entry point** (a game handing frames
  to the service, which then runs the picker, the switch on line 0 and the
  frames in flight) comes with the first game port, where ADR-0038 G2's 8
  and 60 frames are fitted. The choice, the switch and the in-flight ways
  are built and proven; no OS program yet renders a game.
- **The smooth route waits for the owner's colour check (R89).**
