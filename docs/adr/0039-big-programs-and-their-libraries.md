# ADR-0039: A big program (a game port): run alone from a small runner, its libraries placed at build time

- **Status:** Proposed (2026-11-15, drafted by Claude for the owner's vote;
  nothing here is ruled. Four votes wait, listed under "Votes")
- **Date:** 2026-11-15

## Context

Half-Life (Xash3D FWGS with Valve's SDK; docs/design/half-life-readiness.md
line 6) is the first program RetroPane has to run that is megabytes of code
and tens of megabytes of data, and that loads parts of itself by name while
it runs. `cargo xtask xash-eval` builds it; `ports/xash/platform/lib_rp.c`
refuses every load, because this decision is not made. Every later port
(Doom, Quake: docs/design/porting-guide.md) inherits the answer.

The constraints, each read from the source or measured, not assumed:

- **Six libraries.** The engine loads its file system, a renderer, the menu
  (`mainui`), the game's server library and client library by name
  (`COM_LoadLibrary`), and the client library is written against FreeVGUI.
- **Names, both ways.** The engine asks a library for functions by name
  (`COM_GetProcAddress`): a handful for most, but **every entity class** of
  the server library (several hundred: `sv_game.c` looks each map entity's
  class name up as a function), and saved games store functions **by name**
  and need the name **from an address** back (`COM_FunctionFromName`,
  `COM_NameForFunction`). A loader that only knows an entry point is not
  enough.
- **The licence** (docs/research/half-life-licences.md; not a legal
  opinion): the game's logic (Valve's SDK licence) stays libraries of its
  own, loaded at run time, never linked into the GPLv3 engine. The file
  system, the renderers and the menu are the engine's own (GPL; the menu is
  GPL-2.0-or-later, read from its files) and may be linked with it.
  FreeVGUI is BSD 3-clause (read from its `LICENSE`) and may be linked into
  the client library.
- **Memory:** 64 MB of RAM. An image today is loaded at 0xA8008000 and must
  end below 0xA80F0000 (928 KiB, `boot/link.x`); the engine's code alone is
  1,135 KiB, so **it cannot be an ordinary image**. Free for a program: 3 MiB
  at 0xA8100000 and the RAM pool, 48 MiB at 0xA9000000
  (`rp_hal_x51v::devices`).
- **Speed:** the PXA270 has no FPU, so every float operation is a call into
  a helper; the engine and the SDK are float code throughout. Whatever
  stands between a caller and those helpers is paid millions of times a
  second. The owner's target is 60 frames a second with every trick used.
- **What RetroPane has:** `kernel/pe` (ADR-0009: PE images relocated page by
  page, exports and imports read; imports are not resolved by any caller
  yet); the shell's program loader (`shell/today/src/program.rs`: one app,
  192 KiB of file into a 256 KiB slot, an entry point, no imports, no
  exports); the loader (`boot/loader`: whole images started, each returns
  to it); the kernel's processes, in the emulator only.

## What others do (findings, read from Xash3D's source at 707596f4)

| Port | How its libraries are loaded | What it costs |
|---|---|---|
| Linux, Android, BSD (`platform/posix/lib_posix.c`) | The system's `dlopen`/`dlsym`/`dladdr` over position-independent shared objects | Every call between libraries and every global goes through a table (PLT, GOT); the system resolves names at load |
| Windows (`platform/win32/lib_win.c`, `lib_custom_win.c`) | `LoadLibrary`, or the engine's own PE loader reading a DLL out of a `.pak` into memory, relocating it and walking its imports | Relocation and import look-up at every start |
| PS Vita, Switch (`sys_psvita.c`, `sys_nswitch.c`) | Small loaders of ELF shared objects in the program itself (`vrtld`, `solder`), with a **hand-kept table** of the C library functions the main program must export ("HACKHACK: force-export stuff required by the dynamic libs") | As Linux, plus a list kept by hand that breaks when a library starts calling something new |
| Static ports (`platform/misc/lib_static.c`) | Everything in one program, a generated table of names | Fastest, but the engine's maintainers do not support it: the SDK linked into the GPL engine is the licence problem above |
| Windows Mobile (not an Xash3D port; how its games ran) | A process in a 32 MB slot, DLLs relocated at load | A game of this size does not fit its slot |

## Decision (proposed)

1. **A big program runs alone, started by a small runner.** The runner is an
   ordinary RetroPane image in Rust (as `ports/hello`: `rp-image`'s entry,
   crash path and return): it brings the port layer (`rp_port`), the C
   library and the loader below, and nothing of the game. The shell hands
   over to it and is started again when the game is left, so every byte of
   RAM but the runner's is the game's. No process, no kernel under it, no
   memory shared with anything to protect.
2. **The game is module files, placed at build time.** Each module is
   linked by our own build (`cargo xtask`) **at the address it will run at**
   in the RAM pool, against the runner's own addresses for what it imports.
   Loading a module is: read the file to its address, zero its zeroed RAM,
   run its constructors. **No relocation, no name look-up, no tables
   between caller and callee**: a call from the game into the C library is
   a plain branch, exactly as if they were one program.
3. **A module says what it was built against.** Its header carries the
   runner build's identity (a hash of the runner's exported addresses), its
   own address and sizes, and a check sum. A module that does not match
   the runner on the device, or is damaged, is refused with a plain message
   and the game does not start; it is never run on hope. Modules and runner
   are built together and ship together in the base image.
4. **Names from tables made by the build.** Each module carries its exported
   functions as two sorted tables (by name, by address), generated from the
   linked module's own symbols: `COM_GetProcAddress` and
   `COM_FunctionFromName` are a binary search, `COM_NameForFunction` the
   same the other way. No table is kept by hand.
5. **What is linked together follows the licence line, and nothing else
   splits.** Three modules:
   - **the engine**, with its file system, its renderer and its menu inside
     (all GPL; one link, so the optimiser sees across them), answering the
     engine's own load calls for those three from a table inside the module;
   - **the game's server library** (Valve's SDK), alone;
   - **the game's client library** with FreeVGUI inside (BSD), alone.
   The engine never contains the SDK and the SDK modules never contain the
   engine: they meet only through the function tables the engine and the
   game hand each other at run time, as on every other system.
6. **The C library once, the arithmetic everywhere.** What has state (the
   heap, `stdio`, files, the clock, sound, input) lives once, in the runner,
   and modules call it directly (rule 2). What has none and is hot (the
   software-float helpers, `memcpy`, the integer divides) is linked **into
   each module**, so the optimiser can inline it and the tuned float
   runtime (readiness line 9) reaches every module by rebuilding.
7. **The import list is found, not written.** The build already names what
   each part leaves unanswered (`xash-eval`'s link check); that list is
   what the runner is told to keep and export. A module asking for a name
   the runner has not fails the build, not the device.
8. **Where the files are.** The runner and the modules are part of the OS
   (the base image's flash; on the card beside the test images until
   RetroPane is installed, the same files). The game's data stays the
   user's own, in `Ports\<game>` (the owner's ruling, 2026-11-15).
9. **Leaving.** The game ends by returning to the runner, which closes the
   sound stream, puts the keypad and the codec back, and returns as every
   image does. C's `exit()` takes the same road (R253's "leaves the codec
   powered" is closed by it).
10. **Untrusted input stays untrusted.** The game's data (maps, models,
    saved games, a mod's files) is parsed by C code with the whole machine
    in its hands; a fixed layout gives an attacker known addresses. The
    build this ADR covers has no network. Before any port with a network
    (or the browser's kind of exposure) runs this way, it is revisited:
    the kernel's process is where such a program belongs (docs/security.md).

## Why it beats the originals (rule 0)

Every other port pays for *loading at run time* what was known at build
time: position-independent code and call tables (Linux, Vita, Switch),
relocation and import walks at every start (Windows), or a hand-kept
export list. Here the OS and the game are built by the same tool on the
same day, and the game is alone in RAM, so the addresses **are** known at
build time. Using that:

- calls between a module and the C library cost what a call inside one
  program costs (no PLT, no GOT, no position-independent code, which on
  ARMv5 costs a register and a load per global);
- start-up is reading about 3 MiB into place, nothing else;
- the licence line is kept by files that are separate on the disk and
  loaded at run time, without paying a dynamic linker for it.

This is what Linux's `prelink` and Apple's shared cache do for their system
libraries; ours can do it for the whole game because nothing else is in
the address space.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Start-up time, the frame's split and code size are measured in the emulator and then on the X51v against alternative B below (the same modules relocated at load): if placing at build time gains nothing measurable, B's flexibility wins |
| ✨ Modernize | Build-time placement, generated name tables, link-time optimisation across the engine's parts, a build that fails instead of a device that crashes |
| 🗄️ Legacy support | None lost: Windows Mobile programs keep the PE loader and ADR-0009; this is for programs we build |
| ⚡ Better / faster | No relocation, no call tables, the float helpers inlinable, every free byte of RAM the game's |

## Hardware limit or choice?

- The 928 KiB image area is a **choice** (where the vectors and the loader
  were put), not silicon. It is kept: moving it changes every image and the
  installed loader. The runner fits it; the game does not have to.
- The 32 MB slot of ADR-0003 is Windows CE's **choice**, kept for Windows
  Mobile programs. A game that wants 40 MB does not fit it, which is one
  reason this ADR does not make the game a process.
- No FPU is **silicon**: it is why rule 6 puts the float helpers inside
  each module.

## Alternatives considered

| | What | For | Against |
|---|---|---|---|
| **A (proposed)** | Modules placed at build time, a runner, three modules | Fastest calls and start; smallest loader (a read and a check); the licence line kept | Modules run only with the runner they were built with; a library built by someone else needs our build; known addresses (rule 10) |
| **B** | The same three modules as PE files, **relocated at load** by `kernel/pe` (ADR-0009), imports resolved by name | One file format in the OS; a module survives a new runner; could be loaded by the kernel as a process's libraries one day | Imports go through a table (a load and an indirect branch per call into the C library), or the loader patches every call site at each start; `kernel/pe`'s import side has never run; our PE packer (`xtask/src/userimg.rs`) has no exports or imports yet; more code that parses a file at start |
| **C** | Relocatable ELF objects (what `xash-eval` already links with `ld -r`) **linked on the device** at load by a small linker of our own | Direct calls like A, no fixed addresses | A linker on the device: tens of thousands of relocations and a symbol table of megabytes read at every start; the most new code to trust; link-time optimisation and dead-code removal are awkward with `-r` |
| **D** | Six separate libraries, as Linux has them | Nearest to upstream | Splits the file system, renderer and menu from the engine for no licence reason; the optimiser cannot see across; four more files, each with its copy of shared sources (the build shows the engine's `public` folder and miniz compiled into several parts) |
| **E** | One program with everything linked in (`lib_static.c`) | Simplest and fastest of all | The SDK linked into the GPL engine: the road the engine's own guide warns against. Not proposed |
| **F** | The game as a **process under the kernel** | Protection from a bad map; the long-term home for anything with a network | The kernel is not what the device runs yet (emulator only); ADR-0003's slot is 32 MB; page tables and faults between the game and its RAM cost frame time the owner asked us not to spend |

Not measured yet (each is a number A must show, in the emulator first, then
on the X51v): start-up time against B; the share of a frame spent in calls
that B would send through a table; what linking the engine's four parts as
one gains in size and time.

## Consequences

- **Easier:** the loader is small enough to read in one sitting; the next
  port gets the runner, the module format and the name tables for nothing;
  the sound and input waker (R253, R254) has a home, the runner's timer
  interrupt.
- **Harder:** the runner and every module must be built together (one
  command; the header check enforces it); a mod with its own game library
  (Opposing Force, Blue Shift) is built by our build, not dropped in from
  another system.
- **To build:** the module format and its packer in `xtask`; the loader in
  the runner (Rust, host-tested with damaged and mismatched modules); the
  engine's load calls over it (`lib_rp.c`); the runner image; the shell's
  hand-over to it and back (not built: R258's Start-menu entry).
- **Revisit when:** a port needs a network (rule 10); the kernel becomes
  what the device runs (F's "for" grows); a library built elsewhere must
  load (B).
- `docs/adr-areas.md` gets rows for `ports/xash/` and the runner when this
  record is accepted, not before.

## Votes (the owner's; none is given)

1. **How it runs:** alone from a runner, the shell leaving RAM and started
   again after (proposed), or the shell staying in RAM beside the game, or
   a process under the kernel (F).
2. **How libraries load:** placed at build time (A, proposed), relocated at
   load as PE (B), or linked on the device (C).
3. **What is one module:** the engine with its file system, renderer and
   menu, and the two game libraries apart (proposed), or six libraries (D).
4. **The C library:** once in the runner with the arithmetic inside each
   module (proposed), or a whole copy in each module.
