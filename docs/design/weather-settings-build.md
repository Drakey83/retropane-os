# Weather settings, built for real (T1-T4, K1 compact)

The owner, 2026-10-31: "Yes, build them" (the settings sheet
`mockups/settings-sheet.jpg` and K1 compact `mockups/keyboard-compact-sheet.jpg`,
dark and light), and "we should of been building out the framework for all
of this stuff while we were finalizing the design". This page is what is
built behind each screen, so nothing on them is a stand-in.

## What each screen does

| Screen | Works by |
|---|---|
| T1 Settings | One grouped list (Places, Weather from, Look). Changes apply at once; ok and Back return. Every row reads the settings store. |
| T2 Add a place | The typed name or ZIP / postal code goes to Open-Meteo's place search (free, worldwide, no key) through the dock; up to 8 matches (name, state or region, country). **Near your dock PC** asks the PC's own Windows location (no IP-location company), named by OpenStreetMap's Nominatim (one request, rounded to about 1 km). Measured on the owner's PC 2026-10-31: permission granted but no position (a desktop without Wi-Fi positioning), so the screen says so and the search remains. Up to 10 places (WF8), one home. |
| T3 Weather from | The forecast provider: Open-Meteo (default), National Weather Service (US), MET Norway, Tempest's forecast (needs a Tempest). The station list: Tempest (connected through the relay's key), and the others shown with what each needs. |
| T4 Pull to refresh | Pulling the page down past the rebound at the top asks for fresh data; the Refresh softkey does the same for the D-pad. The header says how old the data is. |
| K1 compact | K1's layout (number row, QWERTY, ?123, @, space, ., Done, the Paste strip, the pressed key shown above) at stylus size (30 px keys, the keyboard 208 px), docked on the softkey bar. |

## Rule 0

- **Windows Mobile / HTC / SPB (finding):** settings were property sheets
  of drop-downs behind OK; places came from a fixed city list (HTC) or a
  vendor's feed; there was no ZIP search, no pull to refresh, no choice of
  provider; the SIP was a 15 px panel with no paste.
- **Modern best:** a grouped list with live changes; fuzzy place search by
  name or postal code; provider choice where the trade-off is stated; pull
  to refresh; a phone keyboard with a suggestion / paste strip.
- **Ours and why it wins:** the modern ones, fitted to this device. The
  place search uses Open-Meteo's geocoder (one keyless call, postal codes
  included, recorded samples in `shell/weather/tests/data`). "Near your dock
  PC" uses the PC's own location service instead of an IP-geolocation
  company (named by OpenStreetMap's Nominatim, the position rounded). The forecast is
  parsed **on the device** from each provider's own JSON (allocation-free,
  every number range-checked, as `tempest.rs`), so when TLS moves onto the
  device (C95) nothing but the transport changes. Pull to refresh is the
  page layer's own rebound on the plane path, so showing it costs nothing.

## The dock link, version 2 (dev only, C95)

The device names *what* it wants; the relay maps it to a fixed URL. The
device can never make the PC fetch an address of its choosing.

| Device sends | Relay fetches |
|---|---|
| `RPNET FIND <text>` | Open-Meteo geocoding `search?name=<text>&count=8` (text: letters, digits, space, `-`, `'`, `,`, `.`, at most 40) |
| `RPNET HERE` | the PC's Windows location, named by Nominatim, as a place-search answer |
| `RPNET LOAD` / `RPNET SAVE <825>` | `local/weather-settings.bin` (the record; since version 2 the station key too, the owner 2026-10-31: a key entered once keeps working, until R130's key store) |
| `RPNET CLIP` | the PC clipboard's first line (printable, 128 at most) |
| `RPNET OPENMETEO <lat> <lon>` | Open-Meteo `forecast` (current, 24 hours, 10 days, unixtime, the place's time zone) |
| `RPNET METNO <lat> <lon>` / `RPNET NWS <lat> <lon>` | MET Norway compact / NWS points then hourly and daily (User-Agent as their terms ask) |
| `RPNET TEMPEST` | as before (the key from `local/tempest-key.txt`, never over USB) |

Latitude and longitude are checked as numbers in range by both ends.
Answers are `RPNET <status> <length>` then the body, as now.

## The settings store

`rp_weather::store`: the settings and places as a small versioned record
(magic, version, length, CRC-32, then fields); a damaged or unknown one
reads as the defaults, never as garbage (tests: every truncation, every
flipped bit). The Tempest key is never in it (R130: the protected key
store). **Where it is kept on the X51v:** in the image's RAM while it runs,
and through the dock (`RPNET SAVE` / `RPNET LOAD`, `local/weather-settings.bin`
on the PC) between runs, until the loader reserves a settings file on the
card (R130). The format does not change when it moves.

## What overturns this

A provider's terms that forbid this use (R125 checks each before the
sources ADR); the PC's location service being off (then Near your dock PC
says so and search still works); the owner preferring other screens.

## Build steps (ticked as they land)

1. [x] rp-weather: `json` (shared reader), `Name`, `open_meteo` (forecast,
   place search; recorded samples), settings model (places with positions,
   provider, station, appearance), `store` (record, CRC, tests).
2. [x] rp-shell: `keyboard` (K1 compact: 30 px keys docked 6 px over the
   40 px softkey bar, ?123 layer, shift, the pressed key shown above,
   Paste and From PC chips), T1 settings list, T2 add a place (search as
   typing pauses, Near your dock PC, matches), T3 weather from (providers,
   stations), places list with Remove, the key screen on K1 compact.
3. [x] rp-shell: the dock asks (`Shell::next_ask` / `Shell::net_answer`:
   LOAD, SAVE, FIND, HERE, OPENMETEO, TEMPEST, CLIP), "updated N min ago",
   the Refresh softkey and pull to refresh past the top rebound, the
   Weather screen's softkeys Refresh / Menu (T4).
4. [x] rp-today: the device loop drives the asks over the USB serial port.
5. [x] xtask net: the verbs (NWS not yet served) (fixed URLs, arguments checked), the settings
   file, the PC's Windows location, the clipboard.
6. [ ] MET Norway and NWS readers (recorded samples), Tempest live "now"
   over any provider.
7. [ ] Light appearance across the shell (the colours as a theme).
8. [ ] Host tests (done: workspace green), emulator suite, cross build
   (done: 813,364 B loaded after packing the pictures as the wallpaper is,
   121,765 B from 274,240; they were runs, 225 KB), device run (owner's go).
