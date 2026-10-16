# RetroPane design: everything in one place

The product's design, as decided by the owner: what RetroPane looks like,
which apps and games it has, how it's customized, and what lives on the
card. Start here; every design idea is on this page or one link away.
Engineering decisions (kernel, drivers, memory) live in the ADRs and
`docs/hw/`; they are linked below only where they shape what people see.

**Rule for every item:** a design doc, mock-ups and the owner's vote come
before code (ROADMAP Phase 5), and "imitation is the last resort": Windows
Mobile's feature list is kept on purpose, but its look and behaviour are
only kept where they measurably win. Own artwork only (`CLEANROOM.md`).

## The pages in this area

| Page | What's in it |
|---|---|
| [ui.md](ui.md) | The look: the owner's decisions, the the reference smoked-glass design language (**boards N-R: the chosen design**), the Today screen, the Start menu, wallpapers and their licences, type, how the glass renders on the 624 MHz XScale |
| [apps-and-packs.md](apps-and-packs.md) | Every Windows Mobile 5 default app rebuilt and modernized, a new coat of paint on everything, themes and customization, legacy apps and games, the SD expansion pack and its download location, our own DOOM / Heretic / Quake ports, more ideas to modernize |
| [weather.md](weather.md) | The Weather app and the Today screen's weather, the icon packs (one built in, five on the card, bring your own), the pack format keyed by the WMO codes, the prompt and models for making a pack with the owner's own tools |

**Mock-ups:** the concept canvas, private to the owner (Claude artifact
"RetroPane UI concepts", 2026-10-12): concepts A-K, then boards N-R, the
chosen direction. Third-party art appears only there, credited, never in
this repository.

## Design decisions recorded elsewhere

| Topic | Where |
|---|---|
| Vision and goals, the "Windows-esque, retro, modernised" look, theme engine (Modern + Classic) | [DESIGN.md](../DESIGN.md) §1 Vision, §7 GWES, graphics and the shell |
| Fast and modern from the start: 50 ms touch, no dropped frames, instant wake, pre-rendered chrome and atlases, what lives on the card | [design-brief.md](../design-brief.md) |
| Memory budgets the design must fit (64 MB RAM, non-XIP flash) | [memory-budget.md](../memory-budget.md) |
| Night light: the backlight down to Windows Mobile's dimmest, then the picture through the gamma table; **the control: one slider reaching lower plus a schedule** (decided 2026-10-16); device results | [hw/backlight.md](../hw/backlight.md) "Finer and lower steps", "The night setting's control" |
| Optional packs: a complete base in flash, extras on the card | [ADR-0018](../adr/0018-optional-packs.md) |
| Field Kit: the network and utility tools pack | [ADR-0020](../adr/0020-field-kit-pack.md) |
| The shell library and the first Today screen | [ADR-0021](../adr/0021-shell-library-and-first-today-screen.md) |
| The graphics stack and service: compositor, GDI, the 2D and 3D renderers | [ADR-0022](../adr/0022-graphics-stack.md), [ADR-0030](../adr/0030-graphics-service.md) |
| GLES for games: our own 3D renderer, the binary GLES layer for closed games, native ports for open engines | [ADR-0026](../adr/0026-own-mbx-3d-renderer.md), [research/gles-games.md](../research/gles-games.md) |
| Legacy apps: unmodified WM5/WM6 programs, CAB installs | [DESIGN.md](../DESIGN.md) §6 CE Personality, [ROADMAP](../ROADMAP.md) Phase 4 |
| LegacySync: the Windows 11 companion doing ActiveSync's jobs | [ADR-0028](../adr/0028-legacysync.md), [legacysync/](../legacysync/) |
| A faster Quake 3 port | [ROADMAP](../ROADMAP.md) "At the end: a faster Quake 3 port" |
| Security every design respects (internet-connected device, signed packs, legacy never locked out) | [security.md](../security.md) |
| Which Windows Mobile and Dell values are still copied, not measured | [research/imitation-audit.md](../research/imitation-audit.md) |

## Owner decisions, by date

A running list; each line points to where the detail is.

- **2026-10-12:** the UI direction and its constraints; weather as the
  Today screen's hero; the smoked-glass design language; **boards N-R chosen**
  ("I LOVE it"); wallpapers that ship ([ui.md](ui.md)). Optional packs
  ([ADR-0018](../adr/0018-optional-packs.md)).
- **2026-10-13:** the Field Kit pack ([ADR-0020](../adr/0020-field-kit-pack.md)).
- **2026-10-14:** a faster Quake 3 port ([ROADMAP](../ROADMAP.md));
  weather icons must be realistic and modern, the open sets rejected
  ([weather.md](weather.md)).
- **2026-10-15:** LegacySync ([ADR-0028](../adr/0028-legacysync.md)).
- **2026-10-16:** the night control: one slider plus a schedule
  ([hw/backlight.md](../hw/backlight.md)); every WM5 default app rebuilt
  and modernized, themes, GLES for all games, the SD expansion pack, our own
  DOOM / Heretic / Quake ports ([apps-and-packs.md](apps-and-packs.md));
  the Weather app as a headline feature, icon packs (one built in, five on
  the card, bring your own), packs made with the owner's own tools, never
  Claude-drawn icons ([weather.md](weather.md)); the owner's NAS hosts the
  downloads behind security layers ([apps-and-packs.md](apps-and-packs.md) §5).
