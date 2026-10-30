# The UI workbench: how a UI change gets from idea to the X51v

Written 2026-10-30 (overnight) for the owner's UI/UX work starting that
day. It decides nothing: it says what is ready, the loop to iterate in,
and what waits on a vote. The look itself is [ui.md](ui.md); the
compositor is [compositor.md](compositor.md) and ADR-0034.

## The loop, fastest first

| Step | Command | Time | What it shows |
|---|---|---|---|
| 1. Picture on the PC | `cargo xtask ui-preview` | seconds once built | The Today screen's states (start, D-pad focus, Start menu, Home) drawn by the **same shell and 2D renderer code the X51v runs**, on the 2D engine's model; PNGs and an index page in `target/ui-preview/`, opened in the browser pane |
| 2. Host tests | `cargo test -p rp-shell -p rp-today` | seconds | Layout, hit targets, focus order, partial redraws against full ones |
| 3. Emulator | `cargo xtask emu-today` | a few minutes | The image itself under the loader in the QEMU fork, pixel-exact against step 1's renderer; a stylus tap |
| 4. The X51v | `cargo xtask load rp-today` (the owner's "run it") | a minute | The truth: colours on the real panel, touch feel, timing (touch-to-flush, queue-to-flip: the ratchet's `rp-today` floors) |

A look is iterated at step 1 with the owner (a picture a minute), proven
at step 3, and judged by eye at step 4. Nothing "works" until step 4
(the proven-on-hardware rule).

## What exists to build on

- **`rp-shell`** (`shell/core`): input (touch, D-pad, Home), focus, the
  Start menu, targets and actions, drawing into an `rp_draw::Frame`; the
  shell never draws pixels itself. **`rp-today`** (`shell/today`): the
  Today screen's layout (T1), VClouds' icons baked in (T2), the clock,
  fonts, the touch replay the perf ratchet times.
- **The 2D renderer** (`rp-mbx-render`, ADR-0023): the screen in RGB565,
  flips written in the vertical blanking (R113), partial redraws.
- **The 3D renderer and the graphics service** (`rp-mbx3d-render`,
  `services/graphics`, ADR-0026/0030/0034): scenes composed by the 3D
  core, tear-free on the X51v (R110), lists in the on-die memory (R116),
  the copy path for big changes (R20). This is what the voted option D
  (ui.md "3D acceleration for the whole UI") is built on.
- **Speed facts for UI budgets:** the 3D core at 149.50 / 2 (the shipped
  top rung); divide 1 measured faster (up to 143.00 / 1 game-safe) but
  held for later, and tonight's runs show its copy path needs the
  divider switch (R117S: 1 us, clean) or R120's answer before it ships.

## What waits on the owner before UI code (design first, ROADMAP Phase 5)

- **The UI design spec and mock-ups approved** (`docs/design/`), the
  first box of Phase 5. Boards N-R are the chosen direction (ui.md).
- **ui.md's "Ideas: modern 3D in a retro-modern UI"** (a 3D flip clock, a
  living weather sky, glass that catches the light, a card-stack task
  switcher...): proposals, not voted.
- **The workload brief** ([workload-brief.md](workload-brief.md), W1-W8):
  a draft for the owner to correct; it decides what the compositor
  harness measures.
- **T3/T4** (wallpaper and settings from the card) wait for the OS's
  storage service, not for a vote.

## Suggested first session (for the owner to change)

1. Open `target/ui-preview/index.html` in the browser pane: today's
   screen as the device draws it.
2. Pick one screen (the Today screen is furthest along) and say what to
   change; each change is a preview picture in seconds.
3. When a look is agreed, the emulator check and one device run prove it.

The preview's states are listed in `xtask/src/ui_preview.rs` (`shots`):
adding a state is one line, so any screen the shell can reach can be
previewed.
