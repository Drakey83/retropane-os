# Ready for the Half-Life port: what RetroPane needs first

The owner, 2026-11-15: Half-Life (Xash3D) as part of the OS, its showcase
(R212); "start working on everything that needs done so we can start
working on the Half Life port ... work until RetroPane is ready to start
on that port ... follow rule 0".

This page is the list of what "ready" means, what the OS already has for
each line, how others do it, and ours (rule 0). It is a plan, not a
result: nothing here works until it is proven on the X51v, and every
frame rate is unmeasured until the evaluation (line 1) runs.

## What the port is

- **The engine:** Xash3D FWGS, a reimplementation of Half-Life's engine in
  C (GPLv3). **The game's logic:** Valve's Half-Life SDK (hlsdk-portable),
  C++, under Valve's SDK licence. **The game's files:** the user's own
  `valve` folder on the card; only the port ships.
- **A precedent on the same family of 3D core (found 2026-11-15, to read
  before the evaluation):** a port to the Nokia N95 (2007: a PowerVR MBX
  core, as the 2700G's) reported at about 30 frames a second, with crashes
  on some maps for want of RAM. Xash3D also runs on the PSP, the 3DS and
  the Dreamcast (16 MB). The N95 has a floating-point unit; the X51v's
  PXA270 has none, which is the open question the evaluation answers.
- **What Xash3D asks of a platform** (its porting guide, read by search
  only so far; to be read in full): a C library, files, a clock, memory,
  a sound device taking mixed samples, input, a renderer (it has an OpenGL
  one, a GLES1 one and a software one, each a separate library), and
  **loading its parts as dynamic libraries** (the engine's maintainers do
  not support a platform that cannot).

## The lines

Status: **has** (in the OS, proven on the X51v or in the emulator as
said), **part**, **none**.

| # | What | In the OS today | How others do it | Ours, and why |
|---|---|---|---|---|
| 1 | **The evaluation** (R212): the engine built for ARMv5 with no FPU, run in the emulator on a real map, its frame's CPU split (physics, the renderer's set-up, software float) and its memory measured | none | Ports to phones measure on the phone after the port | Measured **before** the port, in our emulator with the performance counters' zones (`rp-today`'s profile): the float share decides how much of layers 9 and 10 the port needs, and the memory decides line 7. The result and the options go to the owner (a result is not a decision) |
| 2 | **A C and C++ compiler for the X51v** in the build (`cargo xtask`), soft-float ARMv5TE, the same on every PC | none (the repo builds Rust only; this PC has no ARM C compiler) | Windows Mobile: eVC/Visual Studio; Linux handhelds: a GCC cross toolchain; today: Clang, one compiler for every target | Clang/LLD (the back end Rust already uses here, so one code generator, link-time optimisation across C and Rust, and the same tuning flags for the XScale); pinned by version and hash in the repo as the emulator is |
| 3 | **A C library** (and the little of C++'s runtime the SDK uses: new/delete, static guards, no exceptions) | none | Windows Mobile: coredll; Linux: glibc or musl (both need a Linux kernel under them); embedded: newlib, picolibc | **picolibc** (BSD, built for small machines, a small stdio, float printing that can be left out) over **our own system layer in Rust** (line 4), so files, memory and time are RetroPane's and checked, not a POSIX kernel imitated. Its maths (`libm`) replaced where the evaluation shows it hot (line 9) |
| 4 | **The port layer:** what a C program calls for files, memory, time, sound, input, the screen (a small C ABI, one header) | part: the pieces exist in Rust (below), no C face | SDL on Linux and every console port; GAPI on Windows Mobile | One thin layer shaped to what the 2700G and the X51v do well (fixed-point drawing, the three game routes of ADR-0038, the audio service's mixer), not SDL's desktop model; every port (Doom, Quake, Half-Life) draws through it, so a gain lands in all |
| 5 | **Files** as a C program reads them: open, read, seek, write, list a folder, on the card's FAT or exFAT, long names | has in Rust: `kernel/fat` (FAT12/16/32 and exFAT: find, read ranges, write by copy-then-swap, folders, delete; proven on the X51v's cards). No handle-based API | POSIX files over a kernel's cache | Handles over `read_range` with read-ahead sized to a card's fast run (storage-speed.md), and the game's big files (`.wad`, `.bsp`) read once into place; no general cache to pay for |
| 6 | **A program this big, run and left:** megabytes of code, tens of megabytes of data, started from the shell and back to it | part: the shell loads 192 KiB programs into a 256 KiB slot (`rp-today`'s PE loader); the loader starts whole images over USB and they return to it; the kernel has a PE loader, processes and demand paging in the emulator | Windows Mobile: a process per program in a 32 MB slot; Linux: `exec` | To decide after line 1's numbers (an ADR): the game as **its own image** started from the card by the loader (the shell hands over and is started again after: every byte of RAM the game's, nothing shared to protect), against the game as a process under the kernel. Xash3D's dynamic libraries: our own PE loader loads them (the engine already has a custom loader for Windows DLLs), or one statically linked program |
| 7 | **Memory:** the engine, a map, its models and sounds inside 64 MB with the OS, textures in the 2700G's 16 MB | has: the RAM pool (50 MB after the image), the picture store, compressed pages (`zpool`, emulator) | Half-Life asked for 24 MB in 1998; the N95 port ran short on some maps | Measured in line 1; then: textures only in the 2700G's memory (never a copy in RAM), sounds decoded as they play, the map's big files read into place. The game alone in RAM (line 6's image) is the lever nobody else has |
| 8 | **Drawing:** a renderer for Xash3D (`ref_rp`) on our own 3D driver, not a GLES layer | part: `rp-mbx3d-render` (ADR-0026: our renderer, textured, blended, depth, timed on the X51v), the graphics service, the three game routes measured (ADR-0038), `boot/mbx3d-api` | Xash3D's GL and GLES1 renderers (float vertices through a general GL driver); its software renderer (the CPU draws every pixel) | Xash3D's renderer interface implemented straight on our display lists: world surfaces batched by texture and lightmap as the tile accelerator wants them, fixed-point vertices, dynamic resolution by route. What it needs that is not built: a C face for the renderer (line 4), lightmaps (two textures a surface: multi-texture or two passes, to measure), texture formats and their room |
| 9 | **A tuned software-float runtime** (apps-and-packs.md's layer 2): the compiler's float routines replaced by ones made for the XScale | none | Everyone uses the compiler's generic routines | Ours hand-tuned (and iWMMXt where it helps), measured against the generic ones on the X51v first; every float in every port gets faster with no change to the port |
| 10 | **A shared fixed-point maths library** (layer 3) for what line 1 shows hot: matrices, vectors, the renderer's set-up | none | PocketQuake, GP2X and DS ports converted by hand, each alone | One library for all ports, on iWMMXt's multiply-adds; only what the counters show hot is converted |
| 11 | **Sound:** the game's mixed samples out, at its rate, without clicks | has: the audio service (ADR-0032: the mixer, several streams at several rates, proven on the X51v in `rp-audio-s2`); the shell's chime uses it | A sound device opened by the game, mixed by the OS | A stream of the service's mixer (the game keeps its own mixing); needs only its C face (line 4) |
| 12 | **Input:** buttons, the D-pad, the stylus as the view | has: the keypad driver, the touch filter; no game mapping | Xash3D has its own on-screen touch controls and key bindings | Xash3D's own bindings fed by line 4; a default layout made for the X51v's buttons and stylus (a mock-up for the owner first) |
| 13 | **The licences** read in full before anything ships: Xash3D's GPLv3 beside the OS, Valve's SDK licence for the game logic, and what that means for "part of the OS" against "a pack" | none | n/a | A written finding for the owner's ruling before the port goes into the base image |

## Ready means

Lines 2, 3, 4, 5 and 11-12's C faces built and checked in the emulator with
a C test program (files read from the card, a tone through the mixer, a
key read, a triangle drawn); line 1 run with its numbers and options put
to the owner; line 6's ADR accepted; line 13's finding written. Lines 8-10
are the port's own first work, sized by line 1.

## Order

1. Lines 2 and 3 (nothing else can be tried without them), with a C
   "hello" image in the emulator.
2. Line 1, the evaluation, as soon as a C program runs: its result orders
   everything after.
3. Lines 4, 5, 11, 12 (the port layer), each with its test.
4. Line 6's ADR, line 13's finding.

## What is needed from the owner

- **His go for three downloads** (the build needs them; nothing is fetched
  without it): an LLVM/Clang toolchain for Windows (from the LLVM
  project's GitHub releases), picolibc's source (github.com/picolibc), and
  Xash3D FWGS with hlsdk-portable (github.com/FWGS). Sizes and exact files
  named when asked.
- **A Half-Life `valve` folder** (his own copy) for the evaluation's map,
  kept in `local/`, never in the repo.
