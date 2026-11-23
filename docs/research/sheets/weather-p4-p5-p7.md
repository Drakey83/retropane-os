# Sheet: Weather rule 0, the rest: P4 snapshot start, P5 right depth, P7 the forecast (R153-R155)

The last three practices of the Weather rule 0 redo
([weather-rule0.md](../../design/weather-rule0.md)), written before any code
(CLAUDE.md rule 0; the owner's go first). Option C's device run happened:
run today-0048 (2026-11-01) and about 40 Today runs since, to today-0150
(2026-11-15). Their logs (`local/cardlog/`) are the baseline below; nothing
here is a guess unless it says "estimate".

## What start costs now (measured)

Run today-0150 (icon pack 2 chosen, the Tempest station on), times from the
image's start:

| Step before Today shows | Took | Needed for Today's first frame? |
|---|---|---|
| wallpaper unpacked, converted, shown | 85 + 164 ms | yes (or a snapshot instead) |
| radar cells, station dials painted | 42 + 66 ms | **no** (Radar, the station app) |
| clock, organiser and settings files read from the card | 267 ms (organiser 92) | the settings yes; ~175 ms for two files under 1 KB |
| Hourly's graph painted | 25 ms | **no** (Weather) |
| the three Weather pages drawn and written | 93 + 186 + 89 = 368 ms (pages 0-2, drawn + written) | **no** (Weather, opened later) |
| icon pack 2's table read | 160 ms | for the pictures |
| first frame | 50 ms | yes |
| unlogged (display adopt, card start, store, shell made) | ~210 ms | partly |
| **Today shown** | **1,436 ms** (all runs: 1,050-1,573 ms) | target 0.5 s |

After it:

- **Today's pictures:** 20 made in 1,354 ms more (inflate 90 ms, shrink 32
  ms the slowest): the pictures are complete about **2.8 s** in. With the
  default pack (VClouds) its 1,408,989-byte file is read first: 1.5-2.9 s
  in every run since 2026-11-01, so **~4-5.5 s**. Even with pack 2 chosen,
  the default file is read too (the fallback): 2.1 s more of card time.
- **The weather itself:** until the relay answers, Today shows a stand-in
  forecast. In today-0150 the hero said "Partly cloudy" for **18.7 s**,
  then the Tempest answer (95,744 bytes) turned it to "Heavy rain" and 5
  pictures were remade (0.82 s). With no dock it never changes. Nothing of
  the last forecast is kept between starts.
- **Option C's targets, from run today-0048** (option-c-device-run.md):
  a decode at size, slowest 82.5 ms (target 10 ms: **missed**; the sheet's
  own fallback was "decoded pictures kept on the card, with P4's file");
  a pack pick: table 160 ms + pictures 1.35 s (target 0.1 s: **missed**);
  start 1,074 ms (target 0.5 s: **missed**); Hourly's graph one store
  picture (24.8 ms, **met**); the RAM line 3.4 MB of pictures (5.4 MB in
  today-0150 since the store grew to 113 slots for the apps); the radar's
  ten-minute keep never logged (no run stayed ten minutes on Radar).

### Did we do it wrong? The card is read 10x below its speed

The X51v's card reads at **8.2 MB/s by DMA** (`probe.sd_read_256_blocks_dma`,
docs/perf.md: 256 blocks in 15.95 ms). The Today image reads its files at
**0.5-0.9 MB/s** (the default pack: 1.41 MB in 1.5-2.9 s). The cause, read
in the code:

- `rp_fat::Volume::read_range` reads at most its scratch per card command,
  and every caller in `shell/today/src/main.rs` passes `[0u8; 2 * BLOCK]`:
  **two sectors a command**, so a 1.41 MB file is ~1,380 commands, each
  with the card's per-command start-up (docs/hw/sd.md: ~0.27 ms a block of
  overhead even in long runs; 3.85 ms for single commands on the 4 GB card).
- `read_chunk` (the default pack, 32 KB a pass) mounts the volume, finds the
  file and walks its cluster chain from the start **for every 32 KB**: the
  walk grows with the offset.

This is the first fix of P4 (it moves every card read the image makes:
packs, settings, maps, the organiser), and it is the reason the
snapshot's raw form below is affordable at all.

## P4. Snapshot start (R153)

### What others do (findings, not the answer)

- **Windows Mobile 5 / Dell:** the Today screen is drawn live by shell32
  and each Today plug-in at boot, after every plug-in has loaded; weather
  plug-ins (SPB Weather, HTC's) keep their last forecast in the registry or
  a file and draw from it at once (leads from memory, not verified). Dell
  adds nothing to the Today screen's start; no REA search run: this is not
  a hardware question.
- **iOS:** a static launch screen, then state restoration; the app
  switcher's snapshot is shown on a warm return, never on a cold start.
- **Android:** the splash screen API (an icon on the theme's colour), then
  the app draws; Recents keeps snapshots for the switcher only.
- **Modern weather apps:** the last forecast kept on disk, shown with its
  age ("Updated 2 h ago"), refreshed behind it.

None shows the real last screen on a cold start: they show a placeholder,
then real content. Ours can, because the screen is ours to the pixel and
the card reads fast.

### The ways for the X51v

| | Way | Shown at (estimate) | For | Against |
|---|---|---|---|---|
| A | **The screen as left**, RGB565, 614,400 bytes, saved when Today settles after a change; read straight into the display's frame by DMA at start; the live Today replaces it | ~0.15-0.25 s (card start, unmeasured, + 75 ms at 8.2 MB/s) | the very screen the owner left, icons included, before any other work; Game Mode's return wants the same file (game-mode-b.md: "600 KiB at 16 bits ... R153's snapshot start") | the clock and today's appointments are as they were; a moving sky frozen for half a second |
| B | Today's draw list (4 KB, R191 keeps it already) + its pictures, replayed by the 2D engine | ~0.4 s | smaller file | the wallpaper and fonts first anyway; two ways of drawing Today |
| C | **No pixels: the state.** The last forecast (P7, below) and the pictures at the size shown (raw, premultiplied, ~340 KB for Today's 20) kept on the card; the live Today built from them | ~0.5-0.6 s, everything live and correct | no stale pixel ever; R178's wait gone (pictures read in ~45 ms, not made in 1.35 s); option C's own missed-target fallback | still ~0.5 s of wallpaper first |
| D | The phones' way: a placeholder, then the live screen | ~1.1-1.5 s (today) | nothing to keep | what we have; the stand-in forecast for 18.7 s |

**Proposed: A on top of C, with the reads fixed and the start reordered.**

1. **Read at the card's speed.** `read_range` gets a form that reads each
   run of contiguous clusters straight into the caller's buffer, sector
   aligned, in one multi-block DMA command (only a partial first or last
   sector through the scratch); the image keeps each file's cluster runs
   between pieces instead of mounting and walking the chain every 32 KB.
   Estimates from 8.2 MB/s: the default pack 1.5-2.9 s -> **~0.2 s**; a
   radar map 181 KB ~0.3 s -> ~25 ms; the settings and clock files ~175 ms
   -> a few ms.
2. **Only Today before Today** (P1): the three Weather pages, Hourly's
   graph, the dials and radar cells (~500 ms) move after the first frame,
   made in idle slices before anyone can open Weather (the app is composed
   ahead anyway, R188). The default pack's file, when another pack is
   chosen, is read after Today's pictures, not before.
3. **C: the state kept.** `RPFC` (P7's forecast file, below) and `RPSN`
   pictures: Today's pictures at the size shown, raw premultiplied ARGB,
   with each one's pack, condition, size and the pack's file CRC as its
   key. Written when the pictures change (a forecast that changes what is
   shown, a pack pick), in the background. At start, a key that matches
   the chosen pack's file puts the picture straight into the store; any
   mismatch makes it as today. Estimate: Today live with the real forecast
   and every picture at **~0.5-0.6 s** (1,436 - 500 moved - ~230 reads -
   ~155 table, + ~45 reading the pictures).
4. **A: the screen as left.** `RPSCR`: a 64-byte header (magic, version,
   when it was saved, the settings' and look's keys, a CRC-32 of the
   pixels) and the frame. Saved when Today has settled (a second still,
   the same rule as R188's composing ahead) after its pixels changed. At
   start, right after the display is adopted and the card started: header
   checked, then the frame DMA-read straight into the frame buffer. Shown
   only if it is the same day, look and pack (else the live start, never a
   wrong date). The status bar's clock is drawn over it at once (fonts are
   in the image). The live Today replaces it when its first frame is ready,
   in the blanking; with C the two show the same forecast and pictures, so
   the swap shows only what truly changed (the time).
5. **Where the files live:** the SD card at this stage (beside `RPSET`);
   the flash file system once installed (bare metal first: no card needed).
   No card at the SD stage: the live start, as today.

**Why it beats the others:** a cold start shows the real last screen
(nobody does that cold), the live screen behind it is real data rather
than a stand-in, and both cost reads the card does at 8 MB/s rather than
work the CPU does at 1.35 s.

**Costs:** writes of ~0.6 MB (A) + ~0.35 MB (C) + the forecast when they
change (~0.6 s of card time at 1.7 MB/s, in the background; a 15-minute
refresh that changes the screen every time is ~90 MB a day: an SD card's
wear-levelling takes that for decades; on the installed flash it is
checked against the part's rating in the flash design first). RAM: none
new (the frame goes straight into graphics memory; C's pictures go into
the store's own slots).

**Security and privacy:** both files are read as untrusted (docs/security.md):
header, sizes and CRC checked; anything wrong means the live start, no
error box. A tampered snapshot can show wrong pixels for half a second and
nothing else (it is never parsed past its header). The snapshot holds the
Today screen's appointments in the clear, the same exposure as the
organiser's own files on the card; any later "hide on lock" setting covers
the snapshot too.

**What would overturn it:** the card's start-up at boot (unmeasured: the
run's first job) above ~0.3 s, then A's gain over C is small and C alone
is offered; reads straight into a buffer refused by the controller's DMA
for unaligned buffers (then sector-aligned staging in 64 KB runs); the
owner finding a half-second of the old clock or sky wrong to see (then C
only).

## P5. The right depth per picture (R154)

### Where it stands (measured, in the code)

| Picture | Depth now | Settled by |
|---|---|---|
| Now, 10 Day pages | ARGB4444, dithered (0.7 MB each) | P5 as planned |
| Hourly's page | ARGB8888 (2.3 MB, 4.6 MB while rebuilt) | **the owner's eye, 2026-10-31**: at ARGB4444 its line "banded and dithered" (main.rs `wide`) |
| The radar's rain | the relay's palette and runs, ~11 KB a frame in RAM; 16-bit bands in graphics memory, only where it rains | R184 step 2: beat the planned 8-bit palette (1.8 MB) by 160x |
| The radar's map | RGB565, in the wallpaper | ADR-0023 L1 |

So the depth questions P5 opened are answered, one by the owner's eye.
Two pictures by the 2700G's planes were checked as a way to put only
Hourly's line at 32 bits: the page plane takes one format and both planes
(graphics, video) are in use, so it isn't open without a third plane.

What is left is **RAM held at more than the pictures need**, the same
pixels:

| RAM | Now | Proposed | Saves |
|---|---|---|---|
| The CPU's page layer + the plane it flushes into | two full Hourly pages at 32 bits: 2 x 480 x 1,196 x 4 = **4.6 MB** | a band at a time (the pages are already drawn band by band, `PAGE_BAND`): the band drawn, flushed and written to its picture, the next band | **~4.3 MB** (a 64-row band: ~0.25 MB) |
| The radar's map view, the rain view | 32 bits each: 2 x 887 KB | the map coloured straight to RGB565 (it goes to the wallpaper as 565 anyway); the rain blended into it per row | **~0.9 MB** |

**What others do:** Android and iOS draw into tiles (Skia's and Core
Animation's tiles, 256-512 px), never a whole tall page in CPU memory;
Windows Mobile's GDI drew straight into the screen's DIB. Our band is their
tile, cut to the shape of our pages.

**Proposed:** both, as pixel-identical changes: the band drawing held to
the full layer's output by a test on every page (the pictures byte for
byte the same), the 565 map by a PC comparison of every look; any pixel
that differs is drawn through ui-preview and shown to the owner before
it is adopted (R154's eye rule). Weather's RAM line moves toward the 8 MB
target by ~5.2 MB.

**What would overturn it:** a user of the CPU layer that needs the whole
page at once (the look check's read-back, the app composed ahead): each is
listed and checked before the change; one that can't be banded keeps a
full-size layer of its own size only.

## P7. The forecast, cached by what the provider says (R155)

### What the providers say (measured 2026-11-23, Seattle, from the PC)

| Provider | Validators | Lifetime | "Changed since?" |
|---|---|---|---|
| Open-Meteo (3,814 bytes) | none | none in the headers; **in the body**: `current.time` and `interval` 900 s, so the next 15-minute slot is the earliest a different answer can come | no |
| MET Norway | `Last-Modified` | `Expires` (26 min ahead) | **304** to `If-Modified-Since` |
| NWS | `Last-Modified` | `Cache-Control: max-age=3600`, `Expires` | **304** to `If-Modified-Since` |
| Tempest (95,744 bytes) | not measured (needs the owner's key) | the station's readings each minute | the run logs the headers |

**Now:** nothing is kept between starts; a refresh asks on the owner's
timer (`refresh_minutes`) whatever the provider says; the relay passes no
headers.

**What others do:** WM-era plug-ins polled on a fixed timer and kept the
last answer; browsers and phone apps follow HTTP caching (RFC 9111):
`Expires`/`max-age`, then a conditional ask (`If-Modified-Since`, ETag)
answered by 304; Apple's WeatherKit hands each dataset an `expireTime`.

**Proposed (beats each by using what this exact source gives):**

1. **Kept on the card with its time** (`RPFC`): the provider's body as it
   came, its provider and place, when it came, its `Last-Modified`, and
   "good until" (the `Expires` or `max-age`; for Open-Meteo, `current.time
   + interval` read from the body). The body, not our parsed forecast: one
   parser for the network and the card, and a parser fix reaches kept
   files. Parsed at start (the run logs its time; over 20 ms and the
   parsed form is kept instead).
2. **Asked only when it can have changed:** the next ask is at the later
   of "good until" and the owner's refresh setting (his setting stays the
   most often it asks; MET Norway's terms ask clients to respect
   `Expires`: from memory, read before the build). A tap on Refresh always asks.
3. **"Changed since?" over the dock:** the device sends its kept
   `Last-Modified`; the relay asks with `If-Modified-Since`, and on a 304
   answers `SAME` (a few bytes, no parse, nothing redrawn). Open-Meteo,
   with no validators, is compared on the device: an answer whose parsed
   forecast equals the kept one changes nothing (the pages already rebuild
   only when their draw list's key changes).
4. **Shown with its age when old:** a kept forecast past "good until" by
   more than the refresh setting shows "Updated 3:40 PM" on the hero (a
   new small line, drawn through ui-preview for the owner first); with no dock the device still has
   today's real forecast, not a stand-in.

**Security:** the kept body goes through the same checked parser as the
network's; the record's header and CRC are checked first; the relay
passes only those two headers, never a cookie or any other.

**What would overturn it:** a parse at start over 20 ms (keep the parsed
form), or the owner preferring his timer exactly (then point 2 is dropped
and the rest stays).

## One device run for all three (after the build, the owner's "Run it")

Built first, in this order, each through the emulator (`emu-today`) and
CI: 1 the reads, 2 the start reordered, 3 P7's file, 4 P4's C, 5 P4's A,
6 P5's bands and 565 map. Then one run with these log lines:

| Question | Log line | Pass |
|---|---|---|
| Card start at boot | `start: card ready at X ms` (new) | measured (A's gate) |
| The reads at speed | `icons: the default pack read (1408989 bytes ... in X us` | X < 300,000 (was 1.5-2.9 s) |
| The snapshot shown | `start: the last screen shown at X ms (saved H:MM, N bytes in Y us)` (new) | X < 300 |
| The live Today | `start: the Today screen shown at X ms` | X < 600 (was 1,050-1,573) |
| Today's pictures from the card | `icons: N pictures from the card in X us, M made` (new) | M = 0 when nothing changed |
| The forecast at start | `forecast: kept from H:MM (provider, N bytes), parsed in X us, good until H:MM` (new) | X < 20,000 |
| Asked only when it can change | `net: SAME (not changed since H:MM)` (new) and the asks' times | no ask before "good until" except Refresh |
| RAM | `ram: the image's parts of the RAM pool ...` | ~5.2 MB less than today-0150 (17,709,888) |
| Pages unchanged | the pages' log lines; the look check | pictures as before |

The owner's steps (short, plain, before the run): 1 Today appears: "did
the last screen show at once, then the same screen live, the right
time?"; 2 open Weather, Hourly, drag: "the same as before?"; 3 Back to
loader; 4 load again **with `cargo xtask net` off**: "is the real
forecast there, with no dock?". About five minutes; speaker not used.

## Order once the owner says go

As in the run's build order above. P4's A and C and P7 are one change of
behaviour each and can be voted apart: **the votes asked are A+C (or C
only) for P4, the bands and 565 map for P5, and points 1-4 for P7.**
