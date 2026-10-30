# Sheet: divide 1 and the copy path together (R117, test), 2026-10-29

**What it decides.** Whether divide 1 (143.00 / 1, the fastest step
measured right for game frames, GC2BY) can keep the graphics service's
copy path (a frame drawn into screen 1 in SDRAM, then copied: R20,
ADR-0034) and the 3D lists on die (R116 C, ADR-0027), or what changes so
it can. R23E run 2 (gpubench-0227): at 143.00 / 1 the four game frames
drawn into SDRAM (0x296000) with their lists in SDRAM, in row order,
were 23 words wrong with the display's fetch off (nothing else reading).
GC2BY's frames, right at 143.00 / 1, had target and lists on die in
checker order. Three things differ; one run tells which matters.

## 1. Original way (Windows Mobile / Dell's product)
Dell's driver runs the cores at divide 1 only up to 74.75 MHz (GC2,
2700g.md "Dell's colour clear"), back buffers wherever its first-fit
allocator gives (REA #25-#31, SDRAM included): at its clock the place
doesn't matter, so it never met this.

## 2. Dell's way (REA)
Nothing to add: no placement rule and no divide-1 rule past 74.75 MHz in
`ddi_2700g` or `libGLES_CL` (REA #25-#33, logged). Searched; nothing
for this case.

## 3. Modern way
A GPU whose memory traffic outruns one memory keeps its hot buffers
(tile lists, the render target) in on-chip memory and streams only the
finished tiles out: tile-based GPUs from PowerVR's own later cores to
today's phone GPUs keep the parameter buffer and tile memory on chip.
Training knowledge, a claim.

## 4. The alternatives, and why ours wins
| | What | Wins | Loses |
|---|---|---|---|
| A. Give up the copy path at divide 1 | Divide 1 only for frames drawn into screen 0 (on die) | Nothing to run | Frames too big for behind-the-scan tear or wait; no proof that SDRAM is the cause |
| B. Drop to 149.50 / 2 for copy-path frames | Clock per frame | Both, if the switch is cheap | The switch's cost unknown; a PLL step per frame could cost more than divide 1 gains |
| **C. Ours: find the cause in one run, then pick** | At 143.00 / 1, display fetch off, the four game frames, 20 renders each, every render checked against 74.75 / 1's references: target on die / SDRAM x lists on die / SDRAM (row order), then target SDRAM + lists on die in checker order; and the cores' divider alone switched 1 -> 2 -> 1 at 143.00 (no PLL relock), timed, a render checked after each | If lists on die fix SDRAM targets, the copy path works at divide 1 with nothing given up (R116 C becomes required, and SAFE_END's 8 KiB helps it); if not, the switch time says whether B is worth building; if neither, games at half size on die (D) | One run of about two minutes |
| D. Games at half size, all on die | 240 x 320 frames shown at 2x (GC2CA), two buffers + lists in 704 KiB | True double buffering at divide 1, no SDRAM | Half resolution in games: the owner's call, only if C rules out the others |

## 5. What would overturn it
- **C's reading overturned** if 74.75 / 1's references aren't repeatable
  (the run checks them twice) or the on-die / on-die way (GC2BY's) is
  wrong too at 143.00 / 1 cold (then it's temperature or this image, not
  the place: compare GC2BY's warm ladder).
- **B overturned** if the divider-only switch disturbs a render after it
  or takes longer than divide 1 saves on the frame (~4-6 ms, GC2BY).

**Asked of the owner:** the go to build C (`rp-gpubench` run R117 with
an emulator check); a separate go for the device run.
