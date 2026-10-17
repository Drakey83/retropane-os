# ADR-0021: The shell library and the first Today screen

- **Status:** Accepted (2026-10-13, by the project owner)
- **Date:** 2026-10-13
- **Last verified:** 2026-10-13 (against commit 3158b26)

## Context

The owner wants a first tappable interface on the X51v soon, without
waiting for the MBX bring-up (item 69), and without building it twice.
Owner's choice (2026-10-13): the shell is a library, run now by a test
image under the loader (as `rp-blend` is), and moved onto the kernel
unchanged when its display and input services exist. Condition from the
review the owner accepted: the library must not bake CPU drawing in, because
the "know the silicon" track pulled the MBX ahead of the UI design for
exactly that reason.

> **Correction (note added 2026-10-13):** touch did not yet work on the
> device when this was written: `rp-drv-tsc2046` was emulator-tested only,
> and its first device run (rp-today, commit 592dece) put every tap at the
> top left. It works since, after the settling, alignment and pressure
> filter fixes in docs/hw/touch.md (`rp_touch_cal::Filter`).

What exists and works on the device: the two-plane composition at scan-out
(wallpaper in the video plane, interface in the graphics plane, blended by
the 2700G: item 60), the full-colour interface layer and its dithered
flush (`rp_gfx::layer`), glass gradients, glyph masks, touch (`rp-drv-
tsc2046`), the keypad, backlight, and the loader's run log for numbers.

> **Correction (note added 2026-10-13):** the ~1.7 ms is WM's GDI path
> (`gdi_fill_screen_each` in docs/perf.md: "call and read-back overhead"
> per operation), not the engine's own wait: under the loader, rp-mbx's
> fence was written at once (under 1 us) and a full-screen fill takes
> 3.1 ms (docs/hw/2700g.md, "The MBX 2D engine").

What was measured today and shapes this (docs/perf.md, docs/hw/2700g.md):
the 2700G draws WM's fills off the CPU at ~4.5x the CPU's speed, but
waiting for each operation costs ~1.7 ms. A GPU backend must get a whole
frame's drawing at once and wait once.

The design brief's promise: **touch answered within 50 ms**. Nothing
measures it yet.

## Decision

> **As built (note added 2026-10-13):** `Frame`, `Cmd` and the `Renderer`
> trait live in their own crate, `rp-draw` (drivers/draw); `rp-shell`
> depends only on it and `rp-touch-cal`. The commands are `Fill`,
> `HGradient`, `Outline`, `Image` and `Text`: a glass panel is a gradient
> plus an outline, and there is no clip command (the renderer clips to
> each dirty rectangle). A frame holds 160 commands, 1024 text bytes and
> 8 dirty rectangles (more merge). `CpuRenderer`
> (drivers/gfx/src/render.rs) flushes to the plane as ARGB4444, dithered.
> `rp-today` is in shell/today, not boot/today. There is no RTC driver: the clock starts
> from the build machine's time (shell/today/src/clock.rs). Only
> touch-to-flush is logged; touch-to-vsync is not (the vsync wait isn't
> known yet, shell/today/src/lib.rs); first number: worst 35 ms over 5
> taps (docs/hw/touch.md). Added: an align screen (item 76) with an
> "Align screen" Start entry in the test image, and the pressure filter.
> The calendar line reads "No appointments today"; the battery is drawn
> as an outline, not read.

1. **`rp-shell` (in `shell/`), `no_std`, no `unsafe`, host-tested:** owns
   the Today screen, title bar (Start at top left, clock, battery), Start
   menu and the two softkeys (docs/design/ui.md), the widget model,
   layout for 480 x 640, hit-testing, focus ring and D-pad navigation
   (keys keep their WM jobs), and which rectangles changed. It knows
   nothing about pixels, planes or hardware.
2. **Drawing is a per-frame command list, not calls.** Each frame the
   shell emits a `Frame`: a short list of primitives (fill, glass panel
   with gradient and edge, image, glyph run, clip) plus the dirty
   rectangles. A `Renderer` trait takes the whole `Frame`:
   - **`CpuRenderer`** (now, in `rp-gfx`): runs the list into the ARGB8888
     layer and flushes the dirty rectangles to the graphics plane.
   - **An MBX renderer** (when item 69 lands): turns the same list into
     one command buffer, one submission and one fence per frame (the
     ~1.7 ms per-wait cost measured under WM is paid once, not per
     panel). The shell does not change.

   > **Correction (note added 2026-10-13):** the MBX 2D engine found
   > since (docs/hw/2700g.md, "Copies, keys and the rest of the command
   > set") does solid fills, copies (colour key, format conversion) and a
   > solid colour through 4 bpp coverage (anti-aliased text) only: no
   > per-pixel ARGB blending onto the destination, no gradients, and it
   > reads only the 2700G's own memory. Gradients and alpha would need the
   > MBX 3D core, not brought up yet.

   The primitives are the ones a GPU does well (rectangles, gradients,
   textured quads, alpha masks), so nothing in the list is CPU-only.
3. **Text and art are prepared on the PC, at build time:** the font
   (Didact Gothic, SIL OFL, docs/design/ui.md) as glyph masks at the sizes
   the Today screen uses; icons as ARGB images; the wallpaper as the video
   plane's 4:2:0 YUV. No font, image or file parser runs on the device in
   this ADR (that comes with wallpapers from the card, a later ADR).
4. **`rp-today`, a test image** (boot/today, loaded by the loader like
   `rp-blend`): wires touch, keys, the planes and the clock (the PXA27x
   RTC as WM left it) to `rp-shell` and `CpuRenderer`, loops on input,
   sleeps when idle. A "Back to loader" entry in the Start menu (test
   image only) returns; the loader's run limit is the backstop.
5. **The 50 ms promise is measured from the first build:** each touch is
   timestamped when the pen-down is read (OS timer), and again when the
   renderer's flush of the resulting frame has completed; the next vsync
   after it is noted too (when the change is first on the glass). The run
   log reports count, median and worst touch-to-flush and touch-to-vsync.
   Over 50 ms worst case is a finding to fix before the real shell, not a
   number to explain away.
6. **Placeholders, honestly labelled:** weather is a static reading (no
   network yet); the quick buttons and Start entries other than Today and
   "Back to loader" show "not yet". The calendar line reads a fixed day.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Shell logic host-tested (hit-testing, focus moves, dirty rectangles, every `Frame` checked against a CPU reference render); `cargo xtask emu-today` checks it pixel-exact on the emulated screen with scripted taps; the device run log gives the touch latency and per-frame draw times |
| ✨ Modernize | Retained widget model with dirty tracking and a frame-at-a-time command list (as modern compositors batch for the GPU), instead of GDI's immediate calls; focus ring and D-pad navigation everywhere |
| 🗄️ Legacy support | WM's structure kept by the owner's choice: Start top left, two softkeys, Today screen; old apps' own title bars come with GWES later, not here |
| ⚡ Better / faster | The display controller blends wallpaper and interface for free every frame; only dirty rectangles are drawn and flushed; the GPU backend slots in without touching the shell |

## Hardware limit or choice?

- WM's GDI draws through immediate calls, one operation at a time: a
  software choice. Measured cost under WM: ~1.7 ms extra per waited
  operation. Replaced by the per-frame list.
- WM composes the desktop into one 16-bit surface: a choice. Replaced by
  scan-out blending of two planes (item 60, proven on the device).
- The 480 x 640 portrait screen, the stylus and the button set: hardware;
  kept.

## Alternatives considered

1. **Immediate drawing calls behind a trait** (`fill`, `blit`, ... called
   as the shell walks its widgets). Simplest, and the CPU path is the
   same speed. Rejected: a GPU backend would have to guess frame
   boundaries or wait per call, which the WM measurement shows is the
   expensive part.
2. **A full scene graph kept by the renderer** (the renderer owns the
   widget tree and diffs it). Most GPU-native, but couples the renderer to
   the widget model and makes the CPU path heavier. Rejected for now; the
   per-frame list can grow into it if the MBX work shows a benefit.
3. **The shell on the kernel from day one.** Real architecture at once,
   but input services and font handling on the kernel come first: about
   2-3x longer to the first screen. Owner chose the library route; the
   library is the same code either way.

## Security

> **Correction (note added 2026-10-13):** docs/security.md has no row yet
> for rp-today's "Back to loader" (or its "Align screen") entry, though
> the code has landed (shell/core/src/lib.rs, `DEV_MENU`).

No outside input is parsed on the device in this ADR: fonts, icons and the
wallpaper are built in at compile time. Touch coordinates and key codes are
clipped and range-checked before hit-testing; a `Frame` is bounded (fixed
capacity, a full list ends the frame early rather than allocating) and
every rectangle is clipped by the renderer as `rp-gfx` already does.
`rp-today` is a test image (loader-only, like every test image) and its
"Back to loader" entry never exists in the shipped shell
(docs/security.md row added with the code).

## Budget

`rp-today` is a test image: its budget is set from its first build and
ratchets (owner's rule). `rp-shell` is OS code: its code size and RAM are reported from the first
build, and a budget is proposed to the owner before it ships in an OS
image. That proposal must settle what the budget doesn't yet say: the
1.2 MB ARGB8888 interface layer (item 60) is in no line of
docs/memory-budget.md, and whether it is pinned (it is read on every
flush; the pin rules are ADR-0019's) is undecided.

## Consequences

- A tappable Today screen on the device in about 2-3 sessions.
- The 50 ms touch promise gets its first real number.
- Item 69's GPU renderer has a defined target: draw a `Frame`.
- Later ADRs: wallpapers and fonts loaded from the card (parsers, so
  security work), the shell on the kernel's services, the weather service.
