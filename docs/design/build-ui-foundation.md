# Build plan: the Weather app, the Classic and Modern foundations, themes and wallpaper

Part of the design area: [README.md](README.md). Written 2026-10-30 for the
owner's ask: "can you make the weather app and the foundation of what will
be our classic and modern layouts now please? Also you will probably need
to start on the wallpaper theme customization too." **A plan for his go
(the sheets rule): no code before it.**

## What it builds on (already in the tree)

- `rp-shell` (input, focus, Start menu, drawing into a frame), `rp-today`
  (the Today screen, VClouds' icons, clock, fonts), the 2D renderer
  (ADR-0023), the graphics service and 3D compositor (ADR-0026/0030/0034,
  ruled option D), and `cargo xtask ui-preview` (pictures of the shell's
  own code in seconds): [ui-workbench.md](ui-workbench.md).
- The designs: [ui.md](ui.md) (Classic and Modern are desktops only, every
  app one design, light/dark L1-L5), [interaction.md](interaction.md)
  (I1-I12), [weather-flows.md](weather-flows.md) (WF1-WF10 and the owner's
  direction), [weather.md](weather.md) (packs), [icons.md](icons.md),
  [today-screen-plan.md](today-screen-plan.md) (T3/T4 wallpaper).

## Built on the proposed answers (each one cheap to change later)

**37 votes are open:** I1-I12, WF1-WF10, L1-L5, D1-D4, and the verdicts on the
Weather mock-ups (the "mix" frame, light and dark). The build takes the
**proposed option of each** and keeps every one behind a single constant or
table, so a different vote changes one place and the preview shows it in
seconds. Nothing here becomes a ruling until the owner votes.

## The order, each step proven by the loop (preview, host tests, emulator, X51v)

1. **Theme core** (a new `shell/theme` crate): colour roles for light and
   dark with a build-time contrast check (L1), Auto by sunrise and sunset at
   the home place (L2), one accent; the icon atlas from icons.md (the base
   set rendered on the PC at build time).
2. **Wallpaper:** the built-in set (provenance already checked, ui.md),
   converted on the PC to the panel's pixel format, dithered once; picked in
   Settings, Themes & desktop. Pictures from the card wait for the storage
   service (T3), not for a vote.
3. **The app frame both desktops share:** the WM-lens frame from the "mix"
   mock-ups (header strip, content, tabs and softkeys at the bottom), the
   grid and targets (I1-I3), back and softkeys (I6, I7), D-pad focus (I8).
4. **Classic desktop:** free XP/Vista-style icons on the wallpaper under the
   weather widget (D1-D4), tap to open, tap and hold for the icon menu
   (Change icon included), arrange and grid.
5. **Modern home:** the approved Modern home (weather as the default home,
   SPB's whole experience as the reference), the same apps behind it.
6. **Weather app:** places, US units by default with metric and region
   (international rule), the WMO-keyed model, screens Now, Hourly, 10 Day,
   Radar (base maps later), Places, Settings; **sample data first** (no
   network on the device yet; live data later through USB to the PC).
7. **Settings, Themes & desktop:** wallpaper, light/dark/Auto, accent, icon
   pack (base set until the card's packs), desktop (Classic or Modern).

Each step lands with host tests and a preview picture; steps 3-6 get one
emulator check and one device run each (on the owner's "run it").

## Iterate, don't imitate (rule 0)

- **Colours.** WM: a fixed table of system colours. Android: Material
  tokens. **Ours:** roles with a contrast check at build time and Auto
  light/dark by the sun at home: no unreadable theme can be built, and
  the switch costs nothing while running.
- **Wallpaper.** WM decoded the JPEG at every boot and scaled it on the
  device. **Ours:** converted once on the PC (dithered for the panel's
  16-bit colour and its measured gamma), so it costs a copy at start-up.
- **Layout.** WM: each app laid out by hand per screen size. **Ours:** one
  frame and grid shared by every app and both desktops, so a vote such as
  I1 changes every screen at once.
- **Weather.** WM had no weather app. SPB drew a fixed set. **Ours:** WMO
  codes everywhere (any icon pack fits) and sample data first, so the
  whole app is proven before the network exists.

## Size and memory

Each crate gets a budget in the ratchet before it grows; the wallpaper and
the icon atlas sit on flash and are paged in, not held in RAM (memory
first). Figures to measure at step 1: atlas and wallpaper bytes, frame time
of each screen against the `rp-today` floors.

## Needed from the owner

- **Go on this plan,** building on the proposed answers above.
- **A fresh session for the build:** this one's context is long; the work
  runs best from a clean start that reads this page first.
