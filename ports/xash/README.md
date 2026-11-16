# Xash3D FWGS on RetroPane: the Half-Life port's own files

What RetroPane adds to build Xash3D FWGS (the engine) and the Half-Life
SDK (the game's logic) for the Dell Axim X51v. The engine and the SDK
themselves are not in this repository: they are fetched into
`target/ports-src` at pinned commits and built by `cargo xtask xash-eval`
(`xtask/src/xash.rs`). What that build found:
[docs/research/half-life-evaluation.md](../../docs/research/half-life-evaluation.md).

- `patches/`: the four small changes to the fetched sources (a platform
  name of our own in the engine, the SDK and the menu, three places
  that assumed POSIX, and the client's dropped-frames loop bounded to the
  frames it can still change: R285, sixteen seconds of the map's start on
  the X51v), applied by the build and checked against the pinned commits.
- `mpg123/config.h`: the MP3 decoder's settings (mpg123's fixed-point
  build for an ARM with no FPU; its sources are fetched, not here).
- `platform/`: RetroPane's platform backend for the engine, over the port
  layer (`ports/include/retropane.h`): the clock, sound, input, the
  software renderer's buffer, library loading, and the answers for the
  parts of the engine left out of the build (`absent.c`).
- How it runs (ADR-0039): the build links three **module files** at the
  addresses they run at (`cargo xtask xash-modules`: the engine with its
  file system, renderer and menu; the game's server library; its client
  library with FreeVGUI), and a small runner loads them
  (`ports/runner`, a test image; `cargo xtask emu-xash` starts the engine
  in the emulator). `platform/lib_rp.c` answers the engine's load calls.
- `include/`: headers the C library does not have (sockets that answer "no
  network", the few thread calls the engine names) and stand-ins for two
  libraries left out (bzip2, MultiEmulator).

## Licence of this folder

The files in `platform/` are written against the engine's interfaces and
are linked into the engine's program, which is under the GNU General
Public License version 3 or later; they carry that notice and are
**GPL-3.0-or-later**, not the GPL-2.0-or-later of the rest of RetroPane
(the two combine, as version 3: docs/research/half-life-licences.md,
point 1). The patches are changes to the engine's files (GPL-3.0-or-later),
to its public-domain `library_suffix`, and to the SDK's `build.h` (the
Unlicense, as is the menu's copy of it). `mpg123/config.h` is RetroPane's
own (settings only; mpg123 itself is LGPL-2.1 and is linked into the
engine, which the LGPL allows). The headers in `include/` are RetroPane's own, written from
the POSIX specification's names and from the names the engine calls.

The game's logic (the Half-Life SDK, Valve's licence) is built as
libraries of its own and is never linked into the engine.
