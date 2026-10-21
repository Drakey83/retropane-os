# The 2700G's full potential: the map and the program (2026-10-20)

**The owner's brief (2026-10-20):** understand everything about the Intel
2700G (2D engine, MBX Lite 3D core, display controller, MVED1 video
decoder, its memory and clocks) and use every trick to get the most out of
it. Dell's Windows Mobile driver used a fraction of it; RetroPane's own
drivers can do better. Not one number made 2x bigger: every unit mapped,
every unknown probed, every win taken, and nothing left unturned.

This page is the map and the order of work. Each probe is a bounded device
run on the owner's go (numbered steps first), recorded in
[docs/hw/2700g.md](../hw/2700g.md) and the datasheet the same day, modelled
in the emulator or listed as not modellable, with an emulator check before
it runs. Settings past Dell's limits go through a **per-device self-test
with a fallback** (below), the owner's any-Axim rule.

## Verify everything ourselves (owner, 2026-10-20)

Nothing on this page is known because Dell's code, Windows Mobile, Linux's
`mbxfb` or one of RetroPane's own docs says so. "Known" in the tables
below means **measured by RetroPane on the X51v** (the run is named);
everything else read from someone else's code is a claim, listed here
with the run that settles it, and stays unticked until then.

| | Claim (source) | Our check |
|---|---|---|
| **V1** | The 2700G's local memory runs on the system clock, with no clock of its own (Linux `mbxfb`'s register map) | P2: the memory's timing measured as the system clock moves, against a core-only change |
| **V2** | The local memory is rated 100 MHz, and CL3 is needed above it (Dell's table and rule) | P2's steps: where errors start, with CL2 and CL3 at each (run G1: 3D all wrong at 108 MHz with CL3, where run 27 drew right at 133 with CL2; run G2 tries both) |
| **V3** | The 2D engine takes 1, 2, 4 and 8 bpp sources (Dell's format table) | P7: each format copied and checked pixel by pixel |
| **V4** | Dell rotates through the 3D core, so the 2D engine has no rotate (Dell's code) | P6/P13: the 2D engine's direction and rotate bits tried |
| **V5** | The video plane's scaler registers and their meaning (Dell's register names) | P9 |
| **V6** | The cursor plane's formats and size (Dell's register names) | P10 |
| **V7** | Colour keys at scan-out (Dell's register names) | P10 |
| **V8** | MVED1's registers, command windows and frame layout (Dell's code) | P11 |
| **V9** | The second texture unit's words, fog bit 24, TSP bit 14 (static reading) | P5 |
| **V11** | `PLLSTAT` bit 2 means the core PLL has locked at its new frequency (Linux `mbxfb`; run G1 read it set at once even across a frequency change) | **Settled (run G3):** it drops after the write and relocks in ~16 us; read at once it still shows the old lock |
| **V12** | Writes to the memory controller take effect when written (our assumption; Linux waits 1 ms after each) | Run G3: some didn't (`LMTYPE`, `LMREFRESH` read back unchanged); `set_speed` now reads back and rewrites; run G4 counts them |
| **V13** | The cores run at most 75 MHz (Dell's driver's cap) | Run G3: right at 99.67 MHz with divide 1, twice; **G4: right in 4 more checks, 2D 1.89x, 3D 1.81x**; soaks, warm and cold to come |
| **V14** | CL3 above 100 MHz (Dell's rule) | **Run G4: CL3 breaks the 3D core every time at 108-133 MHz; CL2 right to 133.25 MHz** (short checks) |
| **V10** | Every other number in docs/hw/2700g.md and the datasheet marked static, Dell or `mbxfb` | Swept into this table, one line each, with its probe |

## How the work is judged

- **Payoff:** how much faster, smoother or cheaper it makes what people
  see (the compositor's frame budget, uploads, battery).
- **Cost:** device runs, risk (a hang is recoverable; nothing here touches
  user data), build effort.
- **Order:** biggest payoff for the least risk first; the owner asked for
  the clocks first.

## The map: every unit, what we know, what we don't

### 1. Clocks and memory (the chip's speed)

| Known (measured) | Unknown | Probe | Payoff |
|---|---|---|---|
| System clock 99.67 MHz (core PLL), the cores at half (`MBXCLK` divide by 2, 49.8 MHz); times follow the clock exactly (run 26) | The cores at divide by 1: version 29 at 74.75 MHz had 2D right, 26% faster, but 3 of 5 3D renders wrong: why? | **P1** divide by 1 at today's 99.67 MHz and at lower clocks (does 3D fail at divide 1 itself, or above some MHz?), the wrong renders diffed pixel by pixel | Up to **2x drawing** (2D and 3D) with the memory in its rating |
| The local memory shares the system clock (no separate memory clock in mbxfb's map) and Dell's table rates it 100 MHz | How far past 100 MHz this chip's memory, cores and bus run cleanly, hot and cold | **P2** the system clock stepped 99.67 → 108 → 117 → 126 → 134 → 143 → 152 MHz with Dell's timing rule (CL3 above 100 MHz) and the cores at divide 2 then 1, each step stressed (bus, memory patterns over all free memory, 2D, 3D) | The bus ~1.3-1.8x (the 2700G's waits scale with its clock: version 29's reads slowed 13% at a 25% lower clock), and the cores with it |
| The bus: ~33 MB/s writes, 12 MB/s reads, paced by the 2700G (rp-cs3bus) | Whether a faster system clock lets a shorter strobe work (reads failed at 13 clocks a beat at 99.67) | in P2: `rp-cs3bus`'s steps at each passing clock | The bus with it |
| Power: the clocks gate per unit (`MBXCLK`, `M24CLK`) | What the 2700G draws at each clock (battery) | P2 logs the battery's own reading if the CPLD/PMIC gives one; otherwise a long-run drain comparison | Battery cost of each speed |

### 2. The 3D core (MBX Lite)

| Known | Unknown | Probe | Payoff |
|---|---|---|---|
| Our renderer: dirty tiles, retained scene, flat/Gouraud/textured, blend, depth, alpha test, mipmaps, bilinear, ARGB8888/565/4444/1555/332/8-bit grey/PVRTC, rectangles, 16- and 32-bit targets (`0x650` 1, 3, 6); **run P: all 8 target layouts, 24-bit packed, dither bit 3** | **Other target formats** (`0x650` codes 0, 2, 4, 5, 7+: measured, none twiddled): a **twiddled target would be render-to-texture**: layers drawn by the 3D core straight into texture order, no CPU reorder ever | **P3** each `0x650` code rendered into, the bytes read back and decoded (row order? twiddled? another depth?). **Written 2026-10-21 as `rp-gpubench` run P** (`RP_GPUBENCH_TARGETS`, unattended): codes 0-15, bits 4-11 added to 1, bits 4-31 added to 6; a 32 x 32 texture of placed texels drawn 1:1, 4 KiB of guard before and 56 KiB after, each try decoded as 8 layouts x 5 orders (rows, packed rows, twiddled both ways, 8 x 16 tiles); the code set by the image's own register bus, not the OS renderer. **Ran 2026-10-21 (X51v): bits 2-0 = 8 layouts (555, 565, 4444, 1555, packed 888, x888, 8888, 4444), bit 3 dithers, none twiddled: no render-to-texture via `0x650` (2700g.md 'P3'). Open: translucent pixels per code, the dither matrix (a ramp run).** **Run P2 ran 2026-10-21 (X51v): bit 3 = a 4 x 4 ordered dither on every 16-bit layout; codes 2 and 7 identical; code 6 stores alpha (blended like colour); still no twiddled target at two more shapes; `0x644` bits 0-1 move the output (orientation, likely; mapping next). Run P2W (the owner's eye on the pictures) not yet run.** Written: run P2 (`RP_GPUBENCH_TARGETS2`, unattended: two more target shapes, the dither matrix per layout with bit 3, alpha per layout, and P2d: every bit of the render table's unexplained words for a twiddled target, since P19's static read found no render-to-texture in Dell's code) and run P2W (`RP_GPUBENCH_TARGETS_WATCH`, the owner watches: a 24-bit picture on a plane, RGB565 plain and dithered, ARGB8888) | Potentially huge: GPU-made layers, cached effects, blurs, the carousel's reflections, all without the CPU |
| The TA (tile accelerator) takes objects at `0x0FFF4000` and bins them (versions 12-17) | Its speed against our CPU-built object lists; how many objects a frame it handles | **P4** the same scene (100, 1,000, 5,000 quads) both ways, timed | Scenes with many objects (text as quads, particles, games) |
| One texture unit used | The second unit (multitexture: lightmaps, a mask times a picture in one pass) | **P5** two checkers multiplied (the static reading's words, docs/hw/2700g.md) | Effects in one pass instead of two |
| Fog bit 24, TSP bit 14 untested | What fog does; bit 14 | in P5 | Depth cues, cheap tinting |
| Depth isn't stored across renders on the direct path | Whether the TA path stores it (`0x85C` bit 19) | in P4 | Partial 3D renders over 3D content |
| Render done interrupts (GPIO85) | The other status bits (`0x12c` bits 0, 4-10), TA done | in P4 | No polling |

### 3. The 2D engine

| Known | Unknown | Probe | Payoff |
|---|---|---|---|
| Fills (~190 MB/s), copies (~64 Mpx/s), negative strides, colour keys, blends (factor table), 4 bpp coverage (~16 Mpx/s blended), ARGB sources, ~6 us a command (run F) | **Scaling:** Dell sends `SCALE_1_TO_1` (`0x60800200`) once: the engine can stretch | **P6** the scale word stepped (2x, 0.5x, odd ratios), copies checked | Hardware zoom: thumbnails, pinch-zoom, scaled pictures with no CPU and no 3D |
| Source formats 1/2/4/8 bpp exist (Dell's table) | **A palette (LUT)** for 4/8 bpp sources, and mono expansion (1 bpp with two colours) | **P7** an 8 bpp and a 1 bpp source copied to RGB565, looking for a palette register (static first: Dell's driver for palette writes) | Text and icons at 1/4-1/8 the bytes, expanded at copy speed |
| Commands: 10 words a glyph | Whether surfaces persist between operations | **F6** (written, waiting for the owner's go) | Text ~3x |
| The 2D and 3D cores take turns (ADR-0030 G2) | Whether they can run at once safely, and the gain | **P8** a 2D copy stream during a 3D render, both checked | Up to ~2x throughput when both are busy |
| Rotation: Dell rotates through the 3D core | Whether the 2D engine has a rotate | in P6 (static first) | Rotated screens and pictures |

### 4. The display controller (scan-out)

| Known | Unknown | Probe | Payoff |
|---|---|---|---|
| Graphics and video planes, blend modes, RGB and YUV on the video plane, repoints at once, gamma, 60 Hz | **The video plane's scaler** (`SPOCTRL`, `SHCTRL`, `SSSIZE`: we set 1:1) | **P9** the video plane scaled up and down while scanning | Free zoom of a whole page, 0 GPU cost (a magnifier, video, a low-res game scaled to full screen) |
| | **The video plane smaller than the screen and placed** (`VSADR` X/Y: assumed) | in P9 | A sheet or a pop-up moved by registers alone: free sliding at 60 Hz |
| A hardware cursor plane exists (`HCCTRL`, RGB332/4444/1555) | Its size, placement, blend | **P10** the cursor plane drawn and moved | A free sprite layer: the pointer, a drag shadow, a small floating badge |
| Colour keys exist (`GBBASE`, `VBBASE`) | How keys combine with blending | in P10 | Cut-outs at scan-out |

### 5. The MVED1 video decoder

| Known | Unknown | Probe | Payoff |
|---|---|---|---|
| Its registers, command windows, the frame layout, from Dell's code (static) | Everything on the device: never run | **P11** bring-up: a frame of known coefficients through the hardware IDCT and motion compensation; the colour conversion (4:2:0 to YUY2 or RGB, `0x48`) on its own | Hardware video, JPEG-like decoding, **a free 4:2:0 → RGB converter** (photos and wallpapers at 12 bits a pixel) |

### 6. The bus and the CPU's side

| Known | Unknown | Probe | Payoff |
|---|---|---|---|
| CPU bursts 33 MB/s, DMA 30, scattered single writes ~380 ns | DMA burst sizes and widths, the 2D command window fed by DMA (flow control?) | **P12** DMA descriptors with other burst settings; DMA into the 2D window with the FIFO's level as the guard | The CPU free while commands stream |

### 7. Added 2026-10-20 (the owner asked what else there is)

| Probe | What | Payoff |
|---|---|---|
| **P13** | The 2D engine's whole operation set: lines, pattern brushes, every ROP, gradients if any, rotation | Drawing the interface's shapes with no CPU |
| **P14** | The 3D core's limits: objects and textures a frame before it slows, texture size against speed (its texture cache), points, lines and strips, trilinear mipmaps, the remaining texel formats | Knowing what a frame can hold; assets shaped for the cache |
| **P15** | Everything at once: scan-out reading memory while 2D and 3D both draw | Where the engines get in each other's way |
| **P16** | Power and heat: each engine and each clock against battery drain | The fastest setting that's also the right one |
| **P17** | The display: refresh rates above 60 Hz and below (battery), the panel's limits | Smoother motion or longer battery |
| **P18** | Every status and interrupt bit of every unit | No polling anywhere |
| **P19** | A full read of Dell's GPU code (`ddi_2700g`, `libGLES_CL`, `pvrvadd`) for every register and feature it touches that RetroPane hasn't used | Nothing the chip does goes unnoticed (facts only, never copied) |
| **P20** | The display's fetch against the engines (found 2026-10-21, run W: the 32-bit scan-out costs 3D ~10% and CPU reads ~20% at Windows Mobile's clock): the same render with the plane at 32 bits, 16 bits and off, measured; and `0x644`'s orientation field mapped | A 16-bit screen's real saving; rotation for free if `0x644` is one | **Ran 2026-10-21 (run T): 16-bit screen 3D -6%, reads -15% (stopped: -9.5%, -16%); `0x644` = 2:1 averaging downsample on write-out (anti-aliasing resolve, free 2:1 downscale), not rotation. Open: its cost** |

The CPU's own map, and the CPU, 2D engine and 3D core working together,
are in [cpu-full-potential.md](cpu-full-potential.md).

## The checklist rule

The map is the checklist. Every unit and feature has a line here, and
each line is ticked only with evidence: a measurement, a probe's result,
or a reason it can't be done or isn't worth it. Anything found along the
way gets a line. The owner is told "nothing is left" only when every line
is ticked, and can check that here.

## The order (owner, 2026-10-20: the clocks first)

1. **P2 + P1, the clocks** (`rp-gpuclock`): the overclock map with
   the cores at divide 2 and 1, every step stressed. It also decides the
   self-test's design.
2. **F6**, written: text's lean commands.
3. **P3, render targets** (render-to-texture would change the compositor).
4. **P9 + P10, the display's scaler, placed planes and cursor** (free
   motion and zoom at scan-out).
5. **P6 + P7, the 2D engine's scaler and palette.**
6. **P8, 2D and 3D at once.**
7. **P4 + P5, the TA path and the second texture unit.**
8. **P11, MVED1.**
9. **P12, DMA tricks.**

Each step's results can reorder the rest; the map is kept current.

## Past Dell's limits: the per-device self-test (design)

For any setting past Dell's (a faster system clock, the cores past 75 MHz,
a shorter bus strobe):

1. **Calibrate once** (at install, docked, after the chip is warmed by
   load): each step stressed for minutes (bus, all free 2700G memory with
   every pattern, 2D, 3D, with checks); the setting chosen is **one step
   below** the fastest that passed cleanly.
2. **Guard while running:** spare 2700G memory checked in the background
   now and then, renders spot-checked; any error, GPU hang or unexplained
   reset drops straight back to Dell's setting until a re-test.
3. **Re-test** after updates and now and then; a switch to turn it off.

The 2700G's memory holds pictures, textures and commands, not the user's
files: an error shows as wrong pixels or a GPU hang (caught), not silent
data loss, which is why this risk is acceptable where SDRAM timing wasn't.
The voltage is never touched (no control exists), so a too-fast clock
errs before it harms.
