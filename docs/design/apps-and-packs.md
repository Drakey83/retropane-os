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
| Heretic | Raven's 1999 source release (GPL-2.0) | HereticGLES is the legacy reference |
| Quake | id's Quake 1 source (GPL-2.0) | New addition |
| Quake 3 | id's 2005 release or ioquake3 (GPL-2.0) | Already on the ROADMAP: start-up time a goal from day one |

Game data (WADs, PAKs) stays the user's own, except freely distributable
shareware data. Each port is likely its own repository (as the Quake 3
note says), measured against the stock-WM baseline (DESIGN.md §10).

## 7. More ideas to modernize

To be gathered as they come (owner and Claude): each new idea gets a line
here, then a design doc when it's picked up.
