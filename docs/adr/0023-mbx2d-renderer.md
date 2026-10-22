# ADR-0023: The MBX 2D renderer

- **Status:** Accepted (2026-10-14, by the project owner: option B, one
  opaque full-colour picture composed by the engine, double-buffered).
  Stride 0 and 32 bpp scan-out are measured by `rp-mbx-blend` B6 before
  the renderer runs on the device.
- **Amended 2026-10-19 by [ADR-0034](0034-compositor.md) C2:** opaque
  pictures are RGB565 (16-bit), with ordered dither at draw time for
  gradients; 32-bit (ARGB8888) only for glass with smooth gradients.
- **Amended 2026-10-21 by the project owner: the scanned-out screen is
  RGB565 with the 2700G's ordered dither** (see "The screen goes 16-bit"
  below). Layers and textures may stay 32-bit.
- **Amendment proposed 2026-10-21, vote waiting:** how the 16-bit screen
  is built (which engine writes it; the pictures' layout). See "Building
  the 16-bit screen" below.
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

## The screen goes 16-bit (2026-10-21, owner's vote)

Runs P2X and P2Y (`docs/hw/2700g.md`, quality audit C28) found that 3D
renders into a 32-bit picture get wrong pixels (~40-57 a full-screen
render, each one the next object's colour) **only while the display
scans out a 32-bit screen**, at Windows Mobile's clock and memory timing.
With the screen scanned out as 16-bit (or stopped), five 32-bit renders
were exact and identical, and RGB565 renders were exact in every run.
Run P2W showed dithered RGB565 ramps smooth to the owner's eye (4 x 4
ordered dither, measured on every 16-bit layout in run P2). Run T: a
16-bit screen makes 3D ~6% and the CPU's reads ~15% faster than a 32-bit
one, and the screen takes half the memory.

**Options put to the owner:** A, keep 32-bit screens and live with the
wrong pixels (the any-Axim rule rules out memory timings beyond Dell's /
Windows Mobile's, so there's no fix that keeps A); B, scan out a dithered
RGB565 screen, with 32-bit layers and textures kept where compositing
needs the precision.

**Owner's vote: B** (2026-10-21). The vote's verification: a device run
that counts wrong pixels over many 32-bit layer renders while the screen
is shown as dithered RGB565, kept as a regression test (C28's proper
fix). P2Y's five clean renders already show it; the long count checks for
rare errors. If it finds any, layers go to RGB565 too and B stands.

The check is built (2026-10-21): `rp-gpubench` run P2L
(`RP_GPUBENCH_LONG_COUNT`), 1000 renders of P2W's picture at 32 bits
with the screen scanned out as P2W's picture dithered to RGB565, each
checked as P2X checks and compared with the first render, between two
controls of 5 renders with the screen at 32 bits (where P2Y found errors:
if the controls find none, the count can't tell anything). Its emulator
check is in `emu-gpubench`; the device run is waiting.

**Verified on the X51v (2026-10-21, run P2L version 3,
`local/rp-gpubench-runP2L3.txt`):** 1,000 32-bit renders under the
dithered RGB565 screen, **0 wrong or differing pixels in 307.2 million**
(the bars and the whole picture against a reference made under the same
screen, never changed); the controls with the screen at 32 bits found 237
and 213 wrong bar pixels in 5 renders each, as P2Y did. Vote B stands with
32-bit layers. (Versions 1 and 2 broke their own comparison with a 2D fill
given an inverted rectangle, quality audit C47.)

Still to design (in the renderer's next step): which engine writes the
final dithered picture (the 3D core has the dither, bit 3 of `0x650`; the
2D engine's isn't measured) and the double-buffer sizes at 16 bits.

## Building the 16-bit screen (proposed 2026-10-21, vote waiting)

The vote above says what the screen is (dithered RGB565). Two things
are left before the renderer changes: **which engine writes the final,
dithered pixels**, and **where the pictures live** in the 2700G's memory.

### What we know and don't (the facts the options rest on)

- **The 3D core dithers** (`0x650` bit 3, run P2, measured): a 4 x 4
  ordered matrix into any 16-bit target, exact and identical render to
  render (P2X). It writes straight into the screen picture already
  (ADR-0026 D5; the graphics service's 3D area, ADR-0030 G1).
- **The 3D core can't read a 2D-drawn picture**: it reads textures
  twiddled only; no row-order texture mode is known (Dell's driver has
  none, `docs/hw/2700g.md`). So it can't "finish" a picture the 2D
  engine composed, short of the CPU twiddling 1.2 MiB a frame.
- **The 2D engine converts formats on a copy** (32 bits to RGB565,
  measured working since 2026-10-13) but **whether it truncates or
  rounds** each channel isn't measured. Blends are "rounded to the
  destination's bits" (measured with coverage, 2026-10-14). **No 2D
  dither bit is known**: four header bits (19, 21, 22, 27) have never
  been tried; the blend word's `[31:28]` and `[19:0]` were tried (B2) on
  one 16-pixel row, checked within one step, which can't show a 4 x 4
  pattern.
- **Today's renderer already dithers on the 2D engine**: it composes at
  32 bits and, last, adds a 4 x 4 threshold pattern (blend word 0: add,
  saturating) so the panel's truncation dithers. With a 16-bit screen
  the conversion copy takes the panel's place, if it truncates.

### Question 1: which engine writes the final screen

- **A (recommended): the 2D engine, through a 32-bit work picture.** For
  each changed rectangle, exactly today's steps into an off-screen 32-bit
  work picture (background copy, commands, the dither pattern added),
  then **one copy converts it into the 16-bit screen**. The previous
  frame's rectangles come forward from the screen on display, now as
  16-bit copies (about a third of a 32-bit copy's time). 3D areas render
  straight into the 16-bit screen with the core's own dither (bit 3).
  The 2D pattern is changed to **the 3D core's measured matrix**, so a 3D
  area's edge meets the 2D screen with the same dither, no seam.
  - Pros: every blend stays at 32 bits, exact against `CpuRenderer`
    (the host model and the emulator keep checking it pixel for pixel);
    the code change is small (a destination switch and one copy); smooth
    gradients as P2W showed.
  - Cons: one more engine pass per changed rectangle (reads 4 bytes,
    writes 2 a pixel; a full screen is estimated at ~10 ms, to measure).
    It needs the conversion to truncate; if it rounds, the pattern is
    shifted half a step (exact either way, but the measurement decides
    which).
- **B: the 2D engine blends straight into the 16-bit screen.** No work
  picture, no extra pass. Every blend rounds to 5 or 6 bits with no
  dither, so glass gradients and text edges over glass band: what you
  rejected on 2026-10-12 and 2026-10-14. Only worth it if the
  measurement finds a hidden 2D dither bit (then it would be A's look
  with one pass less, and comes back to you as its own vote).
- **C: the 3D core draws the whole shell** (ADR-0022 decision 4's later
  path): text, images and glass as textured quads into the 16-bit screen
  with its measured dither, one engine. Cons: a new renderer (glyphs as
  quads: today's scene holds 64), gradients by the core's interpolation
  are no longer exact against the CPU reference, and every changed tile
  is redrawn whole. Not now; it stays open per command (the renderer
  already picks per command).

### Question 2: where the pictures live (2700G memory, `rp-vram-map`)

Today (32-bit): two screens 1.2 MiB each, wallpaper 1.2 MiB, background
1.2 MiB, area cache 1.2 MiB, rows/images/glyphs/dither ~0.9 MiB; the pool
for textures, 3D and video gets 6.75 MiB.

- **L1 (recommended): two 16-bit screens, one full 32-bit work picture,
  wallpaper and area cache at 16 bits.** Screens 2 x 600 KiB; the work
  picture 1.2 MiB (only the engine reads it, never scanned out); the
  wallpaper RGB565 (600 KiB: you saw no difference in the side-by-side,
  2026-10-17, and it's read only when the background is rebuilt); the
  area cache holds finished screen pixels, so RGB565 (600 KiB; an area
  comes back at its own place, so its dither stays aligned). Background
  stays 32-bit (the glass is blended into it). **Pool grows by 1.2 MiB to
  ~7.9 MiB.** Nothing given up: every precision-bearing picture stays 32
  bits.
- **L2: as L1, the work picture a 64-row strip (120 KiB).** Pool grows by
  ~2.3 MiB. Con: a rectangle taller than the strip is done in bands, each
  band sending its commands again (more CPU time queueing, more words
  over the bus at ~120 ns each). A trade-off, so not recommended now; it's
  the next step if the pool runs short (a 3D game and video at once,
  ADR-0027's watch item).
- **L3: as L1, three 16-bit screens.** Draw never waits for a flip
  pending (+600 KiB, back pictures three frames old: more forward
  copies). Today's waits aren't measured as a problem; not recommended
  without a measurement showing they are.

### The measurement before building (short, unattended, no board bits)

`rp-mbx-blend` stage **B7** (seconds, nothing the owner needs to watch;
only 2D engine commands into its own scratch memory, no CPLD or power
bits):

1. **Conversion:** a 32-bit row of every level 0-255 per channel copied
   to RGB565: truncated, rounded, or something else, per channel.
2. **A blend onto RGB565** (B's arithmetic, for the record): how the
   result is rounded.
3. **A 2D dither bit?** A 64 x 64 grey ramp converted 32 to 16 bits with
   each untried header bit (19, 21, 22, 27) alone, then through the
   blend path (bit 17, a plain-copy blend word) with each of the blend
   word's bits `[31:28]` and `[19:0]` alone: a position-dependent pattern
   means a dither. Each try is logged before it's queued, has a fence
   timeout, is checked for writes outside its rectangle, and a stuck
   engine is reset and the stage goes on (as B1-B6).
4. **Timings:** full-screen and 480 x 80 conversion copies, the pattern
   add at 32 bits, a 16-bit forward copy.

Its emulator check joins the `mbx-blend` check; the fork's conversion is then made to match what the device
does (or listed as not modellable in `docs/hw/emulator.md`).

**B7 is built** (`RP_MBX_BLEND_B7=1 cargo xtask load rp-mbx-blend`;
its emulator check passes: every part runs, the blend state is put back,
the image returns by itself). Waiting for the owner's go.

**Votes waiting:** Q1 (A / B / C) and Q2 (L1 / L2 / L3). Recommended:
**A and L1**, with B7 run first (it can change B's case, and sets A's
pattern shift).

## Open questions

1. ~~Owner: A or B?~~ B (2026-10-14).
2. Stride 0 (B6 measures it).
3. 32 bpp scan-out bandwidth with the engine busy (B6).
