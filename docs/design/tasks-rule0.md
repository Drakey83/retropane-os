# Tasks, Rule 0 (version 2, 2026-11-14)

Version 1 (2026-11-13) was one list of grey rows with a "!" for
priority, everything modern under "Later"; the owner rejected it on
2026-11-14. This version builds what the survey found, under the rules
gate's rule 6 ([app-scope.md](app-scope.md)). Look:
[design-language.md](design-language.md) version 2. Records: the one store
in **internal storage**. Plans it answers to: [apps-and-packs.md](apps-and-packs.md)
(the Tasks row), [ui.md](ui.md) ("tasks with real checkboxes", one tap
from Today).

## The core job

Get a task out of your head in two seconds, see what matters today, and
tick it off with a satisfying tap.

## What others do (findings, not the answer)

| System | What it does well | What it costs |
|---|---|---|
| Windows Mobile 5 Tasks | Real checkboxes; categories; due dates and priority; Outlook sync | A menu to sort; a form to add; no lists, no repeats visible |
| Things 3 | Today / Upcoming / Anytime / Someday; projects with a progress ring; the tick animates; a calm, tinted look | Apple only, no sync to others |
| Todoist | Natural-language quick add ("pay rent every 1st p1 #home") with the words highlighted; colours per project; priorities as coloured flags | Its server for sync |
| Microsoft To Do / Google Tasks | My Day; lists; subtasks (steps); sync with Outlook or Google | Plain look |
| iOS Reminders 18 | Smart lists with counts (Today, Scheduled, Flagged); list colours and icons; subtasks; CalDAV sync | Some features need iCloud |
| Lean Linux (taskwarrior, todo.txt) | Fast typed syntax; urgency computed | Text only |

## Borrow and beat

| Problem | Borrowed | How ours goes further on the X51v |
|---|---|---|
| What matters now | Things' Today; Reminders' smart lists | The home opens on **smart tiles** (Today, Upcoming, Late, All, Flagged) with counts in large numerals over the user's **lists with their colours and a progress ring each**; Today pulls in late tasks first |
| Adding | Todoist's highlighted quick add | One line: "Pay rent fri !1 #home every month" with **date, priority, list and repeat highlighted** as typed and a preview chip row; added to the list on screen |
| Ticking off | Things' animated tick; WM's real boxes | A **round checkbox** in the list's colour that fills with a tick (200 ms) and the row slides to the done group; **swipe right** to complete, **swipe left** for Tomorrow / Next week / Delete |
| Priority | Todoist's coloured flags | **Flags** in red, amber, blue (and none), with the word in the editor (colour never alone) |
| Big jobs | To Do's steps | **Subtasks** under a task with "2 of 5" and a small ring |
| Repeats and reminders | Reminders' alerts | Repeats (daily, weekdays, weekly on days, monthly, yearly, every N, after done); a **reminder** through the same banner and sound as Calendar |
| Calendar | Fantastical's tasks in the calendar | Tasks show on their due days in Calendar's agenda; **"Make a task"** from an appointment |
| Same list everywhere | To Do and Google Tasks sync; Reminders' CalDAV | **CalDAV tasks (VTODO)** and **Google Tasks** through the shared Connections page, over any connection |

## The design

- **Home:** large title "Tasks / 4 due today, 1 late"; a 2 x 2 grid of
  smart tiles (Today, Upcoming, Late, Flagged) with counts in the tint;
  then **My lists**: rows of colour dot, name, progress ring and count;
  "+ List". Softkeys **Back** and **New**.
- **A list:** its name in the list's colour, the add line at the top, open
  tasks grouped by due (Late, Today, Tomorrow, Later this week, Someday),
  each row: round checkbox, title, flag, subtask count, due in words
  ("Tomorrow", "Late 2 days" in red). Done tasks folded at the bottom
  ("12 done").
- **Editor:** title, notes, due date and time, reminder, repeat, flag,
  list (colour chips), subtasks with their own checkboxes. Delete asks in
  place.

## Build list

| Item | From | Status | Code |
|---|---|---|---|
| Home with smart tiles and counts (Today, Upcoming, Late, Flagged) and the large title | sheet | built | apps/tasks/src/lib.rs |
| Lists with colours, a progress ring each, made and renamed by the user | apps-and-packs: lists with colours | pending | |
| Quick add with date, priority, list and repeat highlighted as typed | sheet | built | apps/tasks/src/lib.rs, apps/tasks/src/tests.rs |
| Round checkbox that fills with an animated tick; the row moves to done | sheet | built | apps/tasks/src/lib.rs, shell/core/src/lib.rs |
| Swipe right to complete, swipe left for Tomorrow, Next week, Delete | apps-and-packs: Swipe to complete | built | apps/tasks/src/lib.rs, apps/tasks/src/tests.rs |
| Coloured priority flags (word in the editor) | sheet | built | apps/tasks/src/lib.rs |
| Subtasks with "2 of 5" | apps-and-packs: subtasks | pending | |
| Repeating tasks (daily, weekdays, weekly on days, monthly, yearly, every N, after done) | apps-and-packs: repeating tasks | pending | |
| Reminders through the notification banner and sound | apps-and-packs: reminders | pending | |
| Tasks on their due days in Calendar; make a task from an appointment | apps-and-packs: tie-ins with Calendar | pending | |
| Today screen's tasks line fed from the store | sheet | pending | |
| CalDAV tasks and Google Tasks sync over any connection | sheet | pending | |
| Search across tasks | sheet | built | apps/tasks/src/lib.rs, apps/tasks/src/tests.rs |
| The light look: the app's own light palette (its content is drawn after the shell's look pass, so rounded shapes keep one colour edge to edge) | sheet | built | shell/core/src/lib.rs, apps/api/src/kit.rs, tools/shell-assets/accents.py |
| Chips and cards as rounded shapes of one colour, no glass art inside (the owner, 2026-11-14: "a box on top of the button") | owner 2026-11-14 | built | apps/api/src/kit.rs, shell/core/src/panel_art.rs |
| Its own program, loaded when opened and released when left | owner 2026-11-14 | built | apps/tasks/src/main.rs, shell/today/src/program.rs |
| Records in internal storage (the card only as the test stage's stand-in) | owner 2026-11-14 | pending | |

## What would overturn it

- Swipes misfiring with the stylus on the real panel: then the swipe
  needs a longer travel, or the left swipe's choices move to a hold.
