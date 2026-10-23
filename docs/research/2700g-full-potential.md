# The 2700G's full potential: the map and the program (2026-10-20)

> **GATE (the owner, 2026-10-21): nothing moves forward until this is done.**
> No new phase, feature or design work until every clever software trick has
> been tried to get the most out of the 2700G's 3D core, its 2D engine, the
> PXA270, and all of them together: every line of
> [2700g-full-potential.md](2700g-full-potential.md) and
> [cpu-full-potential.md](cpu-full-potential.md) ticked with device
> evidence (used, or declined with the reason). Meanwhile only finishing and
> proving work already built, fixes and the quality audit, and the
> measurements these pages need. Each summary gives the count of lines
> ticked and open.

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
| **V13** | The cores run at most 75 MHz (Dell's driver's cap) | Run G3: right at 99.67 MHz with divide 1, twice; **G4: right in 4 more checks, 2D 1.89x, 3D 1.81x**; **GS: a 20-min stressed warm soak 0 BAD at 99.67 and 133.25 MHz**; the cold soak to come |
| **V14** | CL3 above 100 MHz (Dell's rule) | **Run G4: CL3 breaks the 3D core every time at 108-133 MHz; CL2 right to 133.25 MHz** (short checks) |
| **V10** | Every other number in docs/hw/2700g.md and the datasheet marked static, Dell or `mbxfb` | Swept into this table, one line each, with its probe |
| **V15** | The 2D engine's command words read from Dell's code (`docs/hw/2700g.md` "The MBX 2D engine", (A) rows): clip rectangle (header bit 18), 8 and 24 bpp fill formats, the scale word, mono masks (`0x40000000`, `0x50000000`, `0xE4000000`), source and brush palettes (`0xC0000000`, `0xD0000000`) | Each used on the device and checked pixel for pixel (P6, P7, P13); fills, copies, blends, keys, stride 0, negative strides, the fence, the reset and the FIFO level are proven already |
| **V16** | The display planes' formats and blend modes from Linux `mbxfb` ((B) rows): indexed graphics plane, inverse alpha, the packed YUV formats | Each shown on the device (with P9/P10); ARGB4444, ARGB8888, RGB565, packed 24-bit, planar 4:2:0, per-pixel and global alpha are proven |
| **V17** | `0x12c`'s status bits from Dell's interrupt handler: bit 10 (queued commands) and the bits not yet named | Each made to set on the device (P18); bits 2, 4 and 6 are proven |
| **V18** | Windows Mobile's own plumbing read from Dell's driver: its virtual mapping, the services layer, escape codes, the power votes | Not needed: RetroPane's own drivers own the chip (no Windows Mobile layer); kept only as a source of facts |

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
| **P20** | The display's fetch against the engines (found 2026-10-21, run W: the 32-bit scan-out costs 3D ~10% and CPU reads ~20% at Windows Mobile's clock): the same render with the plane at 32 bits, 16 bits and off, measured; and `0x644`'s orientation field mapped | A 16-bit screen's real saving; rotation for free if `0x644` is one | **Ran 2026-10-21 (run T): 16-bit screen 3D -6%, reads -15% (stopped: -9.5%, -16%); `0x644` = 2:1 averaging downsample on write-out (anti-aliasing resolve, free 2:1 downscale), not rotation; run T3: the resolve is free (a full-screen render 15.95 ms plain, 14.46 ms resolved): 2 x 2 anti-aliasing costs 4x the tiles only** |
| **P21** | Found 2026-10-21 (`rp-mbx-blend` B7): with 16-bit screens the 2D dither add (four blits, stride x 4, a stride-0 pattern row, blend word 0) is the dearest step, 18.9 ms over a full 32-bit screen, twice the 32-to-16 conversion (9.6 ms). Alternatives to measure: one blit from a full 4-row pattern block repeated by the clip (or a screen-sized pattern picture: one blend pass), the add folded into the background (dithering what never changes once), the 3D core resolving the work picture where it can, and header bit 21 (writes nothing: a mask or pattern mode?) | Cheaper redraws of large areas (a full redraw ~28.5 ms of engine today) | **done: used** (the owner's ruling 2026-10-22: ledger row P21) |

The CPU's own map, and the CPU, 2D engine and 3D core working together,
are in [cpu-full-potential.md](cpu-full-potential.md).

## The ledger (counted by `cargo xtask gate`)

Every line of this page, one row: **done** (used, or declined with the
measured reason), **partial** (some device evidence, more to do) or
**open**. Only done counts as ticked. A row changes only with a run
named in its evidence.

| Line | Kind | Status | Evidence, or what's left |
|---|---|---|---|
| V1 | arch 5 | open | P2's measurement (the local memory's timing against a core-only clock change) not run |
| V2 | arch 5 | partial | run G4: CL3 breaks the 3D core at 108-133 MHz, CL2 right to 133.25 (short checks, one unit); **run GS (2026-10-22): 133.25 MHz CL2 0 BAD in a 20-min stressed warm soak**; **the cold soak 0 BAD too**; other units open |
| V3 | opt | partial | B9 (X51v): 8 bpp (palette) and 1 bpp (palette entries 0/1) proven; 2 and 4 bpp not tried |
| V4 | arch 5 | done | B11 (X51v, 2026-10-22, `local/rp-mbx-blend-B11.txt`): **the 2D engine rotates**: header bits 25-26 = 1 / 2 / 3 lay a copy down 90° clockwise / 180° / 270° clockwise (the rectangle given in the source's shape, the output in the turned box; Dell's table had 1 and 3 the other way round), every pixel checked; the claim "no 2D rotate" was wrong |
| V5 | arch 5 | open | P9 not run |
| V6 | arch 9 | open | P10 not run |
| V7 | opt | open | P10 not run |
| V8 | arch 7 | open | P11 not run |
| V9 | opt | partial | P5's test M: unit 1's words after unit 0's confirmed (emitter and TA output), TSP bit 16 found as the coordinate-set switch, the two-unit object's layout read from the TA; the render hangs (open); fog and TSP bit 14 not run |
| V10 | meta | done | 2026-10-21: the 51 rows of `docs/hw/2700g.md` resting on Dell's code or `mbxfb` alone swept: what device runs since prove is noted in each new line, the rest became V15-V18 |
| V15 | opt | partial | B9, B10 (X51v): clip, scale word, source palette, 8 and 24 bpp fills proven; fills, copies, blends, keys, strides, fence, reset, FIFO before; **B11: mono masks proven** (Dell's words `0x40000000 \| offset << 14`, `0x50000000 \| width << 14 \| rows`, `0xE4000000 \| stride`, address; high bit = leftmost pixel; ROP4 as GDI: low byte where the mask is 1); left: the brush palette (`0xD0000000`), the bit offset's meaning (offset 3 rotated each 8-bit row) |
| V16 | opt | partial | runs rp-blend, P2W: ARGB4444, ARGB8888, RGB565, packed 24-bit, planar 4:2:0, per-pixel and global alpha shown; the indexed plane, inverse alpha and the packed YUV formats not |
| V17 | opt | partial | bits 2, 4, 6 seen on the device; B10 (X51v): bit 10 stayed clear with 130 words queued, so it isn't "commands queued"; run FI: bit 3 sets with bit 2 at a render's end, 2D raises no bit; bit 10's meaning open |
| V18 | opt | done | declined: Windows Mobile's own plumbing (mapping, services layer, escapes, power votes); RetroPane's drivers own the chip, nothing here bears on speed |
| V11 | arch 5 | done | run G3: `PLLSTAT` bit 2 drops after the write and relocks in ~16 us |
| V12 | arch 5 | partial | run G3: writes to `LMTYPE`/`LMREFRESH` can be lost; `set_speed` reads back and rewrites; **run GS (2026-10-22): once read back they held in all 1,320 rounds over 40 min**; a count over many changes open |
| V13 | arch 5 | partial | runs G3, G4: the cores right at 99.67 MHz divide 1 (2D 1.89x, 3D 1.81x), short checks on one unit; **run GS (2026-10-22): warm soak 0 BAD at 99.67 and 133.25 MHz, divide 1, 20 min each (3D 1.56x / 1.98x with the 16-bit screen on)**; **the cold soak 0 BAD too**; the self-test open |
| V14 | arch 5 | partial | run G4: CL3 breaks 3D every time, CL2 right to 133.25 MHz (short checks); **run GS (2026-10-22): CL2 at 133.25 MHz 0 BAD in a 20-min warm soak**; **the cold soak (2026-10-22) 0 BAD too, the same times**; other units and the self-test open |
| P1 | arch 5 | partial | runs G3, G4 (divide 1 right at 99.67 MHz); version 29's wrong 3D at 74.75 MHz unexplained; **run GS (2026-10-22): the warm soak 0 BAD**; **the cold soak (2026-10-22) 0 BAD too, the same times**; other units and the self-test open |
| P2 | arch 5 | partial | runs G1-G4 (steps to 133 MHz with CL2); **run GS (2026-10-22): stressed warm soak 0 BAD at 99.67 and 133.25 MHz CL2 (bus read 31.7 / 30.2 ms a 256 KiB)**; **the cold soak (2026-10-22) 0 BAD too, the same times**; other units and the self-test open |
| P3 | arch 5 | done | runs P, P2: `0x650` = 8 layouts + a dither bit, none twiddled: no render-to-texture through it (declined: the hardware has none) |
| P4 | arch 5 | partial | `rp-mbx3d` versions 12-17 (X51v): the tile accelerator takes objects at `0x0FFF4000` and draws flat and textured, says done by status bit 4 in ~2 ms; its speed against the CPU-built lists (100 / 1,000 / 5,000 quads) not measured |
| P5 | opt | partial | `rp-glesfeat` M (X51v, 2026-10-22): through the TA, **TSP bit 16 = a second coordinate set** (mask bit 3 alone ignored); the TA writes a 32-word two-unit object (list layout 12); **the 3D stage hangs on it** with three unit-1 combine words, and the TA path stays hung after a core reset (C53). Left: what the 3D stage needs for unit 1 (static: the 3D table for a two-unit scene), the direct form with layout 12, fog (TSP bits 24/23, `0x63C`), TSP bit 14 (no effect seen on the alpha test) |
| P6 | opt | partial | B9 (X51v): the scale word scales (nearest pixel, s = 32 x src / dst), a full-screen 2x upscale 18.7 ms (half a copy's speed); its use (zoom, thumbnails) to be wired where the compositor needs it |
| P7 | opt | partial | B9 (X51v): 8 bpp sources through a 256-entry palette (required), 63 ms a full screen (12x a 16-bit copy: not for anything redrawn often); 1 bpp through entries 0/1, high bit first; left: 2/4 bpp, the brush palette, mono masks |
| P8 | arch 5 | partial | run FT (X51v): together 9% faster (85.4 against 93.8 ms) but the 3D render gets ~220 wrong pixels (450+ against 2D fills; the render alone repeats exactly), so ADR-0030 G2's turns hold; left: the mechanism, and whether a memory-controller setting or separate memory areas make it safe |
| P9 | arch 5 | open | the video plane's scaler and placement not tried |
| P10 | arch 9 | open | the cursor plane and colour keys not tried |
| P11 | arch 7 | open | MVED1 never run past its safe stages |
| P12 | opt | partial | DMA writes 2700G memory at 30 MB/s with the CPU 84% free (`rp-mbx`, 2026-10-13); run E (2026-10-20) overlapped the reorder with DMA; burst size and width settings, DMA into the 2D command window works (run FD, T3: the CPU 2 us against 43-60 us for 352 words, same pictures) |
| P13 | opt | partial | B10 (X51v): all 256 ROP3s are GDI's (pattern = the colour word), 8/24 bpp fills; B9: scaling, palettes, 1 bpp, clip; B7: header bit 21 writes nothing (copies and fills, its purpose open); B11: rotation bits 25-26 (90/180/270, every pixel) and mono-mask colour expansion proven; left: lines, the brush palette and pattern brushes |
| P14 | arch 5 | partial | `rp-gpubench` G1 and datasheet (X51v): a bare tile ~5.3 us, a textured one ~11.3 us, each translucent layer ~8 us more, a full-screen textured layer 25-29 ms; objects and textures a frame before it slows, the texture cache's size, points/lines/strips, trilinear not measured |
| P15 | arch 5 | partial | runs P2X, P2Y, P2L: a 32-bit scan-out corrupts 32-bit 3D renders, a 16-bit one doesn't; 2D and 3D both busy with scan-out open |
| P16 | arch 9 | declined | **Declined (the owner's ruling 2026-10-22, docs/research/gate-classification.md, B7): no battery reading exists** (reopen if one does). What was found: needs a battery reading: the TSC2046's VBAT isn't wired (entropy.md); Windows Mobile's gauge (`GetSystemPowerStatusEx2`, used by `rp-wmpower`, +-50 mA) comes from `battdrvr.dll`, which only calls `KernelIoControl` with Dell's own HAL codes `0x0101010D` (function 67) and `0x0101011B` (function 70) (2026-10-22, static); their handler in the kernel image wasn't found by constant (not as literals, nor the SSP/I2C bases): a bounded RE step next, or the WM gauge as a stand-in |
| P17 | arch 5 | partial | display calibration C3: 60.0-60.5 Hz works; rates above and below not tried |
| P18 | arch 5 | partial | run FI (X51v, 2026-10-22): `0x12c` around 2D and 3D jobs with enables 0 / all: **no 2D completion bit** (bits 16-23 the FIFO's fill while busy, nothing latched), so 2D waits stay polled; a render's end sets bits 2 and 3 (3 new, meaning open); the enables don't change the raw status. Left: the TA's bit 4 and other units (the display's `DINTRS`, the system block's interrupt registers, MVED1), and whether the FIFO level can interrupt at a low-water mark |
| P19 | meta | partial | the render table read statically (no render-to-texture in Dell's code); the rest of the GPU code not read for unused features |
| P20 | arch 5 | done | run T: a 16-bit screen saves 3D 6%, reads 15%; `0x644` is a free 2:1 resolve (T3); used: ADR-0023's 16-bit screen |
| P21 | arch 5 | done | **Used (the owner's ruling 2026-10-22):** areas that don't change (wallpaper, background) are dithered once, when they change; 3D-drawn areas dither free in the render (`0x650` bit 3, runs P, P2); the 2D dither add runs only on 2D areas that changed, priced per area (B8: 18.9 ms a full screen). The rest measured: one blit from a full-screen pattern = the four row blits (B8), header bit 21 writes nothing (B7, B10), the 3D core can't read the 2D work picture (no row-order texture mode, run B) |
| F6 | arch 4 | partial | run F, third run (X51v, 2026-10-20): the engine keeps its surfaces and a copy runs without its colour word (4 words a glyph, not 10), but a screen of text stays 12.4 ms: ~6 us a operation is fixed on the engine, so fewer bytes don't help; the lever is fewer operations (whole words or lines kept rendered, one blit each), not yet built or measured |

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
