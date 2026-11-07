# Sheet: R185's look check run on the X51v with no one watching (rule 0, before code)

The owner, 2026-11-06 (sick): "can you somehow automate this test ...
these tests you have been doing just need you to be able to see the logs".
R185's check (switch Light to Dark and back in Weather settings, every
page must show the new look at once) needs two things a person does
today: the taps, and eyes on the screen. Written before any code; the
owner's go first.

## What we already have

- **The taps:** `rp-today` replays a touch recording with no pen
  (`RP_TODAY_REPLAY`, `RP_TODAY_REPLAY_FILE`, pauses up to
  `RP_TODAY_REPLAY_GAP_MS`; `shell/today/build.rs`). The ratchet
  (`cargo xtask perf`) and the plane path's unattended runs used it on
  the X51v (`local/replay-optionc-run1.txt`, `-run2.txt`).
- **The log reaching the PC with no one there:** `cargo xtask load` with
  `RETROPANE_LOG_WAIT_SECS`, the card log as it goes (rp-cardlog), the
  image ending in the loader (Back to loader).
- **Missing:** a recording of this exact path, and something in the image
  that sees what the eyes see.

## What others do (findings, not the answer)

- **Windows Mobile / Dell:** Microsoft's CE Test Kit drives tests on the
  device from the PC and reads pass/fail from logs; screens are judged by a
  person or by comparing a whole captured screen with a stored one. Dell's
  ROM holds no UI test tooling (not searched with REA: test tools never
  shipped in the device's ROM; to log as a no-result search before the
  build).
- **Today (Android, iOS, web):** UI tests replay input (Espresso,
  XCUITest, Playwright) and compare screenshots with stored "golden"
  pictures, with a tolerance; flicker is found by recording video and
  checking every frame.

## Ours

1. **A recording of the path** written by hand from the screen's layout
   (no owner needed): Weather, Menu, Settings, Appearance Dark, back, Now,
   Hourly, 10 Day, Radar, the same back to Light, the pages again, Start,
   Back to loader. Proven first in the emulator (emu-today replays it and
   lands each tap where meant) before it ever runs on the X51v.
2. **Eyes in the image, two of them, independent:**
   - *The bookkeeping:* every frame handed over is checked: its page's
     picture and the chrome made in the look shown (`page_light`,
     `chrome_light`); a mismatch logged as `LOOK WRONG` with the frame.
   - *The pixels:* after every flip or copy, a few pixels of the shown
     screen read back (the page's background, the bars) and compared with
     the look's colours: catches what the bookkeeping can't (a picture
     tagged right but drawn wrong, a stale screen never replaced). Every
     frame, not a person's glance: a one-frame flash (16 ms) the eyes may
     miss is counted.
3. **Pass:** 0 `LOOK WRONG` of every frame across both switches, each page
   visited in each look, the run ending in the loader.

**Why ours wins:** golden screenshots break with every look change we make
(the owner keeps refining it) and need a stored picture per page; a
person's eyes miss one-frame flashes and cost the owner's time. Checking
each frame against the look's own colours needs no stored pictures, runs
every frame, and works the same for any future theme.

## What it can't do

- Start the loader: after a hang or a reset the owner starts it by hand
  (soft resets land in WM). One stuck run stops the unattended work until
  he's back.
- Judge taste: whether the dark look looks right is still his.
- Proof: a pass is a device measurement; whether it closes R185 without
  his eyes is the owner's ruling.

## What overturns it

A run whose log passes while the owner later sees a flash: then the pixel
probe reads the wrong place or the wrong moment, and is fixed first.
