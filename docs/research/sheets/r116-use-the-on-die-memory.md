# Sheet: how to spend the on-die memory beside screen 0 (R116, design), 2026-10-29

**What it decides.** R23T on the X51v (gpubench-0224) measured the first
704 KiB (0x0-0xB0000) as faster memory: the textured game frame renders in
13.04 ms with its target there, 15.29 ms in SDRAM (3.7 us a KiB past
0xB0000); R114 against R23T: the 3D lists on die save another ~0.31 ms a
render. Screen 0 (600 KiB at 0x0) is already there, so frames rendered
behind the scan get it; the copy fallback's frames (into screen 1, SDRAM:
every frame where 2D and the scene both change, by the owner's ruling;
every checkerboard frame; spikes) pay the full cost. What's left on die
beside screen 0 is **104 KiB (0x96000-0xB0000)**, today the video band
(96 KiB, off while the 3D core draws: ADR-0034 amendment P) and 8 KiB
past the safe region's proven end (0xAE000). The question is what to put
there while the 3D core draws. A finding on the way: the safe region (where
the display may read while the 3D core draws: G19 right to 0xAE000, wrong
from 0xB6000) and the on-die memory end in the same place, so they are
likely one fact: the display reading SDRAM while the core writes it is the
clash; on die it isn't.

## 1. Original way (Windows Mobile / Dell's product)
Windows Mobile's primary at 0x0 (G17), the rest of the on die for its
other planes; GLES's buffers and lists wherever the allocator gives.

## 2. Dell's way (REA)
One allocator arena, first fit, no on-die distinction (REA #30, #31): the
3D lists and back buffers land on die only by luck of the free list.

## 3. Modern way
Put the hottest render data in the fastest memory: on-chip tile memory for
the frame being shaded (the MBX already does that per tile), embedded
SRAM for whole render targets or the binner's lists where it exists (the
Xbox One's ESRAM, mobile SoCs' system caches). Training knowledge, a claim.

## 4. The alternatives, and why ours wins
| | What goes in the 104 KiB while the 3D core draws | Wins | Loses |
|---|---|---|---|
| A. Do nothing | The video band (unused while 3D draws) | Nothing to build | Leaves ~0.3 ms every render and 2.2 ms every textured fallback frame on the table |
| B. Dell's | Whatever an allocator gives | — | The same as A, by chance |
| C. Lists on die (the shipped renderer's object lists and region arrays, as `rp-gpubench` keeps them: GC2BB) | Every render, behind the scan or by the copy: ~0.31 ms saved (R114 against R23T); GC2BF found lists on die part of divide 1's cure (0 of 300 wrong at 149.50 / 1 with the checker order) | Must fit (a 480 x 640 scene's lists in ~96 KiB: GC2BB did); the video band can't be shown while a scene is open (it isn't now either) |
| D. A small scene's target on die | A scene whose area fits (~100 KiB: 240 x 200 RGB565) renders there by the copy fallback, then is copied into screen 0 | 15% off textured fallback frames of small scenes | Only small scenes; the space C wants; a copy in place of the render into screen 1 |
| **Ours: C now, D only if measured worth it** | The lists on die for the shipped service; the 8 KiB to 0xB0000 left until R23's correctness half proves the edge for the display | Every frame faster, the divide-1 cure's half in place, no new copy; built from what `rp-gpubench` already proved | The lists' size bounded (a scene whose lists outgrow the space falls back to SDRAM lists, counted) |

## 5. What would overturn each, and the test
- **The test:** `rp-scene` with the service's renderer given lists on die
  against SDRAM lists, same frames: each frame's render time (R110 run 3's
  done-bit times), torn 0 kept, at both clocks; a scene whose lists don't
  fit on die falls back and is counted.
- **C overturned** if the service's frames gain under 0.1 ms (then the
  space goes to D or stays the video band's).
- **D's case** would need scenes small enough and fallback frames common
  enough to matter: measured by the shell's real scenes, later.

**Asked of the owner:** pick C, D, both, or neither; then the go to build
and a separate go for the run.
