# Calendar, Rule 0

Batch 1 of [core-apps-list.md](core-apps-list.md). The look follows
[design-language.md](design-language.md); the records live in the one
store ([pim-store.md](pim-store.md)). Method: [principles.md](principles.md).
Built without a stop for the owner's go (his order of 2026-11-13); his
morning test is the review, and the notes for it are at the end.

## The core job

See what is coming, and put something new in, on a 3.7-inch screen with a
stylus or the D-pad. v1: day, agenda and month views; add and edit
appointments; reminders. No sync, no accounts, no network.

## What others do (findings, not the answer)

| System | Views | Adding | What it costs the user |
|---|---|---|---|
| Windows Mobile 5 Calendar | Agenda, Day, Week, Month, Year behind a softkey that cycles them; Month shows a triangle or block per day | A form of about ten rows, most of them drop-down pickers 12 px tall; "All Day", "Occurs", "Sensitivity" | Eight or more precise taps to add one appointment; Month says a day is busy but not with what |
| Palm OS Datebook | Day by default, the hour rows always there | Tap an hour line and write | The fastest add of its time; little overview |
| iOS Calendar | Month over a list of the chosen day; Day as a timeline | A form with wheels | Good overview; adding is a form again |
| Google Calendar (Android) | Schedule (an agenda) by default | Quick add from one typed sentence, "Lunch with Sam tomorrow 12pm" | The agenda answers "what's next" at a glance; the sentence is the fastest add anyone has shipped |
| Lean Linux (`calcurse`, `remind`) | Text agenda | One typed line per event | Fast for those who know the syntax; nothing to discover |

## Borrow and beat

| Problem | Borrowed | How ours goes further on the X51v |
|---|---|---|
| "What's next?" | Google's Schedule: the agenda is the first view | It opens on today with the days grouped under spaced-capital labels, and is built when the records change, not when the pen moves (P1) |
| Adding | Google's one-sentence quick add; Palm's tap-an-hour | **One line on the keyboard, with what it understood shown as you type** ("Tue 17 Nov, 3:00 PM - 4:00 PM") so a misread is seen before Add, not after. Tapping an hour in Day view starts that line at that hour. Parsed on the device, no network (Google's runs on a server) |
| Changing the details | The form | Rows with **steppers** (`<` value `>`, 40 px targets) for day, start, length and reminder: no drop-down, no picker wheel, nothing smaller than a stylus tip; every row also works from the D-pad |
| Month overview | iOS's month over the day's list | The chosen day's appointments are listed under the grid, so Month answers "with what?" without leaving it; dots under a date count up to three |
| Reminders | WM's modal box with a sound | A banner under the title bar on whatever screen is up, with Dismiss and Open; it never takes the screen or the keys away from what you were doing |
| Losing data | WM needed ActiveSync as its backup | The store: a pulled battery loses at most the change in flight (pim-store.md) |

## The design (v1 as built)

- **The frame:** header "CALENDAR" with a three-way chip row (Agenda,
  Day, Month); softkeys **Today** (left: jump to today) and **New**
  (right).
- **Agenda:** from today on, 60 days ahead. A section label per day that
  has something ("TODAY - FRI 13 NOV", "TOMORROW - SAT 14 NOV", then the
  date). A row per appointment: start time (or "All day") left, title,
  the place in secondary text on the right. Tap opens it. Empty: a panel
  saying "Nothing planned. New adds an appointment."
- **Day:** the date with `<` and `>`; all-day appointments as rows on top;
  then hour rows from 6 AM to 10 PM (the list scrolls; other hours appear
  when something is in them). An appointment is a panel across its hours
  with its title and times. Tapping an empty hour starts New at that hour.
- **Month:** the month's name with `<` and `>`; weekday letters; a 7 x 6
  grid of 64 x 44 cells with the day number and up to three dots; today
  ringed in the accent, the chosen day filled; under it the chosen day's
  appointments (up to four rows). A tap chooses a day; a tap on the chosen
  day opens it in Day view.
- **New:** the quick-add field on the keyboard, a preview panel under it
  that says what was understood and when, and a "More details" row that
  opens the editor with those values. Softkeys Cancel and Add.
- **The editor** (New's details, or an existing appointment): Title and
  Where (text), Day, Starts, Lasts and Remind me (steppers), Notes (text),
  and Delete for an existing one (asks once in the softkey bar). Softkeys
  Cancel and Done.
- **Reminders:** each appointment may carry "remind me N minutes before"
  (none, at the start, 5, 15, 30 minutes, 1 hour, 1 day; new ones default
  to 15 minutes). When it comes due the notification centre shows it.

## How the design was iterated (the sheet arguing with itself)

1. *Draft:* Day view as the first view (WM's and Palm's default).
   **Lost:** most days on a handheld have zero to three appointments, so
   Day view is mostly empty hour rows; the agenda shows the same day *and*
   the next ones in the same space. Agenda first; Day one tap away.
2. *Draft:* a week view, because WM had one. **Lost on the screen's
   width:** seven columns in 448 px are 64 px each, too narrow for a word;
   WM's own week view showed only coloured blocks. Agenda covers the
   week's content better. Week goes on the later list for landscape.
3. *Draft:* the full form as the only way to add (the safe default).
   **Lost:** it is WM's design with bigger rows. Quick add is the way in;
   the form is behind "More details" and for edits.
4. *Draft:* quick add with no preview (Google's). **Lost:** without a
   server-grade parser a misread would only show after adding. The preview
   makes the parser's reading visible before it is kept, and bare numbers
   are never taken as times ("Room 12" stays in the title).
5. *Draft:* a reminder as a full-screen alert. **Lost:** it steals the
   keys from a game or a note being typed. A banner in the notification
   centre keeps the screen yours.

## The practices (principles.md)

- **P1:** the views' lists are built when the store changes or the view
  opens, never while the pen moves; the next reminder's time is worked out
  at a change and compared once a minute.
- **P3, P5:** no pictures of its own; panels and text only.
- **P6:** no frame over 4 KB; the views share one list of 64 rows in the
  shell's own memory.
- **Frame rule:** every screen is under 256 draw commands (Month, the
  largest, is about 150).

## Later (not in v1)

Week view (landscape); repeating appointments; a snooze on reminders and a
reminder sound; time zones and travel; categories and colours; attendees;
search in Calendar; import and export of iCal files; LegacySync; birthdays
from Contacts; a Today-screen calendar panel fed from the store (the panel
exists; wiring it is a small follow-on, kept out of this batch's scope);
drag to move an appointment in Day view; languages other than English in
the quick-add line (the international rule: the parser's word tables are
data, the strings table decides them).

## Notes for the owner's morning test

Decided and built; each is a note, not a wait
([decisions-2026-11-13-apps.md](../reports/decisions-2026-11-13-apps.md)):

- **Agenda is the first view.** The alternative is Day or Month first.
- **A weekday name means the coming one, today included** ("fri" typed on
  a Friday is today). The alternative: always the next one.
- **New appointments get a 15-minute reminder and last an hour** unless
  the line says otherwise. Alternatives: no default reminder; 30 minutes.
- **The week starts on Monday.** The alternative is Sunday (a setting
  later; the US default may want Sunday).
- **Dates typed with a slash read month first** (11/14), as the region's
  units do; day first when the region isn't the US.

## What would overturn it

- The owner finding the typed line slower than the steppers on the real
  keyboard: then New opens the editor and quick add becomes its first row.
- Days with many appointments being common for him: then Day view first.
