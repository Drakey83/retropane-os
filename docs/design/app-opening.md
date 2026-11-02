# Opening an app from a card: the rules (proven 2026-11-01)

The Weather app opens from the Today card by Reveal: the app grows out of
the card in a video-plane window, then rests as the app. It took a day of
the owner's runs and videos to make it clean on the X51v. Each rule below
is a fault he saw, its cause proven from the run's log or his video, and
what now prevents it. **Any app or widget that opens this way (the Modern
and Classic looks' widgets, R170; every app reusing the animation) follows
all of them.**

| # | The rule | What broke without it (proof) | Where it lives |
|---|---|---|---|
| 1 | **Never show an old picture.** Every picture composed ahead (the app, its chrome, its pages, a chart's line) carries the key of the draw list it was made from; at the tap it is checked against the app now, and a stale one is composed again before it shows. | The opening grew the dark app in the light look, then swapped (run of 2026-11-01: no picture rebuilt for 2 minutes while the living sky ran). | `rp-today` main: the tap's key check, `app_check`; `keep_graph` before each frame |
| 2 | **Background work must not wait for a still screen** when something on it never stops (the living sky); but nothing heavy may run beside an animation either: a page build (~110 ms) freezes the sky 3-4 steps. Heavy work happens at the moment it is needed, behind a picture already right. | Pictures stale for minutes; the sky stuttering. | `pages_check` (still only), the hand-over's page build |
| 3 | **The opening ends on the very pixels it showed.** Its last frame and the app at rest are the same picture (a host test holds the two compositions equal in both looks); the screen then rests on it until the app changes or is touched. | Three pictures in 83 ms at the end (DSIG trace). | `reveal_ends_on_the_same_picture_the_app_rests_on`, `rest_on_app` |
| 4 | **Plane registers are written only in scan lines 640-642.** A write in line 643 split across two refreshes: the window at its new place reading its old picture start. | The page 34 px high for one refresh (the owner's video IMG_0558 frame 97, today-0048). | `rp-graphics` layer.rs `PLANE_FROM`/`PLANE_TO`: every client gets it |
| 5 | **The layer is never switched off and on mid-sequence**; it moves. | The same one-refresh split at the hand-over. | the rest keeps the layer on, over the whole picture |
| 6 | **A press never fills over content.** On the 2D engine a fill sets its pixels (mixed with the wallpaper only); a press is the same glass drawn darker. | The card, its words and picture gone for ~130 ms under the pen (IMG_0559 frames 160-168). | `panel::draw_pressed` |
| 7 | **Text bounds hold where the glyphs fall**, not only the line box. | Clouds vanished at the right edge (skipped while visible). | `rp_draw::Cmd::bounds`, `Font::ink` |
| 8 | **Every opening logs itself:** each refresh's signature, registers and the line its last write landed on; its end in one line (picture changes after its last refresh, refreshes not sampled); what ended the rest. A clean end reads "changed 0 times". | Faults were guessed at before the trace existed ("stop guessing"). | `trace of open`, `open N end`, `rest:` lines |

How a fault is chased (the owner, 2026-11-01): the run's log first (the
card log, `cargo xtask cardlog`, when the PC's copy is missing), then his
video split into frames (`ffmpeg`, a contact sheet, the bad frames at full
size, offsets measured in screen pixels), the two matched by time; the
cause named with its evidence before any fix.
