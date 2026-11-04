# Sheet: R184 step 1, the radar map's looks (rule 0, before code)

The owner adopted M1-M6 (docs/design/radar-maps.md, 2026-11-04) and said
"Build step 1"; then: "are we using rule 0 to make sure this is the best
way to do this?" This sheet is that check, written before any code.

## Two things it found in the first draft

1. **The first draft was three finished pictures** (Paper, Ink, Night),
   each a 480 x 462 map packed (~214 KB each), the shown one unpacked.
   Rule 0 below finds a better way: one layered map for every look.
2. **Step 1 alone wouldn't reach the owner's screen.** Only the built-in
   Seattle map comes from our drawer; the owner's own place shows
   OpenStreetMap's tiles, darkened (Night) or converted (Ink) on the device
   (`rp_weather::tiles::dark`, `rp_today::map_row`). So the new looks would
   show only on Seattle. Proposed: step 1 draws the owner's place too,
   through the dock's relay (the start of step 3), so his run shows it.

## What others do (findings, not the answer)

- **Windows Mobile 5:** no radar, no maps.
- **Phone weather apps** (Apple Weather, Google, MyRadar, RainViewer's own
  app): vector maps styled at run time by the GPU (MapLibre/Mapbox styles),
  a separate style per look; dark mode is its own style, never a recolour
  of the light map.
- **Web radar sites:** raster tiles from a server, one tile set per style.
- **Ours until now:** one map and a recolour on the device for Ink, the
  cheap shortcut, which is what lost the detail.

## The ways for the X51v

| | Way | For | Against |
|---|---|---|---|
| A | **Three finished pictures** (the first draft) | the device copies rows, nothing to compute; what the PC drew is what shows | 3 x ~214 KB per place; every new look (a theme, a colour-blind palette) is another file per place; a look change reads and unpacks a new file (~0.5 s, estimate) |
| B | **One map, recoloured better** on the device | one file | a look can only be a function of Night's pixels: where a road crosses water, or a label sits on a hill, the colours can't be told apart again; the same family of loss as today's Ink. Rejected |
| C | **One layered map, coloured on the device**: land or water (1 bit), the terrain's shade (8 bits), and lines and labels as coverage (how much of each pixel each covers, 4 bits each: coast, rivers, roads, borders, label text, label halo), drawn on the PC; the look is a palette applied to the layers as the rows are unpacked | **one file per place for every look**; a look change is instant (no card read); any palette later (themes, colour-blind safe, the owner's own) for free; nothing lost: every layer stays separate until it's coloured, with anti-aliased edges; it is exactly what the X51v drawing its own map (M3's target) will produce, so step 5 replaces the drawer, not the colouring | more device code than copying rows; colouring costs CPU when the map is shown or the look changes (estimate: ~6 layers x 222,000 pixels, fixed-point, ~20-40 ms, to measure) |
| D | **Vector map drawn by the X51v now** (M3's target) | the end state | needs the format sheet and drawing-time measurements first (step 5); too big a jump for step 1 |

**Proposed: C.** It beats the phones' way on this hardware: they restyle
vectors on a GPU every frame; we style once per look change, on the CPU,
from layers made offline at the panel's exact pixels, so the X51v pays
nothing per frame. It beats A on card space (one file instead of one per
look) and on look changes (instant). And it is the half of M3 the device
keeps forever: when the X51v draws its own map (step 5), it draws these
same layers, and the colouring stays as it is.

## How it would be built (if C)

1. **The PC drawer** (`tools/mockups/radar_basemaps.py`) writes the layers
   instead of colours: one packed file per place and zoom (Paeth and zlib,
   the format the device already reads), with a small header (size, layer
   list, version) checked as hostile input (docs/security.md).
2. **The looks as palettes in code** (`rp_weather`): Paper, Ink, Night, each
   a colour per layer and a curve for the shade; Auto = Paper by day,
   Night by night. A test holds each look's coloured map equal to the PC
   drawer's own colouring of the same layers, pixel for pixel.
3. **The device** unpacks the layers once (they stay in RAM, ~1.3 MB for
   the six layers unpacked, estimate; kept packed if that's too much, and
   a row unpacked at a time) and colours the map into the backdrop on a
   look change; today's `map_row` Ink branch goes.
4. **The owner's place:** the relay draws the layers for the place shown,
   at zoom 7 to match today's rain tiles (Natural Earth and the open
   elevation tiles, both already used by the drawer), and sends one file
   instead of nine OpenStreetMap tiles; the device keeps it on the card.
   OpenStreetMap's tiles stop being fetched.
5. ui-preview shows each look on the device path before the run; emu-today
   and CI, then the owner's run.

## What would overturn it

- Colouring measured over ~100 ms on the X51v at a look change: then keep
  the coloured map (A's picture) for the shown look and the layers only
  for switching.
- The layers' file over A's single picture in size: then A for the
  built-in place, C only where looks switch.
