# Game Mode, design B (design only, for the owner's go)

**Status: DRAFT, 2026-11-07.** Written on the owner's go of 2026-11-07 on
sheet A's alternative 3 ([sheet A](../research/sheets/game-mode-alternatives-a.md)):
"One kernel, two user-space profiles, fast tier under 2 s keeping programs
in RAM, full tier up to ~15 s writing them to flash or SD. Start design B
on that basis, design only, no code." His brief is
[game-mode.md](game-mode.md). No code, no ADR and no roadmap change come
from this page until he approves it in his own words. Numbers marked
*measured* come from the X51v (the ratchet's floors, `docs/perf-floors.md`,
and run logs); *to measure* marks a number the device must give before the
design leans on it.

## In plain words

Pressing a game's icon puts the rest of the phone to sleep, not to death.
Everything the person had open stops where it is and is squeezed into less
memory (the fast way) or copied out to the card (the slow way that frees
the most), the desktop's pictures and caches are thrown away (they are
rebuilt later, so nothing is lost), and the game gets the screen, the
buttons, the sound and nearly all the memory. When the game ends, by
quitting, crashing or hanging, the same steps run backwards: the game's
memory is wiped, the sleeping programs are woken exactly as they were, and
the desktop comes back showing the last picture it had while it rebuilds
behind it. The kernel and the drivers never stop, the screen never goes
dark, and the card is never unmounted.

Two tiers, picked per game (by the person, or by what the game asks for):

| | Fast tier | Full tier |
|---|---|---|
| Target, each way | under 2 s | up to ~15 s |
| Sleeping programs | kept in RAM, compressed | written to the card (flash for the small records) |
| Memory for the game | everything but the kernel, drivers, reserve and the compressed programs | everything but the kernel, drivers and reserve |
| Needs a card | no | yes |

## Rule 0: what others do, the best way today, why ours wins

- **Windows Mobile 5:** no mode at all; a game is one more process and gets
  what's left; under pressure the shell sends `WM_HIBERNATE` and then
  closes programs. **Android / iOS:** the low-memory killer or jetsam kills
  background apps and relies on apps saving state (lost when they don't).
  **Windows Game Mode, Android game mode:** scheduling priority and
  background throttling, no memory hand-over. **Consoles:** suspend the
  running app in RAM and give the game a fixed reserved partition, which
  wastes memory when no app is open.
- **Best way today, on this hardware:** the kernel already planned per ADR-0005
  and ADR-0011 compresses idle processes in RAM (LZ4, *measured* on the
  X51v: a 4 KiB heap page compresses in 183 µs and decompresses in 134 µs,
  `probe.lz4_compress_page` / `probe.lz4_decompress_page`) and isolates
  every process in its own slot (ADR-0016 rule 1). Game Mode is those
  pieces run all at once, on purpose, with a reserved return record,
  instead of a new mechanism.
- **Why ours wins:** nothing is killed (WM, Android, iOS all kill); the
  memory is handed over, not just the CPU (Windows / Android game modes);
  nothing is reserved when unused (consoles); and the fast tier's
  compression gives the game most of the frozen programs' memory too, with
  no card write, which neither the console nor the WM way does.
- **What overturns it:** in the test criteria below.

## Architecture in words

```
 ┌──────────────── user space ─────────────────────────────────────┐
 │ Full Mode profile             │ Game Mode profile               │
 │  shell, compositor effects,   │  the game process (native port  │
 │  living sky, net polling,     │  or CE binary through the CE    │
 │  indexing, Today, apps        │  personality + libGLES_CL)      │
 │  (suspended / frozen in Game) │                                 │
 ├───────────────────────────────┴─────────────────────────────────┤
 │ Mode manager (a service): the switch, the watchdog, the record  │
 ├─────────────────────────────────────────────────────────────────┤
 │ Resident services: graphics (2700G), input, audio, storage + FS,│
 │ save handling, service manager (ADR-0016 rule 6)                │
 ├─────────────────────────────────────────────────────────────────┤
 │ Kernel: slots, VM, compressed RAM, scheduler, IPC (ADR-0003,    │
 │ 0005, 0008, 0011, 0016) + the return record's reserved region   │
 └─────────────────────────────────────────────────────────────────┘
```

## Kernel vs user space

- **Kernel (never changes mode):** process slots and their domains, the VM
  and page cache, the compressor (ADR-0005 rule 6, ADR-0011 tier 2), the
  scheduler (freezing a process is taking its threads off the run queues),
  IPC, the reserved region for the return record (allocated at boot, never
  reclaimed).
- **Driver hosts (resident, shared, ADR-0016 rule 5):** display and input
  (2700G, keypad, touch, backlight), audio, storage (SD, the file systems),
  power. They don't know about modes except for **exclusivity grants**
  (below).
- **User space:** the mode manager, Full Mode's services and apps, the
  game.

## Service lifecycle

Every Full Mode service implements four calls on its port, from day one:

| Call | What it does | Budget |
|---|---|---|
| `start` | from nothing, or from its record | its own boot time |
| `suspend` | stops its threads' work at a safe point, writes a small record (what it shows, where it was), drops its caches, answers | **≤ 5 ms** each (to measure per service) |
| `resume(record)` | picks up from the record; caches rebuild on demand | ≤ 20 ms to first useful answer |
| `stop` | as suspend, then exits | — |

A service that misses its suspend budget is named in the mode manager's
log and frozen anyway (its threads come off the run queues; its memory is
compressed like a program's), so one slow service can't stall the switch.
**Services that can't be suspended:** the resident ones above (by the
brief), and any service holding the card mid-write: suspend waits for its
current write to finish (bounded by the storage service's longest write,
to measure), never cuts it.

Apps (native and CE) get no new duty: native apps may implement the same
`suspend` (a record in milliseconds); CE binaries get CE's own
`WM_HIBERNATE` (a hint to drop caches), then are frozen as they are.
Nothing is ever closed.

## The return record (reserved RAM)

A fixed region set aside at boot, **proposed 256 KiB** (to size from the
records once the services exist):

- the list of suspended services and each one's record (budget 4 KiB
  each, ~20 services: 80 KiB);
- each frozen process: its slot, its threads' saved registers (already in
  the kernel's thread objects), its page-table root, and, in the full tier,
  the map of where each written-out page went (8 bytes a page: 30 MB is
  7,680 pages, 60 KiB);
- the last frame shown before entry, **not** kept in RAM: it stays in the
  2700G's memory if the game leaves it room, else on the card, 600 KiB at
  16 bits (*measured* size), so the return can show it first (R153's
  snapshot start).

What it buys: the way back never needs the card to find out what to wake;
a crashed game can't corrupt it (it lies outside every user slot).

## The transition, step by step

**In (fast tier), targets:**

| Step | What | Target |
|---|---|---|
| 1 | the mode manager takes the request, starts the watchdog, shows the game's splash (the 2700G plane, no shell work) | 20 ms |
| 2 | `suspend` to every Full Mode service, in parallel (each on its own thread) | 50 ms |
| 3 | `WM_HIBERNATE` to CE apps, the hint to native apps; then every app frozen | 50 ms |
| 4 | caches dropped (picture stores, icon pack's file, page pictures, glyph atlases, the 2D area cache; *measured* sizes in sheet A) | 10 ms (freeing, not writing) |
| 5 | frozen processes' anonymous pages compressed (ADR-0011 tier 2, run to the end instead of to a watermark): at 183 µs a page, 20 MB is ~0.9 s on one core (*measured* rate; the ratio *to measure*) | ≤ 1.2 s |
| 6 | exclusivity grants (display, input, audio) to the game's slot | 5 ms |
| 7 | the game process created, its first thread run | — (game loading is counted apart, below) |
| | **total** | **≤ 1.4 s, inside 2 s** |

**In (full tier):** steps 1-4 the same; step 5 writes each frozen page to
a preallocated swap file on the card instead of compressing it (page-sized
DMA writes, no file-system metadata per page: the file's blocks are found
once at boot). At an SD write speed of 2-5 MB/s (*to measure*, R173's first
step; the read is *measured* at 8.2 MB/s by DMA, `probe.sd_read_256_blocks_dma`)
30 MB is 6-15 s. Compress first and write the compressed pages: half the
bytes if heap pages halve (ratio *to measure*), so ~3-8 s. Internal flash
holds only the return record's copy and small records (slower to write,
wears).

**Out (either tier), targets:**

| Step | What | Target |
|---|---|---|
| 1 | the game ends (quit, crash, hang, force-close: one path) | — |
| 2 | exclusivity grants revoked; display, input, audio reset to known states by their drivers | 20 ms |
| 3 | the game's slot torn down: every page freed, its 2700G memory freed | 50 ms |
| 4 | the last Full Mode frame shown (from the 2700G or the card) | 50 ms |
| 5 | processes thawed: fast tier, pages decompress lazily on first touch (134 µs a page, spread over use); full tier, read back by DMA (8.2 MB/s: 30 MB is ~3.7 s, compressed ~2 s) | fast ≤ 0.2 s; full ≤ 5 s |
| 6 | `resume` to every service from its record | 100 ms |
| 7 | the shell redraws behind the snapshot, then swaps to the live screen | ≤ 1 s, behind the snapshot |
| | **total to a usable desktop** | **fast ≤ 0.5 s, full ≤ 6 s** |

**Game asset loading (counted apart, as the brief asks):** maps, textures
and sounds from the card at the measured 8.2 MB/s read: Doom's 4 MB WAD
~0.5 s, a Quake map with its textures (~10 MB) ~1.2 s, before
decompression. Stated per port as each is built.

## Driver sharing and exclusivity

- One driver instance per device for the whole session (no reload, ADR-0016
  rule 5).
- **A grant** is a token the mode manager gives one slot: while it holds the
  display grant, only that slot's surfaces are composed (the compositor's
  effects stop; the game's surface goes straight to the plane or the 3D
  core's target, ADR-0038's routes); while it holds input, every key and
  touch goes to it except the one reserved key combination that forces the
  way back; while it holds audio, the mixer has one client.
- A grant ends when the slot ends, whatever the reason; drivers reset their
  device's state on revoke, so a crashed game can't leave the 2700G or the
  codec half-programmed.
- Graphics memory: the shell's surfaces and caches in the 2700G's 16 MB are
  freed on entry (only the last frame kept when room allows), so the game
  gets almost all of it too.

## Memory budget

| Item | Size | Source |
|---|---|---|
| Physical RAM | 64 MB | the X51v |
| Kernel, page tables, resident driver hosts (5 x ~64 KiB, ADR-0016 rule 5) | to measure | the kernel phase's live image |
| Resident services (graphics, input, audio, storage + FS, save handling, service manager, mode manager) | to measure | each service's resident size |
| Return record | 256 KiB proposed | above |
| Full Mode caches dropped on entry | ~4-6 MB RAM + ~1 MB 2700G | sheet A's *measured* pieces; the sum to measure |
| Full Mode services suspended (their working memory, compressed) | to measure | per service |
| Frozen programs, fast tier | their size x the compression ratio (to measure) | the CE personality's process sizes |
| Frozen programs, full tier | 0 in RAM (on the card) | — |
| **Left for the game** | **64 MB minus the rows above** | the number B exists for |

## Failure and crash return

- **Crash:** the game's thread faults; the kernel (ADR-0016: its own slot
  and domain) ends the process; the mode manager, holding a handle to it,
  is signalled (an event, ADR-0016 rule 4) and runs the way out.
- **Hang:** the game must present a frame or call into the system at least
  every 5 s (the watchdog's deadline, set per game for long loads); a missed
  deadline shows a "Game not responding" choice (wait, or close) on the
  system's own plane, which the display grant doesn't block. The forced-way-
  back key combination (to pick from the keys WM already gives no game,
  per the owner's rule of no new button jobs) does the same without the
  question.
- **Force-close:** the same path as a crash.
- **Saves:** game saves go through the storage service, which completes or
  rolls back each write (the file system's staged write, as kept maps use);
  a crash mid-save leaves the last whole save.
- **A failure inside the way out** (a service that won't resume): the
  mode manager restarts that service from scratch (ADR-0016's supervisor)
  and logs it; the rest of the session comes back.
- **The hardware watchdog** keeps running through both modes, so a kernel
  hang still resets as today.

## Test criteria

| Test | Pass |
|---|---|
| Transition in and out, fast tier | each under 2 s on the X51v, the median of 10 |
| Transition in and out, full tier | each under 15 s on the owner's card, the median of 5 |
| RAM freed | the memory left for the game matches the budget table within 5% |
| Session intact | 3 native apps and 2 CE apps open with unsaved state, a document open, a game save made: after the way back, every app's memory hashes equal to before entry (frozen pages compared), the document and the save unchanged |
| Crash return | a test game that faults, one that hangs, one force-closed: each returns to a working desktop with the session intact, 10 times each |
| Native port | the first native port (Doom) entered and left 10 times |
| CE legacy | one unmodified WM5 game through the CE personality entered and left 10 times |
| Asset loading | stated separately per game, never counted in the transition |

## Open for the owner (the brief: he decides on conflicts)

- [ ] **The slot limit.** ADR-0003 gives each process a 32 MB slot, as CE5
  did. A game can use more than 32 MB only through mappings outside its
  slot (CE's large-allocation area or the shared region, which CE5 games
  already use for big `VirtualAlloc`s). Design B assumes those mappings
  carry the game past 32 MB; if ADR-0003's area for them isn't large
  enough, the slot model and "as much of the 64 MB as possible" conflict.
  Ruling wanted once the kernel phase sizes that area.
- [ ] **The process model isn't built yet.** Crash return relies on
  ADR-0016's isolation (accepted, steps 1-2 running in the emulator, not
  yet on the X51v); B's crash path is proven only when that is.
- [ ] **The reserve:** 256 KiB proposed; to be sized from the real records.
- [ ] **The forced-way-back keys:** which existing key combination (no new
  button jobs).
- [ ] **Go on design B** as written, or changes.

## Measurements B needs before any code

1. The X51v's SD write speed, large sequential, on the owner's card
   (R173's first step), and internal flash's.
2. LZ4's ratio on real heap pages of the shell and a CE app.
3. Each Full Mode service's and cache's resident size from a run log.
4. The kernel and resident drivers' size in the kernel phase's live image.
