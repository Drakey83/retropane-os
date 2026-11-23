# Calendar, Rule 0 (version 2, 2026-11-14)

Version 1 (2026-11-13) surveyed the modern calendars and then built a
2005 one: grey rows, an empty hour grid, no colour, no sync, every modern
feature under "Later". The owner rejected it on 2026-11-14. This version
builds what the survey found, under the rules gate's rule 6
([app-scope.md](app-scope.md)): the build list below is the build.
Look: [design-language.md](design-language.md) version 2. Records: the one
store ([pim-store.md](pim-store.md)) in **internal storage**. Plans it
answers to: [apps-and-packs.md](apps-and-packs.md) (the Calendar row and
§7, connected services, decided 2026-10-17: iCal link, then CalDAV, then
Google sign-in), [ui.md](ui.md) ("a PDA, proudly": the organiser first;
page turns for the calendar's days), [interaction.md](interaction.md) (the
Calendar button).

## The core job

See what is coming at a glance, add something in a few seconds, and have
the same calendar as the phone and the PC (Google, iCloud, Outlook,
Nextcloud), on a 3.7-inch VGA screen with a stylus or the D-pad, online or
not.

## What others do (findings, not the answer)

| System | What it does well | What it costs |
|---|---|---|
| Windows Mobile 5 | Agenda, Day, Week, Month, Year behind a softkey; the Calendar button; ActiveSync with Outlook | A ten-row form of 12 px drop-downs to add one appointment; Month says busy, not with what; sync only with a desktop Outlook |
| Google Calendar (Android, 2024) | Schedule view: days with coloured event chips, a month illustration; one sentence quick add; a colour per calendar; every account syncs | Needs Google's servers for the parse; dense on a small screen |
| iOS Calendar 18 | Month over the chosen day's list; Day as a timeline with blocks sized to their length and a red "now" line; CalDAV for any server | Adding is a form with wheels |
| Fantastical 3 | The DayTicker (a strip of days you slide); the parse shown as you type with the words it understood highlighted; weather on the days | Busy when a day is full; a subscription |
| Palm Datebook | Tap an hour line and write | Little overview |
| Lean Linux (calcurse, khal + vdirsyncer) | khal shows CalDAV calendars offline, synced by vdirsyncer on a timer | Text only; setup by hand |

## Borrow and beat

| Problem | Borrowed | How ours goes further on the X51v |
|---|---|---|
| "What's next?" | Google's Schedule and Fantastical's DayTicker | The **large title** says "Saturday 14 / 3 today, next 3:00 PM Dentist"; a **week strip** under it slides by the pen; the agenda's days carry **the Weather app's icon and high/low** for days the forecast covers (Fantastical does this from a server; ours uses the forecast the device already holds, no extra fetch) |
| Telling things apart | Google's colour per calendar | A 4 px **colour bar** and tinted block per calendar, and the calendar's name in the editor (colour never alone) |
| A day's shape | iOS's timeline | **Blocks sized to their length** in the calendar's tint, overlaps side by side, a coral **now line**, all-day items in a strip on top; the empty evening folds into one "Free after 6 PM" row instead of empty hour lines |
| Month | iOS's month over the day | Each day shows up to three **coloured bars** (not dots: a bar's colour says which calendar), and a tap **zooms** the month into that day (the 2700G scales the month's picture: one repoint a frame) and back out |
| Week | WM's week view; Google's 7-column week | Portrait 7-column week with tinted blocks and the times on the left; tap a block opens it, tap a day's header zooms into it |
| Adding | Fantastical's highlighted parse; Palm's tap-an-hour | One line ("Dentist tue 3pm 1h every 6 months") with **the understood words highlighted in the tint** as you type and a preview card; tapping an empty time in Day or Week starts the line at that time. Parsed on the device, offline |
| Details | iOS's grouped form | Grouped cards: what/where (Where uses **address suggestions**), when (steppers, all-day switch, time zone), repeat, alerts (several), calendar (colour chips), notes; field-aware keyboard (capitals at each word in a title) |
| Reminders | WM's modal box | A banner over whatever is up (the notification centre, [notifications.md](notifications.md)), **with a sound** through the audio service's notification volume and **Snooze 5 / 15 min**; never steals the keys |
| Same calendar everywhere | ActiveSync's Outlook sync; vdirsyncer | §7's order: **an iCal link** (read; Google, Outlook, iCloud and Nextcloud all give one), **CalDAV** (read and write; iCloud, Fastmail, Nextcloud, Google), **Google sign-in** by a code shown on the device (OAuth device flow, read and write). Synced on a timer and on opening, over **any connection** (Wi-Fi, or the dock's PC relay now); the local copy always opens instantly; offline changes queue with a "not yet synced" mark |
| The PDA's own touches | WM's Calendar button; paper diaries | The **Calendar button** cycles Agenda, Day, Week, Month as on WM; days turn like pages with the pen (ui.md) |

## The design

- **Home (Agenda):** large title (weekday and date in light 36 px, the
  summary under it), the week strip (seven 64 px day cells, today ringed,
  days with events marked by a colour tick), then the agenda: a day label
  ("TODAY", "TOMORROW", "MON 16 NOV") with the weather icon and high/low
  on its right, and a row per event: colour bar, start and end times
  stacked in secondary text, title, place with a pin mark. Softkeys
  **Today** and **New**; a tint chip row Agenda / Day / Week / Month.
- **Day:** the week strip, the all-day strip, then the timeline from the
  first event's hour (or 8 AM) with blocks sized to their length; free
  stretches over two hours fold into one row; the now line in the tint.
- **Week:** seven columns of tinted blocks, hours on the left.
- **Month:** a 7 x 6 grid with up to three colour bars a day and "+2" past
  three; the chosen day's events under the grid; tap the chosen day zooms
  into Day.
- **New:** a sheet rising over the dimmed screen: the line, the preview
  card (title, the day and time, the calendar's colour, repeat and alert
  marks), Add; "More" opens the full editor.
- **Editor:** grouped cards as in the table above; Delete asks in place.
- **Accounts** (Settings, Connections, shared with every app): add an
  iCal link (pasted, or sent From PC), a CalDAV server, or Google (a code
  to type at google.com/device on any phone or PC); each calendar's colour
  and on/off.

- **Birthdays as built (2026-11-23):** the contacts' birthdays and
  anniversaries worked out from Contacts each time (`rp_pim::birthdays`, no
  records copied): "Sam Lee's 41st birthday", all day, in the Birthdays
  calendar (the eighth colour, made the first time a contact has one, shown
  and hidden as any calendar, never offered in the editor's chips).
- **Today's panel as built:** T1's layout fed from the store: what is left
  of today (all day first), tomorrow's in the room it leaves, "+2" for the
  rest; hidden calendars stay off it; a tap opens Calendar. Worked once per
  change of the store or minute, not per frame (`shell/core/src/today_feed.rs`).

## Build list

| Item | From | Status | Code |
|---|---|---|---|
| Agenda with colour bars, times, places and the large title with a live summary | sheet | built | apps/calendar/src/lib.rs |
| Week strip that slides under the title | sheet | built | apps/calendar/src/motion.rs, apps/calendar/src/lib.rs, apps/calendar/src/tests.rs, shell/core/src/lib.rs |
| The forecast's icon and high/low on the agenda's days | apps-and-packs: weather on the agenda's days | built | apps/calendar/src/lib.rs, shell/core/src/lib.rs |
| Day view: blocks sized to their length, overlaps side by side, now line, all-day strip, free time folded | sheet | built | apps/calendar/src/lib.rs |
| Week view (portrait, seven columns) | apps-and-packs: week view | built | apps/calendar/src/lib.rs |
| Month view with coloured bars and the chosen day's list | sheet | built | apps/calendar/src/lib.rs |
| Smooth zoom between month, week and day | apps-and-packs: Smooth zoom between views | built | apps/calendar/src/motion.rs, apps/calendar/src/lib.rs, drivers/draw/src/lib.rs, drivers/mbx-render/src/lib.rs, drivers/mbx-render/src/model.rs, xtask/src/motion_cost.rs |
| Quick add with the understood words highlighted and a preview card | apps-and-packs: quick add | built | apps/calendar/src/lib.rs, shell/pim/src/quick.rs |
| Editor as grouped cards; address suggestions for Where; capitals at each word in titles | sheet | built | apps/calendar/src/lib.rs, apps/calendar/src/tests.rs |
| Several calendars, a colour each, shown and hidden | apps-and-packs: a colour per calendar | built | apps/calendar/src/lib.rs, apps/calendar/src/tests.rs |
| Repeating appointments (daily, weekly on days, monthly by date or weekday, yearly; every N; until or N times; one occurrence changed or skipped) | apps-and-packs: repeating appointments | built | shell/pim/src/repeat.rs, apps/calendar/src/lib.rs |
| Reminders as a banner over any screen, several per appointment | apps-and-packs: reminders through the notification centre | built | shell/pim/src/remind.rs, shell/core/src/apps/mod.rs, shell/core/src/lib.rs, apps/calendar/src/lib.rs, apps/calendar/src/tests.rs, shell/core/src/tests.rs |
| Reminder sound and snooze | apps-and-packs: reminder sound and snooze | built | services/audio/src/chime.rs, shell/today/src/main.rs, shell/core/src/apps/mod.rs |
| Online calendars: iCal link (read), CalDAV (read and write), Google sign-in (device code, read and write), over any connection, offline queue | apps-and-packs: Google Calendar and other online calendars (iCal link, CalDAV, Google sign-in: §7) | blocked: owner 2026-11-15 sign off without it: CalDAV, CardDAV and Google sign-in need an app registration and accounts only the owner can create, and a server to test against (the iCal link, read only, is built) | |
| Birthdays from Contacts as a calendar | apps-and-packs: birthdays from Contacts | built | shell/pim/src/birthdays.rs, apps/calendar/src/lib.rs, apps/calendar/src/tests.rs |
| Tasks on their due days in the agenda | sheet | built | apps/calendar/src/lib.rs, apps/calendar/src/tests.rs |
| Today screen's calendar panel fed from the store | sheet | built | shell/core/src/today_feed.rs, shell/core/src/lib.rs, shell/core/src/tests.rs |
| Search in Calendar | sheet | built | apps/calendar/src/lib.rs, apps/calendar/src/tests.rs |
| .ics import and export (a file from the PC or the card) | sheet | built | shell/pim/src/ics.rs, apps/calendar/src/files.rs, shell/today/src/cardfiles.rs, shell/today/src/main.rs, xtask/src/usb.rs, apps/calendar/src/tests.rs |
| Time zones: events kept in UTC with their zone; shown in the device's zone | sheet | built | shell/pim/src/zone.rs, shell/pim/src/zone_table.rs, tools/zones/gen.py, shell/pim/src/ics.rs, apps/calendar/src/files.rs, apps/calendar/src/lib.rs |
| Week starts on the region's day (Sunday in the US) | sheet | built | apps/calendar/src/lib.rs, apps/calendar/src/tests.rs |
| The Calendar button cycles the views | sheet | built | shell/core/src/lib.rs, apps/calendar/src/lib.rs, apps/calendar/src/tests.rs |
| The light look: the app's own light palette (its content is drawn after the shell's look pass, so rounded shapes keep one colour edge to edge) | sheet | built | shell/core/src/lib.rs, apps/api/src/kit.rs, tools/shell-assets/accents.py |
| Chips and cards as rounded shapes of one colour, no glass art inside (the owner, 2026-11-14: "a box on top of the button") | owner 2026-11-14 | built | apps/api/src/kit.rs, shell/core/src/panel_art.rs |
| Its own program, loaded when opened and released when left | owner 2026-11-14 | built | apps/calendar/src/main.rs, shell/today/src/program.rs |
| Records in internal storage (the card only as the test stage's stand-in) | owner 2026-11-14 | blocked: owner 2026-11-15 sign off without it: internal storage waits on the flash driver; the card stands in, same code and format | |

## Motion as built (group E, 2026-11-23; the owner's picks: strip B, zoom D)

Mock-ups: [mockups/e-week-strip.jpg](mockups/e-week-strip.jpg),
[mockups/e-zoom.jpg](mockups/e-zoom.jpg) (the choices; B and D drawn with the scaler's
measured arithmetic, `tools/mockups/zoom_scaled.py`), and D as built by the real renderer path, [mockups/e-zoom-built.jpg](mockups/e-zoom-built.jpg).

- **The week strip** follows the pen by the pixel (after 8 px; a tap stays
  a tap) and, released, pages a week on a quarter of its width or a fling
  (12 px between two touch samples), else goes back; the D-pad across a
  week slides it in from that side. While it moves the arriving week's
  dates ride on a blue glass chip (pick B). Six frames on a fitted ease
  table (`motion.rs`: 1 - (1 - t)^3, each frame moving less than the last);
  parts left of the screen are placed exactly (glyphs dropped, spans cut,
  today's ring in four 10 px slices). Priced 4-6 ms a moving frame, the
  landing 33 ms (`cargo xtask motion-cost`).
- **The zoom** (pick D): the arriving view is captured into the 2D
  renderer's layer B over two frames (the screen unchanged, kept in layer
  A), then six frames grow it out of the chosen day's cell (the week's row,
  the day's column) by the engine's scale word, or shrink the day back into
  its cell over the month, and a last frame lands the captured view as the
  screen's with nothing drawn again (`rp_draw::LayerOp`, ADR-0023 rule 3).
  Month's chosen day tapped again, Week's wide column, the tabs and the
  Calendar button all zoom between levels. Priced: capture halves ~30 ms,
  moving frames 10-27 ms, landing 10 ms. A renderer without layers makes
  the change at once.
- **Found and fixed on the way:** every app reply drew the whole screen
  (~47 ms for Day); replies now go through R191's draw-list difference
  (shell `program_event`).

### Version 3 (2026-11-23): files and zones, as built

- **Time zones:** an appointment keeps the wall time of its own zone with
  the zone's tzdata name (`tag::ZONE`), RFC 5545's and Google's way, not
  bare UTC as the row first said: a weekly 9 AM meeting stays at 9 AM
  across a clock change, which a time kept in UTC gets wrong half the
  year (a time from a file in UTC is kept as UTC, its zone "UTC"). Every
  view shows it in the device's zone: the one chosen on the calendars'
  screen (a search over every zone on Earth by city), else guessed from
  the dock PC's offset, a US zone first. A new appointment is kept in the
  device's zone, so it keeps its moment if the device moves; the editor
  says where a time from elsewhere is kept ("KEPT AS 9:00 AM IN NEW
  YORK"). The zones' rules from now on: 526 names, 63 rules, 7.8 KB
  (tools/zones/gen.py from tzdata 2026e); Outlook's own zone names
  ("Pacific Standard Time") matched rule for rule from the file's
  VTIMEZONE.
- **.ics import and export:** the calendars' screen's **Import .ics**
  lists the card's (root and RetroPane folder) and the PC's
  (Documents\RetroPane, Downloads, the Desktop) calendar files together;
  the one chosen is read into a calendar of its own named after the file
  (its own colour: shown, hidden or deleted as one), an appointment
  already kept (the same title, start and zone) left out and counted.
  **Export .ics** writes every appointment, with its notes, repeat, first
  alarm and a VTIMEZONE for each zone used (Outlook needs them), to the
  card or the PC, a piece at a time; a calendar link's VTIMEZONEs now
  reach the device too.

## What would overturn it

- The owner finding the week strip or the zoom slower to use than plain
  chips on the device: then the chips lead and the strip goes.
- A day with many events being common for him: then Day opens first.
