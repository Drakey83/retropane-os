# Sheet: the safe region's end at the on-die edge? (R23, test), 2026-10-29

**What it decides.** Where the display may read while the 3D core draws
(ADR-0034 amendment P's safe region, `rp_vram_map::SAFE_END`). Proven so
far on this X51v: right while the shown screen ends by 0xAE000, wrong from
0xB6000 (G19); the owner's find (R23): one ending at 0xB0000 exactly right,
one at 0xB0400 wrong (2700g.md "A public source agrees"), not yet rerun.
R23T (gpubench-0224) has since **measured the on-die memory's edge at
0xB0000** (a 3D target past it slows 3.7 us a KiB). If the safe region is
the on-die memory (the display reading SDRAM while the core writes SDRAM
is the clash), its end is 0xB0000 too: 8 KiB more for the lists on die
(`LISTS_ON_DIE`, R116: 96 -> 104 KiB), and one rule instead of two.

## 1. Original way (Windows Mobile / Dell's product)
Windows Mobile's primary at 0x0 (G17); Dell's code keeps no such region
(REA #30-#31: one allocator, no on-die rule), so it never knew one.

## 2. Dell's way (REA)
None: no reference to 0xAE000, 0xB0000 or an on-die limit in
`ddi_2700g`, `libGLES_CL` or `d3d_2700g` (REA #1, #13, #20 searched the
display and memory registers and constants; to search `0xb0000|0xb000[0-9a-f]`
in the asm too before the run, logged).

## 3. Modern way
Arbitrate the display's fetch and the GPU's writes in the memory
controller (QoS); where that isn't available, keep the scan-out in its own
memory. Training knowledge, a claim.

## 4. The alternatives, and why ours wins
| | What | Wins | Loses |
|---|---|---|---|
| A. Keep 0xAE000 | As proven | Nothing to run | 8 KiB of the fastest memory unused; the rule stays a measured number, not a reason |
| **B. Ours** | A shown screen ending at 0xAE000, 0xB0000, 0xB0400 and 0xB2000 while the core renders the four game frames into SDRAM (screen 1's place) at 149.50 / 2, 20 renders each, every render checked word for word; then SAFE_END = 0xB0000 only if 0xB0000 is clean and 0xB0400 isn't | The edge measured, the region tied to the on-die memory (one fact), 8 KiB to the lists if it holds | One run of about two minutes |

## 5. What would overturn it
- **B overturned** if 0xB0000 shows wrong words (keep 0xAE000) or if
  0xB0400 is also clean (then the safe region isn't the on-die memory:
  find what it is before moving it).
- The run also reads the underrun flag around each place.

**Asked of the owner:** the go to build; a separate go for the run.
