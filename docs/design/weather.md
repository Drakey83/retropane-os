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

- **One pack in the system** (flash), keeping the base small. **It is VClouds'
  weather icons** (owner, 2026-10-17), used with his permission and credited
  to him on the About screen and beside the files
  ([assets/weather/vclouds/](../../assets/weather/vclouds/README.md)).
- **Five more packs**, the owner's own (Golden Hour, Frosted, Noir, Aurora,
  Chrome), install onto the SD or CF card when one is present,
  from the expansion pack ([ADR-0018](../adr/0018-optional-packs.md),
  [apps-and-packs.md](apps-and-packs.md) §5). With no card, the one
  built-in pack is the only option.
- **Bring your own:** users add their own pack from the card.
- **Where the extra packs come from:** artists' packs with a free licence
  that allows shipping in RetroPane, ones the owner likes (still to find);
  or packs the owner makes. Never icons drawn by Claude. Ruled out by the owner for looks: Meteocons, Erik Flowers'
  Weather Icons, Oxygen, Tango.

### The pack format: keyed by the WMO weather codes

Forecast feeds send a condition *code*, not a picture: the World
Meteorological Organization's weather interpretation codes (the
WMO 4677 family, as the open feeds use them). A pack is a set of pictures
named for the icons below, each covering a group of codes, so any pack that
has every name works. Pictures are transparent PNGs, all the same size
within a pack: square and 512 px or larger (1024 or 816 are fine), or the
native size of an older set such as VClouds' (175 x 120, never stretched to
square); the build (or the installer, for a user's pack) scales them into the
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

## Mock-ups (2026-10-17)

The Today screen with the default pack, VClouds' icons. Credits and licences:
[mockups/README.md](mockups/README.md).

<table>
<tr>
<td><img src="mockups/today-clear.jpg" width="320" alt="Today screen, clear sky"></td>
<td><img src="mockups/today-heavy-rain.jpg" width="320" alt="Today screen, heavy rain"></td>
</tr>
<tr>
<td><img src="mockups/today-night-clear.jpg" width="320" alt="Today screen, clear night"></td>
<td><img src="mockups/today-thunderstorm.jpg" width="320" alt="Today screen, thunderstorm"></td>
</tr>
</table>

<img src="mockups/vclouds-icons.jpg" width="720" alt="The 22 VClouds icons the Today screen uses, at their own size, 96 px and 48 px">

The five optional packs (made by the owner), six icons each:

<img src="mockups/optional-packs.jpg" width="720" alt="The five optional icon packs: Golden Hour, Frosted, Noir, Aurora and Chrome, six icons each">

What the mock-ups showed: the icons keep real transparency and clean edges on
any wallpaper, and the sun, clouds and moon stay recognizable at 48 px; the fog
and clear-night icons (07, 02) are the faintest on a bright wallpaper.
