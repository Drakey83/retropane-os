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
  `valve` folder, in `Ports\valve` on the card (the owner, 2026-11-15:
  every port's files under `Ports`; his copy is there, 509 MiB); only the
  port ships.
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
  menu, and it runs turned (`Orientation 1`). Half-Life has more actions
  than Doom (jump, crouch, reload, the flashlight, second fire): where
  those go is part of the mock-ups below.
- **What `DoomGLES.exe` itself does (read 2026-11-15 with REA and Ghidra,
  read-only, the output kept under `local/`; findings, not the answer):**
  - **The stylus in mode 2 is mouse look, by dragging.** `use_mouse 2`
    switches on the same flag as the program's `-mlook` option. Building
    each move, with that flag on, the stylus's sideways movement turns the
    view (eight units of angle for each unit moved) and its up-and-down
    movement tilts the view up and down (a tenth of a degree a unit, kept
    between 110 down and 90 up; `invert_mouse` swaps it); with the flag
    off (mode 1) up-and-down walks forward and back instead, as Doom's
    mouse always did. Movement is the stylus's own (how far it moved
    since the last frame, scaled by `mouse_sensitivity`), not a stick
    under the thumb.
  - **`Orientation 1` turns the picture a quarter turn** (the drawing is
    rotated by +90 degrees; 2 is -90). Read from the rotation's sign, that
    is the device held with its top edge at the player's right, so the
    D-pad and buttons are under the left hand: our `RP_TURN_RIGHT`. **Confirmed
    by the owner, 2026-11-15:** "D-pad under your left thumb".
  - **The button numbers are Windows Mobile's own key codes:** 193 to 198
    are the six program buttons in order (Calendar, Contacts, Mail, Home,
    Record, wireless: docs/hw/keypad.md), 134 the D-pad's centre. So his
    settings are: the D-pad walks and turns, its centre fires, Calendar
    is always-run, Contacts is use, Mail the menu, Home the map, Record
    and wireless the previous and next weapon.
  - **A touch of the stylus fires** (the owner, 2026-11-15: "yes"). Not
    read from the program: it depends on
    which mouse button the program reports for the stylus).
- **Mock-ups for his pick** (`cargo xtask ui-preview`, drawn by the shell's
  own code and the app kit's glass, so the X51v draws exactly this; in
  `docs/design/mockups`, light and dark): the controls as DoomGLES has
  them with Half-Life's extras on the buttons (`hl-controls-doomgles`),
  or the D-pad stepping sideways with the stylus alone turning
  (`hl-controls-strafe`); and the pop-up that says where the game's files
  go, as three steps (`hl-files-steps`) or short (`hl-files-short`). The
  pop-up is drawn as a window of its own here: over the Start menu, as he
  asked, it needs a small piece of the shell that is not built. **The owner
  picked, 2026-11-15 ("A, A, D-pad under your left thumb, yes"):** the
  pop-up as three steps (`hl-files-steps`); the controls as DoomGLES
  (`hl-controls-doomgles`: the D-pad walks and turns, its centre fires,
  Calendar jumps, Contacts is use, Mail crouches, Home reloads, Record is
  the next weapon, the wireless button the menu, the flashlight and second
  fire as two glass keys in the picture's corners); the device held with
  the D-pad under the left thumb; a touch of the stylus fires as well as
  the drag looking. The backend opens its input that way
  (`ports/xash/platform/in_rp.c`); the default bindings, the pop-up over
  the Start menu and the Start-menu entry are still to build (R258).

## The lines

Status: **has** (in the OS, proven on the X51v or in the emulator as
said), **part**, **none**.

| # | What | In the OS today | How others do it | Ours, and why |
|---|---|---|---|---|
| 1 | **The evaluation** (R212): the engine built for ARMv5 with no FPU, run in the emulator on a real map, its frame's CPU split (physics, the renderer's set-up, software float) and its memory measured | **part: built, not run (2026-11-15,** [research/half-life-evaluation.md](../research/half-life-evaluation.md), `cargo xtask xash-eval`**):** all 382 files of the engine, its file system, its software renderer and the SDK's two game libraries compile with our toolchain after 140 lines of patches and a platform backend of our own over the port layer (`ports/xash`); about 2.9 MiB of programs; the engine's static RAM 12.6 MiB by default, 3.7 MiB in its low-memory mode 2. Not run: loading the libraries waits on line 6 (ADR-0039, drafted). **Since, the same day:** the ten file calls answered (R255), and the menu, FreeVGUI (inside the client library) and the MP3 decoder (mpg123's fixed-point build, no float) built in: 517 files, about 3.5 MiB of programs, nothing unanswered (R264). The owner's `valve` folder is on his SD card | Ports to phones measure on the phone after the port | Measured **before** the port, in our emulator with the performance counters' zones (`rp-today`'s profile): the float share decides how much of layers 9 and 10 the port needs, and the memory decides line 7. The result and the options go to the owner (a result is not a decision) |
| 2 | **A C and C++ compiler for the X51v** in the build (`cargo xtask`), soft-float ARMv5TE, the same on every PC | **part, built 2026-11-15** (`cargo xtask cc`, xtask/src/cc.rs): Clang 23.1.1 (the Rust compiler's own LLVM version), fetched once by its SHA-256, C compiled for the XScale and linked into a Rust image. C++ compiled too (no exceptions, no RTTI). Not yet: link-time optimisation across C and Rust, the Linux PC's archive (R249) | Windows Mobile: eVC/Visual Studio; Linux handhelds: a GCC cross toolchain; today: Clang, one compiler for every target | Clang/LLD (the back end Rust already uses here, so one code generator, link-time optimisation across C and Rust, and the same tuning flags for the XScale); pinned by version and hash in the repo as the emulator is |
| 3 | **A C library** (and the little of C++'s runtime the SDK uses: new/delete, static guards, no exceptions) | **built 2026-11-15, C++ too** (`ports/libc/cxxrt.cpp`, the whole C++ runtime in 50 lines, and since R264 the run-time cast FreeVGUI needs, `dynamic_cast`, in about 150 more with five small headers in `ports/libc/cxx`, checked in the emulator: the SDK uses no exceptions, no RTTI and no standard C++ library, read from its includes; global constructors run before main from boot/link.x's `.init_array`; a C `abort` ends the image as a failure; checked in the emulator: a global's constructor, virtual calls, new and delete, templates): picolibc 1.8.12 built by our own build from its pinned source (818 files, about 5 s, no Meson or CMake), our settings in `ports/libc/picolibc.h`, over `hal/cabi` (eight calls into one safe Rust trait; the memory routines are `rp-hal-mem`'s). **The first C program runs in the emulator** (`cargo xtask emu-chello`: printf with floats, the heap, software floats, the maths library, 64-bit divide, sort, parsing, each checked by the program; a 68 KB image). Not on the X51v yet. Not yet: a heap made for a game's tens of MB (picolibc's simple one over `sbrk` for now) | Windows Mobile: coredll; Linux: glibc or musl (both need a Linux kernel under them); embedded: newlib, picolibc | **picolibc** (BSD, built for small machines, a small stdio, float printing that can be left out) over **our own system layer in Rust** (line 4), so files, memory and time are RetroPane's and checked, not a POSIX kernel imitated. Its maths (`libm`) replaced where the evaluation shows it hot (line 9) |
| 4 | **The port layer:** what a C program calls for files, memory, time, sound, input, the screen (a small C ABI, one header) | part: the pieces exist in Rust (below); the C library's calls land in `hal/cabi`'s trait, answered by `rp_port::sys` (files, console, heap: built 2026-11-15). **The header is started, with the clock** (`ports/include/retropane.h`, 2026-11-15: `rp_time_us()`, whole microseconds since the program started, exact from the PXA270's 3.25 MHz counter (13 ticks are 4 microseconds), the counter's 22-minute wrap folded in, no float and no divide by the game; `gettimeofday` and `time` read the same clock; checked from C in the emulator: it moves forward, never back, 5 ms is 5 ms). Not on the X51v yet. **Sound and input are in the header too** (2026-11-15, lines 11 and 12). Not yet: the date (`time()` counts from 1970 as if the program started then: R252), the screen | SDL on Linux and every console port; GAPI on Windows Mobile | One thin layer shaped to what the 2700G and the X51v do well (fixed-point drawing, the three game routes of ADR-0038, the audio service's mixer), not SDL's desktop model; every port (Doom, Quake, Half-Life) draws through it, so a gain lands in all |
| 5 | **Files** as a C program reads them: open, read, seek, write, list a folder, on the card's FAT or exFAT, long names | has in Rust: `kernel/fat` (FAT12/16/32 and exFAT, proven on the X51v's cards) and, **built 2026-11-15, `ports/sys` (`rp_port::files`)**: handles over its node layer (open by path in any case and either slash, read, seek, a folder listed, a file written whole on close, mkdir, remove), host-tested on all four formats with the card's own check clean after. **Its C face, built 2026-11-15** (`rp_port::sys::System`: the console on handles 0-2, the card's files from 3, the C heap): `fopen`, `fread`, `fwrite`, `fseek`, `ftell`, `fclose` and `remove` from the C test program on an emulated card (`cargo xtask emu-chello`: a file written, read back and removed in its own folder, one made on the PC read). Not on the X51v yet. **What an engine's own file layer asks, built 2026-11-15** (found by `cargo xtask xash-eval`: ten calls nothing answered): `stat`, `opendir`/`readdir`/`closedir`, `mkdir`, `rename` (a file at the new name makes way), `dup` (a second handle with its own place: how the engine reads inside a `.pak`), `fsync`, `getcwd`, and files opened to append or to change in place (taken into the written file's room and put back whole on close); `execve` is refused (a program cannot become another here). All checked from C in the emulator and host-tested on the four formats with the card's own check clean after; the evaluation build now links with nothing unanswered but FreeVGUI. Not yet: more than one written file at a time, or one bigger than its room (a log and settings, not a map), the read-ahead; the table is moved on the stack at set-up (R250) | POSIX files over a kernel's cache | Handles over `read_range` with read-ahead sized to a card's fast run (storage-speed.md), and the game's big files (`.wad`, `.bsp`) read once into place; no general cache to pay for |
| 6 | **A program this big, run and left:** megabytes of code, tens of megabytes of data, started from the shell and back to it | **decided, 2026-11-15:** [ADR-0039](../adr/0039-big-programs-and-their-libraries.md). Accepted by the owner ("Accept all four"): its libraries as three module files placed at build time (the engine with its file system, renderer and menu; the game's server library; its client library with FreeVGUI), names from tables the build makes, the C library once in the runner. **How it runs, ruled the same day ("Yes, Game Mode") after the owner's reminder ("I want to run the OS in a game mode that we talked about, when starting a port such as this"): the game is Game Mode's one program under the resident kernel ([game-mode.md](game-mode.md)), not an image of its own with the shell ended, which Claude had drafted without reading that brief. A playable game on the X51v therefore waits for the kernel on the device and Game Mode; the loader and the engine's first start are tried as a test image meanwhile (R267). Before it: the shell loads 192 KiB programs into a 256 KiB slot (`rp-today`'s PE loader); the loader starts whole images over USB and they return to it; the kernel has a PE loader, processes and demand paging in the emulator | Windows Mobile: a process per program in a 32 MB slot; Linux: `exec` | To decide after line 1's numbers (an ADR): the game as **its own image** started from the card by the loader (the shell hands over and is started again after: every byte of RAM the game's, nothing shared to protect), against the game as a process under the kernel. Xash3D's dynamic libraries: our own PE loader loads them (the engine already has a custom loader for Windows DLLs), or one statically linked program |
| 7 | **Memory:** the engine, a map, its models and sounds inside 64 MB with the OS, textures in the 2700G's 16 MB | has: the RAM pool (50 MB after the image), the picture store, compressed pages (`zpool`, emulator) | Half-Life asked for 24 MB in 1998; the N95 port ran short on some maps | Measured in line 1; then: textures only in the 2700G's memory (never a copy in RAM), sounds decoded as they play, the map's big files read into place. The game alone in RAM (line 6's image) is the lever nobody else has |
| 8 | **Drawing:** a renderer for Xash3D (`ref_rp`) on our own 3D driver, not a GLES layer | part: `rp-mbx3d-render` (ADR-0026: our renderer, textured, blended, depth, timed on the X51v), the graphics service, the three game routes measured (ADR-0038), `boot/mbx3d-api` | Xash3D's GL and GLES1 renderers (float vertices through a general GL driver); its software renderer (the CPU draws every pixel) | Xash3D's renderer interface implemented straight on our display lists: world surfaces batched by texture and lightmap as the tile accelerator wants them, fixed-point vertices, dynamic resolution by route. What it needs that is not built: a C face for the renderer (line 4), lightmaps (two textures a surface: multi-texture or two passes, to measure), texture formats and their room |
| 9 | **A tuned software-float runtime** (apps-and-packs.md's layer 2): the compiler's float routines replaced by ones made for the XScale | none | Everyone uses the compiler's generic routines | Ours hand-tuned (and iWMMXt where it helps), measured against the generic ones on the X51v first; every float in every port gets faster with no change to the port |
| 10 | **A shared fixed-point maths library** (layer 3) for what line 1 shows hot: matrices, vectors, the renderer's set-up | none | PocketQuake, GP2X and DS ports converted by hand, each alone | One library for all ports, on iWMMXt's multiply-adds; only what the counters show hot is converted |
| 11 | **Sound:** the game's mixed samples out, at its rate, without clicks | has: the audio service (ADR-0032: the mixer, several streams at several rates, proven on the X51v in `rp-audio-s2`); the shell's chime uses it. **Its C face, built 2026-11-15** (`rp_port::sound`, `rp_snd_open/room/write/played/close` in `retropane.h`): the game writes 16-bit samples at its own rate into a Media stream's ring (no callback, no audio thread, no copy), and reads back the frames that really left the DMA engine. The C test program plays a second of 880 Hz from a 22.05 kHz stream as a game does (topping a tenth of a second up every 20 ms); the emulator's recording holds it whole, at pitch, with no gap (`cargo xtask emu-chello`). Host-tested against a model DMA engine at 30 and at 12 frames a second, through a 2 s stall (silence, nothing replayed), every refusal. Not on the X51v yet. **The mixer is woken by the game's own calls** (each sound call, each clock read): a C game is one thread and the audio host has none yet, so the stream keeps its own pace (a new option of the mixer: never put on the 15 ms interval by its write pattern) and a frame longer than what the game queued is silence. Its own waker waits on line 6 (R253) | A sound device opened by the game, mixed by the OS | A stream of the service's mixer (the game keeps its own mixing); needs only its C face (line 4) |
| 12 | **Input:** buttons, the D-pad, the stylus as the view | has: the keypad driver, the touch filter. **Their C face, built 2026-11-15** (`rp_port::input`, `rp_input_open(turn)` and `rp_input_poll` in `retropane.h`): 16-byte events, a key by the button's name or the stylus landing, moving and lifting, each with the time it was read. The stylus passes the OS's touch filter and the screen's calibration and arrives in the game's own pixels; a game in landscape says which way the device is held (top at the left, or at the right: both built, which one is the default follows from DoomGLES's reading) and the D-pad's four directions and the stylus are turned to match, in one place for every port. In the emulator the C test program gets presses (the D-pad's Up as the player's Left, a program button unchanged) and two taps at the turned pixels exactly (`cargo xtask emu-chello`). Host-tested: the corners and the D-pad in all three turns, agreeing with each other; a tap and a drag through the filter; the queue's 32 events with the oldest dropped and counted. Not on the X51v yet. Read at the game's polls and clock reads only (a press shorter than a frame can be missed: R254); no game mapping yet | Xash3D has its own on-screen touch controls and key bindings | Xash3D's own bindings fed by line 4; **the default layout as DoomGLES's** (the owner, 2026-11-15; above), landscape, every key bindable; a mock-up for the owner first |
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

## Showing the frame rate (the owner, 2026-11-15)

"I want the ability to show FPS." The engine has it built in (`cl_showfps`:
1 shows frames a second, 2 adds the lowest and highest), and it is in our
build. What the port adds: it is a setting the player can reach without a
keyboard (in the game's menu and as a bindable action), and it is saved
with the game's settings. It also serves the standing rule that every
port's frame rate is measured (30 at least, 60 the target). To build with
the default bindings (R258); nothing shows until the engine runs.

## Sixty frames a second: every lever (the owner, 2026-11-15)

"Remember Rule 0, we need to pull out every trick to make this port run
as smoothly as possible, we are shooting for 60fps." **Nothing here is
measured: the engine has not run. Each line is a lever and how it will be
judged; none is a promise.**

| Lever | What others do | Ours | Judged by |
|---|---|---|---|
| The renderer | Xash3D's GL renderers through a general GL driver; its software renderer on the CPU | `ref_rp` straight on our own 3D driver: surfaces batched by texture and lightmap as the 2700G's tile accelerator wants them, fixed-point vertices, nothing drawn twice (line 8) | frame time against the software renderer's, same map |
| Resolution by load | fixed | the three game routes of ADR-0038 (down to 240 x 320 smooth 2x) chosen as the frame time moves | frames a second held, by eye and by count |
| Float arithmetic (no FPU: every operation is a call) | the compiler's generic routines | the tuned runtime for the XScale (line 9), iWMMXt where it helps | cycles an operation, on the X51v |
| The hot maths | float throughout | fixed point where the counters show it hot: matrices, the renderer's set-up, the player's movement (line 10) | the frame's split before and after |
| The whole program optimised as one | each file alone | link-time optimisation across the engine's C and our Rust (the same LLVM), profile-guided from a recorded run in the emulator | code size and frame time |
| Sound | a second mixer under the game's | the game's mix is one stream of the OS's mixer, no copy, no thread | CPU in the mixer per frame |
| Memory and caches | default tables (12.6 MiB of static RAM) | the engine's low-memory build (3.7 MiB), the game alone in RAM, textures only in video memory, hot data kept together | cache misses and frame time |
| The CPU's clock | Windows Mobile's 624 MHz steps | the measured operating points and the 2700G's faster clock (ADR-0031, ADR-0035) while the game runs | frame time, battery, heat |
| Loading | files through a cache | maps and textures read once into place, in the card's fastest run length (his copy: 214 MiB of maps) | seconds to load a map |
| Input and the frame's pace | events through a desktop queue | the pad read at the frame's start, the frame presented on the display's own tick | milliseconds from press to picture |

| Calls between the game and the C library | a dynamic linker's tables (Linux, the Vita, the Switch) | modules placed at build time: a plain branch, no tables, no position-independent code (ADR-0039) | code size and frame time against the relocated alternative |
| The engine's parts as one | six libraries, each optimised alone | the engine, its file system, renderer and menu linked as one module, optimised across (ADR-0039) | code size and frame time |
| The kernel under the game (Game Mode) | 4 KiB pages, a walk on every miss | the game's memory mapped in 1 MiB sections, so the CPU's 32 translation entries cover it | translation misses a frame, on the X51v |
| The music | the float MP3 decoder the engine picks | mpg123's fixed-point ARM decoder, no float (built, R264) | CPU share of one track; its output against the float decoder's on the PC |

**The limit on every lever (the owner, 2026-11-15): "the most performance
possible while still having the game look and sound as it should".** A
lever that changes the picture or the sound is not taken as a speed-up:
each is checked against a reference (the software renderer's picture of
the same frame, the float decoder's samples of the same track), and a
difference goes to the owner as a choice, never in silently.

The frame rate is shown by the setting above, so every one of these is
checked on the device, not argued.

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
