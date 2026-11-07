# Game Mode: the owner's brief and first thoughts

**Status: brief recorded (2026-11-01); sheet A approved 2026-11-07
(alternative 3); design B drafted ([game-mode-b.md](game-mode-b.md)), for
the owner's go.** The owner asked
for this to be kept so it isn't forgotten. Per the brief itself, no code,
ADR, roadmap or other doc changes until the owner approves the design in
his own words. Deliverables when the work starts: A, a one-page
alternatives sheet; B, the full design document (both defined below).
Reminder R173 brings it up when the process model is designed.

## The owner's brief (2026-11-01, as given)

> You are working on RetroPane OS, my clean-room Rust (no_std)
> reimagining of Windows Mobile for the Dell Axim X51v.
>
> Hardware, fixed: PXA270 at 624 MHz, no FPU, 64 MB RAM, Intel 2700G
> GPU, VGA 480x640 display, SD storage, HaRET boot from FAT32. GPLv2
> tree. No Dell drivers, no proprietary code, no software-renderer
> shortcuts for games.
>
> Context: games are a first-class use of this OS. Native GLES ports
> (Doom, Heretic, Quake, Quake 2, Quake 3, later Half-Life via Xash3D)
> run direct on my own 2700G driver. Legacy WinMo binaries run
> unmodified through the CE personality, with GLES calls served by my
> clean-room libGLES_CL. Game Mode must work for both.
>
> Goal: design Game Mode as a first-class mode of the OS, from the
> beginning, not as a bolt-on later.
>
> Game Mode is option 2 only. Do not design, prototype, or implement
> option 1.
>
> Option 1, rejected and out of scope: rebooting into a separate minimal
> game image, reloading the kernel, reloading drivers, or using HaRET
> again to switch modes. Do not propose it as a stepping stone, a
> temporary version, or a fallback. List it in the alternatives sheet as
> considered and rejected, with the reason, and move on.
>
> What Game Mode must do:
>
> 1. Single kernel, two profiles. One kernel stays resident for the whole
> session. Full Mode and Game Mode are user-space profiles on top of
> that same kernel, sharing the same drivers. No kernel reload, no driver
> reload, no display panel power cycle, no SD remount on switch.
>
> 2. No loss, no clunky return. Switching into Game Mode must not lose
> anything in the OS. No app state lost, no open document lost, no save
> data lost, no session lost. Switching back must return gracefully to
> the complete OS, to the session I left, not to a fresh boot and not to
> a desktop with everything closed. If exact preservation of a running
> app's full memory is impossible within the RAM budget, say so plainly
> in the design, quantify the cost, and propose the closest preservation
> mechanism for my decision. Do not silently kill apps and call that
> preservation.
>
> 3. Fast transition. Transition time, excluding game asset loading,
> must be fast. Target: under 2 seconds each way, measured on device.
> Game asset loading (maps, textures, sounds from SD) is separate and is
> not counted in that transition time, but must be stated separately in
> the design. Design for speed: drivers stay resident, services are
> designed from day one to be stoppable or suspendable quickly, return
> state is kept in a small reserved RAM region, not saved to SD on every
> switch, no full filesystem sync on switch, no lazy work that makes the
> switch feel slow.
>
> 4. Strip only what is not needed for the game. In Game Mode, stop or
> suspend: the full UI shell and compositor effects, background
> services, animations, indexing, and any other fancy OS work that is
> not needed to run the game. Keep, always: kernel, 2700G driver,
> display, input, audio, SD and filesystem, save handling, and for legacy
> games, the CE personality and libGLES_CL. The game gets exclusive
> display, input, and audio while it runs.
>
> 5. Memory, the main restriction. The whole point is to give the game as
> much of the 64 MB as possible. The design must include a memory
> budget, with measured numbers, not guesses: RAM used by the kernel and
> resident drivers, RAM used by Full Mode services, RAM freed by entering
> Game Mode, RAM reserved for return state, and RAM left for the game.
> State the reserve size explicitly, and what it buys. If a service
> cannot be stopped quickly or freed cleanly, name it and explain why.
>
> 6. Failure handling. If a game crashes, hangs, or is force-closed, Game
> Mode must still return to Full Mode gracefully, without losing OS state
> or corrupting saves. Design the watchdog, timeout, and cleanup path for
> this case, not just the happy path.
>
> 7. Applies to every game path. The same Game Mode entry and exit must
> work for native GLES ports and for CE-personality legacy binaries. Do
> not design two different game modes.
>
> Deliverable, in this order, and stop for my go before any code changes:
>
> A. A one-page alternatives sheet, in plain English first, jargon after.
> At least 3 alternatives, including do-nothing (leave Full Mode running
> under games) and the rejected reboot option, plus your proposed
> mode-switch design. For each: original way, modern way, why yours
> wins, and the measurement that would overturn it. If a claim cannot be
> measured on device, say so.
>
> B. A full design document, plain English first, then technical
> detail. Include: architecture diagram in words, kernel vs user-space
> split, service lifecycle (start, suspend, stop, restart), return-state
> preservation mechanism and its RAM cost, transition sequence in and out
> step by step with timing targets for each step, driver sharing and
> exclusivity rules, memory budget table, failure and crash return path,
> and test criteria.
>
> Test criteria must include, at minimum: measured transition time in
> and out, measured RAM freed, session intact after return (apps,
> documents, saves), game crash return test, and a native port test and
> a CE legacy test.
>
> Guardrails for this session:
> - I decide on conflicts. If the design conflicts with an earlier ADR, a
> driver constraint, or the no-loss requirement, stop and ask me, do not
> resolve it by quietly changing the requirement.
> - Alternatives-considered is required, not optional.
> - This is analysis and design, not a roadmap change and not code. Do
> not change code, ADRs, or docs in the repo until I approve the design
> in my own words.
> - Be honest about limits. If fast, no-loss, and maximum RAM cannot all
> be achieved at once on 64 MB, say which one has to give, by how much,
> and let me choose.

## The owner's later decision (2026-11-01, his words)

> can we back up that state onto internal rom? and if it wont fit there,
> sd card, game mode can be a feature that only works properly if you
> have expandable memory. and as long as the transition isnt too long, i
> just dont want it taking 45 second to a minute, but waiting 15 seconds
> before and after a game to get the highest frame rates possible would
> be a good trade off.

So the brief's "under 2 s" holds where nothing has to be written out, and
a transition may take **up to ~15 s each way** when saving app state frees
more RAM for the game. State that won't fit in internal flash goes to the
SD card; Game Mode's full-RAM tier requires one.

## First thoughts (Claude, 2026-11-01; for the design, not decisions)

- **Fits the OS as built.** One kernel, resident drivers, the graphics
  service and the 2700G driver shared: exactly how today's image works.
  Option 1 (reboot into a game image) would cost both no-loss and speed
  on this hardware; rejected, as the brief says.
- **Where the RAM comes from.** Most of what Full Mode holds and can be
  freed fast is caches, not state: today's measurements are the picture
  store ~1.4 MB, the icon pack's file ~1.5 MB, the page pictures, the
  wallpaper copies, the 2D engine's caches (glyph atlases 512 KiB,
  pictures 256 KiB, areas). Dropped on entry and rebuilt on return, they
  lose nothing. The design measures each (the brief: numbers, not
  guesses).
- **The tension, named.** Real app state can't be freed without being
  kept somewhere. Our own apps can each write a small "hibernate" record
  (milliseconds). Unmodified legacy WinMo programs can't: kept resident
  (frozen, never killed) or their whole memory written out (to internal
  flash if it fits, else SD, the owner's decision above): ~30 MB at 2-5
  MB/s is ~6-15 s each way, to be measured on this X51v's card and flash
  first. Internal flash is slower to write than SD and wears, so it suits
  the small records best.
- **The return must be fast.** Today rebuilding the Today screen's
  pictures alone takes ~1.5 s (70-130 ms a picture): the return needs the
  snapshot start already planned (R153), the shell's own state restored
  rather than recomputed.
- **Crash safety needs isolation.** A game that crashes must not take
  the OS with it: the game in its own protected address space, a
  watchdog and a cleanup path that returns the display, input and audio
  to Full Mode. That depends on the process model (an architectural line
  of the kernel phase), which the design will name before relying on it.
- **Graphics memory too.** The 2700G's local memory is shared by the
  shell's screens and caches and a game's textures: Game Mode frees the
  shell's (the wallpaper, both screens' worth of pictures, the atlases)
  for the game, and the design's budget counts it as well as RAM.
