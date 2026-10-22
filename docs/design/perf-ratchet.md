# The performance ratchet (the gate's step 2b; the owner's vote, 2026-10-21)

**What it is, plainly:** a set of speed numbers measured on the X51v,
each kept as its best-so-far ("the floor"). A change that makes one of
them slower beyond its measured noise fails until it's fixed or the owner
approves the new number. It does for speed what the size budgets already
do for code size (`cargo xtask cross`), so performance is held every day,
not remembered once. The gate (docs/ROADMAP.md) lifts only when every
checklist line is done **and** this is running.

## The numbers (what people feel, plus what explains them)

| Group | Number | Where it comes from today | State |
|---|---|---|---|
| Touch | tap to screen, landing to verdict, frame, queue to flip, the pen-down loop's period (median, worst) | `rp-today` built with `RP_TODAY_REPLAY`: recorded taps replayed (see "Taps" below), its `TR` line | built and run on the X51v 2026-10-22 (numbers below); floors wait for the owner's next noise run |
| Frames | the shell's frame times (menu open, scroll) | `rp-today`'s per-frame costs (`FrameStats`) | exists, same |
| 3D | frames a second for a set scene | `rp-gpubench` renders (ms a full screen) | exists (run A/T timings) |
| 2D | fill, copy, convert, dither add a full screen | `rp-mbx-blend` B7/B8 | exists |
| Boot | the loader's start-up stages, HaRET to ready | the loader's `stages` (boot time line) | exists |
| CPU | copies by size, cache walk, SRAM, chase, TLB | `boot/suite` via `rp-probe` (P1-P5 pages) | exists |
| Storage | SD reads (block, multi-block, DMA) | `boot/suite` sd rows, `rp-sdtime` | exists |
| Battery | drain at idle and under load | none yet (needs the battery reading: CPLD/PMIC unknown) | to find (gate P16) |

## How it works

1. **One command, `cargo xtask perf`**, runs the device suite: the
   unattended images in turn (`rp-probe`, a short `rp-gpubench` timing
   run, `rp-mbx-blend` B7/B8, the loader's boot line), collects every
   number from the run logs, and writes `local/perf/<date>.txt`.
2. **Noise first.** Each number's spread is measured by running the suite
   five times on the same build; its tolerance is set from that spread
   (never a guessed percentage), and kept with the floor.
3. **The floors file, `docs/perf-floors.md`** (committed): one row a
   number: its floor, its tolerance, the run that set it, and for any
   number allowed to get slower, the owner's approval (date and words).
4. **The check:** `cargo xtask perf` compares a run against the floors:
   faster beyond the tolerance → the floor ratchets down (committed with
   the change); within → fine; slower beyond → **fails**, naming the
   number, until fixed or approved. A summary line goes into every report,
   like the gate's count.
5. **When it runs:** device numbers need the Axim docked, so not on every
   commit: at the end of every change that touches a measured path, and
   before any push of such a change. The emulator's instruction counts
   (`cargo xtask bench`) stay as the every-commit early warning.
6. **New features:** each comes with its numbers added to the floors file
   (its budget: "this screen opens in under X ms"), and its ADR names the
   checklist lines it uses.

## Built (2026-10-22 night)

`cargo xtask perf` (`xtask/src/perf.rs`, host-tested): the suite is
`rp-probe` (its benchmark rows: copies, caches, SRAM, DMA, iWMMXt, the
filter, LZ4, crypto, TLB), `rp-gpubench` run FT's alone timings (a
full-screen render, ten 2D copies), `rp-mbx-blend` B7d (conversions,
copies), B8 (the dither add) and B9e (scaling, palettes, a 16-bit copy):
every number named from its run-log line, read from the card log.
`cargo xtask perf noise 5` writes `docs/perf-floors.md` (floor = the
median of five runs; tolerance = the largest of four standard deviations
of the runs, twice the smallest step between their values, and two steps
of the number's timer: 4000/13 ns for the probe, 4 us for the images.
Not the runs' range: a sixth run falls outside five runs' range a third
of the time, and the first check failed seven numbers within a tick);
`cargo xtask perf refloor N` recomputes them from saved runs;
`cargo xtask perf` fails on a number slower beyond its tolerance or gone
from the run, and reports faster ones (`--update` ratchets them down).
First floors: 99 numbers (2026-10-22 night); an independent check run
after them: 103 numbers, 0 slower, 1 faster, 4 new. Not yet in it: the
loader's boot line (needs a restart: owner present),
touch (the replay: built, its floors wait for device runs), battery
(P16), the harness's numbers (T1, T4, T7).

## Taps

Touch numbers need taps on the device. Options, for the owner: a person
taps a fixed script when the suite runs (watched runs only), or the image
replays recorded pen samples through the same filter and drawing path
(no taps, but the touch controller's own timing is skipped). Proposed:
replay for the ratchet (repeatable), real taps at milestones.

**Decision (owner, 2026-10-22): the hybrid.** Replay for the ratchet, real
taps at milestones: a ratchet lives on repeatability, and person-taps would
make the floors noisy; the touch controller's timing that replay skips is a
constant offset, and the ratchet measures changes.

**Built (2026-10-22).** `rp-today` built with `RP_TODAY_REPLAY=1` (the
suite's sixth step; `emu-today` checks it):

- *The recording:* the owner's taps of 2026-10-17
  (`drivers/touch-cal/tests/data/owner-2026-10-17-12k.txt`), at today's
  sample rate and conversions (7 ms, X and Y 32): 29 touches, 28 of them
  presses, through the start menu. The older recordings are at 11 ms,
  where the filter's per-sample thresholds mean something else
  (docs/hw/touch.md). The last touch, the owner ending that run, is left
  out: Align screen sits where it lands now. Packed by `rp-today`'s build
  script (8 bytes a sample, 4,400 bytes, in the replay build only); a host
  test checks the packing against the text, and another runs the scenario
  through the filter and the shell and fails if a shell change moves a tap
  off the Today screen.
- *Pace:* within a touch the image's own loop takes one recorded sample a
  pass and still makes the real sample's conversions
  (`Tsc2046::convert_traced`), so a slower loop shows as a slower verdict
  and a longer `pen-down period`, as C58's did. Without the conversions
  a replay would have missed exactly that. Between touches the
  recording's pause, at most 500 ms.
- *Nothing logged mid-run:* the recorder and the slow-frame and
  slow-answer lines are off in the replay. The card log writes a sector
  at every line's end (quality audit C59), and a line inside the loop
  would be measured with it. `RP_TODAY_REPLAY_RECORD=1` keeps the
  recorder on to measure what it costs (its line says "with the
  recorder" and isn't a floor).
- *Numbers:* one `TR touch replay:` line at the end, ten numbers: touch
  to flush, landing to verdict, frame and queueing, queue to flip, and
  the pen-down period, each median and worst (the first 256 of each
  counted for the median). The run returns to the loader by itself.

On the emulator (its times, not the device's): 28 presses, 52 answers,
520 periods.

**On the X51v (2026-10-22, the new loader, the card log's option (a)):**

| Run | Pen-down period, median (worst) | Landing to verdict | Touch to flush, median (worst) |
|---|---|---|---|
| The replay (recorder off), twice (alone and in `cargo xtask perf`) | 6.95 ms (12.6) | 8.9 ms (15.9) | 13.3 ms (44.3) |
| The replay with the recorder (`RP_TODAY_REPLAY_RECORD`), C59's option (a) | 10.6 ms (46.2) | 17.9 ms (28.5) | 30.4 ms (55.1) |
| The same, after C59's follow-up (a busy card left alone) | 7.2 ms (13.0) | 9.5 ms (16.7) | 11.3 ms (44.6) |

The two runs without the recorder agreed to the microsecond on most
numbers. A side effect of the replay: its conversions hold the pen line
low, so the run's "pen line down without pressure" count means nothing
in it. The floors come from the owner's next `cargo xtask perf noise`
(the first `cargo xtask perf` with the step: 15 new numbers, not held).

**The same run, the whole suite (2026-10-22, `local/perf/run-1792689365.txt`,
the first after the card log's option (a) moved every image's code by
~632 bytes):** 114 numbers, 16 faster, 7 slower beyond their noise:
three by 0.0-0.3% (`b7d` 16-bit copy, `b9e` 16-bit copy, `ft_alone` 2D),
`probe.dma_copy_128k_b8_word` +4.7%, `probe.lz4_compress_page` +6.1%,
`lz4_compress_text` +4.1%; the largest faster `probe.cpu_unit_alone` and
`cpu_unit_during_dma` -16.7%. Swings both ways in CPU loops whose code
didn't change look like code placement, not the change: to be checked
before the owner decides (no floor updated).

**A second run after C59's follow-up moved every image's code again
(`local/perf/run-1792692073.txt`):** 105 of 110 numbers within 0.5% of
the first run (`probe.cache_walk_32k` +11%, the noisiest), the same ones
slower than the floors (now 8 with `probe.cache_walk_40k` +0.9%,
`probe.gfx_flush_vga` +0.0%). So moving the code by ~1 KiB doesn't move
them: what both runs share and the floors' runs didn't is the new loader
(installed this session) and C59's first change. Next: one run under
the old loader (the backup) to tell the two apart, then the owner
decides the floors.

**A third run under the old loader (2026-10-22, the owner present;
`local/perf/run-1792700817.txt`; the backup's loader put on the card,
started after `cargo xtask reboot-wm`, then today's put back the same
way):** all 109 numbers the two new-loader runs agree on (within 0.5%)
came out within 0.5% of them again; the same 7 slower than the floors
(`probe.lz4_compress_page` +6.1%, `lz4_compress_text` +4.1%,
`dma_copy_128k_b8_word` +4.7%, `cache_walk_40k` +1.0%, three by
0.1-0.2%), 15 faster (`cpu_unit_alone` and `cpu_unit_during_dma`
-16.7%). **The loader isn't it.** Correction to the line above: the
floors were set at 01:20 that day (`run-*-noise1..5`), 16 commits before
the first run that moved, so the cause is any of the images' code
changes since, not only C59's first change: C8's event counters in
`rp-probe` itself (`afb230c`), C11's CF polling switch, C43's clock
guard, C59 (a). Code placement is ruled out (the second run). Options
for the owner: (a) take the new numbers as the floors (`perf --update`
for the faster ones, approval for the 7 slower); (b) first find which
change moved the three that moved most, by running `rp-probe` built at
the floors' commit and at each suspect under today's loader (about 4
minutes a run, no reset, nobody at the device).

**The owner chose (b) (2026-10-22). Found:** `rp-probe` built at each
commit (the workspace build, as `perf` loads it), run twice on the X51v
under today's loader (`scratchpad` script, the rows parsed as `perf`
does); each commit's two runs agree, and the floors' commit gives the
floors back to the unit:

| Row (ns an iteration) | Floors (8efe37c) | 1708939 may-omit | 7fb0434 C6 | afb230c C8 | Today |
|---|---|---|---|---|---|
| `lz4_compress_page` | 201,691 | 214,191 / 214,038 | 214,076 | 213,884 | 213,903 |
| `lz4_compress_text` | 150,383 | 156,537 | 156,537 | 156,518 | 156,537 |
| `dma_copy_128k_b8_word` | 8,501,690 | 8,904,620 | 8,904,620 | 8,904,620 | 8,904,540 |
| `gfx_photo_row_480` | 32,476 | 30,183 | 30,183 | 30,183 | 30,168 |
| `lz4_decompress_page` | 149,682 | 146,513 | 146,504 | 146,504 | 146,513 |
| `frames_fill_drain_1024` | 430,399 | 420,583 | 420,583 | 417,322 | 420,583 |
| `cache_walk_40k` | 295,888 | 295,125 | 293,095 | 298,485 | 298,769 |
| `cpu_unit_alone` | 19,274 | 19,276 | 19,274 | 16,064 | 16,060 |

- **The three that moved most moved together at 1708939, the switch to
  frame pointers "may-omit" (ADR-0001 option B):** LZ4 compress +6.1% and
  +4.1%, the byte-wide DMA copy +4.7% slower, against the photo row -7.1%,
  LZ4 decompress -2.1% and the frame allocator -2.3% faster. ADR-0001's
  measurement ("nothing slower") missed them: quality audit A7.
- **`cpu_unit` (-16.7%) and `cache_walk_40k` (+1.8% from the commit
  before) moved at afb230c (C8)**, whose only change near them is code
  inserted before their section: neither loop changed, and both time
  with the OS timer, not the counters C8 programs. So both are sensitive
  to where their loop lands (the second run's ~1 KiB shift didn't move
  them; C8's did): their floors carry noise their tolerance doesn't show.
- Nothing else between the floors and today moves these rows (C6, C56,
  C59 and the walker: within the runs' agreement).

The floors are the owner's to set, with ADR-0001's options in its
"Measured again" note.

**The owner kept B and asked why the loops lose (2026-10-22).** The why
and the fixes are in ADR-0001 ("Why, and what was fixed"); the sweep
(four `rp-probe` builds, two runs each, then the DMA pause at 1,000,
3,000 and 10,000): the multiply's cost by size (docs/hw/cpu.md); LZ4
compress page 221.4 / 204.2 / 185.9 / 174.1 us at a 32 / 27 / 16 / 15-bit
multiplier (16 and 15 bits lose 0.7 points of compression); the 8-byte
copy 8.90 / 8.50 / 8.47 / 8.25 / 8.26 / 8.26 ms at 0 / 50 / 200 / 1,000 /
3,000 / 10,000 spins. **The suite after the fixes**
(`local/perf/`, newest): 119 numbers, 15 faster (the 8-byte copy -3.0%,
16-byte -4.9%, page decompress -10.4%), 8 slower: text compress +5.8%,
page compress +1.2%, `cache_walk_40k` +1.2% (placement, C8), five within
0.0-0.2%. Loop alignment (`-align-loops=32`) tried: no help (text +0.4%,
decompress +3.8%, +4 KB). Next for text compress: the event counters
(C8) on its loop in both modes.

**Text compress found (2026-10-22): the 8 KiB table in the loop's frame**
(ADR-0001, quality audit C65). The suite after the fix: 119 numbers, 16
faster, 4 slower: text compress +2.0% (the split's D-cache misses, both
modes alike) and three at 0.1-0.2% (`b9e` 16-bit copy, `ft_alone` 2D,
`gfx_flush_vga`) that were over before any of this. The floors are the
owner's to set.

**The last 2% of text compress, chased (the owner's option b, 2026-10-22).**
`rp-probe` counts the table's clear apart from the loop (`lz4c_*`,
`lz4w_*`; per 16 pages). Against the old build (frame pointers, loop
inline: 1,495,060 cycles), today's is 38,000 cycles slower: about 12,000
are the 8 KiB table's clear missing the cache (a clear of lines not in
the cache: 9,728 misses, 11,558 buffer-stall cycles, ~4,400 cycles a
clear; in the old build the clear mostly hit, by where the table sat on
the stack: same mapping, same clear routine), and about 26,600 are data
stalls in the loop (16,000 from the split's register allocation, 10,800
the mode's). Two reworkings of the loop measured worse and were dropped:
hashing a step ahead (LLVM moves the use back beside the multiply) and
carrying the word from one position to the next (one load instead of
four, but +35,000 instructions: text 157.2 us against 153.4); both
matched byte for byte (`same_output_as_the_plain_loop` stays as the
guard). The kernel's own path is `compress_with` on a table it keeps
(`kernel/mm/src/space.rs`), not this row's fresh stack table: options are
in the session's report to the owner.

## Not yet

Battery (no reading found yet: gate line P16), and the compositor
harness's numbers (T1, T4, T7) once it exists. Both join the floors when
they can be measured.
