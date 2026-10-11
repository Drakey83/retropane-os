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
| 2026-10-10 | Bring-up MMU, caches, interrupts and idle added (`hal/pxa27x`: `init_mmu`, `start_tick`, `wait_tick`); input loop sleeps between 200 Hz ticks instead of busy-waiting. Emulator counts instructions, so no change there; on hardware this enables the D-cache and lets the core idle | `rp-hello` loaded size | 19,184 B | 23,288 B |
| 2026-10-10 | Dropped `Debug`/full-panic formatting from images (fixed names, panic location only), per ADR-0005 rule 8 | `rp-hello` loaded size | 23,288 B | 20,056 B |
| 2026-10-10 | Solid fills: `rp_hal_mem::fill32` (any alignment, 32-byte `stm` bursts) and `fill_rect_rgb565` in the display driver, one burst fill per row | `fb_solid_fill_vga` | 1,075,282 ns | 92,205 ns (**11.7×**) |

## Open leads

- `memcpy` is now 38× faster than a naive byte loop. Next: measure on the real
  X51v, where SDRAM burst behaviour (not instruction count) dominates, and try
  iWMMXt 64-bit loads/stores.
- `sd_read_block` is pure driver overhead in the emulator (no device latency);
  DMA and 4-bit mode need real-hardware numbers.
- Get the first real X51v numbers for all benchmarks.
