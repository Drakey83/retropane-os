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
| **OS timer channels 4-11 start counting on the write to their counter (`OSCRx`), not on the write to `OMCRx`** | channel 5 set as `OSCR5 = 0` then `OMCR5 = 0x81` never moved in 10 ms; the run limit (channel 4) always wrote `OMCR4` first | likely (one run; run 2 checks the fixed order) | (P) run 1; the emulator starts them on the `OMCR` write (emulator.md) |
| The cycle counter `CCNT` is CP14 c1, c1 (`PMNC` c0, c1; bit 0 enable, bit 2 reset, bit 3 count every 64th cycle) | | confirmed (run 1 counted at the core clock) | (M); the emulator fork models the same encoding |
| OS timer channel 5 can count the 32.768 kHz clock on its own counter | `OMCR5 = 0x81` (resolution 1/32768 s, bit 7 own counter) | likely | (M) |
| TSC2046 single-ended inputs: `TEMP0` 0, `VBAT` 2, `AUX` 6, `TEMP1` 7 (SER = 1, reference on with PD = 11) | | confirmed (datasheet) | (D); whether Dell wired `VBAT`/`AUX` is unknown |

## Measurements

Run 1 (2026-10-15, under the loader, interrupts off, 624 MHz): the
on-device first look (SP 800-90B's most-common-value estimate). NIST's
full tool hasn't been run yet; its estimate can only be lower (it takes
the lowest of ten), so these are ceilings.

| Source | Samples | What it showed | MCV estimate (ceiling) | Verdict so far |
|---|---|---|---|---|
| A two-crystal jitter | none | Skipped: the 32 kHz counter never started (the start order above) | | Run 2 |
| B one-clock jitter (cycles for 64 SDRAM reads, low byte) | 1,000,000 in 15.6 s | 9 values (9,662-10,553 cycles); 87% are 9,726; 9,738 occurs **31,250 times, 99% of them exactly 32 samples apart**: a fixed pattern, SDRAM refresh most likely; 78% of samples equal the one before | 0.19 bit | Weak and partly periodic: mixed only, as ADR-0029 planned |
| T `TEMP0` (12-bit) | 1,000,000 in 18.5 s | 878-882, 5 values; 67% equal the one before (slowly moving) | 0.98 bit | Some noise, strongly correlated: the full tool will be well below this |
| T `VBAT` | 1,000,000 | 0-4, 93% read 1 | 0.11 bit | **Not wired to the battery** on the X51v (reads near 0) |
| T `AUX` | 1,000,000 | 3,834-4,095, 262 values, 58% read 3,837; rare excursions to full scale | 0.78 bit | Near full scale (tied high or a divider); some noise, correlated |
| T `TEMP1` | 1,000,000 | 1,058-1,059, 98% one value | 0.02 bit | Dead for this purpose |
| Restart test (A) | | Waits for A | | Run 2 |

So far, as ADR-0029 expected, everything rides on source A. The touch
controller and SDRAM timing are not good enough to carry the generator
alone; at most they are mixed in.

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
