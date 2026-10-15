# X51v noise sources for the random number generator

> **Sources:** (M) Intel PXA27x Developer's Manual (clocks, OS timer,
> performance monitor); (D) TI TSC2046 datasheet; (P) RetroPane's own
> probe and `rp-noise` runs on the owner's X51v. Design: ADR-0029.

What each noise source holds, measured on the device. Until a source has a
measured row here it earns **no credit** (ADR-0029 R1): the generator
can't make keys from it.

## Facts so far

| Fact | Value | Confidence | Evidence |
|---|---|---|---|
| The 32.768 kHz oscillator is on and stable under Windows Mobile | `OSCC = 0x0000000F`: bit 0 OOK (stable) and bit 1 OON (on) set | likely (bit names as we read the manual; `rp-noise` reads it again) | (P) probe 2026-10-11, `probe-2026-10-11.md` |
| So the 32 kHz clock is its own crystal, independent of the 13 MHz one that makes the core clock and OSCR0 | follows from OON | likely | (M), (P) |
| The 32.768 kHz oscillator is on under RetroPane too, and the core runs at 624 MHz under the loader | `OSCC = 0x0F`; `CCCR 0x02000310`, `CCSR 0x30000310`, `CLKCFG 0xB`; the cycle counter measured 624.001 MHz against the 3.25 MHz timer | confirmed | (P) `rp-noise` run 1, 2026-10-15 |
| **OS timer channel 5 counts the 32.768 kHz clock only with `OMCR5` bit 6 set (`0xC1`)**; `0x81` (as the manual reads), `0x01`, `0x82` (1 ms) and `0x84` (1 us) never counted, whichever register was written first. **Channel 11 counts with `0x81`**; channels 6 and 8 with `0x81` don't | `0xC1` and channel 11: 66 counts in 2 ms (32.8 kHz); the rest 0 | confirmed (runs 1-3) | (P) run 3, 2026-10-15, nine ways tried; the emulator counts every one (emulator.md) |
| The cycle counter `CCNT` is CP14 c1, c1 (`PMNC` c0, c1; bit 0 enable, bit 2 reset, bit 3 count every 64th cycle) | | confirmed (run 1 counted at the core clock) | (M); the emulator fork models the same encoding |
| TSC2046 single-ended inputs: `TEMP0` 0, `VBAT` 2, `AUX` 6, `TEMP1` 7 (SER = 1, reference on with PD = 11) | | confirmed (datasheet) | (D); whether Dell wired `VBAT`/`AUX` is unknown |

## Measurements

Runs 1-3 (2026-10-15, under the loader, interrupts off, 624 MHz; B and
T agreed between runs within a few percent, except `TEMP0` (0.98, 0.51,
0.46 bit) and `TEMP1` (0.02, 0.86, 0.48): both drift between runs): the
on-device first look (SP 800-90B's most-common-value estimate). NIST's
full tool hasn't been run yet; its estimate can only be lower (it takes
the lowest of ten), so these are ceilings.

| Source | Samples | What it showed | MCV estimate (ceiling) | Verdict so far |
|---|---|---|---|---|
| A two-crystal jitter (run 3: core cycles between 32 kHz ticks, polled) | 1,000,000 in 30.5 s | Almost only two values, 19,176 and 18,672 cycles (mean 19,043 = 624 MHz / 32,768): the **504-cycle step is one poll of the timer register** (~0.8 us), and which value comes is a near-regular beat between the two crystals (a short sample every 3-4). A predictor trained on the first half guesses the second half 94% right from 3 samples back, 98.4% from 48 (and still improving); LZMA packs the low bytes to 0.07 bit a sample | 0.44 bit | **At most ~0.02 bit a sample as sampled**: the crystals' real jitter is far finer than the 504-cycle poll. Not creditable this way; see "What next for A" |
| B one-clock jitter (cycles for 64 SDRAM reads, low byte) | 1,000,000 in 15.6 s | 9 values (9,662-10,553 cycles); 87% are 9,726; 9,738 occurs **31,250 times, 99% of them exactly 32 samples apart**: a fixed pattern, SDRAM refresh most likely; 78% of samples equal the one before | 0.19 bit | Weak and partly periodic: mixed only, as ADR-0029 planned |
| T `TEMP0` (12-bit) | 1,000,000 in 18.5 s | 878-882, 5 values; 67% equal the one before (slowly moving) | 0.98 bit | Some noise, strongly correlated: the full tool will be well below this |
| T `VBAT` | 1,000,000 | 0-4, 93% read 1 | 0.11 bit | **Not wired to the battery** on the X51v (reads near 0) |
| T `AUX` | 1,000,000 | 3,834-4,095, 262 values, 58% read 3,837; rare excursions to full scale | 0.78 bit | Near full scale (tied high or a divider); some noise, correlated |
| T `TEMP1` | 1,000,000 | 1,058-1,059, 98% one value | 0.02 bit | Dead for this purpose |
| Restart test (A) | 1 row (run 3) | Needs many runs | | Later, with a better sampler |

So far nothing sampled this way is good enough to carry the generator:
ADR-0029's rule (credit only after measurement) did its job. The touch
controller and SDRAM timing stay mixed only.

### What next for A (for the owner's choice)

The noise between two crystals is real but tiny per tick (a crystal's
jitter is picoseconds; one core cycle is 1.6 ns); polling a slow timer
register only sees it in 504-cycle steps. Ways to see it finer, each a
test-image change measured the same way:

1. **Timestamp the timer's interrupt** instead of polling: arm a match
   on the 32 kHz channel and read `CCNT` first thing in the handler. The
   step becomes the interrupt path's few cycles, and the path's own
   variation (crossing from the 32 kHz domain into the core's) adds noise.
2. **Space the samples out**: phase noise adds up over time (a random
   walk), so one sample every N ticks holds more unpredictable phase;
   fewer samples per second, more entropy each.
3. **Wake from idle on the tick** and timestamp the wake-up: the core
   restarting its clock is another crossing.

## How to measure (`rp-noise`)

The test image reads only, apart from its result files on the card; it
returns to the loader by itself after about two minutes.

1. In the emulator first: `cargo xtask emu-noise` (plumbing only: the
   emulator's timing is the PC's, docs/hw/emulator.md).
2. On the device, **only when the owner says go**: `cargo xtask load
   rp-noise`. The run log gives a first look at every source (range,
   distinct values, the 90B most-common-value estimate).
3. Fetch the files: `cargo xtask get noise-a.bin` (and `noise-b.bin`,
   `noise-t0.bin`, `noise-vb.bin`, `noise-ax.bin`, `noise-t1.bin`,
   `noise-r.bin`, `noise.txt`).
4. Each run adds one row to `noise-r.bin` (the restart test, up to 1,000
   rows); a few runs from the loader and about 20 from a fresh power-up
   give the start-up picture ADR-0029 asks for.
5. On the PC, NIST's tool (github.com/usnistgov/SP800-90B_EntropyAssessment,
   built in WSL): the files hold little-endian 16-bit samples; take the
   low byte of A and B (8 bits a symbol) and the 12-bit TSC values as two
   bytes or their low 8 bits, then `ea_non_iid -v <file> 8`, and
   `ea_restart` on `noise-r.bin` (1,000 bytes a row).
