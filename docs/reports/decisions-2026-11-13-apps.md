# Notes for the owner's morning test, 2026-11-13 (core apps, batch 1)

From the core-apps order of 2026-11-13 (branch `fix/core-apps-2026-11-13`,
report [core-apps-2026-11-13.md](core-apps-2026-11-13.md)). **Nothing here
waits:** every item below is decided, built and proven in the emulator.
Each line names what was built and the alternative that was weighed, so a
"change it" at the morning test is one sentence. The reasons are in each
app's Rule 0 sheet (linked); they aren't repeated here.

How to read a line: **built** / the alternative.

## The whole batch

- **All five apps are in the Start menu itself** (Today, Calendar,
  Contacts, Tasks, Notes, Calculator, then the rest) / Tasks, Notes and
  the Calculator under Programs, as Windows Mobile 5 had them. The order
  asks for each app "launchable from the Start menu", and Programs is not
  built yet (batch 2 onwards).
- **The Start menu's rows are 33 px** (were 38 with ten entries) / fewer
  entries at 38 px. With the test image's four extra entries the menu has
  sixteen rows; at 33 px it fits above the softkeys and its picture, open
  and closed, still fits the renderer's area cache (the 18 ms Start menu
  of 2026-11-12). The shipped menu (twelve rows) has room for 38 px again
  once the test entries go.
- **Today's left softkey opens the Calendar, and Today's Note button
  starts a new note at once** / leaving both as placeholders.
- **The records live on the SD card only**, in two fixed files
  (`RPPIM/A.RPP`, `B.RPP`, 1 MiB each, made on a card's first run; see
  [pim-store.md](../design/pim-store.md)) / the CF card, or both. With no
  SD card the apps work and keep their records until the image leaves.
- **A reminder is a banner over whatever screen is up** (the notification
  centre's first piece: the appointment's name and time, Dismiss, Open),
  with no sound / a sound or a blinking light. Sound waits for the audio
  work's volume and mute rules; the banner stays until it is answered.
- **Deleting asks in place:** the Delete row reads "Tap again to delete",
  a note's Delete softkey reads "Delete?" / a pop-up box.
- **How the apps were made to fit the image** (this one is engineering,
  not look; say if you want the other way): the five apps and their store
  are compiled as Thumb code, and rp-today's zeroed RAM and stack moved
  from the image area to the free RAM above it (`0xA8100000`). Loaded
  size 909,936 B of the 921,600 budget (was 820,920 before the apps);
  zeroed RAM 82,800 B (budget raised from 80 to 84 KiB for the store's
  state). The alternative was compiling older cold code (settings
  screens) as Thumb to make room; it would have touched proven code the
  night before your test.

## Calendar ([sheet](../design/calendar-rule0.md))

- **Agenda is the first view** / Day or Month first.
- **A weekday name means the coming one, today included** ("fri" typed on
  a Friday is today) / always the next one.
- **New appointments get a 15-minute reminder and last an hour** unless
  the line says otherwise / no default reminder; 30 minutes.
- **The week starts on Monday** / Sunday (a setting later; the US default
  may want Sunday).
- **Dates typed with a slash read month first** (11/14) / day first (it
  follows the region once regions are a setting).
- **New opens the typed quick-add line** ("Dentist tue 3pm 1h"), with
  "More details" for the full editor / New opens the editor.

## Contacts ([sheet](../design/contacts-rule0.md))

- **Sorted by last name, shown "First Last"** / shown "Last, First" as
  Windows Mobile did.
- **Search matches initials and phone digits** as well as text / text
  only.
- **Three phone rows** (Mobile, Home, Work) / a type picker per number.
- **The keyboard's Done goes to the next field; the softkey Done keeps
  the card** / the keyboard's Done keeps the card.

## Tasks ([sheet](../design/tasks-rule0.md))

- **No sort menu:** late first, then by due date, done at the bottom /
  a sort softkey.
- **Done tasks stay at the bottom, dimmed, until Hide done** / done tasks
  leave at once.
- **`!` in the typed line means high priority** / a priority picker in
  the add line.
- **Due dates read as words** ("Tomorrow", "Late 2 days") / dates.

## Notes ([sheet](../design/notes-rule0.md))

- **There is no Cancel in a note:** leaving keeps it, Delete removes it /
  Cancel and Done like the other editors.
- **The keyboard's Done key reads RETURN in a note** / Done closes the
  keyboard.
- **A note holds 2,000 bytes** / longer notes (needs the store's larger
  group; on the later list).

## Calculator ([sheet](../design/calculator-rule0.md))

- **`2 + 3 x 4` is 14** (the order school teaches) / 20, as Windows
  Mobile's calculator gave.
- **`=` twice does nothing** / it repeats the last operation.
- **No memory keys in v1** (WM5 had MC, MR, M+) / memory keys now; they
  are on the later list.
- **Percent:** `200 + 10 %` shows 20, then `=` gives 220 / percent as a
  plain divide by 100.
- **History:** the sum just worked stays above its answer until the next
  key; there is no tape / a scrolling tape (later list).

## What only the device can show (the morning test)

The emulator proves the pictures and the card; it cannot prove these, so
they are the things to look at on the X51v:

- Whether the keyboard's keys and the 17 px letter strip are comfortable
  with the stylus on the real panel.
- How long a save takes on the real SD card (the run log's last
  `organiser:` line gives the slowest sector write; the card log's writes
  have taken 2 to 98 ms), and that typing never stutters because of it.
- The first run on your card makes the two files (2 MiB of zeros): the
  start-up is that much longer once, and the log's first `organiser:`
  line gives the time.
- The glass and the type on the real panel, light and dark.
