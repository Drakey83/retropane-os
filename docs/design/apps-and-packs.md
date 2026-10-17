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
| Quake 2 | id's 2001 source release (GPL-2.0) | Added 2026-10-16 (owner): lighter than Quake 3 (built for 1997-98 PCs, 16-32 MB); software and OpenGL 1.x renderers to start from; game data (`pak0.pak`) stays the user's own |
| Quake 3 | id's 2005 release or ioquake3 (GPL-2.0) | Already on the ROADMAP: start-up time a goal from day one |

Game data (WADs, PAKs) stays the user's own, except freely distributable
shareware data. Each port is likely its own repository (as the Quake 3
note says), measured against the stock-WM baseline (DESIGN.md §10).

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
