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

## As built (the owner's "Yes, go", 2026-11-06)

- **The recording:** `cargo xtask look-replay` drives the shell itself on
  the PC (its own button places, `Shell::rect_of`, and the Appearance
  row found where the screen lays it out, `Shell::appearance_row`) with
  the device's settings (`local/weather-settings.bin`), checking each
  step lands on the screen meant, and turns each tap into the touch
  panel's raw readings through the X51v's default alignment (every point
  maps back exactly: a test). Appearance is tapped three times round
  (Auto, Light, Dark in turn), so the setting ends where it began and both
  switches, light to dark and dark to light, are seen. Two recordings:
  `target/look-check-taps.txt` (the device's settings) and
  `-emu.txt` (the defaults the emulator starts with).
- **The eyes:** `layers::shown_at` rebuilds the pixel the panel shows at a
  point from the display's live registers and memory (the graphics plane,
  the video plane over it by its slot, window and blend mode; scaled or
  YUV: unknown), `Graphics::shown_at` lends it the service's registers
  and memory. `rp_today::look` judges the look from fitted points; the
  image (`RP_TODAY_LOOK_CHECK`) checks every pass while Weather is up and
  nothing is in flight, logs `LOOK WRONG` with the points' brightness, and
  a summary at the end (right per look and page, wrong, unknown).
- **The verdict:** `lookcheck::judge`: no wrong pass, both looks judged
  right on every page, at most a tenth of the passes unknown. In the
  emulator: `cargo xtask emu-today-look`.

## The emulator's runs (2026-11-06)

- **The points fitted** from 55 shots of the replay (`emu-today-watch`):
  the tab bar's five points split the looks by 187-206 brightness levels,
  each page's nine glass points by 68-95; page and chrome vote apart (two
  thirds of the points read).
- **First run: 27 passes flagged, none a look bug.** All on 10 Day just
  after its hand-over, the page points near black: the evidence logged
  per plane showed the layer's window 2, then 48 rows tall (the page
  growing in) and the points below it reading the wallpaper. Fixed in
  the check: a page on the layer is judged only where the layer covers it.
- **Pass:** `look check: 0 wrong; right: dark on pages 0-3 [497, 249,
  243, 220], light on pages 0-3 [207, 127, 112, 111]; 72 passes unknown`
  (4%).
- **The check bites:** with R185's fix taken out (no rebuild ahead on a
  look change, no rebuild at the hand-over, the chrome not filtered),
  `2 wrong`: at 21,292 ms the Now page dark under the light chrome, at
  43,249 ms the Now page light under the dark chrome: the owner's flash,
  both ways. The probe's own mutant is in `cargo xtask mutants`;
  `emu-today` now runs `emu-today-look` too.

## The X51v's runs (2026-11-06, unattended)

- **r185 run 1** (today-0062, dock link on): `2 wrong`, each a single pass
  on Hourly in the dark look while the page grew in (window 198 and 202
  rows): the pixels were the dark glass (0x484850 at alpha 0xE6, the
  dark look drawn right), the points on its graph while the content moved
  under them. Fixed in the check: the page is judged only at rest
  (`Shell::animating`), the chrome always; the emulator's unknown passes
  went from 72 to 0 and the fix-removed run still fails both ways.
- **r185 run 2** (`local/rp-today-run-2026-11-06-r185a.txt`, dock link on):
  `0 wrong`, but the light look only on Now: the relay's settings arrived
  60 s in and set Appearance back mid-round. The device's runs go without
  the dock link: the settings only from the card, nothing saved back.
- **r185 run 3** (`local/rp-today-run-2026-11-06-r185b.txt`, no dock
  link): **`look check: 0 wrong; right: dark on pages 0-3 [325, 98, 94,
  188], light on pages 0-3 [134, 48, 50, 97]; 0 passes unknown`**: dark to
  light at 40.6 s, light to dark at 62.5 s, every page in both, the run
  back at the loader by itself.
- **The command:** `cargo xtask look-replay`, then `RP_TODAY_REPLAY=1
  RP_TODAY_REPLAY_FILE=target/look-check-taps.txt RP_TODAY_REPLAY_GAP_MS=12000
  RP_TODAY_LOOK_CHECK=1 RETROPANE_LOG_WAIT_SECS=360 cargo xtask load rp-today`.

## Committed run reports

The device-run evidence this sheet quotes, committed (the logs in `local/` are not in git; [the rule](../../reports/runs/README.md)):

- today-0062: [2026-11-06-today-0062-r185-run-1.md](../../reports/runs/2026-11-06-today-0062-r185-run-1.md)
- r185a: [2026-11-06-r185a.md](../../reports/runs/2026-11-06-r185a.md)
- r185b: [2026-11-06-r185b.md](../../reports/runs/2026-11-06-r185b.md)
