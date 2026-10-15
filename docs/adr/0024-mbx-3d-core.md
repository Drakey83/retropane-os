# ADR-0024: Bringing up the MBX 3D core

- **Status:** Accepted (2026-10-14, by the project owner; open: the time box, and the owner starting DoomGLES for M3)
- **Date:** 2026-10-14

## In plain words

The 2700G has two drawing engines. The **2D engine** now draws the whole
Today screen (ADR-0023: smooth, tear-free, the CPU's share of a tap under a
millisecond). The **3D core** (PowerVR MBX Lite) is the other, bigger
half: textured and scaled images, rotation, real gradients from per-vertex
colours, depth, and OpenGL ES, which DoomGLES, Doom 2, Heretic and
Quake 3 on the owner's card all use. Nobody has published how to drive
it, so we find out, the way we found the 2D engine: read Windows Mobile's
own drivers (as findings, no code copied), watch them work on the device,
then draw with it ourselves. The goal of this ADR is **one triangle drawn
by the 3D core and measured, under RetroPane's loader**, in bounded steps.
What it can do then decides how much more the GPU takes over (ADR-0022
decision 4 waits on it, owner 2026-10-14) before the compositor and the
app-facing graphics are designed further.

## Context

- **Known on the device:** the MBX core's registers at `0x0FFE4000` (ID
  `0x01020000` read under the loader and under WM); `0x12c` status (bit 2
  "3D render done", bit 4 "TA done", bit 6 "parameter memory ready" in
  Dell's interrupt handler; none ever set for 2D work); a render started
  by a 15-pair register table (`0x608` region headers, `0x60c` parameter
  buffer, `0x654`/`0x658` tile counts) then `0x680 = 1`, completion
  writing `0x804` (docs/hw/2700g.md, found in Dell's driver); reading an
  address the 2700G doesn't implement **hangs the bus** (four WM probes
  froze), so only offsets the drivers touch are ever read.
- **The scene format (partly):** region entries `x | y << 8` with a
  last-region bit, control words `0x6xxxxxxx` pointing at objects, end word
  `0xF0000000`; objects of four vertices (x/y pairs, then Z, then 1.0,
  then colours). Strongly like the Dreamcast's PowerVR2 (TA, ISP and TSP
  words), whose formats are publicly documented.
- **The drivers, in `local/` (never committed), from the owner's ROMs:**
  `libGLES_CL.dll` (128 KiB, OpenGL ES 1.x "Common-Lite", Imagination
  Technologies' code, 102 exported functions, three ROM versions that
  differ), `d3d_2700g.dll` (182 KiB, Direct3D Mobile: a second client of
  the same core, to cross-check), `pvrvadd.dll` (16 KiB, overlay helper),
  `ddi_2700g.dll` (the display driver, already mapped for 2D).
- **Found today (first look, 2026-10-14):** `libGLES_CL.dll` imports
  nothing that reaches hardware except **`ExtEscape`**: every request goes
  through Dell's display driver's escape handler. So the 3D path has two
  halves: the GLES library builds the scene; `ddi_2700g.dll` (already
  disassembled) allocates memory, starts renders and waits. Settings it
  reads: `Drivers\Display\PowerVR\MBX1\Game Settings\OpenGLES\`
  (`ParameterBufferSize`); it offers `GL_IMG_texture_compression_pvrtc`.
- **Workloads on the owner's CF card:** DoomGLES, Doom2GLES, HereticGLES
  and Quake 3 (all load `libGLES_CL.dll`; Quake 3 by name: 57 GL and EGL
  functions, a ready-made compatibility list). Program files copied to
  `local/apps/`.
- **Non-negotiables (owner):** native drivers for all hardware, the 3D
  core included; no Windows Mobile driver DLLs at runtime; WM5 app
  compatibility with DoomGLES as the flagship.

## Decision (proposed)

Four milestones, each ending in a written finding, each stopping to
report; device runs only with the owner's go.

1. **M1, static: the escape interface.** Every `ExtEscape` call in
   `libGLES_CL.dll` (codes, what goes in, what comes back) matched to
   `ddi_2700g.dll`'s handler for that code (memory allocation and mapping,
   register access, render start, waits). Output: "The 3D path" in
   docs/hw/2700g.md, every register offset named with its evidence, so
   later probes read only those.
2. **M2, static: a triangle's journey.** How `glDrawArrays` on one
   triangle becomes the scene in memory (vertex data, object list, region
   headers, parameter buffer) and which registers start it, cross-checked
   against `d3d_2700g.dll` and the public PowerVR2 formats. Output: the
   scene format as a spec, and a host-tested encoder (pure Rust, like
   `rp-drv-blit2d`) that builds the same bytes.
3. **M3, device under Windows Mobile (read-only probe):** while DoomGLES
   runs, a probe in the style of `rp-wmgpu` snapshots only the registers
   M1 named and the scene buffers at a render. This confirms M2 on a real
   render. The owner launches the game; nothing is written.

   **Built 2026-10-14 (`tools/wmbench/rp-wm3d.c`):** after M4 drew flat
   and shaded triangles, textures are next, and textured objects (like
   the tile accelerator's output) are written by the core, not by code we
   can read. So the probe copies the 2700G's whole 16 MiB, read-only, twice
   while DoomGLES plays (at 1 and 1.5 minutes), at the highest thread
   priority after a 30 ms wait so the frame isn't torn (about a second;
   the game pauses), and saves the copies to the SD card for the PC.
   Registers: only `MBXCLK` and the core's status, both read by Dell's
   driver. The copies are analysed on the PC; nothing is copied into
   RetroPane but facts.

   **First captures (2026-10-14, versions 1-4 on the device):** version 1
   found 4 MiB of RAM free with DoomGLES running (nothing copied) and froze
   the game by drawing when done; version 2 wrote to the card with the game
   suspended and hung (a suspended thread can hold a lock the card needs),
   and had picked the wrong program; version 3's process list came back
   empty; **version 4 works**: `doomgles.exe` (3 threads, window class
   `GLHereticWindowClass`) suspended only while a 1.5 MiB slice is copied
   to RAM. About 5 s a slice: 0x000000-0xBFFFFF saved before the owner
   left. In it: Dell's clear scene at 0x130000 (80 x 40 region entries,
   all to one empty list at 0x12a068) and textures filling
   0x2F0000-0xAEFFFF. The game's own frame is likely above 0xC00000:
   next, a capture of that part first.

   **Version 5 (top 4 MiB, twice, both whole):** no scene there. Textures
   to 0xD57FFF (16-bit colour), from 0xD58000 blocks that look like PVRTC
   (the compressed format the GLES driver advertises), above 0xE00000
   unused (all ones). So the game's scene is in the low part version 4
   saved: candidates the region array at 0x130000 (80 x 40 entries) and
   object words at 0x12A000 (`e0000000 a8800000 0e000000 d6886610
   22220e80`, then floats): our ISP word, a TSP-like word, and
   `0xd6886610`, whose low 21 bits x 8 = 0x433080 lands in the texture
   area (a texture address as in the PowerVR2's texture control word?
   to be confirmed). Next: decode those words against the textures.

   **Version 6 (0x100000-0x2FFFFF, three times; 1 and 2 one frame each):
   the game's objects found.** 978 objects of 26 words, 104 bytes apart,
   from 0x2B5000, and 3,819 list end words (`0xF0000000`) in
   0x2B0000-0x2FFFFF, lists and objects together (the tile accelerator's
   parameter memory). Hypothesis for a textured object, from the first
   ones: `ISP` (`0xEC800000`), three words (`0x03339600`, `0xCE00xxxx`
   varying, `0x86086610` the same throughout, like Dell's `0xD6886610`:
   texture words?), `TSP` (`0x22640200`: bit 22, Gouraud, as found on the
   device), x/y x3 (screen pixels, e.g. 79.25, 292.69), z x3 (~0.998),
   1/w x3 (~0.002-0.003), then per vertex colour (`0xFF575757`) and two
   floats (texture coordinates?). Our 17-word untextured objects lack the
   three words: the list pointer's flag bits likely say which form.
   Region arrays at 0x130000 and 0x188000 (80 x 40, one empty list) are
   templates, not the frame's. Next: the list pointers' format and
   parameter base, the three words, then a textured triangle.

   **Decoded overnight (2026-10-14/15, the PC):** the three words are the
   GLES context's per-unit texture words (`0x02267434`): TEX0
   format/size (one per texture; the nibbles in bits 27-20 vary with it),
   TEX1 `(lod bits << 25) | address / 128` (`0xCE003060` -> 0x183000,
   inside the capture), TEX2 the combine word (from a table by texture
   mode). The game's textures are PVRTC (8-byte blocks). The list pointer
   for its objects: `0x41000000 | offset/4` (also `0x41600000`,
   `0x49600000` for the same objects) from parameter base 0x2B4000.

   **`rp-mbx3d` versions 7-10 on the device (unattended, owner's offer):**
   lists draw past their first object (a flat square after the dark
   background drew with `0x40800000`); the flags must match the object (a
   flat one with `0x41000000` didn't draw); the textured square in every
   form tried (ISP ours or the game's, both corner orders, flags
   `0x41000000` / `0x41600000` / `0x40800000`, Dell's table or with the
   GLES extras) never drew, and **neither did DoomGLES's own object, word
   for word** (only TEX1 pointed at our texture). So textures need state
   outside the object: likely the GLES 3D table's computed values (`0x61C`
   "low bits 7" where Dell's is `0x00020004`, `0x674`, `0x610`), set at
   `0x02261700`-`0x02261848`.

   **The GLES 3D table, read (`0x022616dc`-`0x02261848`):** pairs at
   context `+0x54`: `0x610` (`(n - 1) << 7`, or `0x30001` with no scene),
   `0x614` and `0x618` `0x1E3CE508`, **`0x61C = 0x00020007`** (Dell's
   `0x00020004`), `0x644 = 0`, `0x648 = 1`, `0x64C = 0`, `0x650` the format
   (1, 3 or 6), `0x654`/`0x658` size, `0x65C` the target, `0x660` stride/4,
   **`0x674` = a word loaded from a structure** (`[[ctx+0x794]+4]+8`, or 0),
   `0x6D0 = 1`, `0x6DC = 1`, **`0x860`** an argument, `0x60C`, then
   `(0x680, 1)`. **Version 11** (`0x61C = 0x20007`, then also `0x610 =
   0x30001`, `0x64C = 0` and the extras): still no textured object, the
   game's included. Left: `0x674`, `0x860` and state set once at start-up
   (Dell's escapes), read next. `0x674`: context `+0x794` is the current
   draw surface (stored by `eglMakeCurrent`, `0x0225e124`); `[surface+4]`
   is likely its depth buffer, and `0x674` that buffer's address (else 0),
   with `0x610`'s `(n - 1) << 7` from it. Our objects don't write depth
   (ISP bit 26), so likely not the blocker; a version with a depth buffer
   is a cheap test.

   **Read further (static, 2026-10-15).** The table above is patched per
   frame by the scene routine (`0x0225e5c4`, context `+0x4C` = the full
   table from `0x608`): it adds `0x608` (the region array) and **`0x63C`**
   (from context `+0x588`, set by `glFog` `GL_FOG_COLOR`: the fog
   colour), and sets **`0x6DC` = the clear depth as a float with bit 0**
   (`glClear` stores the clear depth at `+0x6F0`; DoomGLES: `0x3F800001`;
   Dell's table and ours: 0 or 1, a clear depth near 0). `0x610` and
   `0x674` are set only when the surface has a depth buffer: `0x610 =
   (tiles - 1) << 7` (the multiply at `0x02261708` llvm couldn't decode:
   `mulne r3, r2, r3`, columns x rows) `| 2`, bit 0 cleared when the
   frame clears depth; `0x674` the depth buffer's address. **`0x860` =
   `(cols - 1) | (rows - 1) << 16`**, the tile grid. The game's real frame
   region arrays (capture 4 at `0x265000`, capture 6 at `0x191000`): 2400
   entries, 60 x 40 like ours, per-tile lists 64 bytes apart, and **bit 29
   on every header** (ours never; Dell's flush path `0x02118af4` sets it
   on a list's tiles). Dell's one-time start-up writes hold nothing else
   for the 3D stage: its power-up is the `0x80 = 0x111` reset pulse and
   interrupt masks (ours already), the parameter memory's six pairs (ours
   already), and its flush path's `0x848 = 0x7FFF` and `0x85C | 8` around
   a render. **Version 12** tries bit 29, the clear depth, a depth buffer
   and `0x860`, alone and together.

   **Version 12 on the device (2026-10-15, `local/rp-mbx3d-run12.txt`):**
   the flat control with all of them drew (none breaks a render); no
   textured try drew, the game's object or our square, alone or
   together; every render finished in 12.48 ms as before; the depth
   buffer's first word kept its marker (no depth store at `0x674`, or not
   at its start). So the frame state is ruled out: what's missing is in
   the textured object's own path (the list pointer forms the game's
   lists also use, `0x09600000` and `0x61200000`, untried) or in memory
   the TSP reads that we don't set up.

   **The list pointer read (static, 2026-10-15):** bits 30-25 are a
   triangle mask (as the PowerVR2's strip mask), bits 24-21 the vertex
   format and count: `0x41000000` one textured triangle (26 words),
   `0x61200000` a 4-vertex strip drawing triangles 0 and 1, `0x41600000`
   / `0x09600000` a 6-vertex strip (47 words = 5 + 6 x 7) drawing
   triangle 0 or 3. Our textured object and pointer are the game's form.
   Each tile's list sits in a 64-byte block whose first word is a flat
   object (`0x408CE000`), the region entry pointing past it; `0xE0000000
   | address` links blocks. The one render register left differing was
   `0x6D8`: the GLES driver never writes it, so the game renders with the
   parameter memory set-up's `0x09000000`, where Dell's table (ours)
   writes `0x01000000`. **Version 13 on the device**
   (`local/rp-mbx3d-run13.txt`): flat control drew; with `0x6D8 =
   0x09000000`, alone or with version 12's levers, no textured object
   drew. Every register the game's 3D stage writes now matches ours, so
   the difference is in memory: what the core reads beside the object
   (the texture itself, or state the tile accelerator writes and we
   don't), or the textured object reaching the core only through the TA.
4. **M4, device under the loader: `rp-mbx3d`.** Reset the core, build one
   flat-coloured triangle with M2's encoder, start it, wait (status bit 2
   or the `0x804` write, both timed), read pixels back; then a
   per-vertex-coloured one (a gradient), then a textured one. Every wait
   has a timeout; every finding is modelled in the emulator fork or listed
   as not modellable (standing rule).
5. **M5, the tile accelerator path (static reading 2026-10-15; device
   test `rp-mbx3d` version 14).** Hand-written textured objects never
   drew with every register as the game has them, so the next step is to
   draw the way the game does: through the tile accelerator (TA), which
   writes the region array, the lists and the objects itself. Found in
   `libGLES_CL.dll` and `ddi_2700g.dll`:

   - **Three 4 KiB command windows** (Dell's map, `0x02120a54`;
     `0x0211c088`-`0x0211c104`): `0x0FFF4000` the TA's input (register
     window `+0x14000`), `0x0FFF5000` the 2D stream (`+0x15000`, ours
     already), `0x0FFF6000` (`+0x16000`) where a single `1` ends a TA
     batch (`0x022619b4`). Each is written sequentially and wraps at
     4 KiB; the write offsets are software counters in Dell's device
     structure (`+0x164`, `+0x168`, `+0x16C`), shared with the game.
     Space is the same FIFO level as the 2D engine's (`0x12C` bits 16-23
     and 30: `0x02270558`), so the 2D engine and the TA share one FIFO.
   - **The TA stream** (`0x022618d0` copies words into the window):
     a state block, then primitives. The state block's first word is a
     mask of the words that follow (`0x02267868`): bit 0 the TSP word,
     bit 1 the ISP word, bit 2 texture unit 0's three words, bit 3 unit
     1's, bit 4 one more word, bit 6 one more (`ctx+0xF0`); the clear
     sends `0x10000053`. A primitive starts with a word whose bits 31-28
     are its type (`0x4` strip, `0x5` fan, `0x7` triangle list, `0x6`
     lines, `0xE` points; `0x022685f4`), its low 17 bits the polygon
     offset (`glPolygonOffset`, `ctx+0x1C4`), then the vertices.
   - **The GLES clear, word for word** (`0x0226dff0`): `0x10000053`,
     TSP `0x22206F80`, ISP `0xE0000000`, `0`, `0`; `0x40000000` (a
     strip); three vertices of x, y, z as floats: (0, 0, 1), (2w, 0, 1),
     (0, 2h, 1); then `1` to the end window.
   - **The TA's start table** (9 pairs, context `+4`, `0x02261580`):
     `0x840` a zeroed 128 KiB-aligned buffer Dell sizes from the
     bit-interleaved (Morton) tile coordinates, 4 bytes a tile (a
     per-tile tail pointer table, likely; scene `+0x3C`), `0x844` the
     region array (scene `+0x30`/`+0x34`, double-buffered, `(tiles + 1)
     x 8` bytes), `0x848 = 0x00FF0004`, `0x84C`/`0x850` width/height
     `<< 16`, `0x854`/`0x858` `1.0e-5`, `0x85C = 0x400E1 | ((rows - 1)
     | ((cols - 1) & 0xFE) << 9) << 10`, then `0x800 = 1`. The TA
     takes its pages from the parameter memory manager (already set up);
     the 3D stage then renders with `0x608` = the same region array and
     `0x60C` = the parameter memory's base. Status bit 4 = TA done.
   - Tile grid: Dell divides the size by its tile size, rounded up to
     whole groups (`0x02117b18`); 60 x 40 here, as the game's frames.

   **Version 14** replaces the texture tries (versions 7-13, ruled
   out above) with the GLES clear sent through the TA, alone: the TA
   started with that table, the stream into `+0x14000`, the end word into
   `+0x16000`, wait for bit 4; the region array, the first tile's list
   and the object it points to logged; then the 3D stage on the TA's
   region array. If the TA path draws, the textured object goes through
   it next and its output is compared with ours word by word.

   **Version 14 on the device (2026-10-15, `local/rp-mbx3d-run14.txt`):
   the TA works.** Done (status bit 4) 1.9 ms after the start; it wrote
   the region array in the game's form (`0x20000000 | tile`, bit 29 as in
   DoomGLES's frames; list blocks 64 bytes apart) and a list (`0x400B71AD`
   then `0xF0000000`). But it wrote the list at `0x6DC6DC`, 4 MiB below
   the parameter memory we gave (12 MiB): `0xEDC6DC` with bit 23 dropped.
   **The parameter memory manager reaches only 8 MiB** (11-bit page
   numbers, likely); the game's is below it (`0x2B4000`). The 3D stage,
   reading at 12 MiB, found zeros and drew white tiles. **Version 15**
   moves the parameter memory to 6 MiB and logs the words around the
   first list.

   **Version 15 on the device (2026-10-15, `local/rp-mbx3d-run15.txt`):
   the TA path draws.** TA done in 1.9 ms; the 3D stage, with `0x608` =
   the TA's region array and `0x60C` = the parameter memory, drew the
   clear: the middle pixel went from grey to `0x00000000` (the clear's
   colour words are 0), not the white of an empty tile. The TA's object
   for the flat clear is **10 words: TSP, x/y x3, z x3** (`22206f80`, 0,
   0, 960, 0, 0, 1280, 1.0 x3) - no ISP word, no 1/w, no colours (the
   colour comes from the state words) - and its list pointer
   `0x401897C0` has 0 in bits 24-21 (our hand-written 17-word form needs
   4, the game's 26-word textured form 8): those bits are the object's
   layout. The objects landed at `0xC25F00`, 6 MiB past the parameter
   memory's start (the pointer's offset, read against `0x60C`, agrees),
   so the "8 MiB reach" read of version 14 was wrong in detail: where the
   manager puts pages relative to `0x83C`/`0x828` is still open (version
   14's lists at `0x6DC6DC` fit no simple rule). The 3D stage reads them
   right either way. Next: a textured triangle through the TA (the
   vertex words the GLES driver's transform stage sends).

   **The vertex words (static, 2026-10-15):** a draw is the state block
   (`0x02267868`), a primitive word (`0x02274a1c` `0x7...` list,
   `0x02274b80` strip/fan), the vertices, then `1` to the end window
   (`0x02274c78`). The transform stage gives each vertex 16 bytes, + 4
   with a colour, + 4 more with a second colour or fog, + 8 per texture
   unit (`0x02272064`); six emitters for 5-10 words (table `0x0227C4A4`)
   copy finished vertices into the TA window, jumping back to its start
   rather than splitting one. So a coloured, textured vertex is 7 words:
   x, y, z, 1/w, colour, u, v (the order read from the object planes,
   likely). **Version 16** sends the clear (control), then a textured
   triangle that way, with ISP "always" and with the game's.

   **Version 16 on the device (`local/rp-mbx3d-run16.txt`):** the clear
   drew again; the textured triangle (both ISP words) went through the TA
   (done in 1.2 ms) but landed in no tile: every region header
   `0x22000000 | tile` (bit 25 new: an empty tile, likely), no lists. The
   GLES driver's own textured draw (`0x0226eae4`, a full-screen copy)
   confirms the vertex order (x, y, z, 1/w, colour, u, v) and header
   `0x10000057`, and shows the two state words left: the **clip word**
   (mask bit 4, `0x0226deb4`): `1 << 29 | xmin << 21 | xmax << 14 | ymin
   << 7 | ymax` in 16-pixel units (`0x20074027` full screen; the clear's
   0 means no clipping) and **Dell's texture words** (TEX0 `0xA8800000`,
   TEX1 `0x0E000000 | address / 128`, TEX2 `0xD6886610`, TSP
   `0x22220E80`, the same as Dell's object in capture 4). **Version 17**
   tries that draw's words with the full-screen clip, the game's words
   with it, and the game's TSP with Dell's texture words.

   **Version 17 on the device (2026-10-15, `local/rp-mbx3d-run17.txt`):
   the first textured triangle.** With the GLES driver's own draw's words
   (TSP `0x22220E80`, ISP `0xE0000000`, TEX `0xA8800000` / `0x0E000000 |
   address / 128` / `0xD6886610`, full-screen clip), the TA took the
   triangle and the 3D stage drew it textured: inside it `0xFFFFFFFF`,
   `0xFF73184A`, `0xFF000000` at three points (the texel ramp decoded by
   an unknown format), outside 0; the render took 15 ms (9.6 empty). With
   the game's TSP `0x22640200`, with the game's texture words or Dell's,
   nothing reached a tile. So **the TSP word decides**: `0x22640200` (the
   word in the game's parameter memory, which versions 7-13 used in every
   hand-written textured object) is not what the TA takes or the 3D stage
   draws in our set-up. Next: the TA's textured object word for word (the
   right region entry this time), and a sweep of TEX0's format field
   against the texel ramp.

**Time box:** M1 and M2 two working sessions each. If M2 can't produce a
candidate scene format in that time, stop and bring the owner what's
known before going on. Each milestone is small enough to show on its own.

**Safety:** reads only of offsets a driver touches (the bus-hang rule);
WM probes read-only; no flash writes; the 3D core is reset before and
after each loader run, and every register changed is put back (the 2700G
keeps state between images).

**Afterwards (its own ADR):** `Mbx3dRenderer` beside `Mbx2dRenderer`
(ADR-0022 decision 4: the renderer picks per command), and the GLES layer
for Windows Mobile programs, sized by Quake 3's and DoomGLES's function
lists.

## Three questions

- **Clever or copied?** Copied: only facts about the hardware (formats,
  registers), never code. Beyond WM: a scene encoder tested on the PC; the
  3D core used for the interface too (WM used it only for games); every
  render timed against the CPU.
- **Size:** the encoder is pure Rust with no tables of its own; budgets
  set when the test image exists.
- **Modern:** tile-based rendering is what phones still use; GLES 1.1
  first, a modern API shape on top later.

## Hardware limit or choice?

| Behaviour | Limit or choice | Evidence |
|---|---|---|
| 3D requests go through the display driver's escapes | Choice (WM's driver split); RetroPane's graphics service owns both engines | `libGLES_CL.dll` imports |
| The core reads only 2700G memory | Likely hardware, as the 2D engine (measured: 32 MiB decoded) | docs/hw/2700g.md |
| No published documentation | Fact; the PowerVR2 formats are public and close | (A) |

## Alternatives considered

1. **A software OpenGL ES** (as Quake 3's bundled Hybrid "Rasteroid"
   does): works without the chip, slow, and leaves the 3D core unused.
   Kept only as a reference for testing, never as the answer.
2. **Load Windows Mobile's `libGLES_CL.dll` in RetroPane:** forbidden (no
   WM driver DLLs at runtime) and it would still need Dell's display
   driver's escapes.
3. **Probe the core blind** (write registers and see what happens):
   reading unknown offsets hangs the bus and the scene format is complex.
   Rejected: static analysis first, then targeted runs.
4. **Wait until the GLES layer is needed:** leaves ADR-0022 decision 4
   and the compositor design guessing. Rejected (owner, 2026-10-14).

## Open questions (for the owner)

1. Is the time box right (M1 and M2 two sessions each, then report)?
2. M3 needs you to start DoomGLES under Windows Mobile while a probe runs:
   fine when we get there?
