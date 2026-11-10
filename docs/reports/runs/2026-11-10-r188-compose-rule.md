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
