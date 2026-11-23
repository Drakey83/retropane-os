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
| Home grid of cards in their colours with text previews and ink miniatures; list view | apps-and-packs: a grid of note cards | built | apps/notes/src/lib.rs, apps/notes/src/tests.rs |
| Pinned notes first | apps-and-packs: pinned notes | built | apps/notes/src/lib.rs |
| Smooth ink drawn by the GPU: filtered points, anti-aliased strokes, width from speed, five colours, highlighter, stroke eraser | apps-and-packs: Smooth ink on the GPU | built | apps/notes/src/pen.rs, apps/notes/src/lib.rs, apps/notes/src/tests_d.rs |
| Ruled, plain and grid paper | sheet | built | apps/notes/src/lib.rs, shell/pim/src/store.rs |
| A note as blocks: text, ink and checklists in any order | sheet | built | apps/notes/src/doc.rs, apps/notes/src/lib.rs, apps/notes/src/tests_d.rs, xtask/src/app_mockups.rs |
| Checklists with the round tick | apps-and-packs: checklists | built | apps/notes/src/lib.rs, apps/notes/src/tests.rs |
| Search over typed text and titles with the match highlighted | apps-and-packs: search typed notes | built | apps/notes/src/lib.rs, apps/notes/src/tests.rs |
| Export as text or Markdown, ink as SVG or PNG; import text files | apps-and-packs: export | built | apps/notes/src/export.rs, apps/notes/src/import.rs, shell/today/src/cardfiles.rs, shell/today/src/main.rs, xtask/src/emu/apps.rs |
| Notes of any length | sheet | built | apps/notes/src/doc.rs, shell/pim/src/store.rs, apps/notes/src/tests_d.rs |
| Undo and redo for typing and strokes | sheet | built | apps/notes/src/lib.rs, apps/notes/src/tests.rs |
| Sentence capitals and word suggestions in text blocks | sheet | built | apps/notes/src/words.rs, apps/notes/words/en-US.txt, apps/notes/build.rs, shell/core/src/caps.rs |
| The Today screen's Note button opens a new ink note | sheet | built | shell/core/src/lib.rs, apps/notes/src/lib.rs |
| The light look: the app's own light palette (its content is drawn after the shell's look pass, so rounded shapes keep one colour edge to edge) | sheet | built | shell/core/src/lib.rs, apps/api/src/kit.rs, tools/shell-assets/accents.py |
| Chips and cards as rounded shapes of one colour, no glass art inside (the owner, 2026-11-14: "a box on top of the button") | owner 2026-11-14 | built | apps/api/src/kit.rs, shell/core/src/panel_art.rs |
| Its own program, loaded when opened and released when left | owner 2026-11-14 | built | apps/notes/src/main.rs, shell/today/src/program.rs |
| Records in internal storage (the card only as the test stage's stand-in) | owner 2026-11-14 | blocked: owner 2026-11-15 sign off without it: internal storage waits on the flash driver; the card stands in, same code and format | |

## Group D as built (2026-11-23)

Five rows built in one go (the owner's ask of 2026-11-23). For each: the
vendor or textbook way (a finding), ours, why ours wins, and what was
measured.

- **Smooth ink.** WM drew raw points as one-pixel lines; the textbook
  smoothing (a moving average) lags most when the pen is fastest. Ours is
  the 1 Euro filter (Casiez et al., CHI 2012) in integer arithmetic
  (`pen.rs`), its two constants chosen by sweeping 168 pairs over a
  measured test (a circle written at 1.5-12 px a sample with a +-2 px noise
  band, `tests_d.rs`): a slow pen's jitter **12 -> 5 tenths of a pixel**,
  **no lag** from 6 px a sample, 2.2 px at the slowest, and the stroke's end
  snapped to the lift. Width comes from speed (the TSC2046 has no
  pressure): each point keeps a 4-bit width in the stroke (format version
  2, below), and the brush goes from one point's width to the next. Six
  pen colours (five planned; the owner asked for more on 2026-11-14), the
  highlighter in its own or any pen's colour (its see-through band shown
  over each well while chosen), the eraser by stroke within its block (a
  ring where it is). **The GPU's part:** each new segment is painted once
  into a 4-bit coverage layer (a few hundred pixels, microseconds) and the
  2700G composes the layers every frame (`Cmd::Cover`, anti-aliased,
  tinted); re-sending every stroke as triangles each frame was rejected:
  its cost grows with the ink, the coverage layer's never does. Latency and
  the filter on the X51v's own noise: R372 (device run).
- **Blocks in any order.** A note's lines are text, checklist items and
  **ink blocks** (`0x01` and the block's number); each stroke names its
  block in a version-2 extension byte (style bit 7), in the block's own
  coordinates, so text typed above a drawing moves it down whole (version
  1's sheet-wide ink lay under later text). A block is as tall as its
  writing plus a row and a half (seven rows while empty) and grows as it is
  written lower; the pen on blank paper below the last line grows the last
  block or starts a new one; TEXT and LIST put a paragraph or an item after
  the caret's line; a tap on a line moves the caret there. The note
  scrolls (a drag off the ink, or the rail along the right edge). **Looks
  for the owner's pick (R373)**, drawn by the real program
  (`cargo xtask ui-preview`, `v3-notes-*`; copies in
  [mockups/notes-blocks/](mockups/notes-blocks/)): A open sheet (built as
  the default: the ink blocks unmarked, like the whole-sheet ink the owner
  used on 2026-11-15), B framed pads, C margin marks; the ink tools (marker
  wells, eraser ring), Export, and typing with suggestions.
- **Any length.** WM rewrote a note's file whole; version 1 stopped at
  2,000 bytes. Ours keeps the lines in **chunks** (the note's own `TEXT`,
  then text pages: `Kind::Ink` records with `tag::ORDER`, `doc.rs`), each
  edit rewriting the one chunk it touches (a keystroke in a 60 KB note: one
  record appended, tested), a chunk past its room split at the line
  nearest its middle, a line longer than a chunk going on as continuation
  lines (typing goes to the paragraph's end). Bounded only by the store's
  room. **Migration:** none needed: version-2 notes read as they are (their
  ink is block 0, their strokes version 1), tested with a note in the old
  format.
- **Export and import.** WM saved only `.pwi`. Ours streams standard files
  a piece at a time (no buffer a file's size in the 256 KB program slot):
  text (CRLF), Markdown (task items, ink as images beside it), SVG (the
  brush's own Catmull-Rom as exact Beziers, a path per run of one width)
  and PNG (indexed colour, deflate with matches one byte and one row back:
  **5.6 KB for a 448 x 224 block, 100 KB raw**, decoded back to the brush's
  coverage in a host test). Files go to the card's `RetroPane` folder
  through group A's card file service (`Program::card_ask`: `CARDPUT`,
  `CARDFILES`, `CARDFILE`; `rp_today::cardfiles`), one service for every
  app (R374), where the PC sees them over MTP. Notes brought it two things:
  a write streams from the app a piece at a time into a staged run linked
  whole at the end (`put_from`: no file need fit the image's 256 KB pool,
  and a pulled card or a note changed mid-write leaves the old file), and
  text files (`.txt`, `.md`) read a piece at a time, like pictures. The
  image checks every name again (`.vcf .ics .txt .md .svg .png` only, no
  path, no `..`) and passes a refusal's reason back for the tray.
  `emu-apps` exports a note on the image and reads both files off the
  emulator's card. Import lists the card's text files by number (root and
  `RetroPane`) and reads one into the open note, refusing damaged files by
  offset (security.md, 2026-11-23).
- **Sentence capitals and word suggestions.** WM's completion list never
  learned; Android's LatinIME carries 1-2 MB a language. Ours: a ranked US
  English list front-coded at build time (**1,029 words in 6.3 KB**, read
  in place: no RAM), plus the words the user wrote, counted from the store
  when the keyboard is up (names and places offered after two uses);
  capitals as typed. Another language is a word list in
  `apps/notes/words/`. Capitals: the app tells the keyboard where its caret
  is (`Program::caps`): a new paragraph, an item, and after `. `, `! `, `? `.

## What would overturn it

- Ink feeling laggy on the device past one frame: then strokes are drawn
  by the CPU into the layer as the pen moves and only re-smoothed on lift.
- Handwriting recognition becoming the owner's wish: a separate item with
  its own Rule 0 (an on-device recogniser's size against 64 MB).
