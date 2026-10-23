# The XScale core: timings measured on the X51v

Facts about the PXA270's XScale core measured on the device, each with the
run that shows it. Intel's manuals are claims until a row here confirms
them ([Intel] in the code). The clocks and voltages are in
[clocks.md](clocks.md); the research behind each line is in
[../research/cpu-full-potential.md](../research/cpu-full-potential.md).

## Multiply: the cost follows the multiplier's size

Measured 2026-10-22 on the X51v at 624 MHz (`rp-probe`'s `mul_chain_*`
rows, `rp_hal_pxa27x::cpu::mul_chain`: 1,000 dependent `mul` + `mov`
pairs a run, `k` in `Rs`), eight runs across four builds, every run within
0.5% of the others:

| Multiplier (`Rs`) | Example | ns per 1,000 | Extra cycles a multiply |
|---|---|---|---|
| fits in 15 bits (bits 31-15 all 0) | 1, `0x4F1B` | 8,192 | 0 (the base) |
| 16 to 27 bits (bits 31-27 all 0) | `0x9E37`, `0x04F1BBCD` | 9,768-9,807 | +1 |
| 28 to 32 bits | `0x9E3779B1` | 11,384-11,422 | +2 |

So the core ends a multiply early by the size of `Rs`, at the 15-bit and
27-bit marks the Core Developer's Manual gives ([Intel] claim confirmed;
16 bits already counts as large). A use of the product right after the
`mul` waits those cycles out. LLVM has no XScale timing model, so it
doesn't keep the product's use away from the multiply: whether other work
fills the wait is luck of register allocation (frame pointers on or off
moved it: ADR-0001 "Measured again"). Code that multiplies in a hot loop
chooses a multiplier under 2^27 (better, under 2^15) where it can: rp-lz4's
hash uses `0x04F1BBCD`.

Signed values with all high bits set count as small too, per Intel; not
measured.

## Clearing memory: the cache's state decides the cost

Measured 2026-10-22 on the X51v (`rp-probe`'s `lz4c_*`, `lz4r_*`, `lz4w_*`
counts; 16 clears of LZ4's 8 KiB table each):

- A clear of lines **not in the D-cache** misses on every store (9,728
  misses, 11,558-14,930 write-buffer stall cycles; ~4,600 cycles a clear):
  stores don't allocate lines, so the lines stay out of the cache and the
  next clear misses again.
- After the lines are **read** (read-allocate), the same clear has **0
  misses and 0 stall cycles**, and they stay in: LZ4's compress on a warm
  table ran 1.4% faster (1,526,413 against 1,547,575 cycles).
- Whether a stack array starts out cached is luck of where it sits.

**Line allocate (CP15 c7, c2, 5) to zero memory hung the X51v** (the
hardware watchdog, Dell's restore prompt): a probe build zeroing an 8 KiB
buffer by allocating each line then storing to it, after a clean and
invalidate of the range and with the lines already cached, never came back
(`local/cardlog/probe-0065.txt` ends in the benchmarks). Not isolated (one
row a run, the owner present, would). The whole-cache clean's own use of
it (a reserved region, then invalidated) is unaffected and in use. Until
isolated: line allocate stays out of data paths.

## Page sizes and page-table placement (the CPU gate's C4 and V7)

**`rp-probe` pages run 1 (X51v, 2026-10-22, `local/rp-probe-pages1.txt`,
card log `probe-0068`; `RP_PROBE_PAGES`):** the probe's random hop (4,096
hops, each to a new cache line, chained far apart) over 8 MiB of SDRAM
mapped a second time by `diag::with_window`, each mapping checked to be
the backing's own frames:

| Mapping | ns a hop | D-TLB misses in 4,096 hops |
|---|---|---|
| 1 MiB sections | 448.6 | 0 |
| 64 KiB pages, tables in SDRAM / SRAM | 450.8 / 450.2 | 0 / 0 |
| 4 KiB pages, both tables in SDRAM | 484.1 | 257 |
| 4 KiB, the second level in the SRAM | 475.2 | 257 |
| 4 KiB, the first level in the SRAM | 469.4 | 257 |
| 4 KiB, both levels in the SRAM | 450.2 | 257 |
| sections, the first level in the SRAM | 448.6 | 0 |

So a D-TLB miss on 4 KiB pages costs ~565 ns with both tables in SDRAM
(the slowdown over sections divided by the misses), ~424 with the second
level in the SRAM, ~331 with the first, and **~25 with both in the SRAM**
(the walk's two reads, each a missed SDRAM line otherwise). One run, one
unit. **The chase is the cache's worst case, not the TLB's:** it cycles
through a few pages at a time (257 misses, none at 64 KiB). Run 2 adds a
page-hopping chase (`suite::far_chase`: every hop to the next 4 KiB page)
for the TLB's worst case and what 64 KiB pages save.

**Run 2 (2026-10-22, `local/rp-probe-pages2.txt`, `probe-0069`): the
TLB's worst case.** `suite::far_chase` hops 4,128 bytes each time, so
every hop lands on a new 4 KiB page:

| Mapping | ns a hop | D-TLB misses in 4,096 hops |
|---|---|---|
| 1 MiB sections (tables in SDRAM / the first level in the SRAM) | 448.8 / 448.8 | 0 / 0 |
| 64 KiB pages, tables in SDRAM | 483.4 | 266 |
| 64 KiB pages, the second level in the SRAM | 476.5 | 266 |
| 4 KiB pages, both tables in SDRAM | 1,015.6 | 4,224 |
| 4 KiB, the second level in the SRAM | 880.3 | 4,224 |
| 4 KiB, the first level in the SRAM | 787.7 | 4,224 |
| 4 KiB, both levels in the SRAM | 473.1 | 4,224 |

**Every hop a miss, the cost of a miss matches run 1:** ~550 ns with
both tables in SDRAM, ~418 with the second level in the SRAM, ~329 with
the first, **~24 with both** (run 1: 565 / 424 / 331 / 25); a 64 KiB
page's miss costs the same as a 4 KiB one's (~533 ns, tables in SDRAM)
but comes 16 times less often. So with both levels in the SRAM even the
worst case for 4 KiB pages is 5% over sections (473 against 449 ns),
where with the tables in SDRAM it is 2.26x. Run 1's own chase (a few
pages at a time) repeated its numbers within 0.1%.

