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

## Progress

- **2026-10-30, the owner's go, scoped:** "start building the foundation of
  it and how they all work together, i am mostly wanting the weather
  app/homescreen page/widget right now"; then "make this lean, modern,
  attractive, and functional ... animation where we can get away with it".
- **Built (step 6 first, on the shared pieces it needs):**
  - `rp-weather` (shell/weather): the one weather model the home screen's
    hero, the widget and the app all read (WMO codes to pack icons, units
    and regions, 12-hour clock, the sample forecast); 11 tests.
  - The Weather app in `rp-shell` (shell/core/src/weather_app.rs): the
    "mix" frame (header strip, tabs and softkeys at the bottom); Now (big
    temperature, sky, feels like, summary, wind, humidity, UV and sunset
    cards with bars), Hourly (24 hours, a temperature bar per hour,
    scrolling by D-pad or drag), 10 Day (each day's range on the week's
    scale). Opened from the Today hero (tap or D-pad), Home or the Start
    menu's Today back; pages by tab, D-pad or swipe.
  - Movement: each page arrives with its rows sliding in one after another
    (eased out), bars growing and the temperature counting up; frames 30 ms
    apart in `mockups/weather-motion-*.gif` (the preview records them).
  - The Today screen reads the same forecast; the title clock is 12-hour;
    temperatures may be negative or above 99 (they couldn't before).
- **Not yet:** theme roles and light/dark (step 1), wallpaper choice (2),
  the Classic desktop's widget placement (4), Places and the app's Menu
  (softkeys say "Not yet"), live data. rp-today is at the loader's image
  limit: R129 before the next screen goes in.

## The owner's verdict on the first device run (2026-10-30) and the next steps

He tried it on the X51v: "i dont love the animation, i cant drag any list up
and down, it doesnt do anything. it doesnt feel very polished or modern
(the animation and functionality is not there) there is no setting, no
easy way to get back home, and I dont see radar, or no way to put in my
api key". The first slice was short of the ask (memory: build the owner's
ask in full). Next, in this order:

**What the run measured (local/rp-today-weather-run1.txt):** every Weather
page frame took 85-100 ms to the screen: about 53-56 ms of CPU building
and queueing it and 29 ms in the 2D engine (a Today screen change: under
10 ms of CPU). The entrance therefore ran near 10 frames a second, which is
most of why it felt rough. The run log filled (64 KiB, per-touch lines) and
dropped the rest. Before any new movement: find where the 53 ms goes
(`PROFILE` zones; the many translucent gradients and text runs a page
redraws each frame), then move the page as a layer on the 3D compositor or
redraw only what changes, so movement reaches 30-60 fps, measured on the
X51v.

1. **Live touch:** `Pen::Move` already comes from the touch driver
   (shell/today/src/main.rs drops it); add `Input::PenMove`, lists that
   follow the finger 1:1 when pulled up and down, fling with momentum and
   a rebound at each end (the owner missed both, 2026-10-30), press
   states, swipes measured live. Proven on the X51v before anything else.
2. **Movement the owner picks:** moving mock-ups (GIFs from the preview's
   real frames) of at least three ways, e.g. the hero card expanding into
   the app (shared element), pages pushed sideways as one with depth, a
   living sky behind the app (ui.md's ideas); on the 3D compositor where
   that is smoother (ADR-0034, option D).
3. **A clear way back:** a back control in the app's header and on the
   left softkey (vote I7's proposal), Home as now.
4. **Settings:** units and region, places, the data source (sample,
   Open-Meteo, the owner's Tempest), refresh, icon pack.
5. **The Tempest key:** typed on an on-screen keyboard (**owner,
   2026-10-30:** "for now I will need to do it manually with a keyboard";
   pasting it from the browser is the optimal way once copy and paste and
   multitasking exist, now on the ROADMAP's Phase 5 list);
   either way stored by the device's own key store, never in the repo or
   typed by Claude. His key can already be checked from the PC
   (Tempest's REST API, his own PowerShell window).
6. **Radar:** the loop on our base maps (tools/mockups/radar_basemaps.py),
   sample frames until the network.

### Built since the verdict (2026-10-30, the owner's go on two sheets)

- **Step 0, speed** (sheet `research/sheets/weather-frame-speed.md`,
  9e7bad7e): the 53-58 ms frames were the 2D renderer rebuilding its
  whole-screen background whenever glass moved, then drawing two frames
  whole. Now it rebuilds only where glass changed, keeps gradient rows
  across frames, and has a clip command for lists. Host model: a sliding
  panel rebuilds < 80k px instead of 307k. **Device fps: not measured
  yet** (one run-log line per movement now says it).
- **Room** (sheet `research/sheets/today-image-room.md`, 0251653c): the
  wallpaper packed losslessly (460,800 -> 137,537 bytes, rp-inflate),
  unpacked at start into the RAM pool.
- **Step 1, live touch** (ca2ce776): lists follow the pen 1:1, fling and
  spring back past either end; press states; the swipe's underline follows
  the pen.
- **Steps 3-5** (625fea21): back in the header and on the left softkey,
  Settings (region, units, place, source, Tempest key, refresh, icon pack,
  radar map), Places, and the key typed on an on-screen keyboard (masked,
  last four shown; RAM only until the key store: R130).
- **Step 6, radar**: the fourth page, the sample's rain over our Night map
  (Ink made from it by day), a loop with play, pause and a timeline.
- **Step 2, movement:** four ways recorded from the preview's real frames
  (Stagger, Rise, Expand, None) and the live drag, for the owner's pick
  (R131); Stagger stays until he picks.
- **Device run 2 (2026-10-30, `local/rp-today-weather-run2.txt`):**
  entrances ~20 fps (from ~10), an Hourly drag ~14 fps, radar frames
  6-15 fps. **The owner, after it:** "This is the whole reason I wanted to
  have GPU figured out before i moved to UI/UX": movement goes on the
  GPU path decided in ADR-0034 (option D: panned pages on the planes,
  glass over them, 3D layers where those don't fit), not more 2D
  redrawing. Round 2's 2D sheet (`research/sheets/weather-drag-speed.md`)
  is set aside for it. **Next:** a sheet mapping each Weather movement
  (list drag and fling, page turns, entrance, radar loop) to ADR-0034's
  modes and what of the graphics service is proven on the X51v (R121's
  overlay, the 3D compositor harness), then the build.
- **Also from run 2:** Tempest can't fetch (no network on the device:
  proposed, a PC fetch over the dock's USB link first); the settings
  need a redesign as mock-ups (each one working or saying why not,
  Tempest's own options on a page of its own); the Night map is the
  default (30ca144c).
- **The owner's direction, later the same evening:** "one more design
  pass with 3d to get that working"; "I want full pass through of
  internet because we will use it a lot for testing if we are going to
  wait till the end to do wifi"; design work stays in mock-ups "until we
  have it more nailed down once we have 3d fully working and we can tell
  what kinda fps we are going to get"; and the GPU used only while the
  screen moves, idle when it is still, for battery (ADR-0034 already
  composes a still screen with nothing; the 3D core's clock or power off
  while idle is to measure: R133). Order: the 3D design pass (sheet), the
  USB internet pass-through (sheet, R132), then UI work back on the
  device with the fps known.
- **The 3D design pass, written (2026-10-30):** sheet
  `research/sheets/weather-gpu-movement.md`: each movement mapped to
  ADR-0034's modes (the page's glass on the video plane over a still
  screen 0, repointed; 2D only for Stagger and the radar's cells; 3D only
  for Expand), device tests M1-M7, votes V1-V3. Awaits the owner's go.
- **Open:** the owner's pick of the movement, I6 (the right softkey opens
  Settings, not a Menu: a vote).

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
