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
4. **M4, device under the loader: `rp-mbx3d`.** Reset the core, build one
   flat-coloured triangle with M2's encoder, start it, wait (status bit 2
   or the `0x804` write, both timed), read pixels back; then a
   per-vertex-coloured one (a gradient), then a textured one. Every wait
   has a timeout; every finding is modelled in the emulator fork or listed
   as not modellable (standing rule).

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
