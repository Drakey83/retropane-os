# gpubench-0002 to -0006 (X50v), 2026-11-13: the steps above 139.75 / 1, and the soaks

- **Unit:** the owner's X50v, in its cradle, the loader from its own CF
  card; `cargo xtask load rp-gpubench` with one option each (R72, R189
  item 1), after GC2DR drew every game frame right to 139.75 / 1
  ([2026-11-13-gpubench-0001-x50v-gc2dr.md](2026-11-13-gpubench-0001-x50v-gc2dr.md)).
  Card logs `gpubench-0002` to `-0006` on its SD and CF cards; the lines
  below are copied from the PC's run logs.
- **Verdict:** 143.00 / 1 soak-clean (4,800 of 4,800, GC2EG); above it the
  moved textured mesh goes wrong in one column (x = 5, a few colour bits
  lost: `0x0061` for `0x3a61`): 2% of renders at 146.25 / 1 (GC2EF), every
  render at 149.50 / 1 (GC2ED). TEMP0 stayed at 827-831 throughout: the
  line is the clock, not heat. GC2EB (GC2DR's frames) was right to 149.50 /
  1 in one pass: its frames never reach that column. The step between,
  145.60 / 1, soaked clean afterwards (GC2EH,
  [2026-11-13-gpubench-0007-x50v-145.md](2026-11-13-gpubench-0007-x50v-145.md)).
- Odd, not a fault: GC2EF's 149.50 / 2 control finished 98-99 of 100
  renders a frame (none wrong).

| Run | Card log | Option | Result |
|---|---|---|---|
| GC2EB | gpubench-0002 | `RP_GPUBENCH_GC2_FINE_HIGH` | GC2DR's frames at 143.00, 146.25, 149.50 / 1: 100 of 100 right each |
| GC2ED | gpubench-0003 | `RP_GPUBENCH_GC2_SOAK149` | moved frames at 149.50 / 1, 12 parts: the textured mesh wrong 1,200 of 1,200, the other three right |
| GC2EE | gpubench-0004 | `RP_GPUBENCH_GC2_MOVED_HIGH` | moved frames at 146.25 to 137.80 / 1: 100 of 100 right each |
| GC2EF | gpubench-0005 | `RP_GPUBENCH_GC2_SOAK146` | moved frames at 146.25 / 1, 12 parts: the textured mesh wrong 24 of 1,200 (1-2 pixels, x = 5) |
| GC2EG | gpubench-0006 | `RP_GPUBENCH_GC2_SOAK143` | moved frames at 143.00 / 1, 12 parts: 4,800 of 4,800 right |

## The lines (copied)

```
GC2EB 143.00 / 1 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [8526, 8468, 8606, 8465] us; every frame right: true; TEMP0 Some(831)
GC2EB 146.25 / 1 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [8336, 8273, 8411, 8275] us; every frame right: true; TEMP0 Some(830)
GC2EB 149.50 / 1 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [8153, 8094, 8222, 8092] us; every frame right: true; TEMP0 Some(830)
GC2EB 149.50 / 2 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [13011, 12936, 13006, 12938] us; every frame right: true; TEMP0 Some(831)
GC2EB settings with every game frame right: 143.00 / 1; 146.25 / 1; 149.50 / 1; 149.50 / 2;
RETROPANE-GPUBENCH: done, 7378 renders measured, 0 hung renders recovered
GC2ED 149.50 / 1 part 1 (lock 0): wrong of 100 by frame [0, 0, 100, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4174, 4348, 8215, 4360] us; every frame right: false; TEMP0 Some(828)
GC2ED 149.50 / 1 part 2 (lock 0): wrong of 100 by frame [0, 0, 100, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4173, 4347, 8217, 4359] us; every frame right: false; TEMP0 Some(828)
GC2ED 149.50 / 1 part 3 (lock 0): wrong of 100 by frame [0, 0, 100, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4171, 4348, 8220, 4360] us; every frame right: false; TEMP0 Some(828)
GC2ED 149.50 / 1 part 4 (lock 0): wrong of 100 by frame [0, 0, 100, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4173, 4348, 8216, 4359] us; every frame right: false; TEMP0 Some(827)
GC2ED 149.50 / 1 part 5 (lock 0): wrong of 100 by frame [0, 0, 100, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4172, 4348, 8214, 4360] us; every frame right: false; TEMP0 Some(827)
GC2ED 149.50 / 1 part 6 (lock 0): wrong of 100 by frame [0, 0, 100, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4172, 4348, 8218, 4359] us; every frame right: false; TEMP0 Some(827)
GC2ED 149.50 / 1 part 7 (lock 0): wrong of 100 by frame [0, 0, 100, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4172, 4348, 8217, 4359] us; every frame right: false; TEMP0 Some(827)
GC2ED 149.50 / 1 part 8 (lock 0): wrong of 100 by frame [0, 0, 100, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4172, 4348, 8216, 4359] us; every frame right: false; TEMP0 Some(827)
GC2ED 149.50 / 1 part 9 (lock 0): wrong of 100 by frame [0, 0, 100, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4173, 4348, 8215, 4359] us; every frame right: false; TEMP0 Some(827)
GC2ED 149.50 / 1 part 10 (lock 0): wrong of 100 by frame [0, 0, 100, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4173, 4348, 8215, 4359] us; every frame right: false; TEMP0 Some(827)
GC2ED 149.50 / 1 part 11 (lock 0): wrong of 100 by frame [0, 0, 100, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4172, 4348, 8216, 4360] us; every frame right: false; TEMP0 Some(827)
GC2ED 149.50 / 1 part 12 (lock 0): wrong of 100 by frame [0, 0, 100, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4172, 4348, 8216, 4360] us; every frame right: false; TEMP0 Some(827)
GC2ED 149.50 / 2 part 1 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [8353, 8698, 12992, 8726] us; every frame right: true; TEMP0 Some(830)
GC2ED settings with every game frame right: 149.50 / 2 part 1;
RETROPANE-GPUBENCH: done, 10979 renders measured, 0 hung renders recovered
GC2ED 149.50 / 1 part 1 the textured mesh moved wrong render 1 (round 0): 29 pixels, box (5, 189)-(5, 321), first (5, 189) 0x0061 for 0x3a61
GC2ED 149.50 / 1 part 2 the textured mesh moved wrong render 1 (round 0): 29 pixels, box (5, 189)-(5, 321), first (5, 189) 0x0061 for 0x3a61
GC2EE 146.25 / 1 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4266, 4444, 8401, 4459] us; every frame right: true; TEMP0 Some(828)
GC2EE 143.00 / 1 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4365, 4545, 8597, 4560] us; every frame right: true; TEMP0 Some(828)
GC2EE 139.75 / 1 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4463, 4651, 8784, 4666] us; every frame right: true; TEMP0 Some(828)
GC2EE 138.67 / 1 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4500, 4688, 8857, 4703] us; every frame right: true; TEMP0 Some(828)
GC2EE 137.80 / 1 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4528, 4718, 8911, 4732] us; every frame right: true; TEMP0 Some(828)
GC2EE 149.50 / 2 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [8354, 8698, 12996, 8726] us; every frame right: true; TEMP0 Some(830)
GC2EE settings with every game frame right: 146.25 / 1; 143.00 / 1; 139.75 / 1; 138.67 / 1; 137.80 / 1; 149.50 / 2;
RETROPANE-GPUBENCH: done, 8177 renders measured, 0 hung renders recovered
GC2EF 146.25 / 1 part 1 (lock 0): wrong of 100 by frame [0, 0, 1, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4267, 4444, 8404, 4456] us; every frame right: false; TEMP0 Some(827)
GC2EF 146.25 / 1 part 2 (lock 0): wrong of 100 by frame [0, 0, 5, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4266, 4444, 8401, 4457] us; every frame right: false; TEMP0 Some(827)
GC2EF 146.25 / 1 part 3 (lock 0): wrong of 100 by frame [0, 0, 1, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4266, 4444, 8405, 4456] us; every frame right: false; TEMP0 Some(827)
GC2EF 146.25 / 1 part 4 (lock 0): wrong of 100 by frame [0, 0, 3, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4265, 4444, 8404, 4456] us; every frame right: false; TEMP0 Some(827)
GC2EF 146.25 / 1 part 5 (lock 0): wrong of 100 by frame [0, 0, 2, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4264, 4444, 8399, 4457] us; every frame right: false; TEMP0 Some(827)
GC2EF 146.25 / 1 part 6 (lock 0): wrong of 100 by frame [0, 0, 2, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4266, 4445, 8401, 4456] us; every frame right: false; TEMP0 Some(827)
GC2EF 146.25 / 1 part 7 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4264, 4445, 8401, 4459] us; every frame right: true; TEMP0 Some(827)
GC2EF 146.25 / 1 part 8 (lock 8): wrong of 100 by frame [0, 0, 1, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4265, 4445, 8403, 4456] us; every frame right: false; TEMP0 Some(827)
GC2EF 146.25 / 1 part 9 (lock 0): wrong of 100 by frame [0, 0, 3, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4266, 4445, 8403, 4456] us; every frame right: false; TEMP0 Some(827)
GC2EF 146.25 / 1 part 10 (lock 0): wrong of 100 by frame [0, 0, 1, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4265, 4444, 8403, 4458] us; every frame right: false; TEMP0 Some(827)
GC2EF 146.25 / 1 part 11 (lock 0): wrong of 100 by frame [0, 0, 1, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4265, 4444, 8404, 4457] us; every frame right: false; TEMP0 Some(827)
GC2EF 146.25 / 1 part 12 (lock 0): wrong of 100 by frame [0, 0, 4, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4266, 4445, 8403, 4456] us; every frame right: false; TEMP0 Some(827)
GC2EF 149.50 / 2 part 1 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [98, 99, 99, 99], fastest by frame [8357, 8695, 12997, 8726] us; every frame right: false; TEMP0 Some(829)
GC2EF settings with every game frame right: 146.25 / 1 part 7;
RETROPANE-GPUBENCH: done, 10974 renders measured, 0 hung renders recovered
GC2EF 146.25 / 1 part 1 the textured mesh moved wrong render 1 (round 33): 1 pixels, box (5, 213)-(5, 213), first (5, 213) 0x00c3 for 0x3aa3
GC2EF 146.25 / 1 part 2 the textured mesh moved wrong render 1 (round 4): 1 pixels, box (5, 265)-(5, 265), first (5, 265) 0x0165 for 0x3b65
GC2EG 143.00 / 1 part 1 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4364, 4546, 8594, 4560] us; every frame right: true; TEMP0 Some(828)
GC2EG 143.00 / 1 part 2 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4363, 4546, 8595, 4561] us; every frame right: true; TEMP0 Some(828)
GC2EG 143.00 / 1 part 3 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4365, 4546, 8596, 4560] us; every frame right: true; TEMP0 Some(828)
GC2EG 143.00 / 1 part 4 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4363, 4546, 8594, 4560] us; every frame right: true; TEMP0 Some(828)
GC2EG 143.00 / 1 part 5 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4363, 4546, 8596, 4560] us; every frame right: true; TEMP0 Some(828)
GC2EG 143.00 / 1 part 6 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4362, 4546, 8598, 4561] us; every frame right: true; TEMP0 Some(829)
GC2EG 143.00 / 1 part 7 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4363, 4546, 8594, 4560] us; every frame right: true; TEMP0 Some(829)
GC2EG 143.00 / 1 part 8 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4363, 4546, 8598, 4560] us; every frame right: true; TEMP0 Some(829)
GC2EG 143.00 / 1 part 9 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4363, 4546, 8596, 4560] us; every frame right: true; TEMP0 Some(829)
GC2EG 143.00 / 1 part 10 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4362, 4546, 8598, 4560] us; every frame right: true; TEMP0 Some(829)
GC2EG 143.00 / 1 part 11 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4363, 4546, 8596, 4560] us; every frame right: true; TEMP0 Some(829)
GC2EG 143.00 / 1 part 12 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [4363, 4546, 8598, 4560] us; every frame right: true; TEMP0 Some(829)
GC2EG 149.50 / 2 part 1 (lock 0): wrong of 100 by frame [0, 0, 0, 0] ([0, 0, 0, 0] on x = 255 alone), drawn [100, 100, 100, 100], fastest by frame [8353, 8698, 12997, 8726] us; every frame right: true; TEMP0 Some(831)
GC2EG settings with every game frame right: 143.00 / 1 part 1; 143.00 / 1 part 2; 143.00 / 1 part 3; 143.00 / 1 part 4; 143.00 / 1 part 5; 143.00 / 1 part 6; 143.00 / 1 part 7; 143.00 / 1 part 8; 143.00 / 1 part 9; 143.00 / 1 part 10; 143.0
RETROPANE-GPUBENCH: done, 10975 renders measured, 0 hung renders recovered
```

**Merge record (2026-11-13, 9:18 PM):** `main` moved from `732a49e0` to `87192d0c` (PR 17, merged on the owner's "push live"; `cargo xtask ci` passed on d806a95b, GitHub's CI on main then running, about 2.5 hours).
