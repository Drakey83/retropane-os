# gpubench-0138 (X51v), 2026-11-14: GC2DR on a cold chip (R189 item 2)

- **Unit:** the owner's X51v, off and cooling overnight, booted to the
  loader and run first thing ("cold, loader is up for x51v", 11:36 AM CST);
  `RP_GPUBENCH_GC2_FINE=1 cargo xtask load rp-gpubench`, the run prepared
  in decisions-2026-11-09.md. Card log `gpubench-0138` on the SD and CF
  cards; the lines below are copied from the run log.
- **Verdict: colder, fewer faults past the line, and the first wrong step
  moved up one, from 138.67 / 1 to 139.75 / 1.** 138.67 / 1 drew 400 of 400
  right (warm 2026-11-08: 10 wrong; idle an hour 2026-11-09: 1 wrong);
  139.75 / 1 drew 3 of 400 wrong (warm 99, idle 45), each 7 pixels one step
  of red-green off in one row. 137.80 / 1 and every step below right, as
  in every earlier run.
- **What it means for games:** a game heats the chip as this run's own
  warm-up did, so the warm line (137.80 / 1 right, 138.67 / 1 first wrong)
  still decides what is safe in a game; the cold start shows the margin
  grows with a cooler chip, by about one step. One run, one chip. No
  ruling changes (the 136.50 / 133.25 game step is settled).
- **Temperature (TEMP0, lower reads hotter):** 913 at the start, the
  coolest start recorded (warm runs 867, idle an hour 882). The GC2BY
  warm-up at 149.50 / 1 brought it to 884 in ten slices, so the GC2DR
  ladder itself ran at 883 to 880: cooler than the idle run (873-874)
  and the warm runs (867).
- The warm-up (GC2BY at 149.50 / 1): 1 wrong in 5,704 renders, none hung.
- Nothing hung, 0 hung renders recovered, everything put back, the
  controls at Windows Mobile clock 4 of 4 right, card log 85 writes 0
  refused.

## The lines (copied from the run log)

```
GC2: the 16-bit kinds' targets inside the safe region (GC2DR: no map; GC2DJ's shifted textured frames (0, +8, +3, -8 px), each its own reference, at 139.75, 138.67, 137.80, 136.50, 135.20, 134.33 and 133.25 / 1, then 149.50 / 2; the lists on the die in the checker order; each target poisoned first), by the owner's robustness rules: 9 kinds at 0 settings rising then falling, 20 renders a kind a setting, each followed by one at Windows Mobile's clock (B, A, B, A); every picture read twice 50 ms apart; references rendered 5 times, held to the host model of the 3D core and taken again at the end; the 3D core reset and every register written again at each setting; the clock, memory and 3D registers, the underrun flag and DSIG read around every render; every wrong pixel described; a 16-bit label throughout
GC2BY TEMP0 at the start: Some(913)
GC2BY A slice 1: 570 renders, 0 wrong, 0 not finished, mean 4483 us, TEMP0 Some(905)
GC2BY A slice 2: 570 renders, 0 wrong, 0 not finished, mean 4484 us, TEMP0 Some(901)
GC2BY A slice 3: 572 renders, 0 wrong, 0 not finished, mean 4483 us, TEMP0 Some(898)
GC2BY A slice 4: 570 renders, 0 wrong, 0 not finished, mean 4484 us, TEMP0 Some(896)
GC2BY A slice 5: 570 renders, 0 wrong, 0 not finished, mean 4484 us, TEMP0 Some(893)
GC2BY A slice 6: 572 renders, 0 wrong, 0 not finished, mean 4483 us, TEMP0 Some(891)
GC2BY A slice 7: 570 renders, 0 wrong, 0 not finished, mean 4483 us, TEMP0 Some(889)
GC2BY A slice 8: 570 renders, 0 wrong, 0 not finished, mean 4483 us, TEMP0 Some(887)
GC2BY A slice 9: 570 renders, 1 wrong, 0 not finished, mean 4483 us, TEMP0 Some(886)
GC2BY A slice 10: 570 renders, 0 wrong, 0 not finished, mean 4483 us, TEMP0 Some(884)
GC2DR 139.75 / 1 the textured mesh -8 wrong render 1 (round 16): 7 pixels, box (257, 80)-(263, 80), first (257, 80) 0x2a43 for 0x2a44
GC2DR 139.75 / 1 the textured mesh -8 wrong render 2 (round 29): 7 pixels, box (257, 80)-(263, 80), first (257, 80) 0x2a43 for 0x2a44
GC2DR 139.75 / 1 (lock 0): wrong of 100 by frame [0, 0, 0, 3] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [8748, 8684, 8812, 8678] us; every frame right: false; TEMP0 Some(883)
GC2DR 138.67 / 1 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [8815, 8750, 8881, 8745] us; every frame right: true; TEMP0 Some(883)
GC2DR 137.80 / 1 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [8874, 8805, 8940, 8808] us; every frame right: true; TEMP0 Some(882)
GC2DR 136.50 / 1 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [8959, 8894, 9023, 8888] us; every frame right: true; TEMP0 Some(882)
GC2DR 135.20 / 1 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [9046, 8978, 9113, 8974] us; every frame right: true; TEMP0 Some(881)
GC2DR 134.33 / 1 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [9109, 9037, 9174, 9029] us; every frame right: true; TEMP0 Some(880)
GC2DR 133.25 / 1 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [9181, 9111, 9250, 9108] us; every frame right: true; TEMP0 Some(880)
GC2DR 149.50 / 2 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [13017, 12941, 13009, 12944] us; every frame right: true; TEMP0 Some(880)
GC2DR settings with every game frame right: 138.67 / 1; 137.80 / 1; 136.50 / 1; 135.20 / 1; 134.33 / 1; 133.25 / 1; 149.50 / 2;
GC2BY controls at Windows Mobile's clock after: 4 of 4 right
GC2BY done, TEMP0 Some(880)
put back: COREPLL 0xbb1, MBXCLK 0x6, MSC1 0x24842481
RETROPANE-GPUBENCH: done, 8977 renders measured, 0 hung renders recovered
[card log closed: the run returned; 85 sector writes before this line, 0 refused, slowest 10683 us, 0 bytes dropped]
```
