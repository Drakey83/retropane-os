# ADR-0023: The MBX 2D renderer

- **Status:** Accepted (2026-10-14, by the project owner: option B, one
  opaque full-colour picture composed by the engine, double-buffered).
  Stride 0 and 32 bpp scan-out are measured by `rp-mbx-blend` B6 before
  the renderer runs on the device.
- **Date:** 2026-10-14

## In plain words

Today the CPU draws the whole interface into a 1.2 MiB picture in main
memory, then copies the changed parts to the screen chip (the 2700G).
That costs 30-70 ms after a tap, and the stylus isn't read while it
happens. The 2700G's 2D engine can now do almost all of that drawing
itself: fills, copies, and since the 2026-10-14 runs, **blending images
with transparency** (docs/hw/2700g.md). This ADR says how
`Mbx2dRenderer` uses it, so the CPU only describes each frame.

One decision is yours: **where the final picture is put together.**

- **A (today's design, your 2026-10-12 choice):** the wallpaper sits in
  the chip's video plane; the interface is a see-through ARGB4444 layer
  on top, mixed by the chip as it lights the screen. The 2D engine would
  draw into that see-through layer. **Problem:** the engine can't dither,
  so ARGB4444 drawn by it shows the banding you rejected on 2026-10-12;
  and blending onto a see-through layer needs maths the engine doesn't
  have (the colour of a half-clear pixel under a half-clear one needs a
  division), so text and icon edges over glass come out slightly wrong.
  Whether the engine even writes an ARGB4444 destination is unmeasured.
- **B (recommended): one opaque, full-colour picture, composed by the
  engine.** The wallpaper is kept in the chip's memory as a plain RGB
  picture; for each changed rectangle the engine copies the wallpaper in,
  then draws the interface over it. Every blend is then onto an opaque
  pixel, which is exactly what the engine does (measured exact). The
  screen shows XRGB8888: no banding, no dithering, the "WAY smoother" look
  from rp-blend. Two such pictures are kept, and the screen switches
  between them at the interrupt we found (the moving VEVENT line), so no
  half-drawn frame is ever seen.

| | A: see-through layer over the video plane | B: one opaque picture (recommended) |
|---|---|---|
| Look | ARGB4444 bands (engine can't dither) | full 24-bit colour, smoothest |
| Exactness | glass edges approximate; dest alpha behaviour unmeasured | every blend exact, checkable pixel for pixel against the CPU |
| Chip memory | 600 KiB layer + 450 KiB YUV wallpaper | 2 x 1.2 MiB pictures + 1.2 MiB wallpaper = 3.6 MiB of the ~15 MiB free |
| Main memory (SDRAM) | frees the 1.2 MiB CPU layer | frees the 1.2 MiB CPU layer |
| Per-frame cost | engine draws the interface only | engine also copies wallpaper under each changed rectangle (~9 ms for the whole screen at 32 bpp, a few ms for a typical change) |
| Tearing | drawn in place on the visible layer | none: two pictures, swapped at a chosen scan line |
| Video playback later | wallpaper occupies the video plane | the video plane is free for video |
| Unknowns before building | ARGB4444 destination, destination alpha maths | 32 bpp scan-out bandwidth with the engine busy (B5 measured a 16 bpp screen) |

What B gives up: composition at scan-out was free per frame; B pays an
engine copy of the wallpaper under each changed rectangle. That copy runs
on the engine, not the CPU, and only where something changed.

## Context

- **Measured on the device (docs/hw/2700g.md):** fills 3.1 ms and copies
  4.8 ms per full 16 bpp screen, queued in microseconds; negative strides
  (one-command scrolls); colour keys; header bit 17 blends with source and
  destination factors from the blend word, **exact**; ARGB8888 (code 12)
  and ARGB4444 (code 7) sources with per-pixel alpha; 4 bpp coverage with
  a solid colour (16 levels); format conversion on copies; the colour word
  is xRGB8888 when blending; no completion signal (a fence word is read);
  VEVENT0 interrupts on GPIO85 at any scan line; the engine reads only
  its own 16 MiB; DMA writes 2700G memory at 30 MB/s with the CPU 84%
  free; scan-out kept up through 20 back-to-back full-screen fills.
- **The contract (ADR-0021, `rp-draw`):** a `Frame` = dirty rectangles +
  commands: `Fill` (ARGB, glass = translucent), `HGradient` (left to right
  colours, alpha included), `Outline`, `Image` (ARGB8888 straight alpha,
  static), `Text` (glyph coverage 0-255, colour with alpha). Assets are
  `&'static`, so they can be uploaded once.
- **The reference:** `CpuRenderer` (`rp-gfx`), straight-alpha `over`.
  The emulator's golden images compare against it.
- **ADR-0022 decision 4** puts gradients, ARGB images and glass on a later
  3D renderer; the measurements above move them to the 2D engine (owner:
  this can wait for the 3D core, 2026-10-14; this ADR doesn't depend on
  it: the renderer picks per command, so the 3D core can take commands
  over later).

## Decision

1. **Composition: B** (the owner's choice, 2026-10-14; A is described
   above and rejected: banding without dithering, inexact edges). The
   graphics plane is XRGB8888, opaque, double-buffered in 2700G memory;
   the wallpaper is an XRGB8888 copy in 2700G memory, made once from the
   decoded JPEG (the video plane is left off and free).
2. **One batch, one fence, one flip per frame.** For each dirty rectangle
   (plus the previous frame's, since the back picture is one frame old:
   "buffer age 2"), in order: copy the wallpaper in, then each command that
   meets it, clipped by the engine's clip rectangle (`0x00000001`, header
   bit 18). Then a fence, then the flip is requested; the VEVENT0
   interrupt at a line just past the bottom of the screen swaps
   `GSCADR` (no tearing, no polling). The CPU sleeps until the
   interrupt.
3. **Each command on the engine, exactly:**
   - `Fill` opaque: engine fill. `Fill` translucent, `HGradient`: the CPU
     writes **one row** of ARGB8888 colours (at most 640 words) into a
     ring of rows in 2700G memory; the engine blends that row over the
     rectangle with **source stride 0**, every line reading the same row
     (stride 0 is unmeasured; if it fails, the row is doubled by copies:
     log2(h) copies). Blend word `0x09100000` (over), header bit 17.
     The CPU computes the row with `gradient_at`, so the result is the
     reference's.
   - `Outline`: four thin fills, as `Fill`.
   - `Image`: uploaded once to an image cache in 2700G memory (ARGB8888,
     by `&'static` address), then blended with header bit 17.
   - `Text`: glyphs uploaded once per (font, colour) into an ARGB8888
     atlas (coverage x the colour's alpha, the colour baked in: 256 levels,
     the reference's exact maths), then blended per glyph. Fallback when the
     atlas cache is full: Dell's 4 bpp coverage path (16 levels).
   Nothing in today's shell needs the CPU to draw pixels; the trait's
   "which commands did you take" (ADR-0022) stays for later commands.
4. **2700G memory layout** (a table in docs/memory-budget.md): two
   1.2 MiB screens, a 1.2 MiB wallpaper, 256 KiB image cache, 512 KiB
   glyph atlases (about 4 colours of the shell's two fonts), a 64 KiB
   row ring, the fence. About 4.4 MiB; the rest stays for program
   surfaces (ADR-0022 decision 2).
5. **Uploads by DMA** (30 MB/s, CPU 84% free, measured) for the wallpaper
   and large images; small glyph uploads by the CPU.
6. **Checked against the reference.** A host model of the engine (the
   blend arithmetic as measured) runs `Mbx2dRenderer`'s command stream and
   must match `CpuRenderer` pixel for pixel on the Today screen's frames;
   the emulator (fork, being taught the blend model) does the same end to
   end. Text through the 4 bpp fallback is compared within one step.
7. **Security (docs/security.md):** the renderer is the graphics service's
   (ADR-0022 decision 6): every rectangle is clipped to its surface, every
   engine address comes from the allocator, never from a program; a frame
   that would overflow the command window is split, never truncated.

## Three questions

- **Clever or copied?** Copied from Dell: the fill and copy words, the
  fence, the coverage text path (kept as the fallback only). Beyond Dell:
  per-pixel blending Dell never used; gradients from one CPU row with
  stride 0; glyphs as exact 256-level atlases; tear-free double buffering
  flipped by a scan-line interrupt (Dell polled the line); no CPU layer.
- **Size:** SDRAM drops by the 1.2 MiB layer (and the 450 KiB YUV
  wallpaper moves to the chip); code is a command encoder plus a small
  cache; the budget is set when it lands in an image.
- **Modern:** tear-free, vsync-paced presentation; 24-bit colour; one
  validating owner of the GPU; the CPU free while drawing.

## Hardware limit or choice?

| Behaviour | Limit or choice | Evidence |
|---|---|---|
| The engine can't dither | Likely hardware (no dither bit found in the command set) | (A); B avoids needing it |
| Blending onto a see-through destination needs a divide | Hardware (factors only multiply and add) | measured factor set |
| Text at 16 levels (Dell) | Choice: the engine blends ARGB8888 at 256 | measured |
| Polling the scan line for flips (Dell) | Choice: VEVENT0 interrupts at any line | measured |

## Alternatives considered

1. **Keep the CPU renderer, engine only for scrolls.** Proven; keeps
   30-70 ms taps and the 1.2 MiB SDRAM layer. Rejected as the end state.
2. **A (above).** Banding or CPU dithering, inexact edges, two unknowns.
3. **B with RGB565 pictures** (half the chip memory, faster copies):
   bands on the glass gradients (no dithering). Kept as the fallback if 32
   bpp scan-out turns out short of bandwidth.
4. **Wait for the 3D core.** Unbounded; the 2D engine already does
   everything the shell draws.

## Consequences

- Before building on the device: one probe run (rp-mbx-blend B6, built
  with this ADR): source stride 0; a 32 bpp screen's scan-out with the
  engine busy; and for option A's record, an ARGB4444 destination's alpha.
- `rp-gfx`'s ARGB8888 layer stops being on the device path; it stays as
  the reference and the fallback.
- The shell (`rp-today`) changes only its renderer and set-up.

## Measured after acceptance (2026-10-14, `rp-mbx-blend` B6)

Stride 0 works; blending onto 32 bpp is as modelled; 32 bpp scan-out keeps
up; a full-screen 32 bpp fill takes 6.2 ms and a copy about 14 ms (a full
redraw's wallpaper copy). **But the panel bands a 256-level ramp:** it
shows 16 or 18 bits, so B's picture needs dithering too (the engine has no
dither bit and the display none known). **Proposed fix (to build):** as
the last step of each redrawn rectangle, the engine adds a 4 x 4 ordered
dither threshold pattern (kept in 2700G memory) with the blend word 0
(source + destination, saturating, measured), and the panel's truncation
does the rest: the same ordered dither the CPU path used, on the engine,
no CPU cost. Its amplitude follows the link depth (565 or 666), which the
owner's look at the ramps (green's bands finer than red's?) decides.

## First device run of the renderer (2026-10-14, `rp-today`)

The owner: "very responsive", no tearing seen; glass bands (no dither yet).
Measured (67 taps): touch to screen median 33 ms, worst 80 ms; the CPU
renderer's were 30 / 71 ms. No faster yet: the CPU waited for the engine
each frame, 32 bpp is half the engine's 16 bpp speed, and every redraw
re-blended the glass over the wallpaper. Stack 12.6 of 16 KiB. Answered in
the renderer (all checked in the emulator, pending a device run):

- **The CPU doesn't wait:** `draw` queues the frame and its fence;
  `Renderer::poll` (new, default "done" for the CPU renderer) flips when
  the fence lands. The next `draw` waits only for an unfinished frame and
  for the VEVENT0 status bit (scan-out off the picture it draws into).
- **Glass drawn once:** fills, gradients and outlines that precede
  everything they'd cover are drawn into a background picture (wallpaper
  included, 1.2 MiB at 0x6C0000), rebuilt only when their hash changes;
  each changed rectangle starts with one copy of it.
- **Dither:** each redrawn rectangle ends with four blits adding a 4 x 4
  Bayer threshold (red, blue 0-7; green 0-3: sized for a 16-bit link,
  harmless on 18) with blend word 0 (add, saturating).
- **Per-frame costs in the run log** (CPU queueing, waiting, queue to
  flip, background rebuilds); the latency now counts to the flip.
- `rp-today`'s stack is 24 KiB (`--defsym=__stack_size`).

**Second and third device runs (2026-10-14, logs
`local/rp-today-gpu-run3.txt`, `-run4.txt`):** the glass "looks very
smooth", "everything felt great" (owner); no stalls, no tearing; the
background was built once. Typical tap (medians, run 4): touch landing to
the filter's verdict **22 ms**, shell frame and CPU queueing **0.8 ms**,
engine to flip **7 ms**; landing to screen 35.5 ms median, 72 ms worst. The
drawing is no longer what a tap waits for: the CPU's share fell to under a
millisecond and the engine's is ~7 ms; the touch filter's settling is
most of the rest (the same under either renderer). Stack 17.4 of 24 KiB.

## A 16-bit wallpaper, side by side (2026-10-17, `rp-today` b2b699f)

The owner's rule for 16-bit tricks: only with no visible loss in looks or
speed, judged by eye on the device. The test image's "Wallpaper compare"
shows the wallpaper alone and flips it, per tap, between full depth and
RGB565 dithered once on the CPU (Floyd-Steinberg); the Today screen with
its glass can be checked at either depth too. One wallpaper per build.

- **The engine widens a 16-bit source by repeating the top bits** (64 of
  64 probe pixels, both runs): what the comparison showed is what a
  wallpaper kept as RGB565 in 2700G memory would look like.
- **The owner saw no difference and no banding** on the black-and-white
  pier ("even with magnified reading glasses") nor on the Milky Way over
  its orange airglow (#5, the smoothest colour gradient of the approved
  wallpapers on hand).
- What it would save: half the wallpaper's 1.2 MiB of 2700G memory, and
  half the bytes read where a frame copies the wallpaper itself (the
  Today screen copies its 32-bit background picture instead, so there
  only the background's rebuild reads it). The speed side isn't measured
  yet: it needs the real RGB565 storage (the test stores the widened
  pixels at 32 bits). Logs: `local/rp-today-compare-pier.txt`,
  `local/rp-today-compare-milkyway.txt`.

## Open questions

1. ~~Owner: A or B?~~ B (2026-10-14).
2. Stride 0 (B6 measures it).
3. 32 bpp scan-out bandwidth with the engine busy (B6).
