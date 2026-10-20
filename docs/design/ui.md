# RetroPane UI: direction (in progress)

Part of the design area: [README.md](README.md).

Nothing here is approved for code yet (ROADMAP Phase 5: spec and mock-ups
approved before any UI code). This page records the owner's choices as they
are made, and links the concept mock-ups.

## Decided by the owner (2026-10-12)

- **Overall feel: hybrid.** Windows Mobile's proven structure (Today screen,
  Start menu, two softkeys at the bottom, stylus-precise) with modern
  touches: a notification shade, a smooth task switcher, live widgets on the
  Today screen, a first-class dark theme. Old WM programs keep their own
  title bars and softkeys and fit in naturally.
- **Input: stylus and D-pad equally.** Everything works with the stylus,
  and everything also works one-handed with the D-pad and buttons (a
  visible focus ring, keyboard-style navigation). Buttons keep exactly the
  jobs Windows Mobile gave them (owner's rule). Finger use joins them in the
  default Modern look (2026-10-19, below: SPB Mobile Shell's philosophy).
- **Always visible, like a phone:** time, battery and Wi-Fi, at least in a
  status bar on every screen, apps included.
- **Today screen:** a good weather display with the clock and calendar by
  default, fully customisable (add, remove, reorder widgets) if the owner
  doesn't like the defaults.
- **Visual style: not chosen yet.** Three looks of the same Today screen
  were mocked for comparison: A clean flat, dark-first; B glassy accents;
  C retro-modern (concept canvas, private to the owner).

## SPB Mobile Shell's philosophy (owner, 2026-10-19)

**The scope (owner, 2026-10-19, later the same day): the whole
experience,** not one feature: all of SPB Mobile Shell's UI, how each part
is interacted with (touch, scrolling, menus, transitions, feedback) and its
design language as a whole, **with our own take on it.** Two points of
ours already fixed by the owner: **the weather screen is the main screen
by default** (what the device opens to), and **everything is
customisable, themes included** (a theme carries layout as well as skin:
DESIGN.md §7). The carousel below is one part of it.

**Borrow heavily from SPB Mobile Shell's design philosophy**, keep the retro
charm, and modernise it. SPB Mobile Shell (SPB Software, 2007-2010) set out to
do on Windows Mobile what RetroPane wants to do: keep WM underneath and give
it a friendlier, more modern face. Its ideas, as remembered (to be checked
against screenshots and the manual before any concept uses them):

- A Today screen built around a large clock, the weather and the next
  appointment, readable at a glance.
- Big, finger-friendly targets: a full-screen launcher grid of large icons,
  quick contacts with photos.
- Widgets in panels you swipe between (version 3's 3D carousel), not one
  long list. **The carousel** (owner, 2026-10-19: the name is right): the
  panels on a 3D rotating ring, turned with the finger. The owner points
  out the 2700G suits effects like it (textured quads, bilinear filtering,
  the render-done interrupt: docs/hw GPU facts). Its look and motion are
  to be studied from the owner's screenshots and video split into frames
  (rotation, easing, timing, depth and lighting); how it felt to the
  finger (response, stutter) from the owner's own words. "Carousel" is
  SPB's feature named for reference, not our name for ours.

  **What SPB's own demo video shows** (SPB Mobile Shell 3.5, YouTube
  `IGrfQ7IKGBs`, 640x480, stepped through frame by frame in the app's
  browser, 2026-10-19; observations to check against the owner's memory,
  nothing copied):
  - **At rest:** one panel full-face in the middle (each panel titled in
    a bar at its top: "Lifestyle Home", "Contacts", "Favorite Contacts",
    "Weather"...), its neighbours visible at both edges, turned away and
    foreshortened, so the ring reads as a drum seen from the front. A
    glossy black floor reflects the panels; the background is near
    black. The Windows Mobile title bar and two softkeys stay as they are.
  - **A turn:** the view pulls back (the front panel shrinks within about
    0.2-0.3 s), the panels sit small and dim on the drum, the drum turns,
    then the view zooms in on the chosen panel and it brightens. Whole
    turn about 1.5-2 s in the demo (a demo may be slowed for the camera).
  - **The rest of the video, part by part** (the whole experience, as the
    owner asked):
    - *Home (Today):* a big digital clock and the date over a full-screen
      picture (sky, leaves), the weather, the next appointments, two
      large buttons at the bottom (Launcher, Contacts) between the
      softkeys. Later a variant with social status widgets ("What's on
      your mind" boxes) over the sky picture; tapping one opens a
      full-screen entry with the on-screen keyboard.
    - *Launcher:* a full-screen grid of large labelled icons (3 across),
      pages of them; programs and settings alike.
    - *Customising:* a "change background" sheet over the dimmed home
      screen (big choices: pictures, gallery, colours), then a grid of
      background pictures with names; themes swap the whole look.
    - *Contacts:* a grid of favourite contacts as large photos (3 across,
      edge to edge), a contact's action sheet (call, message, change the
      photo, link to a social profile) as big icon tiles, and the contact
      card: the photo large, one wide green Call button, Send SMS / Send
      MMS beside it.
    - *Messages:* conversations as bubbles with the sender's photo.
    - *Agenda:* the day's appointments as a list under a date bar.
    - *Throughout:* black or dark glass behind bright content, big touch
      targets for navigation, sheets that slide over a dimmed screen, the
      Windows Mobile title bar and softkeys kept at the top and bottom.
  - **Fits the 2700G** (docs/hw GPU facts): each panel one textured quad
    (rendered once into a texture, redrawn only when it changes), the
    floor reflection the same quads mirrored and faded, the pull-back and
    turn just their transforms: a few quads a frame. Open: the frame rate
    and the finger's response the owner remembers.
- WM itself still there underneath: programs, Start menu, softkeys.

How it fits what's decided: the **hybrid** feel, live **widgets** and a
**weather-led** Today screen (above) are the same direction; the smoked
glass (below) is our own look, not SPB's. The guardrails: it's a
philosophy, not a clone (no SPB artwork, layouts or names: we design our
own, as for Microsoft's UI, docs/CLEANROOM.md); buttons keep Windows
Mobile's jobs (owner's rule); each idea taken is weighed like any other
design choice, not adopted because SPB did it.

**Decided by the owner (2026-10-19): both, switchable.** SPB was
finger-first, while RetroPane was decided as stylus and D-pad equally; the
answer is two looks, chosen in the customisation settings and switchable
back and forth at any time:

- **Modern (the default): SPB's shape.** Finger-friendly: big targets on the
  Today screen and the launcher, widget panels you swipe between, readable
  at a glance. The stylus and the D-pad still work everywhere.
  **The finger is for navigation only** (owner, 2026-10-19): moving through
  menus, icons and screens. The stylus and the buttons work everywhere, and
  they are the way to do anything the small screen makes precise: typing on
  the on-screen keyboard (it stays stylus-sized), selecting text, small
  controls, old Windows Mobile programs. So only navigation targets grow;
  precise controls keep their stylus size in both looks.
- **Classic: a Windows Mobile 5-style look**, at stylus scale, as DESIGN.md
  §7's theme engine already planned ("a faithful WM5-style homage, using our
  own artwork").

This makes the theme engine carry **layout** as well as skin: target sizes,
the launcher's grid and the Today screen's arrangement differ between the
two looks, not only colours and art. Both looks are designed, mocked up and
tested; old Windows Mobile programs keep their own windows in either.
**The names are Modern and Classic** (owner, 2026-10-19): plain words, and
never "SPB" or "Windows Mobile" (their owners' names).

## Constraints every concept respects

480 x 640 at 3.7 inches; resistive single-touch stylus plus D-pad and the WM
buttons; software rendering until the 2700G 2D blitter is supported, so
the framebuffer is write-only, translucency is used sparingly and chrome is
pre-rendered (docs/design-brief.md); 64 MB of RAM (docs/memory-budget.md).

## Weather and icons (owner, 2026-10-12)

- The weather is the hero of the Today screen: large, front and center.
- Icon language: photoreal, volumetric weather icons (soft 3D clouds,
  glowing suns, a cratered moon, on a dark, moody backdrop). Where the icons come from (one
  pack built in, more on the card, bring your own, keyed by the WMO codes):
  [weather.md](weather.md). No third-party icon is shipped without a
  licence that allows it. On the device the icons are pre-rendered into
  the asset atlas at build time, so realism costs nothing while running
  (docs/design-brief.md).
- Mock-ups D-H on the concept canvas; G (Today screen) and H (the icon
  family: sunny, clear night, partly cloudy, cloudy, rain, thunderstorm,
  snow, cloudy night) use the photoreal icons.

## An ode to Windows Mobile (owner, 2026-10-12)

- The design is an ode to Windows Mobile: WM's structure (status bar, Today
  screen, Start, two softkeys) and the WM 6.x era's best home screen, HTC
  Sense (TouchFLO 3D): the flip clock above the weather, painted sky
  scenes, a daily/hourly forecast, modernized; plus the calendar line and
  the quick-launch row from the first concepts.
- **No Windows 8 / Metro design language:** no live tiles, no flat
  single-colour squares, no big lowercase pivot headings.
- No other company's marks: no Microsoft four-colour logo, no HTC or
  Microsoft fonts (Segoe is not ours to ship), no copied artwork.
- Concepts I (Sense tribute) and K (full sky) on the canvas follow this;
  J (tiles) is set aside.

## More owner decisions (2026-10-12)

- Not a phone: no Contacts in the main design (softkeys: Calendar and Edit
  Today; the Start menu lists File Explorer instead).
- WM feel is a must: the title bar with Start at top left, a WM-style
  Start menu (recent programs row, then Today, Calendar, File Explorer,
  Messaging, Browser, Programs, Settings, Help), the two softkeys.
- The weather blends into the desktop: the wallpaper is the current sky,
  the reading sits on it with no box. Concept A's calendar card and quick
  buttons are kept as they are.
- Icons: the owner wanted an existing artist's realistic set under a free
  licence rather than generated ones. **Settled 2026-10-16/17:** the set the owner
  liked best is VClouds' (CC BY-NC-SA 3.0, non-commercial, which a GPL OS can't
  ship without the artist's permission); the owner asked, and **VClouds agreed on
  2026-10-16** to its use in RetroPane with credit. His icons are the default pack
  ([../../assets/weather/vclouds/README.md](../../assets/weather/vclouds/README.md),
  [weather.md](weather.md)). The alternatives checked earlier, not needed now: KDE's
  Oxygen (LGPL-3), MerlinTheRed's plain set (CC BY-SA), the Xfce "liquid" set
  (licence unclear, avoid).

## A PDA, proudly (owner, 2026-10-12)

RetroPane celebrates what PDAs excel at and doesn't pretend to be a phone:

- **The organiser first:** calendar, tasks with real checkboxes, and notes
  written with the stylus, each one tap away on the Today screen.
- **Capture:** a quick note, and the Axim's Record button for voice memos.
- **The VGA screen for reading, documents, music, video and games** (the
  Axim's 3D chip).
- **Two card slots:** SD and CF storage, shown in File Explorer.
- **Nothing phone-ish:** no Contacts or SMS front and center; mail over
  Wi-Fi is an Inbox.

Today screen quick buttons: Note, Record, Reader, Media, Games. Start menu:
recent programs, then Today, Calendar, Tasks, Notes, Documents, File
Explorer (SD · CF), Inbox, Browser, Programs, Settings.

## The smoked-glass design language (owner, 2026-10-12)

The look the owner chose: smoked glass over a wallpaper, "a good mix of retro and modern".

- **Smoked glass on a wallpaper the owner chooses.** Panels are dark, translucent glass. The gradient runs from near-black on the left to a lit grey on the right. Each panel has a 1 px bright edge, an almost square corner (3 px) and a soft gloss. A thin vertical light streak sits on one edge, and there is a faint reflection under the bottom row. Everything floats over a full-screen photo.
- **Wallpapers.** We ship a few that suit glass: dramatic, moody photography with dark areas where the panels sit, such as mountain lakes and canyons at sunset, an aurora, fog, a black-and-white lake (the approved list is under "Wallpaper provenance"). Users can pick their own picture from SD or CF. Settings will offer glass darkness and dimming the wallpaper at night. Any wallpaper we ship must be our own or CC0/public-domain and recorded in the licence list.
- **Type.** Thin geometric sans, big light numerals (`18°`) and spaced capitals for labels and dates (`MON 12`, `OCTOBER`). Century Gothic is Monotype's and is never shipped. The concepts use Didact Gothic (SIL OFL) as an open stand-in.
- **Icons break out of their frames.** Weather icons are large and overhang the top edge of their glass tile.
- **The WM structure stays.** The title bar has Start at the top left and status at the right. The Start menu is a glass panel dropping from Start. Softkeys sit at the bottom.
- **Clean-room.** The glass panels are rebuilt from plain gradients, not from anyone's images. No third-party icon art ships without a licence that allows it; the weather icons are covered in [weather.md](weather.md).

**Weather icon packs (owner, 2026-10-16):** see [weather.md](weather.md).

**Owner verdict (2026-10-12): "I LOVE it."** Boards N–R (smoked glass over photo wallpapers) are the chosen direction. The cartoon sky wallpaper is dropped. The candidate wallpapers we'd ship are all free of restrictions, from Wikimedia Commons, cropped to 3:4:

| Wallpaper | Commons file | Licence |
|---|---|---|
| Pier, black and white | `File:Monochrome pier with sky (Unsplash).jpg` | CC0 |
| Mountain lake | `File:Lake Blanche Reflection (31122974596).jpg` | Public domain |
| City at night | `File:Long Exposure Photography of City Buildings.jpg` | CC0 |
| Aurora | `File:Night landscape northern lights scenic aurora borealis.jpg` | Public domain |

**Goal (owner, 2026-10-13): at least 10 approved wallpapers ship. Met 2026-10-17:**
the owner approved 15 from 47 candidates and then cut them to ten (the table under "Wallpaper provenance"
supersedes the four above, of which only the lake is still in use as a built-in
default candidate). All ten are fully licensed. The rule stays: our
own, CC0 or public domain, recorded with their Commons file and licence.
**Where they live (owner, 2026-10-13):** the ten ship as a wallpaper pack
on the SD card (ADR-0018); the base system carries just one default, so
the OS stays small. Sizes for scale: one 480 x 640 wallpaper is 450 KB
as the video plane's raw 4:2:0, roughly 80-150 KB as a JPEG, so the pack
is about 1-1.5 MB on the card. The one in use is copied (converted) into
flash when picked, as below, so the screen never depends on the card.

**Fonts the same way (owner, 2026-10-13).** The base system carries one
font, Didact Gothic (SIL OFL) at the sizes the shell uses (~33 KB of
glyphs today). More font choices ship as a font pack on a card; the SD
card or the CF card can hold packs, wallpapers' included. Without a card
there is the one font; a font the user picks from a pack is copied into
flash, so text never depends on the card either. Every pack follows
ADR-0018 (optional packs) and its licences are recorded.

The concepts keep the glass light enough that the photo shows through. A "glass darkness" setting covers busy photos where small grey text gets hard to read.

**Seeing a concept on the real Axim.** Render the board at 480 × 640, convert it to RGB565 (dithered) at `local/concept/picture.rgb565`, then run `cargo xtask emu-picture` (pixel-exact on the emulated screen, clean return) and `cargo xtask load rp-picture`. The Axim shows it for 20 s, then goes back to the loader. `local/` is never committed, so concept art with third-party pieces never reaches the repository. Without the file, the image shows a test gradient.

## Rendering the glass on a 624 MHz XScale (review, 2026-10-12)

An outside review warned that glass is the most compositing-heavy look
possible on this CPU. That's true of *frosted* glass (a blur under every
panel, every frame). The smoked-glass look is tinted glass with no blur, and
it's mostly static, which changes the cost completely:

- **Superseded by item 60 (2026-10-12): the display controller blends at
  scan-out.** The wallpaper sits in the video plane, the interface in the
  graphics plane (drawn in a full-colour ARGB8888 layer in RAM, 1.2 MB,
  flushed dithered to ARGB4444), and the 2700G blends them every frame at
  no CPU cost (docs/hw/2700g.md "Plane composition"). The rest of this
  section was the earlier single-surface plan, kept for its reasoning:
- **Cache the desktop, don't recompose it.** Wallpaper plus every glass
  panel is blended once, into one 480 × 640 RGB565 surface in RAM (600 KB,
  counted in the shell's allotment), when the wallpaper, the layout or the
  darkness setting changes. Live content (the clock, the weather, a task
  ticked) is redrawn over that cache in its own dirty rectangle only, and
  only that rectangle goes to the write-only framebuffer (docs/hw/2700g.md).
  A clock tick is about 2 % of the screen.
- **What a full software recompose costs** (to measure in item 52, these
  are estimates): an alpha blend of 307,200 RGB565 pixels is a few cycles
  per pixel with iWMMXt (four pixels per operation), so a few milliseconds
  of CPU; memory traffic (about 1.2 MB in and out) and the push to the
  2700G's local memory dominate, roughly 15-30 ms. A full recompose is
  fine on a wallpaper or theme change and too slow to do every frame, so
  nothing does it every frame.
- **What moves:** the Start menu opening, scrolling lists and page
  transitions. Each moving panel is pre-rendered once (its glass baked
  over the cached desktop under it) and then copied, not re-blended, as
  it moves; the 2700G's 2D blitter does those copies when we have it
  (ROADMAP Phase 7), and iWMMXt copies do them before. The window-manager
  spec (Phase 5) records a frame budget for the software path next to
  each animation, so the look never depends on the blitter.
- **Glass darkness slider:** the preview recomposes one sample panel
  while dragging; the whole desktop recomposes once, on release.
- **No blur, ever,** unless measured affordable; the look doesn't need it.

## 3D acceleration for the whole UI (proposal, 2026-10-19)

The owner asked (2026-10-19) how the 2700G's 3D core could make the
SPB-style experience run smoother. **A proposal to weigh, not a decision**:
the numbers it needs are GPU facts still to measure (below), and the
owner votes once they're in.

**In plain words.** Today's plan has the CPU draw each screen and the
2700G copy or blend whole planes. The CPU is slow at the one thing
animation needs most: writing pixels into the 2700G's memory (32-41 MB/s
measured, so one full screen takes ~15 ms, a whole 56 Hz frame). The 3D
core is fast at it and costs the CPU nothing (a frame is a few hundred
bytes of commands). So: **the CPU draws each part of the screen once, into
its own picture in the 2700G's memory (a layer), and the 3D core puts the
layers on screen every frame**, moved, turned, scaled, faded. Nothing is
redrawn to move. This is how modern phones compose their screens (iOS's
Core Animation, Android's SurfaceFlinger); on this chip it is what makes
the SPB look smooth instead of merely possible.

**What it gives, part by part:**
- **The carousel:** each panel a textured quad, its floor reflection the
  same quad mirrored with an alpha fade, the pull-back and turn only their
  transforms; mipmaps (measured working) keep a shrunken panel from
  shimmering, bilinear filtering (measured) keeps it smooth.
- **Scrolling lists, the launcher's pages, contacts' photo grid:** the
  content is a tall layer (or tiles of it); a scroll is a new texture
  offset, so flick-scrolling with momentum runs at the panel's full
  56.46 Hz however long the list. The CPU draws only the rows coming into
  view.
- **Sheets over a dimmed screen** (menus, the "change background" sheet,
  the on-screen keyboard): the dim is one translucent quad; the sheet
  slides as a quad. The smoked glass (above) becomes a tinted quad over
  the layers under it, at no CPU cost, even while things move.
- **Frosted glass, if wanted:** a blur behind a panel, cheaply, from a
  small mipmap level of the layer under it sampled with bilinear filtering
  (a fake blur the GPU does for free); today's rule "no blur, ever, unless
  measured affordable" stays until it's measured.
- **The weather screen (the default home):** the sky picture, clouds on a
  few layers moving at different speeds (parallax), rain or snow as small
  textured quads, the clock and the forecast as layers over them: live
  weather that moves, for a few quads a frame.
- **Transitions everywhere** (opening a program, the task switcher, a theme
  change cross-fading): quads and alpha, no redraw.
- **Themes:** a theme is layers, textures and a layout; swapping it
  re-renders the layers once. Pictures can be stored compressed (PVRTC, 4
  or 2 bits a pixel, measured working): a full-screen wallpaper in 256 KB
  of the 2700G's memory instead of 600 KB-1.2 MB, read faster too.
- **Old Windows Mobile programs:** each draws into its own layer, as
  today's plan already lets them (ADR-0018); the compositor treats it like
  any other, so even they get smooth switching and transitions.
- **Power:** the CPU sleeps between frames; frames are timed by the frame
  event (VEVENT0, measured once a frame) and the render-done interrupt
  (measured), and when nothing moves nothing is drawn: the screen holds
  its last picture at no cost.

**Costs and risks (the honest side):**
- **Memory:** a 480 x 640 layer is 600 KB at 16 bits, 1.2 MB at 32; the
  2700G has 16 MB. Ten full-screen layers fit with room for textures, but
  not unlimited: layers are made for what's on screen or about to be, and
  dropped after.
- **Speed (measured, docs/hw/2700g.md S26):** a full-screen render (2,400
  tiles of 8 x 16) took **12.5 ms** with Windows Mobile's core clock
  (49.8 MHz), exactly in proportion to the clock. A 56.46 Hz frame is
  17.7 ms, so at today's clock one full-screen composition a frame fits
  with ~5 ms to spare: smooth for a carousel or a sliding sheet, but not
  for stacking many full-screen translucent layers. The levers, each
  measured or queued: render only the tiles that changed (a sheet sliding
  over a still screen touches a band of tiles, not 2,400); a faster core
  clock (Dell's driver allows 75 MHz, its default setting gives 66.5 MHz;
  `claude/mbx3d-render-2`'s v29 option A at 75 MHz is built and unrun),
  each MHz is speed; and a tile-based renderer (PowerVR MBX) skips hidden
  opaque pixels, so only translucent layers really add cost. What a
  *textured, blended* tile costs against the measured flat fill is not
  measured yet.
- **Texture sizes:** whether textures must be powers of two (a 480 x 640
  layer then sits in 512 x 1024) is not measured yet; it decides the
  memory per layer.
- **Text:** stays the CPU's (drawn into layers, crisp at its size); the 3D
  core only moves it. Text in a turning panel is a picture of text: fine
  while moving, re-drawn sharp when it stops.
- **One more driver to get right:** the compositor lives on our own GPU
  driver (owner's rule: purpose-built for this workload), so the MBX
  bring-up (docs/hw/2700g.md, items 69+) becomes UI-critical. A software
  fallback keeps every screen working if the 3D core is unavailable
  (slower, but never broken).

**The options:**
- **A: planes only (today's plan: ADR-0022 as accepted, its point 3:
  scan-out planes first, the 2D engine second, the CPU last).** Wallpaper
  and interface planes blended at scan-out, the 2D blitter copying moving
  panels. Simple and
  proven so far; no carousel worth the name, scrolling limited by copies.
- **B: a 3D compositor for everything (recommended, pending the
  measurements; it would amend ADR-0022's point 3, so it's an ADR change
  for the owner's vote, with ADR-0030's graphics service).** Every screen part a layer; the 3D core composes each
  frame that moves. The SPB look at full frame rate, the CPU free.
- **C: the 3D core only for the showpieces** (carousel, weather, task
  switcher), planes and the blitter elsewhere. Less to build first, but
  two ways of drawing to keep in step, and lists and sheets don't get
  smoother.

- **D: hybrid (added 2026-10-19 from the measurements).** Whole-screen
  moves (page swipes, flick-scrolling, sliding whole screens) by the
  display itself: pointing the plane at another picture or another place
  in a taller one, in the blanking, at the full 56.46 Hz for no render
  at all. The 3D core composes what changes in part (sheets, dimming,
  glass, fades, the weather's clouds, the carousel's panels), rendering
  only the changed tiles under a frame budget. The SPB look smooth at
  today's clock; two tools inside one compositor; some effects shaped to
  fit (a carousel turn drawn as moving panels, not a full-screen
  picture). A faster core clock, if proven, lets it lean toward B.

**Owner's vote (2026-10-19): D, the hybrid**, after `rp-gpubench`'s
numbers (below). It amends ADR-0022's point 3: the amendment was signed
off by the owner the same day (ADR-0022, "Amendment to point 3").

**To measure first** (a GPU facts batch, on the device, owner watching,
only on the owner's go; listed for the work order: GPU facts before UI
design): textured fill rate at 16- and 32-bit targets, with and without
bilinear and alpha blending; whether non-power-of-two textures work; how
long a frame of N full-screen layers takes, and a partial frame of a few
tiles; the cost of uploading a layer (the CPU's writes, 32-41 MB/s
measured, or a DMA copy from RAM, not measured); whether rendering can
track the frame event without tearing.

**Measured (`rp-gpubench` run A, 2026-10-19, twice; docs/hw/2700g.md
"Texture shapes"), at today's core clock; results for the vote, not a
decision:**
- A 480 x 640 layer is one 512 x 1024 texture (1 MiB at 16 bits):
  rectangles drawn right.
- **One full-screen textured layer takes 25-29 ms** (the background alone
  12.7 ms), longer than a 17.7 ms frame; each translucent full-screen
  layer adds ~20 ms; hidden opaque layers are almost free (8 stacked:
  40 ms). PVRTC is the cheapest texture (21-23 ms). A 16-bit screen saves
  0-3 ms. So **recomposing the whole screen every frame runs at ~35 Hz
  at best today**, not 56.
- **Only what changes is cheap:** under three translucent layers a list
  row (480 x 16) re-renders in 1.6 ms, an icon in 0.85 ms, a 480 x 160
  band in 15.5 ms.
- Uploading a layer: 18.5 ms in row order, 57 ms reordered for the GPU
  by the CPU, or reordered in RAM (37 ms) then copied by DMA (34 ms, the
  CPU mostly free).
- In step with the frame: flips at the frame event don't tear (owner:
  smooth), and one screen can be redrawn in bands just behind the scan
  line (owner: smooth), which saves the second screen buffer. A flip
  takes effect at once, so it is written in the blanking.
- No row-order texture mode was found (run B, 20 tries of the bits Dell
  never sets): layers are reordered for the GPU on upload.
- Still open: a faster core clock (each MHz is speed; version 29's
  75 MHz was rejected as built).

## Ideas: modern 3D in a retro-modern UI (proposal, 2026-10-19)

The owner asked for cool, modern-looking ways to use the 3D core in this
retro-modern UI. **Ideas to weigh, not decisions**; each rests on the 3D
compositor above (option B) and on the measured budget: about one
full-screen composition a frame at today's core clock, so the trick in
every idea is that **only what moves is re-rendered, and the rest of the
screen holds still**. The X51v has no motion sensor, so depth follows the
stylus, the finger and the panels, never tilting the device. Within the
rules: no Metro look, no other company's artwork, buttons keep Windows
Mobile's jobs, and every effect switchable off in the settings (themes may
turn them on or off too).

**The signature set (recommended to try first):**
1. **A 3D flip clock on the weather screen.** The ode's flip clock, now
   real: each digit a pair of half-cards that hinge over in perspective
   when the minute turns, lit from above (the falling half darkens as it
   turns, a soft shadow under it). Retro object, modern motion; a few
   small quads once a minute, so nearly free.
2. **A living weather sky (the default home).** The painted sky as depth
   layers: far hills, near clouds, the sun or moon on its true arc for the
   time of day; clouds drift at the forecast's wind speed and direction;
   rain and snow as falling quads at the forecast's intensity; a
   lightning flash lighting the clouds from behind. Swiping between home
   panels shifts the layers at different speeds (parallax), so the sky
   has depth without a sensor. Moves slowly, so it can run at a low frame
   rate (10-15 Hz) except while you swipe.
3. **Glass that catches the light.** The smoked-glass panels get a soft
   specular highlight that slides across them as you scroll or drag, and
   follows the stylus a little while it's down, as real glass would; the
   panel edges pick up a thin bright rim. One small gradient texture
   moved per frame: the cheapest effect here with the biggest "modern"
   feel.
4. **Sheets push the world back.** Opening the Start menu, a sheet or the
   keyboard, the screen behind scales back a few percent and dims, as if
   it stepped away; closing it, it comes forward again. One transform on
   the layer behind.
5. **A card-stack task switcher.** Open programs as cards standing in
   perspective, slightly overlapped, each its last picture (old Windows
   Mobile programs too); slide through them, flick one away to close it,
   tap to bring it forward full screen. Each card a quad.
6. **The home carousel** (SPB-inspired, our own take, above): the home
   panels on a ring with a glossy floor reflection, the pull-back and turn.

**More, to pick from later:**
- **The Start menu unfolding:** the classic corner Start menu opening as a
  stack of strips that swing out in sequence, then settle flat:
  Windows Mobile's own structure with modern motion.
- **Icons with depth:** a pressed icon sinks into the screen and its shadow
  tightens; launcher icons lean very slightly toward the stylus while it
  hovers near (pressure-free, by position).
- **Page turns** for the calendar's days and the notes: a curled page with
  its shading, flicked by the finger, as old paper organisers did (the
  PDA's roots).
- **Notifications with weight:** toasts that drop in on a spring with a
  soft shadow, and the notification shade pulled down as a sheet of glass
  that bends slightly at its edge as it's dragged.
- **A 3D logo at start-up:** RetroPane's mark as a real lit, turning
  object while the system starts (the GPU's first job of the day; mesh
  drawing is what the core does natively).
- **A retro screen theme:** an optional theme drawing the whole interface
  through a curved-glass "CRT" mesh with scanlines and a soft glow: the
  retro half of retro-modern taken literally, for the Classic look or as
  an extra. Recomposed only when something changes.
- **The forecast as objects:** the week's days as a short row of tiles you
  spin through, each with its weather scene inside, the hourly forecast a
  dial with the sun's arc.

**Pros:** each makes the device feel new without losing the retro
character (real objects, light and depth rather than flat boxes); all of
them run on the 3D core, leaving the CPU for programs and the battery's
sake; switchable off, so the plain look is always there.
**Cons and risks:** each is work in the compositor and needs its art; the
frame budget means two full-screen effects can't run at once (the design
keeps one moving thing at a time); sustained 3D draws power, so slow
effects run at low frame rates and nothing animates when nothing changes;
none is proven until the GPU facts batch measures textured, blended fill
and each is tried on the X51v.

## The wallpaper when the card is gone (ADR-0018 edge case)

- The base system (flash) always has one built-in wallpaper of its own,
  plus a gradient drawn in code (no bytes stored) as the last resort, so
  glass over nothing never happens.
- A wallpaper the user picks from a card is converted once (to the
  screen's size and colour depth, compressed) into the user's storage in
  flash, so pulling the card changes nothing on screen. Card space is
  used for the originals and packs, not for what the screen needs now.

## Third-party art ships as a pack, never in the base

Licensed third-party content (an artist's icon set, if the artist agrees; the
two exceptions below)
ships as a separate, signed, optional pack (ADR-0018), never in the base
image. The base keeps its own complete look (original icons in this
style), so a "no", a withdrawn permission or a licence change removes
one pack and breaks nothing. The pack carries its licence and the
permission text inside it. Permission asked for in writing must name
the scope: distributing the icons in a free pack alongside the OS, and
converting them for the device (resizing, cropping, colour conversion).
The CC0 / public-domain wallpapers need no permission.

**Exception (owner, 2026-10-17): the default weather icons** by VClouds
are in the base system, not a pack: he gave permission for RetroPane with
credit (deviantart.com/vclouds, in the About screen and beside the files),
on his terms, kept apart from the code's licence, for non-commercial use.
The cost is the same as the card icons': if he withdrew permission or the
terms changed, the next release would swap the default for one of the
owner's own packs, and anyone selling RetroPane or a device with it asks him
first ([assets/weather/vclouds/README.md](../../assets/weather/vclouds/README.md)).

**Exception (owner, 2026-10-14): the SD and CF card icons** by Juanje
(puntodepartida.com) are in the base system, not a pack: the author gave
written permission for RetroPane with credit, and they are two small
images. Their record (the permission, what it doesn't cover, the credit,
the fallback set) is `assets/cards/README.md`.

## Wallpaper provenance (checked 2026-10-12)

Each file's source, checked through Wikimedia Commons' own records, not
the label alone. Nothing ships until its row says **ship**.

| Wallpaper | Author and original source | Basis | Status |
|---|---|---|---|
| Mountain lake | N. Lowe, U.S. Forest Service Intermountain Region (Flickr), 2016 | Work of a U.S. federal employee: public domain (`PD US USDA FS`); Commons' Flickr licence review passed | **ship** |
| City at night | Tabitha Mort, Pexels photo 432361, 2014 (Portland) | CC0 as published on Pexels at the time; Commons category "reviewed licenses" | **ship** once the Pexels page date is archived with the pack |
| Pier, black and white | Christopher Sardegna, Unsplash `uDUiRS8YroY`, 2013 | CC0 on Unsplash in 2013. **Checked 2026-10-17:** the Internet Archive holds copies of its Unsplash page from 2016-10-20 and 2017-05-04, both before Unsplash's June 2017 licence change, so the photo was published under CC0 (Commons' own Wayback copy is from July 2017, which alone would not prove it) | **ship** (keep the two archive dates with the pack) |
| Aurora | The Commons file credited to Rich and Rose Primmer via a wallpaper site | Rests on that site's claim, not a government source | **dropped 2026-10-17**, replaced by the NPS aurora photos below (federal employees' work, public domain) |

**Approved by the owner (2026-10-17), from 47 candidates, then cut to ten (2026-10-17, owner):**
numbers are those of the picture sheets. Everything not fully licensed was
dropped (#13, imported from 500px with an unconfirmed CC0 label) and then the
weakest, until ten were left (#10, #16, #44 and #46: a saturated green forest,
a flat fog, dull storm clouds, a vivid orange sunset). The pier and the lake
were not picked either, though the lake photo still ships as the built-in
wallpaper candidate. Checked through Commons' own records:

| # | Commons file | Author and basis | Status |
|---|---|---|---|
| 4 | `File:Aurora over the boreal forest (57be20e1-a3cf-453e-9632-2d38df39b643).jpg` | NPS Photo / Tim Rains: public domain (federal agency work) | **approved** |
| 5 | `File:The Milky Way spans the night sky above a bright orange-yellow airglow (iss073e0982679).jpg` | NASA Johnson Space Center: public domain (federal agency work) | **approved** |
| 7 | `File:Morning light on Electric Peak.jpg` | Jacob W. Frank: public domain (federal agency work) | **approved** |
| 18 | `File:Crooked River and Chimney Rock at Sunset (25903316784).jpg` | Bureau of Land Management Oregon and Washingto: public domain (federal agency work) | **approved** |
| 23 | `File:San Rafael Swell and Canyon - Wedgepanel (27079938169).jpg` | BLMUtah: public domain (federal agency work) | **approved** |
| 29 | `File:McDonald Lake, Glacier National Park 19.jpg` | Ansel Adams: public domain (federal agency work) | **approved** (Ansel Adams, National Park Service mural project) |
| 37 | `File:Morning Reflections at Lake McDonald (18750990873).jpg` | GlacierNPS: public domain (federal agency work) | **approved** |
| 39 | `File:Two Medicine Lake Reflections (20499664708).jpg` | GlacierNPS: public domain (federal agency work) | **approved** |
| 42 | `File:Sunset reflection on Lower Twin Lake (46171178422).jpg` | BLM Oregon & Washington: public domain (federal agency work) | **approved** |
| 45 | `File:The Delong Mountains are reflected in the clear, clean water of Desperation Lake in the northern part of Noatak National (936b4940-1dd8-b71c-070e-3b979ec0a573).jpg` | NPS Photo: public domain (federal agency work) | **approved** |

That is the ten (goal met). Each is cropped to 3:4 from an original at least 1600
pixels wide, so none looks soft. The owner's taste, from three reference pictures:
photographic and cinematic, real depth and leading lines, dramatic skies, warm earth
tones against dark reflective water, black and white, soft bokeh; less saturated than
the first batch. Note: #5 is a NASA space photograph and #29 is Ansel Adams for the
National Park Service (National Archives); the rest are National Park Service, Bureau of
Land Management and similar federal photographs.

Each pack carries this table for its files (author, source link, licence
basis, date checked) as its provenance note.

Other third-party art in the base system:

| Art | Author and source | Basis | Status |
|---|---|---|---|
| SD and CF card icons (`assets/cards/`) | Juanje, puntodepartida.com ("Memory cards") | The author's written permission, 2026-10-14: use in RetroPane with credit; selling not covered | **ship** (credit him) |

## The cached desktop is pinned

(Written for the single-surface plan. Not built: nothing is pinned yet,
and neither this 600 KB nor item 60's 1.2 MB ARGB8888 interface layer is
in docs/memory-budget.md. Under the two-plane design the wallpaper lives
in the 2700G's own memory, outside system RAM, and the candidate to pin
is the interface layer, read on every flush; to settle with the shell's
budget, ADR-0021.) The 600 KB composed desktop is what's on screen, so it must never be aged,
compressed or evicted: it lives in a **pinned** region (`Prot::pinned()`,
loaded up front by `Vm::populate`). That's above the 64 KiB default pin
limit, so the shell is one of the trusted services whose limit the kernel
raises (`set_pin_limit`), and the 600 KB is counted in the shell's
allotment in docs/memory-budget.md.
