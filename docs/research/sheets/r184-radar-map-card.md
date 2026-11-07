# Sheet: R184 step 3, each place's radar map kept on the card (rule 0, before code)

Step 3 of the radar's build order (docs/design/radar-maps.md): "the dock's
relay draws the map for the place shown ... the device keeps them on the
card". The relay half was built in step 1 (`RPNET LAYERS`,
`tools/maps/radar_layers.py`, the PC's own cache `local/maps/cache`); this
sheet is the device half. Written before any code; the owner's go first.

## What it costs now (run 4, `local/rp-today-run-2026-11-06-r184s2d.txt`)

| | When | Took |
|---|---|---|
| Radar opened | 38.5 s | |
| `LAYERS 30.645 -97.838 7` asked | 38,991 ms | |
| its answer, 180,918 bytes over the dock (~140 KB/s) | 40,280 ms | **1,289 ms** |
| unpacked (zlib, 1.77 MB of layers) | after it | 167 ms |
| coloured Night, then into the wallpaper | | 59 + 27 ms |
| the first rain asked (`RAIN 12`): it waits behind the map | 40,571 ms | 1.58 s after the map's ask |
| the newest frame on screen | | 3.3 s after opening |

- **Once per start of the app, not per opening:** the second opening in
  that run (line 650) coloured the same layers in Paper with no ask. The
  layers stay in RAM (their 1.77 MB buffer is reserved for the whole run).
- **The rain waits for the map:** the shell asks the map before any rain
  frame (`Ask::Tile if !self.radar_map`, `shell/core/src/weather_net.rs`),
  so the 1.3 s is added to the rain's wait too.
- **No dock, no map:** the radar says "RADAR NEEDS THE DOCK" even for a
  place it showed yesterday.
- A place never drawn before: the PC draws it first, 3.46 s (today-0053).

## The card's own speeds (measured, docs/hw/sd.md and the image's logs)

- Reads through the image's piece reader (32 KB a pass, ~30 ms): the icon
  pack's 1,408,989 bytes in 2.28 s (run 4), ~0.6 MB/s: **the map's
  181 KB in ~0.3 s** (estimate from that rate).
- Writes through rp-fat: 142,848 B in 85 ms (1.7 MB/s); 768 KB new files
  349-460 ms through the loader: **the map's 181 KB in ~0.1-0.15 s**
  (estimate), once per new place, ever.
- The image writes nothing to the card today except its log's sectors
  (`cardwrite::sd_write`, C59: a sector's programming waited 17-141 ms).
  This step is the image's first file write: rp-fat's `put` is
  copy-then-swap (the old file untouched until one sector links the new),
  already proven by the loader and ADR-0013/0036.

## What others do (findings, not the answer)

- **Windows Mobile 5 / Dell:** no radar, no maps; nothing to borrow.
- **Phone map engines** (MapLibre/Mapbox "ambient cache"): every tile the
  map has fetched kept on disk in a database keyed by the tile's address,
  a size cap with the least recently used dropped, each tile shown from
  the cache at once and checked against the server in the background
  (its ETag: "unchanged" costs a few bytes).
- **Browsers:** the HTTP cache and "stale-while-revalidate" (RFC 5861):
  show what's kept, ask for a newer copy behind it.
- **Apple Weather, Google:** the same tile caches, nothing kept per place;
  a tile still decoded every time it shows.
- **Ours until now:** nothing kept on the device; the PC's relay keeps
  each drawn map (`local/maps/cache`), so only the dock's transfer is paid
  again, every start.

## The ways for the X51v

| | Way | For | Against |
|---|---|---|---|
| A | **The relay's file kept on the card** per place and zoom, read **when Radar opens** (in 32 KB pieces a pass, beside the rain's asks: the card and the dock are separate wires) | map on screen ~0.5 s after opening, not ~1.5 s (estimate: 0.3 read + 0.17 unpack); the rain asked at once, so the newest frame ~1.6-2.0 s instead of 3.3 (the dock's own time: FRAMES 0.5 + RAIN 1.1, run 4); works with **no dock** for any place seen before; keeps the file packed (ADR-0005 rule 4) and loads it only when wanted (rule 10) | ~0.5 s of "Radar loading" still shows before the map |
| B | **A, read at the app's start** (after the settings and the icon pack, in the background) | the map already unpacked when Radar opens: 0 s of map wait | card reads and a 167 ms unpack at every start for a page the person may not open; bends ADR-0005 rule 10 ("load nothing until it's needed") |
| C | Keep the **coloured picture** (RGB565, 443 KB per look) | no unpack, no colouring | 2.5x the bytes to read (~0.75 s), a file per look (Paper and Night), loses step 1's "one file every look" |
| D | Keep the **unpacked layers** (1.77 MB) | no unpack | ~3 s to read: slower than the dock. Rejected |
| E | The PC puts maps on the card at `cargo xtask load` (as the settings, `RPSET`) | no device writes | needs the PC and knows only places named in advance; the shipped OS has no xtask. Rejected as the way; nothing stops it later as a "map pack" |

**Proposed: A, with three refinements that beat the phones' cache on
this hardware:**

1. **One file is a whole view, packed and layered.** A phone keeps ~9-16
   tiles a view and decodes each every time it shows; we keep one 181 KB
   file per place and zoom that is already the view at the panel's
   pixels, every look in it (step 1's layers).
2. **Read on intent, unpack spread out.** The card read starts the
   moment Radar is asked for, in the same pieces as the icon pack (no
   pass over ~30 ms), and the unpack is split a layer a pass (8 layers,
   ~21 ms each, estimate) so the opening's movement stays smooth; today
   the 167 ms unpack is one slow frame.
3. **Shown at once, checked behind it.** Once the rain's frames are in
   and the dock is idle, the device asks `LAYERS lat lon z <crc32>` with
   the checksum of the file it has; the relay answers **empty** when its
   map is the same (a few bytes instead of 181 KB), else the new file,
   which replaces the card's. A changed drawer (a style fix, finer data
   for step 4's zooms) reaches every card without wiping anything.

B stays the choice if the owner wants the map with no wait at all; the
measured opening time in the device run decides between A and B.

## How it would be built (if A)

- **The card file:** `RetroPane/Maps/` (the OS's own folder: the shared
  card rule), one file a place and zoom, an 8.3 name from a hash of the
  key (`1A2B3C4D.RPM`: rp-fat writes short names), holding a small header
  of ours, `RPMC` (version, the key: latitude and longitude in
  thousandths and the zoom, the payload's length and CRC32, a write
  counter), then the relay's RPML bytes as they came. The header is
  checked first (hostile input, docs/security.md): wrong magic, key, length
  or CRC = not used, asked of the dock, written again; then RPML's own
  checks as now. A file torn by a reset mid-write fails its CRC (and the
  copy-then-swap leaves the old one whole).
- **Writing:** after the map is shown, at a quiet moment (no pen down, the
  loop not playing or between steps), one `put` (~0.1-0.15 s, estimate;
  measured and logged). If that shows as a hitch, a piece writer (a
  reserved file written 32 KB a pass, its header last) replaces it.
- **The cap:** 32 maps (~6 MB, under 0.2% of a 4 GB card); the place
  shown and the saved places' maps are never dropped; others by the
  oldest write counter.
- **The shell:** a map from the card counts as the place's map
  (`radar_map`), so the rain's asks go first; the map's ask becomes the
  check with the checksum, last in the order. With no dock: the map
  from the card and "No radar" on the timeline (no rain without the dock)
  instead of "RADAR NEEDS THE DOCK" over the wallpaper.
- **The relay:** the `LAYERS` check answer (empty when the CRC matches);
  the old `TILE` branch removed (OpenStreetMap's tiles and single rain
  tiles: no longer asked since steps 1-2, and OSM's policy forbids them in
  a shipped product: M3).
- **Card absent, full or failing:** the dock as now, every failure a log
  line with its reason (no silent errors); never a broken map shown.
- **Tests:** host tests of the header (each field wrong, torn, too long,
  another place's key), the cap's choice, the check's empty answer;
  emu-today with a card, twice (the first writes, the second reads; the
  emulator's SD model) and without one; ui-preview unchanged (no new
  screen, only the "No radar" wording with no dock).
- **The X51v run:** two starts of rp-today on the owner's place: the first
  writes the map (its write time in the log), the second opens Radar with
  the map from the card. Measured: the map on screen after opening, the
  newest frame after opening, any slow frame at the write and the unpack.

**ADRs checked:** ADR-0005 (rule 4 packed storage: kept packed; rule 10
load only when needed: A reads on opening, B would bend it), ADR-0013 and
ADR-0036 (rp-fat's copy-then-swap writes, FAT32 and exFAT alike; the
image touches only its own folder). The image's FAT use goes through
`rp_fat::Volume` as its reads do; no change to `kernel/fat` expected.

## What would overturn it

- The map on screen measured over ~0.8 s after opening with A: then B
  (read at start).
- The write measured as a visible hitch (over ~50 ms in one pass): then
  the piece writer.
- The second start's map not found because the place's key moved (the
  relay's place and a saved place rounding differently): then key on the
  saved place's own number, not its degrees.

## Not in this step (each its own step or the owner's call)

- Maps fetched ahead for the other saved places while docked and idle
  (switching place instant too): easy on top of this; for the owner's
  vote.
- The rain asked before Radar opens: changes the dock's load for a page
  not open; a design of its own if wanted.
- Zooms 6, 8 and 9: step 4 (the files are keyed by zoom already).

## As built (2026-11-06, the owner's "Go, way A")

- **The card file:** `RPMAPS/<CRC-32 of the key>.RPM` at the card's root
  (8.3, beside the image's own `RPIC` packs; the sheet's `RetroPane/Maps`
  became one short folder name because rp-fat writes 8.3 names), holding
  `RPMC` (32 bytes: magic, version, zoom, latitude and longitude in
  thousandths, the map's length and CRC-32, when it was written in Unix
  seconds from the newest rain frame's time, the header's own CRC-32) and
  the relay's bytes (`rp_weather::mapcard`). Checked on reading: header,
  key, file length, then the whole map's CRC-32 before it is used.
- **The format went to version 2** (`rp_weather::maplayers`, the drawer
  `tools/maps/radar_layers.py`, rp-today's stand-in): one zlib stream a
  layer with each stream's length in the header, so the card's map is
  unpacked a layer a pass. The Liberty Hill map: 176,574 bytes (180,918 as
  one stream). The relay caches version 2 apart (`..._v2.rpml`).
- **The image** (`shell/today`: `kept.rs` on the host, `kept_map_pass` in
  the image): in the pass the shell asks (`Shell::radar_map_wanted`, before
  the frame list can answer) it reads the header and says so
  (`radar_map_card`); the map is read 32 KB a pass and unpacked a layer a
  pass (`radar_map_in` at the end); the same place again in a run uses the
  layers still in RAM. A dock map is unpacked as before (one pass, only the
  first time for a place) and written to the card in the pass after it was
  shown, pen up, in one `put` (32 KB FAT-search scratch); past 32 maps the
  oldest of a place neither shown nor saved is removed (a damaged one
  first). Every step is a log line after the card is let go.
- **The shell** (`weather_net`): the rain is asked at once when the card
  has the map; the check `LAYERS lat lon z <crc32>` goes last, once the
  rain is in or lost; a refused check keeps the kept map. With no dock the
  map from the card shows, the chip says "RADAR NEEDS THE DOCK" over it and
  the timeline "No radar"; the map stays for the same place across refused
  frame lists.
- **The relay** answers the check with nothing when its map's CRC-32 is the
  same (the C# `RpRain.Crc32` equals zlib's: 8afb84fd on the built-in), the
  new file otherwise. Its old `TILE` branch (OpenStreetMap's tiles and
  single rain tiles, unasked since steps 1-2) is gone, and `cargo xtask
  load` no longer puts the built-in `RPMAP` on the card (unread since step
  2).
- **Checks:** host tests of the header (each byte), the per-layer unpack,
  the shell's order (rain first, check last, unreadable kept map to the
  dock, no-dock map), and `kept` on a FAT32 card image (write, rewrite,
  read back in pieces, a foreign, short, damaged or headerless file
  refused, the cap keeping saved places, the shared card's other file
  untouched); `cargo xtask emu-today-radar-card` (part of `emu-today`): the
  image with a kept map for its default place and no relay shows it, reads
  it in 8 unpack passes, and leaves the file as it was.
- Not measurable off the device: the read, unpack and write times, and the
  opening's timings with the dock; the X51v run measures them.

## X51v run (2026-11-06, `local/rp-today-run-2026-11-06-r184s3a.txt` and `-r184s3b.txt`)

**Start 1 (no map on the card: the dock's, then written):**
- The card looked at in 74 ms ("not on the card"); the map over the dock
  1,395 ms (176,574 bytes), unpacked in one pass 169 ms, as before.
- **The write took 532 ms in one pass** (176,606 bytes to `RPMAPS/`),
  over the sheet's ~50 ms hitch line: the overturn the sheet names (a
  piece writer). The newest frame's answer came 1,587 ms after its ask
  (~1.1 s usual) because the write held the loop in between.

**Start 2 (the map from the card):**
- The header in 59 ms (mount and folder walk in one pass); the map read in
  445 ms over 6 passes (~74 ms a pass: each piece mounts the volume and
  walks the folder again), checked, then **unpacked in 168 ms over 8
  passes** (~21 ms each, as estimated); **the map in 655 ms after Radar
  asked** (~740 ms on screen with colouring and the wallpaper write),
  under the 0.8 s line: way A holds, B not needed. Run 4: ~1.55 s.
- **The newest frame answered 1.8 s after opening** (the frame list
  0.77 s, then `RAIN 12` 1.03 s), against 3.3 s in run 4.
- **The check:** `LAYERS ... 4c08daa0` answered empty in 59 ms ("the card's
  is the relay's"), once, after the rain.

**Proposed next (for the owner's go):**
1. The piece writer: the file reserved in one run of clusters and written
   32 KB a pass, its header last (a torn file fails its CRC), so no pass
   holds the loop for ~0.5 s.
2. The read through the file's extent (`rp_fat::Volume::extent`: one
   lookup, then plain sector reads a pass) instead of a mount and folder
   walk per piece: ~445 ms toward ~110 ms at the card's 1.7 MB/s
   (estimate), the map on screen ~0.4 s after opening.

### As built after the run (the owner's "Build them", 2026-11-06)

- **The write a piece a pass:** rp-fat's new staged write
  (`Volume::stage`, `stage_write`, `commit`; `rp_fat::Staged`): the stage
  makes the folder if missing and finds one free run without writing it;
  the data goes into those still-free clusters 32 KB a pass (the volume
  mounted again each pass); the commit checks the run is still free, links
  the FAT, then switches or creates the entry: copy-then-swap's crash order
  across passes, so a reset part way leaves the old map whole and nothing
  allocated (rp-fat test, `kept` test with a card copied mid-write). The
  stage makes the folder first because a commit into a new folder took the
  very cluster the stage had found (the test caught it). Every pass skips
  while the pen is down; the log gives the passes, the total and the
  longest pass.
- **The read straight off the card:** `kept::head` also returns where the
  file lies when it is one run (`Volume::extent`); the image then reads its
  sectors 32 KB a pass with no mount or folder walk (the file's start, its
  header again, then the map), and through the folder only if the file is
  in pieces. The host test reads both ways and checks they agree.
- Emulator (`emu-today-radar-card`): "one run", read 12.6 ms, unpacked in
  8 passes. The X51v run measures both.

## Second X51v run (2026-11-06, `local/rp-today-run-2026-11-06-r184s3c.txt` and `-r184s3d.txt`)

The kept map removed first (`cargo xtask rm RPMAPS/56A5C299.RPM`).

- **The write a piece a pass (start 1):** 8 passes, 317 ms in all (532 ms
  in one pass before): the stage pass 134 ms (finding the free run), six
  pieces ~20 ms each, the commit 60 ms. The longest pass is still over the
  50 ms line: the free-run search.
- **The read (start 2):** the header pass 124 ms (mount, folder, header
  and the extent's walk of the chain), the sectors 236 ms over 6 passes
  (445 ms through the folder before), unpacked 168 ms in 8 passes: **the
  map in 476 ms after Radar asked** (655 ms in run 1, ~1.55 s in run 4).
- **The newest frame 1.58 s after opening** (1.8 s run 1, 3.3 s run 4); the
  check empty in 12 ms.
- The owner: start 2 (run 1) "took a second to load but it wasnt too bad".
- **The owner found a look bug** (start 1, Weather settings Light to Dark):
  "the light mode popped up for a second before switching to dark mode,
  this looks broken". Proven in the log: the switch at ~64.4 s; back on
  Weather at 65,754 ms the Now page's picture made in the light look was
  handed over as built, and redrawn dark only at 65,802 ms (51.7 ms drawn,
  61.8 ms written); a look change asked no page to be rebuilt ahead
  (`pages_check` only on changes while a page is shown). Fixed as a class
  (not a radar fault): each page's and the chrome picture's look recorded;
  a look change rebuilds them ahead while still; a page of the other look
  is made again before it is handed over; the chrome picture of the other
  look is never used for an opening. Device proof owed (R185).
- Left over the 50 ms line: the stage's free-run search (134 ms) and the
  header pass (124 ms). Ideas for the owner: keep the FAT's free-run hint
  from the last write (the search starts where the last file ended), and
  remember the kept file's run from the write so the header pass doesn't
  walk the chain.
