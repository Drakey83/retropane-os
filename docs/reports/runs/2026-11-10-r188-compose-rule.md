# R188: the compose-ahead after a second still (owner's list 2026-11-10, item 2a)

- **Date:** 2026-11-10, afternoon
- **Device:** the owner's Dell Axim X51v (already on the loader; unattended)
- **Card logs:** today-0116, today-0117 (the touch replay), today-0118 (the hero tapped early), today-0115 (the look route: the still case)
- **Commands:** `RP_TODAY_REPLAY=1 RETROPANE_LOG_WAIT_SECS=240 cargo xtask load rp-today` (the ratchet's touch replay, twice); `RP_TODAY_REPLAY=1 RP_TODAY_REPLAY_FILE=target/reveal-early-taps.txt RETROPANE_LOG_WAIT_SECS=120 cargo xtask load rp-today` (one hero tap at 500 ms, the file made for this run); branch fix/deep-dive-2026-11-10 with item 1's cache and item 2a's rule
- **Source:** `local/cardlog/today-0115.txt` to `today-0118.txt` (not in git)

**The rule (the owner's ruling, 2026-11-10):** the Weather app is composed
ahead for Reveal only after Today has been still for 1 s
(`rp_today::ahead_due`, `STILL_BEFORE_AHEAD_MS`). A screen change, a
movement, the pen, or any change but the sky's restarts the second.

## The touch replay

| Build | Card log | Touch to flush median | Frame and queueing median |
|---|---|---|---|
| The floors' build (8067dbe2, 2026-10-30) | today-0083 | 16,248 us | 384 us |
| fix/deep-dive-2026-11-09, compose on | today-0106, -0108 | 20,268, 19,184 | 1,264, 1,260 |
| fix/deep-dive-2026-11-09, compose off | today-0107, -0109 | 18,396, 18,080 | 868, 852 |
| **This branch (the rule, item 1's cache)** | **today-0116, -0117** | **16,912, 18,620** | **784, 784** |

The replay taps about every second, so the rule keeps the compose out of
its frames. Frame and queueing fell from 1,260 to 784 us, below the
compose-off figure of 11-09 (852-868), which item 1's picture cache helps
too. Touch to flush is back toward the floor (16.9 and 18.6 ms against
19.2-20.3). The two runs' frame-and-queueing medians are identical but
their touch to flush differs by 1.7 ms. Touch to flush includes the wait
for the display's refresh, so one run against another moves with where
the taps land in the refresh. The frame-and-queueing figure is the steady
one; the second bisect judges by it.

## Reveal

| Case | Card log | The opening's line | |
|---|---|---|---|
| Still: the hero tapped 12 s after Today settled | today-0115 | "the app composed ahead matches the app now" | Reveal 33 window moves in 540 ms (~61 fps) |
| Within the second: the hero tapped 531 ms after start, Today still drawing its pictures | today-0118 | "the app composed ahead not yet: composed now, aside" | the old way: that tap's touch to flush 126,020 us (the freeze the owner called clunky) |

today-0118's image returned as its one-tap replay ended, so the opening's
own end wasn't logged. The opening line names which way it took.

**What it means:** the rule does what the ruling asked. The compose no
longer costs the taps on Today (frame and queueing 784 us, the compose's
~400 us gone). Reveal opens at once when Today has been still, and the
old way only when the hero is tapped within a second of a change. The
other ~2 ms against the floor (frame and queueing 784 against 384-496 us)
is the second bisect's question (item 2b).

## The second bisect (item 2b): the compose held off in every build

The owner's ruling (2026-11-10): a second bisect with the compose-ahead held
off per commit. The patch turned the compose block's `if app_check` into
`if false && app_check` in every build, from 4fa4dbad (the compose's own
commit) to 04d3d142 (main), 170 commits, first parent, on the X51v
unattended (card logs today-0119 to today-0136). The judge was the
frame-and-queueing median, the steady figure. Touch to flush moved 1.7 ms
between two identical runs today, more than the gap being looked for.
Bad meant over 680 us, and a reading in 600-760 was measured twice.

| Commit (index of 170) | Frame and queueing median | Touch to flush median | Card log |
|---|---|---|---|
| 4fa4dbad (the start) | 508 us | 16,472 us | today-0119 |
| [41] f608326d | 568 | 16,668 | today-0124 |
| [62] 7fcf9bb6 | 652, 656 | 17,732 | today-0125 |
| [67] b5146dce | 660, 656 | 16,704 | today-0130 |
| [70] 6d53a05f | 644, 644 | 16,716 | today-0131 |
| [71] 887d88f1 | 636, 652 | 16,708 | today-0133 |
| **[72] f4ffea28** | **712, 708** | 16,772 | today-0136 |
| [73] dc6c17c9 | 712, 712 | 16,772 | today-0127 |
| [84] e7b2696c | 1,200 | 19,600 | today-0121 |
| [85] b4f40c68 | 808 | 16,812 | today-0122 |
| 04d3d142 (main, the end) | 752 | 16,704 | today-0120 |

**The first commit over the line: f4ffea28** (2026-11-01, "The opening's
flash and sliding box fixed from the trace, whole clouds crossing the panel,
the card's word shadows, 16 glyph atlases ..."), +60 us on the commit
before it.

**What it attributes, and what it doesn't:**
- There is **no single cause.** With the compose off, frame and queueing
  climbs in steps across the features of 2026-10-30 to 2026-11-01:
  +60 us by commit 41, +85 by 62, +60 at f4ffea28. Commit 84 spiked to
  1,200 and later commits brought it back down. Main ends at 752, 244 us
  over 4fa4dbad. f4ffea28 is the one step that crossed the threshold:
  about a quarter of the rise.
- **Most of 2026-11-09's "other ~2 ms" was spread in touch to flush,
  not cost.** With the compose off, touch to flush reads 16,472 us at
  4fa4dbad and 16,704 at main (0.23 ms), and 16.7-16.8 ms at nearly every
  commit between. The 18.1-18.4 ms of 11-09's compose-off runs and the
  16.9 / 18.6 ms pair today differ by where the taps land against the
  refresh. The steady figure, frame and queueing, shows a 0.24 ms rise,
  spread over many commits.
- This branch measures 784 us of frame and queueing (today-0116, -0117)
  with the rule and item 1's cache, against 752 for main with the compose
  off: the rule costs about 30 us over never composing.

The bisect's first start lost two readings to a check of mine that called
the loader's backlog answers stale (the runs took over 300 s, and the
loader handed each log back by its new card number). Their readings are
in the table (04d3d142, [84], [85]). The restart's leftover processes were
ended before a clean run; none of the killed steps reached the device.
