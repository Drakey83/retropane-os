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
display and memory registers and constants). The asm search owed before
the run, done 2026-10-29 (REA #32-#33, `local/re/rea-search-log.md`):
`0xb0000|0xb000[0-9a-f]` and the building and literal-pool forms of
0xAE000, 0xB0000 and 0xB0400 in `ddi_2700g.asm` and `libGLES_CL.asm`
found nothing. Dell's code holds no on-die limit; the edge is ours to measure.

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

## Built (the owner's go 2026-10-29): run R23E, two controls added
`rp-gpubench` run R23E (`RP_GPUBENCH_GC2_HALF` + `RP_GPUBENCH_R23E`;
emulator "run R23E"). Added while building, for the owner to accept or
drop before the run:
- **A control that must fail: a screen ending at 0xB6000** (G19's first
  wrong place). If it comes out clean too, the run can't see the clash
  and proves nothing about 0xB0400.
- **G19's own setting, 74.75 / 1, beside 149.50 / 2.** C71 and GC2S show
  the clash much rarer at divide 2 (7 of 9 kinds right with the screen
  wholly outside), and 0xB0400 puts only 1 KiB of 600 outside: at
  149.50 / 2 alone a clean 0xB0400 would likely mean "too small to see",
  not "the region isn't on die".

So the rule becomes: **SAFE_END = 0xB0000 if 0xB0000 is clean at both
settings, 0xB6000 is wrong (the run sees the clash), and 0xB0400 or
0xB2000 is wrong at 74.75 / 1.** If 0xB6000 is clean, the run is
inconclusive; if 0xB0000 is wrong anywhere, keep 0xAE000. The references
are rendered with the display's fetch off; every render is checked word for
word; nothing is written to on-die memory (the screens show whatever is
there). It takes about a minute and the screen flickers.

## The run (2026-10-29, gpubench-0226; 2700g.md "R23E on the X51v")
At 74.75 / 1, renders wrong of 80: 0xAE000 0, **0xB0000 0**, 0xB0400 1
(1 word), 0xB2000 6 (8), 0xB6000 10 (23); at 149.50 / 2 none anywhere,
the control included. Against the rule: 0xB0000 clean at both, 0xB6000
wrong (at 74.75 / 1), 0xB0400 and 0xB2000 wrong at 74.75 / 1: **met**.
The 149.50 / 2 half saw nothing, so it neither supports nor overturns.
Raising `SAFE_END` to 0xB0000 (lists on die 104 KiB) waits for the owner.

## Run 2 on the owner's pick (2026-10-29: "run R23E at 143.00 / 1 first, then decide")
The edge was measured at 74.75 / 1 and 149.50 / 2 only; divide 1 would
ship at 143.00 / 1 (GC2BY's fastest game-safe step), where the clash may
be stronger. R23E gains 143.00 / 1 as a third setting. The references are
now 74.75 / 1's alone; each later setting's own (fetch off) are held to
them and logged, so a fast clock's wrong render can't become the picture.
Read: 0xB0000 clean at 143.00 / 1 and the places past it wrong supports
raising SAFE_END for divide 1 too; 0xB0000 wrong there means divide 1 needs
the smaller region (or keeps 0xAE000), whatever divide 2 gets. About two
minutes.
