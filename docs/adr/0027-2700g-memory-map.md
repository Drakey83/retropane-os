# ADR-0027: The 2700G memory map

- **Status:** Accepted (2026-10-15, by the owner's review; budgets
  approved: the 16 KiB 3D memory manager block at `0x0FC000`, 1 MiB
  parameter memory at `0x100000`, the 6.75 MiB pool at `0x940000`; the
  review checked the map against `rp-vram-map`: no overlaps, ends at
  exactly 16 MiB). The regions ADR-0023 uses are unchanged.
- **Amended 2026-10-21 by ADR-0023's Q2 (L1, the owner's vote):**
  16-bit screens, wallpaper and area cache, a 32-bit work picture; the
  pool grows to ~8.4 MiB at `0x7A0000`. The 32-bit map is kept unchanged
  as `rp_vram_map::bench32` for the test images measured with it.
- **Date:** 2026-10-15

## In plain words

The 2700G has 16 MiB of its own memory. The 2D engine and the 3D core can
read only that memory (measured: the engine can't reach the main RAM), so
every picture, glyph, texture and 3D buffer must live there. Until today
each program kept its own addresses, and two of them had grown onto the
same place: the old Today screen's video plane and the 2D renderer's image
cache both at 6 MiB (the old one was no longer used, so nothing broke).

Now there is one map, in one crate. A test fails if two regions overlap,
run off the end, or break the alignment the chip needs. What isn't fixed
is a pool, handed out at run time.

## The map (since 2026-10-21: ADR-0023 L1)

| From | Size | What | Status |
|---|---|---|---|
| 0 | 1008 KiB | the loader's and Windows Mobile's display planes | kept while the loader may show them |
| `0x0FC000` | 16 KiB | 3D parameter memory manager's block | **proposed** |
| `0x100000` | 1 MiB | 3D parameter memory (objects, lists) | **proposed** |
| `0x200000` | 600 KiB x 2 | ADR-0023's two screens, RGB565 | accepted (ADR-0023 Q2 L1) |
| `0x32C000` | 1.17 MiB | work picture (32-bit: each changed rectangle composed, then converted) | accepted (L1) |
| `0x458000` | 600 KiB | wallpaper, RGB565 | accepted (L1) |
| `0x4EE000` | 160 KiB | row ring | accepted |
| `0x516000` | 256 KiB | image cache | accepted |
| `0x556000` | 512 KiB | glyph atlases | accepted |
| `0x5D6000` | 1.17 MiB | background (wallpaper + glass, 32-bit) | accepted |
| `0x702000` | 7.5 KiB | dither rows | accepted |
| `0x704000` | 600 KiB | area cache, RGB565 | accepted (L1) |
| `0x7A0000` | ~8.4 MiB | **pool**: textures, the 3D per-tile buffer and region arrays, depth, video frames | **proposed** |
| `0xFFF000` | 4 KiB | 2D engine fence | accepted |

The test images keep the map below (`rp_vram_map::bench32`), at the
addresses their device runs were measured at; an image uses one map or
the other, never both for drawing (they overlap).

### The map with 32-bit screens (2026-10-15 to 2026-10-21; `bench32`)

| From | Size | What | Status |
|---|---|---|---|
| 0 | 1008 KiB | the loader's and Windows Mobile's display planes | kept while the loader may show them |
| `0x0FC000` | 16 KiB | 3D parameter memory manager's block | **proposed** |
| `0x100000` | 1 MiB | 3D parameter memory (objects, lists) | **proposed** |
| `0x200000` | 1.17 MiB x 2 | ADR-0023's two screen pictures | accepted (ADR-0023) |
| `0x480000` | 1.17 MiB | wallpaper | accepted |
| `0x5C0000` | 160 KiB | row ring | accepted |
| `0x600000` | 256 KiB | image cache | accepted |
| `0x640000` | 512 KiB | glyph atlases | accepted |
| `0x6C0000` | 1.17 MiB | background (wallpaper + glass) | accepted |
| `0x800000` | 7.5 KiB | dither rows | accepted |
| `0x810000` | 1.17 MiB | area cache | accepted |
| `0x940000` | 6.75 MiB | **pool**: textures, the 3D per-tile buffer and region arrays, depth, video frames | **proposed** |
| `0xFFF000` | 4 KiB | 2D engine fence | accepted |

Why the 3D parameter memory sits at 1 MiB: on the device its pages
misbehaved when it was placed high (`rp-mbx3d` version 14 wrote 4 MiB
lower than asked; version 15 at 6 MiB worked but put objects 6 MiB past
the start), and the game's own sits low (`0x2B4000`). The only space below
2 MiB that ADR-0023 doesn't use is 1-2 MiB. 1 MiB holds a scene of
roughly 8,000 textured triangles (26 words each plus lists); DoomGLES's
frames used about 320 KiB.

### The on-die memory (2026-10-29: R23T, R116)
The first 704 KiB (`0x0-0xB0000`) are faster memory (R23T on the X51v:
a 3D target there renders the textured game frame in 13.04 ms against
15.29 ms in SDRAM, 3.7 us a KiB past the edge), and the safe region ends
with it. Screen 0 fills 600 KiB of it; **the 3D parameter memory (objects
and lists) of an open scene goes in the remaining 96 KiB to the safe
region's end** (`rp_vram_map::LISTS_ON_DIE`, `0x96000`, the owner's pick C
of `docs/research/sheets/r116-use-the-on-die-memory.md`): the video band's
place, which the display doesn't read while the 3D core draws (ADR-0034
amendment P), so time-shared with the planes and kept out of the overlap
check; a frame whose lists outgrow it moves them to `PARAMS` once
(`Renderer::set_spill`, counted); a program showing the video band with a
scene open keeps them in `PARAMS` (`Graphics::lists_on_die(false)`).

## The pool

`rp_vram_map::Pool`: first fit, any power-of-two alignment (the 3D
per-tile buffer needs 128 KiB, textures 128 bytes), free blocks merged
when given back, double frees refused, no heap (32 free blocks at most).
The graphics service will own it and hand out blocks per client, so a
client's commands can be checked against its own blocks (ADR-0026 D6).

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | The map is data with a test; moving a region is one line, checked |
| ✨ Modernize | A GPU memory manager, as every modern driver has, instead of fixed constants per program |
| 🗄️ Legacy support | The loader's and WM's planes stay where they are |
| ⚡ Better / faster | No copies between programs' private layouts; the 2D renderer's addresses are unchanged (same image, byte for byte) |

## Hardware limit or choice?

- Surfaces must be in 2700G memory: **silicon** (the engine's reads,
  measured 2026-10-14).
- The 3D parameter memory low: **likely silicon** (version 14); the exact
  rule is open (ADR-0026, a device test).
- The loader's plane below 1 MiB: **our choice** (the loader's), kept.

## Alternatives considered

- **Keep per-program constants:** how the clash happened; rejected.
- **A buddy allocator:** simpler merging, but rounds every block to a
  power of two (a 1.17 MiB picture would take 2 MiB); rejected for 16 MiB.
- **Everything from the pool, no fixed regions:** possible later; fixed
  regions keep the 2D renderer's proven layout and make the 3D rules (low
  parameter memory) explicit.

## Consequences

- New users of 2700G memory take an address from `rp-vram-map` or the
  pool, never a literal (test images excepted: they own the whole chip).
- The 3D renderer (ADR-0026) gets its per-tile buffer, region arrays,
  textures and depth from the pool.
- **Watch item (owner's review, 2026-10-15): the pool will be contended.**
  6.75 MiB (~8.4 MiB since L1) must hold textures, the 3D per-tile
  buffer and region arrays, depth and video frames together. Double-buffered 480 x 640 32 bpp frames
  alone are ~2.3 MiB; a 3D game and video playback at once will press on
  it. The graphics service arbitrates at run time (per-client quotas, a
  refused allocation reported to the client, never silent eviction of
  another client's surfaces), and its design must say how before a
  second large client exists.
