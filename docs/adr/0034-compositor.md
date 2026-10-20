# ADR-0034: The compositor: layers, frame modes and the frame budget

- **Status:** **Accepted (owner, 2026-10-19): C1 A, C2 A (with draw-time
  dithering), C3 A, C4 A, C5 A**, with the owner's review fixes (Gap 1,
  the pan-to-partial seam; Gap 2, the price table bounded) written in
  below and in [docs/design/compositor.md](../design/compositor.md).
  **Amends ADR-0023** (C2: 16-bit opaque pictures) **and ADR-0030 G1**
  (C5: layers replace areas). Implements ADR-0022 point 3 as amended.
- **Date:** 2026-10-19

## In plain words

The screen is a stack of layers; every frame the compositor picks the
cheapest way to show it: nothing (still), a repoint of the display (a
whole page moving), the display blending its two planes (glass over a
moving page), or the 3D core redrawing only the changed tiles (sheets,
fades, clouds, the carousel). A frame budget priced from the X51v's
measured costs keeps every frame on time, and effects that don't fit
degrade in an order chosen in advance. The design, part by part, is
[docs/design/compositor.md](../design/compositor.md); this ADR records the
decisions.

## Decisions (owner, 2026-10-19)

| | Chosen | Why (the owner's review) |
|---|---|---|
| C1. Pictures for layers the 3D core moves | **A: drawn by the CPU in SDRAM, reordered there, copied by DMA** (a strip 3.0 ms, a whole layer ~71 ms once) | The direct write is 14 ms quicker but holds the CPU the whole time; DMA leaves it 79% free, and responsiveness is the priority |
| C2. Depth | **A: 16-bit for opaque pictures (RGB565), 32-bit only for glass with smooth gradients; opaque gradients dithered when the CPU draws them** (once, so free: a weather sky in RGB565 without banding) | Half the memory and uploads; the 3D core's 16-bit target saves 0-3 ms |
| C3. Full-screen 3D effects at today's clock | **A: designed as moving panels or short (~0.3 s) effects at ~20-35 fps**; 60 for everything else | Honest to the measured cost (a full-screen textured layer 25-29 ms); SPB's own effects weren't all 60 fps either |
| C4. The video plane | **A: the panned page, its wallpaper drawn into it**; the graphics plane the glass and chrome | Every pan stays a free repoint at 60 Hz; a wallpaper change redraws the page pictures once |
| C5. Text and glass over 3D content | **A: yes, as layers** (replaces ADR-0030 G1's areas) | Sheets over the carousel and the clock over the weather sky are the SPB look's defining moments; the GPU's single owner and the per-client quotas don't change; translucency is priced per tile, so it's bounded |

## The review's fixes

**Gap 1, the seam between a pan and a partial render.** The 3D core
writes each tile whole from its own inputs and can sample only textures;
it can't draw over a row-order picture (a panned page) as a background.
So:

- **While a page pans, changes over it go through the graphics plane**
  (mode 3, pan under glass): a sheet, a dim, glass. A sheet sliding up or
  down is a repoint of the graphics plane over a taller glass picture
  (free); one appearing or changing is drawn into the glass picture by the
  CPU or the 2D engine. One translucent layer this way; a second drops its
  translucency for the pan (ADR-0022 point 3).
- **The 3D core composes only over content that exists as textures** (the
  carousel's panels, the weather sky, a page prepared as a texture). A
  page that should take a 3D effect after it stops is uploaded as a
  texture first (~71 ms, a static cost: the effect starts after it, or is
  prepared ahead when it can be foreseen), or the effect stays on the
  planes.
- The mode for each frame follows from where the changed layers live:
  plane pictures → modes 2-3; textures → mode 4; never a 3D render over a
  plane picture.

**Gap 2, the price table bounded.** For v1 the table is **fixed** (refit
from `rp-gpubench`'s measurements: quality audit C20) and the measured
render time of every frame is **logged only**. A drift is reported, never
acted on, until there is evidence that the fixed table is wrong; any later
self-correction acts only on sustained deviation, with its range clamped.
One slow frame (a stall from elsewhere) can't inflate the prices.

**C2's dithering, one more case.** Gradients the **3D core** makes on a
16-bit target (a fade, Gouraud colours) may band too, and whether the core
dithers is unmeasured: until measured, 3D-made gradients target 32-bit
pictures (or are drawn by the CPU, dithered, as layers).

**C4's pages:** each tall page holds its own copy of the wallpaper, so
pages are **built on demand and dropped** when they're off screen and not
about to be shown (ADR-0027's pool, the client's quota).

## Measured since (`rp-gpubench` run E, 2026-10-20)

A full-screen layer into a texture now takes **34.8 ms** with the reorder
by table and the CPU writing straight in (was 71 by C1's path), or 41.0 ms
reordered beside the DMA (the CPU free only 11.6 ms of it: the reorder and
the DMA share SDRAM). That narrows C1's trade: the DMA path no longer
leaves the CPU 79% free. **Not re-decided:** C1 stands until the owner
looks at it again with these numbers. 8-bit layers take half the upload
and draw faster than RGB565 (22.7 ms vs 27.2 full screen); PVRTC a quarter
or an eighth; text as one 2D command a glyph moves 11x fewer bytes but
takes as long as pixels.

## Process

- The pan path is **proven** when its device run passes, and **done** only
  when the emulator models the repoint (ADR-0022 point 3): the fork models
  it from rp-v0.29.0 (`plane-repoint`, `emu-gpubench`'s W4 check),
  released and pinned 2026-10-20: the build order's step 1 is done once
  its device run passes.
- C2 amends ADR-0023 (its screen pictures were 32-bit) and C5 amends
  ADR-0030 G1 (areas): both ADRs carry a pointer here.

## Consequences

- The graphics service (ADR-0030) gains the layer list, the frame loop on
  VEVENT0 and the price table; the 3D renderer (ADR-0026) gains band
  rendering behind the scan line and rectangular textures (measured).
- The 2D renderer (ADR-0023) draws opaque layers in RGB565 with ordered
  dither at draw time, glass in ARGB4444 + dither or ARGB8888.
- Open measurements that feed the design: the GPU bus (`rp-cs3bus`, approved),
  whether the 3D core dithers on 16-bit targets, an RGB565 page panned on
  the video plane under shaped glass (the combination not yet run).

## Build order

As in [compositor.md](../design/compositor.md) "How it would be built":
the frame loop with still and pan (device-proven, emulator-modelled), pan
under glass, the price table and partial mode, the upload path, then the
first showpiece. Each step: a design note, a host test, an emulator
check, a device run on the owner's go.
