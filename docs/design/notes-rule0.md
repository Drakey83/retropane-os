# Notes, Rule 0

Batch 1 of [core-apps-list.md](core-apps-list.md). Look:
[design-language.md](design-language.md). Store: [pim-store.md](pim-store.md).
Method: [principles.md](principles.md). Built without a stop for the
owner's go; the notes for his morning test are at the end.

## The core job

Write something down before it is forgotten, and find it again. v1: typed
notes, search, add, edit, delete. Ink and handwriting are a later item.

## What others do (findings, not the answer)

| System | The list | Writing | Keeping |
|---|---|---|---|
| Windows Mobile 5 Notes | Files (`Note1.pwi`) in My Documents: name, date, size; names had to be given or were "Note1", "Note2" | A page that takes ink or typing; a recording bar | A file per note; "ok" saves; renaming through a menu |
| Palm OS Memo Pad | The first line of each memo | A plain text page | Kept as typed |
| iOS Notes | Title (the first line) and a snippet, newest first; search across all text | Type; no save | Saved as typed, always |
| Google Keep | A wall of cards | Type; no save | Saved as typed |
| Lean Linux (a folder of `.txt`) | File names | Any editor | Explicit save |

## Borrow and beat

| Problem | Borrowed | How ours goes further on the X51v |
|---|---|---|
| Naming | Palm and iOS: the first line is the title | No name to give and nothing called "Note1". The list shows the first line, then the date and the next line in secondary text |
| Losing a note | iOS: never ask to save | **Every way out keeps the note:** Done, Back, the title bar's ok, Home. There is no Cancel to lose a page by. Deleting is the one deliberate act (a softkey that asks once). And the store keeps it through a pulled battery, which WM's open file did not |
| Finding | iOS: search all the text | The search field is always above the list and matches anywhere in any note as it is typed: a scan of the store's notes in RAM (2,000 notes of 200 bytes is 400 KB of plain bytes: tens of milliseconds, with no index to keep in step) |
| Writing with a stylus keyboard | The phone keyboard | The keyboard's Done key becomes **Return** while a note is open (a note needs new lines; the softkey says Done); the text wraps at words; **a tap in the text moves the caret there**, so a note is corrected in the middle, not only at its end. WM's SIP had the caret but 15 px keys; ours keeps K1's 30 px keys |
| Order | iOS: newest first | By last change, newest first: the note just written is on top |

## The design (v1 as built)

- **The list:** header "NOTES"; the search field; two-line rows (52 px):
  the first line, then "13 Nov - " and the start of the rest in secondary
  text. Softkeys **Back** and **New**. Empty: "No notes yet. New starts
  one." A search with no match says "No note has that".
- **A note:** the text in a panel above the keyboard (ten lines of 24 px
  show; it scrolls to keep the caret in view), the caret in the accent.
  The keyboard's big key reads RETURN. Softkeys **Delete** (asks once:
  "Delete?") and **Done**. An empty note isn't kept.
- **Limits:** a note is up to 2,000 bytes (about 300 words); the count
  left shows in secondary text once under 200.

## How the design was iterated

1. *Draft:* Cancel and Done, as the design language gives an editor.
   **Lost for this app:** a note is the one thing that is all typing; a
   Cancel that throws away a page is the costliest slip there is. The
   design language's rule ("nothing is kept until Done") is for forms;
   this sheet records the exception: **a note keeps itself**, and the
   left softkey is Delete. design-language.md is unchanged: its editor
   rule names forms.
2. *Draft:* the caret only at the end (the settings fields' way: type and
   delete at the end). **Lost:** fixing a word in the first line would
   mean deleting everything after it. The text gets a real caret: tap to
   place it, the D-pad to move it, insert and delete where it stands.
3. *Draft:* a title field above the text (WM's file name). **Lost:** a
   field to fill before the thought is written down. The first line is
   the title.
4. *Draft:* notes as `.txt` files on the card (way C of pim-store.md).
   **Lost on the store's own sheet:** a FAT change per keystroke-save.
   Export as text files is on the later list.

## The practices (principles.md)

- **P1:** the list is built when the store or the search text changes;
  the text's line breaks are worked out when the text changes (a key),
  never while the pen moves.
- **P6:** the note being edited is one 2,000-byte buffer in the shell's
  own memory; no frame over 4 KB.
- **Frame rule:** the editor is ten text runs and the keyboard, about
  100 draw commands.

## Later (not in v1)

Ink and handwriting (batch 6); voice notes (batch 5); longer notes (the
store's group size is the limit today); undo; copy and paste beyond the
keyboard's Paste; folders or tags; a note pinned to Today; export as text
files; text beyond ASCII on screen (the fonts' limit: the store keeps
UTF-8).

## Notes for the owner's morning test

- **There is no Cancel in a note:** leaving keeps it; Delete removes it.
  Alternative: Cancel and Done like the other editors.
- **The keyboard's Done key reads RETURN in a note.**
- **A tap in the text moves the caret.**
- **2,000 bytes a note.** Alternative: longer notes need the store's
  larger group (later).

## What would overturn it

- The owner wanting Cancel: then the note opened is kept aside until Done
  (one more buffer), with Cancel restoring it.
