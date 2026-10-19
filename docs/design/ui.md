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
  long list.
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
- **Classic: a Windows Mobile 5-style look**, at stylus scale, as DESIGN.md
  §7's theme engine already planned ("a faithful WM5-style homage, using our
  own artwork").

This makes the theme engine carry **layout** as well as skin: target sizes,
the launcher's grid and the Today screen's arrangement differ between the
two looks, not only colours and art. Both looks are designed, mocked up and
tested; old Windows Mobile programs keep their own windows in either.
"Modern" and "Classic" are working names; the final names are the owner's
to choose, and never "SPB" or "Windows Mobile" (their owners' names).

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
