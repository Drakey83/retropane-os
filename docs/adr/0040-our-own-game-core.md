# ADR-0040: Our own game core, Xash as the starting point

- **Status:** Accepted (2026-11-18, by the project owner: "Accept all
  four")
- **Date:** 2026-11-18

## Context

The owner, 2026-11-18, after kits 89-91 and the A/B options: "you are only
thinking of how to get the most out of xash, that isn't the right way to
think, the correct way is how can we create our own thing that works
better than xash using xash as a starting point."

Where the frame goes today (run 109, kit 91, the c1a0e cascade's calm
frames, 40.1 ms a frame, samples grouped by subsystem; the heavy half adds
~23 ms, mostly the ~110 debris pieces' light, draw and bones, run 103):

| Subsystem | ms a frame | What Xash does there |
|---|---|---|
| World (BSP walk, projection, surfaces) | 9.8 | `ref_soft`'s walk and per-surface work, every frame, in soft floats |
| Server game and physics | 9.0 | hull traces in floats, entity links, think, every frame |
| Engine misc and the runner | 6.9 | the frame's sending, the sound's writing, the rest |
| Client to server and back | 4.2 | delta-encoding entities into a bit stream, then parsing them back, in the same process |
| Client game | 3.4 | copying entities into physics lists, the HUD |
| Models | 3.1 | bones, lighting, emitting |
| Sending to the 2700G | 2.0 | the faces' words |
| Sound | 1.7 | the mixer |

Three facts measured on the X51v say why tuning Xash has stopped paying:

1. **The time is spread out.** No function is above 4% of the frame (run 109).
   A kit that halves one function gains about 1 fps.
2. **The CPU mostly waits on memory, not on arithmetic.** In runs 89-92, the
   I-cache failed to deliver code for 22% of calm frames and 44% of heavy
   ones, and loads waiting on memory took another 19-33%. 682.5 MHz gained
   nothing (run 88). Xash's code and data layout is what costs the time.
3. **Much of the work is Xash's design, not Half-Life's.**
   - Xash is a networked engine. In single-player, the server and the
     client are the same program, and each frame the server packs every
     entity into a bit stream and the client unpacks it again.
   - Xash recomputes in floats every frame what never changes in a map:
     the BSP's planes, the surfaces' corners, the static light.
   - Xash runs each debris piece through the whole model path one at a
     time, so the code is fetched from main memory again for every piece.

## What others do (findings, not the answer)

- **Xash and GoldSrc:** a client/server engine. Single-player is a
  loopback network game. It was built for x86 with an FPU, and every map
  structure is loaded as the file stores it.
- **Console ports of their day (PS2's Half-Life, Quake on the GBA and
  Saturn):** each rebuilt the engine core for its machine. The PS2 port
  converted maps to the console's own formats and ran the world through
  the vector units. Fixed-point Quake ports precomputed planes and
  vertices in integers. They kept the game code and replaced the core.
- **Modern engines:** precooked, platform-native assets made once, not
  at every load; data-oriented systems (all entities through one stage,
  then the next); a fixed simulation tick with render interpolation.

## Decision

**The game code stays.** The SDK's server and client libraries (monsters,
weapons, HUD, every mod compiled from source) keep the interfaces they
call: the engine functions, entity variables and the client's engine
functions, unchanged. **The engine core under them becomes ours**, one
subsystem at a time. Xash is the starting point and the reference: every
replacement is measured against it, and its picture and sound must match
Xash's, byte for byte where the two can be compared. Each part ships only
when it beats Xash's version on the X51v.

1. **One program, no network inside it (the direct link).** In
   single-player, the client reads the server's entity state straight
   from shared memory: no delta encoding, no bit stream, no packet parse,
   no second copy into the physics lists. Saved games and the game
   libraries' interfaces are unchanged. A future multiplayer goes through
   Xash's network path as it is, kept beside this one.
2. **The map cooked once for the X51v.** At a map's first load, the core
   converts the BSP into its own format and keeps it on the card beside
   the game: integer planes and nodes (kit 90's light nodes, for every
   use), the surfaces' corners pre-transformed into fixed point and packed
   in visibility-cluster order, lightmaps already in the 2700G's texture
   layout, and collision hulls as packed integer nodes. Every later load
   reads the cooked file, which also cuts the 21 s map load. The cooked file
   is checked against the BSP's hash and remade if the hash differs. Data
   from the card is still untrusted (ADR-0039 rule 10): the converter
   bounds-checks everything it reads.
3. **The world drawn from cooked clusters.** The BSP walk runs only when
   the view changes visibility cluster; otherwise last frame's visible list
   is reused. Corners are transformed in integers with iWMMXt (the
   PXA270's SIMD unit: two products a cycle), and only the faces facing
   the view are sent. The 2700G hides what is behind (it sorts every pixel
   by depth itself), so the CPU does no visibility work beyond the PVS
   and the frustum.
4. **Physics in integers.** Hull traces, the player's movement and
   entity links run on the cooked integer hulls (Xash's
   `PM_RecursiveHullCheck` and `SV_ClipToEntity` today in soft floats),
   with the same results. Where an integer result could differ from the
   float one, the float path is kept and the difference is counted.
5. **Models stage by stage.** Every model of the frame goes through bones,
   then lighting, then emitting, each stage over all models before the
   next (kit 89 did this for the lighting: 9.7 to 14.7 fps on the heavy
   half). Rigid pieces such as debris and gibs take a fast path: one
   matrix and no skeleton. Bones and vertices are integer and iWMMXt.
6. **Built around the memory.**
   - The hottest code runs from the PXA270's 256 KiB of on-chip SRAM,
     which the port leaves unused today. A page-table read takes ~94 ns
     there against ~544 in SDRAM (docs/hw/cpu.md). The linker gathers the
     hot functions together, and the page table maps those pages onto the
     SRAM, so no address changes.
   - Each stage's data is packed into cache lines in the order it is read.
7. **Sound mixed once, in integers with iWMMXt,** straight into the OS
   mixer's buffer.

**The order of the work**, biggest measured share first, each a kit with
its own run against kit 91's 24.6 / 15.7 fps (calm / heavy):

| Step | Part | Estimate, calm ms saved | Why that much |
|---|---|---|---|
| 1 | Direct link (1) | 4-6 | the 4.2 ms of packing and parsing, plus the client's copies |
| 2 | Hot code in SRAM (6) | 3-5 | a third or more of the I-cache waits |
| 3 | Cooked map and world (2, 3) | 5-6 of the 9.8 | the walk mostly skipped, the floats gone |
| 4 | Integer physics (4) | 2-4 of the 9.0 | the traces and their soft floats |
| 5 | Models stage by stage (5) | heavy half: 8-12 | the I-cache refetch per piece, the rigid path |
| 6 | Sound (7) | ~1 | the mix and its copy |

**Estimate if every step pays as expected:** the calm frames from 40 to
about 20-24 ms (40-50 fps) and the heavy half from 63 to about 32-38 ms
(26-31 fps). These are estimates from the profile, not measurements. Each
kit's run replaces them.

## Why it beats the originals (rule 0)

- **Xash** pays every frame for a network it does not use, and it
  recomputes in floats what is fixed for the whole map. Ours does that
  work once, at the first load, in integers.
- **The console ports** precooked their maps by hand for one game. Ours
  cooks any Half-Life map, or a mod's, on the device itself. Any user's
  own copy works, with nothing to prepare on a PC (the turnkey rule).
- **Modern engines** cook their assets in a studio's build. Ours cooks on
  the X51v and checks every input, because the card's files are
  untrusted.

## The four lenses

- **Iterate:** Xash is the starting point and stays the reference that
  each part is measured against.
- **Modernize:** cooked assets, data-oriented stages, SIMD, on-chip
  memory.
- **Legacy:** every Half-Life map, model and mod compiled from source
  runs unchanged. Multiplayer stays possible through Xash's network path,
  kept beside the direct link.
- **Better and faster:** each step ships only with a run that shows it
  faster and the same picture.

## Hardware limit or choice?

Choices. The memory waits are fixed by the hardware, but how much code
and data each frame touches is Xash's design, and that is what this
changes. The clock was tried and is not the limit (run 88).

## Alternatives considered

- **Keep tuning Xash (kits 89-91's way):** about 1 fps a kit now; it
  can't reach 30 on the cascade (R324's arithmetic).
- **A new engine from nothing:** loses Xash's tested behaviour, and
  Half-Life's monsters, saves and scripts depend on GoldSrc's exact
  timing. Too much to re-prove.
- **Lower the server's tick (Xash's `sv_fps`, runs 113-116):** +1 fps at
  20 Hz, and it changes the game's timing. It could still be an option,
  but it is not this decision.

## Consequences

- The port's patch grows into a core of its own, still under the
  ADR-0039 rules: modules placed at build time, the game libraries
  unchanged.
- A cooked map on the card is a few MiB per map, in the game's own folder
  (ADR-0039 rule 8).
- Two paths exist while the work runs (Xash's and ours, chosen by a
  switch), so every run can compare them on the X51v.
- Later ports (Doom, Quake, porting-guide.md) inherit the pattern: keep
  the game, cook the data, own the core.

## Votes (the owner's, 2026-11-18: "Accept all four": the direction, cooking at the first load on the X51v (A), the order as tabled, single-player first)

1. **The direction:** our own engine core under Half-Life's unchanged game
   code, Xash as the starting point and the reference.
2. **Cooking:** maps converted at their first load on the X51v and kept on
   the card (A, proposed), or converted on the PC by `xtask` (B: no
   turnkey).
3. **The order:** the table above (direct link first), or another order.
4. **Single-player first:** the direct link replaces the loopback
   network in single-player, and multiplayer keeps Xash's network path.
