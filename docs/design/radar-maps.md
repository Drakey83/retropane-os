# The radar's maps: the design session (R179, 2026-11-04)

The owner, 2026-11-01: "I want to make sure that we have a design session
over just the maps for the radar". This page is that session's material:
what the radar's maps are now, what is wrong with them, and the choices,
each with its pros and cons, for the owner's votes (M1-M6).

**Decided 2026-11-04: the owner adopted every proposal ("Adopt
proposals"):** M1 Paper as the light map, Ink redone kept as a choice;
M2 Night with its roads dimmed; M3 the X51v draws its own map from map
data on the card, reached through maps drawn on the PC; M4 zooms 6-9,
opening at 8, the rain enlarged past 7, NOAA's radar for the US once its
terms are checked; M5 cities by size per zoom, the place's pin, faint
interstates and borders, attribution; M6 the past 2 hours only, rain a
little stronger on the light map, R177's loop; our own nowcast designed
later. Build order at the end of this page.

Every mock-up below is drawn by `cargo xtask ui-preview` through the
X51v's own shell and 2D-engine code (`RP_PREVIEW_MAPS`, the map at the
screen's 5-6-5 depth), so each is a screen the device can show as drawn.
They use the built-in Seattle map and the sample rain; the app's chrome is
the light look in every shot (the previews' setting), the map is what is
being judged.

## Where the maps are now

- **Two map paths.** The built-in map (Seattle only, drawn on the PC from
  Natural Earth and open elevation data by `tools/mockups/radar_basemaps.py`)
  and, for any other place, OpenStreetMap's standard tiles fetched through
  the dock's relay and darkened on the device (`rp_weather::tiles::dark`).
  The owner's own place gets the second.
- **Ink is not a map of its own.** The device makes it from Night
  (`rp_today::map_row`): every pixel where blue leads becomes black, the
  rest is flipped and pushed toward white. That throws away the terrain,
  swells every inlet and river into a black shape, breaks thin roads into
  dots, and leaves light rain (see-through green) weak on white. The owner,
  2026-11-04: "do you see how much detail the ink map loses?" and "if we
  could fix that id probable be much happier with it" (R169).
- **Rain** is RainViewer's frames, colour scheme 2, zoom 7.

### Findings that limit the choices (checked 2026-11-04)

- **RainViewer** (its API page): tiles to **zoom 7 at most**, and **past
  frames only** (2 hours, every 10 minutes): no forecast frames. Our
  timeline's "+1 h" can't be filled from it.
- **OpenStreetMap's tile servers** (the OSMF tile usage policy): viewing
  only what is on screen is allowed, with a named User-Agent, caching by
  the server's headers and visible attribution; **offline use, prefetching
  and tile archives are forbidden**, access can be cut without notice, and
  a product shipped to many people is told to host its own tiles or use a
  provider that allows it. Fine for our development runs, not for the
  shipped OS.
- **CARTO's dark tiles** went behind a key (2026-10-31).
- **The map never changes.** Unlike the rain, it can be made once, ahead,
  in our own colours, at the screen's own pixels, with our own font.

## M1. The light map (light mode)

`mockups/radar-maps-light.jpg`: today's Ink beside four maps drawn as maps
of their own, each keeping the hillshade, a mid-tone for water and land
light but not white.

| | Look | For | Against |
|---|---|---|---|
| A | **Ink, redone**: off-white land with its terrain, charcoal water, black lines | keeps the newspaper look the owner chose, with the detail back; the sharpest coast | the darkest water of the four: a big lake or sea is a large dark area in the light look |
| B | **Paper**: warm light land, steel-blue water, warm grey roads | water and land read at a glance; warm, goes with the sunset wallpapers; every rain colour stands out on it | the most "map-like", less of a statement than Ink |
| C | **Slate**: cool neutral greys, water a shade darker | the rain carries all the colour on screen (as Apple's Weather does); calm | land and water close in tone; the least character |
| D | **Relief by day**: soft colours by height under the shade | the most terrain: valleys, mountains and snow read | the colours compete a little with light rain's green |

**Proposed: B (Paper),** with A kept as a choice in Menu > Radar map for
those who want the newspaper look. Paper puts the most distance between
the rain's colours and the map in light mode, and it keeps every detail
Night has.

## M2. The dark map (dark mode)

`mockups/radar-maps-dark.jpg`.

| | Look | For | Against |
|---|---|---|---|
| E | **Night** (today): dark slate land with terrain, near-black water, white roads | the owner already likes it; sits under the smoked glass; rain glows | white roads are the brightest thing on the map after the rain |
| F | **Relief, dark**: land tinted by height, deep-blue water | the most to look at | busy under rain; greens near the rain's greens |

**Proposed: E (Night), its roads dimmed** from white to a light grey so
the rain stays the brightest thing on screen.

## M3. Where the map comes from

| | Way | For | Against |
|---|---|---|---|
| 1 | **Fetch OpenStreetMap's tiles** as now | works today, anywhere | forbidden for offline use or prefetching; must show attribution; can be cut off; looks are made by recolouring someone else's map (Ink's problem) |
| 2 | **Our own map, drawn on the PC per place**, kept on the card (Seattle's map is this) | exactly our look, our font, the screen's pixels; no fetching, works offline | a PC (or a map pack) is needed for each new place and zoom |
| 3 | **Map data on the card, drawn by the X51v**: Natural Earth (public domain: coast, lakes, rivers, roads, borders, places) and coarse elevation for the shade | any place on Earth, any zoom, any look, switched instantly; labels in our font; no network for the map, ever (international rule) | the most to build: a compact map format, its parser (checked as hostile input), and drawing time (to measure: drawn once per view and kept) |

**Proposed: 3 as the target, built through 2.** The PC drawer
(`radar_basemaps.py`) becomes the reference the device's drawing is tested
against, pixel for pixel, as the 2D engine's model is; maps from it ship
first, and the device draws its own once the format is measured. Natural
Earth's detail fits the radar's zooms (5-9); finer data (OpenStreetMap
extracts, ODbL, attribution required) only if a closer zoom is wanted.

## M4. Zoom and place

- RainViewer's rain stops at zoom 7: one screen is ~500 km across at the
  owner's latitude. Seattle's built-in map is zoom 9 (~100 km).
- **Proposed:** zooms 6 to 9 with + and &minus; (the panel is single-touch:
  no pinch), **opening at 8** (~250 km: your area and the storms coming
  in), centred on the place with its pin. Beyond zoom 7 the rain is
  enlarged smoothly by our fitted 2x upscaler (bigger, not sharper: said
  plainly on screen as "Rain: 1 km grid"); in the US, NOAA's radar (public
  domain, served as tiles by Iowa State's Mesonet, to zoom 10+) for sharp
  rain at 8 and 9, its terms checked first (R159).

## M5. Labels and lines

- **Proposed:** cities by size per zoom (the biggest few at 6, more as
  you zoom in), the place's own pin, interstates and state or country
  lines faint; labels in the shell's font with a halo, drawn last, never
  under the glass panels; attribution of the rain's source in the corner
  (required by both RainViewer and NOAA's terms of use, to confirm).

## M6. The rain over the map

- Today's rain opacity (175 of 255) and colours are one set for both
  maps. **Proposed:** opacity per look (a little stronger on the light
  map, where see-through green is weakest), colours unchanged.
- **The "+1 h" half of the timeline:** RainViewer has no forecast
  frames. Options: (a) show the past 2 hours only, the timeline ending at
  now; (b) **our own nowcast**: the last frames' motion measured on the
  device and the rain moved on by it, frame by frame (what Dark Sky's
  "rain in 12 minutes" did), labelled "estimate". **Proposed: (a) now, (b)
  as its own design** after the maps.
- **The loop (R177):** the newest frame first and shown still; the rest
  in the background; the loop only once all are in; frames kept between
  visits, only the newest fetched again; each frame composed over the map
  once into a picture.

## Weight, and every country (the owner, 2026-11-04: "will this be too heavy and will it work for every body even people in different countries?")

Measured from the Natural Earth files (2026-11-04) where it says
measured; the rest are estimates to measure on the X51v.

| | What | How much |
|---|---|---|
| The world's map data | land, islands, lakes, rivers, roads, state and province lines | 2.03 million points (measured): ~8 MB at 4 bytes a point, less packed; the place names, 7,342 places, well under 1 MB |
| The terrain shading | elevation for the whole world's land at ~2 km | ~10-20 MB packed (estimate); optional: without it the land is flat, every other detail stays |
| Drawing a view | the map for one screen, once, then kept | well under a second (estimate: a few thousand points in view and one pass over the screen's 222,000 pixels) |
| Memory | one drawn view; the loop's 12 frames composed | 444 KB; 5.3 MB (2.7 MB at R154's 8-bit palette) of the 64 MB |
| Network | the rain only, never the map | a frame ~10-30 KB (the run of 2026-11-02: tiles 300-8,000 bytes, 9 a frame); a full loop ~100-300 KB |

Lighter than today: the map is never fetched or decoded (today's path
fetches nine map pictures and decodes them every time Radar opens).

**Every country:**

- **The map** works everywhere: Natural Earth covers the whole world at
  the same detail, and it draws its borders by a chosen country's view
  where borders are disputed (its "point of view" files): the region
  setting picks it.
- **Place names** in the person's language: Natural Earth's full places
  file carries names in many languages; scripts beyond Latin, Greek and
  Cyrillic need fonts the OS doesn't have yet (the international rule's
  scripts item, not the radar's alone).
- **Units**: the scale bar in miles or km by the region setting.
- **The rain** is where the honest limit is: RainViewer gathers ground
  radar from many countries (North America, most of Europe, Japan,
  Korea, Australia and others), but large parts of Africa, Asia, South
  America and the oceans have no weather radar at all. There the radar
  says "No radar coverage here" (RainViewer's coverage map tells us
  where), never an empty map that looks like a dry day. A worldwide
  fallback to weigh later: satellite rain estimates (NASA's IMERG, public
  domain, global, ~10 km, every 30 minutes), coarse but everywhere.
- NOAA's sharper radar (M4) is a US extra on top; nothing depends on it.

## Build order (decided 2026-11-04)

Each step proven on the X51v before the next (proven-on-hardware rule);
each through ui-preview first.

1. **The looks as maps of their own** (M1, M2): the PC draws Paper, Ink
   (redone) and Night (roads dimmed) for the built-in place; the maps go
   on the card as files (the image has no room for three: rp-today loads
   ~686 KB of its ~886 KB), read like the icon packs; the device's Ink
   conversion (`map_row`'s ink branch) removed. The setting's looks:
   Paper, Ink, Night, and Auto (Paper by day, Night by night, the
   default).
2. **The timeline and the rain** (M6, R177): past 2 hours only, ending at
   now; the loop's newest-first fetch, kept frames, each frame composed
   once over the map; opacity per look.
3. **Any place, drawn on the PC** (M3's first half): the dock's relay draws
   the map for the place shown (the same drawer as step 1, Natural Earth
   and open elevation tiles) at each zoom, instead of OpenStreetMap's
   tiles; the device keeps them on the card.
4. **Zoom** (M4): + and &minus; for zooms 6-9, opening at 8, the rain past
   zoom 7 through the fitted 2x upscaler; the place's pin; "No radar
   coverage here" from RainViewer's coverage map.
5. **The map drawn by the X51v** (M3's target): a sheet first (the compact
   map format, its parser as hostile input, drawing time measured),
   tested pixel for pixel against the PC drawer; then labels by zoom
   (M5) in the shell's font.
6. **NOAA's US radar** (M4): its terms (R159), then sharper rain at 8-9 in
   the US.
