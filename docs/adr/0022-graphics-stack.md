# ADR-0022: The graphics stack: compositor, GDI layer and the MBX renderer

- **Status:** Proposed (2026-10-13, drafted overnight for the project
  owner's review; nothing here is decided, and the open questions below
  stay open until the owner rules and the measurements are in)
- **Date:** 2026-10-13

## Context

What exists (all measured on the owner's X51v unless marked):

- **The shell's path today (ADR-0021):** `rp-shell` emits a `Frame` (a
  command list plus dirty rectangles); `CpuRenderer` (`rp-gfx`) draws it
  into an ARGB8888 layer in SDRAM (1.2 MiB) and flushes the dirty
  rectangles, dithered to ARGB4444, into the graphics plane in 2700G
  memory. Touch-to-flush: median 30 ms, worst 71 ms (`rp-today`, 107
  touches); redraws stall input sampling.
- **Scan-out composition works (rp-blend, 2026-10-12):** the wallpaper
  sits in the video plane (YUV 4:2:0), the interface in the graphics plane
  with per-pixel alpha, mixed by the display controller at no CPU or
  memory cost per frame.
- **2700G memory is slow for the CPU:** writes 32-41 MB/s even
  write-combined; a full 16-bit screen takes 14.5 ms to write; reading it
  is ~4x slower again (61 ms for a pattern fill that read it).
- **The MBX 2D engine (item 69, 2026-10-13, docs/hw/2700g.md):** fed
  through a 4 KiB command window; solid fills **3.1 ms** per full screen
  (CPU 14.5 ms), 20 fills queued in 36 µs, the CPU free meanwhile; WM's
  own fills 3.2 ms. Copies, colour-keyed copies, clipping, format
  conversion, scaling, rotation, palettes, 1 bpp masks and solid colour
  through a 4 bpp coverage map (anti-aliased text) exist in Dell's
  driver; copies pass in the emulator, the device run is pending.
- **What the 2D engine can't do (A: Dell's driver never does it, and pays
  to work around it):** read system RAM (every address is masked to its
  own memory; the driver copies bitmaps into four staging slots in local
  memory with the CPU, then blits, waiting on a fence per band);
  per-pixel ARGB blending onto the destination; gradients.
- **The MBX 3D core** answers at `0x0FFE4000` (ID read on the device);
  its tile/region and object formats are partly known (docs/hw/2700g.md:
  per-vertex colours, textured objects with blend words, Dreamcast
  PowerVR2 lineage). Its render-target registers are set by a client
  library outside Dell's display driver and are unknown. Not brought up.
- **Plans already written:** DESIGN.md §7.1 (legacy GDI apps and GLES
  apps into one compositor, a backend trait with the MBX 3D core and a
  software rasteriser as the reference); ROADMAP: "compositor/UI and GDI
  designs written GPU-first, with the CPU as fallback", "2D blitter
  driver; GDI and compositor accelerated"; DESIGN.md §8: all three 2700G
  blocks (3D, 2D, video) are required; the software path is the test
  reference, not a shipping substitute.
- **ADR-0021 assumed** the MBX renderer would take the same command list
  with "nothing CPU-only" in it (gradients, alpha masks). The 2D engine
  alone can't honour that for gradients and ARGB alpha; the 3D core
  might.

**The owner's goal (2026-10-13):** the GPU drives the screen, not only so
the CPU can rest and save battery but because it is faster and more
responsive. The likely hardware limit above (the 2D engine reads only its
own memory) does not block that; it decides where the screen's content has
to live (decision 2).

Constraints: 64 MB RAM and non-XIP flash (every SDRAM byte counts:
docs/memory-budget.md), 16 MiB of 2700G memory (about 15 MiB free with the
two planes in it), no FPU, touch answered within 50 ms (design brief),
looks win over RAM when a real trade-off remains (owner, 2026-10-12).

## Decision (proposed: every numbered part is for the owner's vote)

1. **One stack, three layers.** Programs draw into **surfaces**; the
   **compositor** puts surfaces on the screen; a **renderer** executes
   drawing for a surface. Native programs (the shell first) draw with
   `Frame` command lists (ADR-0021); legacy WM programs draw with **GDI**
   calls, translated by a GDI layer into the same operations. Both meet
   the same renderer trait, so legacy support never constrains the
   native design (DESIGN.md §6).
2. **Surfaces live where their renderer can reach them.** Because the 2D
   engine reads only 2700G memory, a GPU-drawn surface lives in 2700G
   memory (RGB565 or ARGB4444, 600 KiB for a full screen); a CPU-drawn
   one lives in SDRAM and reaches the screen by a flush. The surface type
   says which; the compositor never reads 2700G memory with the CPU
   (docs/hw/2700g.md, "the framebuffer is write-only").
3. **The compositor uses scan-out planes first, the 2D engine second, the
   CPU last.** Wallpaper and full-screen video: the video plane (zero
   copies). The interface: the graphics plane, per-pixel alpha over the
   video plane (measured). A full-screen program's surface becomes the
   graphics plane by pointing `GSCADR` at it (a flip, zero copies).
   Windows on the interface plane: 2D-engine copies from their surfaces,
   clipped, colour-keyed where they have a transparent colour. The CPU
   composes only what the engines can't (below).
4. **Renderers behind one trait, chosen per operation, not per frame:**
   - `CpuRenderer` (exists): the reference and the fallback for every
     operation; the emulator's golden images compare against it.
   - `Mbx2dRenderer` (next): fills, copies, keyed copies, clipping,
     anti-aliased text from a 4 bpp glyph atlas kept in 2700G memory,
     1 bpp masks; one command batch and **one fence per frame**.
   - `Mbx3dRenderer` (later, its own ADR): gradients (per-vertex colour),
     ARGB-blended images and glass, scaling and rotation, GLES.
   Operations the active GPU renderer can't do are drawn by the CPU into
   a small SDRAM tile and copied in (the open question below decides
   whether by CPU or DMA). ADR-0021's trait changes from "takes the whole
   frame" to "takes the whole frame and says which commands it took";
   the shell still doesn't change.
5. **The GDI layer is a translator with a software floor.** `PatBlt`,
   `BitBlt`/`StretchBlt` with the common ROPs, `TransparentBlt`,
   `ExtTextOut`, `FillRect`, `Polyline` (horizontal/vertical runs as
   fills) go to the 2D engine on device surfaces; DIB sections and other
   system-RAM bitmaps are staged into 2700G memory first; everything else
   (arbitrary ROP4, diagonal lines, `AlphaBlend` until the 3D core) runs
   on the CPU reference. Correctness first: every GDI call has a CPU
   implementation and a test, and the engine path must match it pixel
   for pixel in the emulator.
6. **Only the graphics service talks to the engines.** The command window
   and the MBX registers are mapped into the graphics service alone
   (ADR-0016 driver host). Programs hand it command lists or GDI calls;
   the service validates every rectangle, surface and address before it
   becomes an engine command, because the engine can write any byte of
   2700G memory, including other programs' surfaces and the planes.
7. **Completion without spinning:** the service queues a frame's
   commands plus one fence and sleeps; it learns the fence landed at the
   next vsync or timer tick (no 2D completion interrupt is known yet:
   open question 5), so no CPU time is spent polling.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Every renderer is checked against `CpuRenderer` pixel for pixel in `cargo xtask emu` (the fork models the 2D engine since rp-v0.12.0; an overlapping copy without its direction bit smears there as on the chip). Per-frame timings (queue, fence, flush) go to the run log, so each backend is measured, not assumed |
| ✨ Modernize | A compositor at all (WM drew straight onto the screen); per-pixel alpha interface over a video-plane wallpaper; anti-aliased text everywhere (WM's ClearType was optional); GPU access confined to one validating service |
| 🗄️ Legacy support | WM programs keep their GDI calls; the common ones run on the same 2D engine WM used, at WM's measured speed (fills 3.1 vs 3.2 ms), the rest on a tested CPU path |
| ⚡ Better / faster | Fills 4.7x faster than the CPU with the CPU free; surfaces in 2700G memory move the 1.2 MiB ARGB8888 layer (and window back-buffers) out of SDRAM; full-screen programs flip instead of copying |

## Hardware limit or choice?

| Behaviour | Limit or choice | Evidence |
|---|---|---|
| The 2D engine reads only 2700G memory | **Hardware (measured 2026-10-14)**: the engine decodes 32 MiB, its 16 MiB mirrored and a constant between; no address reaches system RAM. Dell's driver masks every address to it and pays for CPU staging plus a fence per band | (A) + (P) docs/hw/2700g.md |
| A copy down over itself can't run bottom-up | **Hardware, as far as measured**: bit 23 smears; Dell's driver sends every such overlap to the CPU. Strips (bottom first) work around it on the engine | (A) + (P) docs/hw/2700g.md |
| The CPU reads 2700G memory slowly | Hardware (measured: 61 vs 14.5 ms) | docs/perf.md |
| No ARGB blending or gradients in the 2D engine | Likely hardware for the 2D engine (Dell's driver has `AlphaBlend`-shaped code only for 4 bpp text coverage); the 3D core can blend (its objects carry blend words), so not a limit of the chip | (A) |
| WM has no compositor: programs draw on the screen | Choice (2005 RAM and CPU budget) | WM's design; replaced here |
| WM stages bitmaps with the CPU, synchronously, four slots | Choice | (A); alternatives: DMA, surfaces born in 2700G memory |
| WM's interface is opaque RGB565 | Choice: the planes do per-pixel alpha (measured) | rp-blend |
| 16 MiB of 2700G memory | Hardware | docs/hw/2700g.md address map |

## Alternatives considered

1. **Keep ADR-0021's path for everything (compose in SDRAM, CPU flush),
   use the GPU only for fills and scrolling.** Cheapest, proven on the
   device. Rejected as the end state: every visible change still costs
   CPU writes at 32-41 MB/s (a full ARGB4444 flush is ~600 KiB of them),
   the 1.2 MiB layer stays in SDRAM, and legacy GDI programs get no
   acceleration where WM had it (fills 4.7x slower). Kept as the
   fallback renderer.
2. **Surfaces in 2700G memory, 2D engine only (WM's model plus a
   compositor).** Fast for what the engine does; glass, gradients and
   ARGB images fall back to the CPU, and the shell's look is mostly
   those. Proposed as the next step, not the last.
3. **The 3D core for everything now** (DESIGN.md §7.1's plan: textured
   quads, per-vertex colour gradients, blending). The most capable and
   the plan of record, but its render-target registers are unknown and
   the tile setup unproven: unbounded risk on the shell's critical path.
   Proposed as its own ADR once a first 3D triangle is measured.
4. **Keep SDRAM composition but flush with the PXA27x's DMA controller**
   (the CPU free during the copy). Unmeasured: depends on DMA write
   speed into CS3 versus the CPU's 32-41 MB/s. Not rejected: it is the
   answer to "CPU-only operations" in decision 4 if it wins, and open
   question 2 measures it.

## Consequences

- `rp-draw`'s `Renderer` trait gains "which commands did you take"; the
  shell doesn't change. `rp-gfx` keeps `CpuRenderer` as the reference.
- A graphics service owns 2700G memory: an allocator for surfaces, the
  glyph atlas and staging, with the planes at fixed places
  (docs/memory-budget.md to gain a 2700G memory table).
- New crates: the 2D command encoder (pure, host-tested: the words
  already in `rp-mbx`), `Mbx2dRenderer`, later the GDI layer and
  `Mbx3dRenderer`.
- ADR-0021's "nothing in the list is CPU-only" holds only once the 3D
  core is up; until then gradients and ARGB images are CPU work.

## Owner's answers (2026-10-14)

- **Question 4, format:** mixed. The shell's glass layer stays ARGB4444
  with per-pixel alpha over the video-plane wallpaper; a full-screen
  program's surface is RGB565 and becomes the graphics plane by a flip.
- **Question 6, legacy GDI:** what WM accelerated (fills, copies,
  transparent copies, text, horizontal/vertical lines) on the 2D engine;
  everything else on the tested CPU path.
- **Question 7, the 3D core:** after `Mbx2dRenderer` draws the shell;
  then a time-boxed start (goal: one measured triangle) under its own ADR.
- **Downward overlaps are the service's job** (the owner asked that the
  fix be automatic and the most efficient that works): the graphics
  service sends a copy down over itself as one copy with a negative
  stride (measured fastest, no CPU work); bottom-first strips are the
  fallback. Scrolling content can avoid the copy altogether by moving the
  surface's start address. Nothing above the service sees it.
- Question 5 (a completion interrupt) is a measurement, not a vote: next
  `rp-mbx` step. The overall decision is still to be voted.

## Open questions (for the owner; none decided here)

1. **Can the 2D engine read system RAM after all?** A device test points
   a copy's source past the 2700G's 16 MiB (cheap; a hang is recovered by
   a soft reset). If yes, decisions 2 and 4 simplify a lot.
2. **DMA into 2700G memory:** throughput and CPU time for a full screen,
   versus the CPU's 14.5 ms. Decides how CPU-drawn tiles get to the
   screen.
3. **Copies on the device** (built into `rp-mbx`, emulator passes): do
   the direction bits, colour key and format codes behave as found? Is
   there a 16-bit format code that copies ARGB4444 unchanged (or is a
   16 bpp copy raw)?
   **Measured 2026-10-14 (`rp-mbx` on the device, docs/hw/2700g.md):**
   (1) **No.** The engine decodes 32 MiB, local memory mirrored at 32 MiB
   and a constant (`0x1000008C`) at 16-31 MiB; system RAM is out of
   reach: decision 2 stands as written, now on measurement. (2) DMA
   writes 30 MB/s against the CPU's 33 MB/s, the CPU keeping 84% of its
   pace: no faster, but nearly free, so CPU-drawn tiles reach 2700G memory
   by DMA (alternative 4 joins decision 4 as the copier, not a separate
   path). (3) Plain, right-to-left and colour-keyed copies behave as
   found; full-screen copies 4.8 ms; **bottom-up does not** (bit 23 alone
   smears; Dell's driver sends such copies to the CPU); a **negative
   stride** on both surfaces does it in one command, exact, 4.2 ms full
   screen, the CPU idle (fifth run); strips (CPU queueing up to 4.5 ms for
   a 1 px scroll) and a scratch round trip (9 ms) also work and are slower. The ARGB4444 format code is
   still untested.
4. **The interface plane's format:** ARGB4444 dithered (the owner's
   choice for SDRAM composition) assumes CPU dithering on flush; with
   GPU-drawn surfaces, is ARGB4444 still right, or RGB565 windows under
   an ARGB4444 glass layer?
5. **A completion interrupt for the 2D engine** (bit 10 of `0x12c` is
   used for queued commands in Dell's driver): if it works, decision 7
   uses it instead of vsync polling.
6. **How far the GDI layer goes in the first release:** which ROPs,
   `AlphaBlend`, printing-free subset; driven by the apps the owner wants
   to run first.
7. **When to start the 3D core**, and how to time-box it (DESIGN.md
   requires it for GLES and DoomGLES).
