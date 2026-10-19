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
| [ui.md](ui.md) | The look: the owner's decisions, **SPB Mobile Shell's whole experience as the reference with our own take (2026-10-19): two looks, Modern and Classic; the weather screen the default home; everything customisable, themes included; the carousel and a part-by-part survey of SPB's demo video**; **3D acceleration for the whole UI (a proposal, options A/B/C) and ideas for modern 3D effects (a 3D flip clock, a living weather sky, glass that catches the light, a card-stack task switcher...), not voted yet**; the smoked-glass design language (**boards N-R: the chosen design**), the Today screen, the Start menu, wallpapers and their licences, type, how the glass renders on the 624 MHz XScale |
| [apps-and-packs.md](apps-and-packs.md) | Every Windows Mobile 5 default app rebuilt and modernized, a new coat of paint on everything, themes and customization, legacy apps and games, the SD expansion pack and its download location, our own DOOM / Heretic / Quake ports, more ideas to modernize |
| [display-calibration.md](display-calibration.md) | Getting the most out of the X51v's screen: what we control (gamma table, backlight, refresh, PWM1, dither, content) and what we can't, and the plan (C1-C5) to calibrate it by eye on the device |
| [today-screen-plan.md](today-screen-plan.md) | The plan for the Today screen on the device: what runs now, the four steps, the icon format |
| [weather.md](weather.md) | The Weather app and the Today screen's weather, the icon packs (VClouds' built in, five optional ones on the card, bring your own), the pack format keyed by the WMO codes |
| [peripherals.md](peripherals.md) | Keyboards, mice and headsets over USB and Bluetooth: the owner's requirement and the hardware questions; the design is [ADR-0033](../adr/0033-keyboards-mice-headsets.md) |
| [mockups/](mockups/README.md) | Images of the Today screen with the default weather icons (VClouds'), an icon sheet, the five optional packs, and their credits and licences |

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
| GLES for games: our own 3D renderer, the binary GLES layer for closed games, native ports for open engines | [ADR-0026](../adr/0026-own-mbx-3d-renderer.md), [research/gles-games.md](../research/gles-games.md), the whole 2700G 3D library: [research/3d-games-catalogue.md](../research/3d-games-catalogue.md) |
| Sound: the audio service (mixing, volumes per output and kind, headphones unplugged, tone controls, the microphone's permission); **decided 2026-10-18** | [ADR-0032](../adr/0032-audio-service.md) |
| Keyboards, mice and headsets over USB and Bluetooth; **decided 2026-10-18** (P1-P5 all A) | [ADR-0033](../adr/0033-keyboards-mice-headsets.md), [peripherals.md](peripherals.md) |
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
  DOOM / Heretic / Quake / Quake 2 ports ([apps-and-packs.md](apps-and-packs.md));
  the Weather app as a headline feature, icon packs (one built in, five on
  the card, bring your own), never Claude-drawn icons ([weather.md](weather.md)); the owner's NAS hosts the
  downloads behind security layers ([apps-and-packs.md](apps-and-packs.md) §5).
- **2026-10-17:** VClouds gave permission to use his weather icons and skin
  design with credit; his icons are the default pack, the owner's first pack is
  dropped and the other five are optional ([weather.md](weather.md),
  [ui.md](ui.md)); the optional packs and all other generated icons stay off the
  front page; fifteen wallpapers approved ([ui.md](ui.md)); Quake 2 added to the
  game ports; connected services (iCal, then CalDAV, then Google sign-in) with
  one Connections page, and streaming from the owner's own libraries as a pack
  ([apps-and-packs.md](apps-and-packs.md) §7, §7b); Tailscale for away from home:
  a gateway at home (A) until our own tailnet client (C) exists (§7c); every
  game port draws textures at 16 bits dithered or better (never 4 bits a
  channel) and has its own brightness curve (§6a); the
  Today screen plan ([today-screen-plan.md](today-screen-plan.md)).
- **2026-10-18:** the audio service ([ADR-0032](../adr/0032-audio-service.md)):
  44.1 kHz fixed; music pauses when headphones come out (old programs
  muted); volumes per output and per kind, with a gentle hearing warning;
  the codec's bass and treble in Settings plus speaker tuning, no software
  equaliser; the microphone asks once per program, with a mark while
  recording; thirteen engineering conditions before S1.
- **2026-10-18:** keyboards, mice and headsets ([ADR-0033](../adr/0033-keyboards-mice-headsets.md)):
  both Bluetooth radios (built-in for classic, a dongle for Low Energy and
  secure pairing), our own Rust Bluetooth stack, PIN pairing only for
  keyboards, mice and headsets, an iPad-style pointer, modern keyboard
  shortcuts; radio routing and a profile allowlist per radio before P-S1.
- **2026-10-19:** audio S1 passed; when the sound chip sleeps
  ([ADR-0032](../adr/0032-audio-service.md) R5): after 5 s of quiet, the link
  kept running, woken early by a touch or a key so clicks stay instant.
- **2026-10-19:** the UI follows SPB Mobile Shell's whole experience with
  our own take: two looks, Modern (the default, SPB-shaped; the finger for
  navigation only) and Classic (WM5, stylus), switchable; the weather
  screen the default home; everything customisable, themes included; the
  carousel ([ui.md](ui.md)). Proposals waiting for a vote: 3D acceleration
  for the whole UI (A planes only = ADR-0022 as accepted, B a 3D compositor,
  C showpieces only) and ideas for modern 3D effects ([ui.md](ui.md)).
