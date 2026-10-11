# Performance log

Every optimisation is measured, recorded here with before/after numbers, and
locked in by the benchmark baseline (`bench/baseline-emulator.json`) or a size
budget (`xtask/src/size.rs`). See ADR-0005 and the four lenses (🔁 iterate,
⚡ better/faster).

## How we measure

- **Emulator, deterministic:** `cargo xtask bench` boots `boot/bench` in QEMU with
  `-icount shift=0`, so one virtual nanosecond = one instruction. Results are
  repeatable to within 0.03% (timer resolution), so CI fails on > 2% regressions.
  This measures **work done**, not real time: memory and bus speeds aren't modelled.
- **Real X51v:** the same image booted from SD (planned). Absolute numbers will
  differ, especially for 2700G memory, which sits behind a slower bus.
- **Size:** `cargo xtask cross` checks every bare-metal image against its budget.

## Log

| Date | Change | Metric | Before | After |
|---|---|---|---|---|
| 2026-10-10 | Startup stack 64 KB → 16 KB (measured usage is far smaller) | `rp-hello` zeroed RAM | 65,544 B | 16,392 B |
| 2026-10-10 | Benchmark runner made non-generic (`dyn`, never inlined); LLVM had unrolled 64 × 512 FIFO reads | `rp-bench` loaded size | 189,308 B | 10,276 B |
| 2026-10-10 | Test-pattern fill: build one row as solid spans, copy it down with `memcpy`, no per-pixel maths, no extra memory | `fb_pattern_vga` | 16,434,564 ns | 642,974 ns (**25.6×**) |
| 2026-10-10 | Own `memcpy`/`memset`/`memmove`/`memcmp` (`hal/mem`): 32-byte `ldm`/`stm` bulk loops + `pld`, instead of compiler-builtins' one word per four instructions. Exhaustively tested on host and on target | `memcpy_64k` | 65,585 ns | 10,292 ns (**6.4×**) |
| 2026-10-10 | (same change) | `memset_64k` | 49,200 ns | 6,185 ns (**8.0×**) |
| 2026-10-10 | (same change, the pattern fill copies rows with `memcpy`) | `fb_pattern_vga` | 642,974 ns | 127,692 ns (5×; **129×** vs. the original) |

## Open leads

- `fb_solid_fill_vga` (one 32-bit store per loop) is *slower* than the row-copy
  fill: the library `memcpy` moves data in multi-word bursts. Fills should use
  burst stores (`stm`) too.
- `memcpy` is now 38× faster than a naive byte loop. Next: measure on the real
  X51v, where SDRAM burst behaviour (not instruction count) dominates, and try
  iWMMXt 64-bit loads/stores.
- `sd_read_block` is pure driver overhead in the emulator (no device latency);
  DMA and 4-bit mode need real-hardware numbers.
- Get the first real X51v numbers for all benchmarks.
