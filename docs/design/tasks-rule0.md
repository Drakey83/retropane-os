# Tasks, Rule 0

Batch 1 of [core-apps-list.md](core-apps-list.md). Look:
[design-language.md](design-language.md). Store: [pim-store.md](pim-store.md).
Method: [principles.md](principles.md). Built without a stop for the
owner's go; the notes for his morning test are at the end.

## The core job

Keep a list of things to do, see what is due, tick them off. v1: a list
with due dates and priority; add, edit, complete. Calendar tie-ins are on
the later list.

## What others do (findings, not the answer)

| System | The list | Adding | Completing |
|---|---|---|---|
| Windows Mobile 5 Tasks | One list; a sort menu (status, priority, subject, start, due) and a filter menu; overdue in red; an entry bar at the top ("Tap here to add a new task") with `!` and down-arrow buttons for priority | The entry bar for the subject, or a form of about twelve rows | A 12 px tick box |
| Palm OS To Do | Priority numbers 1-5 in a column, due dates in another | Write on a new line | A tick box |
| iOS Reminders | Lists; due items first in "Today"; done items hidden | Type a line; "i" for details | A round box; the row fades away |
| Things, Todoist | "Today" and "Upcoming"; natural-language dates typed in the title ("fri", "tomorrow") | One line | A box; swipe |
| Lean Linux (`todo.txt`) | A text file; priority `(A)`, sorted by it | One line | `x` |

## Borrow and beat

| Problem | Borrowed | How ours goes further on the X51v |
|---|---|---|
| What to do now | iOS and Things: due things first | **One order, no sort menu:** late, then due today, then by due date, then the undated; within a day, high priority first. WM made the user pick a sort; the list that answers "what now?" needs no choosing. Done tasks sink to the bottom, dimmed |
| Adding | WM's entry bar, Todoist's typed dates | The entry field is always at the top; the same quick-add line as Calendar: "Pay rent fri !" is the task, due Friday, high priority. One parser, tested once (`rp_pim::quick`) |
| Saying when | WM's date picker | The due date reads as a person says it: "Late 2 days", "Today", "Tomorrow", "Fri", then "14 Nov". Late is red and says "Late", so colour never carries it alone |
| Completing | The tick box | A 30 px box (WM's was 12): one tap, a green tick, and the row moves down. A second tap undoes it: nothing is lost by a slip |
| Priority | Palm's five levels, WM's three | Three (Low, Normal, High) as a chip row in the editor; in the list only High is marked (`!`) and Low is dimmed. Five levels are four more than most lists use |

## The design (v1 as built)

- **The list:** header "TASKS"; the entry field "Add a task" under it;
  rows of 36 px: the box, the title, the due words right-aligned (and `!`
  before a high one's title). Softkeys **Back** and **Hide done** /
  **Show done**. Empty: "Nothing to do. Type a task above."
- **Adding:** tap the field, type, Done on the keyboard adds it and keeps
  the field for the next one (several tasks are typed in a row).
- **A tap on the box** completes or reopens; **a tap on the row** opens
  the editor.
- **The editor:** Title (text), Due (a stepper: none, then a day at a
  time; it starts at today), Priority (Low, Normal, High chips), Notes
  (text), Delete (asks once). Softkeys Cancel and Done.

## How the design was iterated

1. *Draft:* WM's sort and filter menus. **Lost:** they exist because one
   fixed order was wrong for someone; an order by urgency is right for the
   question the screen is opened to answer, and one softkey (hide done)
   covers the filter people use.
2. *Draft:* swipe to complete (the Oct 16 table's idea). **Lost for v1,
   kept for later:** on a resistive screen a sideways drag on a 36 px row
   fights the list's own scroll, and the box is already one tap. It needs
   the gesture work of interaction.md first; recorded there, not built
   here.
3. *Draft:* a start date as well as a due date (WM). **Lost:** two dates
   to set for a to-do; due is the one people mean.
4. *Draft:* done tasks vanish at once (iOS). **Lost:** a slip of the
   stylus would hide the row before it can be undone. They sink and dim;
   Hide done removes them from view.

## The practices (principles.md)

- **P1:** the order is worked out when the store changes or the day
  turns, never while the pen moves.
- **P6:** the list shares Contacts' 1,024 entries; no frame over 4 KB.
- **Frame rule:** 13 rows with boxes are about 90 draw commands.

## Later (not in v1)

Calendar tie-ins (a task on its due day in Agenda; a task made from an
appointment); swipe to complete; repeating tasks; reminders for tasks
(through the same notification centre); categories and lists; subtasks;
percent complete; a Today-screen tasks line fed from the store; export.

## Notes for the owner's morning test

- **No sort menu:** the order is fixed by urgency. Alternative: add a
  sort softkey.
- **Done tasks stay at the bottom, dimmed,** until Hide done.
- **The `!` in the typed line means high priority.**
- **Due dates read as words** ("Tomorrow", "Late 2 days").

## What would overturn it

- The owner wanting his own order (by priority first): then one softkey
  cycling two orders, not WM's menu.
