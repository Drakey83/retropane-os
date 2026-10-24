# The CPU's full potential, and the CPU, 2D engine and 3D core together (2026-10-20)

> **GATE (the owner, 2026-10-21): nothing moves forward until this is done.**
> No new phase, feature or design work until every clever software trick has
> been tried to get the most out of the 2700G's 3D core, its 2D engine, the
> PXA270, and all of them together: every line of
> [2700g-full-potential.md](2700g-full-potential.md) and
> [cpu-full-potential.md](cpu-full-potential.md) ticked with device
> evidence (used, or declined with the reason). Meanwhile only finishing and
> proving work already built, fixes and the quality audit, and the
> measurements these pages need. Each summary gives the count of lines
> ticked and open.

**The owner's brief (2026-10-20):** the PXA270 gets the same treatment as
the 2700G ([2700g-full-potential.md](2700g-full-potential.md)): fully
tested and understood, for the best performance it can give, **in spite
of what Dell did**. RetroPane isn't bound by Dell's way; where the hardware
allows something better or more clever, RetroPane does it. CPU
overclocking is explored. And the biggest prize may be the **CPU, the 2D
engine and the 3D core working together** instead of taking turns.

**Exhaustive, not one test and done.** Every item below is measured more
than once, under the conditions that matter (cool and hot, full and low
battery, idle and loaded), with its spread recorded, kept as a benchmark
that runs again after every change (`cargo xtask bench` on the emulator
for logic; device runs for speed), and ticked here only with evidence.
Sweeps of settings are scripted and their results analysed automatically,
so the search covers whole ranges rather than a guess or two. Dell's
values and Windows Mobile's behaviour are findings to beat, never limits
(docs/research/imitation-audit.md).

**Verify everything ourselves (owner, 2026-10-20).** Nothing here is
known because Dell, Windows Mobile, Intel's manual or one of RetroPane's
own docs says so. Every fact carries its source:

- **[us]** measured by RetroPane on the X51v (the run and date given);
- **[Dell]** read from Dell's or Windows Mobile's code (static, unproven);
- **[Intel]** Intel's manual (the part as designed, not this board).

Only **[us]** counts as known. Every **[Dell]** and **[Intel]** fact has
a verify line in Part 0 and stays unticked until our own measurement
agrees or shows otherwise. A fact that can't be checked without opening
the device (never done: the any-Axim rule) is marked so, with the best
indirect check, and never quietly becomes known.

The same rules as the GPU program: device runs only on the owner's go
with numbered steps first; results into docs/hw and the datasheet the same
day; the emulator models what it can or lists what it can't; settings
past the parts' ratings only through a per-device self-test with a
fallback, the owner's any-Axim rule (the RAM's timing excepted: the owner
decided it stays Dell's).

## Part 0: verify what's written down

Each of these is a claim until our own run settles it.

| | Claim (source) | Our check |
|---|---|---|
| **V1** | The CPU is a PXA270 at C5 stepping with iWMMXt, 32+32 KiB caches, a 2 KiB mini cache (Intel, our CPU ID) | Read the CPU ID, cache type and coprocessor access registers; time a walk over 16-64 KiB to see where each cache really ends; run an iWMMXt instruction |
| **V2** | 256 KiB internal SRAM, and its speed (Intel) | Write and read every byte with patterns; time it against the caches and SDRAM |
| **V3** | WM's seven speeds and the voltage pattern for each (Dell) | Set each one ourselves, measure the clock with the cycle counter against the 13 MHz-derived timer, and run a stress test at each |
| **V4** | The mV labels for the pin patterns, and that 1470 and 1364 share a pattern (Dell) | The real voltage can't be read without opening the device. Indirect check: the lowest speed each pattern runs cleanly at, which ranks the patterns; and the battery's drain at each |
| **V5** | 624 MHz is the top (Intel) | C2's stepped test |
| **V6** | The memory clock 104 MHz and refresh ~7.5 us as WM leaves them (our rp-probe read of the registers) | Measure the memory clock from timing, not just the register, at each CPU speed |
| **V7** | WM's ~280 ns memory hop, and the TLB walks as the cause (estimate) | Time the same chase with large mappings and with the tables in SRAM; read the TLB miss counter (C8) |
| **V8** | 32 DMA channels, their bursts and priorities (Intel) | C11's sweep |
| **V9** | Every number our own docs quote from Dell or Intel about the CPU, clocks, memory and power | Swept from clocks.md, ADR-0031, memory-timing.md and the datasheet into this table, one line each |
| **V10** | The memory controller's decode (memory-timing.md, Intel's manual): the controller uses the larger CL of the two pairs; partition 3 (`0xAC000000`) enabled but empty | The CL rule only matters for timing changes, which stay Dell's (the owner's rule); partition 3: a read probe of `0xAC000000` (no write) |
| **V11** | The power manager's bit names in `PCFR` and `PSLR` (clocks.md, from the manual, unchecked) | Each bit's effect seen on the device when power states are built (C10) |

## Part 1: the CPU (Intel PXA270, XScale, 624 MHz)

What's public: Intel's developer's manual and electrical specs (unlike the
2700G). What's not: how this board's clocks, voltage and memory really
behave at the edges, and how well RetroPane's own code uses the core.

| | What we have, by source | Unknown or unused | Probe | Payoff |
|---|---|---|---|---|
| **C1. Clocks** | [us] the core at 624 MHz under the loader (`CCCR 0x02000310`, `CLKCFG 0xB`; the cycle counter 624.001 MHz against the OS timer: rp-noise run 1, 2026-10-15); [Dell] WM's table of 91-624 MHz and its switching code (clocks.md); [Intel] 13 MHz x L x 2N/2, the fast bus (`A`) | Half-turbo (`CLKCFG` HT, never used by WM); the fastest **run** (bus) for a given core speed; switching cost; which point is best for each workload | Every operating point timed on a fixed set of workloads (integer, memory, copies, the mixer, the renderer's CPU side) and its switch time | The right speed for each job: faster where it matters, slower where it doesn't |
| **C2. Overclocking** | [Intel] 624 MHz is the part's rated top; [Dell] the core voltage set by three pins (GPIO3/4/38) and WM's pattern for each speed, with mV labels (1050-1574) whose real voltages nobody has measured; [Dell] no PMIC on I2C found | Whether this core runs past 624 (L 17-19 with 2N 6 is 663-741 MHz) at the top VID; what the bus and memory clocks then do (raising L raises the bus: the SDRAM clock must stay at 104 MHz, so its divider changes); heat; battery | **C2a** static: the clock tree at L 17-19 (bus, SDCLK dividers) and which keep the SDRAM at or under 104 MHz; **C2b** a stepped device test like run G (each step stressed: integer, memory patterns, caches, DMA, a long soak), stopping at the first error; the board's temperature where readable | Up to ~10-19% more CPU, only through the per-device self-test (the voltage can't go past WM's top pattern: no control exists) |
| **C3. Internal SRAM** (256 KiB, on chip) | [Intel] 256 KiB, four banks; [Dell] barely used by WM; [us] a TLB miss 5.2x cheaper with the page table in SRAM (`tlb_spill_60`, perf.md) | Its speed against SDRAM and the caches; what lives best there | Measured; candidates moved there one at a time and timed: page tables (ADR-0006: a TLB miss 5.2x cheaper), the mixer's buffers, the 2700G upload staging (Part 3), hot interrupt paths | Faster memory for what's touched constantly |
| **C4. Memory lookups** | [us] RetroPane's random SDRAM hop 491 ns (`chase_sdram_256k`, perf.md); WM's ~280 ns and "most of the gap is TLB walks" are **unsourced estimates** in memory-timing.md: to measure | The best page sizes, table placement, TLB lock-down (the XScale can lock entries) | Large mappings (64 KiB / 1 MiB), tables in SRAM, locked TLB entries for the kernel and the GPU's windows, each timed | Up to ~1.8x on memory-bound work, no risk |
| **C5. Caches** | [Intel] 32 KiB I and D, a 2 KiB mini data cache, the write and fill buffers, line locking; [us] `pld` helps our `memcpy` (perf.md) | How RetroPane's hot loops use them; the mini cache's best use (streaming data that would evict the main one); cache locking (the XScale can lock lines) | Each hot path profiled with the performance counters (C8); the mini cache for streams; locked lines for the kernel's hottest code | Fewer misses everywhere |
| **C6. iWMMXt** (64-bit SIMD, 16 registers) | [Intel] 64-bit SIMD, 16 registers; [us] the CPU ID `0x69054117` (the coprocessor itself never exercised by us) | Its speed on pixel work (reorder, colour conversion, blends, dithering), audio (mixing, filters, the virtual bass), copies | Each routine written plainly and in iWMMXt, timed, checked equal | 2-4x on pixel and audio loops |
| **C7. Copies and fills** | [us] `memcpy_64k` ~95 MB/s (perf.md) | The best copy for each case: cache-line bursts, `pld` distance, iWMMXt 64-bit, DMA for large ones | A sweep of methods and sizes | Every subsystem's copies faster |
| **C8. Performance counters** | [us] the cycle counter used (audio, rp-noise); [Intel] the event counters (cache and TLB misses, stalls), never used by us | Not yet wired into RetroPane's benches | A counter reader in the bench harness; every benchmark reports misses and stalls, not only time | Seeing why something is slow, not guessing |
| **C9. Code** | [us] Rust, release builds | ARM against Thumb density and speed, inlining, layout of hot code (cache), link-time optimisation, the compiler's XScale tuning | A/B builds of the same workload | Smaller and faster code |
| **C10. Interrupts and sleep** | [us] interrupts and the run limit work; idle measured | Wake latency from each sleep depth; the cost of each interrupt path | Measured | Low latency and long battery together |
| **C11. DMA** | [Intel] 32 channels; [us] SD, audio, CF, UDC and the 2700G uploads use them | Memory-to-memory speed by burst and width, descriptor chains as a general copy engine, priorities | A sweep | The CPU free while data moves |
| **C12. Idle clocks** | [us] `MDREFR` as Windows Mobile leaves it: `K0RUN` set, SDCLK0/3 running for synchronous flash at CLK_MEM / 4 while `SXCNFG` enables none (memory-timing.md) | Whether stopping SDCLK0/3 (and other clocks nothing uses: `CKEN` units) saves battery, and that nothing needs them | Each stopped in turn, battery drain compared (with P16's reading) | Battery for nothing given up |

## Part 2: the CPU's overclock, plainly

- **What's possible:** the core multiplier stops at 624 MHz by Intel's
  rating, but the clock generator can be set higher (663, 702, 741 MHz).
  Whether this chip runs there cleanly is unknown until measured.
- **What limits it:** the core's voltage is set by three pins, and
  Windows Mobile already uses their top setting at 624 MHz, so nothing
  can raise it further: a faster clock at the same voltage either works or
  makes errors. The bus and memory clocks are tied to the multiplier, so
  each step needs the memory's divider chosen to keep the RAM at its rated
  104 MHz (the RAM stays Dell's, as decided).
- **The danger to a chip that can do it:** more heat and battery while
  busy (power rises about with the clock); damage is unlikely at an
  unchanged voltage, errors come first. The self-test catches errors.
- **How it would ship:** only as the per-device self-test with Dell's 624
  as the fallback, like the GPU's.

## Part 3: the CPU, the 2D engine and the 3D core together

Tonight's runs show where they wait on each other: the CPU waits on the
2D engine's command queue (run F: 12 of 12.5 ms), the DMA and the CPU
fight over SDRAM (run E: the overlap lost), the 3D core takes turns with
the 2D engine (ADR-0030 G2), and reading from the 2700G is the bus's
slowest direction (12 MB/s).

| | Idea | Probe | Payoff |
|---|---|---|---|
| **T1** | **A pipeline, not turns:** while the 3D core renders frame N, the 2D engine copies, the DMA uploads and the CPU prepares frame N+1; interrupts say "done", nobody waits and polls | The compositor's frame loop instrumented: each unit's busy and idle time per frame | Up to the sum of the units' speeds instead of the slowest's |
| **T2** | **Upload staging in internal SRAM:** the CPU reorders a strip into SRAM, the DMA sends it from there, so the two no longer share SDRAM | Run E's upload with SRAM staging (expected: the CPU mostly free and the upload at the bus's speed) | Run E's lost overlap back |
| **T3** | **Commands fed by DMA:** the 2D engine's command list built in memory and streamed by DMA, so the CPU doesn't wait on the queue | DMA into the command window, guarded by the queue's level | The CPU free during text and copies (run F: 12 ms back) |
| **T4** | **Each unit on its best job:** the CPU rasterises, decodes and decides; the 2D engine fills, copies and blends; the 3D core composes and transforms; the display moves and scales whole planes | The compositor's routing table measured per kind of work | No unit doing what another does faster |
| **T5** | **Data made in the GPU's formats on the CPU side** (twiddled, 4 and 8 bpp, PVRTC) by table and iWMMXt routines | Each converter timed (run E's table reorder was 3x the plain one) | Uploads at the bus's speed |
| **T6** | **Never read from the 2700G:** shadows of everything the CPU would read back | An audit of every read; the bus's reads counted per frame | The bus's slowest direction gone |
| **T7** | **Clocks set per moment:** the CPU slowed or asleep while the GPU works, the GPU's clocks off while the CPU works | Battery drain against frame rate for the compositor's modes | Speed and battery together |
| **T8** | **The 2D engine and 3D core at once** (P8 in the GPU map) | As there | Up to ~2x throughput when both are busy |

## C6 wired in: the SIMD crate (design, 2026-10-22; the owner's choice b)

The owner chose a small HAL-level crate over a HAL hook (no callback in the
hot path; the plain loop runs on host and emulator; the ARM path is already
checked equal). The plan, built in this order:

1. **`hal/simd` (`rp-hal-simd`)**, beside `hal/mem` and shaped like it: the
   dot product's assembly moves here from `rp_hal_pxa27x::iwmmxt` (which
   re-exports it, so the probe's `wmmx_dot` row measures the same code),
   with the plain loop as the only path on the host. One call,
   `dot(win, at, coef) -> i32`: wrapping 32-bit sums as today, the plain
   loop whenever iWMMXt's conditions (8-aligned, a multiple of 8 long,
   reads inside `win`) don't hold.
2. **Never a fault:** iWMMXt runs only after whoever guarantees the unit is
   reachable says so (`rp_hal_simd::set_unit(true)`: the kernel once its
   lazy context switching is set up, a bare image after `single_access`).
   The default is the plain loop, so an image that never says so is right,
   just slower; one flag read a block, not a sample.
3. **The mixer's block path** (`services/audio` `add_fast`): the window
   becomes 16-bit (it is 16-bit samples today, widened to `i32`), 8-aligned,
   with the padding the routine reads; each phase's coefficients become one
   contiguous row (the left wing reversed, then the right, zeros to a
   multiple of 8: 24 for 48 kHz's 11 taps, 48 at most). 48 kHz's table
   `T48` is stored in that shape (replaced, not added: memory first), the
   whole-number strides and the general case build it once a phase as now.
   `add_exact` (the frame-by-frame reference) stays as it is; the existing
   tests that hold `add_fast` equal to it are the check, plus a host test
   of the crate against a naive sum at every alignment and length.
**Built 2026-10-22 (steps 1-3):** `hal/simd` (`dot`, `dot_plain`, `wmmx`, `set_unit`; host tests at every start and length, wrapping), `iwmmxt::dot` re-exported from it; `T48` is now `Rows48` (148 rows of 24, 8-aligned, 7,104 B against 6,512: the old rows were half empty); `add_fast` takes 16-bit aligned windows and one row a frame through `rp_hal_simd::dot`; `rp-audio-s2` switches the unit on (logs "the filter's dot product: iWMMXt"; `RP_AUDIO_PLAIN` builds it with the plain loop) and puts the access back at the end. The block path still equals the frame path at every rate (77 audio tests; two new mutants for the rows' reversal and the T48 one moved, all three caught by hand). Emulator (rp-v0.30.0 code): `emu-audio-s2` passes with the unit on (the tune at pitch from 22.05, 11.025 and 48 kHz), `emu-audio-sleep` and `emu-probe` pass, `emu-audio-e3` 3 of 3 after one C49 late wake. Not yet: step 4 on the device (owner present or an unattended go).

4. **Measured:** the mixer's cycles a frame on the X51v (669 at 48 kHz
   today: `docs/hw/audio.md` "The mixer's cost", re-run the same way), the probe's rows for the floors, and the emulator
   audio check; a device run waits for the owner.

## The order

The GPU program's clock run (`rp-gpubench` run G) comes first, as asked.
Then, interleaved with the GPU map by payoff: **T2** (SRAM staging: quick,
and it fixes a measured loss), **C4** (memory lookups: big, safe), **C8**
(the counters, so every later probe shows why), **C6/C7** (iWMMXt and
copies), **C1** (operating points), **C2** (the overclock study, static
then stepped), **T1/T3/T7** (the pipeline, with the compositor), then the
rest. Each result can reorder what follows; this page is kept current.

## The ledger (counted by `cargo xtask gate`)

Every line of this page, one row: **done** (used, or declined with the
measured reason), **partial** (some device evidence, more to do) or
**open**. Only done counts as ticked. A row changes only with a run
named in its evidence.

| Line | Kind | Status | Evidence, or what's left |
|---|---|---|---|
| V1 | opt | partial | rp-probe (X51v, 2026-10-22): the D-cache's 32 KiB confirmed (a line walk misses from 40 KiB, ~233 ns a miss); the CPU ID read; iWMMXt present and working (`wCID` `0x69051020`, access on as Windows Mobile leaves it; the emulator reports `0x69051051`); the I-cache and the mini data cache not measured |
| V2 | arch 2 | done | rp-probe (X51v, 2026-10-22, `local/rp-probe-gate-run7.txt`): all 256 KiB of the SRAM written and read back with 0x55555555, 0xAAAAAAAA, each word's address and its complement, every word right (no aliasing: the size is the manual's); its speed: 286 MB/s copy, 475 MB/s fill, 111 ns a random hop |
| V3 | arch 9 | open | WM's seven speeds not set and measured by us |
| V4 | arch 9 | declined | **Declined (the owner's ruling 2026-10-22, docs/research/gate-classification.md, B7): the voltages can't be measured without opening the device (the owner's rule: never)**; reopen if a way appears. Before: the voltage patterns not ranked |
| V5 | arch 9 | open | C2's stepped test not run |
| V6 | opt | declined | **Declined (backlog rule, 2026-10-22): no payoff** (the refresh period only matters for memory timing changes, and timing stays Dell's by the owner's rule of 2026-10-19); reopen if that rule changes. Evidence: rp-probe (X51v, 2026-10-22): `memset` of 64 KiB (past the cache) writes 377 MB/s, more than any memory clock below 104 MHz allows (52 MHz: 208 MB/s at most on the 32-bit bus), so the 104 MHz SDRAM clock is confirmed from timing; the refresh period (~7.5 us) not measured |
| V7 | arch 2 | partial | rp-probe (X51v): our random hop over 256 KiB is 491 ns with no TLB misses at all (the bring-up map's 1 MiB sections: the chase stays in one); so the gap to Windows Mobile's "~280 ns" isn't TLB walks under our mapping, and that figure has no source (an estimate in memory-timing.md); what's left: the hop with 4 KiB pages (the kernel's map) and a sourced WM number; **pages run 1 (2026-10-22): the hop 448.6 ns on sections, 484.1 on 4 KiB pages, 450.2 with both table levels in the SRAM; a 4 KiB D-TLB miss ~565 ns, ~25 with both levels in the SRAM** (cpu.md); **run 2, every hop a new 4 KiB page: 1,016 ns a hop with the tables in SDRAM, 473 with both levels in the SRAM (sections 449)**; left: a sourced Windows Mobile number |
| V8 | opt | partial | C11's sweep (X51v, 2026-10-22, `local/rp-probe-dma-shapes.txt`): memory-to-memory bursts of 32/16/8 bytes copy 128 KiB in 2.90/4.78/8.50 ms (about 0.5 us fixed a burst: 32 bytes best, as shipped); the width field makes no difference (0, as Linux sets it, and word the same); priorities not measured (they matter only under contention: with SD or audio DMA running alongside) |
| V9 | meta | done | 2026-10-21: clocks.md, ADR-0031, memory-timing.md and the datasheet swept: the clock limits are V3/V5, the voltage patterns V4; the rest became V10, V11 and C12 |
| V10 | opt | declined | **Declined (backlog rule, 2026-10-22): no payoff** (partition 3 and the CL rule only matter for timing changes, which stay Dell's). Evidence: the CL rule declined with the RAM's timing (the owner's rule: Dell's stays); partition 3's emptiness not probed |
| V11 | arch 9 | open | the power manager's bit names not checked |
| C1 | arch 9 | partial | 624 MHz measured under the loader (rp-noise run 1); the operating points and switch costs not swept |
| C2 | arch 9 | open | the overclock study not started |
| C3 | arch 2 | done | **Ticked (the owner's ruling 2026-10-22, docs/research/gate-classification.md, B10): the owner's shared SRAM layout is the answer** (ADR-0006 amendment); building it is in the backlog (C3 rest). Evidence: rp-probe (X51v, 2026-10-22): SRAM copies 3x SDRAM, hops 4.4x faster, page tables there make TLB spills 5.2x cheaper; what lives there best (tables, hot buffers, upload staging: T2) to decide |
| C4 | arch 2 | done | **Used: the owner's ruling 2026-10-22 ([ADR-0006](../adr/0006-physical-memory-beyond-the-textbook.md) amendment "Page sizes, page tables and lock-down"): 4 KiB pages; the first-level table and the busiest second-level tables in the SRAM (32 KiB), the rest in SDRAM; three hot kernel pages locked; 64 KiB pages a lower-priority optimization.** The runs: RetroPane's random hop 491 ns (perf.md); first-level table in SRAM: a section's TLB miss ~64 ns against ~333 (`tlb_spill_60_l1sram`); page sizes and placement: **pages run 1 (2026-10-22): a 4 KiB miss ~565 ns with the tables in SDRAM, ~25 with both levels in the SRAM; 64 KiB pages and sections no misses in that chase** (cpu.md); **run 2, a new page every hop: miss ~550 ns in SDRAM, ~24 with both levels in the SRAM, worst-case 4 KiB hop +5% over sections with the tables in the SRAM (2.26x in SDRAM); 64 KiB pages miss 16x less**; **run 3: D-TLB lock-down works on the X51v (16 of 64 locked: misses 4,224 to 3,264 a pass), a walk saved ~544 ns (tables in SDRAM) or ~94 ns (in the SRAM) a hot page**; all three measured: the choice (page size, table placement, what to lock) is the memory ADR's |
| C5 | opt | partial | `pld` helps `memcpy` (perf.md); the mini cache and line locking not tried |
| C6 | opt | partial | rp-probe (X51v, 2026-10-22, `local/rp-probe-wmmx.txt`): iWMMXt routines (`rp_hal_pxa27x::iwmmxt::{copy, mix, avg}`) checked equal to plain code; **a saturating 16-bit mix 6.7x faster in the cache (12.8 against 85.4 us for 8 KiB), 2.1x from memory; a rounded 50% pixel average 2.8x in the cache (12.8 against 36.2 us, against the plain SWAR), 1.5x from memory; 64-bit copies no faster than `memcpy` (2.409 against 2.410 ms for 256 KiB: the bus)**. **The resampler's filter** (48 kHz, 11 taps a wing; a block of 32 stereo frames, `local/rp-probe-wmmx-dot.txt`): the mixer's loop as it is 16.5 us, one plain contiguous dot product (left coefficients stored reversed) 15.2 us (1.09x), iWMMXt `dot` (`WMADDS`, two interleaved chains, `WALIGNR` for the window's alignment) 11.3 us (**1.46x**; 13.6 us with one chain); all equal at every alignment. Wiring it in: **the owner chose (b), 2026-10-22: a small HAL-level SIMD crate** the audio service uses on ARM with the plain loop elsewhere (no callback in the hot path, the plain loop runs on host and emulator, the ARM path already checked equal); then measured in the mixer's cycles a frame (669 at 48 kHz). Left also: colour conversion and the T5 converters (WSHUF/WUNPCK/WPACK), the virtual bass filter (WMAC); **wired into the mixer (2026-10-22, X51v, `local/rp-audio-s2-c6-unit.txt` / `-plain.txt`): cycles a frame plain against iWMMXt, 48 kHz 819 to 721 (-12%), 22.05 kHz 583 to 536 (-8%), 11.025 kHz 585 to 533 (-9%), the two-rate test 636 to 580 (-9%); 44.1 kHz (no filter) unchanged at 346**; both runs above the 669 docs/hw/audio.md recorded for 48 kHz before (to explain); left: colour conversion, T5's converters, the virtual bass filter |
| C7 | opt | done | **Ticked (backlog rule, 2026-10-22): used the prefetch; the rest declined by measurement** (iWMMXt 64-bit copies no faster than `memcpy`, 2.409 against 2.410 ms; DMA copies 45 MB/s against `memcpy`'s 108: C6, C11). Evidence: rp-probe (X51v, 2026-10-22): `memcpy` ~375 MB/s in the cache, ~96 MB/s from memory; **the prefetch swept (none to 256 B): 128 B ahead now in `memcpy`, -12% on every large copy (109 MB/s)**; the destination line allocate (CP15 c7, c2, 5) **declined: it crashed the probe twice** (wild jumps; cause not found, likely the scratch region's cache mapping); left: iWMMXt 64-bit moves, DMA for large copies (C11) |
| C8 | opt | partial | the cycle counter used; **the event counters built (2026-10-22): `rp_hal_pxa27x::pmu` ([Intel] layout and event numbers, claims until seen), the probe's `pmu` section: known-answer checks (a `subs`/`bne` loop of 10,000: 20,000 instructions and 10,000 branches; D-cache misses for a 64 KiB walk, every line, and a 16 KiB one, none) and `count` lines for a 64 KiB copy (misses, write-backs, buffer stalls); emulator: the plumbing (counters read 0 there)**; left: the device run, then counts around the other benches and into the ratchet; **seen on the X51v 2026-10-22** (`local/rp-probe-c9-s-1.txt`): the loop 20,004 instructions and 10,000 branches, the 64 KiB walk 2,056 D-cache misses and 16 KiB 0 (the manual's layout and events 0x05/0x07/0x0B confirmed); **a 64 KiB `memcpy` from memory: 252,653 of its 310,394 cycles stalled on full write buffers (81%), 17,931 D-cache misses, 5 write-backs**: copies are write-bound |
| C9 | opt | declined | **Declined (the owner, 2026-10-22): the rest of C9 measured not worth it** (opt-level 3 at most 2.1%, one row 6.4% slower, +14 KB; images keep their opt-levels; frame pointers used, `target-cpu=xscale` no effect, Thumb for cold code only). Evidence: rp-probe built four ways on the X51v (2026-10-22, perf.md): `target-cpu=xscale` byte-identical (no effect); Thumb 26% smaller and 25-120% slower (declined for hot code; a size option for cold code); **frame pointers optional: 2-7% faster compute, 1.5 KB smaller, nothing slower once the SD driver's polling was fixed**: ADR-0001 amended (B, may-omit: the owner's vote 2026-10-22, both target specs); LTO and inlining A/B not run; **opt-level 3 against `s` under LTO, the probe on the X51v (2026-10-22, two runs each alternating, `local/rp-probe-c9-*.txt`, spread 0-1%): 83 rows, the best gains 2.1% (frame allocator) and ~1% (lz4 decompression), most unchanged, `gfx_blend_fill_vga` 6.4% slower, the image 14 KB bigger** |
| C10 | arch 9 | partial | interrupts, the run limit and idle work; wake latency per sleep depth not measured |
| C11 | opt | done | rp-probe (X51v, 2026-10-22): DMA copies 128 KiB SDRAM to SDRAM in 2.92 ms (45 MB/s) against `memcpy`'s 1.21 ms (108 MB/s), the CPU keeping 86% meanwhile; **polling the SD controller during its DMA slowed it: a 200-spin pause between polls makes SD DMA reads 28% faster (now in `pxa-mmc`)**; the burst sweep (V8): 32-byte bursts fastest, 45 MB/s the controller's best memory-to-memory (`memcpy` 108); left: the same polling question for the USB drive's and the CF slot's DMA waits, and priorities under contention; **the polling audit (2026-10-22): only CF's DMA wait polls back to back; `RP_CF_POLL_SPIN` built (default 0), the device comparison waits**; **the polling audit measured (2026-10-22, X51v, `rp-storfast` C2, `local/rp-storfast-c11q-*.txt`): CF DMA reads 5,868 KiB/s with no pause and 5,838 with 200 spins: no gain (CF's DMA is paced by the card's window, not the polls); CF keeps no pause; nothing else polls a DMA back to back. Done: DMA used where it pays (SD, audio, CF, uploads), the poll pause where it pays (SD), the copy engine declined (45 against `memcpy`'s 108 MB/s)** |
| C12 | opt | declined | **Declined (the owner, 2026-10-22): its only payoff is battery life, and no battery reading exists (P16), so it can never be ticked with evidence; reopen if a reading appears.** Before: SDCLK0/3 run for synchronous flash nothing uses (`K0RUN`, `MDREFR` as WM leaves it); stopping them, and unused `CKEN` units, not tried |
| T1 | arch 5 | partial | **run CH, the compositor harness (X51v, 2026-10-24, `local/rp-gpubench-runCH3.txt`, 2700g.md "The compositor harness..."): the CPU's frame work (~4.3 ms) beside the render instead of before it: frames 9-10% shorter (W5 9.0 to 8.15 ms median, worst 29.3 to 22.4; W6 34.3 to 37.7 fps), the render ~1.5 ms slower while the CPU works**; 2D and 3D still in turns (P8). Left: the owner's ruling (pipeline or turns), why the render slows (bus or memory), C71 |
| T2 | arch 5 | done | **Ticked (the owner's ruling 2026-10-22, docs/research/gate-classification.md, B10): the shared SRAM layout lets staging borrow two 32 KiB buffers during a transfer** (ADR-0006 amendment); wiring it into the upload path is in the backlog (T2 rest). Evidence: run E's E1e (X51v, 2026-10-22): a 1 MiB texture staged in SRAM (two 32 KiB buffers) and overlapped with DMA: **32.2 ms (32 MB/s, the 2700G bus's limit)**, the CPU free 7.4 ms of it, against 34.8 ms with the CPU writing straight in and 41.0 staged in SDRAM; to use it, the OS needs an SRAM plan (page tables, upload staging, the kept log: who owns the 256 KiB): **the owner chose a shared layout, 2026-10-22** (ADR-0006 amendment: staging borrows two 32 KiB buffers only during a transfer) |
| T3 | arch 5 | partial | run FD (X51v, 2026-10-22, `local/rp-gpubench-runFD.txt`): 2D commands DMA'd into the command window (`0x0FFF5000`) draw exactly as the CPU's writes (3 of 3); 352 words: the CPU busy 2 us to arm (13 the first time) against 43-60 us writing them, the move 47 us (the bus's pace), the work done at the same time either way. Limit: a batch must fit the FIFO's free room (the DMA can't wait for room; 510 words with the engine idle). Left: feeding the renderer's frames this way (in batches sized to the room) and measuring the CPU it frees in a real frame (the harness, T1). **Run CH (2026-10-24): in real frames (a W2 row's copy, a W4 key's fill) the DMA'd commands cost the CPU what writing them does (~0 us for 10 words) and draw in the same time (620 against 616 us; 52 against 52): DMA pays only for long batches (FD) or pixels (a row's pixels by DMA: 1.5 ms, the CPU 12 us, against 2.3 ms of CPU stores)** |
| T4 | arch 5 | partial | **run CH's routing table (X51v, 2026-10-24): a list row 480 x 48: CPU 2.32 ms, 2D 0.62 (the CPU ~0), 2D by DMA 0.62, DMA pixels 1.50 (the CPU 0.01); a key 44 x 52: CPU 0.22, 2D 0.05; a screen copy (2D) 9.1 ms; a screen turned: 3D 26.1 ms, 2D rotated copy 41.8; pans, sheets, fades: the planes, 0 us; an app picture scaled: 3D ~10 ms a frame (the video scaler: P9, the owner's look); text: cached lines by 2D 9.9 ms, the CPU 0.07 (F7)**. Left: the owner's ruling on the table |
| T5 | opt | partial | the 32-bit twiddle reorder by table: 11.9 ms a MiB (88 MB/s, ~81% of `memcpy` from memory; run E E1a), and overlapped with uploads through the SRAM the upload reaches the bus's 32 MB/s (E1e), so the reorder isn't the limit; PVRTC is compressed on the PC (`upload_pvrtc`), no device converter; left: the 16-bit reorder timed, an iWMMXt reorder only if a converter ever limits an upload |
| T6 | arch 5 | done | the OS path's reads of 2700G memory audited (2026-10-21): mipmap levels no longer read back (made from SDRAM, a test fails on any read); the fence polling measured harmless to the engine (B8c, X51v: 61.8 ms polled nonstop vs 62.2 every 1 ms); what's left is single-word register reads |
| T7 | arch 9 | open | clocks per moment against battery not measured (harness) |
| T8 | = 2700G P8 | partial | as the GPU page's P8 (run FT: together 9% faster but the 3D render corrupts; the turns hold) |

## The checklist rule

As for the GPU: every line here is ticked only with evidence (measured,
probed, or shown impossible or not worth it), anything found gets a line,
and the owner is told "nothing is left" only when every line is ticked.
