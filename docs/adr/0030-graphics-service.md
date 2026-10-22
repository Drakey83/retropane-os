# ADR-0030: The graphics service: one owner for the 2D engine and the 3D core

- **Status:** Accepted (2026-10-15: G1 option A by the owner's vote; G2-G4
  as recommended, under the owner's standing go-ahead)
- **Date:** 2026-10-15

## In plain words

Two renderers now draw on the 2700G: the 2D renderer (ADR-0023: the
interface, glass, text, images) and our own 3D renderer (ADR-0026: quads
with any shape, textures, blending, depth). Both draw into the same screen
picture, and both reset the same chip when something hangs. ADR-0026 D6
decided that **one owner**, the graphics service, drives both. This ADR
says how that owner works. One question is the owner's to decide (G1:
who draws where on the screen); the rest has a recommendation.

## Context

- The 2D renderer rebuilds each changed rectangle from the wallpaper up,
  then flips between two pictures (ADR-0023).
- The 3D core writes whole 8 x 16 tiles from its own tile buffer: it never
  reads what the picture held (measured: uncovered pixels come out white).
  So 2D and 3D on the same pixels overwrite each other unless the owner
  orders them.
- The 3D renderer's direct path doesn't use the 2D engine's command FIFO;
  only the tile accelerator (for games, later) shares it (ADR-0026 T8,
  not yet tested).
- A 3D recovery resets the whole MBX, the 2D engine included.
- The 3D clock is gated (D8); the core keeps its state across it (T9).
- 2700G memory beyond the fixed regions is one pool (ADR-0027), which the
  owner hands out per client with quotas (ADR-0027 watch item).
- IPC today moves 3 words a call and no pages (ADR-0016): the service
  can't yet run as its own process with clients elsewhere.

## Decisions

### G1. Who draws where on the screen: **A, areas (owner, 2026-10-15)**

**Amended 2026-10-19 by [ADR-0034](0034-compositor.md) C5:** layers replace
areas; text and glass may sit over 3D content, the 3D core composing them
(priced per tile). The single GPU owner and the per-client quotas stay.

| | **A: areas (recommended first)** | B: 3D under 2D | C: 3D over 2D through a texture |
|---|---|---|---|
| How | Each frame, every tile-aligned area of the picture belongs to one engine: the 3D scene owns its rectangles (an animation, a transition, a rotating icon); the 2D renderer leaves them out | The 3D scene renders first; the 2D renderer then draws its commands over those areas, starting from the 3D result instead of the wallpaper | The 2D result is copied into a texture that the 3D scene draws (e.g. a page turning) |
| Text or glass over 3D | No (beside it only) | Yes | Yes (inside the 3D scene) |
| Cost | None extra | Every 3D re-render also redraws the 2D commands over it | A copy per frame plus twiddling on the CPU (the 2D engine can't twiddle; linear textures untested) |
| Unknowns | None | None in hardware; the 2D renderer needs a "start from the picture" mode | Linear (stride) textures, a device test |
| Work | Small: an area list both renderers respect | Medium: changes in the 2D renderer | Large |

Why A first: it needs nothing new from the hardware or the 2D renderer and
covers what the interface wants 3D for first (transitions, animated or
rotated elements, effects in their own area). B can follow when something
needs text over a 3D effect; C when a transition needs the old screen as a
picture.

### G2. One queue (recommended)

The owner runs one job at a time: a 2D frame (its batch and fence) or a 3D
render. A 3D render starts only when the 2D engine is idle; a 2D frame
waits for a 3D render in flight. After any 3D reset the 2D engine's stream
restarts. Later, once a device test shows it's safe, the two engines can
run at the same time on different areas (they are separate hardware).

*Device result, 2026-10-21 (`rp-gpubench` run FT, the performance gate's
P8; a measurement, not a decision):* with the two at once on different
areas the 3D render got ~220 wrong pixels in 307,200 (450+ against 2D
fills) while saving 9% of the time; alone it repeats exactly. So as set
up now the turns stay. Open: the mechanism, and whether a memory
controller setting or separate memory areas make overlap safe
(`docs/hw/2700g.md`).

### G3. Clients get handles, not addresses (recommended)

A client's textures and surfaces are blocks from the pool, under a quota
per client; its commands name its blocks by handle, never by address, so
"is this address the client's?" is answered by construction, not checked
each time. A refused allocation is reported to the client; nothing is
evicted behind its back (ADR-0027 watch item).

### G4. Where it runs (recommended)

A library (`services/graphics`, `rp-graphics`) now, used by the test images
and the shell in one process; hosted in the display driver host (ADR-0016
D5) when IPC can move pages, with the same interface.

## Built (2026-10-15, `services/graphics`)

`rp_graphics::Graphics` owns the 2D renderer (and through it the register
window and memory), the pool and the one 3D scene. A frame goes:

1. **2D** draws its changed rectangles into the picture not on screen; the
   2D renderer holds the flip (`Mbx2dRenderer::hold`, new).
2. **3D** renders its area into the same picture: its own changed tiles,
   the tiles the other picture missed, and the tiles 2D just drew over
   (2D draws under the area like anywhere else; the scene draws over it).
3. **The flip.** Nothing half-drawn or stale reaches the screen.

A frame with only 3D changes renders into the picture not on screen (it
holds the same 2D frame) and shows it; a frame with no change does
nothing. Closing the scene gives the area back to 2D, redrawn whole. Each
step is a `Step::Wait(time)` for a tickless caller, never a spin.
Host-tested on both engines' models at once (`services/graphics`
tests). Next: the shell's Today screen on the service, then a device run.

Later (B in G1): 2D could leave out the area instead of drawing under it
(saves the 2D engine's work there), and text over 3D.

**3D state lent, not held (2026-10-17, stack audit).** The scene and the
64 texture blocks (`ThreeD`, about 5 KiB of the service's 5.4) are kept by
the program and lent to `Graphics::new` only by programs that use 3D; a
2D-only one (the Today screen) passes none, and 3D requests are then
refused with `Error::No3d`. Nothing changes for a program that lends it.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Host-tested against both engines' models together; the areas (G1) and the queue (G2) are where later iterations add overlap and concurrency |
| ✨ Modernize | One GPU owner and handle-based clients, as modern drivers work; Dell maps the chip into every GLES program |
| 🗄️ Legacy support | GLES games and the CE personality's GDI become clients of the same owner |
| ⚡ Better / faster | No copies between engines (A); resets coordinated, so a 3D hang never leaves the 2D engine wedged |

## Hardware limit or choice?

- 3D tiles written whole: **silicon** (measured; no "load tile" found).
- One FIFO for the 2D engine and the tile accelerator: **silicon**
  (ADR-0026); the direct 3D path doesn't use it.
- A reset resets both engines: **silicon** (one reset register).

## Alternatives considered

- **Each renderer owns its own bus** (as the test images do today): two
  owners of one chip; resets and the clock would race. Rejected by D6.
- **Everything through the 3D core** (the 2D renderer retired): the 3D core
  can't read the picture, so glass over the wallpaper would need the
  wallpaper as textures; the 2D engine does that work well already.
