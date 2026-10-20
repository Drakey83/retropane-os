# The compositor (option D): the design (2026-10-19)

**Voted (owner, 2026-10-19): C1-C5 all A, with the review's fixes; the
decisions are [ADR-0034](../adr/0034-compositor.md).** What follows was the
review copy, with the fixes folded in (marked).

**Originally a design to review, not a decision.** The owner chose option D in
[ui.md](ui.md) ("3D acceleration for the whole UI") and signed off its rule
in [ADR-0022](../adr/0022-graphics-stack.md) point 3 (amended 2026-10-19).
This page says how the compositor would work, part by part, from what the
X51v measured, and lists the choices left for the owner's vote (C1-C5 at
the end). Nothing is built on it before the vote; the votes then become an
ADR (or amend ADR-0030).

## In plain words

The screen is a stack of **layers**: the page you're looking at, a sheet
sliding over it, a dimmed backdrop, glass, the status bar. The compositor
decides, each frame, the cheapest way to show the stack:

- **When a whole page moves** (a swipe, a flick through a list, a screen
  sliding in), the display is pointed at a different place in a taller
  picture. Nothing is drawn; it runs at the panel's full 60 frames a
  second. Measured: the repoint takes effect at once, so it's written in
  the gap between frames, and the move looked smooth (runs A and C).
- **When part of the screen changes** (a sheet sliding up, a fade, the
  weather's clouds, the carousel's panels), the 3D core redraws only the
  tiles that changed, under a **frame budget** so a frame is never late.
- **Glass over something that moves** is blended by the display itself:
  the moving page on one plane, the glass on the other, mixed as the
  panel is fed, every frame, free (measured smooth in run C).
- **Everything is drawn once and kept:** the CPU draws text and pictures
  into layers when their content changes, never to move them.

## What the hardware gives (measured on the X51v, 2026-10-19)

| Fact | Number | Source |
|---|---|---|
| The panel's frame (every program runs it at 60 Hz) | **16.6 ms** | `Display::adopt`, display calibration C3 |
| A plane repoint (`GSCADR`, `VSCADR`) | takes effect **at once**, mid-frame: write it in the blanking (VEVENT0, line 643) | gpubench W1, W4 |
| Two planes mixed at scan-out | graphics over video, per-pixel or global alpha, free; the video plane takes RGB565 / ARGB8888 / ARGB4444 / ARGB1555 / RGB888 as well as YUV | rp-blend, gpubench run D |
| A bare 3D tile (8 x 16 px) | ~5.3 us | gpubench G1 |
| A textured tile, 1:1 | ~11.3 us (PVRTC ~9, a magnified small texture ~6.5) | G1 |
| Each translucent layer over a tile | ~+8 us (a full screen: ~20 ms) | G2 |
| Hidden opaque layers | almost free (8 stacked: +13 ms over 1) | G2 |
| A full-screen textured layer | **25-29 ms**: more than a frame | G1 |
| A list row (480 x 16) under three translucent layers | **1.6 ms** | G3 |
| A layer's upload, CPU, row order | 18.5 ms (16-bit), 33 MB/s | G5 |
| A layer into texture order | 57 ms by the CPU; or 37 ms reordering in SDRAM + 34 ms DMA (the CPU 79% free) | G5 |
| A strip (480 x 48) | 1.4 ms in row order, 3.0 ms in texture order | G5 |
| Textures | powers of two, rectangles allowed (480 x 640 in 512 x 1024); twiddled only (no row-order mode found) | G4a, run B |
| Rendering in step with the scan | four bands behind the scan line, or two pictures flipped at VEVENT0: both smooth | W2, W3 |

The GPU-to-CPU bus (`MSC1`) may get faster (the bus experiment,
`rp-cs3bus`, approved, not run): uploads would shrink with it.

## The parts

### 1. Layers

A layer is a picture plus how it's shown:

- **content:** a picture the CPU (or the 2D engine) drew, in one of two
  forms: a **plane picture** (row order, in 2700G memory, any size, the
  form the display and the 2D engine use) or a **texture** (twiddled,
  power-of-two sides, the form the 3D core samples). A layer is made in
  the form its use needs (C1);
- **placement:** position, and for 3D layers a transform (scale, turn,
  tilt) and alpha;
- **kind:** opaque, translucent (per-pixel alpha), or tint (one colour,
  one alpha);
- **owner:** the client (the shell, an app, a legacy program's window)
  under its memory quota (ADR-0030 G3).

### 2. Every frame, at VEVENT0

The compositor wakes on the frame event (an interrupt, measured), looks at
what changed since the last frame, and picks a **mode** for the frame:

1. **Still:** nothing changed. Nothing is drawn; the CPU sleeps.
2. **Pan:** a whole page moved (and nothing else did, or only things that
   ride with it). The plane is repointed in the blanking. Cost: one
   register write.
3. **Pan under glass:** a page moved under a translucent layer that
   stays. The page on the video plane (RGB565), the glass on the graphics
   plane (per-pixel alpha): both repointed or held, the display mixes
   them. Cost: one or two register writes. One translucent layer this
   way (the display has one blend stage); a second falls back to dropping
   its translucency for the pan (ADR-0022 point 3's rule).
   **The seam (review, Gap 1):** while a page pans, every change over
   it goes through the graphics plane (this mode): the 3D core can't draw
   over a row-order picture. A sheet sliding up or down is a repoint of
   the graphics plane over a taller glass picture (free).
4. **Partial:** some layers moved or changed, the rest held, **over
   content that exists as textures** (Gap 1: never a 3D render over a
   panned plane picture; a page that should take a 3D effect is uploaded
   as a texture first, ~71 ms, or the effect stays on the planes). The 3D core
   renders the changed tiles (ADR-0026's dirty tiles), into the visible
   picture in bands behind the scan line (no second buffer, measured) or
   into a second picture flipped in the blanking (when the change is large
   enough that bands would run into the scan).
5. **Full:** everything changed (a theme switch, a 3D transition across
   the whole screen). The frame is allowed to take two or three frame
   times, and the transition is designed for ~20-35 frames a second, or
   shaped as moving panels (C3).

### 3. The frame budget

Before rendering, the compositor prices the frame from the measured table
(not the renderer's own estimate, which quality audit C20 found low):
tiles x (5.3 us + 6 us if textured + 8 us per translucent layer above).
If the price is over ~14 ms (the 16.6 ms frame less the CPU's own work
and a margin), it **degrades in a fixed, designed order** (C3), and logs
it, rather than running late:

1. effects marked optional (a blur, a reflection, the clouds' second
   layer) are dropped for the frame;
2. translucent layers over moving content lose their translucency for the
   motion (the rule above);
3. the animation takes more frames (it runs to its end on time, with
   fewer steps: never stutters).

The measured render time of each frame (the render-done interrupt) is
**logged only** in v1 (review, Gap 2): the table is fixed (refit from
the measurements, quality audit C20), a drift is reported, never acted
on, until evidence says the table is wrong; any later self-correction
acts only on sustained deviation, clamped.

### 4. Memory (16 MiB of 2700G memory)

| What | Size |
|---|---|
| The visible picture(s): one, or two when a full mode needs a flip | 600 KiB each at 16 bits (1.2 MiB at 32) |
| A tall page for panning (two screens, RGB565) | 1.2 MiB |
| The glass layer (480 x 640 ARGB4444, or 8888 for smooth gradients) | 600 KiB (1.2 MiB) |
| Layers as textures (512 x 1024 at 16 bits, or PVRTC: 256 KiB) | 1 MiB each |
| The 3D parameter memory and region array | 1 MiB + 16 KiB |
| Glyph atlases, image cache, fence (existing map) | ~0.8 MiB |
| Left for layers and textures, in the pool | ~6.7 MiB |

So about six full-screen layers fit at once, plus small ones. Each tall
page holds its own copy of the wallpaper (C4), so **pages are built on
demand and dropped** when off screen and not about to be shown; layers are
made for what is on screen or about to be, and dropped after (ADR-0027's
pool, quotas per client).

### 5. Where content comes from

- **Text and interface pictures:** drawn by the CPU (crisp, at their
  size), into SDRAM, then copied to 2700G memory by DMA (the CPU free):
  row order for plane pictures (a strip 1.4 ms), twiddled for textures (a
  strip 3.0 ms; a whole layer ~71 ms, a static cost, never per frame).
- **Fills and copies:** the 2D engine, in 2700G memory (plane pictures
  only: it can't write texture order).
- **Long lists:** a window of fixed height (two screens) over the
  content, rows drawn ahead of the scroll and the window re-based by one
  2D copy (~4.5 ms) when the scroll nears its end (ADR-0022 point 3).
- **Legacy programs:** each draws into its own layer (a plane picture),
  shown like any other.

### 6. What becomes of today's design

- **ADR-0022 point 3** (amended): this is its implementation.
- **ADR-0023** (the 2D renderer, the interface plane ARGB4444 over the
  wallpaper on the video plane): the wallpaper and the panned page now
  compete for the video plane (C4).
- **ADR-0026** (the 3D renderer): unchanged, plus rectangular textures
  (measured), the price table (C20) and the band rendering.
- **ADR-0030 G1** ("areas": 3D only in its own rectangles, nothing over
  it): replaced by layers. Text and glass may now sit over 3D content
  because the 3D core composes them as layers (C5).

## The choices for the owner's vote

**C1. How layers that the 3D core moves get their pictures.**
- **A (recommended): the CPU draws them in SDRAM, reorders them there
  and copies them by DMA** (a strip 3.0 ms, a whole layer ~71 ms once).
  Plus: the CPU mostly free, any content. Minus: a big layer takes ~4
  frames to appear the first time (shown by a fade-in, or prepared ahead).
- B: the CPU writes them straight into 2700G memory in texture order (57
  ms, the CPU busy). Simpler; slower.

**C2. Screen and layer depth.**
- **A (recommended): 16-bit for pictures that are opaque (RGB565), 32-bit
  only for glass with smooth gradients (ARGB8888; ARGB4444 + dither where
  it looks the same).** Half the memory and upload time; the 3D core's
  16-bit target saves 0-3 ms (G1). Minus: an ADR-0023 amendment (its
  screen pictures are 32-bit today).
- B: keep 32-bit everywhere. Simplest; twice the memory and uploads.

**C3. What a full-screen 3D effect does at today's clock** (a whole page
turning in 3D, a cross-fade of two full screens).
- **A (recommended): design those transitions as moving panels or short
  (~0.3 s) effects at ~20-35 frames a second**, and keep 60 for
  everything else. Plus: honest to the measured cost. Minus: a few
  effects aren't as fluid as a phone's.
- B: avoid full-screen 3D effects entirely until a faster GPU clock is
  proven.

**C4. The video plane: wallpaper or panned page?**
- **A (recommended): the panned page's picture includes its wallpaper**
  (drawn once into the tall page), the video plane is the page, the
  graphics plane the glass and chrome. Plus: the cheapest pan. Minus: a
  wallpaper change redraws the page pictures (once).
- B: the wallpaper stays on the video plane (ADR-0023) and pages are 3D
  layers over it. Minus: every pan becomes a partial render (most tiles
  change): not 60 Hz at today's clock.

**C5. Text and glass over 3D content (replacing ADR-0030 G1's areas).**
- **A (recommended): yes, as layers**, the 3D core composing them (the
  price table says when). Needed for sheets over the carousel, the clock
  over the weather sky.
- B: keep areas (3D beside text, never under it): simpler, but most of the
  SPB look's moments need text over motion.

## How it would be built (after the vote)

1. The frame loop on VEVENT0 with the still and pan modes (planes only),
   proven on the device with a scrolling list.
2. Pan under glass (video + graphics planes), the device judging it.
3. The price table and the partial mode (the 3D core, bands behind the
   scan), with the degrade order; the emulator modelling what it can
   (the flips, the pictures; not the times, docs/hw/emulator.md).
4. The upload path (SDRAM, reorder, DMA) with its strip updates.
5. The first showpiece on it (the carousel or the weather sky), judged
   on the device.

Each step: a design note, a host test, an emulator check, then a device
run on the owner's go.
