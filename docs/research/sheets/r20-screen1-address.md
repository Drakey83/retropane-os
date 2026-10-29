# Sheet: the textured frame +15% into screen 1 (test), 2026-10-29

**What it decides.** GC2DE: the textured game frame renders 15% slower
into screen 1 (0x296000) than into screen 0 (14,972 against 13,028 us);
the other three frames the same. A lead: the target's memory banks
against the texture's. With the copy fallback every fallback frame
renders into screen 1, so this costs ~2 ms on textured fallback frames.

## 1. Original way (Windows Mobile / Dell's product)
Back buffers wherever the driver's allocator puts them.

## 2. Dell's way (REA): NOT FOUND, so this sheet stops here
GLES allocates its back buffers through `ExtEscape` code 0x18800
(100352) to the display driver (`libGLES_CL` 0x02271460, called by
0x02270a50; the escape also in 0x0226ca40, 0x0226cb04). I searched
`ddi_2700g` for the handler with `0x188|0x18800|100352` and
`0x187[0-9a-f][0-9a-f]\b|0x188[0-9a-f][0-9a-f]\b|0x189[0-9a-f][0-9a-f]\b`:
no handler found (REA log #18, #20). **Where Dell places a back buffer
relative to the screen and textures is not found.** Per the rule, I stop
on this item.

## 3. Modern way (for when it resumes)
Place render targets and textures in different DRAM banks, or tile
surfaces so a target's and a texture's accesses interleave across banks.
Training knowledge, a claim.

## 4. Alternatives (outline only, not proposed for building yet)
| | What |
|---|---|
| A. Do nothing | Accept ~2 ms on textured fallback frames |
| B. Dell's | Not found (above) |
| C. Modern | A screen-1 address chosen so its banks differ from the texture region's |
| Ours (proposed later) | A sweep: the textured frame into screen 1 at offsets of 2 KiB to 1 MiB from 0x296000 inside free 2700G memory, each timed and checked exact |

## 5. What would overturn
- A sweep with no address faster → not banks (another lead: scan-out
  reading screen 0 while the core writes screen 1).

**Asked of the owner:** either (a) more REA time to find Dell's escape
handler (the GPE escape table in `ddi_2700g`, by its vtable), or (b) your
go to proceed without Dell's way on this one, or (c) park it (a reminder).
