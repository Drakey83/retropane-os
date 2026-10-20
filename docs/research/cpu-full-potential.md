# The CPU's full potential, and the CPU, 2D engine and 3D core together (2026-10-20)

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

The same rules as the GPU program: device runs only on the owner's go
with numbered steps first; results into docs/hw and the datasheet the same
day; the emulator models what it can or lists what it can't; settings
past the parts' ratings only through a per-device self-test with a
fallback, the owner's any-Axim rule (the RAM's timing excepted: the owner
decided it stays Dell's).

## Part 1: the CPU (Intel PXA270, XScale, 624 MHz)

What's public: Intel's developer's manual and electrical specs (unlike the
2700G). What's not: how this board's clocks, voltage and memory really
behave at the edges, and how well RetroPane's own code uses the core.

| | Known | Unknown or unused | Probe | Payoff |
|---|---|---|---|---|
| **C1. Clocks** | 13 MHz x L (run) x 2N/2 (turbo); WM's table 91-624 MHz, the 208 MHz bus (`CCCR` A=1), 624 = 16 x 3 (ADR-0031, clocks.md) | Half-turbo (`CLKCFG` HT, never used by WM); the fastest **run** (bus) for a given core speed; switching cost; which point is best for each workload | Every operating point timed on a fixed set of workloads (integer, memory, copies, the mixer, the renderer's CPU side) and its switch time | The right speed for each job: faster where it matters, slower where it doesn't |
| **C2. Overclocking** | 624 MHz is the part's rated top (C5 stepping); the core voltage is three GPIO VID pins (GPIO3/4/38), WM's top pattern at 624 (1574 mV label, unmeasured) | Whether this core runs past 624 (L 17-19 with 2N 6 is 663-741 MHz) at the top VID; what the bus and memory clocks then do (raising L raises the bus: the SDRAM clock must stay at 104 MHz, so its divider changes); heat; battery | **C2a** static: the clock tree at L 17-19 (bus, SDCLK dividers) and which keep the SDRAM at or under 104 MHz; **C2b** a stepped device test like run G (each step stressed: integer, memory patterns, caches, DMA, a long soak), stopping at the first error; the board's temperature where readable | Up to ~10-19% more CPU, only through the per-device self-test (the voltage can't go past WM's top pattern: no control exists) |
| **C3. Internal SRAM** (256 KiB, on chip) | Barely used (the loader's clock-change code is the main user under WM) | Its speed against SDRAM and the caches; what lives best there | Measured; candidates moved there one at a time and timed: page tables (ADR-0006: a TLB miss 5.2x cheaper), the mixer's buffers, the 2700G upload staging (Part 3), hot interrupt paths | Faster memory for what's touched constantly |
| **C4. Memory lookups** | RetroPane's random SDRAM access ~490 ns against WM's ~280 ns, most of it TLB walks (memory-timing.md) | The best page sizes, table placement, TLB lock-down (the XScale can lock entries) | Large mappings (64 KiB / 1 MiB), tables in SRAM, locked TLB entries for the kernel and the GPU's windows, each timed | Up to ~1.8x on memory-bound work, no risk |
| **C5. Caches** | 32 KiB I and D, write-back, a 2 KiB mini data cache, the write buffer and fill buffers, `pld` | How RetroPane's hot loops use them; the mini cache's best use (streaming data that would evict the main one); cache locking (the XScale can lock lines) | Each hot path profiled with the performance counters (C8); the mini cache for streams; locked lines for the kernel's hottest code | Fewer misses everywhere |
| **C6. iWMMXt** (64-bit SIMD, 16 registers) | Present; little used | Its speed on pixel work (reorder, colour conversion, blends, dithering), audio (mixing, filters, the virtual bass), copies | Each routine written plainly and in iWMMXt, timed, checked equal | 2-4x on pixel and audio loops |
| **C7. Copies and fills** | `memcpy_64k` ~95 MB/s (perf.md) | The best copy for each case: cache-line bursts, `pld` distance, iWMMXt 64-bit, DMA for large ones | A sweep of methods and sizes | Every subsystem's copies faster |
| **C8. Performance counters** | The XScale has them (cycles, cache and TLB misses, stalls) | Not yet wired into RetroPane's benches | A counter reader in the bench harness; every benchmark reports misses and stalls, not only time | Seeing why something is slow, not guessing |
| **C9. Code** | Rust, release builds | ARM against Thumb density and speed, inlining, layout of hot code (cache), link-time optimisation, the compiler's XScale tuning | A/B builds of the same workload | Smaller and faster code |
| **C10. Interrupts and sleep** | Interrupts and the run limit work; idle measured | Wake latency from each sleep depth; the cost of each interrupt path | Measured | Low latency and long battery together |
| **C11. DMA** | 32 channels; SD, audio, CF, UDC use some | Memory-to-memory speed by burst and width, descriptor chains as a general copy engine, priorities | A sweep | The CPU free while data moves |

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

## The checklist rule

As for the GPU: every line here is ticked only with evidence (measured,
probed, or shown impossible or not worth it), anything found gets a line,
and the owner is told "nothing is left" only when every line is ticked.
