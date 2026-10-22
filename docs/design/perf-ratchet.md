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
| Touch | tap to screen, landing to verdict, frame, queue to flip, the pen-down loop's period (median, worst) | `rp-today` built with `RP_TODAY_REPLAY`: recorded taps replayed (see "Taps" below), its `TR` line | built 2026-10-22, emulator-checked; floors wait for its first device runs |
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
520 periods. The first device runs set the floors, and the same run
answers C58: the replay's period on the device, with and without the
recorder.

## Not yet

Battery (no reading found yet: gate line P16), and the compositor
harness's numbers (T1, T4, T7) once it exists. Both join the floors when
they can be measured.
