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
| 2026-10-11 | Compressed RAM wired into the VM (`Vm::compress`, restore on fault, stored entries in page tables) and run live in `rp-kernel`. Image grew 26.7 → 37.7 KB; built the image crate for size and removed `&str` padding code: 34.8 KB. **Kernel load budget raised 32 → 40 KiB; approved by the owner and ratcheted to 38 KiB (measured + ~10%)** | `rp-kernel` loaded size | 26,732 B | 34,840 B |
| 2026-10-11 | First thread switches (`boot/threads`): event set + wait + schedule + register switch, per switch. Emulator real time; the device gives the real number (ADR-0010 target: well under 2 µs) | `thread_switch` | — | ~1.4 µs (emulated) |
| 2026-10-11 | `Scheduler::create_event`: plain loop instead of an iterator chain that unrolled to 1 KiB | `rp-threads` loaded size | 15,404 B | 14,500 B |
| 2026-10-11 | Preemption test added to `rp-threads`; **load budget raised 15 → 17 KiB, approved by the owner** (measured + ~9%) | `rp-threads` loaded size | 14,660 B | 16,012 B |
| 2026-10-11 | New bring-up images, each with its own budget at measured + ~10% (new budgets, not raises): `rp-light` (first light), `rp-usb` (USB link test), `rp-loader` (resident loader) | loaded size | — | 16,064 / 18,320 / 20,032 B |
| 2026-10-11 | `rp-probe`: startup colours (step table instead of six inlined two-colour calls) | `rp-probe` loaded size | 40,756 B | 40,864 B |

## Real hardware

First run on the owner's X51v (2026-10-11, `rp-probe` build `8bd30913+`,
core at 520 MHz turbo as Windows Mobile left it; every line checksum-verified,
raw values in `docs/hw/probe-2026-10-11.md`). The emulator counts
instructions, so the ratio shows where real memory and buses dominate.

| Benchmark | Emulator (ns) | X51v (ns) | X51v / emulator |
|---|---|---|---|
| `timer_read` | 13 | 846 | 65.1× |
| `frames_alloc_free_split` | 567 | 1,467 | 2.6× |
| `frames_fill_drain_1024` | 165,323 | 516,460 | 3.1× |
| `memcpy_64k` | 10,292 | 687,430 | 66.8× |
| `memcpy_naive_64k` | 393,231 | 2,473,353 | 6.3× |
| `memset_64k` | 6,200 | 173,953 | 28.1× |
| `lz4_compress_page` | 132,500 | 366,595 | 2.8× |
| `lz4_decompress_page` | 76,745 | 180,388 | 2.4× |
| `lz4_compress_text` | 88,327 | 246,845 | 2.8× |
| `lz4_decompress_text` | 50,909 | 123,052 | 2.4× |
| `lz4_compress_noise` | 33,442 | 83,730 | 2.5× |
| `lz4_decompress_noise` | 10,476 | 24,345 | 2.3× |
| `chase_sdram_256k` | 61,462 | 2,071,614 | 33.7× |
| `tlb_fit_16` | 128 | 325 | 2.5× |
| `tlb_spill_60` | 436 | 21,646 | 49.6× |
| `chase_sram_256k` | 61,423 | 478,653 | 7.8× |
| `tlb_fit_16_l1sram` | — | 325 | — |
| `tlb_spill_60_l1sram` | — | 4,174 | — |
| `fb_pattern_vga` | 127,692 | 61,016,717 | 477.8× |
| `fb_solid_fill_vga` | 92,205 | 14,473,743 | 157.0× |
| `sd_read_block` | 4,519 | 3,854,205 | 852.9× |

What stands out (leads, not conclusions yet; one run):

- **Page table in SRAM makes TLB misses 5.2× cheaper** (`tlb_spill_60`
  21,646 vs 4,174 ns) and pointer-chasing in SRAM is 4.3× faster than SDRAM.
  ADR-0006's idea is worth doing.
- **Reading 2700G memory is very slow**: the pattern fill copies rows within
  the framebuffer and takes 61 ms, 4.2× the solid fill (14.5 ms, ~41 MB/s
  write). Build rows in SDRAM and only write to the 2700G.
- **SD reads are slow**: 3.85 ms per 512-byte block (~133 KB/s). The driver
  polls in 1-bit mode at a low clock; 4-bit mode, a higher clock, multi-block
  reads and DMA are the obvious steps.
- **`memcpy` 64 KiB: ~95 MB/s** (3.6× the naive loop; 6.4× in the emulator);
  `memset` ~377 MB/s. Check the SDRAM mapping's cache/write-allocate settings
  and try iWMMXt before tuning further.
- **`timer_read` 846 ns** is far more than one peripheral read should cost
  (~65× the emulator). Probably loop/measurement overhead or an uncached path:
  investigate before trusting the small benchmarks (`tlb_fit_16`,
  `frames_alloc_free_split`).
- **LZ4**: a 4 KiB heap page decompresses in 180 µs (~23 MB/s), so a
  compressed-RAM fault costs ~0.2 ms. Acceptable, but the decoder is the first
  thing to optimise on real hardware.
- **Open question:** the probe hung before drawing anything until the startup
  colours (about 1 s of extra delay) were added. Cause unknown; possibly
  something that needs time to settle after HaRET. Revisit with a serial link.

## Open leads

- `memcpy` is now 38× faster than a naive byte loop. Next: measure on the real
  X51v, where SDRAM burst behaviour (not instruction count) dominates, and try
  iWMMXt 64-bit loads/stores.
- `sd_read_block` is pure driver overhead in the emulator (no device latency);
  DMA and 4-bit mode need real-hardware numbers.
- Done 2026-10-11: first real X51v numbers (see Real hardware above).
- LZ4: short literal and match copies go through `memcpy`/`memmove` calls,
  and unaligned 32-bit reads cost four byte loads on ARMv5. Inline small
  copies, slide the compressor's 4-byte window instead of re-reading it, and
  skip faster over incompressible data.
- ADR-0006: page tables in on-chip SRAM, 64 KiB large pages, DMA pre-zeroing:
  each needs before/after numbers on hardware.
