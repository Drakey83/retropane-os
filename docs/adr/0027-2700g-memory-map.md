# ADR-0027: The 2700G memory map

- **Status:** Proposed (2026-10-15). The regions ADR-0023 already uses are
  unchanged (accepted there); **the 3D regions and the pool are this ADR's
  proposal and need the owner's approval** (an OS memory budget). The map
  and allocator are built (`rp-vram-map`) so the proposal can be checked.
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

## The map

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
