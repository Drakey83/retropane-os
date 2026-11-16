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

1. ~~Loading the libraries~~ **Built 2026-11-15** (ADR-0039, R267: "The
   engine's first start" below). The engine loads its file system, its
   renderer, the menu and the game's two libraries by name at run time,
   and the game's logic must stay a library of its own (the licence
   finding): the first three are inside the engine's module and answered
   from a table the build generates; the game's two are module files the
   runner's loader puts in place.
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

Point 1 is decided and built: [ADR-0039](../adr/0039-big-programs-and-their-libraries.md)
(accepted by the owner, 2026-11-15).

## The engine's first start (2026-11-15, `cargo xtask emu-xash`)

**What this is:** the engine started in the emulator, as a test image,
far enough to say its own start-up lines. **What it is not:** a frame, a
map, or anything on the X51v. A test image is a harness: the OS starts a
port as Game Mode's one program (ADR-0039 rule 1).

Run it again with `cargo xtask emu-xash` (about 20 seconds; it writes
`target/xash-modules/report.txt`). `cargo xtask xash-modules` builds
without booting.

What is built (third run: 520 files compiled):

| File | Holds | File | Zeroed RAM | Names exported | Names imported from the runner |
|---|---|---|---|---|---|
| `server.rpm` at 0xA9000000 | the game's server library (Valve's SDK), alone | 983 KiB | 99 KiB | 499 | 35 |
| `client.rpm` at 0xA910F000 | its client library with FreeVGUI inside | 553 KiB | 223 KiB | 970 | 43 |
| `engine.rpm` at 0xA91D1000 | the engine with its file system, software renderer, menu and MP3 decoder | 1,943 KiB | 20,868 KiB | 0 (it is the program) | 92 |
| the runner (a test image) | the port layer, the C library, the loader; nothing of the game | 251 KiB of code | 2.1 MiB with its 2 MiB stack | 101 names kept for the modules | |

The set ends at 0xAA818000: 24.1 MiB of the 48 MiB pool are the modules'
(20.7 MiB of that is the engine's and the renderer's zeroed tables at the
default settings), 23.9 MiB are left for the game's heap. The engine's
low-memory builds give 7 to 9 MiB of it back (the table at the top).

What the emulator's serial line showed, in order: the runner's own lines
(its identity, where the modules end); the server and the client module
loaded through the loader and every one of their names looked up and
every function named (a check of the test image's own: the engine loads
them only once it has a game); the engine's module loaded; then the
engine's own lines: `Console initialized.`, `FS_LoadProgs:
filesystem_stdio successfully loaded` (the file system, from the table
inside the module), `FS_InitStdio: done`, and its own stop, `Xash Error:
Couldn't find game directory 'valve'` (the card has no game files: the
expected end). It came back to the runner with 1; the deepest stack was
68,632 bytes of 2 MiB.

Measured in the emulator (its clock is not the X51v's: R271): the
engine's module read, checked and its RAM zeroed in 572 ms; the server's
in 285 ms; the client's in 160 ms. The engine's module is 1,943 KiB where
its four parts alone come to 2,104 KiB: one link drops what nothing
reaches across all four.

What the work found:

- **The client library's "nothing unanswered" was wrong until now.** The
  evaluation's check link of the client stopped at a name defined twice
  (FreeVGUI brings its own `operator new`; so did our C++ run-time) before
  it had looked for undefined names, and its result was not checked. Four
  functions were missing: FreeVGUI's platform files (its clock, its mouse
  arena, its tick, its system font), which its own build takes from its
  `platform/posix` folder. Fixed: the run-time's `new` and `delete` are
  weak, as the language has them; `platform/posix` is built as fetched
  (nothing of RetroPane's is compiled into the client library: the SDK's
  licence); `clock_gettime` is answered by the C boundary; and a check
  link that fails now fails the evaluation.
- **The build refuses what would break on the device:** a name a module
  asks for that the runner has not (this is how the four above were
  found), a call that would need a veneer, an import of a Thumb function.
- The engine asks for its libraries by plain names here
  (`XASH_INTERNAL_GAMELIBS`: "server", "client", "menu", "ref_soft",
  "filesystem_stdio"); a module's file is that name with `.rpm`.

## With the game's files (2026-11-15, the same day, later)

The owner's own copy, read from his Steam folder on the PC (never copied
into the repository, never changed): `RETROPANE_HL_FILES=<the folder that
holds valve>` makes `cargo xtask emu-xash` build a 1 GiB emulated card
with his 4,262 files (509 MiB) under `Ports/valve` beside the modules and
shows what the engine says. An exploration, not a check: the plain
`emu-xash` (no game on the card) is the check, and its expected stop is
now the engine's `Changing directory to /Ports failed`.

How far it gets, each read from the serial line:

1. The engine makes `/Ports` its folder (the port layer has a current
   folder since this run: `chdir`, `getcwd`), finds the game, and mounts
   its nine texture files (`halflife.wad`: 3,116 pictures).
2. **The server module runs:** loaded through the loader (283 ms), `Dll
   loaded for game "Half-Life"`, the SDK's entity interface found by name.
3. The software renderer starts and the menu loads (both from the table
   inside the engine's module).
4. **The client module runs:** loaded (231 ms), FreeVGUI inside it starts,
   160 text messages parsed, the game's own configuration files read.
5. `Time to first frame: 104 seconds`. 100 of them are 494 look-ups of
   menu pictures the Steam copy does not have, about 200 ms each in the
   emulator (R276).
6. 27 seconds later, with no frame in between, it stops: `_Mem_Alloc: out
   of memory` while loading a menu picture (R275). The heap is the 23.9 MiB the modules
   leave; the deepest stack was 160,784 bytes of 2 MiB.

**Where the memory went (measured the same day, on the owner's "Yes,
measure first").** The runner now says the heap's size on the serial line
as it passes each MiB and when it refuses a request, and with `-rpmemlist`
the engine lists its own memory pools every five seconds and at a fatal
error (every block of 64 KiB or more with the source line that asked).

| When | Heap in use (of 23.9 MiB) | The engine's pools | The software renderer's pool |
|---|---|---|---|
| The first frame | 12 MiB | 10.5 MiB | 2.7 MiB |
| Out of memory, 27 s later | 23.9 MiB | 22.2 MiB | 14.1 MiB: 70 blocks of 128 KiB and more of the same |
| The HD layout left off the card: in the menu, steady for 3 minutes | 19 MiB | 17.0 MiB | 9.1 MiB |

The 11.5 MiB that appeared after the first frame are one thing: the
Steam copy's HD menu background (`resource/HD_BackgroundLayout.txt`, the
25th-anniversary update's: 105 pieces of 256 x 256, a picture for a 4K
wide screen). The software renderer keeps each piece at 128 KiB, 13.1 MiB
in all, and the heap ran out at piece 91. With that one file left off the
emulated card (`RETROPANE_XASH_SKIP=hd_backgroundlayout`; the owner's
copy untouched) the menu takes its 800 x 600 background (12 pieces,
1.5 MiB) and the engine **stays up in its menu**, drawing frames, its
memory not growing. Memory is not short for the menu; that picture is.
Which background the port shows is a choice about the picture, and the
owner's (R275). Not measured: what a map needs (4.9 MiB of heap are free
at the menu), any frame time.

**The background, done once on the PC (the owner: "do exactly 640 x 480";
"is there no rule 0 option?").** Two things were found first: the engine
does not fit a picture to the video mode (it keeps every piece at its
stored size and stretches it in each frame, so the mode being 640 x 480
saved nothing), and in this copy the 800 x 600 pieces are all black: the
artwork is only the HD set. So the PC's preparation of the game's files
(`xtask/src/hlprep.rs`) makes the background for the screen: the HD
picture brought to exactly 640 x 480 (an area average in linear light,
its shape kept, the sides cut off equally as the menu itself would), cut
to the screen's 16-bit colours by error diffusion (the engine's own cut
drops the low bits, which bands a dark glow), and written as one plain
picture file beside the game (`retropane/background.tga`). The menu loads
that file if it is there; nothing on the device scales or decodes.

| | The engine's own way (HD set) | Prepared on the PC |
|---|---|---|
| The renderer's pool in the menu | 14.1 MiB when the heap ran out, 13.1 MiB of it the pieces | 8.7 MiB |
| All pools / heap in the menu | out of memory | 16.55 MiB / 18 MiB of 23.9 |
| Read at a start | 105 pieces, 20 MiB | one file, 900 KiB |
| Work in a frame | every piece stretched | one picture drawn one pixel to one |

The test image can now show a frame: with `-rpshot` the engine writes its
30th frame to the card and `cargo xtask emu-xash` turns it into
`target/xash-modules/frame.png`. That is how the menu was seen for the
first time (the artwork, the logo, the eight menu lines, the right way
up). The pictures are the owner's to judge (R269) and are kept outside
the repository: they are Valve's artwork.

**The first map (2026-11-15, later still).** A test run can now say what
the engine should do (`RETROPANE_XASH_ARGS="+map c1a0"`: the runner reads
`args.txt` from its folder). With the engine's default tables the map
runs out of memory while loading. With the engine's own low-memory build
1, **as an experiment and not a setting of the port** (R277: it cuts
limits that can change the picture; sizing each table to what the game
needs is to do first), Anomalous Materials loads and plays in the
emulator: the server spawns it, the client connects, its scripted scenes
fire, and the frame read back shows the map with its chapter title.
Memory then: the engine's pools steady at 28 to 29 MiB (the software
renderer 13.2 MiB), the heap at 32 of 35.1 MiB. No frame time follows from
the emulator. The port layer now keeps several written files at once (the
engine holds one open during a map, and nothing else could be saved).

Also fixed on the way: a module's file is found from the card's root
wherever the program's current folder is; the modules no longer carry the
building PC's folder names (the engine prints its source files' names:
`-ffile-prefix-map`).

Not known from this: any frame time (nothing was timed, and the picture
goes nowhere: no screen call yet), what the menu looks like, whether a map
loads. No option for the memory is taken: R275 lists them.

Not done, not verified: nothing on the X51v; no map (R275 first); the server
and client modules' own code has not run (their names were checked, not
their constructors or a single call); the caches after a load cannot be
seen in the emulator (R271); the comparison with modules relocated at
load (R272).

## The test run for the X51v, prepared (2026-11-15, evening)

The owner gave his go for a first run on the X51v ("my go on the test
run", then "Yes, prepare it"; R278): a frame rate with the software
renderer and the low-memory experiment build, a floor and not the port's.
Nothing has run on the X51v yet. What was built for it, each piece checked
in the emulator:

- **The run's text reaches the PC.** The runner printed only to a serial
  line the X51v has no connector for. Its own lines and all the engine
  prints now go to the run log when the loader started the image (and
  from there onto a card, line by line), to the serial line only in the
  emulator's direct boot. Never both: a byte down the unconnected line is
  87 us of waiting once its buffer is full, inside the frames being timed.
- **A quiet command line.** The runner's built-in command line is the
  program's name; `-dev 2 -rpmemlist -rpshot` are what `cargo xtask
  emu-xash` writes to `rp-xash/args.txt`. The device run's whole log was
  5.5 KiB of the run log's 64 in the emulator.
- **The screen says what is running** (`ports/runner/src/panel.rs`,
  `rp_port::progress`): a title, `STEP n of 5` in large text with what the
  step is, the newest frame rate, a clock that goes on every second (from
  the program's clock reads and from every card read, so it moves while
  files are read for minutes), and in small print the command line and
  when the run ends. The engine's platform layer says its own steps in two
  kinds of plain line (`rp-step: n/5 text`, `rp-note: text`); the runner
  knows nothing of the game. The steps: 1 the runner starts (card, sound,
  modules), 2 the engine starts and reads the game's files, 3 the engine
  is up and loads the map, 4 the map plays, 5 it ends.
- **The run ends by itself**: `-rpend 120` (two minutes after the map
  began to play) or `-rplimit 1200` (twenty minutes on the run's clock,
  whatever the engine is doing), through the engine's own `quit`, so the
  codec is brought down, the board's bits put back and the log handed to
  the loader. `-rpendshot` writes the last frame to the card once, at the
  end.
- **The kit** (`cargo xtask xash-kit`, needs `RETROPANE_HL_FILES`): builds
  the runner and the three modules as the low-memory experiment whatever
  the environment says, and lays out `target/xash-kit`: the runner's image
  for `cargo xtask load`, the card's files, and `kit.txt` with each file's
  SHA-256 and the exact commands. **Every file is inside `rp-xash`**: the
  three modules, `args.txt`, and the menu's prepared background at
  `rp-xash/game/valve/retropane/background.tga` (the engine looks in its
  own write folder before the game's, so nothing has to be put beside the
  owner's files). The run writes only there too (`-rpbase /rp-xash/game
  -rodir /Ports`).
- **`cargo xtask emu-xash-kit [--cf]`**: the kit's own bytes run as the
  X51v runs them: the owner's game and the kit on an emulated SD card, the
  loader booted, the image sent as `cargo xtask load` sends it, the run
  left to its own end. Then the run log the loader sent back is checked
  (the five steps in order, at least 22 frame-rate lines after the map
  began to play, the last frame written, the codec down, `the program
  returned 0`, no overflow, no crash), the last frame is turned into a
  picture, and the whole card is compared with what the PC put on it.

**The emulator's runs of the kit** (three, all passed): without a CF card
the engine up at 69 s on the emulated clock, the map playing at 218 s, the
end at 339 s, 24 frame-rate lines, a run log of 5,478 bytes; with a CF
card in 55 s, 184 s, 304 s, 23 lines; and the kit as it stands (with the
line below about signs of life, a CF card in) 53 s, 176 s, 296 s, 23
lines, 5,605 bytes. In each the last frame shows the map (kept outside
the repository) and **all 4,262 files of the owner's game (509 MiB) are
the same byte for byte, none new, none gone, nothing new at the card's
root**. The kit built again gives the same bytes. The emulator's frame
rate (about 12 a second) is the PC's and says nothing of the X51v.

**Signs of life.** The loader ends a run that does not check in for 30 s.
The runner checks in at every card transfer, clock read and printed line,
keeps the longest gap between two of them, and says it in the run's last
lines; `emu-xash-kit` fails a run whose longest gap is half the limit. In
the emulator: 2,736 ms.

**The card log while the runner holds the SD controller** (read from
`hal/x51v/src/cardlog.rs` and `cardsink.rs`, and seen in both emulator
runs): the runner's first lines go onto the SD card; when it takes the SD
controller for the game's files the card log **moves to the CF card if
one is in** (a line says so, and every later line of the run is on that
card as it ends), and **with no CF card it pauses** (a line says so): the
rest of the run is in RAM only, and the loader writes it to the SD card
when the run comes back. So with a CF card in, a reset in the middle of
the run keeps the log up to its last line; without one, a reset loses
everything after the runner's first few lines. The rule 0 option (the
runner, which owns the SD driver for the run, writing the log's sectors
itself, so no second card is needed) is R279.

**Not known until it runs on the X51v:**

- The frame rate. It will be a floor: the software renderer (the port
  draws through the 2700G), an experiment build (R277), and the frames are
  counted, not shown (no screen call yet, so the turn and the copy to the
  screen are not in the time).
- How long the start takes. The emulated card needed about a minute for
  the engine's start (most of it look-ups of files that are not there,
  R276) and two and a half more for the map; the X51v's card and CPU are
  not the emulator's.
- Whether the loader's 30 s run limit is ever reached. A stretch of pure
  computing longer than 30 s with no card transfer, clock read or printed
  line would end the run (cleanly: back to the loader with its log and a
  report). 2.7 s in the emulator; the X51v's CPU is slower than the PC's
  by an unknown factor here, and the run's last lines will say.
- What a crash leaves on the board (R12): the runner turns the codec's
  power and the speaker amplifier on and puts both back at a clean end; a
  crash or a run the loader ends leaves them on until the next image
  takes them or the device is reset.
- Misaligned reads (R280). The emulator is not known to treat them as the
  PXA270 does, and the runtime leaves the CPU's alignment-fault bit as it
  finds it; code that reads a word from an odd address would fault or read
  a rotated word on the X51v and work in the emulator. Not seen, not ruled
  out.

## Run 1 on the X51v (2026-11-15, 22:35 to 22:56): no frame rate

The first run of the kit on the real X51v (card log `port-runner-0001`;
the owner's "ready"). The loader's `put` refused the three files over
about 900 KiB as too big (not checked before the run: a miss), so they
went on through the loader's drive mode with the owner's yes; the X51v's
own SHA-256 of all five files matched the kit that was checked.

**What happened:** the run ended by itself, cleanly, without ever reaching
the map. The engine's start, which took 63 s in the emulator, took more
than twenty minutes on the X51v: the heap stood at 5 MiB from 13.6 s to
1,243.8 s on the run's clock, the first frame had not come when the
1,200 s limit passed, and the limit ended the run at its first chance
(1,272 s). Step 3 never showed; the screen stood on step 2 with its clock
running the whole time.

**What it means:** there is no frame rate from this run, and it says
nothing about how fast the game draws. What took the time is the engine
looking for files at its start (R276: in the emulator, 494 menu pictures
the Steam copy does not have, each looked for under several names and
folders, every look-up walking folders on the card). The runner saw a
sign of life at least every 154 ms for the whole twenty minutes, so the
CPU was not computing: it was waiting on card transfers. Which reads, and
how many, is not yet counted.

**What the run did measure:**

| What | X51v | Emulator |
|---|---|---|
| engine.rpm (1,939 KiB read, checked, 9,372 KiB zeroed) | 456 ms | 860 ms |
| server.rpm (983 KiB) | 295 ms | 434 ms |
| client.rpm (553 KiB) | 212 ms | 241 ms |
| The engine's first MiB of heap | at 10.5 s | at 3.0 s |
| The stretch of look-ups (heap 5 to 6 MiB) | 1,230 s | 63 s |
| Longest stretch without a sign of life | 154 ms | 2,736 ms (with the map) |

It also showed the pieces built for the run working on the device: the
text came back through the run log, the run ended itself and returned 0,
the codec went down and the board's bits were put back as found
(`14=0008 1C=000C`), and the card log moved to the CF card (31 sector
writes, none refused, the slowest 98 ms).

**Next (nothing decided by this result):** R276 first. The rule 0 option
is to stop asking the card at all for what was already read: a folder's
names kept in RAM once listed, so a file that is not there costs no card
read; proven by counting card reads in the emulator (a count is the same
on any clock), then run 2 with the owner's go.

## Why the start took twenty minutes, and the answer (2026-11-16)

**Proven first, in the emulator** (a count of the card's read commands
is the same on any clock). The runner now counts every read command it
sends the card and what each path the engine looks up costs of them,
and says the count at each step and at the end; with `-rplooks` in a
run's command line it writes one line per look-up (thousands of lines:
the emulator's serial line, never the X51v's run log). The engine's
start to its first frame, with the game's files and the low-memory
build, as run 1 had it:

| | Before | With the folders' names kept |
|---|---|---|
| Read commands to the first frame | 321,517 (346,363 sectors) | 30,251 (55,097 sectors) |
| Spent answering look-ups | 291,728 (12,552 found, 4,217 not there) | 544 |
| Emulated start to the first frame | 70 s | 13 s |

What took them: the engine asks what `/Ports/valve/gfx/shell` is 4,002
times and `/Ports/valve/gfx` 2,016 times (about 43 read commands each),
and each answer walked every folder on the way, one read command for each
of the folder's sectors; `valve/custom` and `valve_downloads` (folders the
Steam copy does not have) another 4,211 times. The missing menu pictures
named in R276 were a smaller share than the folder walks of files that
are there.

**The answer (rule 0).** An operating system answers this with a general
cache of the card's sectors or directory entries, megabytes of RAM and
still a search through each folder's sectors per look-up; a console port
most often reads the game's whole file list into a hash table at its
start, including the folders nobody asks about. Ours keeps exactly what a
look-up needs: for each folder a program has looked into, its names
sorted for a binary search, each with where it is, what it is, its size
and its date (32 bytes and the name's own bytes; `ports/sys/src/names.rs`).
A name that is there and one that is not are both answered without the
card. The room is the runner's to size (320 KiB of the scratch RAM; the
whole `valve` folder, 4,262 files in 83 folders, is 192 KiB); a folder
that does not fit is walked from the card as before, so nothing is ever
wrong for want of room. A folder the table changes (a file written,
renamed or removed, a folder made) is forgotten and listed again; every
file test of the port layer runs three ways (no names kept, names kept,
too little room for most of them) and gives the same answers.

**The kit again, under the loader in the emulator** (`emu-xash-kit --cf`,
passed): the engine up at 7 s on the emulated clock (53 s before), the map
playing at 16 s (176 s), the end at 136 s; 24 frame-rate lines; the names
of 34 folders kept (2,561 names, 114 KiB of the 320), none refused; the
whole run 50,454 read commands, 3,709 of them for look-ups; all 4,262 files
of the owner's game the same byte for byte. The modules' bytes changed with
the runner (they are linked against its table of symbols), so run 2 needs
all three put on again; `args.txt` and the background are unchanged.

**The loader's `put` limit, now said before anything is sent:** one put
takes at most about 863 KiB (the image area less the loader's 64 KiB of
work room); `cargo xtask put` refuses a larger file on the PC with the
drive-mode steps, and the kit's `kit.txt` lists drive mode for those files.

**What the X51v should show** (a guess until run 2 says): run 1 spent
about 1,230 s on roughly 320,000 read commands, about 4 ms each. The
30,000 left would be about two minutes of start, not twenty, if each
costs the same. What the 30,000 are (file data read about two sectors a
command) is for after run 2 measures them (R281).

## Run 2 on the X51v (2026-11-16): 3.1 frames a second, the start 67 s

The kit with the folders' names kept, run with the owner's "ready"
(`port-runner-0002`; log in `local/half-life/device-run-2-port-runner-0002.txt`,
not in the repository). It ended by itself: returned 0, the board as
found, the card log on the CF card (80 sector writes, none refused).

| | Run 1 | Run 2 |
|---|---|---|
| The engine up (step 3) | not reached in 1,200 s | 67 s, after 43,535 read commands for 55,520 sectors |
| The map playing (step 4) | | 112 s, after 66,109 commands for 100,612 sectors |
| The frame rate in play | | 3.1 a second (313 to 315 ms a frame; 2.5 at its start) |
| The longest stretch without a sign of life | 154 ms | 13,255 ms (the loader's limit 30,000) |

**What the frame rate is:** the X51v's CPU running the engine's own
software renderer on the low-memory experiment build, with the picture
computed and not shown. A floor, not the port's: the port draws through
the 2700G (R282 says how the 313 ms is to be split before anything is
tuned). The 13.3 s stretch came while the map loaded (its first frame
took 16 s); it is under half the limit, as the kit's check asks of the
emulator, and is watched in the next run.

**What the start was** (the owner: "two minutes load is way too long"):
2 s to 64.5 s with the heap standing still, 43,535 read commands,
about 1.4 ms each. Counted in the emulator by the runner, which now
says what each file read, each folder entry listed and each look-up
cost (`-rpreads` writes a line for each):

| Card read commands, emulator | To the engine up | To the map playing |
|---|---|---|
| As run 2 had it | 19,691 | 31,310 |
| Of them folder listings | 18,428 (10,299 entries) | 25,573 |
| Of them file reads | 326 (855 KiB) | 1,132 (12 MiB) |
| A remembered folder listed from its names | 1,280 | 5,792 |
| And the sectors read last kept | **515** | **1,790** |

The engine lists the same folders again and again (10,299 entries before
it is up, the game has 4,262 files), and the card's code read each
entry's sector again for every entry. Two answers (rule 0):

- **A folder whose names are kept is listed from them** (`Files::next`
  over `rp_port::names`), in the names' order; a folder that changes
  while it is listed goes on after the last name given, never a name
  twice or one left out, and when the room cannot hold it again a walk of
  the card finds the next name in the same order (tested both ways).
- **The single sectors read last are kept under the file system**
  (`rp_port::recent`, 16 of them, 8 KiB): an operating system keeps a
  buffer cache of megabytes in front of the card; ours keeps only the
  sectors that repeat (folder and entry sectors; a file's data comes in
  longer reads, which pass through), and drops any a write touches, so it
  can never answer with what the card no longer holds.

What is left is mostly the game's file data. The emulator's card is not
the X51v's (4 KiB clusters against 32 KiB: R283), so the file reads'
count is the emulator's. The runner now says the card's own time beside
its commands (`in N ms`): run 3 splits the start into the card's time
and the engine's, which no count in the emulator can.

Found on the way: **an empty file failed to close** with "no room"
(the card's reservation refuses a size of 0); an empty file is now made
as an entry alone, the old one of its name gone first, and a test holds
it.

## Run 3 on the X51v (2026-11-16): the engine up at 4 s, the map at 29 s

The kit with folder listings from the kept names and the last sectors
kept (7c7652b0), run with the owner's "ready" (`port-runner-0003`; log in
`local/half-life/device-run-3-port-runner-0003.txt`, not in the
repository). It ended by itself, returned 0, the card log on the CF card.

| | Run 2 | Run 3 | The card busy (run 3) |
|---|---|---|---|
| The engine up (step 3) | 67 s, 43,535 read commands | **4 s**, 453 commands | 1.6 s |
| The map playing (step 4) | 112 s, 66,109 commands | **29 s**, 1,670 commands | 5.0 s |
| The frame rate in play | 3.1 a second | 3.1 a second (313 ms a frame) | |
| The longest stretch without a sign of life | 13,255 ms | 16,100 ms (of 30,000) | |

Estimated before the run: about 5 s and about 20 s. The engine was up
sooner; the map later, because what is left is the CPU's: the map's data
was in by 11.6 s (the heap at 32 MiB), then 16.5 s of computing while the
engine drew its first frames at 0.2 to 0.5 a second, with no clock read
or card transfer for up to 16.1 s, more than half the run limit. What it
computes there is R285, found with the same tool as the frame's split
(R282). The card is now 5 s of the 29.

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
