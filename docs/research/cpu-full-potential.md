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

| Line | Status | Evidence, or what's left |
|---|---|---|
| V1 | open | the CPU ID read (`0x69054117`); the caches' real ends, the mini cache and iWMMXt not measured |
| V2 | open | the SRAM's size and speed not measured |
| V3 | open | WM's seven speeds not set and measured by us |
| V4 | open | the voltage patterns not ranked |
| V5 | open | C2's stepped test not run |
| V6 | open | the memory clock not measured from timing |
| V7 | open | WM's ~280 ns hop and its cause not measured |
| V8 | open | C11's DMA sweep not run |
| V9 | done | 2026-10-21: clocks.md, ADR-0031, memory-timing.md and the datasheet swept: the clock limits are V3/V5, the voltage patterns V4; the rest became V10, V11 and C12 |
| V10 | partial | the CL rule declined with the RAM's timing (the owner's rule: Dell's stays); partition 3's emptiness not probed |
| V11 | open | the power manager's bit names not checked |
| C1 | partial | 624 MHz measured under the loader (rp-noise run 1); the operating points and switch costs not swept |
| C2 | open | the overclock study not started |
| C3 | partial | a TLB miss 5.2x cheaper with the table in SRAM (perf.md); the SRAM's speed and best uses not measured |
| C4 | partial | RetroPane's random hop 491 ns (perf.md); page sizes, table placement, TLB lock-down not tried |
| C5 | partial | `pld` helps `memcpy` (perf.md); the mini cache and line locking not tried |
| C6 | open | iWMMXt never exercised |
| C7 | partial | `memcpy_64k` ~95 MB/s (perf.md); the sweep of methods and sizes not run |
| C8 | partial | the cycle counter used; the event counters not wired into the benches |
| C9 | open | ARM/Thumb, inlining, layout, LTO A/B builds not run |
| C10 | partial | interrupts, the run limit and idle work; wake latency per sleep depth not measured |
| C11 | partial | DMA used for SD, audio, CF, UDC and 2700G uploads; memory-to-memory speed by burst not swept |
| C12 | open | SDCLK0/3 run for synchronous flash nothing uses (`K0RUN`, `MDREFR` as WM leaves it); stopping them, and unused `CKEN` units, not tried |
| T1 | open | the pipeline needs the compositor harness (owner's vote: built inside the gate) |
| T2 | open | SRAM staging for uploads not tried |
| T3 | open | commands fed by DMA not tried |
| T4 | open | the routing table per kind of work not measured (harness) |
| T5 | open | the GPU-format converters not timed together |
| T6 | partial | audit 2026-10-21 of the OS path's CPU reads of 2700G memory: (1) `upload_mipmapped` read each level back to make the next (a 256 x 256 texture: ~64 K word reads at 12 MB/s): **fixed** 2026-10-21, levels made from the base in SDRAM by the same cascade, a test fails on any read back (its device speed not measured yet: no device image uploads mipmaps); (2) the 2D renderer spins on the fence word while the engine works: whether that slows the engine is `rp-mbx-blend` B8c (built); register reads (FIFO level, status) are single words. Both to finish |
| T7 | open | clocks per moment against battery not measured (harness) |
| T8 | open | as the GPU page's P8 |

## The checklist rule

As for the GPU: every line here is ticked only with evidence (measured,
probed, or shown impossible or not worth it), anything found gets a line,
and the owner is told "nothing is left" only when every line is ticked.
