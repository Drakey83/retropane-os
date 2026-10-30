# Icon set brief: for an artist

Part of the design area: [README.md](README.md). Written 2026-10-30 for
the owner to send to artists (R126). Claude drafts; the owner sends.

## The project, in two lines

RetroPane is a free, open-source operating system (GPL-2.0-or-later) for
the Dell Axim X51v, a 2005 handheld: 480 x 640 pixels on a 3.7-inch
screen. Its Classic desktop puts free icons on the wallpaper under a
weather widget, as on a Windows XP or Vista desktop
([mockups/classic-desktop-sheet.jpg](mockups/classic-desktop-sheet.jpg)).

## The look we want

The glass-and-light style of the mid-2000s (what people now call
Frutiger Aero), drawn fresh: **your own drawings, never traced from or
based on Microsoft's or anyone else's icons.**

- Real objects in three-quarter view, light from the top left, soft
  reflections and a gentle shine.
- **Folders as teal-green glass,** open at the front like a book, with
  what they hold standing inside (music, film, pictures, documents).
- **Small badges** laid over a base icon (a check, an arrow, a lock, a
  magnifier, a plus, a warning) instead of a new drawing for each case.
- One colour family: teal, aqua, a little green, with warm accents
  (orange, red) only where an object calls for it.
- Readable at 48 pixels on a small screen: strong silhouettes, nothing
  that turns to mush when small.

## The full set (owner, 2026-10-30: "make sure we have an entire set of icons that will cover everything we will need")

**Ruled (owner, 2026-10-30): these drafts ship as the stand-in base set** until he finds a set he likes better with the right licence; custom icons and packs: [icons.md](icons.md).

Every icon the OS draws, grouped. **Claude has drafted all of them**
(2026-10-30, in six batches by group: sources `tools/mockups/icon_drafts.js`
for the shared glass parts and the first twelve, then
`icon_drafts_1_programs.js` to `icon_drafts_6_status.js`; mock-ups
`mockups/icon-drafts-<group>.jpg`, rendered by
`local/mockups/icons/shoot-icons.py`). Owner verdicts so far: the
direction is liked; the Calendar was redrawn without its see-through
stand, "Deleted Items" is called the **Trash can**, and Programs gets
other takes (`icon-drafts-programs-alt.jpg`: windows, folder, box; the
tile grid was not liked); asked for "a new one", Claude drew a cube, a rocket and a toolbox (`icon-drafts-programs-new.jpg`). **Ruled (owner, 2026-10-30): Programs is the folder take** ("lets use Program folder"). Not loved: Themes & desktop (the palette), Storage (the stacked drums) and the oval in Sound's speaker; Sound's oval is removed and three new takes each for the other two are on `icon-drafts-settings-redo.jpg`. **Ruled (owner, 2026-10-30): Storage is the card with a usage bar** (blue end squared off flush with the green, his note); Sound as redrawn is liked. Themes & desktop: none of the palette, swatches, roller or colour wheel liked; a new round (wallpaper swap, prism, paint drops, paint can) on `icon-drafts-themes-new.jpg`; **ruled (owner, 2026-10-30): the wallpaper swap**. Packs: the owner likes the add-on idea but not the pieces; three takes with generated jigsaw pieces (two pieces, piece and plus, a set) on `icon-drafts-packs.jpg`; **ruled (owner, 2026-10-30): the set**. A drawing check, `tools/mockups/icon_audit.py`, runs before every sheet render (the owner found keyboard keys falling off the keyboard): it flags anything past the canvas, off centre, NaN, or an id two icons define differently; parts placed inside another use `place()` (by centre), not hand-worked corner offsets. **Glass recipe 2 (owner, 2026-10-30: tried on three icons, then "Roll it out")**: computed bevels lit from the top left, inner shading for glass thickness, light pooled inside glass and a soft drop shadow, in the shared parts (`FX_DEFS` in `icon_drafts.js`, the last pass in `icon_drafts_zz_finish.js`); status-bar icons stay crisp without shadows, and Juanje's cards are left as he made them. The effects are worked out when the icons are made into images on the PC, so they cost the device nothing. Sources of
the list: the Start menu and quick buttons (ui.md), every WM5 default app
(apps-and-packs.md), File Explorer's folders and file types, Settings'
groups, the status bar and the shade.

| Group | Icons |
|---|---|
| Programs (apps-and-packs.md) | Today/home, Calendar, Contacts, Tasks, Notes, Inbox, Browser, Documents (Word), Spreadsheet (Excel), Presentation (PowerPoint), Media player, Pictures & Videos, File Explorer, Calculator, Solitaire, Bubble Breaker, Games folder, Clock & Alarms, Weather, Reader, Record (voice), Search, Help, LegacySync, Remote desktop, Programs (the folder take, ruled), Settings |
| Device and places | My Device, SD card and CF card (Juanje's, in the base: not redrawn), Flash storage (a flash chip in Juanje's card style, the owner's reference and ask; redrawn on his rules, owner: "much better") and RAM (a memory stick): two different things, the owner 2026-10-30, Network (Wi-Fi), Bluetooth, USB/PC connection, Trash can empty and full, Desktop, Start (the button's mark: three candidates for R127) |
| Folders (teal glass) | Folder, open folder, Documents, Pictures, Music, Video, Downloads, Games, Programs, Shared/network, Favorites, Recent |
| File types | Text, Document, Spreadsheet, Presentation, PDF, Picture, Music, Video, Archive (zip), Program (.exe), Install package (.cab), Font, Web page, Contact card (vCard), Calendar file (iCal), Note, Unknown |
| Settings (each applet) | Display and brightness, Sound, Buttons, Power and battery, Memory (RAM: the stick), Storage (card and bar, ruled), Clock/date and region, Owner info, Themes and desktop (wallpaper swap, ruled), Light/dark mode, Touch alignment, Keyboard and input, Wireless (Wi-Fi, Bluetooth), Security and keys, Sources and accounts, Packs (the set, ruled), About, Updates |
| Status and shade | Battery (levels, charging), Wi-Fi strength, Bluetooth on, Volume (levels, mute), Alarm set, Notification, Sync running, Card inserted, Lock |
| Badges (overlays) | Shortcut arrow, Check, Lock, Magnifier, Plus, Minus, Warning, Error, Info, Sync, Download, Star |
| Weather | VClouds' pack (the base); the five optional packs |

About 110 icons in all, all drafted (plus the status variants: four battery levels and charging, three Wi-Fi strengths and off, two volume levels and mute). Masters at **256 x 256** with transparency (PNG, or SVG/PSD sources if
you work that way); we make every smaller size and the light and dark
variants on the PC, and send them back to you for approval.

## What we ask, and what we give

- **The licence:** either **CC BY 4.0** (anyone may use them with credit
  to you), or written permission that names the scope: shipping with
  RetroPane and in its optional packs, and converting them (resizing,
  cropping, colour depth). Not NonCommercial or NoDerivatives, since the
  OS's licence lets anyone share or sell it.
- **Credit:** your name and link on the About screen and beside the
  files, as for VClouds, whose weather icons RetroPane ships.
- **Terms of payment:** the owner's to set with you.
- **A first step:** three icons (Calendar, a Music folder, My Device) to
  see whether the style fits before the whole set.
