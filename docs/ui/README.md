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
  the VClouds set the owner pointed to. **Clean-room for artwork too:**
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
  licence rather than generated ones. Licence check: VClouds (the owner's
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
