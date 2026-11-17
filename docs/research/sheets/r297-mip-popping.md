# R297: Half-Life's textures popping, and the atlas (sheet, 2026-11-16)

ADR-0039 (ref_rp), docs/design/ref-rp.md choice 2. Reminders R297, R299.

## What the runs show

- **The cause, by code and count.** ref_rp lights each wall's surface at one
  mip level, chosen from the face's nearest corner (the software
  renderer's rule), and the chip samples the nearest texel only. Walking
  toward a wall moves it from level 1 to 0: the whole face swaps to a
  sharper copy in one frame (a pop). Each swap also takes a new place in
  the atlas while the old place stays taken until its whole page is
  dropped. That leak is what run 35's busy view showed: 8 pages dropped a
  frame, 12 faces not drawn.
- **Run 36 (`+rp_mip 0`, every face at level 0, port-runner-0036).**
  0 swaps by construction; the start view's lit surfaces are
  **5,488 KiB** (run 35 at mixed levels: 1,881 KiB), and they fit the
  7 MiB atlas (93% of the shelves; 3 pages dropped twice in 10 minutes);
  2 surfaces relit a frame (18 KiB, 1.0 ms); 20.2 frames a second (run 30:
  21.0). The owner did not play run 36, so the eye check of "no popping
  at mip 0" is still owed.

## The options (the owner decides)

| | A: mip chain in the atlas | B: texture and lightmap, two passes | C: second texture unit |
|---|---|---|---|
| What | each lit surface stored once at level 0 with levels 1-3 after it in its page (layout B, measured T12b), TEX0 bits 12-9 = 3, TEX1 `0xCE...` (trilinear, DoomGLES's word) | the map's own textures (their four levels are in the BSP) and the lightmaps (1/256 the area) each their own textures; the face drawn twice, the second pass multiplied | as B in one pass |
| Popping | gone (the chip picks per pixel, blends between levels) | gone | gone |
| Look | the software renderer's lighting, smoothed by trilinear | the GL look (smooth bilinear light) | the GL look |
| Memory | 5.5 MiB x 4/3 = **7.3 MiB: over the 7 MiB atlas**; fits only if the TA's 5 MiB parameter memory gives ~2 MiB (its use is not measured yet: one register read in the next run) | ~1-2 MiB for a map | as B |
| CPU | lights each surface at level 0 (4x a level-1 face), downsampling 1/3 more | no surface lighting at all; the world's words twice (feed +~4 ms until DMA, R292) | the world's words once, 9 not 7 a corner |
| 2700G | trilinear ~2 fetches a pixel (render 11.7 ms now, CPU 49 ms) | two layers on lit pixels (~+10 ms render, still under the CPU) | one layer |
| Risk | the chain past level 3 must not be read (filled anyway as a guard) | blend words new to ref_rp (emulator check) | hangs the 3D stage today (rp-glesfeat M) |

Iterate, don't imitate: `ref_soft` swaps whole faces (what we see now);
`ref_gl` does B or C with the GPU's mips; ours A keeps the software
renderer's look and gives the chip the choice per pixel, B spends the
2700G's headroom (the render is a quarter of the frame) to take the
lighting off the CPU. My lean: **B** (no surface cache CPU, no memory
pressure, frees CPU toward 45 a second), with A as the fallback if B's
second pass costs more than its CPU saves; C only after the second unit's
hang is solved.

What would overturn it: B measured slower than A on the X51v, or the owner
preferring the software renderer's lighting look.

## The pick (2026-11-16)

**B**, as recommended. The owner: "ill do whatever you recommend as long as
it doenst end with me giving up on 45 fps". Why B serves 45: the frame is
CPU-bound (runs 40-43: 30-45 ms of CPU against a 9-11 ms render), and B
moves the walls' lighting and uploads off the CPU (run 39-41: the atlas's
uploads and the CPU lighting 4-7% of the frame, more in busy views, and
the cause of the black walls and page-drop hitches), paying with GPU time
the chip has spare and ~1 ms more feed (the walls are ~15% of the words).
To build next (emulator first: blend and wrap words new to ref_rp).
