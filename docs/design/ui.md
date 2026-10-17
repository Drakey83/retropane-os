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
  jobs Windows Mobile gave them (owner's rule).
- **Always visible, like a phone:** time, battery and Wi-Fi, at least in a
  status bar on every screen, apps included.
- **Today screen:** a good weather display with the clock and calendar by
  default, fully customisable (add, remove, reorder widgets) if the owner
  doesn't like the defaults.
- **Visual style: not chosen yet.** Three looks of the same Today screen
  were mocked for comparison: A clean flat, dark-first; B glassy accents;
  C retro-modern (concept canvas, private to the owner).

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
- Icons: the owner wants an existing artist's realistic set under a free
  licence rather than generated ones. Licence check: the set the owner liked best is CC BY-NC-SA 3.0, non-commercial, which a GPL OS can't ship without the artist's permission; KDE's Oxygen weather icons are LGPL-3
  (shippable with attribution and their SVG sources); MerlinTheRed's plain
  set is CC BY-SA (a plainer style); the Xfce "liquid" set's licence is
  unclear (avoid).

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
- **Wallpapers.** We ship a few that suit glass: dramatic, moody photography with dark areas where the panels sit, such as a black-and-white pier, a mountain lake, a city at night or an aurora. Users can pick their own picture from SD or CF. Settings will offer glass darkness and dimming the wallpaper at night. Any wallpaper we ship must be our own or CC0/public-domain and recorded in the licence list.
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

**Goal (owner, 2026-10-13): at least 10 approved wallpapers ship.** The four
above are approved; six more are to be found under the same rule (our own,
CC0 or public domain, recorded with their Commons file and licence here),
suited to glass (dark areas where the panels sit), and put to the owner.
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

Licensed third-party content (an artist's icon set, if the artist agrees)
ships as a separate, signed, optional pack (ADR-0018), never in the base
image. The base keeps its own complete look (original icons in this
style), so a "no", a withdrawn permission or a licence change removes
one pack and breaks nothing. The pack carries its licence and the
permission text inside it. Permission asked for in writing must name
the scope: distributing the icons in a free pack alongside the OS, and
converting them for the device (resizing, cropping, colour conversion).
The CC0 / public-domain wallpapers need no permission.

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

**Proposed to reach ten (2026-10-17, checked through Commons' own records, for the
owner's approval; picture sheet: [wallpaper-cands.jpg](wallpaper-cands.jpg)):**

| Wallpaper | Commons file | Author and basis | Status |
|---|---|---|---|
| Aurora over Denali | `File:Aurora in Denali (98bfa86a-4e3a-4cec-b641-9dccd7377fab).jpg` | NPS Photo / Tim Rains: federal employee's work, public domain | proposed |
| Aurora over the boreal forest | `File:Aurora over the boreal forest (57be20e1-a3cf-453e-9632-2d38df39b643).jpg` | NPS Photo / Tim Rains: public domain | proposed |
| Milky Way over Earth's glow | `File:The Milky Way spans the night sky above a bright orange-yellow airglow (iss073e0982679).jpg` | NASA Johnson Space Center: public domain | proposed |
| Desert sky at dusk | `File:Desert sky scene at dusk.jpg` | Steve Hillebrand, U.S. Fish and Wildlife Service: public domain | proposed |
| Electric Peak, morning light | `File:Morning light on Electric Peak.jpg` | Jacob W. Frank, National Park Service: public domain | proposed |
| Sunset thunderstorm | `File:Sunset Thunderstorm (53721202248).jpg` | Bob Hyatt, U.S. National Weather Service: public domain | proposed |
| Snowy dunes at alpenglow | `File:Alpenglow on Snowy Star Dune and Crestone Peaks (31601451293).jpg` | Great Sand Dunes National Park and Preserve (NPS): public domain | proposed |
| Forest sunrise | `File:Forest sunrise.jpg` | Peter Heeling: CC0, as the uploader declares | proposed (**verify** the source page) |
| Quebec City at night | `File:Chateau Frontenac illuminated at night in Quebec City.jpg` | Wilfredor: CC0 own work, as the uploader declares | proposed, an alternative to the Portland city photo |

With the lake, the pier and the city photo this makes at least ten.
The other rows work only as a pack's wallpapers: each is cropped to 3:4 from
the original, which is 3000 pixels or more wide, so none looks soft.

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
