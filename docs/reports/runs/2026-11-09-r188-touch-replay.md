# R188: the touch replay's slowdown, bisected on the X51v (2026-11-09)

- **Date:** 2026-11-09, late morning to early afternoon
- **Device:** the owner's Dell Axim X51v (already on the loader; unattended, the owner's list of 2026-11-09 item 4)
- **Card logs:** today-0083, today-0094 to -0109
- **Command:** `RP_TODAY_REPLAY=1 RETROPANE_LOG_WAIT_SECS=240 cargo xtask load rp-today` (the ratchet's touch replay), the bisect from a second checkout (`../rp-bisect`) with `RP_TODAY_WALLPAPER=sunset` and each commit's own default pictures; the measurement variants `RP_TODAY_REPLAY_HOLD_MS` and `RP_TODAY_NO_APP_AHEAD`
- **Source:** the load logs (scratch files of the session, the `TR` lines copied below)

## What the owner's brief assumed, and what was found

The brief: bisect the commits between the Nov 7 and Nov 8 builds. The
floors the Nov 8 re-measure was held to weren't from Nov 7: they were set
on 2026-10-30 (commit 8067dbe2, "The touch floors the owner approved",
runs today-0016 to -0018), and no touch replay ran between then and Nov 8.
So the range is 8067dbe2 to aa8d9a4c, 254 commits.

## Measured

| Build | Card log | Touch to flush median | Frame and queueing median |
|---|---|---|---|
| 8067dbe2 (the floors' build, its own pictures) | today-0083 | 16,248 us | 384 us |
| The bisect, commit 64 of 254 (ca2ce776) | today-0099 | 16,428 | 480 |
| 95 (3c7eaf42) | today-0100 | 16,448 | 488 |
| 102 (6d53dde6) | today-0102 | 16,508 | 500 |
| **103 (e75a61c7)** | today-0105 | **16,508** | **496** |
| **104 (4fa4dbad)** | today-0104 | **40,756** | **1,596** |
| 107 (4091205c) | today-0103 | 35,340 | 1,588 |
| 110 (54f26b43) | today-0101 | 27,796 | 1,548 |
| 126 (46a507ad) | today-0098 | 25,948 | 1,564 |
| (62, 63, 106 don't build today: past their size budgets with today's compiler) | | | |
| The branch, at once | today-0094, -0096 | 20,852, 21,868 | 1,244 |
| The branch, the replay held 10 s (icons all made first) | today-0095, -0097 | 21,468, 21,492 | 1,244 |
| The branch (packs back on the card) | today-0106, -0108 | 20,268, 19,184 | 1,264, 1,260 |
| The branch, the app not composed ahead | today-0107, -0109 | 18,396, 18,080 | 868, 852 |

**First slow commit: 4fa4dbad** (2026-10-30 22:40, "Reveal without the
frozen start or the double hand-over"): rp-today composes the Weather app
ahead into a picture whenever the Today screen is still, so a tap opens
Reveal at once. Later commits cut its cost from 40.8 ms to about 19-20 ms.

**What it means:**
- **Ruled out:** the start-up icon work. Holding the replay until every
  picture is made changes nothing (21.5 ms either way).
- **Ruled out:** the pictures alone. A run whose card held no usable pack
  was as slow.
- **Proven, about half the rise: the compose-ahead.** Turned off, the
  touch-to-flush median falls 1.5 ms (19.7 to 18.2 ms) and frame-and-
  queueing 400 us (1,262 to 860 us). Composing the app draws a whole app
  frame through the 2D renderer. That replaces its background picture (the
  app's glass instead of Today's) and pushes Today's pictures out of the
  256 KiB picture cache, so the next Today frame rebuilds its background
  and uploads its pictures again.
- **Not yet attributed: the other ~2 ms** (18.2 against 16.2 ms) and
  ~470 us of frame-and-queueing. The bisect's threshold found only the
  first crossing. The commits after 104 change both the compose's cost and
  Today's own drawing, so this needs a second bisect with the compose held
  off in every build, which old commits can't do.

The fix for the proven part changes how Reveal behaves, so it waits as
the owner's tick-box (docs/reports/decisions-2026-11-09.md).

## Lines

```text
8067dbe2 TR touch replay: touch to flush median Some(16248), touch to flush worst Some(57468); landing to verdict median Some(8944), landing to verdict worst Some(15912); frame and queueing median Some(384), frame and queueing worst Some(62396); queue to flip median Some(4963), queue to flip worst Some(17655); pen-down period median Some(6964), pen-down period worst Some(11628) (us; 52 answered, 28 presses, 520 periods)
[103 of 254] e75a61c768 16508 496 card log today-0105
[104 of 254] 4fa4dbadc2 40756 1596 card log today-0104
FIRST BAD: 4fa4dbadc2f61786fa001c231682aa0899087c09 (after e75a61c7688f40358ed52c529ddbf5996ac2c1cd)
today-0106 tf 20268 worst 78572 fq 1264   (the branch)
today-0107 tf 18396 worst 67960 fq 868    (no compose-ahead)
today-0108 tf 19184 worst 67616 fq 1260   (the branch)
today-0109 tf 18080 worst 70172 fq 852    (no compose-ahead)
```
