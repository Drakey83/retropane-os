# How a game is ported to RetroPane

The owner, 2026-11-15: "can we use this port as a basis for how we build
all of our ports in the future?" Yes: the Half-Life port was built so that
everything but the game's own backend is shared. This page is the recipe,
read off what that port did (docs/research/half-life-evaluation.md); the
next port (Doom, Heretic, Quake: apps-and-packs.md) follows it and
improves it.

## What every port gets without writing it

| Piece | Where | What it gives |
|---|---|---|
| The compiler and the C and C++ library | `cargo xtask cc` (`xtask/src/cc.rs`, `ports/libc`) | Clang and picolibc pinned by hash, built for the X51v, the same on every PC; C++ without exceptions, with `new`/`delete`, and run-time casts (`dynamic_cast`) for the library that needs them |
| The C boundary | `hal/cabi` | Every pointer a C program hands the OS checked in one place; all the `unsafe` a port brings |
| The port layer, one header | `ports/include/retropane.h`, `ports/sys` (`rp_port`) | The clock in whole microseconds; sound as one stream of the OS's mixer; buttons and the filtered stylus, turned for landscape; the card's files through `fopen`, `open`, `stat`, `opendir` and the rest |
| The self-check | `ports/hello`, `cargo xtask emu-chello` | A C and C++ program that proves each of those in the emulator (a recorded tone, scripted presses and taps, a card) |

A gain in any of these lands in every port: that is the point of one
layer instead of SDL under each game.

## Where a game's files go (the owner, 2026-11-15)

"lets have all of the port files be housed under the SD/CF root file in a
'Port' folder then 'valve' folder for this example so I:\Ports\valve ...
doom will be I:\Ports\doom ... so on and so forth."

- **One folder for all ports at the card's root, `Ports`** (the name of
  the folder he made), **and one folder a game inside it**, named as the
  game itself names its data: `Ports\valve` (Half-Life), `Ports\doom`.
- The user drags his own copy of the game's files in there, as they are:
  no packing tool, no other step.
- The search order stays the owner's earlier ruling: the internal flash
  first, then the SD card, then the CF card; the same `Ports\<game>` path
  on each.
- A port's Start-menu entry is hidden with no card in, shown on any card;
  opened with no files, it makes its folder and says how to fill it.

## The recipe

1. **Fetch the source at a pinned commit** into `target/ports-src` (a
   download: the owner's go first), never into the repository.
2. **Read how its other ports are built** (its DOS, console and SDL
   backends, its build's options) and write down what they do, what we do
   instead, and why ours wins (rule 0), before any code.
3. **A folder `ports/<name>`** with: `patches/` (the few changes the
   fetched source needs, kept small so they fail loudly when upstream
   moves), `platform/` (the game's backend over `retropane.h`: clock,
   sound, input, picture, loading), `include/` (only what the C library
   lacks), and a `README.md` saying what is whose and under which licence.
4. **A build in xtask from a list** (as `xtask/src/xash.rs`): the pinned
   commits checked, the patches applied, every part compiled with the
   shared flags, each part linked against the C library to **name what
   nothing answers**. That list is the port's to-do, not a guess.
5. **Leave out what the game never uses on this device** (codecs,
   network, voice), each answered with "not there" in one file and each
   listed with what bringing it back costs. **A codec that stays is built
   in its fixed-point form** (the PXA270 has no FPU): Half-Life's MP3
   decoder is mpg123's own no-FPU ARM build, not the float one the engine
   picks.
6. **Keep libraries apart where a licence asks it** (the Half-Life SDK is
   never linked into its engine).
7. **Measure before tuning:** sizes and static RAM from the build; the
   frame's split from a run with the performance counters; then the
   shared speed work (below) where the counters point.
8. **Mock-ups for anything the owner sees** (controls, the no-files
   pop-up), drawn by the shell's own code, before it is built.
9. **30 frames a second at least, 60 the target, measured** (the owner's
   standing rule), and a way to show the frame rate in every port.

10. **The upstream port is a starting point, not the design** (the
   owner, 2026-11-16: "use rule 0 to get the most performance out of it
   for our device"). For every hot part the upstream code keeps (its
   renderer, its float maths, its file system, its memory and loading
   habits), write down the upstream way, the best way on the X51v, and the
   device measurement that proves ours wins. The order comes from sampling
   the program on the X51v (`-rpsample`, `cargo xtask xash-profile`), not
   from guessing: Half-Life's run 4 put six tenths of a frame in the
   software-float routines and one tenth in drawing.

## The shared speed work (every port gains from each)

- A renderer on our own 3D driver (the 2700G's display lists), not a GL
  layer and not the CPU drawing pixels.
- The tuned software-float runtime (half-life-readiness.md line 9): the
  PXA270 has no FPU, so every float operation is a call; ours replace the
  compiler's generic ones.
- The shared fixed-point maths library (line 10) for what the counters
  show hot.
- The game alone in RAM, the mixer's stream instead of a second mixer,
  textures only in video memory, big files read once into place.

## Not yet shared (to do when the second port starts)

The build recipe lives in `xash.rs` with Half-Life's lists in it: the
second port lifts the common part out (pinned sources, patches, parts,
the link check) instead of copying it (docs/reminders.md).
