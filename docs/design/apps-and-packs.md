# Apps, games and packs: the owner's direction

Part of the design area: [README.md](README.md).

**Recorded 2026-10-16 from the owner's own words; specifics later.** This is a
direction, not a design: each app or pack still gets its own design doc,
mock-ups and the owner's vote before code (`docs/design/`, work style). It
gathers ideas that were scattered (DESIGN.md §6 and §9, ROADMAP Phases 4-5,
the Field Kit pack, the Quake 3 port, LegacySync) into one picture.

> "I basically want to mimic everything WM5 had to offer ... every default
> app (for nostalgia and also for legacy support), WM legacy games/app
> support, but modernize everything wherever we can, and EVERYTHING gets a
> new shiny coat of paint no matter how trivial it is." (owner)

## 1. Every Windows Mobile 5 default app, rebuilt

The owner's choice, for nostalgia and for legacy support: RetroPane ships
its own version of **every app a stock WM5 Pocket PC came with**. This is a
*feature list*, not a design to copy. Each app is RetroPane's own work, built
from public behaviour and our own artwork (`CLEANROOM.md`). The
"imitation is the last resort" rule still applies to *how* each app looks
and works: survey modern alternatives first, keep what WM did only where it
measurably wins.

| WM5 app | Keep (nostalgia, muscle memory) | Modernize (first ideas, to vote on) |
|---|---|---|
| Today screen | The layout idea: date, owner, appointments, tasks, unread | Live, GPU-composited cards; themes; dark mode (Phase 5 has started it) |
| Start menu, task switcher | Start in the corner, recent programs | Real task switcher with previews; search |
| Calendar | Day / week / month / agenda views | Smooth zoom between views; quick add; reminders through the notification centre |
| Contacts | Card list, letter strip | Fast search; photos; vCard import/export |
| Tasks | List with due dates and priority | Swipe to complete; tie-ins with Calendar |
| Notes | Typed and handwritten (ink) notes | Smooth ink on the GPU; search typed notes; export |
| Messaging (Inbox) | Mail folders, SMS-less as on the X51v | Modern mail protocols (IMAP with TLS) when networking lands (Phase 6) |
| Internet Explorer Mobile | A browser in the Start menu | RetroPane's own browser (DESIGN.md) |
| Word / Excel / PowerPoint Mobile | Open and edit simple documents | Open modern formats (.docx/.xlsx) read-first; plain-text and Markdown editing |
| Windows Media Player | Music and video player | Hardware video decode on the 2700G; modern codecs where the hardware allows |
| Pictures & Videos | Album grid, camera-less on the X51v | Fast thumbnails, pinch-like stylus zoom, slideshows |
| File Explorer | Folders, storage card, copy/paste | Two-pane mode, both cards side by side, search, previews, the PC drive view |
| Calculator | Basic calculator | Basic + scientific + unit conversion; history |
| Solitaire | A throwback to the original Klondike | Same game, new card art and smooth animation; undo; stats |
| Bubble Breaker | The puzzle game | Same rules, new paint |
| Clock & Alarms | Time, date, alarms | Multiple alarms, world clock |
| Settings (every Control Panel applet) | Same groupings so people find things | One searchable Settings app; brightness slider reaching into the night range and its schedule (backlight.md, decided) |
| Search, Help | Find files and data; help pages | One system search; help written for RetroPane |
| ActiveSync (on the device) | Sync with the PC | LegacySync (ADR-0028), see §4 |
| Terminal Services client | Remote desktop | Later, with networking; may become part of the Field Kit |

**Beyond WM5, a big emphasis (owner, 2026-10-16): a built-in Weather app.**
Windows Mobile had none by default; RetroPane makes weather a headline
feature: the hero of the Today screen (docs/design/ui.md "Weather and
icons": the sky as wallpaper, photoreal icons breaking out of their tiles) plus a full Weather app (hourly, days ahead, places).
Icon packs, the pack format and how to make one: [weather.md](weather.md).

Open: which WM5 extras count as "default" (Pocket MSN, the Dell X51v's own
utilities such as its Wi-Fi tool and display control panel). Proposed rule:
Microsoft's stock WM5 set is in; Dell's utilities become RetroPane Settings
pages for the same hardware.

## 2. A new coat of paint on everything

Nothing ships in its 2005 look, however small: every dialog, icon, sound and
error message is redrawn for the 216 dpi VGA screen in RetroPane's design
language (DESIGN.md §6, `docs/design/`). Owned art only, no WM artwork.

## 3. Themes and customization: pushed hard

The theme engine (DESIGN.md §6: *Modern* default, *Classic* homage) grows
into real customization: colour schemes, wallpapers, icon sets, fonts,
sounds, Today screen layouts, and themes installed from the card or the
pack location (§5). Themes are data (no code), so they can't harm the
system: validated and size-capped like any parser input (`docs/security.md`).
Fits the memory budget: pre-rendered chrome and atlases (design-brief.md).

## 4. Legacy apps and games, and LegacySync

- Unmodified WM5/WM6 apps and games run (DESIGN.md G2, ROADMAP Phase 4):
  GDI, GAPI and GLES programs, installed from CABs (`wceload` stand-in).
- **GLES for all games:** legacy games that draw through GDI or GAPI get
  their frames composited by the 2700G, and GLES titles run on our own purpose-built
  2700G driver (DESIGN.md §8), so every game benefits from the 3D
  core. What "GLES support for all games" means per game type is a design
  question for Phase 4-5 (for example scaling and filtering a GAPI game's
  frames on the GPU).
- **LegacySync** (ADR-0028): the Windows 11 companion that does ActiveSync's
  jobs, so old `setup.exe` installers and syncing work again from a modern
  PC. Already in progress (ROADMAP).
- Legacy programs are never locked out for lacking modern features; only
  damaged or dangerous ones are refused (`docs/security.md`).

## 5. The SD expansion pack, and one download location

Programs and extra functionality that don't belong in the flash-installed
system come as an **expansion pack on the SD card** (the mechanism:
[ADR-0018](../adr/0018-optional-packs.md), optional packs): optional apps, the
Field Kit tools (ROADMAP: terminal, hex editor, SSH ...), extra themes,
games and the game ports below. One **download location** serves the pack
and the ports; the device and LegacySync can both fetch from it. Security:
everything downloaded is signed and checked before it is installed or run
(`docs/security.md`, ADR-0017's signing design); the system in flash stays
complete without the pack (design-brief.md "What lives on the card").

**Hosting (owner, 2026-10-16):** the owner's NAS hosts the pack and the
ports, behind the owner's own security layers (never downloaded from it
directly). Every file is also signed and checked on the device before
install, so even the host isn't trusted. Open: how updates are offered.

## 6. Game ports built for RetroPane's GPU driver

Custom ports of the GPL engines, written for the best performance on
RetroPane's own 2700G driver (not through a generic GLES layer where a
direct path is faster), downloadable from the same location as the pack:

| Game | Engine source (GPL) | Note |
|---|---|---|
| DOOM, DOOM II | id's 1997 source release (GPL-2.0) | DoomGLES stays the legacy flagship test (DESIGN.md); the port is our own fast path |
| Heretic | Raven's source: released 1999-01-11 under a restrictive licence, **re-released under the GPL on 2008-09-04** (survey 2026-10-23) | HereticGLES is the legacy reference |
| Hexen | Raven's source: as Heretic's, **GPL since 2008-09-04** (survey 2026-10-23) | Added 2026-10-22 (owner): HexenGLES (the Pocket PC GLES version) is the legacy reference, as HereticGLES is for Heretic; Heretic's engine with hubs, ACS scripts and three classes, so about a week once Heretic is ported; game data (`hexen.wad`) stays the user's own |
| Quake | id's Quake 1 source (GPL-2.0) | New addition |
| Quake 2 | id's 2001 source release (GPL-2.0) | Added 2026-10-16 (owner): lighter than Quake 3 (built for 1997-98 PCs, 16-32 MB); software and OpenGL 1.x renderers to start from; game data (`pak0.pak`) stays the user's own |
| Quake 3 | id's 2005 release or ioquake3 (GPL-2.0) | Already on the ROADMAP: start-up time a goal from day one. Today's Q3CE measured, the CPU / 3D-core split and projections: [research/quake3-on-the-x51v.md](../research/quake3-on-the-x51v.md) |

**Frame rate, every port (owner, 2026-10-23): a solid 30 fps at the very
least, 60 fps the target** (the workload brief's "games: 60 fps, full
stop"). Each port's frame rate is measured on the X51v, never assumed.
Where a port can't reach 60, the trade-offs (a lower 3D resolution scaled
up by the display's video plane, fewer effects, a rate between 30 and 60)
go to the owner as measured options; none is chosen silently. The
physics as measured so far (2026-10-23): a frame at 60 fps is 16.6 ms,
and one full-screen textured layer costs the 2700G 25-29 ms at Windows
Mobile's clock, so the multi-pass engines (Quake 3, Half-Life, Turok)
can approach 60 only at a reduced 3D resolution (gate line P9: the
scaler), and then only if their game logic fits the same frame on a
624 MHz CPU with no FPU. Expected, to be measured: Descent at 60 at full
resolution; the 3D Doom family (DOOM, Heretic, Hexen) near 60, at full
resolution only with faster clocks; Quake and Hexen II near 60 at a
reduced 3D resolution; Quake 2 uncertain; Quake 3, Half-Life and Turok
30 at a reduced resolution, 60 unlikely on this hardware.

**An option to measure when a port's renderer is designed (the owner,
2026-10-24): draw with textures and blending instead of Gouraud
shading.** Run GC2S (2700g.md) found textured, blended and a full 32-bit
textured scene right at every divide-1 step to 133.25 MHz, where Gouraud
shading is wrong at every step above 74.75: so a renderer that bakes its
lighting into textures (lightmaps, pre-lit or palette-shaded textures, as
the software Doom and Quake engines already do) may run the cores at up
to 1.8-2.2x Windows Mobile's speed, against 1.4-1.5x with shading at
today's 149.50 / 2. Results, not decisions: each port measures both ways
on the X51v (the picture checked, the frame time logged), and the owner
chooses; whether divide 1 above Intel's 75 MHz is used at all is ADR-0035's
and the gate's P1/P2 ruling.
**Since (run GC2I, 2026-10-24, 2700g.md "GC2I on the X51v"): Gouraud is
right at divide 1 too, to 133.25 MHz, once the 3D core writes its picture
inside the safe region** (every 16-bit kind 20 of 20 at every step; the
32-bit kinds, too big for the region, still wrong). RGB565 Gouraud 2.0x
Windows Mobile's speed at 99.67 / 1 (a 20-minute soak clean) and 2.67x at
133.25 / 1 (not yet proven: one wrong pixel after 3 minutes). So shading
stays available at divide 1 for a port that renders into the region
(ADR-0034 amendment P: P1, the owner's), and texture lighting is one
option among several, not the only way to divide 1's speed.

Game data (WADs, PAKs) stays the user's own, except freely distributable
shareware data. Each port is likely its own repository (as the Quake 3
note says), measured against the stock-WM baseline (DESIGN.md §10).

**Future 3D game packs (owner, 2026-10-22).** True 3D games (polygon
worlds, not 2.5D) not yet in the table above that fit the X51v, each a
game pack that needs the owner's own game files, like every port. Planned
for later, after the ports above; each one's code licence is read in full
before it becomes a plan (from memory below, not yet checked), since the
pack ships the engine code even when the data is the user's.

| Game | Engine source | Fit on the X51v (624 MHz, 64 MiB, the 2700G) | Effort, and what to check |
|---|---|---|---|
| Hexen II | Raven's source release: **GPL-2.0, November 2000** (with Portals of Praevus and HexenWorld; survey 2026-10-23) | Good: Quake 1's engine | Small once Quake is ported (the same renderer and file formats) |
| Half-Life | No source from Valve; Xash3D (FWGS), a reimplementation of its engine | Plausible: built for 1998 PCs; Xash3D runs on old ARM phones | Moderate. Check: Xash3D's licence, and the game logic (Valve's HLSDK, a non-commercial licence) shipped as a separate pack |
| Descent, Descent II | Parallax's source release (1997-1999): **the Parallax licence, non-commercial use only**; DXX-Rebirth's additions GPLv3 with an exception where they conflict (read 2026-10-23: a free pack is allowed, a paid one isn't) | Excellent: full 3D on 1995 hardware | Small to moderate. Check: the licence allows a free pack |
| Turok: Dinosaur Hunter | **Turok EX** (Samuel Villarreal, github.com/svkaiser/TurokEX, 2012-14, unfinished beta): 181 of its 197 engine files carry GPL-2.0-or-later headers (checked 2026-10-22; the other 16 are imgui, tinyxml2, an unzip library, a convex-hull routine and tools, each to check or replace). Not the leaked original code (never looked at or used: CLEANROOM.md) | Fits (the N64 game ran in 4 MiB; Turok EX targets 2012 PCs: memory and speed to fit) | **Moderate (revised 2026-10-22 from "very hard, clean-room")**: the engine, game logic (AngelScript) and a converter for the 1997 PC version's files (KPFGen) exist; left: its OpenGL renderer onto RetroPane's 2700G driver, SDL2/OpenAL onto our input and audio, fitting 64 MiB, and finishing what it never did. **First: an evaluation** (what plays, what's missing) before any estimate. Needs the owner's 1997 PC copy (the Nightdive remaster's formats differ) |

**Estimates (2026-10-22, at the pace so far; rough).** Each after the OS
work, in this order, and counting on what the earlier ports build once: a
renderer layer onto RetroPane's 2700G driver, input and audio glue, and
fixed-point maths for engines written for an FPU (the X51v's CPU has
none, so float-heavy engines run on software floats until their hot paths
are converted: most of the Quake-family and later effort). DOOM and DOOM
II 2-3 weeks; Heretic ~1 week; Hexen ~1 week; Quake 3-5 weeks; Hexen II
1-2 weeks after Quake; Quake 2 4-5 weeks; Descent and Descent II 3-5 weeks
together (a fixed-point engine built for 486s: the best fit here); Turok
1.5-3 months after its evaluation (~1 week); Half-Life 2-3 months
(Xash3D's maths, 64 MiB, and its licence first); Quake 3 2-3 months. In
all about 10-15 months of port work after the alpha.

Considered and left out (2026-10-22): Return to Castle Wolfenstein,
Enemy Territory, Jedi Outcast and Jedi Academy, and Doom 3 (too heavy for
64 MiB and the 2700G), and the 2.5D games (Wolfenstein 3D, Strife,
Rise of the Triad, the Build games, Marathon), which the owner didn't ask
for here (Hexen joined §6's ports on 2026-10-22). Quake 3 stays the stretch goal it is (about 64 MiB and a real GPU).

### Floating point in the ports (discussed 2026-10-22; for the owner's confirmation)

The X51v's CPU (PXA270) has no floating-point unit: a hardware fact no
driver changes. Every float operation is a software routine costing tens to
over a hundred cycles (perf.md's rules). Nothing in our 2700G findings shows
a geometry unit that could do the maths either: its 3D core takes vertices
the CPU already transformed. Doom, Heretic, Hexen and Descent are integer
or fixed-point engines; the Quake family, Half-Life (Xash3D) and Turok EX
were written for FPUs.

**Three layers, built once for every port:**

1. **The driver speaks fixed point:** the renderer API the ports draw
   through takes fixed-point coordinates (as OpenGL ES Common-Lite did) and
   transforms in integer maths, so nothing drawn passes through a float.
2. **A tuned software-float runtime:** the compiler's generic float
   routines replaced by ones hand-tuned for the XScale; every remaining
   float operation in every port gets faster with no change to the port
   (its gain measured on the X51v first: a backlog item, not a gate line).
3. **A shared fixed-point maths library on iWMMXt** (the PXA270's SIMD
   unit: several 16-bit multiply-adds an instruction) for the hot paths
   ports call: matrices, vectors, interpolation, physics.

What remains per port is converting only what the performance counters
still show as hot. Not automatic: a blanket float-to-fixed conversion would
break physics that need float's range.

**What others did without an FPU (survey of known practice, 2026-10-23;
each to be measured on the X51v before it is used):**

- **Fixed point by kind of value**, not one format (Q3CE's six;
  PlayStation 1 games on its fixed-point GTE coprocessor; Nintendo DS and
  GBA games; OpenGL ES 1.x's Common-Lite profile, a fixed-point API made
  for chips like ours; J2ME's M3G).
- **Integer SIMD for vector maths:** iWMMXt multiply-accumulates four
  16-bit values an instruction (`WMADD`): transforms, lighting, skinning.
- **No divides in hot code:** ARMv5 has no divide instruction. Reciprocal
  tables refined by one Newton-Raphson step, a reciprocal computed once
  per object or frame and multiplied after (Doom's `FixedDiv` already
  avoids most).
- **Tables for the costly functions:** sine and cosine, square root and
  its inverse, arctangent, at the precision each use needs.
- **Do it once, ahead of time:** game data converted to fixed point at
  install or first run (by the device or LegacySync on the PC): map
  geometry, model vertices, lighting baked in.
- **Smaller numbers where they suffice:** 16-bit vertices relative to a
  model's bounding box (Quake's models are already 8-bit).
- **A faster software float for what remains:** hand-written ARM
  assembly routines (GCC's `ieee754-sf.S` for ARM; Qfplib's measured gains
  over generic code on Cortex-M0), and a relaxed float for games (no
  denormals, NaNs or rounding modes) where IEEE exactness isn't needed.
  Rust's own software float is generic code: the first measurement is how
  far behind an ARM assembly version it is.
- **Floats built from integers for the 3D core:** the 2700G's vertex
  words are IEEE floats; ARMv5's `CLZ` normalises an integer into one in
  a few instructions.
- **Find the floats that matter:** the event counters and the cross
  build's float-helper check (perf.md) show which calls are hot.

**Every one of these is implemented** (the owner's directive,
2026-10-23), in the shared libraries the ports build on (layers 2 and 3
above), each with its own benchmark on the X51v and an exactness check
against a float build (Q3CE's method), not only the ones a first port
happens to need.

**The Doom family means the 3D (GLES) versions** (owner, 2026-10-23):
DOOM, DOOM II, Heretic and Hexen as hardware-rendered 3D ports, like
DoomGLES, HereticGLES and HexenGLES, not the 2.5D software renderer. Their
game logic stays the integer engine; their renderer is ours, on the
2700G, in fixed point. Like every 3D port they are fill-bound: walls,
floors and sprites at 480 x 640 are at least one full textured layer
(25-29 ms at Windows Mobile's clock), so 60 fps at full resolution needs
the faster clocks (above) or a reduced 3D resolution; sector light is a
flat tint a polygon, which avoids the Gouraud limit found by run GC.

**Reuse first: earlier ports' conversions.** These engines were ported to
FPU-less ARM before (Pocket PC, GP2X, the Nintendo DS), and those ports are
GPL like the engines, so their fixed-point work can be reused with credit
(CLEANROOM.md's rule covers Dell's and Microsoft's code, not GPL engines).
Before each port is planned, a short survey: two to four earlier ports for
FPU-less ARM, each one's licence read in full, what it converted and what
it gave up (some cut precision for speed: wobbly physics, jittery aim),
then measured against layers 2 and 3 on the X51v; the faster faithful one
is kept. **Candidates to check, from memory, none verified yet:** Quake:
PocketQuake (Pocket PC), GP2X and Nintendo DS ports; Quake 3: Q3CE's own
source (it ran on XScale Pocket PCs); Quake 2, Hexen II: Pocket PC or GP2X
ports, if any; Half-Life (Xash3D), Turok EX: likely nothing (FPU targets),
so the layers matter most. **The surveys' results are recorded in this
section** as they are done.

**The surveys (overnight 2026-10-22/23; sources read on the web and in the
repositories named, nothing copied into this repository yet; each to be
read in full before its port is planned):**

| Engine | Floating point today | Earlier work for FPU-less CPUs, and what to reuse | Licence |
|---|---|---|---|
| DOOM, Heretic, Hexen | **None needed:** all integer; 16.16 fixed point throughout (`fixed_t`, `FixedMul`/`FixedDiv`, the latter needing a 64-bit multiply or a split), no float anywhere in the engine | Nothing to convert; the ports' work is the renderer onto our 2700G driver | GPL (Doom 1999; Heretic and Hexen 2008) |
| Descent, Descent II | **None needed:** a fixed-point engine (`fix`) | Nothing to convert | Parallax non-commercial + GPLv3 additions |
| Quake | Float-heavy (game, physics, transforms); its software renderer's inner span loops are already fixed point | **PocketQuake** (Dan East, Pocket PC, 2001; GPL): a 16.16 conversion for ARM, and a later community rewrite (a PocketMatrix forum post) with its multiply in ARM assembly, ~1.4-1.5x faster than the original and fixing several of its graphical bugs, tested on Symbian. **Handheld Quake** (SourceForge, CVS) built on East's changes for Qt Palmtop and the iPAQ. Nintendo DS ports (QuakeDS, Q1DS, DSQuake) exist; their float handling isn't documented in what was found. First source to read: PocketQuake's | GPL-2.0 (id) |
| Hexen II | Quake's engine, so Quake's floats | A Palm OS port (ZHexen 2, MetaViewSoft, on yoyofr92's work: ARM CPUs without an FPU) exists; its float handling not documented. Hammer of Thyrion (uHexen2) is the maintained GPL engine to start from; Quake's conversion should carry over | GPL-2.0 (Raven, 2000); uHexen2 GPL-2.0-or-later |
| Quake 2 | Float-heavy, as Quake | **Pocket Quake 2** (Dan East, January 2002; ~5 fps on a ~200 MHz iPAQ, MIPS and ARM builds); whether it converted to fixed point isn't documented, and its source wasn't found. Quake2DS (Nintendo DS) moved most drawing to the DS's 3D hardware; source and float handling not found. Likely little to reuse: Quake's and Quake 3's conversions are the better guides | GPL-2.0 (id, 2001) |
| **Quake 3** | Float-heavy | **Q3CE** (Christien Rioux, NoctemWare, 2005-2008; github.com/crioux/q3ce, 4 commits, the last 2008; GPLv2 per its listing, no licence file in the repository: to confirm): **the whole engine converted to fixed point**, in C++. Six fixed-point formats by kind of value (12.20, 21.11 for coordinates, 8.24 for unit values, 24.8, 16.16, and a 64-bit 32.32), a range check in debug builds (`VERIFY_FIXEDREP`), ARM assembly for the core operations, and a switch (`FIXED_IS_FLOAT`) that builds the same code with floats to compare. Used in 87 files: renderer, bots, game, UI, sound. Also: OpenGL ES through a wrapper, the X51v's 2700G supported, textures halved, the mod VM removed for speed (modules built in), memory moved to mapped files, many original bugs and overflows fixed. **The strongest find of the survey:** both the conversion and its method (per-kind formats, checked ranges, a float build to compare against) | GPL-2.0 (id) |
| Half-Life (Xash3D FWGS) | Float throughout; written for CPUs with FPUs (Android, PCs); its software renderer comes from Quake 2's (fixed-point inner loops) | No FPU-less port found: the three layers carry this one, as planned | **Xash3D: GPLv3** (CMake files MIT); the game logic (Valve's HLSDK, hlsdk-portable) under Valve's SDK licence: free distribution only, with the licence; it ships as its own pack, as planned |
| Turok (Turok EX) | Float throughout (a 2012 PC engine: "float" in ~400 files, last commit 2014-06-20) | None: the layers carry it, after its evaluation | GPL-2.0-or-later headers on most files (read 2026-10-22) |

**What the surveys change:**

1. **Quake 3 gets cheaper:** Q3CE's conversion exists, works on XScale
   (15-25 fps on the X50v per its forum reports) and is GPL. Its estimate
   (2-3 months) leaned on converting the engine ourselves; reading,
   measuring and adopting Q3CE's conversion should take much of that
   off. Re-estimated when the port starts, after reading it in full.
2. **Q3CE's method shapes layer 3** (the shared fixed-point library): a
   format per kind of value instead of one 16.16, overflow checks in
   debug builds, and a float build of the same code to compare against.
   It is the same discipline as the project's testing rules (both
   directions, disagreeing inputs).
3. **Quake gets a head start** from PocketQuake's 16.16 conversion and
   its community rewrite: to read before converting anything.
4. **A licence point for the ports' SDK:** Xash3D is GPLv3, RetroPane is
   GPL-2.0. A GPLv3 port can't be combined with GPL-2.0-only code. The
   SDK the ports build against (the repositories plan below) needs a
   licence that GPLv3 ports can use: GPL-2.0-or-later, or a linking
   exception. A decision for the owner when the SDK is published.
5. **Descent's licence is non-commercial:** a free pack only, never sold
   or bundled with anything sold.
6. **Correction to this page:** Heretic's and Hexen's source were first
   released in 1999 under a restrictive licence and became GPL only on
   2008-09-04; the table above now says so.
7. **Not a source of fixed point but worth a look:** PocketQuake for the
   Analogue Pocket (2026, github.com/thinkelastic/PocketQuake) runs on a
   RISC-V soft CPU *with* an FPU, but moves span drawing and model
   transforms into hardware: the same split RetroPane plans with the
   2700G.

**The ports' repositories (proposed 2026-10-22, the owner's vote to
come):** one GitHub organization holding the OS and the ports; one
repository per engine family (Doom, Heretic and Hexen; Quake and Hexen II;
Quake 2; Quake 3; and each later pack); the OS publishes a versioned SDK
(the GPU driver's API, input, audio, file access) on its GitHub Releases,
which each port pins; a shared library repository for what crosses
families (the renderer layer, the fixed-point maths, input and audio
glue); reusable GitHub Actions workflows and a port template repository;
the organization's `.github` repository for the shared contributing,
security and conduct files; one documentation site for hosting, install
and download-location instructions, each port's README linking to it.

## 6b. The Emulation pack (owner, 2026-10-18)

**Owner:** an **emulation program pack** as a download pack on the card
(§5), starting with **PocketCultMAME**, which the owner uses for GLES
games on the Axim. "At the very least, document my emulation program
pack."

**PocketCultMAME (researched 2026-10-18, desk only).** Site:
<https://www.pocketcult.com/> (`pocketcultmame.com` doesn't resolve), MAME
page <https://www.pocketcult.com/mame.php>.

| Fact | Source |
|---|---|
| A Pocket PC port of **MAME 0.34-0.36** ("opens the way to MAME 0.36"), with fast cores DrZ80 (Z80) and Cyclone (68000); ARM, Windows Mobile 2003 or later | pocketcult.com |
| Versions: **0.4a** (full release; `pcmame04a.zip`, and `pcmame04a_noasm.zip` with the original CPU cores), **0.5 WIP** (`pcmame05wip.zip`), a NeoGeo add-on for Metal Slug (`pcneomame05wip.zip`, "very unstable", needs 500 MHz or more) | mame.php |
| An **"Intel 2700g double pixel"** mode for the Dell Axim X51v. GLES or OpenGL ES isn't named on the pages read; the double pixel is likely the 2700G's scaler, not its 3D core (to check on the device) | mame.php |
| **No source code published.** Licence: "not licensed yet, but can be freely distributed and copied". MAME of that age also carries the old MAME licence's non-commercial terms | pocketcult.com; HPC:Factor's listing of 0.4a |
| Same site: **PocketCaprice** (Amstrad CPC), **GPL with source** | pocketcult.com |

**What that means for RetroPane:**
- **As a legacy program in the pack: yes.** It runs as a Windows CE binary
  under RetroPane's CE personality (§4; its 2700G mode through our GPU
  driver's legacy path), shipped as the author's original archive, since
  "freely distributed and copied" allows it non-commercially.
- **As a native RetroPane emulator: no.** Without source it can't be
  ported, fixed or made to use our renderer.
- **Native emulators** come from open sources instead, built for
  RetroPane's 2700G driver like the game ports (§6), with the §6a rule
  (RGB565 dithered or better, a picture curve per emulator). Candidates,
  each licence to confirm against its own source before use:

| System | Candidate | Licence |
|---|---|---|
| Arcade | MAME 0.37b5-era ports (MAME4ALL lineage): sized for 200-600 MHz ARM | old MAME licence, non-commercial (to verify) |
| Arcade, NeoGeo | FinalBurn Neo | non-commercial (to verify) |
| NES | FCEUX | GPL-2.0 |
| Game Boy / Color | Gambatte | GPL-2.0 |
| Game Boy Advance | gpSP | GPL-2.0 |
| SNES | Snes9x | its own non-commercial licence |
| Mega Drive / Master System | PicoDrive (ARM-tuned) | non-commercial (to verify) |
| Amstrad CPC | PocketCaprice / Caprice32 | GPL |

**Rules for the pack:**
- **ROMs and BIOS files are the user's own;** the pack ships none.
- **Every emulator treats a ROM as untrusted input** (docs/security.md): a
  damaged or hostile file must fail safely, never run code outside the
  emulator. Native ones get the same parser rules as everything else.
- **Signed and checked** before install, like every pack (§5).
- **Licences recorded per program**; non-commercial ones are marked, since
  they bind how the pack may be shared.

**Next (owner):** which systems first; whether to include PocketCultMAME
0.4a, 0.5 WIP or both; a device check of its 2700G mode under Windows
Mobile, then under the CE personality when that exists.

**X50Mix 0.10 (owner, 2026-10-18: add it to the pack).** A tuner for the
X50 series' hardware equalizer (bass and treble), with separate headphone
and speaker profiles; by Jonathan Misurda after a discovery by "Emulamer"
on Aximsite, who released his first version's source
(<https://people.cs.pitt.edu/~jmisurda/ppc/mobile.htm>; the owner's copy:
`x50mix.ARM.10.CAB` on the SD card, 2005-01-23). Read only (facts, no code
taken): it opens Dell's sound driver as `WAV1:` and sends it two private
control codes with a 9-byte settings block, **`0x1D2000`** (set) and
**`0x1D2004`** (get); Dell's `wavedev.dll` handles both in one routine
(`0x02463E54`, a set/get flag). **For RetroPane:** as a legacy program it
runs only if the CE personality's `WAV1:` answers those two codes with
the same 9-byte block, mapped onto the WM8750's bass and treble registers
(the block's layout to read from Dell's routine when the CE audio layer is
built; docs/hw/audio.md). Licence: none stated for 0.10 (the page offers
the CAB; the first version's source was published by its author): to
confirm before the pack ships it. The same page also lists **PocketNester
0.8beta** (an NES emulator with X50v screen rendering): a candidate for
the pack too.

## 6a. Picture quality in every game (owner, 2026-10-17)

**Why.** DoomGLES under Windows Mobile is hard to see: dark, with walls,
floors and monsters blending together. The cause is mostly the game's
textures, not the screen: captured on the owner's X51v they are
**ARGB4444**, 16 shades per colour channel ([ADR-0024](../adr/0024-mbx-3d-core.md)),
so the dark end of Doom's palette rounds to a few shades, and the game's
lighting then darkens those further. Doom was also made for CRTs, which
showed dark tones brighter than this LCD does. The owner sees the same in
all the GLES game ports. The panel itself shows 16 bits ("64K colours"),
and a 16-bit picture dithered once looks identical to 32 bits on it (the
side-by-side of 2026-10-17, [ADR-0023](../adr/0023-mbx2d-renderer.md)).

**Requirements, for every game port we make** (DOOM, DOOM II, Heretic,
Quake 1-3 above, and any later one), and as far as possible for the
legacy GLES games run through our own driver:

1. **Textures at 16 bits dithered or better, never 4 bits a channel.**
   Colour textures are RGB565, dithered once when they are made (as the
   16-bit wallpaper), or ARGB8888 where alpha is needed with colour, or
   PVRTC where it looks the same; ARGB4444 is never used for colour.
   All three are proven on the 3D core (ADR-0024: RGB565, ARGB8888,
   PVRTC 4 and 2 bpp). The same for render targets and lightmaps: the
   game's lighting is applied at that depth, so shadows keep their detail.
2. **A brightness curve per game.** The 2700G's display has a colour
   lookup table every pixel passes through on its way to the panel (found
   for the night light, [hw/backlight.md](../hw/backlight.md): 17 points per colour,
   8 bits each, confirmed on the device). A game loads its own
   curve when it takes the screen and the system puts the normal one back
   when it leaves: shadows lifted for CRT-era games without crushing the
   highlights, at no cost per frame. Each game has a sensible default and
   a slider in its game settings (the "Gamma" of the old PC games, done by
   the hardware).
3. **Checked by eye on the device** before a port ships: dark scenes side
   by side, the 4-bit look against ours, as the 16-bit wallpaper was.

**Closed legacy GLES games** (the commercial titles, ADR D7): the
brightness curve works for them too. Their texture depth depends on who
made it 4-bit: if the game hands our libGLES 8-bit texels and it was Dell's
driver that stored them at 4 bits, our driver keeps them at 16 or 32 bits
and the game looks right unchanged; if the game itself sends 4-bit data,
the lost shades can't be recovered (only the curve helps). Which applies
to each title is read from its texture uploads (open; DoomGLES first).

**Decided (owner, 2026-10-18): texture work only for our own ports.** The
closed titles read so far send their textures already reduced, in their
own data (Enigmo and Virtual Pool PVRTC 4 bpp, THPS2 PVRTC 2 bpp, Call of
Duty 2 RGB565 + ARGB4444: [research/gles-games.md](../research/gles-games.md)).
Options considered for them (replacement texture packs, decoding PVRTC and
re-uploading smoothed RGB565 with mipmaps, debanding 4444) are **not
pursued**: our GLES layer gives each closed game exactly what it asks for.
The quality work above is for the GLES ports we rebuild natively (DOOM,
DOOM II, Heretic, Quake 1-3), which load the games' original art.

**Open, to try later:** the panel's own contrast. Dell's display driver
sets PWM1 to a panel-specific constant (70 of 216 on the owner's Sharp),
most likely the panel's bias or contrast voltage, not the backlight
([hw/backlight.md](../hw/backlight.md)). If it is, a little more or less may
improve the picture beyond what the gamma table can. A device test changes
it in small steps with the owner watching, and puts Dell's value back
(docs: no blind board bits).

## 7. Connected services (owner, 2026-10-17)

The owner wants the apps to reach modern services; the first example is the
Calendar app and Google Calendar. This is a plan for a vote, not built.

**Ways to reach a calendar, simplest first:**

| Way | What it is | Pros | Cons |
|---|---|---|---|
| A. Secret iCal address | Paste the calendar's private `.ics` link; the device downloads it on a timer | No login, no account on the device, a small parser, works for Google, Outlook, iCloud and Nextcloud | Read-only; the link is a secret (anyone with it can read the calendar); a long link is hard to type, so it is copied from the PC over USB or a file on the card |
| B. CalDAV | The standard calendar protocol | One piece of code for many services (Nextcloud, iCloud with an app password, Fastmail, Google with OAuth); read and write | Heavier than A (XML, sync tokens); Google needs OAuth anyway |
| C. Google sign-in (OAuth device flow) | The device shows a short code, you approve it on a phone or PC, the device keeps a revocable token | First-class Google, read and write, no password on the device, you can revoke it from your Google account | Needs a Google Cloud project; Google's check of the app for calendar access (until then "unverified", limited test users, and tokens that expire in 7 days); TLS 1.3 on a 624 MHz CPU (fine once per sync) |
| D. Through the PC | LegacySync (ADR-0028) or a home server does the cloud side; the device talks only to it on the home network | Lightest for the device, the PC holds the Google secrets | Needs the PC or server on; not "connect from anywhere" |

**Decided (owner, 2026-10-17): the order A, then B, then C**, with D left as
an option. Sync runs on a timer and on opening the app, never as a live
connection, to save battery and RAM.

**Improvements over the first sketch**, so every service shares one piece of
machinery instead of each app growing its own:

- **One "Connections" page in Settings.** Every account and library (calendar,
  media server, weather provider) is added, tested and removed in one place,
  with a "Test connection" button that says plainly what failed (no network,
  wrong address, certificate, login).
- **One token store and one sync scheduler** for all apps. Syncs are batched so
  the Wi-Fi radio wakes once, not once per app, and they are skipped on low
  battery.
- **Everything cached for offline use.** The Calendar always opens from its local
  copy; a sync that fails changes nothing. Changes made offline are queued and
  sent later, with a visible "not yet synced" mark.
- **Addresses without typing.** A long link or server address is copied from the
  PC over USB, from a small text file on the card, or typed once with a code
  shown on the device; nobody types a secret link with a stylus.
- **Home servers with their own certificates.** A self-signed certificate can be
  trusted by its fingerprint, shown on screen and confirmed once, instead of
  "ignore all certificate errors".

## 7b. Streaming from your own library (owner, 2026-10-17)

Not too much: audio is easy, video is the part to measure. The owner named Plex
and Audiobookshelf; Jellyfin and Navidrome (music) are added because they are
open source and speak simple, documented web APIs.

| Service | Kind | How the device talks to it | Difficulty |
|---|---|---|---|
| Audiobookshelf | Audiobooks and podcasts | Its web API with a token; progress is synced both ways, so you resume where the phone stopped | Easy |
| Navidrome and other Subsonic servers | Music | The Subsonic API; the server can send MP3 or Opus at a chosen bit rate | Easy |
| Jellyfin | Music, video | Its web API; the server converts video to the format and size the device asks for | Medium |
| Plex | Music, video | Sign-in through a code shown on the device; the server converts to a stream the device can play | Medium |

**What limits it, and how it is handled:**

- **The Wi-Fi is 802.11b** (11 Mbit/s on paper, roughly 4 to 5 in practice). Audio
  needs well under 1; a 480 x 320 video at 600 to 900 kbit/s fits. So the device
  always asks the server for a small, low-rate stream (the server does the heavy
  work), and never plays a 1080p file as it is.
- **The CPU is 624 MHz.** Music and audiobook codecs (MP3, AAC, Opus, FLAC) are
  light. For video the 2700G has hardware WMV9 decode (docs/hw/2700g.md); whether
  that or a software H.264 baseline decoder is better is settled by measurement
  on the device, not by guessing.
- **Download for offline.** Audiobooks and albums can be copied to the card ahead
  of time (over Wi-Fi at home, or from the PC), then play with Wi-Fi off, which
  also saves battery.
- **Reaching the server away from home** is §7c (Tailscale). At first the library
  is reached on the home network.
- The same rules as above apply: tokens in the encrypted store, TLS, the smallest
  permission, one switch removes a library and its cache.

**A pack, not the base (owner, 2026-10-17).** The Plex, Audiobookshelf, Jellyfin
and Subsonic clients ship as a **Media Servers pack** on the card (§5), signed
and loaded only when used, so the flash image stays small and nobody pays for
what they don't use. The base keeps only what every service shares: the
Connections page, the token store, the sync scheduler, and the TLS and web
client the browser needs anyway. The same split fits the other services: small
ones that most people want (calendar by iCal link, weather) stay in the base;
the heavy or niche ones (media servers, SMB, Immich) are packs.

**Order:** Audiobookshelf and Subsonic (audio) first, then Jellyfin and Plex
audio, and video last, after the decoder measurement.

**Rules that apply to every service** ([security.md](../security.md)): tokens
live in the encrypted store, never in plain files on the card; the smallest
permission that does the job (events only, never the whole account); TLS only,
certificates checked, no password stored for Google; one switch in Settings
removes an account and wipes its token and cached data; a failed sync never
loses local events. Contacts stay out (RetroPane is not a phone).

Other apps follow the same pattern later (Tasks, Reader feeds, weather
providers); each gets its own line here when picked up.

## 7c. Away from home: Tailscale (owner, 2026-10-17)

The owner uses Tailscale all the time and wants it for reaching the home
libraries and servers (§7b) when out. This is a plan for a vote, not built;
it needs the Wi-Fi, TCP/IP and TLS work of Phase 6 first.

Tailscale is built on WireGuard. Its client is open source (BSD licence,
written in Go, too heavy to run as it is on 64 MB of RAM, but free to read and
learn from, with the licence notice kept wherever code is copied); the
service that hands out keys and the map of your devices (the coordination
server) is Tailscale's own and is not open. Headscale is an open-source
stand-in for that server. Three ways for the X51v to use it, smallest first:

| Way | What it is | Pros | Cons |
|---|---|---|---|
| A. A gateway at home | A machine on the tailnet makes the library reachable over ordinary HTTPS (Tailscale's Funnel gives a public address with a real certificate); the device needs only TLS | No VPN code on the device at all; works the day networking lands | Funnel puts the service on the public internet, guarded only by the server's own login; so it is opt-in, and the library must be set up with strong passwords |
| B. WireGuard to a gateway | The device is a plain WireGuard peer of a home machine that is also on the tailnet and passes traffic through | Small code (the WireGuard pieces are well specified), no Tailscale account logic on the device | The device is not a tailnet member: no per-device rules, names or revoking from the Tailscale console; keys are set by hand |
| C. A real tailnet node | RetroPane's own client in Rust: registers with the control service, keeps the device map, talks through Tailscale's relay servers over HTTPS (no router tricks needed), and carries WireGuard inside | The proper answer: the device appears in your tailnet, gets a name, can be limited by access rules and revoked from the console | The most work: the control protocol, the relay protocol, WireGuard and a TCP/IP stack; Tailscale's terms for third-party clients to be checked first, with Headscale as the fallback |

**Decided (owner, 2026-10-17): A until C exists.** A gateway at home
(way A) is how the device reaches the library away from home until
RetroPane's own tailnet client (way C) is built; B is not planned.

**Recommendation: C, built as an operating-system service** (the owner's
word: "a service"), so every app benefits, not only the media ones: media
servers, NAS shares, calendar sync, remote desktop. Stages: **T1** use A
until the service exists; **T2** the service, relay-only (always works, speed
is limited by the 802.11b link anyway), signing in with a **tagged,
expiring auth key** the owner creates; **T3** direct paths between devices
later if the relay is too slow. It ships as a pack (§5, §7b), loaded when a
connection is wanted, not always on, to save battery and RAM. Sizes (the
crypto is a few tens of KB, the netmap is read as a stream) are to be
measured, not guessed.

**Terms check (2026-10-17, reading Tailscale's customer terms; not legal advice):**
nothing in them mentions third-party clients, for or against. They forbid
reverse engineering or copying "the Services" and commercially exploiting them;
the client's source is open (so writing our own client from it is allowed by its
licence), but whether an unofficial client may use Tailscale's hosted service is
**not stated**. So before stage T2
the owner (or a note from the project) asks Tailscale, and Headscale is the
fallback that needs no one's permission. Stage T1 (a gateway) uses the official
client on a home machine and has no such question.

**A free tailnet for the project (owner, 2026-10-17, from Tailscale's own pages as
summarised by a search overview; to be confirmed on tailscale.com before applying):**
Tailscale offers open-source projects with an OSI-approved licence on GitHub a free
"Community on GitHub" plan, up to 25 users and 5 devices each. RetroPane (GPL, on
GitHub) should qualify: the owner applies. It would give the project its own tailnet
for test devices, the download host and the media-server tests. The client's
components are open source under BSD, MIT or Apache 2.0 depending on the part, free to
fork or build on; `tsnet`, the library that embeds Tailscale in a program, is Go, so
it can help the PC or home-server side but cannot run on the Axim.

**Security rules specific to this** ([security.md](../security.md)):
the device's key lives in the encrypted store; the auth key is tagged so the
access rules let the device reach **only** the media server and nothing else
on the tailnet; the node is "ephemeral", so a lost device drops out by itself,
and can be removed from the Tailscale console at any time; the device is never
an exit node or a subnet router; every packet the service parses is treated as
hostile input and fuzzed.

## 8. More ideas to modernize

To be gathered as they come (owner and Claude): each new idea gets a line
here, then a design doc when it's picked up.
