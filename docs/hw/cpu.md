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
