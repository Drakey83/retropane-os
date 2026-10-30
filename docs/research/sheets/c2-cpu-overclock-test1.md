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
- **The voltage code is never changed:** it stays at the top pattern, as
  found under Windows Mobile at 624. No board bits are touched.
- **Steps, lowest risk first:** 624 (the control), 2N 7 (728), 2N 8
  (832). Each step:
  - **the latch:** write `CCCR` and `CLKCFG`, then read back `CCSR`. If
    the PLL refused, the step is logged and skipped;
  - **the speed:** count the core clock;
  - **the stress, ~60 s:** integer and multiply checksums (known
    answers), memory patterns over 3 MiB of SDRAM (walking ones and
    address-in-address), I-cache and D-cache thrash with checks, a DMA
    copy checked;
  - **the record:** temperature where readable (the 2700G's `TEMP0`; the
    battery thermistor waits for R122). Then Dell's 624 is put back.
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
