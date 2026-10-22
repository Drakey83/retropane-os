# X51v touchscreen (TI TSC2046 on PXA27x SSP1)

> Spec v0.1, 2026-10-10. Facts only, per [`../CLEANROOM.md`](../CLEANROOM.md).
> **Sources:** (D) TI TSC2046 datasheet; (M) Intel PXA27x Developer's Manual
> (SSP and GPIO chapters); (C) AxDroid board code (working touch on X50/X51).

## Wiring (C)

| Signal | Connection | Confidence |
|---|---|---|
| Data | PXA27x **SSP1**: GPIO23 SCLK, GPIO25 TXD, GPIO26 RXD (SSP1 alternate functions) | likely |
| Chip select | **GPIO24** as a plain GPIO output, active low (not the SSP frame signal) | likely |
| Pen interrupt | **GPIO94**, active low while touched | likely |
| Axes | X and Y are swapped relative to the screen | likely |
| Clock | ≤ 2.6 MHz (AxDroid ran it at 100 k samples/s × 26 clocks) | likely |

AxDroid's settings: internal reference with 100 µs settle, 150 µs input settle.

**As Windows Mobile leaves the pins (P, probe 2026-10-11, `GAFR0U =
0x001A851A`; consequence seen 2026-10-13):** GPIO23 function 2 (SSP1
clock), **GPIO24 function 2 (SSP1's frame signal)**, GPIO25 function 2
(transmit), GPIO26 function 1 (receive); SSP1's clock is on (`CKEN` bit
23). So WM drives chip select with the SSP's own frame. RetroPane drives
it as a GPIO across the three bytes of a conversion and must first make
GPIO24 a plain GPIO (`Gpio::set_function(24, 0)`, after setting it high
as an output): without that, every reading came back 0 and every tap
landed at the top left (`rp-today`, the first device test of touch).

## Measured on the owner's X51v (2026-10-13, `rp-today`'s run logs)

**Settling (P).** With the panel powered down between conversions (PD =
00), the X plate read 104-265 wherever the pen was: every tap at the top
edge. With the converter kept on (PD = 01), four back-to-back conversions
still climbed steadily (X 66, 412, 589, 669; Y 3804, 2475, 1761, 1357).
Read until two conversions in a row agree within 8, a channel took 2 to
20 conversions (~10 us each at 2.6 MHz): the lines settle over 20-200 us,
starting near 0 (X) or near 4095 (Y). The approach is geometric (about
30% of the gap a conversion), so "within 8" still stopped several pixels
short; the driver now waits for two conversions within 2 (or 64 at most)
and powers down only on the last conversion of a sample.

**Orientation and range (P),** from taps at the four corners and the
centre:

| Tap | Raw X | Raw Y |
|---|---|---|
| top left | 150 | 3645 |
| top right | 135 | 417 |
| bottom left | 2622 | 3650 |
| bottom right | 2793 | 364 |
| centre | 1423 | 2017 |

The X channel follows the screen's vertical axis, the Y channel the
horizontal one **mirrored** (high at the left). Linear: the centre falls
where a straight line puts it. `Calibration::X51V` takes the edges as
X 99..2741 (top to bottom), Y 3719..319 (left to right), assuming the
corner taps landed ~10 px inside. A calibration screen (item 76) will
refine it per device.

**Latency (P):** touch to flush, worst 35 ms over 5 answered taps
(the promise: 50 ms).

**The landing (P, later runs the same day).** Even 128 settling
conversions (~1.3 ms) left the X channel creeping (2535, 2627, 2716,
2864 at 16, 32, 64, 128 conversions) for the first tens of milliseconds
of a touch, by a different amount each tap: the contact firming up as the
stylus lands, not a capacitor charging. And a sample taken as the stylus
lifted read X = 0 (the top of the screen). Taps drifted upward at random.
After the landing, both channels hold their value within 16 conversions
(X 2314, 2326, 2327, 2325). So: `rp_touch_cal::Filter` ignores the first
15 ms of a touch, reports a position only when three consecutive samples
agree (within 14 raw), and lifts at the last stable position; the driver
settles X for 64 conversions and Y for 32, then averages 8; samples are
taken back to back while the pen is down (with 5 ms sleeps between them
the first answer took a median of 54 ms). The owner: "much better".

**Calibration (P):** the align screen (affine fit, `rp-touch-cal`), four
alignments in a row agreeing within 0.3% in scale and ~9 px in vertical
offset. The result is `Calibration::X51V_DEFAULT`: the X channel runs
from ~195 at the top to ~3207 at the bottom (the earlier 99..2741 came
from corner taps that weren't at the edges), the Y channel ~3696 at the
left to ~330 at the right, with a slight skew.

**Tap to press (R, replay of the recordings, 2026-10-14).** The GPU Today
screen's log split a tap's 35.5 ms (median, runs 3-4) as landing to the
filter's verdict 22.2 ms, CPU 0.8 ms, engine to flip 7.1 ms. From the `R`
lines of six sessions (230 touches, `drivers/touch-cal/tests/data/`):

- While the pen is down the loop runs every **11 ms** (median of 3,562
  gaps): the 5 ms sleep plus ~6 ms of loop, almost all of it the sample's
  329 conversions (X settles for 256 and averages 8, Y 32 + 8, Z1 16 + 8,
  Z2 1; three polled SPI bytes each). When the loop slept 1 ms (a frame
  with the engine) the gaps were 7 ms.
- The rule of 2026-10-13 pressed on the **third** sample (pressure steady
  twice) in 188 of 225 taps (the rest later): two periods, 22 ms. That
  is the log's "landing to verdict": it is stamped before the first sample's
  conversions and before the deciding sample's, so the deciding sample's
  own ~5.4 ms shows only in the total (35.5 - 22.2 - 0.8 - 7.1).
- The interrupt found every touch (38, 38 and 25 in runs 2-4; the 50 ms
  idle check none). The time from the pen's edge to the first sample is
  not in the log (waking from idle takes microseconds; a frame being
  drawn can delay it).

Scored by replaying every recording through each rule (time from the
first sample with pressure to the press; "far" = pressed more than 20 px,
the larger axis, from where the stylus was released; five of the touches
are deliberate drags, so 5 is the floor):

| Rule | Median | p90 | Worst | New taps | Lost | Far |
|---|---|---|---|---|---|---|
| 2026-10-13: pressure steady on two samples in a row | 22 ms | 33 | 44 | - | - | 5 |
| **pressure steady and the position where the previous sample was (within 20 raw), else as before** | **11 ms** | **23** | **44** | **0** | **0** | **5** |
| the same within 12 raw | 22 | 23 | 44 | 0 | 0 | 5 |
| the same within 30 raw | 11 | 23 | 44 | 0 | 0 | 6 |
| the same, and pressure not falling | 22 | 23 | 44 | 0 | 0 | 5 |
| pressure steady once (no position check) | 11 | 22 | 33 | 2 | 0 | 13 |
| the first sample with pressure | 0 | 0 | 0 | 5 | 0 | 40 |

**Adopted: the second row** (`rp_touch_cal::Filter`). Most landings are
in place by the second sample; the ones still sliding (X creeping
~24-190 raw a sample, a pressure dip as the stylus bounces) fail the
position check and are pressed by the old rule a sample later. No press
moved more than 19 px from where the old rule put it, and the two that
moved most are better: the old rule took them on a pressure spike or
while pressure was already falling. Expected on the device: the median
landing to verdict 22 -> 11 ms, a tap answered in ~24.5 ms instead of
35.5 (not yet measured there). `tests/replay.rs` replays all six
sessions and fails if the median passes 11 ms, p90 23, worst 44, if a
tap appears or is lost, or if more presses land far.

Rejected:

- *Show the press on the first sample, confirm or move it later.* The
  first sample is more than 20 px from where the old rule pressed in 34 of 225
  taps (15%): about one tap in seven would light a neighbouring button
  first, and a press outside an open menu closes it, so the mistake would
  act, not just flicker.
- *Pressure steady once, no position check.* 2 brushes become taps and
  13 presses land far (a landing still sliding, up to 37 px).
- *Sample faster while landing* (sleep 1 ms until the press, ~7 ms
  periods). Not replayable: the recordings are at 11 ms, and the rule's
  pressure and position thresholds are per sample, so they mean something
  else at another rate (a creep of 24 raw per 11 ms is ~15 per 7 ms and
  would pass the 20 raw check while still sliding). 1 ms sleeps were
  "much worse" in the build of 2026-10-13. Needs a recording at that rate
  first.
- *Cheaper samples* (fewer X settling conversions, ~80% of each sample's
  time): would shorten the period and the deciding sample's ~5.4 ms. Not
  replayable: the log keeps only the settled value, not the curve; 64
  conversions "with faster sampling" was worse on 2026-10-13, with two
  changes at once. Needs a recording of X at 64, 128 and 256 conversions.

**On the device (P, 2026-10-14, log `local/rp-today-touch-merge.txt`,
now the seventh recording `owner-2026-10-14-merge.txt`).** The owner: "it
feels great". The log: landing to verdict median **22 ms** (worst 44), no
faster. Only 6 of 31 taps pressed on the second sample: in this session
the second sample was within 20 raw of the first in 9 of 31 landings (29%;
the six earlier sessions 129 of 226, 57%), the stylus sliding 20-70 raw
(~3-10 px) between them, many on the alignment crosshairs (two alignments
this run). No tap was lost or added and none pressed far. The replay of
all seven sessions: median 12 ms, p90 23, worst 45 (the guard's numbers
now). Touch to flush median 55 ms is not comparable with run 4's 35.5:
the two alignments rebuilt the background 27 times (run 4: once).
Next: the sliding landings, and faster samples (both need a recording
at the new rate first).

**Faster samples, recorded (P, 2026-10-14, log
`local/rp-today-touch-fast.txt`).** 1 ms sleeps while the pen is down,
each sample also logging X as settled 16/32/64/128 conversions and Y 16.
The owner: "working really well". The log (39 touches, 2 drags, no
alignment):

- Samples every **7 ms** (median; p10 6, p90 8).
- Landing to verdict median **7 ms** (worst 27): 25 of 36 taps pressed on
  the second sample. Touch to flush median **20.4 ms** (worst 63; run 4
  35.5).
- No tap pressed more than 20 px from where it was released (only the two
  drags).
- **X settled 16 conversions is as steady as 256**: sample to sample
  while held (586 pairs), X's change median 5 raw, p90 14 at 256; 5 and
  13 at 16; 5 and 14 at 32. Y at 16 as at 32 (2 and 12). The early value
  differs from the kept one by more (p99 120-300 raw) only while the
  stylus moves within the sample. The 64-conversion build that was
  "much worse" on 2026-10-13 changed two things at once, with the filter of
  then. A sample could be ~1/3 of today's conversions (X 32 + 8 instead of
  256 + 8): ~2 ms instead of ~6.
- The run's "touches found by the interrupt 376" counted samples, not
  touches (282 light-pressure samples before landings); now counted once
  a touch.

**Shorter samples (P, 2026-10-14, log `local/rp-today-touch-short.txt`).**
X settles 32 conversions instead of 256 (~2 ms samples), 5 ms sleeps: a
sample every **7 ms** (p10 and p90 7). 40 touches, 3 of them drags:

- Touch to flush median **15.0 ms** (the 1 ms-sleep run 20.4, run 4
  35.5). Worst 101 ms: the slow frame every run has (CPU queueing worst
  54 ms, 54 in run 4), not touch.
- Landing to verdict median **7.0 ms**, worst **14** (27 before): 19 of
  36 taps pressed on the second sample, 17 on the third.
- Held still, X changes less sample to sample than with 256 conversions
  (median 4 raw, p90 10; was 5 and 14): a shorter sample has less time
  for the stylus to move within it. Y median 1, p90 5.
- Only the three drags pressed more than 20 px from where they ended.

**Adopted:** X and Y 32 conversions, Z1 16; 5 ms sleeps while the pen is
down.

**The recorder's gap (fixed 2026-10-14).** A sample with no pressure was
logged only when it ended a reported touch, so a touch that lifted before
its press ran on into the next one in the log. `rp-today` now logs the
sample that ends any touch. The older recordings are replayed with a gap
of more than 30 ms before any report taken as that lift; with it the
2026-10-13 rule reproduces the device's own verdicts on runs 2-4 exactly.

**The 16-bit screen's run (P, 2026-10-22, the owner tapping; quality
audit C58).** Touch to flush median 35.5 ms, landing to verdict 31.9 ms
(the short-sample run: 15.0 and 7.0). Its `R` lines: while the pen was
down the loop ran every **18 ms** (median), against 7 ms in every run
since the short samples (933+ gaps each in three runs). The run differs
from those in the card log (on since 2026-10-20), which writes a sector
at every logged line's end and waits for the card to program it, and the
recorder logs a line every sample (C59). (The 18 ms came from the
session record's last 14 gaps; the whole run's card log,
`local/cardlog/today-0001.txt`, says **11 ms** median over 223 gaps, p10
10, p90 18.) **Confirmed on the X51v (2026-10-22, the touch replay,
`RP_TODAY_REPLAY`):** with the recorder off the loop runs every **6.95
ms** (median), landing to verdict 8.9 ms, touch to flush 13.3 ms (worst
44.3); with it on (`RP_TODAY_REPLAY_RECORD`, the card log's option (a)
already in) 10.6 ms, 17.9 ms and 30.4 ms. The 16-bit screen didn't slow
touch; logging every sample did.

## TSC2046 protocol (D)

SPI mode 0. A conversion takes 24 clocks; with 8-bit frames that's three transfers:

1. Control byte: `[7]` S = 1, `[6:4]` A2..A0 channel, `[3]` MODE (0 = 12-bit,
   1 = 8-bit), `[2]` SER/DFR (0 = differential), `[1:0]` PD power-down mode.
   The device returns 0.
2. Returns a busy bit (0), then D11..D5.
3. Returns D4..D0, then three zero bits.

So `result = ((byte1 << 8 | byte2) >> 3) & 0xFFF`.

Channels (differential): **5 = X position, 1 = Y position, 3 = Z1, 4 = Z2**.

## PXA27x SSP (M)

SSP1 base `0x41000000`, clock enable `CKEN` bit 23.

| Register | Offset | Fields used |
|---|---|---|
| `SSCR0` | 0x00 | `[3:0]` DSS = data bits − 1, `[5:4]` FRF (0 = Motorola SPI), `[7]` SSE enable, `[19:8]` SCR: bit rate = 13 MHz / (SCR + 1) |
| `SSCR1` | 0x04 | `[3]` SPO clock polarity, `[4]` SPH clock phase (both 0 for mode 0) |
| `SSSR` | 0x08 | `[2]` TNF (transmit FIFO not full), `[3]` RNE (receive FIFO not empty), `[4]` BSY |
| `SSDR` | 0x10 | data FIFO |

## PXA27x GPIO (M)

Base `0x40E00000`. Banks of 32 pins: bank *n* = pins 32*n*..32*n*+31.

| Register | Banks 0-2 | Bank 3 |
|---|---|---|
| `GPLR` level | 0x00 + 4*n* | 0x100 |
| `GPDR` direction (1 = output) | 0x0C + 4*n* | 0x10C |
| `GPSR` set output high | 0x18 + 4*n* | 0x118 |
| `GPCR` clear output low | 0x24 + 4*n* | 0x124 |

## Emulator

retropane-qemu's `tsc2046` model implements the byte-level protocol above (not
QEMU's simplified ads7846), on SSP1 with GPIO24 chip select and GPIO94 pen
interrupt. Touches come from the emulator's absolute pointer (mouse button =
pen down), mapped linearly to raw values 200..3900 on both axes, with X and Y
swapped like the real panel.
