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
| Reminders | WM's modal box | A banner over whatever is up (the notification centre), **with a sound** through the audio service's notification volume and **Snooze 5 / 15 min**; never steals the keys |
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

## Build list

| Item | From | Status | Code |
|---|---|---|---|
| Agenda with colour bars, times, places and the large title with a live summary | sheet | pending | |
| Week strip that slides under the title | sheet | pending | |
| The forecast's icon and high/low on the agenda's days | apps-and-packs: weather on the agenda's days | pending | |
| Day view: blocks sized to their length, overlaps side by side, now line, all-day strip, free time folded | sheet | pending | |
| Week view (portrait, seven columns) | apps-and-packs: week view | pending | |
| Month view with coloured bars and the chosen day's list | sheet | pending | |
| Smooth zoom between month, week and day | apps-and-packs: Smooth zoom between views | pending | |
| Quick add with the understood words highlighted and a preview card | apps-and-packs: quick add | pending | |
| Editor as grouped cards; address suggestions for Where; capitals at each word in titles | sheet | pending | |
| Several calendars, a colour each, shown and hidden | apps-and-packs: a colour per calendar | pending | |
| Repeating appointments (daily, weekly on days, monthly by date or weekday, yearly; every N; until or N times; one occurrence changed or skipped) | apps-and-packs: repeating appointments | pending | |
| Reminders as a banner over any screen, several per appointment | apps-and-packs: reminders through the notification centre | pending | |
| Reminder sound and snooze | apps-and-packs: reminder sound and snooze | pending | |
| Online calendars: iCal link (read), CalDAV (read and write), Google sign-in (device code, read and write), over any connection, offline queue | apps-and-packs: Google Calendar and other online calendars (iCal link, CalDAV, Google sign-in: §7) | pending | |
| Birthdays from Contacts as a calendar | apps-and-packs: birthdays from Contacts | pending | |
| Tasks on their due days in the agenda | sheet | pending | |
| Today screen's calendar panel fed from the store | sheet | pending | |
| Search in Calendar | sheet | pending | |
| .ics import and export (a file from the PC or the card) | sheet | pending | |
| Time zones: events kept in UTC with their zone; shown in the device's zone | sheet | pending | |
| Week starts on the region's day (Sunday in the US) | sheet | pending | |
| The Calendar button cycles the views | sheet | pending | |
| The light look: the app's own light palette (its content is drawn after the shell's look pass, so rounded shapes keep one colour edge to edge) | sheet | pending | |
| Chips and cards as rounded shapes of one colour, no glass art inside (the owner, 2026-11-14: "a box on top of the button") | owner 2026-11-14 | pending | |
| Its own program, loaded when opened and released when left | owner 2026-11-14 | pending | |
| Records in internal storage (the card only as the test stage's stand-in) | owner 2026-11-14 | pending | |

## What would overturn it

- The owner finding the week strip or the zoom slower to use than plain
  chips on the device: then the chips lead and the strip goes.
- A day with many events being common for him: then Day opens first.
