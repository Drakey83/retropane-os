# RetroPane UI: direction (in progress)

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
  glowing suns, a cratered moon, on a dark, moody backdrop), the style of
  the the reference set the owner pointed to. **Clean-room for artwork too:**
  RetroPane draws its own original icons in that language; no third-party
  icon is copied or shipped. On the device the icons are pre-rendered into
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
  licence rather than generated ones. Licence check: the reference (the owner's
  reference) is CC BY-NC-SA 3.0, non-commercial, which a GPL OS can't ship
  without the artist's permission; KDE's Oxygen weather icons are LGPL-3
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

The owner picked the reference's Rainmeter work ( "Glasses", "the reference Weather 2") as the look to borrow heavily from: "a good mix of retro and modern".

- **Smoked glass on a wallpaper the owner chooses.** Panels are dark, translucent glass. The gradient runs from near-black on the left to a lit grey on the right. Each panel has a 1 px bright edge, an almost square corner (3 px) and a soft gloss. A thin vertical light streak sits on one edge, and there is a faint reflection under the bottom row. Everything floats over a full-screen photo.
- **Wallpapers.** We ship a few that suit glass: dramatic, moody photography with dark areas where the panels sit, such as a black-and-white pier, a mountain lake, a city at night or an aurora. Users can pick their own picture from SD or CF. Settings will offer glass darkness and dimming the wallpaper at night. Any wallpaper we ship must be our own or CC0/public-domain and recorded in the licence list.
- **Type.** Thin geometric sans, big light numerals (`18°`) and spaced capitals for labels and dates (`MON 12`, `OCTOBER`). His skins bundle Century Gothic, which is Monotype's and is never shipped. The concepts use Didact Gothic (SIL OFL) as an open stand-in.
- **Icons break out of their frames.** Weather icons are large and overhang the top edge of their glass tile.
- **The WM structure stays.** The title bar has Start at the top left and status at the right. The Start menu is a glass panel dropping from Start. Softkeys sit at the bottom.
- **Clean-room.** the reference's icons are CC BY-NC-SA 3.0, so they can't ship in a product we might sell, and they never enter this repository. They appear only in the private concept canvas (boards N, O and P), credited. The glass panels are rebuilt from plain gradients, not his images. Shipping icons will be original work in his volumetric style, unless the artist grants permission, or the Oxygen set (LGPL-3) is the fallback.

**Owner verdict (2026-10-12): "I LOVE it."** Boards N–R (smoked glass over photo wallpapers) are the chosen direction. The cartoon sky wallpaper is dropped. The candidate wallpapers we'd ship are all free of restrictions, from Wikimedia Commons, cropped to 3:4:

| Wallpaper | Commons file | Licence |
|---|---|---|
| Pier, black and white | `File:Monochrome pier with sky (Unsplash).jpg` | CC0 |
| Mountain lake | `File:Lake Blanche Reflection (31122974596).jpg` | Public domain |
| City at night | `File:Long Exposure Photography of City Buildings.jpg` | CC0 |
| Aurora | `File:Night landscape northern lights scenic aurora borealis.jpg` | Public domain |

The concepts keep the glass light enough that the photo shows through. A "glass darkness" setting covers busy photos where small grey text gets hard to read.

**Seeing a concept on the real Axim.** Render the board at 480 × 640, convert it to RGB565 (dithered) at `local/concept/picture.rgb565`, then run `cargo xtask emu-picture` (pixel-exact on the emulated screen, clean return) and `cargo xtask load rp-picture`. The Axim shows it for 20 s, then goes back to the loader. `local/` is never committed, so concept art with third-party pieces never reaches the repository. Without the file, the image shows a test gradient.
