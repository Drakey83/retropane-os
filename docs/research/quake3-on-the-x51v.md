# Quake 3 on the X51v: what it does today, and what RetroPane could do

**Status: research, 2026-10-24. Results and estimates, not decisions.** One
measurement on the owner's X51v and the 2005 claims it's compared with;
the projections below are estimates built on named assumptions, each to be
replaced by a measured number when it exists. Companions:
[3d-games-catalogue.md](3d-games-catalogue.md) (Q3CE's row, sources),
[gles-games.md](gles-games.md) (its imports),
[apps-and-packs.md](../design/apps-and-packs.md) section 6 (the port plan,
30 fps minimum, 60 the target), [performance-outlook.md](performance-outlook.md)
(the clocks' and drivers' gains in general).

## 1. The 2005 claim

All from one announcement (Imagination Technologies at G-Star 2005, Korea):

- bit-tech, 2005-11-11 (archived copy:
  `web.archive.org/web/20060107114535/http://www.bit-tech.net/news/2005/11/11/q3ce_powervr/`):
  **Q3CE v1.1** by Christien Rioux (NoctemWare), for Windows CE 4.2 (the
  X50v's Windows Mobile 2003 SE), tuned with his SpeedDemon profiler,
  "currently achieving **up to 25 fps** on the Dell Axim X50v", "totally
  playable on the 3.7-inch 640x480 colour TFT screen".
- The Inquirer (article 27609, reposted on TechEnclave 2005-11-13):
  "currently clocked at a respectable 25 frames per second".
- Khronos (Nov 2005): "up to 25 fps on the Dell Axim X50v".

**Not stated anywhere:** the resolution Quake 3 rendered at (640x480 is
the screen, not the render size), its settings, the map, or whether 25 is
an average or a peak ("up to" reads as a peak). A vendor demo.

## 2. Measured on the owner's X51v (2026-10-24)

Windows Mobile 6.1 (LennySh ROM), Dell's 2700G driver at its own clock
(99.67 MHz / 2), run by the owner from his CF card. The unit is the X51v
(its front bezel is a replacement X50v one, so the photos read "X50v").
Photos: `local/q3ce-2026-10-24/` (not in the repo).

- **Build:** "Q3 1.32b wce-ARM Feb 13 2008" (the console's first line), a
  later build than 2005's v1.1, with Russian ReadMe files (likely a
  community rebuild). Copy on the PC: `local/apps/Quake 3/`.
- **13 fps** on the game's own counter (`cg_drawFPS 1`), one reading in an
  open area: a single snapshot, not an average.
- **Settings** (System Setup photo and the card's `BASEQ3/q3config.cfg`):

| Setting | Value | What it costs |
|---|---|---|
| Video mode | **320x240** (`r_mode 0`), full screen | a quarter of the screen's pixels |
| Lighting | **Vertex** (`r_vertexLight 1`) | the cheapest: no lightmap pass |
| Geometric detail | Medium (`r_subdivisions 4`, `r_lodbias 2`) | light |
| Textures | compressed (`r_ext_compressed_textures 1`), detail full (`r_picmip 0`), trilinear | moderate |
| Multitexture | on (`r_ext_multitexture 1`) | fewer passes |
| **Sync Every Frame** | **on** (`r_finish 1`) | the CPU waits for the 3D core every frame: no overlap |
| Dynamic lights, high-quality sky, marks on walls | on | some cost each |
| Sound | 11 kHz, volume 0 | small |
| Frame cap | none (`com_maxfps 0`) | |

- **Its memory file `Q3CE_MEM.CFG`** (the PC copy; the card's may differ):
  `aaac`, `40`, `40`, `10`. Decoded from the build's Russian ReadMe:
  line 1 is one letter per memory pool (HUNK, ZONE, sound) for where it
  lives: `m` RAM, `d` storage, **`a` automatic: RAM if there's room,
  otherwise a swap file**; the last letter is where that swap file goes
  (`s` built-in storage, **`c` the card**). Lines 2-4 are the pools'
  sizes in MB (HUNK: maps and textures; ZONE: working memory, which the
  ReadMe says "should be in memory for performance"; sound). So the
  owner's settings ask for 90 MB on a 64 MB device, with the overflow
  swapped to the CF card. The ReadMe's author, on his own 64 MB X51v,
  used `dms`, `32`, `17` (HUNK on storage, ZONE in RAM).
- **The CF card under Windows Mobile** (docs/perf.md): reads ~1.9 MB/s,
  **writes 0.20-0.24 MB/s**. A swap file there slows play, not only
  loading.

**What Q3CE.EXE itself shows** (its strings, read on the PC): 24 screen
modes, from 176x208 up to **480x640 (mode 23) and 640x480 (mode 21)**,
each also rotated; **its built-in default is mode 3, 176x220**. So full
resolution is a setting, not a given, and the 2005 demo's resolution
can't be assumed.

## 3. Reading it

At 320x240 with vertex lighting the 2700G has little to do: one
full-screen 16-bit textured layer costs it ~19.9 ms at Windows Mobile's
clock at 480x640 (`rp-gpubench` GC2S), so roughly 5 ms at 320x240, ~10 ms
with overdraw (estimate). A 13 fps frame is 77 ms, so most of it isn't
the 3D core: Quake 3's game code and its 3D math emulating floating
point (the PXA270 has no FPU), Dell's GLES layer, the Sync-Every-Frame
wait, and likely **the swap file on the CF card**. Which of those dominates
isn't measured; the tests in section 5 split them.

**What the 2700G can take over, and what it can't.** It rasterizes
(texturing, blending, depth) and its tile accelerator (TA) bins triangles
into tiles. The 3D math before that (transforming each vertex, clipping,
lighting) runs on the CPU: Dell's GLES driver transforms vertices in
software and hands the TA screen-space x, y, z, 1/w (ADR-0024's reading of
its vertex words); no geometry unit is used, and as far as our reads go
the 2700G has none. Game logic, physics, bots, visibility, model animation
and sound are the CPU's in any case. So for Quake 3:

- **Give the 2700G everything it can take:** the TA for binning (gate
  P4), the second texture unit (texture and lightmap in one pass, which
  also halves the CPU's per-pass vertex work: backlog rank 20, waits on
  P4 and C53), DMA to feed it (T3), and the pipeline (the CPU on frame
  N+1 while the core draws frame N; ADR-0034 T1), not Sync Every Frame's
  lockstep.
- **Make the CPU's share cheap:** fixed-point math in place of emulated
  floating point (as PocketQuake did for Quake 1), and iWMMXt for the
  vertex math (four 16-bit multiply-adds at once; 6.7x on the audio mix,
  cpu-full-potential.md C6).

**The speed-ups that apply** (measured, `rp-gpubench`, X51v; render time
against Windows Mobile's clock, the 16-bit target **inside the safe
region** as option C needs: GC2I, 2026-10-24; the soak status from GC2J):

| Clock | Gouraud | Bilinear texture | Blended 4444 | Status |
|---|---|---|---|---|
| 149.50 / 2 (today's overclock) | 1.50x | 1.48x | 1.47x | proven |
| 99.67 / 1 | 2.00x | 1.65x | 1.76x | soaked clean 20 min |
| **108.33 / 1** | **2.17x** | **1.79x** | **1.89x** | **soaked clean 20 min (GC2J)** |
| 117.00 / 1 | 2.35x | 1.94x | 2.06x | 1 wrong pixel in 4,578 renders |
| 133.25 / 1 | 2.67x | 2.12x | 2.22x | 1 wrong pixel in ~470 renders |

Texturing gains less than shading because texels come from local memory,
which runs on the system clock. With the target outside the region
(GC2S) texturing gained less again (1.56x at 108.33 / 1), so C helps
texture-heavy games twice over: it allows divide 1, and it frees local
memory from the display's competing reads. Divide 1 above 74.75 needs C
(or small targets inside the region): under ADR-0034 amendment P's A the
3D core stays at 74.75 / 1 or divide 2.

## 4. Projections (estimates)

**Assumption A (the owner's, to be tested):** the CF swap file is why the
owner's run shows 13 fps, and with Quake 3's memory in RAM it reaches the
2005 figure, **25 fps (40 ms) at 320x240**, split roughly **30 ms CPU,
10 ms 3D core** (estimate). **Assumption B:** a native port's fixed-point
math halves the CPU's share to ~15 ms (PocketQuake-like; not measured).
The 3D core's share scales by the texturing column above (Quake 3 is
mostly textured); full screen quadruples it (~40 ms at Windows Mobile's
clock).

RetroPane on a fast SD card, our drivers, option C:

| | 320x240 | 640x480 (full screen) |
|---|---|---|
| Today's Q3CE binary (A), any clock above | ~28-33 fps | ~20-33 fps |
| Our own port (A, B), today's 149.50 / 2 | ~60 fps | ~37 fps |
| Our own port (A, B), **108.33 / 1 (proven step)** | ~60 fps | **~45 fps** |
| Our own port (A, B), 133.25 / 1 (if the edge fault is fixed) | ~60 fps | ~53 fps |

The binary's ranges: the low end with Sync Every Frame's lockstep (CPU
then core, added), the high end with the pipeline (the longer of the
two); its CPU share (~30 ms) caps it whatever the clock. The port's
figures are the longer of the CPU (~15 ms) and the 3D core. Lightmaps in
place of vertex lighting: roughly a quarter off the port's figures
(estimate). Which clock step ships is the owner's ruling (ADR-0035; every
X51v measured by the self-test).

Separately from fps: loading (the owner's ~5 minutes under Windows Mobile;
the target 15-30 s, DESIGN.md) gains from our faster storage path, and
RetroPane leaves more of the 64 MB free than Windows Mobile (OS at most
12 MiB, docs/memory-budget.md), so even the old binary's pools fit in RAM
without a swap file.

**If assumption A fails** (the memory settings don't bring 25), the 2005
figure came from a different build, system or resolution, and the table is
redone from the measured split.

## 5. Measurements that would firm it up (when the port is taken up; no one's queue)

Same spot in the same map each time, the fps noted after each step:

0. `Q3CE_MEM.CFG` on the card set to the ReadMe author's X51v values
   (`dms`, `32`, `17`). Tests assumption A.
1. Sync Every Frame off.
2. Dynamic Lights, High Quality Sky, Marks on Walls off.
3. Video mode at its smallest (176x220 or 240x320).
4. Video mode 640x480 (or 480x640).

Steps 3 and 4 together give the CPU / 3D-core split the projections
assume. Later: an average over a recorded demo (`timedemo`) in place of
one reading, and Q3CE v1.1 (NoctemWare's 2005 build) if it can be found,
to repeat the 2005 demo as it was.
