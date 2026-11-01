# Option C on the X51v: the device-run plan

Option C of the Weather rule 0 redo ([weather-rule0.md](weather-rule0.md),
"Option C, as decided") is built and passes the emulator end to end; nothing
of it "works" until a run on the real X51v proves it (the owner's rule).
This is that run: what to load, what to do, and the log line that answers
each of the sheet's targets. Nothing starts until the owner says "Run it".

## What the run answers

| Target (the sheet) | The log line that answers it | Pass |
|---|---|---|
| A decode at size under 10 ms a picture | `icons: N pictures made in P passes, X us in all (slowest Y us: inflate A us, shrink B us; ...)` | the slowest Y < 10,000 (the first run: 73 ms, inflate 34-49 ms); else P2's fallback (decoded pictures kept on the card, with P4's file) is designed next |
| A pack pick under 0.1 s | `icons: pack N chosen: its table read in X us`, then the `icons: ... made` line that follows it | X plus the passes until the pictures on screen are in < 100,000 us |
| Weather's RAM under 8 MB | `ram: the image's parts of the RAM pool ...` (start) | the pictures' parts about 3.5 MB (store 1.49 MB, default pack 1.5 MiB, card stream 256 KiB, decode scratch ~0.19 MB; above the sheet's 2.4 MB by Hourly's 160 px x6 not x2, the graph's picture and the card stream); the whole of the image's pool use < 8 MB is the radar's and the pages' question too |
| Start to Weather shown under 0.5 s | `start: the Today screen shown at X ms after the image began` and `open: ... Reveal done` | measured, the baseline for P4 (snapshot start), whose design waits for this number |
| Frames during touch 30+ fps | `movement at ...` lines (written only for a movement under 30 fps over five frames or more) and `movement: N more at 30 fps or better ...` at the end | no `movement at` line for a drag on Hourly (the graph now one picture) |
| Stack: deepest under 75% | `stack: deepest X of 24576 bytes` | X < 18,432 |
| The default pack read and kept | `icons: the default pack read (1408989 bytes from the SD card in X us, kept in RAM)` | read; X recorded (about 1.4 s at the card's ~1 MB/s) |
| Hourly's graph one picture | `graph: Hourly's line painted in X us (in the store)` | in the store; X recorded |
| The radar refresh keeps frames (P7) | `radar: N frames kept by their time, the rest asked` after ten minutes on Radar | N = frames - 1 (typically 7), then 9 tile answers, not 81 |

Also read from the same log: every `slow frame` and `slow answer` line,
`frame N: full` (none expected: 208 commands, the busiest test frame 155),
and the store's use (`... of 33 slots`).

## The run, step by step (the owner's hands)

Before it: `cargo xtask load` puts the packs on the SD card (now only their
250 px pictures, ~27% smaller: `RPIC0A.PAK` and on) and the image. Then:

1. Loader up, "Run it". STEP 1 on screen: the Today screen; the weather
   pictures appear within about two seconds (the default pack being read
   from the card while nothing moves). Question: did the pictures appear,
   sharp, with no box where one should be?
2. STEP 2: tap the hero; Weather opens. Turn to Hourly. Tap three hours on
   the grid. Question: does the hour's big picture change at once with each
   tap, and is the graph's line smooth?
3. STEP 3: drag Hourly up and down a few times. Question: smooth?
4. STEP 4: Settings (right softkey), Icons, pick pack 2 (Golden Hour), back
   to Hourly. Question: did the pictures change at once?
5. STEP 5: pick VClouds again, Home. Question: back to the default pictures?
6. STEP 6 (only with `cargo xtask net` running, about 12 minutes): open
   Radar and leave it ten minutes. Question: did the radar keep showing
   while it refreshed?
7. Start menu, Back to loader. The log comes back to the PC.

## Not in this run (and why)

- **The CF card and the PC as the default pack's source** (b9b8a6c6): the
  SD card has the pack, so the image never falls back. A later run with
  `RPIC0*.PAK` taken off the SD card (the owner's choice, not done for him:
  the shared card) checks both; R152 covers the CF card the loader didn't
  attach.
- **A card pulled mid-run**: pulling the card while the image runs is the
  owner's call (it holds the card log); a run that does it reads `icons:
  ... the card failed, the rest from the default` and the pictures stay.
- **P4 (snapshot start)**: designed after this run measures what start
  costs now (the owner's rule: prove the cause first).
- **P5 (depth per picture)**: each depth is shown to the owner on the panel
  through ui-preview before adopting it; the pages are already 32-bit for
  Hourly and 16-bit-class (ARGB4444) for the others; the radar's rain at 8
  bits and its map at 16 are the open ones.
