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
- **What Xash3D asks of a platform** (its porting guide, read in full in
  the source fetched 2026-11-15, commit 707596f4; summarised by search
  before): a C library, files, a clock, memory,
  a sound device taking mixed samples, input, a renderer (it has an OpenGL
  one, a GLES1 one and a software one, each a separate library), and
  **loading its parts as dynamic libraries** (the engine's maintainers do
  not support a platform that cannot).

## What the fetched source shows (2026-11-15, read, nothing built yet)

- **A port without an operating system exists already:** `engine/platform/dos`
  (input, video and system for DOS), beside `platform/stub` (no sound, no
  network) and a renderer that draws nothing (`ref/null`). They are the
  nearest models for line 4's backend and for the evaluation's first build.
- **Its software renderer is in the tree** (`ref/soft`): the evaluation can
  run a real map before `ref_rp` exists, and its time is the baseline ours
  must beat.
- **A low-memory build is an option of the engine** (`--low-memory-mode` 1
  or 2; 2 gives up network compatibility): a lever for line 7, to measure.
- **Static linking is in the engine** (`platform/misc/lib_static.c`, a table
  of the libraries' functions), **and the guide says of it** that the
  maintainers do not support a platform that cannot load dynamic libraries,
  "as supporting full-static ports are violating the GPL license in various
  ways" (the engine is GPLv3, the game logic under Valve's SDK licence).
  This is a finding for line 13 and weighs on line 6: our own loader loading
  the game's logic as a library of its own keeps the two apart; one
  statically linked program is the road the guide warns about. Not a legal
  opinion; the owner's ruling comes after line 13's reading.
- **The Half-Life SDK is built separately** (hlsdk-portable, commit
  89e83b92) and names its libraries per platform (`library-naming.md`):
  the X51v gets a name of its own in both.

## The owner's requirements for how it plays (2026-11-15)

"i want the controls to be handles like Doom Gles controls, and I want the
game to run full screen in landscape."

- **Full screen, landscape:** the game owns the whole 640 x 480 screen
  turned on its side, no title bar or softkeys; the renderer draws rotated
  (the panel is 480 x 640 upright) through the game routes of ADR-0038.
  Decides line 8's output and line 12's axes (the D-pad and the stylus are
  turned with the picture).
- **Controls as DoomGLES handles them.** What his own copy's settings file
  shows (`local/apps/Pocket Doom/doomgles.cfg`, read 2026-11-15; a finding,
  the scheme is the answer he asked for, the key numbers are not): the
  D-pad turns and moves (`key_left/right/up/down` 172-175), the hardware
  buttons are fire, use, run, next and previous weapon, the map and the
  menu (`key_fire` 134, `key_use` 194, `key_alwaysrun` 193, `key_escape`
  195, `key_tab` 196, `key_prevweapon` 197, `key_nextweapon` 198), the
  stylus is the mouse in its mode 2 (`use_mouse 2`, `mouse_sensitivity 16`,
  a touch fires: `mouseb_fire 1`), every key can be bound again in its
  menu, and it runs turned (`Orientation 1`). **Not known yet:** what the
  stylus does in mode 2 exactly (look by dragging, or a stick under the
  thumb), and which button each number is on the X51v: to read from
  `DoomGLES.exe` (REA) before line 12's default layout is drawn, with a
  mock-up for him. Half-Life has more actions than Doom (jump, crouch,
  reload, the flashlight, second fire): where those go is part of that
  mock-up.

## The lines

Status: **has** (in the OS, proven on the X51v or in the emulator as
said), **part**, **none**.

| # | What | In the OS today | How others do it | Ours, and why |
|---|---|---|---|---|
| 1 | **The evaluation** (R212): the engine built for ARMv5 with no FPU, run in the emulator on a real map, its frame's CPU split (physics, the renderer's set-up, software float) and its memory measured | none | Ports to phones measure on the phone after the port | Measured **before** the port, in our emulator with the performance counters' zones (`rp-today`'s profile): the float share decides how much of layers 9 and 10 the port needs, and the memory decides line 7. The result and the options go to the owner (a result is not a decision) |
| 2 | **A C and C++ compiler for the X51v** in the build (`cargo xtask`), soft-float ARMv5TE, the same on every PC | **part, built 2026-11-15** (`cargo xtask cc`, xtask/src/cc.rs): Clang 23.1.1 (the Rust compiler's own LLVM version), fetched once by its SHA-256, C compiled for the XScale and linked into a Rust image. C++ compiled too (no exceptions, no RTTI). Not yet: link-time optimisation across C and Rust, the Linux PC's archive (R249) | Windows Mobile: eVC/Visual Studio; Linux handhelds: a GCC cross toolchain; today: Clang, one compiler for every target | Clang/LLD (the back end Rust already uses here, so one code generator, link-time optimisation across C and Rust, and the same tuning flags for the XScale); pinned by version and hash in the repo as the emulator is |
| 3 | **A C library** (and the little of C++'s runtime the SDK uses: new/delete, static guards, no exceptions) | **built 2026-11-15, C++ too** (`ports/libc/cxxrt.cpp`, the whole C++ runtime in 50 lines: the SDK uses no exceptions, no RTTI and no standard C++ library, read from its includes; global constructors run before main from boot/link.x's `.init_array`; a C `abort` ends the image as a failure; checked in the emulator: a global's constructor, virtual calls, new and delete, templates): picolibc 1.8.12 built by our own build from its pinned source (818 files, about 5 s, no Meson or CMake), our settings in `ports/libc/picolibc.h`, over `hal/cabi` (eight calls into one safe Rust trait; the memory routines are `rp-hal-mem`'s). **The first C program runs in the emulator** (`cargo xtask emu-chello`: printf with floats, the heap, software floats, the maths library, 64-bit divide, sort, parsing, each checked by the program; a 68 KB image). Not on the X51v yet. Not yet: a heap made for a game's tens of MB (picolibc's simple one over `sbrk` for now) | Windows Mobile: coredll; Linux: glibc or musl (both need a Linux kernel under them); embedded: newlib, picolibc | **picolibc** (BSD, built for small machines, a small stdio, float printing that can be left out) over **our own system layer in Rust** (line 4), so files, memory and time are RetroPane's and checked, not a POSIX kernel imitated. Its maths (`libm`) replaced where the evaluation shows it hot (line 9) |
| 4 | **The port layer:** what a C program calls for files, memory, time, sound, input, the screen (a small C ABI, one header) | part: the pieces exist in Rust (below); the C library's calls land in `hal/cabi`'s trait, answered by `rp_port::sys` (files, console, heap: built 2026-11-15); the header for sound, input, time and the screen is not written | SDL on Linux and every console port; GAPI on Windows Mobile | One thin layer shaped to what the 2700G and the X51v do well (fixed-point drawing, the three game routes of ADR-0038, the audio service's mixer), not SDL's desktop model; every port (Doom, Quake, Half-Life) draws through it, so a gain lands in all |
| 5 | **Files** as a C program reads them: open, read, seek, write, list a folder, on the card's FAT or exFAT, long names | has in Rust: `kernel/fat` (FAT12/16/32 and exFAT, proven on the X51v's cards) and, **built 2026-11-15, `ports/sys` (`rp_port::files`)**: handles over its node layer (open by path in any case and either slash, read, seek, a folder listed, a file written whole on close, mkdir, remove), host-tested on all four formats with the card's own check clean after. **Its C face, built 2026-11-15** (`rp_port::sys::System`: the console on handles 0-2, the card's files from 3, the C heap): `fopen`, `fread`, `fwrite`, `fseek`, `ftell`, `fclose` and `remove` from the C test program on an emulated card (`cargo xtask emu-chello`: a file written, read back and removed in its own folder, one made on the PC read). Not on the X51v yet. Not yet: append and read-write opens (refused with `ENOSYS`; a game's log wants append), `opendir`/`stat` for C, the read-ahead; the table is moved on the stack at set-up (R250) | POSIX files over a kernel's cache | Handles over `read_range` with read-ahead sized to a card's fast run (storage-speed.md), and the game's big files (`.wad`, `.bsp`) read once into place; no general cache to pay for |
| 6 | **A program this big, run and left:** megabytes of code, tens of megabytes of data, started from the shell and back to it | part: the shell loads 192 KiB programs into a 256 KiB slot (`rp-today`'s PE loader); the loader starts whole images over USB and they return to it; the kernel has a PE loader, processes and demand paging in the emulator | Windows Mobile: a process per program in a 32 MB slot; Linux: `exec` | To decide after line 1's numbers (an ADR): the game as **its own image** started from the card by the loader (the shell hands over and is started again after: every byte of RAM the game's, nothing shared to protect), against the game as a process under the kernel. Xash3D's dynamic libraries: our own PE loader loads them (the engine already has a custom loader for Windows DLLs), or one statically linked program |
| 7 | **Memory:** the engine, a map, its models and sounds inside 64 MB with the OS, textures in the 2700G's 16 MB | has: the RAM pool (50 MB after the image), the picture store, compressed pages (`zpool`, emulator) | Half-Life asked for 24 MB in 1998; the N95 port ran short on some maps | Measured in line 1; then: textures only in the 2700G's memory (never a copy in RAM), sounds decoded as they play, the map's big files read into place. The game alone in RAM (line 6's image) is the lever nobody else has |
| 8 | **Drawing:** a renderer for Xash3D (`ref_rp`) on our own 3D driver, not a GLES layer | part: `rp-mbx3d-render` (ADR-0026: our renderer, textured, blended, depth, timed on the X51v), the graphics service, the three game routes measured (ADR-0038), `boot/mbx3d-api` | Xash3D's GL and GLES1 renderers (float vertices through a general GL driver); its software renderer (the CPU draws every pixel) | Xash3D's renderer interface implemented straight on our display lists: world surfaces batched by texture and lightmap as the tile accelerator wants them, fixed-point vertices, dynamic resolution by route. What it needs that is not built: a C face for the renderer (line 4), lightmaps (two textures a surface: multi-texture or two passes, to measure), texture formats and their room |
| 9 | **A tuned software-float runtime** (apps-and-packs.md's layer 2): the compiler's float routines replaced by ones made for the XScale | none | Everyone uses the compiler's generic routines | Ours hand-tuned (and iWMMXt where it helps), measured against the generic ones on the X51v first; every float in every port gets faster with no change to the port |
| 10 | **A shared fixed-point maths library** (layer 3) for what line 1 shows hot: matrices, vectors, the renderer's set-up | none | PocketQuake, GP2X and DS ports converted by hand, each alone | One library for all ports, on iWMMXt's multiply-adds; only what the counters show hot is converted |
| 11 | **Sound:** the game's mixed samples out, at its rate, without clicks | has: the audio service (ADR-0032: the mixer, several streams at several rates, proven on the X51v in `rp-audio-s2`); the shell's chime uses it | A sound device opened by the game, mixed by the OS | A stream of the service's mixer (the game keeps its own mixing); needs only its C face (line 4) |
| 12 | **Input:** buttons, the D-pad, the stylus as the view | has: the keypad driver, the touch filter; no game mapping | Xash3D has its own on-screen touch controls and key bindings | Xash3D's own bindings fed by line 4; **the default layout as DoomGLES's** (the owner, 2026-11-15; above), landscape, every key bindable; a mock-up for the owner first |
| 13 | **The licences** read in full before anything ships: Xash3D's GPLv3 beside the OS, Valve's SDK licence for the game logic, and what that means for "part of the OS" against "a pack" | **the finding is written, 2026-11-15:** [research/half-life-licences.md](../research/half-life-licences.md) (our code with the engine is allowed, under GPLv3; the game's logic must stay a library of its own, not linked into the engine; Valve's terms are "only for free"; three choices). **Ruled 2026-11-15: part of the OS** (the base image); the game's files looked for on the internal flash first, then the SD card, then the CF card (whether they fit the flash: as far as known no, about 256 MB of flash in all against several hundred MB of game; to measure on his copy) | n/a | A written finding for the owner's ruling before the port goes into the base image |

## Ready means

Lines 2, 3, 4, 5 and 11-12's C faces built and checked in the emulator with
a C test program (files read from the card, a tone through the mixer, a
key read, a triangle drawn); line 1 run with its numbers and options put
to the owner; line 6's ADR accepted; line 13's finding written. Lines 8-10
are the port's own first work, sized by line 1.

## The C program's start, as built

A C image is a Rust image (`ports/hello`): the entry, the crash path and the
return to the loader are `rp-image`'s, as in every RetroPane image; Rust
registers what answers the C library (`rp_hal_cabi::Sys`: the console, the
heap, and for a game the port layer's files) and calls the C `main`. The C
images are built only by their own commands (the `c` feature), so `cross`
and `ci` need no C compiler and a fresh checkout does not fetch 467 MB.

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
