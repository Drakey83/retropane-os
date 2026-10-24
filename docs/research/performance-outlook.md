# Performance outlook: what RetroPane's drivers and clocks could give

**Status: research, 2026-10-24. Measurements and estimates, not
decisions.** Every number is either measured on the owner's X51v (named
run) or an estimate (marked, with its assumption). Kept as reference for
when the shell, the GLES layer or a game port is designed; the clock
steps and screen placement it depends on are the owner's rulings
(ADR-0035, ADR-0034 amendment P). Companions:
[quake3-on-the-x51v.md](quake3-on-the-x51v.md),
[2700g-full-potential.md](2700g-full-potential.md),
[cpu-full-potential.md](cpu-full-potential.md),
[apps-and-packs.md](../design/apps-and-packs.md) section 6.

## 1. The 3D core, by clock (measured)

Render time against Windows Mobile's clock (99.67 MHz system, cores / 2),
`rp-gpubench` GC2I (2026-10-24: every 16-bit target inside the safe
region, as ADR-0034's option C needs; full-screen 480 x 640 kinds), soak
status from GC2I/GC2J:

| Clock | 16-bit Gouraud | 16-bit bilinear texture | 16-bit blended 4444 | 32-bit textured scene (GS) | Status |
|---|---|---|---|---|---|
| Windows Mobile (99.67 / 2) | 12.5 ms | 18.9 ms | 31.5 ms | 34.4 ms | reference |
| 149.50 / 2 (today's overclock) | 1.50x | 1.48x | 1.47x | 1.47x | proven |
| 74.75 / 1 (Dell's limit) | 1.50x | 1.27x | 1.33x | 1.30x | proven |
| 99.67 / 1 | 2.00x | 1.65x | 1.76x | 1.65x | soaked clean 20 min |
| **108.33 / 1** | **2.17x** | **1.79x** | **1.89x** | **1.79x** | **soaked clean 20 min (GC2J)** |
| 117.00 / 1 | 2.35x | 1.94x | 2.06x | 1.93x | 1 wrong pixel in 4,578 renders |
| 133.25 / 1 | 2.67x | 2.12x | 2.22x | 2.07x | 1 wrong pixel in ~470 renders, one patch in one scene |

- **Untested steps, extrapolated** (2026-10-24, not run at divide 1):
  fitting each kind's GC2I times at 108.33 and 133.25 / 1 as a fixed part
  plus a part that shrinks with the clock (the fit predicts 117.00 / 1
  within 0.4 ms; Gouraud's fixed part is zero), **141.83 / 1** would give
  Gouraud 2.85x, texturing 2.23x, blending 2.32x, the 32-bit scene 2.16x;
  **149.50 / 1** 3.00x, 2.33x, 2.42x and 2.24x. Unknown whether either
  draws right: the cores would run at about twice Intel's 75 MHz rating,
  wrong pixels already appear at 117 and 133.25 / 1 (GC2J), and the local
  memory's own ceiling lies between 149.50 (right at divide 2, GC2H) and
  156 MHz (GC2H2).
- **Shading scales with the cores' clock; texturing less**, because
  texels come from local memory, which runs on the system clock. The
  textured kinds were right at every divide-1 step even with their target
  outside the region, but slower there (bilinear 1.56x at 108.33 / 1 in
  GC2S against 1.79x inside): the display's reads compete.
- **What divide 1 above 74.75 needs:** the 3D core's 16-bit picture
  written inside the safe region (~704 KiB at the start of local memory):
  option C (draw into the screen) or small targets that fit there. Under
  amendment P's A, as accepted, the core draws off screen and stays at
  74.75 / 1 or divide 2 (G21, GC2I's control). Also open: C's tearing
  design, a longer warm soak and real content, the self-test proving the
  region and the clock on every X51v (ADR-0035 rule 1), and any step above
  74.75 entering ADR-0035 by its own rules.
- **32 bits:** 32-bit *Gouraud* targets (1.2 MiB, too big for the region)
  were wrong at every divide-1 step above 74.75 (GC2I). The 32-bit
  *textured* scene (GS) was right at every step to 133.25 / 1 with its
  target outside (GC2S, GC2I, and run GS's 20-minute soaks of 2026-10-22),
  so "no 32-bit 3D above 74.75" holds for shaded 32-bit content, not
  textured, on the evidence so far.
- The fault above 108.33 (GC2J) is one patch beside a textured quad's
  corner, at rows 604-607, columns 21-37: possibly a tile seam (to check
  against the core's tile size). Until it's isolated, 108.33 / 1 is the
  highest step proven, not the ceiling.

## 2. The drivers (measured, any clock)

- **The pipeline** (the CPU prepares frame N+1 while the core draws N):
  interface frames 9-10% shorter; the worst task-switcher frame 29 to
  22 ms (run CH; ADR-0034 T1).
- **Text from cached lines:** a screen of text 9.9 ms against 12.5; the
  CPU's share 0.07 ms against 12 (run F7; ADR-0023).
- **Each job on its fastest unit:** turning the screen 26 ms on the 3D core
  against 42 on the 2D engine (ADR-0034 T4).
- **The screen in the safe region:** ~1.5-2 ms back on every 3D render at
  any clock, and 4-5% on GC2's kinds at divide 2 (C71, GC2S). Not a gain
  over Windows Mobile, whose driver keeps its screen there already.
- **Uploads:** bus-bound at ~33 MB/s either way; the CPU free for most of
  it with SRAM staging and DMA (backlog T2).
- **Audio:** the mixer 8-12% less CPU (iWMMXt, backlog C6).

## 3. The CPU (unchanged)

624 MHz is the PXA270's rated top, as under Windows Mobile; its overclock
study (cpu-full-potential.md C2, phase 9) hasn't started. CPU-bound work
gains only from better code: fixed point in place of emulated floating
point (no FPU), iWMMXt (6.7x on the mix), SRAM for hot data, the mini
cache. For games this is usually the limit (section 4).

## 4. Games (estimates)

- **The Doom family (Doom, Heretic, Hexen), our ports at 480 x 640:** one
  full-screen 16-bit textured layer is 18.9 ms at Windows Mobile's clock
  (GC2I's bilinear kind; the plan's earlier 25-29 ms came from other
  content), 12.8 ms at 149.50 / 2 and 10.5 ms at 108.33 / 1. With walls,
  floors and sprites at ~1.3-1.5 layers (estimate), the 3D core needs
  ~17-19 ms at 149.50 / 2 (just past 60 fps's 16.6) and **~14-16 ms at
  108.33 / 1 (inside it)**: full-resolution 60 fps looks reachable on the
  3D side at 108.33 / 1. The CPU side (game logic, the fixed-point
  renderer's set-up) is unmeasured; the engines' logic is integer already.
- **Quake 3:** today's Q3CE measured at 13 fps at 320x240 (its memory
  partly swapped to the CF card); our own port ~45 fps at full screen at
  108.33 / 1, ~37 at today's overclock, ~53 at 133.25 / 1, ~60 at
  320x240; the old binary ~20-33 fps, capped by its CPU share. Details
  and assumptions: [quake3-on-the-x51v.md](quake3-on-the-x51v.md).
- **Texture-lit renderers** (lightmaps, pre-lit textures) gain less from
  divide 1 than shaded ones (1.79x against 2.17x at 108.33 / 1), but are
  the safer content above 108.33 so far.
- The rest (Descent, Quake, Hexen II, Quake 2, Half-Life, Turok):
  apps-and-packs.md section 6's expectations, each to be redone from its
  own measured CPU / 3D split when its port is taken up.

## 5. How the estimate moved (2026-10-24)

1. Before: divide 1 wrong above 74.75; the best was 149.50 / 2, ~1.4-1.5x.
2. GC2S: the screen in the safe region made divide 2 fully right, but
   divide 1 still failed above 74.75.
3. G21 and GC2I (the divide-1 session): the 3D core's *target* inside the
   region makes divide 1 right to 133.25: the cause was where the picture
   is written, not the clock.
4. GC2J: 108.33 / 1 soaked clean; 117 and 133.25 nearly.
5. GC2I's textured times inside the region beat GC2S's outside, raising
   the texture-heavy estimates (Quake 3's port at full screen ~39 to
   ~45 fps at 108.33 / 1).
