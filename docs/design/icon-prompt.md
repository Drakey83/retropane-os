# Icon pack prompt for other AIs

Part of the design area: [README.md](README.md). Written 2026-10-30 at the
owner's request ("give me a good prompt to give to some other AI's that
convey the look i am going for ... I want an entire icon pack"), to compare
other AIs' sets with Claude's drafts (`icon-brief.md`, `mockups/icon-drafts-*.jpg`).
The prompt below is meant to be pasted as it is; it leaves the metaphors
open on purpose so each AI brings its own ideas.

---

## The prompt

I want a complete, original icon pack for a handheld operating system: about
110 icons that look like one family. Please design every icon in the list
below.

**The look: mid-2000s glass, drawn fresh (what people now call Frutiger
Aero; think of the feel of Windows Vista and the KDE Crystal era, but do not
copy any existing icon).**

- Real objects, slightly three-quarter or front-on, lit from the top left.
- Glossy and glassy: see-through tinted glass bodies where it suits the
  object, a thin crisp white inner rim along the edges, a strong soft shine
  across the top third, a faint glow along the bottom edge, a soft oval
  shadow beneath.
- Thin, darker outlines in the object's own colour (dark teal for teal glass,
  navy for blue), never flat black cartoon outlines.
- One colour family: teal and aqua glass first, then sky blue, a little
  fresh green; warm accents (orange, red, gold) only where the object calls
  for them (a red calendar header, an orange play button, a gold star).
- **Folders are teal-green glass**, open at the front like a book, and what
  they hold stands inside them (sheets of paper, a music note, a film
  clapper, a gamepad). What a folder is *for* goes on its front as a badge
  (a download arrow, a star, a clock).
- Badges are small glossy discs or shapes laid over a base icon (bottom left
  for a shortcut arrow, bottom right for states such as check, lock, sync).
- Clean and crisp, like a polished vector illustration with gentle
  gradients: no photographic noise, no grunge, no heavy texture.
- **Readable small:** every icon must still read at 32 x 32 and 48 x 48 px
  on a 3.7-inch screen. Strong simple silhouettes; no fine detail that turns
  to mush.
- Works on both a dark photo wallpaper and a light grey background.

**Format.**

- Each icon centred on a square 256 x 256 canvas with a transparent
  background and about 16 px of empty margin all round; nothing may poke
  past the canvas or off the object it belongs to.
- Every part sits on what it belongs to: keys inside the keyboard, a person
  inside their badge circle, contents inside their folder.
- No words on icons, except where the object itself carries them (a date on
  a calendar page, "PDF" on a PDF file, "FLASH MEMORY" on a flash chip).
- Deliver as sheets of 12 to 20 icons per group, each icon labelled with its
  name beneath, on a mid-grey background; SVG or 256 px PNG.

**Hard rules.**

- Entirely your own drawings. Do not trace, copy or closely imitate
  Microsoft's, Apple's, KDE's or any other set.
- No company logos or trademarks: no Windows flag, no Apple, no Microsoft
  Office letters, no SD-card or USB logos, no browser brands. Use generic
  objects (a globe for the web, a page with an "A" tile for a document).
- The pack must be releasable under a free licence (CC BY 4.0), so nothing
  in it may come from someone else's art.

**The icons.**

1. **Programs:** Today (the home screen), Calendar, Contacts, Tasks, Notes,
   Inbox (mail), Browser, Documents (word processor), Spreadsheet,
   Presentation, Media player, Pictures & Videos, File Explorer, Calculator,
   Solitaire, Bubble Breaker (a game of popping coloured bubbles), Games,
   Clock & Alarms, Weather, Reader (e-books), Record (voice recorder),
   Search, Help, LegacySync (syncing with an old PC), Remote Desktop,
   Programs (the list of all apps), Settings.
2. **Device and places:** My Device (a handheld with a screen and a round
   centre button, held upright), Flash storage (the built-in flash memory
   chip) and RAM (program memory; these are two different things and must
   look different), Wi-Fi network, Bluetooth, USB / PC connection, Trash can
   (empty) and Trash can (full), Desktop, and a Start button mark (a simple
   original emblem, not a flag).
3. **Folders (teal glass):** Folder, Open folder, Documents, Pictures, Music,
   Video, Downloads, Games, Programs, Shared, Favorites, Recent.
4. **File types:** Text, Document, Spreadsheet, Presentation, PDF, Picture,
   Music, Video, Archive (zip), Program, Install package, Font, Web page,
   Contact card, Calendar file, Note, Unknown.
5. **Settings pages:** Display & brightness, Sound, Buttons (the device's
   hardware keys and 5-way pad), Power & battery, Memory, Storage, Clock &
   region, Owner info, Themes & desktop, Light & dark mode, Touch
   alignment, Keyboard & input, Wireless, Security & keys, Sources &
   accounts (online data sources and accounts), Packs (add-on packs), About,
   Updates.
6. **Status bar** (these show at 16 to 24 px, so simplest of all): Battery
   full, half, low, empty and charging; Wi-Fi 3 bars, 2, 1 and off; Volume
   high, low and mute; Notification; Alarm set; Sync running; Card inserted;
   Locked.
7. **Badges** (overlays): Shortcut arrow, Check, Lock, Magnifier, Plus,
   Minus, Warning, Error, Info, Sync, Download, Star.

Start with three icons (Calendar, the Music folder and My Device) so we can
check the style, then do the rest group by group. For each group, say in a
line or two what idea you chose for any icon that is not obvious.

---

## Notes for the owner

- **Attach images or not?** Pasted alone, the prompt lets each AI bring its
  own take, which is the comparison you asked for. Attaching our sheets
  (`mockups/icon-drafts-128.jpg`, `-folders.jpg`) would pull them toward
  Claude's drawings.
- **Image AIs** (Midjourney, ChatGPT images, Gemini, Firefly) draw best a
  sheet at a time: paste the whole prompt once, then ask for one group per
  message, and give the first sheet back as the style reference for the
  next ones so the set stays one family.
- **AIs that write code** can answer in SVG, which we can render, check
  with `tools/mockups/icon_audit.py` and resize like our own drafts.
- **Licences:** check each service's terms on who owns what it makes before
  anything from it ships (R126 rule: CC BY 4.0 or written permission).
- **Left out on purpose:** the SD and CF card icons (Juanje's, already in
  the base) and the weather condition icons (VClouds' pack).
