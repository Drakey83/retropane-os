# Sheet A: Game Mode, the alternatives (DRAFT for the owner's vote)

**Status: DRAFT, 2026-11-07.** Written while the owner was away (his list of
2026-11-06, item 9; reminder R173; his brief in
[docs/design/game-mode.md](../../design/game-mode.md)). No code, no ADR, no
roadmap change. Design B follows only after his go on this sheet. Numbers
marked *measured* come from the X51v's logs; numbers marked *to measure*
are estimates that a device run must replace before B relies on them.

## In plain words

When a game starts, the phone should give it nearly all of its 64 MB and
the whole screen, sound and buttons, and when the game ends, everything
the person had open should be exactly where they left it. The same
switch must work for our own game ports (Doom, Quake, ...) and for old
Windows Mobile games run unchanged. There are three ways to do this:

1. **Do nothing:** run games on top of everything else. Simple, but the
   game gets only what the rest of the system leaves over.
2. **Reboot into a game-only system:** rejected by the owner; listed here
   only to say why.
3. **One system, two profiles (proposed):** the kernel and every driver
   stay loaded; entering Game Mode puts the desktop and its helpers to
   sleep, drops their caches, and hands the freed memory to the game;
   leaving reverses it. Two tiers: a fast tier (under 2 s) that keeps
   frozen programs in RAM, and a full tier (up to ~15 s, the owner's limit)
   that writes frozen programs' memory to internal flash or the SD card to
   free even more.

## The alternatives

### 1. Do nothing: Full Mode keeps running under the game

- **Original way (Windows Mobile 5):** a game is one more process; the
  shell, Today plug-ins and services keep running and keep their memory;
  the game asks for what is left.
- **Modern way:** the same on desktops and phones, with the OS trimming
  background apps under memory pressure (Android's low-memory killer, iOS
  jetsam), which kills them.
- **Why not ours:** on 64 MB the shell's caches alone are several MB
  (*measured*: the icon picture store ~1.4 MB, the icon pack's file ~1.5
  MB, glyph atlases 512 KiB of 2700G memory, pictures 256 KiB, the page
  pictures up to ~2.3 MB for Hourly), and its living sky, net polling and
  page rebuilds take CPU in the middle of a frame (the frame budget's
  slow frames, R171/R172). Trimming by killing is what the brief forbids.
- **Overturns it:** a measurement that the game's frame rate and memory
  with Full Mode running are within ~5% of the same game with Full Mode
  asleep (then Game Mode buys too little to be worth its code).

### 2. Reboot into a game image (rejected by the owner)

- **Original way:** dedicated game devices and some homebrew loaders boot a
  separate small system; on the X51v that would mean HaRET again.
- **Why rejected:** it loses the session (everything open is gone or must
  be saved to card and reloaded), reloads drivers and power-cycles the
  panel, and HaRET's start alone is seconds. The owner ruled it out as a
  design, a stepping stone or a fallback.
- **Overturns it:** nothing: out of scope by the owner's ruling.

### 3. One kernel, two user-space profiles (proposed)

- **Original way:** none on Windows Mobile. The closest modern ideas are
  Android's "game mode" and Windows' "Game Mode" (scheduling priority and
  background throttling, no memory hand-over) and console "suspend
  application" (an app's memory kept while a game runs).
- **Ours:** the kernel and drivers (the 2700G, display, input, audio, SD
  and the file system, save handling; for legacy games the CE personality
  and libGLES_CL) never stop. Entering Game Mode:
  1. Full Mode's services get a *suspend* call (each designed from day one
     to stop within a few milliseconds and to keep its state in a small
     record): the shell, the compositor's effects, the living sky, net
     polling, indexing, page rebuilds.
  2. Every cache is dropped, not saved (it rebuilds on return): picture
     stores, the icon pack's file, page and wallpaper pictures, glyph
     atlases, the 2D engine's area cache.
  3. Programs that are open are *frozen* (scheduled no more), never
     killed. **Fast tier:** they stay in RAM. **Full tier:** their pages
     are written to internal flash, or to the SD card when flash is too
     small, and their RAM handed to the game (the owner: Game Mode's full
     tier may require a card, and ~15 s each way is acceptable).
  4. A small reserved RAM region holds the return state (which services
     to resume, the frozen programs' records, the screen's last state),
     so the way back never depends on the card.
  5. The game gets exclusive display, input and audio; a watchdog watches
     it (a crash, a hang, a force-close all take the same way back).
  Leaving reverses it: the game's memory freed, frozen programs thawed
  (read back in the full tier), services resumed from their records, the
  caches rebuilt on demand while the first screen shows from the snapshot
  (R153).
- **Same for both game paths:** a native port and a CE-personality binary
  are both "a game process" to Game Mode: one entry, one exit, one
  watchdog.
- **Why it wins:** nothing reloads (the drivers stay warm, the panel stays
  lit), nothing is lost (frozen, never killed), and the RAM handed to the
  game is everything Full Mode held that isn't the kernel, the resident
  drivers or the reserve. The fast tier meets the brief's 2 s; the full
  tier trades up to ~15 s for the most RAM, which the owner chose.
- **The limit, said plainly:** fast, no-loss and maximum RAM can't all hold
  at once on 64 MB. The fast tier keeps frozen programs' memory, so the
  game gets less; the full tier frees it but pays the write and read time
  (*to measure*: ~30 MB at 2-5 MB/s would be ~6-15 s each way on SD;
  internal flash is slower to write and wears, so it suits the small
  records best). The person picks the tier, or the OS picks it from what
  the game asks for.
- **Overturns it:**
  - the fast tier's switch measured over 2 s each way on the X51v;
  - the full tier's switch over ~15 s on the owner's card;
  - any frozen program found changed after a return (the no-loss rule);
  - a crashed game that leaves the display, input or audio unrecovered;
  - the RAM freed by entering measured under what the budget below claims.

## The memory budget (to measure before B)

| Item | Now | Source |
|---|---|---|
| Kernel and resident drivers | to measure | the kernel phase's process model (an open architectural line) |
| Full Mode's caches dropped on entry | ~4-6 MB RAM + ~1 MB 2700G memory | *measured* pieces above; the sum to measure |
| Full Mode's services suspended | to measure | each service's resident size |
| Frozen programs (fast tier: kept; full tier: written out) | to measure | the CE personality's process sizes |
| Return-state reserve | proposed 256 KiB | to size from the records; the brief asks it stated |
| Left for the game | the rest | the number this sheet exists for |

## What B needs first (measurements, no code)

1. The X51v's SD write and read speed for large sequential files on the
   owner's card, and internal flash's (R173's first step).
2. Each Full Mode piece's resident size from a run log (the picture store,
   the pack's file, page pictures, atlases).
3. The process model's isolation (the game in its own address space, for
   the crash path), which depends on the kernel phase's architectural line.

## For the owner

- [ ] Go on alternative 3 (one kernel, two profiles, fast and full tiers)
  as the design B is written from.
- [ ] Or a change (say which).
