# Sheet: the glass panels' cost on the 2D engine (R176, R174), 2026-11-01

**What it decides.** How the glass panels (design B, the owner's pick) are
drawn so screen changes are back to their speed before the panels, with
the look unchanged. No code until the owner's go.

## The cause, proven

- **Symptom (the X51v, local/rp-today-run-2026-11-01-panel.txt against
  the card run before the panels):** frames that draw a page whole
  (into Station, Settings, back to Weather) CPU worst 114 -> 159-188 ms;
  the sky's frames CPU median 5.0 -> 11.9 ms.
- **Count (host, `engine_commands_per_screen`):** Today's frame is 60
  commands, but **786 engine commands** (12 panels, 690 of them panel
  fills); Weather's Now page drawn whole 68 -> 432; Settings 73 -> 503.
  Before the panels a box was 2 commands.
- **Why so many (mbx-render `command`):** a panel is a fill per run of
  like rows (~60 a panel), and **every see-through fill first copies the
  wallpaper under it, then blends** (a fill sets its pixels): ~120 engine
  operations a box, each queued by the CPU (the FIFO drains ~7.7 us a
  word, P18).

## Rule 0

- **Windows Mobile and Dell (a finding):** WM's shell draws flat, opaque
  boxes; nothing translucent, nothing to borrow.
- **The modern way:** a nine-slice image or a shader (CSS backdrop and
  gradient), one draw a panel, on a GPU.
- **Ours:** the 2D engine has no shader and no stretch-blend; the panel's
  picture whole is too big for its cache (the hero alone 369 KB, cache
  256 KiB). So: fewer, cheaper commands, the same pixels.

## The options

| | What | Engine commands, Today | Look | Cost to build |
|---|---|---|---|---|
| A | **Rows the screen can show:** the art's 64 row colours rounded to what RGB565 can show (red, blue to 8 levels, green to 4), so like rows join: ~10-15 fills a panel, not ~60 | ~786 -> ~300 | unchanged to the eye (each step under one 565 level, the screen's own limit) | the generator only |
| B | **One wallpaper copy a panel:** the panel's command copies the wallpaper under its whole rectangle once, then only blends its rows, rim and corners (now: a copy and a blend for every row) | ~786 -> ~420 | identical (the reference renderer drawn the same way, the model test holds it) | the panel command in both renderers |
| **C** | **A and B together** (recommended) | **~786 -> ~150-200** | unchanged | both, small |
| D | The panel as one cached picture | 1 a panel | identical | doesn't fit the engine's cache (hero 369 KB) |
| E | Back to flat glass | ~30 | loses the owner's pick | none |

## What overturns it (one device run)

The page-drawn frames' CPU worst back to ~114 ms or under, the sky's frame
CPU median back near 5 ms, and the owner seeing no difference in the
panels. If the frames stay slow with the commands cut ~4x, the cost is
elsewhere (the reveal's freshness checks, the page builds) and the next
sheet follows that.
