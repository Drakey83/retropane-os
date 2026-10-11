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
| 2026-10-11 | Kernel frame allocator (`kernel/mm`, buddy, side-array links, `.bss`-only state): first measurement, the baseline for ADR-0006's ideas | `frames_alloc_free_split` (worst case: 10 splits + 10 merges) | — | 567 ns |
| 2026-10-11 | (same) | `frames_fill_drain_1024` (1024 allocs + 1024 frees) | — | 165,323 ns (~81 ns per operation) |
| 2026-10-11 | UART transmit wait made bounded (a real X51v may have no clocked FFUART; debug output must never hang) | `rp-hello` loaded size | 23,400 B | 23,328 B |
| 2026-10-11 | On-screen crash box for faults and panics (real device has no serial port) | `rp-hello` loaded size | 23,328 B | 24,516 B |
| 2026-10-11 | ADR-0005 rule 8: cold crates (`rp-hello`, `rp-drv-sdcard`, `rp-drv-cpld`) built with `opt-level = "s"`; hot crates stay at 3. No benchmark moved (incl. `sd_read_block`). Budget tightened 24 → 20 KiB | `rp-hello` loaded size | 24,516 B | 18,028 B (**−26%**) |
| 2026-10-11 | "Never silently discard": unclaimed IRQs and a dead UART are reported on screen | `rp-hello` loaded size | 18,028 B | 18,360 B |
| 2026-10-11 | Benchmarks moved into a shared suite (`boot/suite`) used by `bench` and the device `probe`; added `chase_sdram/sram_256k` and `tlb_fit/spill` (+ `_l1sram` on device) for ADR-0006. Existing results unchanged (emulator has no latency model, so the new ones only become meaningful on hardware) | all existing benchmarks | — | ±0.1% |
| 2026-10-11 | Interrupt-driven input (ADR-0007), then: non-generic critical sections, a plain bounds-check message in `Mmio` (no offset formatting at every register access), all integers printed as `u32` (one formatter per kind) | `rp-hello` loaded size | 18,392 B (polling) → 20,904 B (IRQ lines) | 19,544 B |
| 2026-10-11 | `tlb_fit/spill` raised from 64 to 1024 rounds: at 64 a run was ~26 timer ticks, so code-layout noise of one tick read as a 4% "regression". Baseline re-recorded (measurement fix, not a code change) | `tlb_fit_16` resolution | ±4% | ±0.3% |
| 2026-10-11 | First live page faults (`boot/kernel`): zero-fill fault including abort entry, `handle_fault`, frame allocation, zeroing and mapping. Emulator real time (no `-icount`), so only a smoke number until it runs on the device | `vm_fault_zero` | — | ~22 µs (emulated) |
| 2026-10-11 | LZ4 block codec (`kernel/lz4`, ADR-0005 rules 4 and 6): first measurement on a heap-shaped 4 KiB page (pointers and small integers). Correct but slow: ~26 emulated instructions per decoded byte, where LZ4 can do a few. Baseline for optimisation | `lz4_decompress_page` | — | 105,803 ns |
| 2026-10-11 | (same) | `lz4_compress_page` | — | 130,212 ns |
| 2026-10-11 | LZ4 decoder: fast path for short sequences (one bounds check for the whole sequence), short copies inlined, per-byte bounds checks removed by slicing once and copying with `zip` (the listing showed two checks per copied byte). Found by reading the generated ARM code, not guessing | `lz4_decompress_page` | 105,803 ns | 76,745 ns (**−27.5%**) |
| 2026-10-11 | LZ4 compressor: same treatment for emission and match extension: no change (+0.3%). Its miss loop is already ~22 instructions; on this page (936 minimum-length matches in 4 KiB) the cost is inherent. Next: benchmark representative pages (code, text) beside this stress page | `lz4_compress_page` | 130,212 ns | 130,654 ns |
| 2026-10-11 | LZ4 benchmarks on representative pages: text (UI strings: long matches) and noise (already-compressed data) beside the heap stress page. Text decodes at ~12 emulated instructions per byte | `lz4_decompress_text` | — | 50,909 ns |
| 2026-10-11 | LZ4 compressor: skip acceleration (search step grows after each 64 misses in a row; a match resets it), so incompressible pages are skimmed. Ratios unchanged on heap and text pages; those got ≤1.5% slower | `lz4_compress_noise` | 142,019 ns | 33,462 ns (**4.2×**) |
| 2026-10-11 | Probe: one shared crash routine for faults and panics instead of two inlined copies | `rp-probe` loaded size | 40,928 B | 40,664 B |

## Open leads

- `memcpy` is now 38× faster than a naive byte loop. Next: measure on the real
  X51v, where SDRAM burst behaviour (not instruction count) dominates, and try
  iWMMXt 64-bit loads/stores.
- `sd_read_block` is pure driver overhead in the emulator (no device latency);
  DMA and 4-bit mode need real-hardware numbers.
- Get the first real X51v numbers for all benchmarks: `cargo xtask sdkit`
  now boots the probe, which shows them on screen for a photo.
- LZ4: short literal and match copies go through `memcpy`/`memmove` calls,
  and unaligned 32-bit reads cost four byte loads on ARMv5. Inline small
  copies, slide the compressor's 4-byte window instead of re-reading it, and
  skip faster over incompressible data.
- ADR-0006: page tables in on-chip SRAM, 64 KiB large pages, DMA pre-zeroing:
  each needs before/after numbers on hardware.
