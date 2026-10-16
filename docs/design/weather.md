# Weather: the app, the Today screen and the icon packs

A headline feature Windows Mobile never had by default (owner, 2026-10-16).
The look is in [ui.md](ui.md) ("Weather and icons", "The smoked-glass design language", boards N-R); the app list it belongs to is in
[apps-and-packs.md](apps-and-packs.md).

## What it is

- **The Today screen's hero:** the current conditions large, front and
  centre, the sky as the wallpaper, the icon breaking out of its glass tile
  ([ui.md](ui.md)).
- **A Weather app:** now, hourly, days ahead, saved places.
- **Forecast data** over the network once networking lands (ROADMAP
  Phase 6): source, privacy and caching are a design to come; every feed
  is parsed as hostile input (`docs/security.md`).

## Icon packs (decided by the owner, 2026-10-16)

- **One pack in the system** (flash), keeping the base small: the reference's set
  if the artist grants permission (the owner asked him; no answer yet),
  otherwise the owner's choice.
- **Five more packs** install onto the SD or CF card when one is present,
  from the expansion pack ([ADR-0018](../adr/0018-optional-packs.md),
  [apps-and-packs.md](apps-and-packs.md) §5). With no card, the one
  built-in pack is the only option.
- **Bring your own:** users add their own pack from the card.
- **Where the extra packs come from:** artists' packs with a free licence
  that allows shipping in RetroPane, ones the owner likes (still to find);
  or packs the owner makes with the owner's own tools (below). Never icons drawn by
  Claude. Ruled out by the owner for looks: Meteocons, Erik Flowers'
  Weather Icons, Oxygen, Tango.

### The pack format: keyed by the WMO weather codes

Forecast feeds send a condition *code*, not a picture: the World
Meteorological Organization's weather interpretation codes (the
WMO 4677 family, as the open feeds use them). A pack is a set of pictures
named for the icons below, each covering a group of codes, so any pack that
has every name works. Pictures are square transparent PNGs in the
pack, 512 px or larger and all the same size (1024 or 816 are fine); the build (or the installer, for a user's pack) scales them into the
asset atlas, so they cost nothing while running (design-brief.md). A pack
is data only, checked like any input: size-capped, decoded by our own
validating PNG reader, refused if damaged.

| File | Icon | WMO codes |
|---|---|---|
| `01` | Clear sky, day | 0, 1 |
| `02` | Clear sky, night | 0, 1 |
| `03` | Partly cloudy, day | 2 |
| `04` | Partly cloudy, night | 2 |
| `05` | Cloudy | 3 (thin) |
| `06` | Overcast | 3 |
| `07` | Fog | 45, 48 |
| `08` | Drizzle | 51, 53, 55 |
| `09` | Light rain | 61 |
| `10` | Heavy rain | 63, 65 |
| `11` | Freezing rain | 56, 57, 66, 67 |
| `12` | Rain showers, day | 80, 81, 82 |
| `13` | Rain showers, night | 80, 81, 82 |
| `14` | Light snow | 71, 77 |
| `15` | Heavy snow | 73, 75 |
| `16` | Sleet | mixed rain and snow |
| `17` | Snow showers, day | 85, 86 |
| `18` | Snow showers, night | 85, 86 |
| `19` | Thunderstorm | 95 |
| `20` | Thunderstorm with hail | 96, 99 |
| `21` | Windy | (from wind speed, not a code) |
| `22` | Unknown / no data | anything else |

Open: the exact code-to-file mapping (some codes, such as 3 split into
cloudy and overcast, depend on extra data like cloud cover), and whether a
pack may leave optional icons out (night variants falling back to day).

## Making a pack with the owner's own tools (owner, 2026-10-16)

**Models:**

- **First choice: an image generator (image generator).** Follows long,
  exact lists well, and outputs transparent backgrounds. Give it the first
  good icon and ask for the same style for the rest.
- **For looks: an image generator with a style reference** (a style reference of the
  first icon). Often the most beautiful; no transparent background, so
  every icon needs background removal.
- **For full control: Flux** (Black Forest Labs), run locally, with a small
  style model (LoRA) trained on the best results to keep the set
  consistent. The most work.
- For any of them: check the terms allow redistributing the images, since
  packs may ship publicly.

**the reference-style packs (owner, 2026-10-16):** the owner wants six packs that
are all variations of the reference's look, and every part 1 names the reference. The
prompts are on the owner's private prompts page (a Claude artifact), not in
this repository. These are AI-made icons in a named artist's style, not
copies of his work. For the owner's own device that is the owner's call; for
a pack shipped in this public repository, the artist's permission is wanted
(asked, no answer yet), or the name comes out of the prompt and the pack is
judged on its own. The pack's licence note records which.

**Method:** generate the style block and one icon first (`03`, partly
cloudy, day); regenerate until it's right, then use it as the style
reference for every other icon so the set matches.

```
STYLE (use for every icon):
A single weather icon for a premium handheld device, photorealistic 3D render,
volumetric soft clouds with real depth and subsurface light, glossy and slightly
glassy materials, cinematic lighting from the upper left, subtle rim light,
rich but natural colours, crisp clean edges. Style inspired by modern
Rainmeter weather skins (the reference style): realistic, elegant, slightly
dramatic. The icon floats on a fully TRANSPARENT background, no ground, no
frame, no text, no shadow plate, no border. Centred, filling about 85% of a
square 1024x1024 canvas, with some elements allowed to extend near the top
edge. Must read clearly when shrunk to 96x96 pixels on a dark smoked-glass
panel: strong silhouette, high contrast against dark grey.
Consistent across the whole set: same camera angle (straight on, very
slight top-down), same light direction, same cloud material, same sun and
moon design, same scale.

ICON (one per image):
<insert one line from the list>
```

```
01 clear sky, day: a bright golden sun with soft glowing rays
02 clear sky, night: a crescent moon with a few small stars
03 partly cloudy, day: sun partly behind one fluffy white cloud
04 partly cloudy, night: crescent moon partly behind one cloud
05 cloudy: two layered grey-white clouds
06 overcast: heavy dark grey cloud mass
07 fog: soft cloud with horizontal mist bands below
08 drizzle: light grey cloud with fine short rain streaks
09 light rain: grey cloud with a few clear raindrops
10 heavy rain: dark cloud with dense diagonal rain
11 freezing rain: grey cloud with raindrops turning to ice crystals
12 rain showers, day: sun behind a cloud with raindrops
13 rain showers, night: moon behind a cloud with raindrops
14 light snow: soft cloud with a few delicate snowflakes
15 heavy snow: grey cloud with many falling snowflakes
16 sleet: cloud with mixed raindrops and snowflakes
17 snow showers, day: sun behind a cloud with snowflakes
18 snow showers, night: moon behind a cloud with snowflakes
19 thunderstorm: dark storm cloud with a bright lightning bolt
20 thunderstorm with hail: storm cloud, lightning bolt and white hailstones
21 windy: stylised curved wind streaks with a few leaves
22 unknown / no data: a soft translucent cloud with a small question mark
```

Name each file by its number (`03.png`); export square and transparent, 512 px or larger, every icon the same size (816x816 or 1024x1024).
