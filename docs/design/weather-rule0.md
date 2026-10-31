# Weather, rule 0 redo: smart, small and fast, the look kept

The owner, 2026-10-31, after run today-0037 crashed: "I feel like this whole
app needs a rule 0 optimization redo ... once we establish these cheap
clever practices we will be able to keep using them into other parts of the
UI/UX design process." This sheet is the plan. Nothing in it is built until
the owner says go. Its practices (P1-P7) are meant for the whole shell.

## Where it stands (measured)

| What | Now | Source |
|---|---|---|
| Icon packs in RAM | all 7 unpacked, 5.3 MB each: ~37 MB of the 48 MB pool | pack tables, R146 build |
| A pack's first pick | 1.4-2.2 s read + 1.3 s unpack (v1) | today-0035, -0036 |
| Page pictures (graphics memory) | up to 7.5 MB: Hourly 2.3 MB at 32-bit, Now and 10 Day 0.7 MB each, two 0.6 MB previews | page log lines |
| Radar | map 0.9 MB + 8 rain frames 7.1 MB, 32-bit | radar buffers in main.rs |
| Stack | overflowed 16 KiB and 24 KiB (0036, 0037); the shell made on the stack at start took a 15.8 KB frame | -Zemit-stack-sizes, card logs |
| Code | 0.69 MB | cargo xtask size |

The weather changes only at a refresh (15 min to 2 h). Between refreshes
nothing on these screens needs the CPU except moving pictures already made.

## How others did it (leads from memory, not verified)

- Windows Mobile weather apps (SPB Weather, HTC TouchFLO weather,
  WeatherPanel-style today plug-ins): an icon set is a folder of picture
  files on the storage card, one per condition, often one per size. The app
  opens only the files for the conditions on screen when the forecast
  changes and keeps those few decoded. A skin switch re-reads a handful.
- Modern phone apps: images fetched lazily, decoded at the size shown, kept
  in a small in-memory LRU plus a disk cache; screens composed into layers
  once and moved by the GPU; a cold start shows the last screen saved (a
  snapshot) before any work.
- The X51v: 48 MB pool, 8.4 MB graphics memory, no FPU, a 2D engine that
  moves rectangles and pictures well, an SD card at about 1 MB/s with fast
  seeks.

## The practices (the two combined, fitted to the X51v)

- **P1. Work at refresh, never during touch.** Fetch, parse, pick pictures,
  compose pages, decode radar: once after a refresh, in idle slices. Touch
  only moves pictures already made.
- **P2. Read only what is on screen, from the card, by offset.** Pack v2
  already stores each picture as its own stream with an offset table. At a
  refresh the shell lists the conditions on screen (usually 3-8 of 22) and
  reads just those (one seek each), decoded at their size into a small
  cache (16 pictures, about 1 MB). A pack pick re-reads those few: tens of
  ms instead of 3 s, and ~1 MB instead of 37 MB. WM's folder of files plus
  the modern decode-at-size cache.
- **P2a. The default icons without a card** (the owner, 2026-10-31: "a
  default fall back ... if there is no sd card or cf card, but those
  versions won't have icon packs available to switch to"). The default set
  (VClouds) is also kept in the OS's own storage, in the same v2 format and
  read through the same on-screen-only path, so a card-less device costs
  the same small cache, not a second way of drawing. Its home: the flash
  file system once RetroPane is installed (bare metal first); the image
  itself at the SD stage. To fit the image area it stores two sizes (250
  and 100 px) and makes the 66 and 40 px ones while decoding, with a
  downscale filter fitted offline to these pictures (measured on the panel
  against the stored sizes before adopting). Packs are looked for on the
  SD card, then the CF card. With neither, Settings > Icons shows only the
  default and says "More icon packs need an SD or CF card"; a card pulled
  mid-run keeps the pictures already decoded and falls back to the default
  for the rest, with no error box.
- **P3. Compose once, then drop the parts.** Each page picture is composed
  at refresh; its icons, curve pieces and text are then given back until
  the next refresh. Only the page pictures stay.
- **P4. Snapshot start.** The composed pages and Today are saved to the
  card at each refresh (a few hundred KB, packed). The next start shows
  them at once, then refreshes.
- **P5. Right depth per picture.** 32-bit only where the eye sees it
  (Hourly's line); 16-bit for the rest; rain frames as 8-bit palette (7.1
  to 1.8 MB); the radar map 16-bit. Each checked on the panel first.
- **P6. Budgets the build enforces.** A RAM budget per app (Weather: 8 MB
  of pool plus its graphics quota) and a CI rule: no function frame over
  4 KB except main (from -Zemit-stack-sizes); big objects made in place
  (Slot::put_in). It would have caught both overflows before a device run.
- **P7. Cache what the network gives, by key.** Radar tiles and frames on
  the card by zoom/x/y/time; a forecast with its time; a refresh fetches
  only what changed.

## Targets (measured on the X51v; the owner proves)

| | Now | Target |
|---|---|---|
| Weather's RAM | ~45 MB with packs and radar | under 8 MB |
| A pack pick | 3.5 s the first time | under 0.1 s |
| Start to Weather shown | several seconds | under 0.5 s |
| Frames during touch | 6-14 fps in places (40 ms CPU frames) | 30+ fps |
| Stack | overflowed | deepest under 75%, checked in CI |

## What would overturn it

- Decoding a picture on demand costing more than about 10 ms on the X51v:
  then P2 keeps decoded pictures on the card (with P4's file).
- 16-bit pages or 8-bit rain looking worse on the real panel: they stay at
  today's depth.
- Card seeks slower than measured: the on-screen pictures are read as one
  block per refresh.

## Order once the owner says go

1. P6 (the stack rule in CI, big objects in place): runs safe again.
2. P2 + P3 (icons by offset, small cache, parts freed after composing).
3. P4 (snapshot start).
4. P5 (depths, each checked on the panel).
5. P7 (tile and frame cache).
6. A device run with the targets table filled in from its log.
