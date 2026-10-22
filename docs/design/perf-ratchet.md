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
| Touch | tap to screen (median, worst) | `rp-today`'s run log ("touch to flush"), taps by a person | exists; needs taps (see "Taps" below) |
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
touch and frames (taps), battery (P16), the harness's numbers (T1, T4,
T7).

## Taps

Touch numbers need taps on the device. Options, for the owner: a person
taps a fixed script when the suite runs (watched runs only), or the image
replays recorded pen samples through the same filter and drawing path
(no taps, but the touch controller's own timing is skipped). Proposed:
replay for the ratchet (repeatable), real taps at milestones.

**Decision (owner, 2026-10-22): the hybrid.** Replay for the ratchet, real
taps at milestones: a ratchet lives on repeatability, and person-taps would
make the floors noisy; the touch controller's timing that replay skips is a
constant offset, and the ratchet measures changes. To build: recorded pen
samples replayed through the same filter and drawing path in `rp-today`,
its numbers into the floors.

## Not yet

Battery (no reading found yet: gate line P16), and the compositor
harness's numbers (T1, T4, T7) once it exists. Both join the floors when
they can be measured.
