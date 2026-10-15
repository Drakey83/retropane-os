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
| The cycle counter `CCNT` is CP14 c1, c1 (`PMNC` c0, c1; bit 0 enable, bit 2 reset, bit 3 count every 64th cycle) | | likely | (M); the emulator fork models the same encoding |
| OS timer channel 5 can count the 32.768 kHz clock on its own counter | `OMCR5 = 0x81` (resolution 1/32768 s, bit 7 own counter) | likely | (M) |
| TSC2046 single-ended inputs: `TEMP0` 0, `VBAT` 2, `AUX` 6, `TEMP1` 7 (SER = 1, reference on with PD = 11) | | confirmed (datasheet) | (D); whether Dell wired `VBAT`/`AUX` is unknown |

## Measurements (pending the owner's go)

| Source | Samples | Min-entropy per sample (NIST `ea_non_iid`) | Credit (half, at most 1 bit) | Date |
|---|---|---|---|---|
| A two-crystal jitter (low byte) | | | | |
| B one-clock jitter (low byte) | | | | |
| T `TEMP0`, `VBAT`, `AUX`, `TEMP1` (12-bit) | | | | |
| Restart test (A, first 1,000 after each start) | | | | |

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
