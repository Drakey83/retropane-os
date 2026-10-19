# 3D-hardware-accelerated software for the Dell Axim X50v / X51v

Catalogue of games, emulators, benchmarks, demos and game-adjacent
software with positive evidence of using the Intel 2700G's 3D hardware
(PowerVR MBX Lite, OpenGL ES 1.0 Common-Lite). Both the X50v and X51v
carry the 2700G7, so the list is identical for both. The non-v X50/X51
lack the 2700G and are out of scope.

*Owner's research (2026-10-18), added to the repository the same day and
reconciled with RetroPane's own import-table reads ("Reconciled with our
own reads" below). Facts only: no game files are in the repository, and
none are acquired for it (the owner's games are read where they already
are, read-only). What it is for: the scope of the binary GLES layer for
closed games (ADR-0026 D7) and of the SD-card game pack
(design/apps-and-packs.md). The owner's rule stands: GLES games are
evidence about the GLES engine and the GPU; RetroPane fixes textures only
in its own GLES ports.*

Companion file: [`gles-games.md`](gles-games.md) holds the import-table ground truth for
the games the owner has on card (which GLES entry points each binary
imports, by ordinal). This file is the wider commercial and community
library beyond that core; the core titles are repeated here so the
catalogue is complete in one place, with their evidence cited back to
`gles-games.md`.

## Confidence levels

- **CONFIRMED** — PE import table showing Intel's GLES DLL, or a vendor
  statement (publisher, Imagination, Khronos, Dell) of 2700G/OpenGL ES
  support or an on-device hardware-accelerated demo.
- **LIKELY** — a contemporary 2700G-optimized badge or strong
  contemporary evidence (Dell's recommended-games list, bundled on the
  Dell game CD, a review naming 2700G optimization, shipping a GLES DLL).
- **POSSIBLE** — claimed somewhere but unverified, or the title could
  not be positively identified.

Accuracy beats length: a title is listed only with positive evidence of
*hardware* 3D. "3D graphics" that were software-rendered (GAPI/GDI) are
in the excluded list at the bottom, not here.

## Games

| Title | Publisher | Year | 3D API / evidence | Confidence | Notes |
|---|---|---|---|---|---|
| Enigmo | Aspyr Media (Pangea Software) | 2004 | `libGLES_CL.dll`, 54 imports by ordinal (`gles-games.md`); bundled on the Dell X50 Game CD | CONFIRMED | The 2700G showcase title; also uses `gx.dll` for input only and FMOD CE for sound |
| Tony Hawk's Pro Skater 2 (Pocket PC Edition) | Aspyr Media (published by Activision) | 2006-04 (the owner's `THPS2.exe` is built 2005-10-27) | **Import table: `libGLES_CL.dll`, 45 by ordinal (`gles-games.md`).** Also: "Optimized for the Intel 2700G processor found in Dell Axim devices" (Wikipedia, citing Aspyr); demonstrated running OpenGL ES on X50v/X51v at Imagination's GDC 2006 booth (Khronos, Mar 2006); named in the Dell X51 user manual ("play intense 3D games like Tony Hawk Pro Skater 2"); on the ixbtlabs "recommended for Dell Axim x50v/x51v" list | CONFIRMED | Full counterpart of the PC game, not a demake |
| Quake III Arena (Q3CE) | Noctemware (id Software) | 2005 | `libGLES_CL.dll`, 57 imports (`gles-games.md`); "requires the Intel 2700g multimedia accelerator" (EyeOnMobility, 2005); Imagination demoed it "running with full hardware acceleration using OpenGL ES on PowerVR-enabled Dell Axim X51v" at G-Star 2005, "up to 25 fps on the Dell Axim X50v" (Khronos, Nov 2005); ixbtlabs: "decent performance only with Intel 2700G" | CONFIRMED | Needs the PC game's data files (~120-150 MB); ships its own `libGLES_CM_NoE.dll` + `libEGL.dll` which looks like a software fallback — on the 2700G it uses Intel's DLL |
| Quake Mobile (QuakeMobile) | Pulse Interactive | 2005-2006 | Installer on owner's card, not installed (`gles-games.md`: imports still to read); Imagination: "demonstrating Quake Mobile ... running with full hardware acceleration using OpenGL ES on PowerVR-enabled Dell Axim X51v" (Khronos, Nov 2005); "QuakeMobile and Expansion Packs" on the GDC 2006 demo list | CONFIRMED | Name variants: "Quake Mobile 1.2" (desitwist review). Expansion packs existed |
| Geo Rally EX | (unknown) | ~2005 | `libGLES_CL.dll`, 48 imports (`gles-games.md`); ixbtlabs: "GeoRally EX — another racing simulator", in the "require a graphics processor" list | CONFIRMED | Name variant: "Geo Rally". The more realistic racer next to Stuntcar Extreme's stunt racing |
| Solterra | (unknown) | ~2005-2006 | `libGLES_CL.dll`, 50 imports (`gles-games.md`); desitwist X51v review lists the "space shooter Solterra" among 2700G-optimized games | CONFIRMED | Space shooter; found on owner's card 2026-10-15, previously unknown to us |
| DoomGLES / Doom2GLES / HereticGLES | Kokak | ~2005-2006 | `libGLES_CL.dll`, 59 imports each (`gles-games.md`); XDA review thread documents the OpenGL-ES (2700G) version vs the later software DoomPPC port | CONFIRMED | Open-source engines — native ports planned instead of binary GLES (per ADR-0026 D7) |
| Pocket Cube | (Cube engine) | ~2005 | `libGLES_CL.dll`: game and server 35 imports each, its SDL port 14 EGL imports (`gles-games.md`) | CONFIRMED | Open-source engine — native port planned; desitwist notes "this is a 3D engine, not a full featured game" |
| Stuntcar Extreme | Fathammer | 2004-2005 | X-Forge 3D engine ("truly console quality 3D graphics", XDA VGA 3D-games thread); bundled on the Dell X50 Game CD (`SCERC0B.exe`); ixbtlabs: "included into the list of recommended games for Dell Axim"; desitwist: "optimized to Intel 2700G" | LIKELY | Wi-Fi multiplayer; 20 cars, 7 stunt tracks. Import table not yet read (InstallShield installer, not unpacked) |
| Call of Duty 2 (Pocket PC Edition) | IonFx (published by Activision) | 2007-01 | **Import table: `libGLES_CL.dll`, 40 by ordinal (`cod2.exe` built 2006-12-06, calls itself `cod2_gles`; `gles-games.md`).** Also: ships `libgles_cm.dll` in the program directory (XDA/Modaco 3D-game test by thierryb: "if you remove libgles_cm.dll from the COD2 (demo) program directory, it'll work"); Aspyr's 2006 plan was to "port Call of Duty 2 to Pocket PCs using the Intel 2700G 3D accelerator" (EyeOnMobility) | **CONFIRMED** (import read 2026-10-18; was LIKELY) | Full 3D FPS with new WM-only levels (GameFAQs WM reviews). Note: bundled `libgles_cm.dll` may be a software fallback like Q3CE's — 2700G path unconfirmed at import level. "Too dark by default, no gamma correction" per thierryb |
| Virtual Pool Mobile GLES | Celeris | 2005 | **Import table: `libGLES_CL.dll`, 48 by ordinal (`VPMobileGLES.exe`, built 2005-09-06, on the owner's SD card; `gles-games.md`).** Also: CdrInfo's X51v review: "I also tested a new version of Virtual Pool Mobile that took advantage of the 3D acceleration and the graphics and gameplay were simply the best I have ever seen on any handheld"; won Pocket PC Magazine awards | **CONFIRMED** (import read 2026-10-18; was LIKELY) | The GAPI builds (v1.78, and Dell's CD trial of 2005-05-22) are software-rendered: build matters. The GLES build's version number is not recorded in the binary we read, so "v1.29" is unconfirmed |
| 3D Chopper Fight | OmniGSoft (3D Game Jumbo Pack 2) | ~2005 | desitwist X51v review lists "3D Chopper Fight" among games optimized to 2700G; reviewed on x50v.it | LIKELY | RC toy choppers, indoor arena. Name variant: "Chopper Fight". OmniGSoft's engine is otherwise software 3D — the 2700G claim rests on the one contemporary review |
| Jump | The Code Monkeys | 2006 | Khronos GDC 2006: "The Code Monkeys' extreme sport game 'Jump' on Dell Axim X50v / X51v" in the OpenGL-ES demo lineup | LIKELY | Demo confirmed on hardware; commercial Pocket PC release unconfirmed (Giant Bomb lists "Jump - Mobile", platform ambiguous) |
| Marble Worlds 2 | Limelight Software | ~2006-2008 | Clickgamer retail listing (via the XDA "VGA 480x640 HD 3D Games" thread) gives compatibility as "**Pocket PC 3D Accelerator (Axim x50v/x51v)**" — the publisher listing names the accelerator and both devices explicitly | LIKELY | Marble-rolling 3D puzzler (100 play areas). The compatibility line is a strong contemporary signal but not an import-table read |
| Toy Golf | (unknown) | ~2005 | Bundled as a **full version** on the Dell X51v companion CD, described as one of "2 games that utilize the full capacities of the Intel 2700G graphics accelerator" (X51v review, darrellpugsley.com); also in the XDA/Modaco GLES test corpus ("starts, but the game itself is rendered in Portrait ... unplayable" — a GLES title with an orientation bug) | CONFIRMED | 3D mini-golf. Publisher unidentified. Note the X50v CD carried Enigmo + Stuntcar Extreme while the X51v CD carried Toy Golf + Geo Rally |
| JellyFish | (unknown) | ? | Modaco corpus: "as with CoD2, removing libgles_cm.dll from the home might help (untested!)" — implies it ships a GLES DLL | POSSIBLE | Title unidentified; may be a demo rather than a commercial game |
| Sphere | (unknown) | ? | Modaco 3D-game test corpus only | POSSIBLE | Title unidentified |
| Pocket-Jongg | (unknown) | ? | Modaco 3D-game test corpus only | POSSIBLE | Title unidentified; presumably a 3D mahjong |
| Jackpot Casino | (unknown) | ? | Modaco 3D-game test corpus only | POSSIBLE | Title unidentified |
| Quake II | (Dan East port lineage) | 2001+ | Modaco corpus lists "Quake2" (did not run on GoForce); the known 2001 Dan East Pocket PC port predates the 2700G and was software-rendered | POSSIBLE | A GLES/2700G build is unconfirmed — do not assume the 2001 port used hardware 3D |
| Anthelion 2: Celestial Vigilance | PDAmill Game Studios | 2005 | Listed in the XDA "VGA 480x640 HD 3D Games" thread ("a blazing-fast, highest quality 3D graphics engine for the Pocket PC", VGA-enhanced) | POSSIBLE | Space shooter. Caution: PDAmill's other titles (Snails) were software-rendered 2D — no 2700G-specific evidence for this one, the thread listing is the only signal |

## Emulators (with a 2700G / hardware-3D renderer option)

| Emulator | Author | Year | 3D API / evidence | Confidence | Notes |
|---|---|---|---|---|---|
| PocketSNES (2700G-optimized build) | (unknown) | ~2005 | desitwist X51v review: "optimized PocketSNES emulator (the way to plenty of SNES games)" among 2700G-optimized software; Modaco: "the 2700G-specific PocketSNES" | LIKELY | SNES is a 2D system — the 2700G path here is almost certainly the 2D blitter/scaler, not the 3D core. Still the only emulator with a named 2700G build |
| FPSEce (GPU plugin) | schtruck | ~2006-2008 | Task background: the WinMo FPSEce's external GPU plugin had 2700G acceleration | LIKELY | Contemporary confirmation not found in this pass — verify from the plugin README/binary before relying on it. (The XDA "FPse OpenGL Plugin" thread is Android-era, not WinMo.) PSX is a 3D system, so a GPU plugin is the real 3D-core case |
| PocketGBA (2700G build) | (unknown) | ~2005 | Modaco: "the 2700G-specific PocketGBA"; PocketGBA itself is a real PPC GBA emulator (used in the 2011 HTC Hermes "GameBoy Advance Phone" project) | POSSIBLE | GBA is 2D; the 2700G build likely used 2D acceleration. Existence of the 2700G-specific build rests on the one Modaco mention |
| PocketNester x50v build (v0.8beta) | Jonathan Misurda | 2005-05 | "The best NES emulator for PocketPC but with support for improved rendering on the x50v, even without the GAPI tweak. It runs at a full 60fps with sound." (people.cs.pitt.edu/~jmisurda/ppc) | LIKELY (2D) | Almost certainly the 2700G **2D** path (framebuffer blits), not the 3D core — listed so it isn't mistaken for 3D evidence. NES is a 2D system |

Checked, no hardware-3D evidence: **MorphGear** (multi-system PPC emulator; no GLES/2700G video module found — software rendering via GAPI/DirectDraw). No N64 emulator for Pocket PC with a hardware renderer was found.

## 3D benchmarks and tech demos

| Demo / benchmark | Author | Year | Evidence | Confidence | Notes |
|---|---|---|---|---|---|
| Virtual Pompeii | SpinVector | 2006 | Khronos GDC 2006: "Virtual Pompeii from SpinVector on Dell Axim X50v / X51v" in the OpenGL-ES demo lineup | LIKELY | A demo, not a commercial game. SpinVector is an Italian studio |
| PowerVR Insider SDK OpenGL ES demos | Imagination Technologies | 2004-2006 | The Register's X50v review: "if you're interested in seeing what your Axim can do, you can download some demos from PowerVR's web site" | LIKELY | The MBX-era SDK sample binaries ran on the 2700G's GLES 1.0 implementation |
| 3DMarkMobile / 3DMarkMobile06 | Futuremark | 2006 | 3DMarkMobile06 was demoed on the **Freescale iMX31** dev system at GDC 2006 (Khronos) — a different chip. No Pocket PC / Windows Mobile build found | — (not 2700G) | Do not list as a 2700G benchmark; the mobile 3DMark line targeted phones, not PPC |

No 2700G tech demo has been identified as *bundled on the Dell companion CDs* — the Dell X50 Game CD carried full games (Enigmo, Stuntcar Extreme installers), not tech demos. The Modaco GoForce test notes "None of the demos work, except for the Benchmark app" (unidentified) — not 2700G evidence either way.

## 3D screensavers

None found. No Pocket PC screensaver with positive 2700G/OpenGL ES evidence turned up in this pass — the 3D-screensaver market was desktop-only (SereneScreen, Dream Aquarium, 3Planesoft). If one surfaces, it belongs here.

## Game-adjacent: 2700G video (not 3D — listed to avoid confusion)

- **TCPMP (The Core Pocket Media Player)** — 2700G video-decode acceleration "almost right after the release of Dell X50v" (desitwist); ixbtlabs benchmarked the 2700G decoder path vs raw framebuffer/GDI. This is the 2700G's **video engine** (MPEG-1/2/4, WMV), not the 3D core.
- **PocketTV Classic** — the other 2700G-accelerated video player (ixbtlabs).

## Checked and excluded

These came up as candidates but fail the positive-evidence-of-hardware-3D rule:

- **Warfare Incorporated** (Handmark/Handheld Games) — 2D RTS; runs on a 33 MHz Palm. Not 3D at all.
- **Snails** (PDAMill) — 2D Worms-like artillery. Not 3D.
- **GTS World Racing / GTS Racing Challenge** (Astraware) — pseudo-3D: "pretty much uses a flat course engine ... just another 2D game made look 3D" (Aaron Ardiri, 2003). No hardware 3D.
- **Enigmo 2** — Mac/PC/iOS only (from Feb 2006); never shipped on Pocket PC.
- **Gameloft Windows Mobile 2008-2009 titles** (Asphalt 4: Elite Racing, Prince of Persia, Gangstar 2, Real Football 2010, Spider-Man: Toxic City, Ferrari GT Evolution — via Windows Marketplace for Mobile) — targeted later 3D-capable WinMo devices (HTC Touch Pro 2 era); no 2700G evidence.
- **Gameloft N-Gage/Symbian 3D titles** (e.g. Splinter Cell Chaos Theory on N-Gage, Vijay Singh Pro Golf 2005 on Sony Ericsson P990) — different platform/chip, not Pocket PC.
- **Bling my Ride, Whiptail Mobile Engine** (TI OMAP2420), **3DMarkMobile06** (Freescale iMX31) — PowerVR MBX chips, but not the 2700G / not the Axim.
- **Blender Pocket** — OpenGL ES via the Rasteroid *software* wrapper; not hardware 3D on the 2700G.
- **Ogre3D GLESX RenderSystem (WinCE port)** — an engine with checked-in code "that works on accelerated Dell Axims" (Ogre forums, 2007-2009), but no shipped game on the 2700G found.
- **Tomb Raider** (Eidos, Ideaworks3D port, Pocket PC July 2002) — yes, the original 1996 game shipped on Pocket PC and runs on the X50v/X51v, but it is software-rendered: contemporary review notes "the original Tomb Raider had little reliance on hardware acceleration," so the 2700G does nothing for it. Not a hardware-3D title.

## Reconciled with our own reads (2026-10-18)

RetroPane's import-table reads ([`gles-games.md`](gles-games.md)) are the
ground truth where they exist:

| Title | Catalogue said | Our read | Now |
|---|---|---|---|
| Call of Duty 2 | LIKELY; 2700G path unconfirmed (bundled `libgles_cm.dll` might be a fallback) | `cod2.exe` imports Intel's `libGLES_CL.dll` (40 ordinals) | CONFIRMED |
| Virtual Pool Mobile GLES | LIKELY; import check pending | `VPMobileGLES.exe` imports `libGLES_CL.dll` (48); the GAPI builds import none | CONFIRMED (version unknown) |
| Tony Hawk's Pro Skater 2 | CONFIRMED by vendor statements | `THPS2.exe` imports `libGLES_CL.dll` (45); all 927 world textures PVRTC 2 bpp | CONFIRMED, now also by imports |
| Toy Golf, Quake Mobile, Stuntcar Extreme | CONFIRMED / CONFIRMED / LIKELY | not read yet (Toy Golf not found on the cards; Quake Mobile and Stuntcar only as installers) | unchanged; an import read settles each |
| Tomb Raider | excluded (software) | GAPI only (`gx.dll`), second-batch scan | agrees |

The owner's second batch (105 programs, 66 installers) found no further
GLES program, which agrees with the catalogue's conclusion below that the
2700G's commercial library was this small.

## Name variants and duplicates (do not double-count)

- Geo Rally = GeoRally EX (same game)
- Quake Mobile = QuakeMobile (Pulse Interactive); "Quake Mobile 1.2" is a version number, not a sequel
- Quake III Arena (PPC) = Q3CE (Noctemware's port name)
- Tony Hawk's Pro Skater 2 (PPC) = "Tony Hawk's Pro Skater 2 Pocket PC Edition"
- 3D Chopper Fight = "Chopper Fight"
- Virtual Pool Mobile: `VPMobileGLES.exe` = GLES build (version number unrecorded; "v1.29" unconfirmed); v1.78 and Dell's CD trial = software (GAPI) builds — same title, different renderer; build matters

## Sources

- `gles-games.md` (same directory) — PE import tables read from the owner's cards, 2026-10-15
- desitwist.com X51v review thread — contemporary owner review listing 2700G-optimized games
- ixbtlabs.com "Intel 2700G Chipset — 3D Accelerator in Your Pocket" (Sergei Solomatin, Jun 2006) — "games that require a graphics processor" + Dell recommended list
- Khronos news: "PowerVR MBX enabled OpenGL-ES 1.1 and OpenVG demos at GDC Booth 1632" (Mar 2006); "Quake III in the palm of your hand" (Nov 2005, G-Star demo on X51v/X50v)
- Modaco "O2 XDA Flame and GoForce" thread (XDA member thierryb's GLES game test corpus, incl. `libgles_cm.dll` findings for CoD2/JellyFish)
- XDA Developers "VGA 480x640 HD 3D Games" thread (Stuntcar Extreme / X-Forge engine description)
- EyeOnMobility: "Quake III on the Dell Axim" (2005); "Windows Mobile Call of Duty 2 screenshots" (2006, Aspyr 2700G port plan)
- Wikipedia: Tony Hawk's Pro Skater 2 (Aspyr WM port "optimized for the Intel 2700G"); Enigmo (Aspyr WM port bundled with the X50v); Call of Duty (CoD2 Pocket PC Edition by IonFx, 2007)
- The Register: Dell Axim X50v review (Dell CD games; PowerVR website demos)
- GameFAQs Windows Mobile reviews of Call of Duty 2 (IonFx build is a full 3D FPS)

## Exhaustive re-sweep (2026-10-18)

Second pass at the owner's "exhaustive" standard after the first catalogue:
queried the two canonical contemporary lists (ixbtlabs Jun 2006,
desitwist X51v review) against each other, Dell's recommended-games
lists, the Dell X51/X51v user manual, four X50v/X51v hardware reviews
(Gadgeteer, CdrInfo, internetnews, darrellpugsley), the XDA "VGA
480x640 HD 3D Games" thread, the full "List of Windows Mobile
Professional games" (scanned title-by-title for 3D candidates:
Atomic Battle Dragons, Interstellar Flames, K-Rally, Metalion 1/2,
Ultima Underworld, Leo's sims, Spb AirIslands — none with 2700G
evidence), French/German/Russian-language queries, Fathammer and Aspyr
catalog checks, and the Tomb Raider question (resolved: PPC port
exists, software-rendered, excluded). Direct Wayback CDX access failed
from this environment (network error, twice), so archive.org coverage
came via still-indexed contemporary threads, reviews and retail
listings instead. Findings: Toy Golf promoted LIKELY to CONFIRMED
(Dell X51v CD, "full capacities of the Intel 2700G"); Marble Worlds 2
added as LIKELY (Clickgamer "Pocket PC 3D Accelerator (Axim
x50v/x51v)" compatibility line); Anthelion 2 added as POSSIBLE (XDA
thread only; PDAmill's other titles were software-rendered);
Virtual Pool Mobile v1.29 evidence strengthened (CdrInfo: 3D-accelerated
build "the best I have ever seen on any handheld"). Every independent
list converges on the same core set — the commercial 2700G game
library really was this small.
