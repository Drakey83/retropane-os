# The shell's workload: what the compositor harness measures

**Status: a DRAFT HYPOTHESIS for the owner to correct (2026-10-22). Not a
spec, not design.** The gate's compositor harness (CPU T1, T3, T4: the
owner's vote 2026-10-21, a benchmark only, built inside the gate) should
measure the frames the shell will really draw, not a generic load. Those
frames come from the owner's design, which isn't drawn yet; so this page
guesses them, from what is decided ([compositor.md](compositor.md), ADR-0034:
the five frame modes; [ui.md](ui.md): the SPB-style shell, weather home,
smoked glass), and marks every guess. The harness is built only after the
owner has corrected this page.

## The frames (each a hypothesis)

| # | Frame | Mode (compositor.md) | What's drawn, guessed | Target, guessed |
|---|---|---|---|---|
| W1 | **Home (weather) screen scrolled** by a finger or the stylus | Pan / pan under glass | the page on the video plane, a glass header or dock held on the graphics plane | 60 fps, every frame |
| W2 | **A list scrolled under a translucent header** (Contacts, Messaging, Start menu) | Pan under glass, partial when items change | text-heavy rows; the header's glass over them | 60 fps; a row changing (a new message) costs a partial frame, not a drop |
| W3 | **An app opening from its icon** (and closing back) | Full | the icon grows to the full screen, ~250 ms, the app's first picture under it | 20-35 fps allowed (compositor.md), ends on time |
| W4 | **The soft keyboard sliding up**, then typing | Pan under glass (the slide), partial (key highlight) | a ~480 x 240 panel over the app; one key lit per tap | the slide at 60 fps; tap to lit key within the 50 ms touch promise |
| W5 | **The task switcher**: live thumbnails of open apps (SPB's card stack) | Partial, from textures | 3-6 scaled app pictures, one dragged | 30-60 fps while dragging |
| W6 | **The weather's living sky**: clouds drifting behind the Today content | Partial (optional effect, first to drop) | a 3D layer of 1-2 cloud textures under the glass and text | 30 fps, dropped under load (degrade order 1) |
| W7 | **A screen rotation** (portrait to landscape) | Full | the 2D engine's rotation, or a 3D turn | ends within ~300 ms |
| W8 | **Video with controls over it** (phase 7: its own gate) | video plane + graphics plane | MVED1's picture, a control bar's glass | the video's own rate; listed so the harness leaves room, measured later |

## What the harness reports for each

- frame times (median, worst, the share over 16.6 ms) and frames degraded,
  by which step of the degrade order (compositor.md §3);
- the CPU's share left over (for the app under the shell), and the touch
  to flush latency where a touch drives the frame (W1, W2, W4, W5);
- which unit drew what (CPU, 2D engine, 3D core, display planes): the
  routing per kind of work (T4) and whether units overlapping in time
  helps or corrupts (P8, P15), and whether commands by DMA beat the CPU
  writing them (T3);
- the measured price against the compositor's price table (5.3 us a tile,
  +6 us textured, +8 us a translucent layer; quality audit C20).

## For the owner to correct

1. Are W1-W8 the frames that matter? Missing ones (a notification shade?
   pinch zoom in Pictures? the lock screen?), wrong ones, wrong order?
2. The targets: 60 fps everywhere it moves, or 30 acceptable somewhere?
3. Does the living sky (W6) belong in v1, or is it an extra?
4. Finger or stylus first for the scroll feel (W1, W2)?
