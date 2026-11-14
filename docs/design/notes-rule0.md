# Notes, Rule 0 (version 2, 2026-11-14)

Version 1 (2026-11-13) was a list of grey rows and a plain text box,
with handwriting, the PDA's heart (ui.md: "notes written with the
stylus"), put off to a later batch. The owner rejected it on 2026-11-14.
This version builds what the survey found, under the rules gate's rule 6
([app-scope.md](app-scope.md)). Look: [design-language.md](design-language.md)
version 2. Records: the one store in **internal storage** (ink strokes
included). Plans it answers to: [apps-and-packs.md](apps-and-packs.md)
(the Notes row: smooth ink on the GPU), [ui.md](ui.md) (capture; page
turns for the notes; the Today screen's Note button).

## The core job

Catch a thought in a second, by stylus or keyboard, and find it again.

## What others do (findings, not the answer)

| System | What it does well | What it costs |
|---|---|---|
| Windows Mobile 5 Notes | Writing and typing modes; ink on ruled lines; a voice recording in a note; the Record button | One-colour ink, jagged strokes; a plain list of names |
| Apple Notes 18 | Folders, pinned notes, checklists, search, ink mixed with text, smart handwriting (straightens, searchable) | Heavy; iCloud |
| Google Keep | A staggered grid of coloured cards; pinned; checklists; labels; quick capture | Little structure for long notes |
| Samsung Notes / GoodNotes | Smooth ink with pressure and palm rejection; paper styles | Tablets and pens |
| Lean Linux (Xournal++, plain Markdown) | Ink to SVG/PDF; text notes as files | Desktop sized |

## Borrow and beat

| Problem | Borrowed | How ours goes further on the X51v |
|---|---|---|
| Seeing your notes | Keep's grid of coloured cards; Apple's pinned | A **two-column grid of cards** in the note's colour, each showing the first lines or a **miniature of the ink**, pinned notes first; a list view one tap away |
| Writing | WM's ink on ruled lines; Samsung's smooth strokes | **Ink smoothed on the fly** (points from the touch controller filtered, then drawn as anti-aliased strokes by the 2700G, so the pen never waits on the CPU); width from speed (no pressure on the TSC2046), five colours, a highlighter, an eraser by stroke; **ruled paper**, plain or grid |
| Mixing | Apple's text and ink together | A note is **blocks**: text paragraphs, ink panels and checklists, in any order; the keyboard or the pen at any point |
| Lists | Keep's checklists | **Checklists** with the same round tick as Tasks; ticked items sink |
| Finding | Apple's search | **Search** over the typed text and titles with the match highlighted |
| Keeping it | WM's sync | **Export** a note as text or Markdown, and its ink as an SVG or PNG, to the PC or the card; import text files |
| Length | n/a | Notes **as long as the user writes** (the store's chained groups; version 1 stopped at 2,000 bytes) |
| Mistakes | Apple's undo | **Undo and redo** for typing and strokes |
| The PDA's own touches | the paper notebook; WM's Record button | Pages turn with the pen between notes (ui.md); the Today screen's Note button opens a new ink note at once |

## The design

- **Home:** large title "Notes / 23 notes", search field, pinned cards,
  then the grid (two columns of 216 px cards, heights from their content,
  up to 180 px). Softkeys **Back** and **New** (New asks nothing: a new
  note opens ready for the pen, the keyboard one tap away).
- **A note:** the title line (first line if not set), then its blocks on
  ruled paper in the note's colour; a tool strip at the bottom over the
  softkeys: Pen (colours), Highlighter, Eraser, Text, Checklist, Undo.
  Softkeys **Back** (keeps it) and **Menu** (Pin, Colour, Paper, Export,
  Delete; Delete asks in place).

## Build list

| Item | From | Status | Code |
|---|---|---|---|
| Home grid of cards in their colours with text previews and ink miniatures; list view | apps-and-packs: a grid of note cards | pending | |
| Pinned notes first | apps-and-packs: pinned notes | built | apps/notes/src/lib.rs |
| Smooth ink drawn by the GPU: filtered points, anti-aliased strokes, width from speed, five colours, highlighter, stroke eraser | apps-and-packs: Smooth ink on the GPU | pending | |
| Ruled, plain and grid paper | sheet | pending | |
| A note as blocks: text, ink and checklists in any order | sheet | pending | |
| Checklists with the round tick | apps-and-packs: checklists | built | apps/notes/src/lib.rs, apps/notes/src/tests.rs |
| Search over typed text and titles with the match highlighted | apps-and-packs: search typed notes | pending | |
| Export as text or Markdown, ink as SVG or PNG; import text files | apps-and-packs: export | pending | |
| Notes of any length | sheet | pending | |
| Undo and redo for typing and strokes | sheet | pending | |
| Sentence capitals and word suggestions in text blocks | sheet | pending | |
| The Today screen's Note button opens a new ink note | sheet | built | shell/core/src/lib.rs, apps/notes/src/lib.rs |
| The light look: the app's own light palette (its content is drawn after the shell's look pass, so rounded shapes keep one colour edge to edge) | sheet | pending | |
| Chips and cards as rounded shapes of one colour, no glass art inside (the owner, 2026-11-14: "a box on top of the button") | owner 2026-11-14 | built | apps/api/src/kit.rs, shell/core/src/panel_art.rs |
| Its own program, loaded when opened and released when left | owner 2026-11-14 | built | apps/notes/src/main.rs, shell/today/src/program.rs |
| Records in internal storage (the card only as the test stage's stand-in) | owner 2026-11-14 | pending | |

## What would overturn it

- Ink feeling laggy on the device past one frame: then strokes are drawn
  by the CPU into the layer as the pen moves and only re-smoothed on lift.
- Handwriting recognition becoming the owner's wish: a separate item with
  its own Rule 0 (an on-device recogniser's size against 64 MB).
