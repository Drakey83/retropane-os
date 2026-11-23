# What the Half-Life port taught us (read before any port)

The owner, 2026-11-22: "should we make a design port doc so that you know
in the future what will and won't work when making these ports so that we
don't have to do 6000 test every single time?" Yes. This page is the
Half-Life port's lessons, each with the run or commit that proved it and
the ports it carries to. [porting-guide.md](porting-guide.md) is the
recipe (what to build, in what order); this page is what to know before
building, so the next port starts where Half-Life ended, not where it
began. Half-Life took more than 330 device runs; most of the expensive
ones are in the sections below, each now a line to read instead of a run
to make.

**How to use it.** Read it whole before a port's design. When a port
finds a new lesson (a cause proven, a way that lost, a trap), add it here
in the same commit as the fix, with its run. A lesson that a later
measurement overturns is changed here, not left standing.

Which ports a lesson carries to: **All** (every port), **Q** (Quake and
the Quake-lineage engines: Half-Life's GoldSrc grew from Quake, so its
renderer, maps, traces and decals carry nearly whole), **D** (Doom and
Heretic: a column renderer, no 3D world on the chip, so the renderer
lessons carry only where they say so).

## 1. The test method (what makes a run worth its time)

| Lesson | Proof | Carries |
|---|---|---|
| **Compare builds on a fixed-seed scripted scene, never across plays.** Two plays take different routes and use the flashlight differently; their averages differ by more than any one change. A scripted scene (the player placed, the step and seed fixed, ending at a set frame) repeats to the millisecond. | Runs 330-332 looked 11% slower than run 318 (31 against 35 fps). Runs 335a-e, the same `-rpfight`: run 318's build 16,599 ms, today's 16,579 ms for 500 frames. The code had not slowed. | All |
| **Scripts must step on game time, not the wall clock.** The scripted walk (`-rpwalk`) turns five times a second of real time, so a faster build walks a different route: it is a soak test, not a yardstick. | R360, 2026-11-22 | All |
| **Put what the player really does into the yardstick.** The fight had no flashlight; the owner plays with it on, and it relights walls every frame (995 of every 1,000 frames in run 334 against 68 in the whole fight). `-rpflash` adds it. | Runs 334, 335b | All |
| **Every change keeps its old way behind a switch** (`-rpskybox`, `-rpcoplanar`, `-rpnostage`, `-rphullfloat`, `-rpdecalsoft`), so one run per switch names its cost. Only `args.txt` changes between such runs: copy the kit once, run many. | Runs 335a-e: five answers in one sitting | All |
| **Prove the cause before the fix**: the symptom counted in the run's own log, the code read, the evidence named. Guessing cost whole kits (the flashing textures, R352, had four wrong suspects before the map's own planes named it). | R352; memory "prove cause first" | All |
| **Read the samples, not the averages.** `-rpsample` and `cargo xtask xash-profile` give each function's share and the forty slowest frames one by one; turn shares into milliseconds a frame (share x frame time) before comparing two runs. | R357, R358, R360 | All |
| **Step through the owner's video frame by frame** (`ffmpeg -vf select=...,tile`): one clip showed the ribs present in frames 0-10 and gone in 12-23 with the view still, which no screenshot could. | IMG_0666, aa40d006 | All |
| **Know what the emulator can and cannot prove.** It proves code paths, counts, memory, a whole run's end and the software renderer's picture. It cannot show the 2700G's picture of a game frame (its 3D model draws only the test harness's objects), model caches or the PXA270's timing, or catch a race between the CPU and the chip. A fix whose proof is a picture or a speed needs the X51v. | R288, R311, run 334's emulator check | All |
| **The emulator wants its pinned release** (`RETROPANE_QEMU=release`); a fork build that is not exactly it refuses to run. | 2026-11-22 | All |
| **Every fixed class gets a guard test** reading the source or the build (`xtask/src/xashkit.rs` tests): the binds' layout number, the held rewrites, the sky faces, the coplanar faces. A test that fails is fixed the same day: the binds test failed for a day unnoticed and the owner's touch-key change never reached his saved config. | 959307f9 | All |
| **Copied values are claims.** Dell's and the engine's numbers (the TA's 1.0e-5 epsilon, the engine's table sizes, its near plane) are first drafts to measure, never the cause or the answer by default. | R359, R354 | All |
| **Logs fill.** A 10-minute play overflows a 64 KiB run log; the card log (`cargo xtask cardlog`) holds the whole run and survives hangs and resets. Keep per-frame lines out of play runs. | R326, run 334 | All |
| **Start the chip from a known state.** A run that hung the 2700G leaves the next run blind (sound, no picture) unless the runner resets it. | R344 | All |

## 2. Drawing on the 2700G

| Lesson | Proof | Carries |
|---|---|---|
| **Our own renderer over the tile accelerator**, not a GL layer and not the CPU drawing pixels: the software renderer was 71% of a frame. | R287, ref-rp.md | Q; D for its 2D and sprites |
| **Frames in flight: never write video memory a render may still read, and never wait for the chip to avoid it.** Two frames are in flight (`RP_IN_FLIGHT` 2). Writing a page a render reads flashes textures; waiting for the render before writing throws away the CPU and chip working side by side (R352's hold made 995 of 1,000 frames wait in run 334). Write somewhere no in-flight frame reads (a fresh place or a ring), then reuse places only after two frames. Half-Life's way: a rewritten lightmap whose home a frame in flight drew goes to a three-slot ring (128 KiB a slot), else is written at home at once; a dynamic light alone never rewrites a home. Run 338a: 1.3 ms a frame back with the flashlight on (27.4 to 28.5 fps), nothing held. | R340, R352, a68545bb, R360 (runs 337a-b, 338a-b) | All |
| **A dynamic light added as a second pass is not the same look when the light is capped.** Half-Life caps light plus lightmap at full texture brightness; the flashlight's centre reaches the cap by itself, so an additive pass would show the spot up to twice as bright. Keep the light in the lightmap (or draw only the capped difference). | R360, docs/research/sheets/r360-flashlight.md | Q |
| **Two surfaces at one depth flicker on the chip.** A brush entity whose face shares a world face's plane and side (c2a5f's steel ribs, 666 faces on that map) is settled by the chip's depth rounding, which changes as the view moves. Find the ties once at load from the map's shared planes and push the entity's face 2^-9 of its depth back (the world wins, as the software renderer draws it). Decals go the other way, 2^-9 forward. | aa40d006 (run 334: "no glitching") | Q |
| **The sky is a surface at its own depth.** Drawing the sky box first and skipping the sky faces let whatever a map keeps behind its sky brushes show through. | bd491b71 | Q |
| **Draw every surface kind from day one.** Sky, water, conveyors, sprites, beams, particles, see-through `{` textures and render modes were each left for "a later step" and each found by the owner in play. List the engine's surface kinds and render modes before the first picture and draw each, plainly at worst (memory "draw everything"). | R309, R320, R303, R321 | All |
| **Textures and lightmaps on the chip with its own mipmaps** (option B), not the software renderer's surface cache at one mip level a face: the cache's level snapped as the player neared a face (popping). | R297 | Q |
| **Clip at the near plane in fixed point; the weapon in the hands at the game's own near plane (4 units), the world at 1.** Dropping a triangle with a corner behind the eye made model parts vanish; a near plane of 1 put the shotgun's shell in front of the face. | R299, R316 | Q |
| **Point sampling: the chip takes the nearest texel, the software renderer floor(u)**: half a texel apart unless corrected. | R330 | All |
| **Engine helpers may read data only the GL renderer builds.** `R_DecalSetupVerts` reads the GL per-face polygons a software renderer never makes, and `R_DecalVertsClip` returned without setting its count, so decals came and went on a stack word's leftover. Read each engine helper a new renderer calls for what it assumes. | b98784be | Q |
| **Know the game's own effects before chasing a bug**: the "red trapezoid" chased for three runs was Half-Life's damage-direction arrow. | Runs 320-327 | All |
| **Decals as their wall draws them** (lit by the wall's lightmap on the second texture unit, one pass), built once and kept, not baked into a rebuilt surface. | fb6eec9a, f68f7b32 | Q |

## 3. The CPU (a PXA270 with no FPU)

| Lesson | Proof | Carries |
|---|---|---|
| **Sample before tuning.** Run 4 put six tenths of a frame in the software-float routines and one tenth in drawing; the order of work comes from the samples. | porting-guide step 10 | All |
| **Our float helpers in assembly, and a fused dot product**, bit-exact against IEEE: the floats went from the top of the list to a few percent. | R316 (kit 65) | All |
| **Integer rewrites must be measured in play, not only proven equal.** R358's integer hull traces had 0 differences over ~92,000 traces but gave no speed in play (their count, not their floats, was the cost). | R358, run 331 | Q |
| **Prefetch hints can lose.** Two kits' hints made their functions slower on the PXA270's four fill buffers; each hint is measured and taken out if it loses. | R317, R318 | All |
| **Monsters' planning comes in bursts.** One soldier's cover search made 3,300 trial walks in one think (a 290 ms frame). Budget each think's walks and resume at the next think. | R357, 3ec15951 | Q (and any game AI) |
| **The OS mixer's per-stream cost** shows up as a few percent of a game's frame: one stream at the output's rate should take the fast path. | R296, R333 | All |
| **Read the compiler's output for the hot loop.** For ARMv5 LLVM made each 64-bit product with a constant coefficient a `UMULL` and an `MLA` sign fix, and wrote a byte slice it couldn't prove aligned a byte at a time: the mixer's chain went 176 to 135 cycles a frame, bit-exact, by hiding the constants (`black_box`) and viewing the ring as words (`rp_hal_simd::words_mut`). | R360, runs 341a-d | All |
| **A kit's speed is only comparable with its own build checked.** A kit built while another build of the same checkout ran linked the generic soft float (12% slower, runs 340a-c void); the kit build now refuses a module without our helpers. | R360, `helpers_linked` | All |

## 4. Memory (about 37.6 MiB of heap in Game Mode)

| Lesson | Proof | Carries |
|---|---|---|
| **The engine's tables are sized for a PC.** 20.4 MiB of zeroed tables at default sizes; each is cut to what the game needs, and each cut is a decision written down. | R277 | All |
| **Keep textures as 8 bits with their palette**, expanded into video memory only: 13.9 MiB saved, the out-of-memory in free play gone. | R335 (kit 120) | Q; D's are 8-bit already |
| **Watch the hidden big allocations**: a save's 640 x 480 32-bit preview screenshot (1.17 MiB, and a copy) ran out of memory at an autosave; a texture slot's 256-byte name times 1,536 slots. | Run 318w, R354 | All |
| **A level change holds two maps at once.** Plan the peak for both, not one. | R298, R310, R354 | Q |

## 5. Loading and the card

| Lesson | Proof | Carries |
|---|---|---|
| **A missing file costs ~200 ms to find**: the engine asked for 494 pictures the Steam copy does not have. The read-only tree is listed once and missing names answered from the list. | R276, e142bd33 | All |
| **Small reads are slow** (30,251 read commands for one start): big files read once into place. | R281 | All |
| **Cook a map once on the device** (integer planes, nodes as cache lines, the face tables) and keep it on the card beside the game, checked against the map's own hash. | Kit 103 | Q |
| **Make generated pictures once** (the menu's background built on the PC) instead of at every start. | R275 | All |

## 6. Controls, sound, settings

| Lesson | Proof | Carries |
|---|---|---|
| **Saved bindings follow a layout number raised with every change to the shipped binds**, checked by a test against the binds' checksum, so a user's remaps survive and a new default still arrives. Forgetting the raise kept the owner on old binds twice. | 48051789, 959307f9 | All |
| **Client-side weapon prediction must match the server**: turning it on made the crowbar, the gluon gun and the gauss misbehave until each weapon's predicted state was fixed. | R348-R350, 4020e46f | Q |
| **The stylus look needs the touch chip read at the frame's pace and smoothed**, or it is jerky. | R355 | All |
| **After a crashed run the sound link may not start**: the controller is reset before use. | R301 | All |
| **Set the backlight for games**: the runner otherwise inherits whatever Windows Mobile left. | R341 | All |
| **Every port has the Picture brightness slider** (the owner, 2026-11-22: "universal in every port I make"). The X51v's panel shows a dark game's lower third as near black, and the game's own gamma and brightness did nothing useful on it. One slider replaces them: nine steps, each moving the screen's tone curve (the 2700G's scan-out gamma table, the runner's, free each frame) and the game's light together (Half-Life: its lightmaps lifted, textures' contrast kept), seen at once behind the menu and saved with the options. A port supplies only its light half; the screen-curve half is the runner's. | R343 (runs 290-292: the owner's pick M75), `rp_picture` in r_emit.c and mainui VideoOptions | All |
| **Every port has a Show FPS setting in its own menu**, saved with the options (the owner, 2026-11-22: "a selectable thing in each port"): the engine's own counter where it has one (Half-Life: `cl_showfps`, Video options), else the runner's frame-rate bar (`-rpfpsbar`, a test word until a port needs it as a setting). | R261, R361 | All |

## 7. A new port's first day (checklist)

1. Read this page and [porting-guide.md](porting-guide.md).
2. List the engine's surface kinds, render modes, effects and HUD effects:
   each gets a drawing path before the first picture (section 2).
3. Build the fixed-seed yardstick scene first (a fight or the heaviest
   scripted moment, stepped on game time, the player's flashlight or
   equivalent on), with an end frame, before any tuning (section 1).
4. Give every optimisation and every look change a switch for its old way.
5. Size the engine's tables and find its hidden big allocations (saves,
   screenshots, names) against the heap before the first map (section 4).
6. Plan video memory for two frames in flight from the start: no write to
   a place a render may read, no wait (section 2).
7. Sample on the X51v before the first optimisation (section 3).
8. List every copied value (Dell's, the engine's) as an open question
   (docs/quality-audit.md).
9. Put the Picture brightness slider and the Show FPS setting in the
   game's own options menu from the first build (section 6).
