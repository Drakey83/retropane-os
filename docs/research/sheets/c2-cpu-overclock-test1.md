# Sheet: C2 test 1, the PXA270 past 624 MHz (a stepped test), 2026-10-30

**What it decides.** Whether this X51v's CPU core runs cleanly above
Intel's 624 MHz, and by which route, as the first measurement of
cpu-full-potential.md's C2 (arch 9, open). The owner, 2026-10-30: "ramp
up the first cpu test". **A test only:** nothing ships without an
ADR-0031 amendment and the per-device self-test (ADR-0035's pattern),
with Dell's 624 as the fallback.

## 1. Original way (Windows Mobile / Dell's product)
Seven operating points, top 624 MHz: `CCCR` `0x02000310`, L 16 x 2N 6,
run and bus 208 MHz, turbo on, the memory clock 208 with SDRAM at 104
(`MDREFR` `0x201EE030`). The core voltage is set by three GPIO pins
(GPIO3/4/38), at their top pattern ("1574 mV") for 624 (clocks.md).

## 2. Dell's way (REA)
No point above 624 in the OAL's table and no other `CCCR` write
(clocks.md). No PMIC: the voltage can't be raised past Dell's top pattern
(cpu-full-potential.md V4, declined: unmeasurable without opening the
device). A finding, not a ceiling.

## 3. Modern way
Phone SoCs run per-unit binned operating points chosen by a boot-time
test (adaptive voltage and frequency scaling), with errors caught by
self-checks. Training knowledge, a claim. Ours: the same idea, by
measurement on each device, at a fixed voltage.

## 4. The routes past 624, and why the test steps through the first
| | Route | Core | Run, bus, memory | Wins | Loses |
|---|---|---|---|---|---|
| **A** | **L 16 x 2N 7 or 8** (turbo only) | 728 / 832 MHz | unchanged: 208 / 208, SDRAM 104 (Dell's) | No memory or bus change (Dell's RAM timing rule holds); turbo switches without a PLL relock | 2N 7-8 are outside Intel's legal set (our `Point::legal`: 2N 2-6). Whether the PLL takes them is unknown: `CCSR` says. Two coarse steps (+17%, +33%) |
| B | L 17-19 x 2N 6 | 663 / 702 / 741 | run and bus 221-247 MHz (over Intel's 208); SDRAM at 110-124 (over its rated 104) unless divided by 4 (55-62) | Fine steps (+6% each) | Breaks the RAM rule, or halves the memory's speed; the 2D engine and DMA ride the faster bus too |
| C | L 14-15 x 2N 8 | 728 / 780 | run 182-195, SDRAM 91-98 (under rated) | Fine steps, memory under rating | The bus and memory 6-13% slower: memory-bound work loses |

**Test 1 runs A first.** It is the only route that leaves the memory and
bus as Dell set them. It answers in one run: does the PLL latch 2N 7 and
8 (`CCSR`), and does the core count 728 / 832 MHz (CP14 cycle counter
against the OS timer)? C is test 2 if A's PLL refuses. B only if both
fail, as it needs the owner's ruling on the RAM rule.

## 5. The test (an `rp-clocks` stage, C2b)
- **Built on `rp-clocks`:** its proven switching (one I-cache line, IRQs
  off), breadcrumbs (a screen stripe and `RPCK` in 2700G memory before
  each step), the cycle-counter check and the put-back.
- **The voltage code goes no higher than Dell's own for 624** (the top
  pattern). If Windows Mobile left a lower one (it boots at 520 or 624),
  it is raised to that, as `rp-clocks`' C2 already does, and put back at
  the end. No board bits are touched.
- **Steps, lowest risk first:** 624 (the control), 2N 7 (728), 2N 8
  (832). Each step:
  - **the latch:** write `CCCR` and `CLKCFG`, then read back `CCSR`. If
    the PLL refused, the step is logged and skipped;
  - **the speed:** count the core clock;
  - **the stress, ~60 s:** integer and multiply checksums (known
    answers), memory patterns over 3 MiB of SDRAM (walking ones and
    address-in-address), I-cache and D-cache thrash with checks, a DMA
    copy checked;
  - **the record:** the board's temperature from the touch controller's
    `TEMP0`, as the GPU's warm runs read it (the battery thermistor waits
    for R122). Then Dell's 624 is put back.
- **Stop at the first error.** A hang is the watchdog's: the breadcrumb
  shows the step, and the card log (rp-cardlog) has the lines so far.
  **The owner watches the first run** (a new clock region).
- **Length:** about 5 minutes. A one-hour soak of the best clean step is
  test 2, at night.

## 6. What would overturn it
- **`CCSR` doesn't latch 2N 7 or 8:** route A is closed on this chip. Go
  to C (test 2).
- **An error or hang at 728:** this unit's core can't run above 624 at
  Dell's voltage by route A. C's 728 / 780 is still possible, because its
  run clock is lower.
- **Clean at 832 for 60 s:** a long warm soak decides before any self-test
  design.

**Asked of the owner:** the go to build the stage (code and emulator
check); a separate go for the device run (you watching).

## 7. Test 1 again, in the port runner (2026-11-16, the owner: "Yes, start C2")

`rp-clocks`' C2b never reached the overclock (runs 1-3 hung on its card log
and the loader's restart, R123; parked with two items open). The port
runner has since run Half-Life on this X51v for hours with its log on the
CF card throughout, and it is the program the overclock is for, so test 1
moves there (`ports/runner/src/cpu.rs`, a test run's `-rpcpu 7` or `8`):

- **Route A, unchanged:** L 16 x 2N 7 (728 MHz) or 8 (832); run, bus,
  memory and the SDRAM's timing exactly Dell's 624 point. The step is
  refused unless the core is at Dell's 624 point as found (its voltage
  pattern then the top one); the voltage pins are never touched.
- **The latch and the speed:** `CCSR` read back (a PLL that refuses 2N 7
  puts 624 back), the core's clock counted against the OS timer.
- **The core checked as it runs, not trusted:** `rp-clocks`' integer mix
  (xorshift, multiply-add, rotate, 64-bit product) with its answer worked
  out at 624 before the step, run every frame and before every card write.
  One wrong answer puts 624 back for the rest of the run; a card write is
  refused while the answer is wrong (the card is shared: the owner's game files are on it).
- **The stress is the game:** Half-Life's frames load every part of the
  core for minutes, with the integer check each frame.
- **Back to 624** when the program closes its screen, and again after it
  returns (a net under it); the run's counts logged (checks, wrong).
- **The owner watches the first run** (a new clock region): a hang is the
  hardware watchdog's (Dell's restore screen: Mail, then the loader).

What would overturn it: the PLL refusing 2N 7 (route C next: L 14-15 x
2N 8, 728 / 780 MHz with the bus and memory a little slower); a wrong
answer or a hang at 728 (this unit's core can't run past 624 at Dell's top
voltage by route A).

## 8. Run 44 on the X51v (2026-11-16, the owner watching; port-runner-0044)

Half-Life with `-rpcpu 7`. **The PLL took 2N 7** (route A: L 16, the bus,
memory and SDRAM as at 624) and **the core counted 727,965 kHz** against
the OS timer; the first known-answer check was right. **Within seconds,
while the map loaded, a data abort in abort mode at the prefetch-abort
vector** (`0xFFFF000C`, the vector's own load faulting: a fault inside a
fault), the run lost. Read: this unit's core does not hold 728 MHz at
Dell's top voltage pattern (one run; the voltage cannot be raised: no
control exists). The CPU was put back to 624 by a soft reset into Windows
Mobile right after (the loader keeps the clock it finds after a crash).

Left for the owner (results, not decisions): route C (L 14-15 x 2N 8:
728 / 780 MHz with the run clock and bus 182-195 MHz, the SDRAM 91-98)
asks the same core speed or more, so it is unlikely to hold where route A
did not; route B (L 17 x 2N 6: 663 MHz, +6%) raises the bus to 221 MHz and
the SDRAM to 110 MHz, past Intel's and the memory's ratings (the owner's
RAM rule); or C2 closed for this unit at 624.

**The owner's ruling (2026-11-16): "Close C2 at 624".** This unit runs 624
MHz; the `-rpcpu` test stays in the port runner for other units.

## 9. Route B, ordered (2026-11-16, the owner: "i want that option, i was confused")

The close at 624 above is withdrawn: the owner wants 663 MHz tried, **the
RAM rule ("RAM timing stays Dell's", 2026-10-19) lifted for this watched
test alone**. Route B: L 17 x 2N 6, turbo, fast bus, A set: core 663, run
and bus 221 MHz (Intel's 208), the memory clock 221 and the SDRAM 110.5
(rated 104) with Dell's timing registers; ~+6% core and memory.

Its risk is not route A's: RAM past its rating fails silently (a wrong bit
read back, not a crash), and the bus's slower parts (CF's and the 2700G's
timings are counted in memory-clock cycles) run ~6% tighter. So the test
is memory first:
1. **The HAL:** `Point::legal_route_b` (L 17-19, 2N 6, A and B; bus up to
   247) and `clocks::change_route_b` behind `past-rating`, its refresh
   interval worked out for the 221 MHz memory clock by the same
   `mdrefr_around` (a stale interval corrupts SDRAM silently: the
   memory-timing report), host tests on the plan's words.
2. **Before the game, nothing written to any card:** step to 663, then
   walking-ones and address-in-address over every free megabyte of SDRAM
   (the heap's unused part and the scratch), several passes, a DMA copy
   checked, the core's known answer; any wrong word puts 624 back and the
   run says so. Then 624 back, the result logged.
3. **Only if (2) is clean:** the game at 663 with the memory check repeated
   (a few hundred KiB a frame, rotating through free RAM) beside the core's,
   every card write gated on both.
4. **The owner watches.** A silent error is the risk: the log's counts, not
   the picture alone, decide.
