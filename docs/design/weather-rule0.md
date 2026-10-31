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

## What the big systems do, and what we take

Moved to [principles.md](principles.md) (the owner, 2026-10-31: "it
should be our design philosophy for the entire UI/UX"): the method, the
table of what Windows, Android, Apple and lean Linux do and how ours goes
further, and the practices below as rules for every screen. Weather is
their first app.

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

## Option C, as decided (the owner, 2026-10-31)

P2a as first written (the default set inside the image) doesn't fit: VClouds'
250 px pictures pack to 1.41 MB and its 100 px ones to 312 KB, against about
240 KB free in the 928 KiB image area (pack 0's table, measured). Four
options were put to the owner; A (only the 66 px set in the image, the big
pictures enlarged from it) was drawn on the device's own path
(docs/design/mockups/default-icons-a-compare.png: Lanczos the closest of
three filters, RMS 8.7 against the stored pictures) and turned down: "too
soft you lose all of the detail". He chose C, without a loader change:

- **Stored once, made smaller, never enlarged.** Each pack keeps only its
  250 px pictures (VClouds 1.93 -> ~1.41 MB on the card); every other size
  (Hourly's 160 px, the cards' 66, the rows' 40) is made smaller from it
  while decoding, with a downscale filter fitted offline to these pictures
  (the packs' own 100/66/40 versions, made from the masters, are the fit's
  targets). The image carries no icons (its 122 KB of samples go).
- **The default set, with or without a card.** VClouds' file comes from
  the SD card, else the CF card, else the PC through the dock relay (the
  link the forecasts use; at the SD stage "no card" means a test image
  loaded over USB, so the PC is there), and stays packed in RAM (~1.4 MB):
  a card pulled mid-run never leaves a gap, no error box. Installed on
  flash, the same file is read from the flash file system.
- **Read only what is on screen.** The shell lists the pictures it shows
  (a bit per condition per size: the Now and Today big pictures, the
  forecast cards, Hourly's rows and card, 10 Day's rows); at a refresh,
  a pack pick or a place change the image decodes just those, from the
  default file in RAM or the chosen pack on the card by offset (one read
  per picture, `rp_fat::Volume::read_range`).
- **A store of pictures, handles to the shell.** Decoded pictures live in
  a fixed RAM store (about 1 MB of slots; ADR-0022 rule 2: the CPU, which
  composes the pages, never reads 2700G memory). The shell holds small
  handles (slot, generation, size), never pointers, so a slot can be
  reused safely without `unsafe` (ADR-0021 rule 1: the shell knows
  nothing of pixels); the 2D renderer keys its copies in 2700G memory by
  slot and generation, so a reused slot is uploaded again, never shown
  stale (it keys by address today).
- **Built so far (2026-10-31):** the shell holds `Option<Pic>` handles per
  size and lists what its screens show (`Shell::wanted`: the hero's, the
  five cards', every hour's card and row picture, 10 Day's rows);
  `rp_today::Keeper` makes the store hold exactly that, a picture a step,
  a pick replacing each in turn (old shown until its new one is in). The
  store's classes are 250 x2, 160 x6, 66 x8, 40 x16 (~1.15 MB): 160 x6,
  not the x2 first planned, because every hour's card picture is listed so
  a pick never waits on a decode (P1); a day of mixed skies shows up to
  about six.
- **The filter, fitted (2026-10-31):** `tools/shell-assets/shrink_fit.py`
  fits a kernel per size by least squares to every pack's masters (the
  device's own arithmetic: premultiplied, separable, clamped as it
  clamps). RMS over all seven packs (premultiplied 8-bit): 160 px fitted
  1.17 (Lanczos-3 0.74-1.70 a pack, bilinear 2.5-5.0, box 2.6-8.8); 66 px
  fitted 0.63 (Lanczos-3 0.40-0.85, slightly worse in every pack); 40 px
  fitted 0.80, a tie with Lanczos-3. Finding: the packs' small pictures
  were themselves made by Lanczos-3 from the masters, so Lanczos-3 is
  nearly the optimum here and the fit's gain is small; box and bilinear
  are 4-8x worse. The fit's first run fitted the unclamped output (its
  ringing) and came out worse than its start: fixed by modelling the
  clamp. The Rust test holds the device's integer arithmetic to the
  fit's RMS on two VClouds conditions. VClouds' pack: 1,932,247 -> 1,408,989
  bytes; the image 701,828 -> 584,672 bytes loaded.
- **Built in full (2026-10-31):** steps 1-6 (1282027a, 7044287b,
  b9b8a6c6, c1a6ef36): handles and the wanted list; decode at size with
  the fitted filter; packs at 250 px only (VClouds 1.41 MB); the default
  pack from the SD card, else the CF card the loader attached, else the PC
  through the dock relay, kept packed in RAM; other packs a picture at a
  time by offset; no pictures in the image (701,828 -> 589,492 bytes
  loaded); Hourly's graph one store picture on both paths (MAX_CMDS 256 ->
  208). P7's radar half (b23caa98): a refresh keeps frames by their time,
  9 tiles asked, not 81. The X51v run: docs/design/option-c-device-run.md.
  Open: P4 (R153, after the run's start measurement), P5 (R154, the
  owner's eye), P7's forecast half (R155).
- **Hourly's card picture at 160 px** (the owner's pick of three drawn
  sizes, docs/design/mockups/hourly-card-picture-sizes.png), the text
  moved right; built 2026-10-31, the picture shown at 160 once the store
  decodes it at that size.
- **What it costs, against R146:** the image -122 KB; RAM ~37 MB -> ~2.4
  MB (the default file and the store); each pack on the card ~27% smaller;
  graphics memory a fixed ~1 MB of copies.
- **Measured before it is called done:** a decode at size under 10 ms a
  picture on the X51v (else the decoded pictures are kept on the card,
  P4's file), a pack pick under 0.1 s, the log's RAM and timing lines.

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
