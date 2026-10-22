# The shell's workload: what the compositor harness measures

**Status: corrected by the owner, locked (2026-10-22).** The gate's
compositor harness (CPU T1, T3, T4: the owner's vote 2026-10-21, a
benchmark only, built inside the gate) measures the frames the shell will
really draw, not a generic load. This page was drafted as a hypothesis
from what was decided ([compositor.md](compositor.md), ADR-0034: the five
frame modes; [ui.md](ui.md): the SPB-style shell, weather home, smoked
glass) and the owner corrected it the same day; the frames, targets and
rules below are his decisions.

## The frames

Targets (owner, 2026-10-22): **60 fps for finger-driven motion**, where
jank is visible; **30 fps acceptable** for the task switcher's drag (W5)
and the living sky (W6); W3, W7 and W8 are judged by **completion time,
not frame rate**. **Games are separate: 60 fps, full stop.**

| # | Frame | Mode (compositor.md) | What's drawn | Target |
|---|---|---|---|---|
| W1 | **Home (weather) screen scrolled** by the finger | Pan / pan under glass | the page on the video plane, a glass header or dock held on the graphics plane | 60 fps, every frame |
| W2 | **A list scrolled under a translucent header** (Contacts, Messaging, Start menu) | Pan under glass, partial when items change | text-heavy rows; the header's glass over them | 60 fps; a row changing (a new message) costs a partial frame, not a drop |
| W3 | **An app opening from its icon** (and closing back) | Full | the icon grows to the full screen, ~250 ms, the app's first picture under it | completion time: ends on time (~250 ms) |
| W4 | **The soft keyboard sliding up**, then typing | Pan under glass (the slide), partial (key highlight) | a ~480 x 240 panel over the app; one key lit per tap | the slide at 60 fps; tap to lit key within the 50 ms touch promise |
| W5 | **The task switcher**: live thumbnails of open apps (SPB's card stack) | Partial, from textures | 3-6 scaled app pictures, one dragged | 30 fps while dragging |
| W6 | **The weather's living sky**: clouds drifting behind the Today content | Partial | a 3D layer of 1-2 cloud textures under the glass and text | 30 fps. **A v1 feature, a showpiece, not a cut candidate** (owner, 2026-10-22): the sky is always drawn; under load compositor.md §3's step 1 drops only its optional second cloud layer for that frame |
| W7 | **A screen rotation** (portrait to landscape) | Full | the 2D engine's rotation, or a 3D turn | completion time: ends within ~300 ms |
| W8 | **Video with controls over it** (phase 7: its own gate) | video plane + graphics plane | MVED1's picture, a control bar's glass | completion time for the controls; the video at its own rate; listed so the harness leaves room, measured later |
| W9 | **The notification shade** pulled down (owner, 2026-10-22) | Pan under glass (the slide) | a translucent sheet sliding down over the app; a gesture distinct from W4's keyboard slide-up; high frequency | 60 fps (finger-driven) |

**Measured but not critical** (owner, 2026-10-22): real, frequent and
cheap; the harness measures them so they aren't ignored, but no target
gates on them:

- toast and banner notifications: a small slide-down, dismissed by
  itself;
- the volume and brightness overlay: a transient slider on a hardware
  button press;
- the screen waking: a fade-in from the power button to the lock screen.

**The lock screen** stays a note, not a W-number: mostly static, unless
the living sky is behind it (then W6's cost). **It must be able to be
turned off in settings** (owner, 2026-10-22; in [ui.md](ui.md)).

**Finger and stylus:** as [ui.md](ui.md) decided (owner, 2026-10-19): the
finger navigates in the Modern look; the stylus and the buttons do
anything precise (the keyboard stays stylus-sized, text selection, small
targets); the Classic look is stylus and buttons only. The harness drives
W1, W2, W5 and W9 as finger motion, W4's typing as stylus taps.

## What the harness reports for each

- frame times (median, worst, the share over 16.6 ms) and frames degraded,
  by which step of the degrade order (compositor.md §3);
- the CPU's share left over (for the app under the shell), and the touch
  to flush latency where a touch drives the frame (W1, W2, W4, W5, W9);
- which unit drew what (CPU, 2D engine, 3D core, display planes): the
  routing per kind of work (T4) and whether units overlapping in time
  helps or corrupts (P8, P15), and whether commands by DMA beat the CPU
  writing them (T3);
- the measured price against the compositor's price table (5.3 us a tile,
  +6 us textured, +8 us a translucent layer; quality audit C20).

## The owner's corrections (2026-10-22)

The draft asked four questions; the answers are folded in above: the
frames (W9 added, the lock screen a note, three measured-but-not-critical
frames), the targets, the living sky in v1. The fourth (finger or stylus
first?) was already answered in [ui.md](ui.md) (2026-10-19) and shouldn't
have been asked.
