# R360: the OS mixer's chain and the model corners' cache (rule 0 sheet, 2026-11-23)

Measured first (X51v, the fixed-seed `-rpfight -rpflash` on c2a5f, 500
frames, run 339a: 17,465 ms, 28.6 fps). The play stretch's samples are
flat; two places were read down to the instruction:

- **The OS mixer** (`rp_snd_write` 3.3% + `rp_snd_played` 1.2% of the
  stretch: both are the mixer's wakes, run with interrupts masked, so the
  samples land at the call's end). Its own cycle line: 374 cycles an
  output frame, of them **187 the chain** (speaker filter, limiter,
  dither), 50 adding the streams, 57 moving the ring's end. The runner's
  disassembly (`rp_audio::mixer::Mixer::mix`): every 64-bit product with
  a constant coefficient compiled as `UMULL` plus an `MLA` sign fix (six
  multiplies a sample for three products) and each subtraction a 64-bit
  one, ~32 instructions a sample with the three coefficients reloaded
  from the literal pool every sample; a separate peak pass; the dither's
  generator stored to memory every frame; four byte stores a frame
  (the compiler can't prove the ring word-aligned); the game's stream
  copied in with two byte loads and a sign extension a sample.
- **The model corners** (`RP_StudioCmdSend` 4.6%, the top engine
  function): ~200 cycles a triangle for a loop of ~30 instructions; the
  samples bunch just after the loads of each corner's 1/z from `rp_sc`
  and its point from `rp_scxy`, which `RP_StudioCmdLit` wrote just before.

## Lever 1: the mixer's chain and copy (built)

| | What it does |
|---|---|
| Windows Mobile / Dell | the waveout mixer in C, a sample at a time; no speaker filter or limiter |
| Textbook | direct-form-I biquad, then a limiter pass, then a dither pass, each a loop |
| Ours before | the textbook passes in integers, the numerator folded (three products); compiled as above |
| **Ours now** | the denominator's coefficients stored negated and hidden from the compiler (`black_box`) so each product is one `SMULL`/`SMLAL` (16 instructions a sample); the peak branch-free; dither and packing in one branch-free pass, the generator in a register; a word store a frame (`rp_hal_simd::words_mut`: the ring starts on a cache line), bytes only if it ever isn't; the stream copied by halfword loads (`rp_hal_simd::halves`) |
| Why it wins | the same samples bit for bit (`the_fused_pass_is_the_old_way_exactly`: 40,000 blocks, speaker on and off, overloads, release, silence, 16-bit values, word and byte outputs), roughly half the instructions; nothing to set |
| Old way | `-rpsndchain0` (the runner: `Mixer::set_reference_chain`) |
| What overturns it | the mixer's cycle line in a device run not lower than 187 for the chain, or a sound difference (impossible by the test, checked by ear in the owner's play) |

## Lever 2: the corner tables' lines asked for before they are written (built, measured, taken out)

| | What it does |
|---|---|
| GL / ref_soft | a vertex buffer in a write-allocate cache: written lines stay |
| 2005 drivers | vertex buffers uncached or write-combined, read back never |
| Ours before | `rp_sc` / `rp_scxy` written by `RP_StudioCmdLit`, read back by the sort; the PXA270 allocates on reads only, so a line the model's streams (vertex records, commands) pushed out is written to memory and missed on the read (kit 53's finding for `rp_mq`) |
| **Ours now** | `PLD` of the table lines three corners ahead of their first write (four for `rp_scxy`), so the writes and the sort's reads hit; spread through the loop, not all at once (a PLD with no fill buffer free waits) |
| Why it wins | the same words; a line already there costs about a cycle |
| Old way | `-rpnoscpld` |
| What overturns it | the fight not faster than with `-rpnoscpld` in the same kit |

## Results (runs 341a-d, 2026-11-23, one kit, hands-free)

| Run | Switches | 500 frames | Mixer chain, cycles a frame | Mixer wake whole |
|---|---|---|---|---|
| 339a (the last kit) | | 17,465 ms (28.6 fps) | 187 | 374 |
| 341b | `-rpsndchain0 -rpnoscpld` (both old) | 17,449 ms | 176 | 365 |
| 341a | (new) | 17,412 ms (28.7 fps) | 135 | 332 |
| 341d | (new, again) | 17,411 ms | 135 | 327 |
| 341c | `-rpnoscpld` | 17,409 ms | 135 | 331 |

- **Lever 1 holds:** the chain 176 to 135 cycles a frame (-23%), the
  fight 38 ms faster over 500 frames (0.08 ms a frame); the two runs of
  the new kit agree within 1 ms. Kept, the default.
- **Lever 2 does nothing** (17,409 without it, 17,411 with): the sort's
  hot loads were not missing on these tables, so the samples' bunching
  after them is the load-use and multiply latency of the loop itself, or
  misses elsewhere. Taken out (porting lessons: a hint that does not win
  comes out). What it leaves: `RP_StudioCmdSend`'s ~200 cycles a
  triangle are still unexplained; next, the event counters (data-cache
  misses, CPU C5) around the sort, not another guess.
- **Runs 340a-c are void:** that kit was built while another session
  built in this checkout, and its client and server modules linked
  compiler_builtins' generic soft float in place of ours (19,627 ms,
  25.4 fps). `cargo xtask xash-kit` now refuses a module without our
  helpers (`xashkit::helpers_linked`).

## Step 3: where the sort's cycles go, counted by the X51v (runs 342a-d)

| | What it does |
|---|---|
| Vendor / textbook | VTune or `perf record -e cache-misses` on a PC; instrumenting the function with counter reads (spans) |
| Ours | the runner's event sampler (kit 88, `-rpsampleon <event>`): the PMU interrupts every 2,000 D-cache misses (or 200,000 stall cycles) and the sample keeps the instruction, so the misses and stalls land on the sort's own instructions (`pchist.py`-style histogram beside the disassembly), with nothing added to the code being measured |
| Why it wins | a span says *how much* the sort waits; the samples say *on which load*, which is what a fix needs; no instrumentation to perturb a ~30-instruction loop |
| Runs | one kit (HEAD), the fixed-seed fight: 342a cycles, 342b D-cache misses (0x0B), 342c data-stall cycles (0x02), 342d I-cache stall cycles (0x01) |

**What runs 342a-d show** (the fight: 17,404 ms in 342a, the kit's
repeat; 342b's sample room filled during the map's load, so its fight
has no samples, only its totals): the fight's cycles wait on data 27.3%,
on the I-cache 15.1%, on full write buffers 4%, ~8 D-cache misses a
thousand cycles. `RP_StudioCmdSend` is 5.1% of the cycles but 2.6% of the
data stalls and 1.8% of the I-cache stalls, so **~14% of its own cycles
wait on data, ~5% on the I-cache, ~80% are its own instructions**
(`RP_StudioCmdLit` the same: ~15%, ~3.5%). That is why lever 2's PLD did
nothing. The cycle samples spread evenly over the state loop (sort's first
pass): ~80 instructions a triangle, each corner's 1/z and x, y read by
three triangles, the cross product in 64 bits (`SMULL` and a
`SUBS`/`SBCS` compare), the screen's four edges tested on three corners,
values spilled to the stack, then a call to `RP_LitFill` for each of a
kept triangle's three corners (two of every three rejected by the mark),
and a counter in memory a culled triangle. ~1.78 ms a frame for ~4,800
model triangles: ~190 cycles a triangle.

## Lever 3: the sort from corner codes (built, measured, taken out)

| | What it does |
|---|---|
| GL / ref_soft | clip with a code a vertex (Cohen-Sutherland outcodes); cull a triangle at a time in floats |
| Ours before (kit 68) | each triangle's three corners tested one by one: three 1/z reads, a 64-bit cross product, twelve edge compares; three `RP_LitFill` calls a kept triangle |
| **Ours now** | `rp_sort.h`: each vertex's code made where it is projected (`RP_CornerCode` in `RP_StudioVertSet`: behind the near plane, past each of the four edges, x and y within 2^14), a corner copies its vertex's byte (`rp_sccode`); `RP_SortStates`: a triangle is the AND and OR of three bytes, a loop each for strips, fans and no cull, its cross product in 32 bits when the three corners are within 2^14 (each difference under 2^15, each product under 2^30, their difference under 2^31: exact), else the old 64-bit one; `RP_SortNeed`: the corners a sent triangle uses, so each is filled once in corner order; the cull counters in registers |
| Why it wins | the same state for every triangle and the same corners filled (host check, all four states, strips and fans, cull -1/0/1, random, edge and extreme corners: 12.1 million triangles, 0 differ; `-rpsort2` on the device counts any); roughly half the instructions a triangle |
| Old way | `-rpsort0`; `-rpsort2` both, compared |
| What overturns it | the fight not faster than `-rpsort0` in the same kit, or `-rpsort2` counting a difference |
| The check | `ports/xash/ref_rp/check/sort_check.c` compiles `rp_sort.h` itself (gcc, `-fwrapv`) |

**Runs 343a-d (the first build: the codes made in a pass over a command's
corners, rolled through registers along the strip):** exact (`-rpsort2`:
1,972,567 triangles, 0 differed, 0 commands filled other corners) but
**slower**: 17,501 and 17,506 ms against 17,447 with `-rpsort0` in the
same kit. The samples: the sort 418 (new) against 214 + 147 (old, the old
states now their own function), `RP_LitFill` 47 against 62. The corner
pass waited on each 1/z the corner builder had just written (the PXA270
allocates on reads only: 38 samples on one load's use), and the rolled
loop spilled ten words a triangle. Rebuilt as above (the codes with the
vertex, no corner pass, a loop a kind); stop rule written before runs
344a-d: not faster than `-rpsort0` in the same kit, and lever 3 comes out.

**Runs 344a-d (the rebuild, one kit, 2026-11-23):** exact again (`-rpsort2`:
1,972,567 triangles, 0 differed, 0 fills). The fight: new 17,494 and
17,497 ms, `-rpsort0` 17,515, `-rpsort2` 17,699. The new way beats the
switch in its own kit by ~20 ms, but **the kit is ~90 ms slower than the
code before lever 3** (342a 17,404; 341a/d 17,412 and 17,411): the
switch's "old way" still carried the lever's support code. The samples
against 342a: `RP_StudioCmdLit` +75 (the code byte stored each corner,
two identical runs differ by up to 34), `RP_StudioVertsFixed` +26 (the
code made each vertex), the sort -25. **Lever 3 is taken out** (the stop
rule; r_emit.c back to f2d782b9's). What it leaves: the state loop was
about a third of the sort's samples; the rest is the run emission (the
inlined `RP_StudioRun`, the TA's words), the next place to read.

## Step 4: the run emission, read (run 342a, no lever)

The lead step 3 left was wrong, and the samples say so: the TA's words
are copied by `RP_StudioWords`, a function of its own at **11 samples
(0.2%)**. `RP_StudioCmdSend`'s 324 samples are its passes over the
triangle states, each a few instructions a triangle: the state loop
~120, the run loop that builds each run's indices ~45, the three
`RP_LitFill` calls a kept triangle ~25, the gap fill ~15, the entry ~25,
counters stored to memory. Lever 3 showed that reshaping those passes
costs more where the corners are built than it saves, so no lever here.

**Where the fight's time is (run 342a), for the next session:** no
function above 5.1%; the software-float, division and copy helpers
**18.1%** together, their callers spread (`rp_fadd_asm` 3.1%: 13% from
`RP_HullCheckQ`, 8% `R_StudioSetupChrome`, 7% `R_AddDynamicLights`;
`rp_fmul_asm` 2.3%: `R_AddDynamicLights` 13%, chrome 9%, lighting 9%);
sound ~5.3% (`rp_snd_write` 2.5%, `rp_snd_played` 1.3%, the engine's
`S_MixChannelToBuffer` 1.5%: ROADMAP 7f); `rp_gfx_frame_parts` 3.4% (the
runner); the models' path (`RP_StudioCmdSend`, `RP_StudioCmdLit`,
`RP_StudioVertsFixed`, `RP_ProjectS`) ~13.5% together. A gain toward 40
fps (~30% of the frame) has to come from a stage removed, not a function
tuned: ADR-0040's 7d (collision on the integer hulls) and 7f (the
engine's mixing into the OS mixer), and the float callers above moved to
integers as a class.

## Step 5: the float callers ranked, and the dynamic lights' luxels in integers (built, kept)

**The ranking** (run 342a, every helper sample charged to the function at
its return address): the helpers are 16.1% of the fight, and no caller
holds more than 1.8% with its own samples: `RP_HullCheckQ` 1.8% (its
crossings' divide and lerps, which go straight into the game's physics:
left), `R_AddDynamicLights` 1.0% (two thirds in helpers), `CL_PredictMovement`
0.9% (copies), `R_StudioSetupChrome` 0.9%, `S_PaintChannels` 1.3% (a
memset), `R_StudioSetupLighting` 0.7%. The fused dot (`rp_fdot3_asm`)
costs what its separate calls did (run 239), so most of this class only
goes faster as integers, and outside the luxel loop below those are
close, not bit for bit (a ruling for the owner, as kit 51's bones were).

| | What it does |
|---|---|
| ref_soft / ref_gl | each luxel a light reaches walked in floats: the distances' subtract, multiply and truncation, a compare, the amount's subtract, multiply and truncation (about nine software-float calls a lit luxel on the PXA270) |
| **Ours** | the same loop in integers from the floats' bits (`r_surf.c`): the amount `floor(256 rad) - 256 dist` (exact: `rad - dist` is exact in float when 0 <= dist < rad), the test `dist < ceil(minlight)`, each distance from `sl`, `tl` in 16.16 with a flag for the bits below; a light whose fraction lies within 2^-8 of a whole number (where the float subtraction's rounding could cross one), `sample_frac` not 1, or values past 2^13 keeps the float loop |
| Why it wins | the same sums bit for bit, no calls in the loop |
| Old way | `-rpdlfloat`; `-rpdlcheck` both ways on every luxel |
| Proof | the PC (gcc, SSE floats: IEEE as our helpers, `-rpfloat`): 2.67 million lights, 329 million luxels, 0 differ; the X51v (run 345c): 111,439 lit luxels of the fight, 0 differ, no light on the float loop |

**Runs 345a-d (2026-11-23, one kit):** new 17,360 and 17,360 ms (28.8
fps), `-rpdlfloat` 17,406 (the build before it, run 342a: 17,404),
`-rpdlcheck` 17,905. **Kept: 44 ms over the 500 frames.**

## Step 6: the paint buffer only when something paints into it (built, kept)

| | What it does |
|---|---|
| Quake / Xash | each pass clears the room and paint buffers, mixes into the room buffer, adds it with the master gain into the cleared paint buffer, then clips the paint buffer out to the DMA ring (a 6 KB clear, an add pass and a re-read a pass) |
| **Ours** | the paint buffer cleared only when a voice or music stream paints into it directly (the only writers); otherwise the transfer clips the room buffer with the gain straight out: 0 + x is x, the same samples (`s_mix.c`, `RP_TransferRoom`) |
| Old way | `-rpsndfuse0`; `-rpsndcheck` both, every output sample compared |

**Runs 346a-d (2026-11-23, one kit):** new 17,304 and 17,308 ms (28.8
fps), `-rpsndfuse0` 17,367 (the build before it: 17,360),
`-rpsndcheck` 18,002 with 1,936,248 output samples compared, 0 differed,
every pass straight from the room buffer. **Kept: ~60 ms over the 500
frames.**

## Step 7: the resampling loop at a step of one half, each source sample once (built, kept; iWMMXt measured, out)

Run 342a: ~90 of `S_MixChannelToBuffer`'s 95 samples in one loop, 8-bit
mono resampled (Half-Life's 11,025 Hz sounds into the 22,050 Hz mix).

| | What it does |
|---|---|
| Quake / Xash | each output sample steps the phase, loads its source sample and multiplies it by both volumes |
| **Ours** | at a step of exactly one half the phase's carry alternates, so each source sample feeds exactly two outputs (the first alone when the phase starts in its upper half): loaded and multiplied once, added twice; any other step the old loop |
| iWMMXt (asked for, measured) | the pair added as 64-bit lanes (TMCRR, two WLDRD, two WADDW, two WSTRD): 17,275 ms, the same as the scalar pair (LDM, four ADD, STM): taken out, so the game's code stays off the media unit's registers |
| Old way | `-rpmixpair0`; `-rpmixcheck` the old loop on a copy, every sample compared |
| Proof | the PC: 2,000,000 calls (8- and 16-bit, every phase), 0 differ; the X51v (run 347d): 2,709,818 samples in 4,097 calls, 0 differ |

**Runs 347a-e (2026-11-23, one kit):** the pair 17,272 and 17,275 ms
(28.9 fps), `-rpmixpair0` 17,297 (the step 6 build: 17,304 / 17,308),
`-rpmixwmmx` 17,275, the check 17,956. **Kept: ~25 ms.**

**R360 since run 342a (17,404 ms):** steps 5 to 7 together 17,272 ms,
132 ms (0.26 ms a frame) faster, every picture and sample the same.

## Step 8: the weapon data copied into its prediction slot only when it changed (built, measured, taken out)

| | What it does |
|---|---|
| Xash / GoldSrc | `CL_PredictMovement` copies the last frame's 5.6 KB of weapon data into its prediction slot every rendered frame, though packets come at the server's 20 Hz tick |
| Tried | each slot tagged with the generation and parse it was copied from (the generation moved by `CL_ParseClientData` and `CL_ClearState`, a slot forgotten when prediction writes it): the copy skipped when the slot still holds it; `-rpwdcopy` the old way, `-rpwdcheck` |
| Proof | the X51v (run 349c): 461 of 769 copies skipped, 0 differed |

**Runs 348a-c and 349a-d (2026-11-23):** new 17,290 / 17,283 / 17,279 ms,
`-rpwdcopy` 17,304 / 17,299: ~16 ms faster than its own switch in both
kits, but **not faster than the build before it** (step 7, runs 347a/e:
17,272 / 17,275): the whole kit moved ~25 ms, old way included, more than
one branch a frame can cost (the code's layout). By the rule step 3 left
(a lever is judged against the build before it), **taken out**; worth
retrying only inside a batch with another lever, where a layout shift
shows as both moving.

The test runs from run 349 play at 5% of full volume (the owner,
2026-11-23): `-rpvol` sets the codec's hardware level, applied after
every sample is computed. Runs 348a/b (full) against 349a/b (level 5),
the same code: 17,290 / 17,283 and 17,304 / 17,299 ms, the mixer's
cycles the same (135 the chain, 47 adding): the level changes no reading.

## Step 9: the server's edict walk (ADR-0040 7d; built, measured, taken out), and what a kit's layout costs

Run 347a: physics about 16% of the fight with its helpers, spread out
(`SV_Physics` itself 2.1%, `RP_HullCheckQ` 2.0%, `RP_TraceContents`
1.2%, the rest under 1% each). `SV_Physics`' own samples sit on its walk
over every edict: the edict's index made again from its address (a
divide) to test it, and four lines asked for two edicts on whether that
edict is free or not.

| | What it does |
|---|---|
| Xash (kits 61-71) | every edict tested through `SV_IsValidEdict` (its index from its address), four hints two on for each, used or free |
| Tried | `!free` (the index is the loop's, in range: the same answer), the `free` line asked four on and the physics lines two on only for an edict in use; the count read each step as before (an entity's physics can make new edicts); `-rpphywalk0` the old walk, `-rpphycheck` both tests (140,868 edicts, 0 differed) |

**Runs 350a-d:** 17,312 and 17,316 ms, `-rpphywalk0` 17,312: **no gain**,
taken out.

**Runs 351a-b (the committed code, step 7, its own kit):** 17,276 and
17,278 ms, kit 347's 17,272 and 17,275: the device has not drifted. Kits
349 and 350 ran their old ways at 17,299-17,312 though the code on that
path was the same: **a rebuild that moves the code shifts the fight by up
to ~40 ms (0.25%) on its layout alone** (the modules sit one after
another, so a change in the engine moves the client's and server's
addresses too). A lever smaller than that is believed only when its own
switch in the same kit and the build before it both say so.

## Step 10: the hull crossings (ADR-0040 7d), read, not built

Run 351a (the committed code): `RP_HullCheckQ` 127 samples, 2.0% of the
fight. 85 are its own, the hottest on loading each clip node's children
and its plane's type (the walk's memory, not its arithmetic); 42 in the
float helpers, the crossings' work (`rp_fadd_asm` 23, `rp_fdot3_asm` 6,
`rp_fmul_asm` 4): two plane differences, the fraction's add, subtract
and divide, four lerps, ~17 operations a crossing.

- **No exact integer form:** the fraction is an IEEE divide and the mid
  point float multiplies and adds; the physics must get the same bits,
  which means the same roundings, which is what our software-float
  routines do. In integers it would be the soft float written again.
- **The exact speed-up left is a fused routine** (`a + f (b - a)` in one
  call with the same roundings, as `rp_fdot3_asm` is for the dot): the
  fused dot measured about its separate calls' cost (run 239), so the
  most this could save is ~0.2% of the fight, below the ~0.25% a kit's
  layout alone moves it (step 9): not measurable, not built.

**The larger lead in this area:** `CL_PredictMovement` replays every
command not yet acknowledged, from the last server frame, on every
rendered frame (1.0% with its copies, plus its player traces in
`PM_PlayerTraceExt` and the hull walk), and a packet comes only at the
server's 20 Hz tick: the same commands from the same state give the same
moves until a packet arrives. Keeping each replayed command's result
while its inputs (the state it starts from, the command, the physics
entities it can touch) are unchanged would run each command once, not
once a frame. It needs a design that proves the inputs the same (the
moving physics entities are part of them), so it is a sheet first.

## Step 11: the prediction replay (design sheet; not built yet)

**What it costs now (run 351a, the committed code, helpers charged to
their callers):** `CL_PredictMovement` 1.0% of the fight (its own 12
samples, `memcpy` 47: the 6.5 KB `local_state_t` copied for every command
it runs, `*to = *from`, and the 5.6 KB weapon data from the frame),
`PM_PlayerTraceExt` 0.55%, `HUD_WeaponsPostThink` 0.34% (the client's
weapons, run for every replayed command), `CL_RunUsercmd` 0.16%, the
client's own `PM_*` ~0.1%, and the client's share of `RP_HullCheckQ`'s
2.0% (most of it the server's monster moves; not split in the samples).
About **2.2-2.6% of the fight** runs inside the replay loop.

**Why most of it repeats.** Every rendered frame (~29 a second) the
client starts again from the last server frame and runs every command the
server hasn't acknowledged, one after another (`CL_RunUsercmd`: set up
`pmove`, `pfnPlayerMove`, `pfnPostRunCmd`). An acknowledgement comes only
with a packet, at the server's 20 Hz tick (kit 106). Between packets, a
command already run last frame is run again from the same start, with the
same command, time, seed and physics entities. Only the newest command,
which is still being built, changes. Run 250 showed the cost follows the
count: at 15 Hz (more commands waiting per packet) `CL_PredictMovement`
went 67 -> 82 samples and `PM_PlayerTraceExt` 58 -> 83.

| | What it does |
|---|---|
| Quake / QuakeWorld (1996), GoldSrc, Xash | the whole chain replayed from the last server frame every rendered frame |
| Source (2004; `cl_pred_optimize`, default 2, per Source SDK 2013's `prediction.cpp`: a claim to read again, not verified here) | (1) no copy and no replay of old commands when no packet came since the last frame, only the new command run; (2) when a packet's state matches what was predicted for its command, no replay at all |
| Ours, phase 1 | Source's (1) proven input by input, with counters: a command's stored result reused while every input to its run is the same, the newest command always run |
| Ours, phase 2 (only if phase 1's counters say it pays) | Source's (2) bit for bit: when a packet arrives, the server's state for the acknowledged command compared with our stored prediction for it (6.5 KB, once a packet, 20 a second); equal means the chain after it is still valid and only moves slots |

**Why ours beats the original.** Source decides by "no packet arrived",
an argument; ours keys every input the run reads and proves each skip
with a check switch on the X51v (every skipped command also run and its
result compared byte for byte), so a missed input shows up as a count,
not as a feel. And phase 2 is taken only if the counters show the
server's state usually equals the prediction to the bit (the server's
weapons and the client's are different code and may round differently).

**The inputs of one command's run, and how each is keyed** (phase 1; a
slot's stored key is compared before the run, and the run skipped only
when every part is equal):

| Input | Where it comes from | Key |
|---|---|---|
| the start state (`from`) | the server frame `cl.frames[parsecountmod]` for the first command, else the previous command's result | `cl.parsecount` plus a generation moved by every write to a frame's player, client and weapon data (`CL_ParseClientData`, `CL_ClearState`, as step 8 had), and for later commands: the previous command was skipped or its run was equal |
| the command | `cl.commands[n].cmd` (52 bytes) | its bytes, compared every time (the newest command is rewritten each frame until sent; a sent one should not change, but a compare costs less than a doubt) |
| time and seed | `frame->time` plus each command's `msec`; the seed is the command number | the command number; the time after the run stored with the result (so a double added in the same order, the same bits) |
| `runfuncs` | true the first time a command runs, false after | the result the same either way is the claim the check proves (HL's `pm_shared` and weapons gate sounds and events on it); the first run of a command is never skipped |
| the physics entities | `CL_SetSolidEntities` (step 7a's direct link already keys its build on the parse) and `CL_SetSolidPlayers` | the build's key (parse, entity range, world, player) and `numphysent` |
| the rest of `pmove` | `CL_SetupPMove`: `clgame.movevars`, `cl.local.health` (`dead`), `cl.maxclients`, `cls.spectator` | the movevars' bytes compared (~200, once a frame) and the three words |
| the slot itself | `cl.predicted_frames[(parsecountmod + i) & mask]`, written only by this loop | a slot's key cleared whenever anything else writes it (the base copy) |

**What a skipped command still does:** its predicted origin copied, its
`processedfuncs` set, the stored time taken, `cl.local.lastground` taken
from the stored result. The loop's last command (the one the screen is
lerped to) always runs, so the client DLL's globals (the weapon objects
`HUD_WeaponsPostThink` loads from `from` each run, `g_lastFOV`) end each
frame as the old loop leaves them.

**Switches and proof.** `-rpreplay0` the old loop; `-rpreplaycheck` runs
every command anyway and compares the whole `local_state_t`,
`lastground` and the stored time with what the skip would have used
(must be 0 differences), and counts what phase 2 would see: at each
packet, whether the server's state for the acknowledged command equals
our stored prediction for it. The log prints commands run, skipped,
differed, and the phase-2 matches.

**Estimate.** At ~29 frames a second against 20 packets, about one
command in two to three per frame is a repeat. Skipping it saves roughly
a third to a half of the loop's 2.2-2.6%: **~0.7-1.2% (~120-200 ms of
the fight)**, three to five times the layout noise. Phase 2 would add
the frames right after a packet, if their states match.

**What overturns it.** The check counts any difference (then the
missing input is found and keyed, or the lever is taken out). The fight
is not faster than both its `-rpreplay0` run and the build before it
(the step 9 rule). The counters show fewer repeats than estimated.

**The batch (hands-free, 5% volume, `-rpfight -rpflash`, 500 frames):**
a/b the lever, c `-rpreplay0`, d `-rpreplaycheck`. The kit is built and
checked on the PC first (it builds, and the emulator's fight runs with
`-rpreplaycheck` at 0 differences, if the emulator still runs the fight).
