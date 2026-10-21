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
- **USB link (item 74):** `cargo xtask usbbench`, on the device through the
  resident loader, under the Axim lock. *Serial* (what image loading runs
  on): a 921,600-byte image frame with a deliberately wrong CRC, timed from
  the PC's first byte to the loader's `RPL ERR bad crc` (every byte crosses,
  one CRC pass on the device, nothing runs). *Drive* (mass storage, ADR-0014):
  a 16 MiB file written to the SD card's drive and read back with Windows'
  cache bypassed (`FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH`),
  compared byte for byte, deleted; the card's own speed is in these numbers.
  *Latency*: 20 round trips of a one-packet command, each after 300 ms of
  quiet (the loader asleep). `usbbench trial <loader>` runs all of it on a
  new loader tried from RAM and puts the card's loader back. Full speed's
  ceiling: 19 bulk packets of 64 bytes per 1 ms frame, 1,216 KB/s on the
  wire; ~1.1-1.15 MB/s is what a host's scheduling leaves.

## Rules

- **No floating point in device code** (2026-10-15). The PXA270 has no FPU:
  every float operation is a library call costing tens to over a hundred
  cycles. Hot code uses fixed point; floats the 3D core consumes (object and
  TA vertex words are IEEE floats) are built from integers or folded at
  compile time. `cargo xtask cross` fails any image whose symbol table names
  a software float helper (`__aeabi_fmul`, `__addsf3`...) unless it is listed
  in `FLOAT_ALLOWED` (`xtask/src/size.rs`) with a reason. On the day the
  check went in, no image linked one. It also fails float formatting (`{}` of an `f32`
  pulls in ~20 KiB of `core::fmt` code; found in `rp-mbx3d` version 21):
  log floats as integers or labels.

## Log

| Date | Change | Metric | Before | After |
|---|---|---|---|---|
| 2026-10-21 | `rp-gpubench` run FF (quality audit C37): the 2D renderer and its frame in the image (`FF_FRAME`, a static). Zeroed RAM budget 22 -> 24 KiB (test image rule); `cargo xtask load` refused the run at 22,840 B. The emulator check builds variants without the size check: quality audit C45 | `rp-gpubench` (run FF) zeroed RAM | 20,216 B (run A) | 22,840 B |
| 2026-10-21 | Every crash through one path, `rp-image` (quality audit C38): each image now carries the card-log writer for its crash line (~4 KB, the largest part; images that never wrote the run log before gain it), the crash box where the image takes the display (console and font, ~2 KB; none in the kernel or other images without a display), the fault's word and SPSR, and the panic's message. Trimmed on the way: one crash-line type instead of a closure per kind (no call shims, ~1 KB an image), and `return_to_loader`'s stack line without `&str` padding (`Formatter::pad`, ~1 KB in images that had no other use). **Kernel raised 95 -> 112 KiB under the standing rule (+6.6 KB, under 8 KB a step)**; 15 test images ratcheted to measured + ~10% (reasons beside each in `xtask/src/size.rs`); 13 shrank or held. Clean `main` vs the C38 branch, both built the same day (`rp-looks` and `rp-today` left out: their photos are `local/` files) | `rp-kernel` / `rp-bench` / test images in all (36) loaded size | 96,732 / 54,293 B | 103,360 / 69,444 B (+6,628 / +15,151; all 36: +116,103 B, ~3.2 KB each) |
| 2026-10-20 | The card log for every run (quality audit C24, owner's vote B): the loader reserves the files, every image writes its lines through the run log's writer (`rp_hal_x51v::cardsink`). **Loader budget raised 112 -> 124 KiB, approved by the owner**; the test images over their budgets ratcheted to measured + ~10% (the test-image rule; reasons beside each in `xtask/src/size.rs`); rp-gpubench lowered 196 -> 117 KiB (its own card log and file systems gone). The image side is the board HAL's own SD/CF sector writer (the drivers sit a layer above it), checked register for register against the drivers. The kernel doesn't grow | `rp-loader` / a typical test image (`rp-light`) / `rp-gpubench` loaded size | 114,516 / 19,885 / 185,436 B | 121,204 / 24,185 / 112,152 B |
| 2026-10-16 | DMA channel map (`hal/pxa27x/src/dma_map.rs`): one table of channel, owner and chain size, checked at compile time (no channel twice, all chains within 1,664 B); chains sized per owner instead of 32 descriptors each: SD 0, audio 1, CF 2 (reserved), USB controller 3 with 8 (its largest transfer is the drive's 32 KiB buffer, checked in the loader) | `rp-loader` zeroed RAM with four channels | 20,608 B (four 32-descriptor chains: over the 20,480 budget) | 20,224 B |
| 2026-10-16 | **USB device controller (item 74), measured on the device:** double-buffered bulk endpoints (`UDCCRx.DE`), every waiting packet drained per poll, OUT reads that wait for the controller's status to settle (with two buffers a full packet's count once read short and the packet was lost), the drive's blocks by DMA both ways (halves of the staging buffer: the card reads or writes one half while USB moves the other), the UDC interrupt as the waiting loader's wake-up | image loading (921,600-byte frame) / drive write, 16 MiB / drive read, 16 MiB / idle round trip, median (max) | 514 KB/s / 411 KB/s / 765 KB/s / 1,170 us (4,271) | **1,048 KB/s / 856 KB/s / 1,146 KB/s / 292-334 us (4,372-4,730)**; 80 MiB written and read back identical in the soak, Windows' inbox drivers only |
| 2026-10-16 | The drive's staging buffer: 2 x 32 KiB halves (64 KiB), 2 x 16 KiB (32 KiB), 2 x 8 KiB (16 KiB). Per 64 KiB write command the first half's arrival and the last half's card write can't overlap anything (the PC waits for the status), so smaller halves win until the card's per-command cost takes over; reads stay at the USB ceiling (read-ahead). 32 KiB kept: fastest, and half the memory of 64 | drive write / read, 16 MiB | 800 / 1,146 KB/s (32 KiB halves) | 856 / 1,146 KB/s (16 KiB); 763 / 1,146 (8 KiB) |
| 2026-10-16 | UDC DMA, raw (the loader's bring-up experiment, since removed: 64 KiB through the serial endpoints, the CPU only arming and waiting, every byte checked on the PC) | 64 KiB OUT / IN | CPU path 1,049 / 765 KB/s | 1,070 / 1,149 KB/s |
| 2026-10-16 | `rp-loader` with the UDC work (driver +2.4 KB, the drive's DMA paths, `udc stats`, the frame timeout); a third DMA chain (+512 B zeroed RAM in every image using DMA) | `rp-loader` loaded / zeroed | 106,648 / 19,536 B | 110,860 / 20,096 B (inside 112 / 20 KiB) |
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
| 2026-10-11 | `rp-loader`: SD card writes and FAT32 copy-then-swap (ADR-0013): sdcard + MMC transmit path + `rp-fat` + the reply line. **Ratcheted 22 → 41 KiB** (test image, owner's rule of 2026-10-11) | `rp-loader` loaded size | 22,348 B | 36,870 B |
| 2026-10-11 | `rp-probe`: lists the card's FAT32 root directory (the FAT reader meets the real card read-only first). **Ratcheted 45 → 53 KiB** (test image) | `rp-probe` loaded size | 44,216 B | 48,636 B |
| 2026-10-11 | `rp-probe` writes its report to the run log and returns to the resident loader (ADR-0012), so `cargo xtask load rp-probe` delivers the checksummed report to the PC with no photos. **Load budget raised 40 → 45 KiB, approved by the owner** (measured + ~10%) | `rp-probe` loaded size | 40,864 B | 41,920 B |
| 2026-10-11 | Pattern fill builds its bar row in RAM and only writes the framebuffer (the device measured reading 2700G memory as the slow part: 61 ms vs 14.5 ms for a plain fill). Emulator counts instructions, so no gain shows there; the device number comes with the next probe run | `fb_pattern_vga` (emulator) | 127,692 ns | 128,821 ns (+0.9%, baseline re-recorded) |
| 2026-10-11 | SD: 4-bit data bus (ACMD6 + `CMDAT.SD_4DAT`) and multiple-block reads (CMD18/CMD12). New benchmark reads 4 blocks per command; on the device the per-command start-up (most of the measured 3.85 ms per block) is paid once per 4 | `sd_read_4_blocks` (emulator) | — | 17,769 ns |
| 2026-10-11 | **Measured on the device** through the USB loader (no photos): 4-bit bus and 4 blocks per command | `sd_read_4_blocks` / 4 vs `sd_read_block` | 3,904,480 ns per block | 1,164,134 ns per block (**3.4×**) |
| 2026-10-11 | **Measured on the device**: pattern rows built in RAM, framebuffer only written | `fb_pattern_vga` | 61,016,717 ns | 14,480,000 ns (**4.2×**; now equal to the solid fill) |
| 2026-10-11 | `CTL NOW 00003A7F` on the device: MMU, D-cache, I-cache and BTB on while the benchmarks run (so the bus-latency reading of `timer_read` stands). The CPU reported turbo 624 MHz this time (520 earlier): Windows Mobile's choice at boot, not ours | — | — | — |
| 2026-10-11 | `rp-hello` **load budget raised 20 → 22 KiB, approved by the owner**: the HAL code every image shares grew through the day (DMA stop at start-up, fault diagnostics, aligned MMU transitions, the user-fault path); hello was 4 bytes from its ceiling and had been trimmed four times to stay under | `rp-hello` loaded size | 20,476 B | 20,476 B (budget 22,528) |
| 2026-10-11 | FAT free-cluster search in runs (`rp-fat`): the FAT is read through the caller's scratch, one sector first and doubling up to the scratch (64 KiB in the loader), instead of one sector per command. Emulator `boot_loader_put` (16 MiB card, ~88% taken behind the FSInfo hint), read commands for the whole 70 KB write (the scan itself ~28 → 5). Host test (32 MiB, 513 FAT sectors): 479 → 19. `rp-loader` 38,372 → 39,400 B (the scan plus the read count in its reply), inside its budget | `boot_loader_put` read commands | 35 | 12 (**2.9×**) |
| 2026-10-11 | ADR-0010: mutexes with priority inheritance, user faults in the thread's context, lazy iWMMXt, interrupts as waitable objects, plus their live tests. Scheduler entry points kept out of line; free events via a bitmap instead of a loop LLVM unrolled to 1 KiB. **Load budget raised 17 → 27 KiB, approved by the owner** (measured + ~10%) | `rp-threads` loaded size | 16,012 B | 24,500 B |
| 2026-10-11 | ADR-0010 rule 8: KData written on every switch (3 words through its one user address, no cache maintenance), the top MiB split into pages, TLS blocks on user stacks, and the `USER KData` test. Paid for in part by one shared user-thread trampoline instead of five (−576 B) and an opaque length that stopped LLVM unrolling a 32-line cache clean (−320 B). Inside the 27 KiB budget, no raise. Zeroed RAM 28,968 → 29,496 B (a TLS word per thread). `thread_switch` smoke number unchanged within noise (emulator real time, ticks per 2,000 switches: 218,114 before, 204,689 and 222,276 in two runs after); `cargo xtask bench` flat | `rp-threads` loaded size | 24,564 B | 26,092 B |
| 2026-10-15 | MBX 3D core: render only the changed tiles (region array lists them; ADR-0026 D2, device test T1) instead of the whole screen as Dell's driver does | 3D render of a 100-tile change, real X51v | 12,484 us (whole screen) | 528 us (**23.6x**; 1 tile 16 us) |

| 2026-10-12 | `rp-loader`: the SD card as a USB drive (ADR-0014). **Ratcheted 41 → 52 KiB** (test image): composite descriptors, the Bulk-Only/SCSI stack (2.6 KiB `Msc::poll`), card capacity from the CSD; the 32 KiB transfer buffer is in the scratch area, not the image | `rp-loader` loaded size | 39,400 B | 47,743 B |

| 2026-10-12 | `rp-loader`: SD read recovery (timeouts, slow blocks, weak ranges), drive diagnostics, file creation in `rp-fat`. **Ratcheted 52 → 58 → 70 KiB** (test image; reasons in `xtask/src/size.rs`). `rp-bench` 24 → 27 KiB for the same SD code | `rp-loader` loaded size | 47,743 B | 63,447 B |

| 2026-10-12 | SD DMA for multi-block reads (`hal/pxa27x` DMA channel, `PxaMmc::with_dma`): the CPU arms a descriptor chain and waits, instead of reading the FIFO a byte at a time. Emulator numbers are instruction counts, so this is the CPU's share of the work; the device's wall-clock gain is still to measure (the probe now records a 64-block programmed-I/O row to compare against) | `sd_read_4_blocks` | 13,635 ns | 1,250 ns (**10.9×**) |
| 2026-10-12 | (same change, new row: 32 KiB, a cluster's worth, 8 descriptors) | `sd_read_64_blocks` | — | 12,923 ns |
| 2026-10-12 | `rp-bench` **ratcheted 27 → 30 KiB** (test image): the DMA channel, the controller's DMA path and the 64-block row | `rp-bench` loaded size | 26,608 B | 28,764 B |

| 2026-10-12 | `rp-loader` built at opt-level "s" (ADR-0005 rule 8: it is cold code). The generic drivers and file systems are compiled in the loader's crate, so their own crates' "s" never applied there. Emulator drive and put timings unchanged within noise (1 MiB read 2,792 → 2,836 ms) | `rp-loader` loaded size | 104,427 B | 91,904 B (−12%) |
| 2026-10-12 | exFAT writes: free space from the allocation bitmap instead of a FAT scan (emulator `boot_loader_put_exfat`, 70 KB on a 16 MiB card) | read commands for the put | 12 (FAT32, same file) | 8 |
| 2026-10-12 | `rp-loader` **ratcheted 87 → 96 KiB** (test image): exFAT read and write, `AnyVolume` | `rp-loader` loaded size | 86,043 B | 91,904 B |

| 2026-10-12 | Audio by DMA: the loader's 500 ms tone streamed from an 8 KiB ring the CPU refills by halves, instead of polling the I2S FIFO (emulator; device pending) | CPU busy during playback | 100% | <1% |

| 2026-10-12 | The loader sleeps between polls after 50 ms of quiet (1 ms tick, XScale idle mode) | Waiting loader asleep (emulator) | 0% | 99% (answers in 1-2 ms; the drive still reads 1 MiB in ~3 s) |

| 2026-10-12 | ADR-0015 step 2: the loader built as Thumb (the HAL's assembly marked ARM per function; global assembly already `.arm`), every emulator test passing; budget lowered 128 -> 101 KiB | `rp-loader` loaded size | 127,664 B | 94,208 B (−26%) |
| 2026-10-12 | IPC (ADR-0016 step 2) live in `rp-kernel`: a client process calls a server process through a port, the thread migrating (FCSE PID + domain switch, registers saved and cleared) and back; 1,002 calls including a faulting server and a refused handle. Emulator real time (no `-icount`), so a smoke number until the device runs it | `ipc_call_roundtrip` | — | ~12 µs per call (39,104 ticks of 3.25 MHz / 1,002, emulated) |
| 2026-10-12 | IPC in `rp-kernel`. Handle tables one packed word each (no `Option`, no padding) and only for CE5's 31 process slots, not all 63 FCSE PIDs: 9.9 → 5.7 KB of zeroed RAM, inside the old budget. The user test programs copied into a frame at start instead of a page-aligned page in the image (−7.6 KB of padding; no kernel-image page is user-visible). `rp-kernel-ipc` built for size (its table scans unrolled to 0.8 KB at opt-level 3). **Kernel load budget raised 38 → 46 KiB, approved by the owner** (measured + ~10%) | `rp-kernel` loaded / zeroed | 34,928 / 105,456 B | 42,312 / 111,128 B |
| 2026-10-12 | Thread contexts kept on each thread's own kernel stack (`push {r4-r11, lr}`; only sp in the table): one word per thread instead of ten. Every threads and kernel test passes on it | `rp-threads` zeroed RAM | 29,496 B | 24,888 B (budget lowered 30 -> 27 KiB) |
| 2026-10-12 | The scheduler in `rp-kernel` (ADR-0010) with two client threads in one server, preempted inside calls; IPC caller frames dropped (the caller's slot waits in its kernel stack frame), ports without `Option`. Thumb for `main` tried: 51.5 -> 53.4 KB (helpers outlined), reverted. **Budgets raised 46 -> 55 KiB load, 112 -> 120 KiB zeroed, approved by the owner** | `rp-kernel` loaded / zeroed | 42,376 / 111,368 B | 51,324 / 115,644 B |
| 2026-10-12 | Two client threads calling one server at once (1,011 calls each incl. nested), with switches between them. Emulator real time | `ipc_call_roundtrip` (2,022 calls) | ~13 µs per call | ~17 µs per call incl. switches (113,091 ticks of 3.25 MHz) |
| 2026-10-12 | PE images loaded into processes (ADR-0009) in `rp-kernel`: parser, per-page load + relocation, page-cache fill, W^X; plus the test's image writer. `rp-kernel-pe` built for size (-0.7 KB). **Kernel load budget 55 -> 65 KiB, owner's standing rule** (planned subsystem, measured + ~10%) | `rp-kernel` loaded size | 51,324 B | 60,328 B |
| 2026-10-12 | Reclaim sweep in `rp-kernel`: 16 pages visited, 8 re-aged (write-back + entry change) and 8 compressed. Emulator real time | `vm_reclaim_sweep` (16 pages) | — | 1,042 ticks: ~20 µs per page (emulated) |
| 2026-10-12 | Supervised hosts and the service manager (ADR-0016 step 3) in `rp-kernel`, with two more test processes (their static address-space records: 1.6 KB each; to be allocated per process, item 44). **Kernel load budget 65 -> 76 KiB, owner's standing rule** | `rp-kernel` loaded size | 63,504 B | 70,732 B |
| 2026-10-12 | Processes at run time in `rp-kernel` (SYS_SPAWN/SYS_EXIT, a 5-record process table of 1.6 KB records in initialised data). **Kernel load budget 76 -> 84 KiB, owner's standing rule** | `rp-kernel` loaded size | 70,732 B | 78,432 B |
| 2026-10-12 | ADR-0019: regions in per-process pages (records 1.6 KB -> ~60 B), then the 300-region emulator check. **Kernel load budget 84 -> 82 KiB** | `rp-kernel` loaded size | 82,636 B | 76,672 B (78,500 B with the check) |
| 2026-10-12 | Pixel paths (`rp-gfx`, item 52): full-screen fill through `fill32` instead of `[u16]::fill` | `gfx_fill_vga` (emulator, instructions) | 928,718 | 96,103 |
| 2026-10-12 | Pixel paths: full-screen alpha blend, one multiply for three channels vs per channel | `gfx_blend_fill_vga` vs `gfx_blend_reference_vga` (emulator) | 7,987,231 | 3,079,077 |
| 2026-10-12 | Loader boot time, first measured: the bring-up beacon filled all 16 MiB of 2700G memory four times (now 1 MiB, where a plane lies); GPIO, card, DMA self-test and USB detach moved inside the 500 ms display settle. Emulator real time | `boot_loader_ready` | 742 ms | 504 ms |
| 2026-10-12 | Kernel: entry to its own page tables and device map. Emulator real time | `boot_kernel_tables` | — | 2-3 ms |
| 2026-10-15 | Backlight (item 76): prescale 0 and 432 counts instead of Windows Mobile's prescale 1 and 216, the same 30.09 kHz. The display host grows 12 B (1,244 of its 1,280 B budget; the take-over written to fit) | dimmest light step, fraction of full / display host loaded size | 1/216 / 1,232 B | 1/432 (then the picture down to 1/8 of it, once the gamma table's format is known) / 1,244 B |
| 2026-10-17 | Device: the loader's boot after the CF warm attach (a slot found powered with its card is reset, not power-cycled; docs/hw/cf.md), measured by `handoff` after a soft reset into the loader | `boot_loader_ready` / the 2700G taken | 629 ms / 583 ms | 347 ms / 152 ms |
| 2026-10-16 | Device: the loader after HaRET's hand-off with no fixed wait (three soft resets) and an image started by it (`rp-handover`). The loader's pre-display work takes ~585 ms on the device (emulator ~200 ms): open question, docs/hw/2700g.md | `boot_loader_ready` (`handoff`) / an image's 2700G | — | 623-627 ms / 0.37 ms |
| 2026-10-15 | The 2700G's fixed 500 ms settle after a hand-off replaced by waiting for the DMA channels the previous owner left running to stop (the hang it guarded against was that DMA, found the same day it went in; docs/hw/2700g.md "After the hand-off"). The loader is now held by its 200 ms USB detach; every image the loader starts reaches the 2700G at once. Budget 560 -> 260 ms. Emulator real time; device numbers below when measured | `boot_loader_ready` / an image's 2700G (`rp-handover`) | 504 ms / ≥ 500 ms | 204 ms / 0.25 ms |
| 2026-10-12 | **Measured on the device**: SD DMA self-test passed on the real X51v (reads on, 120 transfers); drive 7,377 blocks read, 0 errors, 0 retries | SD DMA on device | pending | passed |
| 2026-10-12 | **Measured on the device**, the owner's new 32 GB SDHC card (FAT32, 32 KiB clusters) against the old 4 GB card: the first SDHC card on the real X51v, 4-bit bus. Single-block commands much faster; through the file layer both cards are at ~1.4-1.9 MB/s, so our driver (no DMA writes yet, small commands) is the limit, not the card | `sd_read_block` / `sd_read_4_blocks` per block / put `haret.exe` 143 KB write, read back | 3.90 ms / 1.16 ms / 85 ms (once 811 ms), 83 ms | 0.89 ms / 0.42 ms / 100 ms (twice), 83 ms; `sd_read_64_blocks` 0.27 ms per block |
| 2026-10-12 | Plane blending (item 60): the glass-over-wallpaper look mixed by the 2700G during scan-out instead of by the CPU. `rp-blend` test image budget 22 KiB (test image rule: measured + ~10%); its three 480 × 640 test scenes are generated on the device, not stored | `rp-blend` loaded size; CPU work per frame for the glass look | — | 19,624 B; 0 (the display does it); the wallpaper as 4:2:0 YUV is 450 KiB vs 600 KiB as RGB565 |
| 2026-10-12 | Crash boxes drawn on the plane actually on screen (`Display::on_screen` decodes the 2700G's plane registers; on the device Windows Mobile's plane is at offset 0, so boxes drawn at `0x60000` never showed). Budgets ratcheted (test image rule): `rp-hello` 24 → 26, `rp-usb` 20 → 22, and `rp-crash` 8 → 14 KiB (it now draws the shared crash box, on a plane it moves to offset 0, for the emulator check) | `rp-hello` / `rp-usb` / `rp-probe` / `rp-crash` loaded size | 24,498 / 20,381 / 66,016 / 7,201 B | 24,738 / 21,001 / 66,596 / 13,161 B |
| 2026-10-19 | `rp-gpubench`, a new test image (the 3D compositor proposal's GPU facts, docs/design/ui.md: runs A and B). Budget 106 KiB (test image rule: measured + ~10%) | `rp-gpubench` loaded size | new | 98,164 B |
| 2026-10-15 | `rp-mbx3d-api`, a new test image (ADR-0026: RetroPane's own 3D renderer, `rp-mbx3d-render`, through its API; the renderer itself ~10 KiB of code, ~4.5 KiB of state). Budget 56 KiB (test image rule: measured + ~10%) | `rp-mbx3d-api` loaded size | new | 51,972 B |
| 2026-10-15 | `rp-mbx3d-api` after run 3: S8 removed (its question answered). Budget tightened 70 -> 66 KiB | `rp-mbx3d-api` loaded size | 64,776 B | 61,404 B |
| 2026-10-15 | `rp-mbx3d-api` run 3 (S8: the depth "less" exploration; exact render times beside the prediction). Budget 56 -> 70 KiB (test image rule: measured + ~8%) | `rp-mbx3d-api` loaded size | 53,416 B | 64,776 B |
| 2026-10-15 | ADR-0026 render-time prediction fitted to `rp-mbx3d-api` run 2's exact times: 10 us + 5.25 us a tile + per quad a tile 0.5 us flat, 1 us textured, 5 us blended (a blended quad costs about a tile) | prediction error, 11 measured frames | 16 us + 5.2 us a tile: up to -37% (additive 3,387 us predicted 2,128) | within 6% (a test pins it) |
| 2026-10-15 | ADR-0026 renderer, host model: a frame after the first changing one 8 x 16 tile's worth of scene vs Dell's driver (whole screen, ~29 register writes through calls, CPU spinning) | bus writes per frame (registers + words to 2700G memory) / render time / status reads | ~29 registers + full region array (4,800 words), 12.5 ms, spinning | 1 register + 34 quad words + 2 a tile; ~5.2 us a tile (version 19); 1 read at the predicted time (device numbers wait for the `rp-mbx3d-api` run) |
| 2026-10-14 | `rp-mved`, a new test image (ADR-0025 S0-S2: the MVED1 video decoder's clock, reset and frame conversion). Budget 31 KiB (test image rule: measured + ~10%); its log prints hex by hand instead of `{:?}`, and both conversions share one runner: 9,139 B less. The test frame and both destinations live in 2700G memory (24 + 8 + 12 KiB at 4 MiB), none in system RAM; the screen panels are read straight from it, not copied to the stack | `rp-mved` loaded size | new (37,524 B with Debug formatting) | 28,385 B |
| 2026-10-15 | `rp-clocks`, a new test image (ADR-0031: every operating point, turbo and relock costs, clocks put back). Budget 32 KiB (test image rule: measured + ~10%) | `rp-clocks` loaded size | new | 30,677 B |
| 2026-10-15 | `rp-clocks` version 3 (after run 1 never returned: memory-clock-keeping points only, lowest risk first, a breadcrumb per step on screen and in 2700G memory). Budget 32 -> 36 KiB (test image rule) | `rp-clocks` loaded size | 30,677 B | 32,657 B |
| 2026-10-14 | `rp-mbx3d` version 2 (the parameter memory set-up, the 3D clock vote, two start-table tries). Budget 22 -> 25 KiB (test image rule: measured + ~10%) | `rp-mbx3d` loaded size | 20,401 B | 23,145 B |
| 2026-10-14 | `rp-mbx3d`, a new test image (ADR-0024 M4: the MBX 3D core's first triangle). Budget 22 KiB (test image rule: measured + ~10%) | `rp-mbx3d` loaded size | new | 20,401 B |
| 2026-10-15 | `rp-noise` version 4: the 32 kHz ticks taken as interrupts and timestamped on IRQ entry (`rt::last_irq_cycles`, +16 B in `rp-kernel`), plus a spaced series. Budget 104 -> 117 KiB (test image rule) | `rp-noise` loaded size | 97,517 B | 109,085 B |
| 2026-10-15 | `rp-noise`, a new test image (ADR-0029: the RNG's noise sources measured, samples to the card). Budget 104 KiB (test image rule: measured + ~10%); most of it is `rp-fat`'s FAT32 and exFAT writes | `rp-noise` loaded size | new | 96,557 B |
| 2026-10-14 | `rp-today` on the GPU renderer: first device run 33 ms median / 80 ms worst touch-to-screen (CPU renderer: 30 / 71); stack 12.6 of 16 KiB. Its stack becomes 24 KiB; zeroed-RAM budget 20 -> 30 KiB (test image rule: measured + ~10%) | `rp-today` zeroed RAM | 18,544 B | 26,736 B |
| 2026-10-14 | Every image reports its deepest SVC stack on returning to the loader (`rt::stack_used`, the stack painted at start-up): a silent overflow of the 16 KiB stack lost rp-today's glyph atlas in the emulator. `rp-hang` budget 9 -> 10 KiB (test image rule) | `rp-hang` loaded size | 8,329 B | 9,269 B |
| 2026-10-14 | `rp-mbx-blend` adds B6 (ADR-0023: source stride 0, blending onto 32 bpp against the reference, an ARGB4444 destination, a 32 bpp screen with the engine busy). Budget 42 -> 51 KiB (test image rule: measured + ~10%) | `rp-mbx-blend` loaded size | 37,705 B | 47,233 B |
| 2026-10-14 | `rp-mbx-blend`, a new test image (renderer step 1, ADR-0022): the blend-state and display-event probe; the 2D command words move from `rp-mbx` into `rp-drv-blit2d`, shared. Budget 42 KiB (test image rule: measured + ~10%) | `rp-mbx-blend` loaded size | new | 38,281 B |
| 2026-10-15 | The time slice (ADR-0010 amendment): 20 ms quantum and a 1 ms wake slice instead of CE's 100 ms, measured with `rp-quantum` on the device: touch to screen with a busy equal-priority program 73.5 / 101.7 ms (median / worst) -> 5.1 / 5.1; 60 Hz frames on time 0 -> 35 of 35; never-blocking loop's worst gap 105 -> 25 ms; a busy pair keeps 99.5% of its work (~100 us per switch with cache refills). Cost: `thread_switch` 86,708 -> 88,868 ticks per 2,000 (13.3 -> 13.7 us, two device runs each); kernel +424 B code, +16 B RAM | touch to screen, worst (device) | 101.7 ms | 5.1 ms |
| 2026-10-15 | Process and thread limits (owner's vote, docs/research/process-limit.md): native slots 96-126 above the kernel (62 processes, CE's layout untouched), handle tables 8 inline + pages on demand (quota 4,104), 255 threads. One out-of-line VM+IPC locking trampoline instead of a copy per call site (-~3 KB). Inside the kernel's 95 KiB load budget, no raise; `rp-threads` and `rp-quantum` zeroed budgets 27 -> 31 KiB (test image rule) | `rp-kernel` loaded / zeroed | 92,128 / 115,948 B | 95,424 / 119,300 B |
| 2026-10-15 | `rp-quantum`, a new test image (ADR-0010 amendment): eight time-slice policies' touch, frame, busy-loop and switch-cost tests. Budget 34 KiB (test image rule: measured + ~10%) | `rp-quantum` loaded size | new | 30,732 B |
| 2026-10-13 | `rp-mbx` becomes the first GPU-rectangle test (item 69: MBX 2D engine commands, fence, edge read-back, GPU vs CPU timing); it now links the display driver to adopt the screen and the CPU fill to time against. Budget 12 -> 19 KiB (test image rule: measured + ~10%) | `rp-mbx` loaded size | under 12,288 B (ID reads only) | 17,077 B |
| 2026-10-13 | `rp-mbx` adds the GPU copy checks (item 69 step 2: from off-screen memory, overlapping right and down, colour-keyed, ten full-screen copies timed). Budget 19 -> 27 KiB (test image rule: measured + ~10%) | `rp-mbx` loaded size | 18,205 B | 24,297 B |
| 2026-10-13 | `rp-mbx` adds DMA from SDRAM into 2700G memory (timed against the CPU, with the CPU's spare pace) and the probe of local addresses past 16 MiB (ADR-0022 open questions 1-2); the HAL gains memory-to-memory DMA. Budget 27 -> 38 KiB (test image rule) | `rp-mbx` loaded size | 24,297 B | 34,825 B |

## Real hardware

### SD card writes (2026-10-11, `cargo xtask install`, ADR-0013)

| File | Bytes | Write | Read back | Clusters |
|---|---|---|---|---|
| `retropane.bin` (loader) | 36,870 | 36 ms | 23 ms | 10, contiguous |
| `startup.txt` | 116 | 14 ms | 1 ms | 1 |
| `haret.exe` | 142,848 | 85 ms (811 ms on a second run: the card's own housekeeping) | 83 ms | 35, contiguous |

About 1.7 MB/s each way through the FAT layer, multi-block commands, 4-bit
bus; write times vary with the card's internal erase/garbage collection. The
free-cluster scan after a stale FSInfo hint read the FAT one sector per
command (3.7 s once on this card: 1,490 commands). It now reads the FAT in
runs (one sector, doubling to 64 KiB per command): ~18 commands for the
same scan, so it should be bound by the ~0.75 MB transfer (~0.4 s).
Open lead: measure it on the device (the loader's reply now gives the
write's read commands); if the transfer still matters, keep a free-cluster
summary in RAM after the first scan instead of re-reading the FAT.

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
- **`timer_read` 846 ns** is one OS-timer read (the loop adds a few cycles):
  a peripheral read crosses the system and peripheral bus bridges. Together
  with ~506 ns per SDRAM miss (`chase_sdram_256k` / 4096 hops) and ~117 ns
  per internal-SRAM hop, this looks like the PXA270's real bus latencies, not
  a cache problem: memcpy's ~190 MB/s of traffic is consistent with caches
  on. **Consequence: read the timer sparingly** (the scheduler's tickless
  design already does). The probe now also reports CP15 control after the
  MMU is on (`CTL NOW`; `0000387D` in the emulator) to confirm caches on the
  device rather than infer it.
- **LZ4**: a 4 KiB heap page decompresses in 180 µs (~23 MB/s), so a
  compressed-RAM fault costs ~0.2 ms. Acceptable, but the decoder is the first
  thing to optimise on real hardware.
- **Answered (2026-10-15):** the probe hung before drawing anything until the
  startup colours (about 1 s of extra delay) were added. The cause was
  Windows Mobile's DMA still running after HaRET's hand-off (stopped by
  `_start` since 2026-10-11); the 500 ms settle that stayed in its place is
  gone (docs/hw/2700g.md "After the hand-off").

### Threads on the real X51v (2026-10-12, `cargo xtask load rp-threads`)

All 13 scheduler and user-mode tests pass on the device (ADR-0010: ping-pong,
interrupt thread, priority inheritance, sleep, user mode, yield, preemption,
contained user fault, lazy iWMMXt, KData). OS timer ticks at 3.25 MHz:

| Measure | Device | Per unit |
|---|---|---|
| `thread_switch` (2,000 switches: event set, wait, schedule, register switch, KData) | 79,385 ticks | **~12.2 µs per switch** |
| `irq_to_thread_max` (interrupt to the waiting thread running) | 26 ticks | ~8 µs |
| `sleep_lateness_max` | 56 ticks | ~17 µs |

> **Note (2026-10-15):** the same image measured 86,708 ticks (13.3 µs per
> switch) on 2026-10-15, steady over two runs, before the wake slice; 88,868
> (13.7 µs) with it (ADR-0010 amendment). The 12.2 µs above was another
> day's clock state.
> **Note (2026-10-15, later):** with 255 threads and the native process
> slots: 86,924 ticks (13.4 µs). `irq_to_thread_max` had read 4,294,960,808
> (a wrapped negative: the thread woke ~2 ms before its timer was due)
> because an earlier image left OS timer channel 2 armed and its match
> fired when the line was claimed; the HAL now hands the timer out
> quiesced, and the next run read 38 ticks (~12 µs).

### The time slice on the real X51v (2026-10-15, `cargo xtask load rp-quantum`)

Touch to screen, 60 Hz frames and switch cost against a busy thread at the
interface thread's priority, for eight policies: the table is in ADR-0010,
"Amendment: the time slice". Chosen: a 20 ms quantum with a 1 ms wake slice
(touch 5.1 ms worst, the frame's own time; 35 of 35 frames on time; a busy
pair keeps 99.5% of its work). A switch between two threads with 16 KiB
working sets costs ~100 µs in all, of which 13 µs is the register switch.

### Windows Mobile baseline (2026-10-13, `rp-wmbench` v2 and v3, ROADMAP Phase 0)

The same rows measured under the stock Windows Mobile 6.1 (CE 5.2 build
20273) on the owner's X51v, by `tools/wmbench` (`cargo xtask wmbench`; run
by hand, log `rp-wm-baseline.txt` next to it). Every result feeds a sink
the compiler can't drop; the clock is measured after a 2 s warm-up (WM
idles at ~135 MHz and raised it to **624 MHz turbo** under load); card
reads come after a second 4 MB file evicts the first from WM's cache. v1's
memcpy, memset and SHA rows were optimised away and are discarded. v2's
drawing rows timed only the queueing of work for the 2700G's 2D engine
(~0.1 ms "per fill"); v3 reads a pixel back to wait for the engine, and
its rows replace v2's. Where v2 and v3 differ on CPU rows, both are given
(run-to-run noise).

RetroPane's rows are the earlier device runs above, at **520 MHz** (WM's
choice at that boot), so CPU-bound rows favour WM by up to 1.2x; a rerun
at the same clock is due (item 67 sets 624 MHz ourselves).

| Row | Windows Mobile | RetroPane | Reading |
|---|---|---|---|
| `timer_read` | 1,769 ns (`QueryPerformanceCounter`) | 846 ns (OS timer) | RP 2.1x: no kernel call |
| `memcpy_64k` | 606,503 / 736,116 ns (~89-108 MB/s) | 687,430 ns (~95 MB/s) | level; memory-bound, try iWMMXt and `pld` tuning (item 71) |
| `memset_64k` | 179,200 / 172,466 ns | 173,953 ns | level |
| `chase_sdram_256k` (per hop) | 291 / 275 ns | ~506 ns | **WM 1.8x**: SDRAM timings, memory clock or TLB misses (RP's 4 KiB pages); items 68, 78 |
| `sha256_4k` | 441,573 / 481,029 ns (~8.5-9.3 MB/s) | not yet on the device | - |
| `thread_switch` | 9.0 / 5.6 us (event ping-pong, per switch) | ~12.2 us | **WM 1.4-2.2x**: ADR-0010's target is < 2 us; items 67, 80 |
| `sleep_lateness_max` | 1,100 / 1,052 us (1 ms tick) | ~17 us (tickless) | RP ~60x |
| `fb_solid_fill_vga` (CPU writes) | 18,981,615 / 19,003,800 ns (plain word loop) | 14,473,743 ns (`stm` bursts) | RP 1.3x; both bound by the 2700G's bus (~32-41 MB/s) |
| `gdi_fill_screen` (v3, finished) | 3,191,892 ns (~190 MB/s) | - | **the 2700G's 2D engine: 4.5x RP's CPU fill**, and the CPU is free while it runs |
| `gdi_fill_screen_each` | 4,927,400 ns | - | one fill and a wait each time: ~1.7 ms of call and read-back overhead per operation |
| `gdi_fill_colour_screen` | 3,228,861 ns | - | a coloured brush: no black/white shortcut |
| `gdi_bitblt_screen` (v3) | 5,065,815 ns (6.1 ms waiting each) | - | full-screen copy from an off-screen bitmap: 2D engine, ~120 MB/s |
| `gdi_text_line` | 2,513,277 ns | - | 54 characters, opaque: font rendering on the CPU |
| SD write / read, 4 MB | 840 / 2,987 KB/s | write ~1.4-1.7 MB/s (`put`) | RP's writes 2x already; reads not yet measured the same way |
| CF write / read, 4 MB | 192-240 / 1,921 KB/s | not yet | WM's CF writes are slow: item 75 |

What it says:

- **The 2700G's 2D engine fills the screen ~4.5x faster than the CPU
  can** (3.2 ms against RetroPane's 14.5 ms; ~190 MB/s against the CPU's
  ~41 MB/s over the bus), and runs while the CPU does other work. WM uses
  it; RetroPane doesn't yet. A real argument for the GPU-first track
  (items 69, 70), though not the 140x v2 suggested.
- **Per-operation overhead matters:** waiting for each fill costs ~1.7 ms
  more. Many small draws through a call-and-wait API lose most of the
  engine's gain; batching operations and fencing once per frame keeps it.
- **WM beats RetroPane where RetroPane hasn't been tuned yet:** SDRAM
  latency (1.8x) and thread switches (1.4-2.2x, noisy). Both are now
  targets with a number to beat, not guesses.
- **RetroPane already wins where the design differs:** tickless timers
  (~60x lower sleep lateness), direct timer reads, SD writes.
- **Lesson (the class, not the instance):** time asynchronous work only to
  a completion barrier. v2 timed the 2D engine's queue; RetroPane's DMA
  rows (SD, tone) already wait for completion, and every GPU row from
  item 69 on must too.
### GPU fills (2026-10-13, `cargo xtask load rp-mbx`, item 69)

| Full-screen RGB565 fill (480 × 640) | Per fill | CPU busy |
|---|---|---|
| CPU (`fill_rect_rgb565`, burst stores) | 14.5 ms | 14.5 ms |
| MBX 2D engine (RetroPane, bare metal) | 3.1 ms | 36 µs for 20 fills (queueing only) |
| MBX 2D engine (Windows Mobile `PatBlt`, baseline above) | 3.2 ms | ~0.1 ms |

RetroPane now matches WM's fill speed; the CPU is free while the GPU
draws. Details and edge checks: docs/hw/2700g.md, "The MBX 2D engine".

### Slow frames around the Start menu (2026-10-14, `rp-today`)

The GPU Today screen's worst taps (one of 57 at 101 ms) came from frames
of 24-29 ms of CPU, in pairs, as the Start menu opened and the next
frame drew. The CPU time is waiting for FIFO room while the 2D engine
works: no atlas builds, uploads or mid-frame waits (counted on the PC,
`rp-today`'s `menu_frames_cost`). Opening the menu covers ~140,000 px
three times (background copy, glass blend, dither) plus text. The frame
after drew it all again: the picture drawn into is two frames old.

- **Fix 1 (`rp-mbx-render`): copy the previous frame's changes forward**
  from the picture on screen, one plain copy. On the PC: 1,851 command
  words to 147. On the device (log `local/rp-today-menu-fwd.txt`): slow
  frames 8 to 3 in a similar session, one per menu opening, none after;
  touch to flush median 19.0 ms, worst 54 (101); queue to flip median 3.0
  ms (5.9).
- Left: the menu opening itself (~24 ms of CPU waiting on the engine);
  and the first tap after start-up (the second full-screen frame after
  the background is built: 14.7 ms of CPU, 23.7 of engine), which a
  full-screen copy forward would also cover.
- **Fix 2: an area cache** (1.17 MiB of 2700G memory, approved): an
  area of 16K px or more is kept the second time it's drawn and copied
  in when its look (background, commands, text, dither: hashed) is the
  same; and the frame after a whole one copies the screen forward. On
  the PC: the menu opening or closing from the third time, 127 words
  instead of ~1,840. On the device (log `local/rp-today-menu-cache.txt`,
  the menu opened and closed 5-6 times): 3 slow frames, all before the
  cache held the menu, none after; the first tap after start-up not slow;
  touch to flush median **13.0 ms**, worst **42** (fix 1: 19.0 and 54;
  before: 20.4 and 101).

## Interface layer flush (2026-10-12, emulator)

`rp_gfx::layer::flush` converting a 480 x 640 ARGB8888 layer to the
dithered ARGB4444 graphics plane (`cargo xtask bench`, instructions):

| Version | Full screen | Clock-sized |
|---|---|---|
| Per-pixel reference (`to_4444_dithered`, byte writes) | 16.6 ms | - |
| Two channels per lane word, no divides | 11.9 ms | 0.098 ms |
| + four pixels a step (thresholds fixed), whole-word writes | **9.5 ms** | **0.080 ms** |
| ARGB8888 plain copy, for comparison | 0.22 ms | - |

The emulator doesn't charge for the 2700G's slow memory; on the device
writes dominate (a 16-bit screen took 14.5 ms to write), so ARGB8888's
twice-as-many bytes cost more there than the conversion saves. Device
numbers: rp-blend's run log (`draw F`). Next: iWMMXt (one pixel's four
channels in one 64-bit register).

## Open leads

- **Thread switch ~12 µs on the device vs ADR-0010's < 2 µs target.**
  Suspects to measure one at a time: the CPU clock as Windows Mobile left
  it (the probe reads CCCR/CLKCFG), the per-switch KData write and the
  cache work around it, the event path rather than the register switch
  itself (split the benchmark). The emulator can't answer this; the device
  run takes seconds now.

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
