# The design language: one page every app follows

Distilled 2026-11-13 from what is built and liked: the Today shell
(`shell/core`, `shell/today`), the Weather app and its settings screens
(`shell/core/src/weather_app.rs`, `weather_settings.rs`), the glass panels
(`shell/core/src/panel.rs`), the keyboard (`keyboard.rs`),
[icon-brief.md](icon-brief.md) and the icon prompt. It records the look; it
doesn't re-decide it. An app that needs something this page lacks adds it
here first, so five apps never drift into five looks. How a design is
argued: [principles.md](principles.md), modelled in
[weather-rule0.md](weather-rule0.md).

## The frame every screen shares (480 x 640, portrait)

| Band | Where | What |
|---|---|---|
| Title bar | y 0-36 | Start (left, 104 px), the app's name, battery and clock (right). The shell's, never the app's |
| Header | y 36-94 | A back chevron (64 px wide), the screen's name in spaced capitals; at most two controls on the right |
| Content | y 98-594 | Panels on the wallpaper, 16 px side margins (448 px wide) |
| Softkey bar | y 600-640 | Two softkeys, 160 px each: left is the way back or the quiet action, right is the screen's main action |

- Keys keep their Windows Mobile jobs: Home goes to Today, the D-pad moves
  a focus ring, Enter acts, the title bar's ok and the left softkey go back.
- **Every target works by stylus and by D-pad.** Nothing is reachable by
  one only.

## Colour

All colours are the dark look's, written once through `rp_draw::look::theme`;
the light look is derived from them (`light_twin`, `light_glass`,
`light_text`), never chosen per app.

| Use | Colour (dark look) |
|---|---|
| Text | `0xFFF0F2F4` |
| Secondary text (labels, hints, times) | `0xFFCCD3DA` |
| Focus ring, the selected chip, the caret | `0xFF7FD0FF` (the one accent) |
| Under the pen (pressed) | `0xFF3C4656` |
| Done, held, a ticked box | `0xFF5CE07A` |
| Warning, overdue, delete | the alert chip's amber and red (`weather_app`), nowhere else |

- **One accent.** The cyan marks where you are (focus, today's date, the
  chosen segment). It is never decoration.
- Colour never carries meaning alone: overdue is red *and* says "overdue";
  high priority is a mark *and* a word in the editor.

## Glass

- Every box is a glass panel (`panel::draw`): frosted, 12 px rounded
  corners, a bright rim, a sheen at the top; 90% opaque in the dark look,
  96% in the light. The wallpaper stays visible between panels.
- A panel under the pen is the pressed glass (`panel::draw_pressed`).
- Bars (title, header, softkeys, the keyboard) are left-to-right glass
  gradients with a one-pixel light edge.
- No drop shadows, no new gradients, no per-app box styles.

## Type

Didact Gothic, four cuts (`shell/today/src/fonts.rs`), ASCII for now:

| Font | Use |
|---|---|
| `ui` (18 px, 24 px line) | Everything read as words: rows, fields, softkeys, menus |
| `label` (spaced capitals) | Section labels, screen names, weekdays, chips, key caps |
| `big` (light numerals) | One hero number a screen: the clock, a temperature |
| `num` (36 px numerals and signs) | Numbers that are typed and read at arm's length: the calculator's display and keys (digits and `. , - + x / = %` only, 5 KB) |

- Sentence case in `ui`; capitals only through `label`.
- Text is left-aligned; a row's value is right-aligned. Numbers in columns
  align right.
- Text that doesn't fit is cut at a word or with `..`, never shrunk.

## Spacing and sizes

- The grid is 4 px. Side margins 16; a 6 px gap between panels; 12 px of
  padding inside a panel before text.
- **A row is 36 px** (text baseline 25 px down), a two-line row 52. A list
  is a stack of one-row panels under spaced-capital section labels (18 px).
- **The smallest stylus target is 30 x 30** (the keyboard's keys); anything
  a thumb should hit (softkeys, calculator keys, the header) is 40 or more.
- A text field is a 40 px panel with the focus ring while typing and a
  cyan caret. The keyboard (K1 compact, 208 px) docks above the softkey bar;
  the field and what it changes stay visible above it.

## Panel rules

- Lists scroll under the header; the header and softkeys never move.
- A choice of two to four is a segmented chip row inside the row (the
  chosen one cyan), not a drop-down. More than four opens a list screen.
- **Changes apply as they are made.** No Save buttons for settings-like
  rows. An editor screen (a new appointment, a contact) has Done on the
  right softkey and Cancel on the left, and nothing is kept until Done.
- Deleting asks once, in place, never in a pop-up box: the control that
  deletes changes its own words for the next tap (an editor's Delete row
  reads "Tap again to delete"; a note's Delete softkey reads "Delete?"),
  and any other tap takes the question away.
- An empty list says what it is for and how to add the first one, in
  secondary text, in a panel.
- Motion: pages slide with the pen and settle in under 200 ms; nothing
  animates that the user didn't move. Every frame meets the 33/45 ms rule.

## Icons

- The Frutiger Aero glass set of [icon-brief.md](icon-brief.md): real
  objects in three-quarter view, lit from the top left, teal and aqua with
  warm accents only where the object calls for it, readable at 48 px.
- An app's icon appears in the Start menu, Programs and the task switcher.
  **Inside an app, controls are words or simple drawn marks** (the back
  chevron, a tick box, a plus), never small pictures.
- State is a badge over a base icon (a check, a warning), not a new
  drawing.

## What an app must not do

- Draw its own title bar, softkey bar, keyboard or panel style.
- Add a colour, a font or a size that isn't on this page.
- Put work in the touch path (principles.md P1): lists are sorted and
  searched when the data changes, not while the pen moves.
