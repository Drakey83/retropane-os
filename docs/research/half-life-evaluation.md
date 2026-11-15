# Half-Life on the X51v: the evaluation build

The first part of the evaluation (docs/design/half-life-readiness.md line
1): Xash3D FWGS and the Half-Life SDK compiled for the X51v with
RetroPane's own toolchain. **What this is:** a build, measured. **What it
is not:** a run. Nothing here has drawn a frame, in the emulator or on
the X51v, and no frame rate follows from it. Results and options for the
owner; a result is not a decision.

Run it again with `cargo xtask xash-eval` (about a minute; it writes
`target/xash-eval/report.txt`). The sources are fetched by hand into
`target/ports-src` (the owner's go, 2026-11-15) and checked against their
pinned commits: Xash3D FWGS 707596f4, its `library_suffix` submodule
0d4aec02, hlsdk-portable 89e83b92, and since the second run FreeVGUI
73bfb465, the menu `mainui` 8981066a with its `miniutl` 048a416f, and
mpg123 20c59b07.

## What was built (2026-11-15, Clang 23.1.1, picolibc 1.8.12, ARMv5TE, no FPU)

| Part | Files | Code and constants | Data | Zeroed RAM | Compiles | Still unanswered at link |
|---|---|---|---|---|---|---|
| The engine (common, server, client, with our platform backend) | 131 | 1,135 KiB | 20 KiB | 12,951 KiB | all | `execve`, `getcwd` |
| Its file system (`filesystem_stdio`) | 24 | 124 KiB | 1 KiB | 9 KiB | all | `stat`, `opendir`, `readdir`, `closedir`, `mkdir`, `rename`, `fsync`, `dup` |
| Its software renderer (`ref_soft`) | 36 | 245 KiB | 3 KiB | 7,959 KiB | all | nothing |
| The game's logic, server side (Half-Life SDK `dlls`) | 106 | 987 KiB | 33 KiB | 99 KiB | all | `mkdir` |
| The game's logic, client side (SDK `cl_dll`) | 85 | 396 KiB | 2 KiB | 222 KiB | all | 264 names of FreeVGUI's |

**The second run, the same day, with the menu, FreeVGUI and the MP3
decoder built in (R264):**

| Part | Files | Code and constants | Data | Zeroed RAM | Still unanswered at link |
|---|---|---|---|---|---|
| The engine, with the MP3 decoder inside | 132 (and mpg123's 17) | 1,269 KiB | 20 KiB | 12,951 KiB | nothing |
| of which the MP3 decoder (mpg123, fixed point) | 17 | 131 KiB | 1 KiB | 0 | nothing |
| Its file system | 24 | 124 KiB | 1 KiB | 9 KiB | nothing |
| Its software renderer | 36 | 245 KiB | 3 KiB | 7,959 KiB | nothing |
| **Its menu (`mainui`)** | 81 | 440 KiB | 3 KiB | 27 KiB | nothing |
| The game's logic, server side | 106 | 987 KiB | 33 KiB | 99 KiB | nothing |
| The game's logic, client side, **with FreeVGUI inside** | 85 (and FreeVGUI's 37) | 542 KiB | 3 KiB | 225 KiB | nothing |
| of which FreeVGUI | 37 | 143 KiB | 1 KiB | 3 KiB | nothing |

**517 files, every one compiles, and no part leaves a name unanswered.**
The engine's parts are now 2.03 MiB of code (the menu and the decoder
added 571 KiB), the game's two libraries 1.49 MiB: **about 3.5 MiB of
programs**. The engine's low-memory builds are as before (5,888 KiB and
3,804 KiB of zeroed RAM).

What the second run took:

- **The MP3 decoder is mpg123's fixed-point one, not the engine's choice.**
  The engine's own build takes mpg123's generic float decoder
  (`REAL_IS_FLOAT`); on the PXA270 every multiply of every sample would be
  a call into the software-float helpers. mpg123 has a build for exactly
  this kind of CPU (its `arm_nofpu`: `REAL_IS_FIXED` with `OPT_ARM`, an
  ARM `smull` a multiply and the synthesis in ARM assembly, `synth_arm.S`,
  tables computed beforehand). That is what is built: decoding uses no
  float. Its settings are one file of ours
  (`ports/xash/mpg123/config.h`), its sources as fetched. **Not measured:**
  how much of the CPU one 44.1 kHz stereo track takes (to measure when the
  engine runs; mpg123's faster-but-rougher and accurate-rounding variants
  are the two knobs).
- **The menu needed one more patch** (`ports/xash/patches/mainui.patch`, 21
  lines: our platform's name in its own copy of the engine's platform
  header) and draws its text itself (the game's bitmap fonts, and
  `stb_truetype` for scalable ones, which is float code: how slow opening
  a menu is waits on the run).
- **FreeVGUI is linked into the client library** (its own static option,
  `INTERNAL_VGUI_SUPPORT`; BSD 3-clause, so the licence allows it), so the
  engine loads one library less.
- **FreeVGUI asks what kind of panel it holds** (C++'s `dynamic_cast`, 12
  places), which the fifty-line C++ runtime did not have. It has now
  (`ports/libc/cxxrt.cpp`, about 150 lines more: one walk over the type
  records the compiler writes, instead of the general engine in libc++abi),
  checked in the emulator by the C++ self-check (single and multiple
  bases, a base held twice, virtual bases, a private base refused:
  `cargo xtask emu-chello`). FreeVGUI and the client library are built with
  type information, as the SDK's own Waf build has them; the rest is not.
- **Five small C++ headers of our own** (`ports/libc/cxx`: `<new>`,
  `<cstdint>`, `<cinttypes>`, `<cmath>`, `<initializer_list>`), all the
  menu and FreeVGUI ask of a C++ standard library.

**The first run's count:** every file compiled: 382 in all. The three engine parts are 1.5 MiB of
code, the game's two libraries 1.35 MiB: **about 2.9 MiB of programs**
before any tuning, which fits the base image's flash budget (the estimate
in docs/research/half-life-licences.md was "a few MB").

The engine's own low-memory builds (an option it has, not our change):

| `XASH_LOW_MEMORY` | Engine's zeroed RAM | What it gives up |
|---|---|---|
| 0 (the default) | 12,951 KiB | nothing |
| 1 | 5,888 KiB | smaller limits (entities, sounds, models) |
| 2 | 3,804 KiB | as 1, and network compatibility with other builds (the port has no network) |

## What it took

- **Three small patches to the fetched sources** (`ports/xash/patches`, 140
  lines of diff in all; a fourth of 21 lines for the menu since): a new platform name, `XASH_RETROPANE`, picked by
  the compiler flag `-D__retropane__` (the engine's platform header and the
  SDK's copy of it), a library file extension for it, and three places in
  the engine that assumed POSIX where RetroPane has the same call (the
  portability header's includes, the file sync, the input hooks).
- **A platform backend of our own** (`ports/xash/platform`, six files, about
  550 lines): the clock (`rp_time_us`), sound (the engine's ring handed to
  `rp_snd_write`, its play position from `rp_snd_played`), input
  (`rp_input_poll`: buttons as engine keys, the stylus as its mouse), a
  640 x 480 16-bit buffer for the software renderer, and library loading
  (refused for now, below).
- **No change to the C library or the compiler.** The SDK's C++ needed
  nothing beyond the fifty-line runtime already in the build.
- **Headers for what picolibc has not** (`ports/xash/include`): BSD sockets
  and name look-ups that answer "no network" (single-player runs over the
  engine's internal loopback), and the four thread calls the network code
  names (a lock is nothing with one thread; making a thread is refused).

## What is left out of the build, and why

Each is something single-player Half-Life from the user's own `valve`
folder never reaches, left out so the base image carries no code it does
not use. Each is answered with "not there" in
`ports/xash/platform/absent.c`, and each comes back by fetching its
submodule (a download: the owner's go) and dropping the stand-in.

| Left out | Used for | Cost of leaving it out |
|---|---|---|
| Ogg Vorbis and Opus decoders | mods' sounds | none for Half-Life |
| Voice chat and its codec | multiplayer | none without a network |
| bzip2 | packing files sent between server and clients | none without a network |
| MultiEmulator | Steam ID tickets for joining servers | none without a network |
| mbedTLS | HTTPS downloads | none without a network |
| The path-of-this-program code | finding the game folder beside the program | the port layer finds it (flash, then SD, then CF) |

## What stands between this build and a running map

In the order they block:

1. **Loading the libraries** (readiness line 6: its ADR is not written).
   The engine loads its file system, its renderer, the menu and the game's
   two libraries by name at run time, and the game's logic must stay a
   library of its own (the licence finding). `ports/xash/platform/lib_rp.c`
   refuses every load, plainly. The choice the ADR has to make is how the
   OS's loader gives the engine these five libraries; nothing else below
   can be tried on a device before it.
2. ~~Ten calls the port layer's C boundary does not answer~~ **Built
   the same day:** `stat`, `opendir`, `readdir`, `closedir`, `mkdir`,
   `rename`, `fsync`, `dup`, `getcwd` in `hal/cabi` over `rp_port::files`,
   with files opened to append and to change in place; `execve` (the
   engine restarting itself to change game) is refused. `cargo xtask
   xash-eval` now reports 0 calls unanswered. (The table above is the
   first run's, kept as measured.)
3. ~~FreeVGUI~~ **Built in the second run**, inside the client library.
4. ~~The menu~~ **Built in the second run** (`mainui`).
5. **A screen call in the port layer** for the software renderer's buffer
   (and, for the port proper, `ref_rp` on our own 3D driver: readiness
   line 8).
6. **The owner's `valve` folder** in `local/` (R251): there is no map to
   load without it. **This is where this work stops.**

**Fetched since (2026-11-15, the owner: "yes, do whatever you need to get
it going") and built in the second run (R264 closed):** FreeVGUI
(73bfb465), the menu `mainui` (8981066a, with `miniutl` 048a416f) and the
MP3 decoder mpg123 (20c59b07), each at the commit its repository pins and
checked by the build.

Point 1 has its proposal: [ADR-0039](../adr/0039-big-programs-and-their-libraries.md)
(drafted 2026-11-15, the owner's four votes waiting).

## Findings worth the owner's eye

- **Memory, before anything is loaded:** with the default build the engine
  and the software renderer zero 20.4 MiB of static tables between them
  (the engine's 12.6 MiB; the renderer's 7.8 MiB, of which 2.5 MiB is a
  picture buffer for a much larger screen than ours and 2.5 MiB a texture
  table). The engine's low-memory mode 2 brings its own share to 3.7 MiB.
  A map, its models and sounds come on top; that is the measurement that
  needs the `valve` folder. `ref_rp` replaces the software renderer and
  its tables.
- **The engine's own "no network" and DOS backends do not build today**
  (its stub sockets header lacks the types newer code uses; `XASH_DOS` is
  tested for and never defined). Ours do not lean on either.
- **Floats:** the engine and the SDK are float code throughout, and every
  operation is a call into the software-float routines on this CPU (the
  link lists them: add, multiply, divide, compare, convert, in both
  widths). How much of a frame that is cannot be told from a build:
  readiness lines 9 and 10 wait on the run.

## Rule 0: how this differs from the usual port

Ports of this engine to a new system fork it and bend its Waf build;
ports of C games to small devices put SDL underneath. Ours changes 140
lines of the fetched source, builds it with the OS's own compiler and C
library from a list in `xtask/src/xash.rs` (no Waf, no Python, the same on
every PC, a minute), and sits on a port layer shaped to the X51v (integer
time, the mixer's stream, the filtered stylus) instead of SDL. What would
overturn it: an engine update that moves the patched lines (the patches
then fail loudly, by design), or a need for something SDL gives that the
port layer does not (none found in this build).
