# Sheet: the textured frame +15% into screen 1 (test), 2026-10-29

**What it decides.** GC2DE: the textured game frame renders 15% slower
into screen 1 (0x296000) than into screen 0 (14,972 against 13,028 us);
the other three frames the same. A lead: the target's memory banks
against the texture's. With the copy fallback every fallback frame
renders into screen 1, so this costs ~2 ms on textured fallback frames.

## 1. Original way (Windows Mobile / Dell's product)
Back buffers wherever the driver's allocator puts them.

## 2. Dell's way (REA): found 2026-10-29 late (REA log #25-#31)
The earlier "not found" was a misread: GLES builds its escape codes
`mov #100352` then `orr` (0x18A88, 0x18A8C), and the decompiled C drops
call arguments, so the search for 0x18800 could never hit. GLES's back-
buffer allocation (`libGLES_CL` 0x2270a50) sends escape **0x18E73**;
`ddi_2700g`'s DrvEscape switch (0x2101cb0) calls 0x2113b80, which takes a
block from 0x2114d30: **PowerVR services' resource allocator** (an arena
over 2700G memory, the size rounded to its quantum) whose allocation
(0x2116b28) is **first fit by size class**: 32 free lists by power-of-two
size, walked from the request's class upward, the first segment that fits
with the requested alignment taken. **Dell places a back buffer wherever
the free list gives: no rule relative to the screen, the textures or the
memory's banks.** So Dell's way is the textbook allocator, not a design
for this cost; it offers nothing to copy here.

## 3. Modern way (for when it resumes)
Place render targets and textures in different DRAM banks, or tile
surfaces so a target's and a texture's accesses interleave across banks.
Training knowledge, a claim.

## 4. The alternatives, and why ours wins
| | What | Wins | Loses |
|---|---|---|---|
| A. Do nothing | Accept ~2 ms on textured fallback frames | No work | ~15% on every textured frame by the copy |
| B. Dell's | Screen 1 wherever a first-fit allocator puts it | Nothing to build | Placement by chance: the cost could be worse or better by luck, never by measurement |
| C. Modern | Targets and textures in different DRAM banks (or tiled to interleave) | Fixes the cause if it is banks | Needs the 2700G's bank mapping, which no document gives (to measure) |
| **Ours** | A sweep: the textured frame into screen 1 at offsets of 2 KiB to 1 MiB from 0x296000 inside free 2700G memory (and the texture moved instead, the control), each render timed and checked exact; screen 1 placed (`rp_vram_map`) where the sweep says, if any offset wins | Measures the cause on this chip, fixes it in the memory map at no run-time cost; the bank size falls out of the sweep's period | One device run (about 5 minutes); a map change if it wins |

## 5. What would overturn
- A sweep with no address faster → not banks (another lead: scan-out
  reading screen 0 while the core writes screen 1).

**Asked of the owner:** the go to build the sweep (an `rp-gpubench`
stage, emulator-checked first); a separate go for the device run.

## Built (2026-10-29, the owner's "Build R114")
`rp-gpubench` run R114 (`RP_GPUBENCH_R114`, with `RP_GPUBENCH_GC2_HALF`):
at 149.50 / 2, the four game frames rendered into 0x296000 + each of 0,
2 KiB, 4, 8, 16, 32, 64, 128, 256, 512 KiB and 1 MiB, 8 renders each, the
core's time kept and the first checked word for word against the frame at
0x296000; then the control, the target kept and the textures (copied to
0x600000 + the same offsets) moved instead. Places checked clear of the
pool and each other before anything runs. Emulator-checked ("run R114":
every render right, everything given back); the device run waits for the
owner's run it.
