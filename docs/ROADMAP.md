# Roadmap

> Status: living document, updated with each change. Phases overlap. The **reverse-engineering (RE) track runs
> in parallel from Phase 0**, because it's the critical path for the hardware
> requirements (3D, 2D, Wi-Fi).
> No dates yet. Each phase ends with a demo on the **real X51v**, not just the emulator.

```
Phase:   0     1     2     3     4     5     6     7     8     9
Build:  [fnd][emu][krn][drv][ce ][ui ][net][gfx][web][pwr/flash]
RE:     [rom dump · tracer · CPLD · 2700G 2D/3D/video · ACX100 · power IC ───────────────]
GATE:   ▲ now: the performance gate (below) blocks every phase until it is done
```

---

## Now: the gate (owner, 2026-10-21; restructured by the owner's rulings, 2026-10-22)

**No building on unmeasured foundations.** The gate holds the
**architectural** lines of [2700g-full-potential.md](research/2700g-full-potential.md)
and [cpu-full-potential.md](research/cpu-full-potential.md): the ones whose
answer changes a design (an ADR's decision, memory layout, clock or power
settings, driver architecture, a compositor or renderer choice), each tagged
with the phase whose design needs it (`arch N` in the ledgers). Everything
else is an **optimization**, in [backlog.md](research/backlog.md) by payoff
against device time, worked continuously from the top, each landing with
its benchmark in the ratchet. The classification and the owner's rulings:
[gate-classification.md](research/gate-classification.md).

The rules (owner, 2026-10-22):

- **A phase's gate lifts** when its architectural lines are ticked (used, or
  declined with the reason written down: measured, unmeasurable, or not
  worth the device time) **and the ratchet runs**; the owner lifts it. The
  gate now holds **phases 2, 4 and 5** (kernel memory, drawing, the shell
  and compositor); video (phase 7) and power (phase 9) lines gate their own
  phases when those start.
- **Work is held back only by a named line.** Work waits only if its design
  depends on an open architectural line, and the session names that line;
  "the gate" alone is not a reason. Everything else proceeds.
- **The gate can't grow on its own.** No new architectural line without the
  owner's explicit yes (sweeps and reads like P19 propose their finds).
- **Unmeasurable lines are declined with the reason**, no defaults (P16:
  no battery reading; CPU V4: voltages need the device opened), and
  reopened if they become measurable.
- **The ratchet** (`cargo xtask perf`) is the regression guarantee,
  unchanged. "Verify yourself" is unchanged: an unverified Dell or Intel
  claim used in code stays an open quality-audit item wherever it sits.
- **This section is read at the start of every session (CLAUDE.md).**

**Count (`cargo xtask gate`, 2026-10-22): phases 2, 4, 5: 14 of 30
architectural lines ticked (12 partial, 4 open)**; phase 7: 0 of 2; phase 9:
2 of 11; the backlog: 24 optimizations (7 ticked, 16 partial, 1 open).
`cargo xtask ci` prints the count every run, and every summary quotes it.

In order (the gate's own steps; the backlog runs alongside, top first):

- [ ] 1. **Finish everything already started, then the gate's own work (owner, 2026-10-21):**
  - [x] the 16-bit screen (ADR-0023 Q1 A, Q2 L1): built and committed (8219182), pushed; **proven on the X51v 2026-10-22** (`rp-today`, the owner: "it looks good"; touch latency a new finding: audit C58)
  - [x] the device batch (one session, numbered steps first): loader reinstall on the card; `rp-today` on the 16-bit screen (its proof); the guard run version 6 (C46: each end-of-card wait logged); run FS (C43: a real 2D stall, the owner watching, last); results recorded *(done 2026-10-22: loader installed (backup `retropane-backups/2026-10-22`) and started after a soft reset; `rp-reconnect` put the CPLD's USB bits back; run FS: queued 2D work survives the clock off, one MBX read with it off hangs the bus (watchdog): the guard is C43's fix; the loader's board line isn't readable over USB: C57)*
  - [ ] C39: what each image changes on the board and whether a crash puts it back, measured; then the put-backs (quality audit)
  - [x] **Queued (owner, 2026-10-22): the crash-screen backtrace walker**, after the `rp-today` 16-bit run passes and before the next round of device GPU runs (depth, transparency); a small standalone task, nothing else moves. The exception's entry captures r11; a bounded walk (each link in RAM, word-aligned, strictly rising, at most 16 frames, a broken chain reported as such) to the crash screen and log; frame pointers opt-in per image (`rp-crash` first, `--cfg rp_frame_pointers`, ADR-0001); `cargo xtask symbols <image> <addr...>` (llvm-symbolizer) turns the addresses into functions. Accepted when: host tests on synthetic chains (valid, corrupt mid-chain, truncated, may-omit garbage), an emulator check, and a deliberate crash on the X51v shows a correct chain on the frame-pointer build and only the no-chain line on a may-omit build; no crash-path rule broken. *Built 2026-10-22 (ADR-0001 "Built"): host tests (9, 6 mutants), `emu-crash` follows six crash kinds to `_start` (2 emulator mutants), `emu-run-limit` checks the may-omit line; `symbols` uses llvm-nm and llvm-objdump (no llvm-symbolizer in the toolchain); found and fixed C61. **Proven on the X51v 2026-10-22 (the owner present):** the frame-pointer build's panic gave `CHAIN 0xa8008ae8 0xa8008714 0xa80082cc 0xa8008ab8 0xa80080a8 start` (`panic_fmt`, `main` at main.rs:150, `__rp_image_main`, `rp_main`, `_start`: the emulator's chain), the may-omit build (`RP_MAY_OMIT=1`) only `no frame chain (built may-omit)`; both back at the loader, on the card as crash-0001/0002*
  - [x] **The default picture (owner, 2026-10-22): the S-curve at strength 1.75 and saturation 1.2x** as the shell's default, both before the dither (the curve in the renderer's dither step, the saturation as a picture loads); then `rp-today` on the device with it (docs/design/display-calibration.md) *(built 2026-10-22: `rp_draw::look` on the wallpaper as it loads, the icons at build time and the shell's theme colours; `RP_PICTURE_ORIGINAL` builds the old picture; emu-today pixel-exact. **Proven on the X51v 2026-10-22: the owner, "it looks great"**)*
  - [x] C43's fix: no MBX register access while its clock is off (a guard, a host test, a mutant, the fork modelling the hang), then FS3 on the device *(FS3 passed on the X51v 2026-10-22: 7 checks, 0 BAD, the owner watching; the fork's hang model is a fork release item)*
  - [x] C40: build scripts that miss a picture arriving in `local/` (quality audit) *(fixed 2026-10-22: the last case, an older-dated copy overwriting a picture, by the `RP_LOCAL_INPUTS` fingerprint)*
  - [x] C41: the emulator checks refuse a build that isn't the pinned release (quality audit) *(fixed 2026-10-21, c45f881: quality audit C41)*
- [x] 2. Both checklists made countable: every line gets a status (open / used / declined) and the run that shows it; **`cargo xtask gate`** counts them (and, from step 2b, the benchmark results) and `cargo xtask ci` prints it every run, so the count comes from a check, not from memory
- [ ] 2b. **The performance ratchet (owner's vote, 2026-10-21):** a device benchmark suite of what people feel (tap to screen, frame times scrolling and in the menu, 3D frames a second, boot time, copy and upload speeds, battery at idle and under load), each result kept as its floor, as code size already is: a change that makes one slower fails until fixed or approved by the owner. Every new feature from then on comes with its own benchmark and budget, and its ADR names the checklist's tricks it uses. *Built 2026-10-22: `cargo xtask perf`, 99 floors. Touch: the owner chose the hybrid (2026-10-22): recorded pen samples replayed through `rp-today`'s filter and drawing path for the ratchet, real taps at milestones (docs/design/perf-ratchet.md). *Built 2026-10-22:* `RP_TODAY_REPLAY` (the owner's 2026-10-17 taps, the real sample's conversions kept, nothing logged mid-run), the suite's sixth step, `emu-today` checks it; its floors wait for its first device runs, which also answer C58 (the card log's line cost: C59)*
- [ ] 3. The 2700G's clocks (phase 5: P1, P2, V1, V2, V12, V13, V14): soaks warm and cold at the cores' divide 1 (run G4: 3D 1.81x, 2D 1.89x, short checks on one unit), the per-device self-test, an ADR
  - **The plan (2026-10-22; the owner's go for the warm soak given, the cold soak: the owner's date, 2026-10-22 evening).** Only G5's 60 s soak exists (99.67 MHz, cores / 1: 37 rounds, 0 bad, 2026-10-20). To build first, as an `rp-gpubench` run (soak), emulator-checked before the device:
    - **Two settings, 20 minutes each:** 99.67 MHz with the cores at divide 1 (the candidate), then 133.25 MHz CL2 (the fastest seen right); Windows Mobile's clock put back and a control after each.
    - **Stressed every round:** a textured 3D render, 2D fills and copies, the CPU streaming SDRAM, the 16-bit screen scanning out; every render checked pixel-exact against its reference; the underrun flag (`DINTRS` bit 4) and the memory controller's registers (`LMTYPE`/`LMREFRESH`, V12's lost writes) read each round; a card-log line a minute (rounds, BAD count, the worst render time); a check-in each round (the loader's 30 s run limit); the screen labelled with the setting and the minute in big text.
    - **Stops on the first BAD,** puts the clock back, says so on screen and in the log (a failure is the finding, never retried silently). The dark-display and slow-state findings (docs/hw/2700g.md, runs G4-Q) are watched for, not assumed away.
    - **Warm soak:** run unattended once built (about 45 minutes), the device warm from the session.
    - *Built 2026-10-22: `rp-gpubench` run GS (`RP_GPUBENCH_SOAK`; `RP_GPUBENCH_SOAK_SECS` for a shorter one; device build `RP_GPUBENCH_SOAK=1 cargo xtask codegen-variant rp-gpubench soak`). 133.25 MHz runs with the cores at divide 1 (G5: right there, 3D 16.6 ms). Each round: G5's check (flush and read-back, memory pattern, 2D fill, the 3D scene pixel-exact), a 2D copy of the picture while the CPU streams 4 MiB of SDRAM (the copy checked word for word), `COREPLL`/`LMTYPE`/`LMREFRESH`/`LMTIM`/`MBXCLK` read back, `DINTRS` bit 4; the screen a 16-bit RGB565 label scanned out throughout; `DSIG` read each minute. `emu-gpubench` run GS: the pinned rp-v0.30.0 doesn't model Windows Mobile's 2700G clocks and the check accepts its refusal; on the wm-clocks fork build (`RETROPANE_QEMU_ANY`) both 70 s settings stepped, 240 rounds each, 0 BAD, controls GOOD, put back. Quality audit C66 (the crash path's clock put-back for every clock run) fixed with it.*
    - *Warm soak run on the X51v 2026-10-22 (`local/rp-gpubench-runGS1.txt`, card log `gpubench-0052`, unattended): **0 BAD at both settings**, 643 rounds at 99.67 MHz / 1 and 677 at 133.25 MHz CL2 / 1 in 20 min each, no drift, registers held, no underrun, the display sending throughout; 3D 22.85 / 17.95 ms and 2D fill 4.23 / 3.09 ms with the 16-bit screen on, against 35.58 / 6.35 ms at Windows Mobile's setting (docs/hw/2700g.md "The clocks' warm soak"). Next: the cold soak (the owner's date), then the per-device self-test and the ADR; which setting ships is the owner's call.*
    - *Cold soak run 2026-10-22 evening (`local/rp-gpubench-runGS2-cold.txt`, `gpubench-0053`; an hour asleep, the owner watching the first five minutes): **0 BAD at both settings, the same times as warm**, no underrun, the display sending throughout. Next: V1's short run, the per-device self-test, the ADR.*
    - *V1 run 2026-10-22 (run GV, `gpubench-0054`): the local memory runs on the system clock (ticked). Next: the per-device self-test (drafted for the owner's vote: [design/clock-selftest.md](design/clock-selftest.md)), then the clocks ADR.*
    - *Self-test voted 2026-10-22 (1A, 2A, 3A, 4A); **[ADR-0035](adr/0035-2700g-clocks.md) accepted by the owner the same day: P1, P2, V2, V12, V13, V14 ticked.** Left in step 3: building the self-test and the guard (phase 5 work, with the GPU driver).*
    - *Run GC, the content map (2026-10-22, `gpubench-0057`): **the cores at divide 1 draw Gouraud wrong at every system clock**; the cores at divide 2 under 133.25 MHz right but one word in one kind. ADR-0035's ladder reopened; P1, P2, V2, V13 back to partial. Next: map the cores at divide 2 under every system clock and the cores at divide 1 at or below 75 MHz, then the owner rules the new ladder; then the soaks of the chosen settings with every kind of drawing, warm and cold.*
    - *The overclock is not given up (owner, 2026-10-23: "there must be more clever ideas"). Ideas to measure, each a short unattended run: **(a) the fine map:** the divider can be 1-4 and the system clock set in fine steps, so the cores' Gouraud limit is found to the MHz (divide 1 at 74.75, 83, 91 MHz) and the memory's limit apart from it (divide 2 at 108-133 MHz; divide 3 at 150+ for the memory alone); **(b) the error's size:** every wrong pixel tallied by how far off it is (+1 step, other, wrapped), to test the carry inference; **(c) colours one step below the maximum** at divide 1: does the wrap vanish, leaving only +1 steps; **(d) Gouraud drawn as a texture:** textures and bilinear filtering were right at every clock, and a 2 x 2 texture whose fourth texel is the sum of two corners minus the third interpolates exactly linearly, so a lit triangle could be drawn as a textured one; **(e) the divider per pass:** the divider alone changes without relocking the PLL (G5), so the cores could run at divide 1 for textured passes and divide 2 for Gouraud ones, if the switch is fast enough (to time). Whether a game may accept a measured +1 step that a UI may not is the owner's call, after (b) and (c).*
    - ***The owner's directive (2026-10-23): no setting is called a real limit until everything is ruled out: 133 MHz is the aim, and "we did something wrong" is checked before "the chip fails".*** Two different 133s: **the system clock (the memory's) at 133.25 with the cores at divide 2** nearly works (1 word in 2 of 18 renders); **the cores at 133** fail Gouraud already at 99.67, where only the cores' divider differs from Windows Mobile (the same PLL and memory timing), so that failure is the cores' clock itself. The checklist, beside (a)-(e) above: **(f) our set-up against Dell's:** every MBX register Dell's driver writes before 3D work (the render table's unexplained words `0x61C`, `0x644`-`0x64C`, `0x6D0`, `0x6D8`; any gating or pipeline bit) compared with ours, and the MBXCLK enable (Dell writes 2D+3D before 3D work: so do we) checked at each step; **(g) the same frequency from a different PLL setting** (M, N and P giving 133.25 MHz with a different internal VCO frequency) in case jitter, not the frequency, is the limit; **(h) the one wrong word at 133.25 / 2 pinned:** the memory at 133.25 with the cores at divide 3 (44 MHz, below Windows Mobile's 49.8): an error that stays is the memory's, one that goes was the cores'; **(i) the 2700G's local-memory timing at 133** (`LMTIM`, `LMREFRESH`) loosened a step, toward safety, if the memory is the one at fault (a timing change on the 2700G's own memory, not the CPU's SDRAM, which stays Dell's by the owner's rule: to confirm with the owner before any run).*
    - ***Run GC2's rules, so a failure can't be the test's fault (the owner, 2026-10-23: "make the overclocking test as robust as possible"):*** **1. the references proven:** each kind rendered 5 times at Windows Mobile's clock (all equal), checked against the host model's exact pixels (`rp-mbx3d-render`'s model, which matched the device before), and taken again at the end of the run (equal to the start: nothing drifted); **2. the reading proven:** every picture read twice, the second after a 50 ms wait (equal: not read before the core's writes landed), and the reference read the same way; **3. interleaved controls:** each setting's renders alternate with renders at Windows Mobile's clock (A, B, A, B), so a stuck state or a slow drift shows in the controls, not as a clock failure; **4. order:** every setting run rising, then again falling (G2's way: a failure that depends on the step before shows as a difference); **5. a clean start each time:** the 3D core reset and every MBX register written again before each setting, and the clock, memory and MBX registers read back before and after each render; **6. enough renders:** 20 per kind per setting, so an error rate, not a guess; **7. every error described:** each wrong pixel tallied by kind (+1 step, -1, other, wrapped at white) and by place; **8. isolation:** the same frequency from two PLL settings (g), the cores and the memory moved apart with dividers 1-4 (a, h), and colours capped below the top (c); **9. the display's state:** the underrun flag and `DSIG` read around every render. A setting is called a real limit only when it fails with every one of these holding.*
    - *Built 2026-10-23: `rp-gpubench` run GC2 (`RP_GPUBENCH_GC2`; device build `RP_GPUBENCH_GC2=1 cargo xtask codegen-variant rp-gpubench gc2`, sha a44fd8ca, unattended, about 45 minutes: 8.1 MB/s CPU reads of 2700G memory, run GS). Nine kinds (GC's seven and Gouraud with every colour one step below the top, RGB565 and ARGB8888: (c)); twelve settings, run up then down (rule 4): the cores at divide 1 at 74.75, 83.20, 91.00 and 99.67 MHz (a); the cores at divide 2 at 108.33, 117.00, 125.67 and 133.25 (a); the memory at 133.25 with the cores at divide 4 and 3 (h; dividers 3 and 4 never used before: their render times show whether `MBXCLK` bits 3-2 divide as read); 99.67 / 1 and 133.25 / 2 again from a second PLL setting, its internal frequency twice the first's (g). Each rule: (1) references rendered 5 times, their bands hashed and held to the host model's (`boot/gpubench/src/gc2.rs` shares the scenes with a host test that renders them through the model; the table `gc2_model.in`), taken again at the end; (2) every picture read at once and 50 ms later, both against the reference, and a wrong picture read a third time at Windows Mobile's clock (in the memory, or in the reading?); (3) every render at a setting followed by one at Windows Mobile's clock; (5) `Renderer::start_clean` (new: the core reset and every register written) at each setting, and `COREPLL`, the divider, `LMTYPE`, `LMTIM`, `LMREFRESH` and 15 render-table registers read before and after every render (the 3D ones first read once, logged first: only `0x650` had been read before); (6) 20 renders a kind a setting; (7) every wrong pixel as +1, -1, wrapped or other, its 16-row band, the box and the first three; (9) the underrun flag and `DSIG` around every render (`DSIG` watched only where it reads live with the label up: the emulator doesn't model it). **The model is not a proven truth for every kind:** its Gouraud and modulate rounding are listed as assumed (`model.rs`), so a reference unequal to it is reported, not stopped on; only an unrepeatable reference stops the run. Emulator: the pinned rp-v0.30.0 refuses (no Windows Mobile clocks); the wm-clocks fork build steps all 24 settings, every kind right with everything held, 8 of 9 references the host model's (the dithered one not: that build has no 3D dither model). Host tests 6 (model table, error classes at their edges, tallies, textures), 7 mutants.*
    - *GC2's first try on the X51v 2026-10-23 (`gpubench-0058`; docs/hw/2700g.md "Run GC2's references"): **stopped at the references, before any clock changed.** All nine repeatable (5 renders, both reads, registers held), but one underrun flag before the first reference's second render (most likely latched before the run: the test's fault, fixed: the flag cleared and logged with the label up, the first render watched, sha 04e7068f); and **8 of 9 references not the host model's** (only the GS scene equal; every Gouraud kind and bilinear differ in every band): rule 1's model check can't hold as written until the model is fitted to device pixels. **For the owner:** run on with references proven by repeatability and the end check (the model mismatch recorded), or first a pixel-dump run to fit the model.*
    - ***Every divider, fully implemented (the owner, 2026-10-23: "it would still be nice to be able to use divide 4, and everything needs to be fully implemented, especially once we get to battery life").*** The 2700G clock driver (`drivers/mbx-display` `clock`) and ADR-0035 support all four of `MBXCLK`'s dividers (1-4, measured: GC2, GC2H, GC2R) and `M24CLK`'s with them, each a proven setting on the ladder: the fast steps for drawing, divide 3 and 4 (and the system clock lowered, as Windows Mobile does with `SYSCLKSRC` on the display PLL while idle: `rp-wmprobe`) for battery, chosen by the power policy (phase 9: P10, V3, V5, C1, C2). The per-device self-test proves every step it may switch to, the slow ones included, and the ratchet times the switch itself.
    - ***The overclock's idea ledger (the owner, 2026-10-23: "exhaust every idea... read all the manuals, attempt to change the voltage, test the clock signals").*** Each idea, its status and the run or reason:
      - **(a) the fine map:** done (GC2). **(b) the errors' size:** done (GC2: RGB565 mostly +1; ARGB8888 a few steps). **(c) colours capped:** no help (GC2). **(g) a second PLL setting:** no effect (GC2). **(h) the memory apart from the cores:** the memory right at 133.25 MHz (GC2).
      - **The clock's path (the owner's question):** **divide 1 fails at every speed, 58.5 MHz included (GC2D)**, where divide 2 at the same speed is right: the cores' speed comes only through divide 2 or more.
      - **(d) Gouraud as a texture:** fails at divide 1 too, and isn't Gouraud's picture even at Windows Mobile's clock (GC2D). **(e) the divider per pass:** moot for interpolated drawing (divide 1 fails it at any speed); near-1:1 textures alone survive divide 1 (GC), too narrow for a game.
      - **(f) Dell's OpenGL ES render table: ruled out (GC2F, `gpubench-0061`):** no word, alone or together, changes the error rate at divide 1 or at 133.25 / 2; `0x61C` = 7 changes Gouraud's picture itself (a rounding mode, likely), for the GLES layer to know.
      - **Past 133 MHz through divide 2: done (GC2H, `gpubench-0062`):** the memory right at 143.00 and 149.50 MHz; **at 149.50 / 2 (the cores at 74.75 MHz, Intel's rated 75) every RGB565 kind right, 1.40-1.50x Windows Mobile's speed**; ARGB8888 Gouraud a few words wrong. The setting for a 16-bit game, for the owner to rule.
      - **Further through divide 2: the ceiling found (GC2H2, `gpubench-0065`):** the local memory fails from 156.00 MHz (every kind wrong even at divide 4; hangs at 162.50 and 169.00); **149.50 / 2 is the fastest right setting.**
      - **The owner's brute force for divide 1 (2026-10-23):** (1) **done (GC2R, `gpubench-0063`): no hidden register follows the divider** (6 of 161 differ, each the clock's own); (2) **moot** (nothing to walk back); (4) **done: five ways in, all wrong alike**; (5) **none public** (Intel's datasheet lists divide 1 for GFXCLK; no 2700G errata found); (3) **the full sweep: done (GC2X, `gpubench-0064`): 426 flips, none hung; only `0x644` bit 0 and bit 1 (the write-out downsample) drew right at divide 1.** **Follow-up GC2W (`gpubench-0066`): the downsample only hides a share of the errors** (ARGB8888 Gouraud wrong in 20 of 20 renders at 99.67 and 133.25 / 1 under every downsample): **divide 1 not fixed by any register bit under our renderer; 149.50 / 2 is the overclock today.** **Reopened (the owner, 2026-10-23): Intel's datasheet lists divide 1 as valid to 75 MHz, and Dell's driver picks divide 1 for a `CoreClock` at or under 75 MHz (`HKLM\Drivers\Display\PowerVR\CoreClock`, in Hz): the first test is Dell's own driver at divide 1 (the Windows Mobile probe tool's first module, `rp-wmprobe`); the brute force covered registers under our renderer, not our objects' own words or Dell's sequences** (GC2W's comparison of unwritten memory under `0x644`: quality audit C68). Note: divide 2 already reaches Intel's rated 75 MHz (GC2H); divide 1 would matter only past it.
      - **(i) the 2700G memory's timing loosened:** needed only if GC2H finds the memory failing past 133 (the owner's confirmation before any run).
      - **Voltage: no control exists (declined, the reason):** Intel's datasheet (2700G, April 2004): VCC_CORE 1.104-1.296 V from an external supply, no register for it; Dell's code sets only the PXA's core voltage (GPIO3/4/38), already at its top pattern at 624 MHz; Linux mbxfb and Axdroid touch none. Raising it would mean opening the device (the owner's rule: never).
      - **Drive strength (`SYSCFG` bits 3-2, Linux's `0xFFFFFF0C`): declined:** the datasheet gives them to the system bus's data and interrupt pins, not the cores.
      - **The system clock from the display PLL (`SYSCLKSRC` 1): declined:** it would tie the cores to the pixel clock's PLL, no faster source.
      - **What the target format offers:** at 133.25 / 2 every RGB565 kind is right (GC2): a game rendering to the shell's 16-bit screen (ADR-0023) gets 1.34x with no wrong pixel; ARGB8888 Gouraud alone has the few words.
    - ***The owner's ruling (2026-10-23): option 1.*** GC2 runs against the device's own references: the five equal renders and the two reads are the proof; the model mismatch is its own finding, and rule 1's model check stays open until the model is fitted to the device's pixels.*
    - *Run GC2 on the X51v 2026-10-23 (`gpubench-0059`, docs/hw/2700g.md "Run GC2"): **every rule held** (references the same start and end, reads settled, registers and display held, all 72 controls right, every wrong word still wrong at Windows Mobile's clock: the errors are rendered, not read). **Every kind right at 108.33 / 2, 117.00 / 2, 133.25 / 3 and 133.25 / 4; Gouraud wrong in every render at divide 1 from 74.75 MHz up; ARGB8888 Gouraud a few words wrong at 125.67 / 2 and 133.25 / 2** (RGB565 right there). (h): the memory at 133.25 is right, the limit is the cores' (ARGB8888 Gouraud between 58.5 and 62.8 MHz, RGB565 between 66.6 and 74.75). (g) no effect; (c) no help; (b) RGB565 errors mostly +1, ARGB8888 not. Fastest all-right: 117.00 / 2 (Gouraud and the GS scene 1.17-1.18x Windows Mobile's). **For the owner (results, not decisions):** the ladder from these settings; whether to try (d) Gouraud drawn as a 2 x 2 texture at divide 1 (textures right there) and (e) the divider per pass; (f) our set-up against Dell's; (i) the memory timing (not needed at 133.25: the memory is right).*
    - *Built 2026-10-23 (the owner: "build idea (d)"): `rp-gpubench` run GC2D (`RP_GPUBENCH_GC2_D`; device build `RP_GPUBENCH_GC2_D=1 cargo xtask codegen-variant rp-gpubench gc2d`, sha 57f88174, unattended, about 12 minutes), GC2's rules and code with four kinds: a planar Gouraud quad (its fourth corner the sum of two less the first, so one plane) and the same quad drawn as a bilinear ARGB8888 texture of its corners (side 8, the least: the corners in its top left 2 x 2, coordinates from texel 0's centre to texel 1's), on RGB565 and ARGB8888; the cores at divide 1 at 58.50 and 66.63 MHz (speeds GC2 drew right through divide 2: wrong here, the divide-1 clock path is at fault, not the speed: the owner's question) and at 74.75, 99.67, 108.33, 117.00 and 133.25 MHz, up then down. Also logged: each texture-drawn reference against its Gouraud twin at Windows Mobile's clock (what (d) costs in pixels: the filter's own rounding, not a fault). Emulator: the pinned release refuses; the wm-clocks build maps every setting right. Host tests 7, mutants 3 more.*
  - **The cold soak's steps (the owner present for the start only; the owner picks the day):** (1) the Axim off and undocked for at least an hour at room temperature (not a fridge or a cold car: condensation); "off" is Windows Mobile's suspend, since the loader's power button only turns the screen off: the SD card out, `cargo xtask reboot-wm` (without the card Windows Mobile stays up, no autostart), the power button, undocked; (2) the card back in, docked and powered on, the loader started by hand (File Explorer, Storage Card, haret.exe: the StartUp shortcut runs only at a boot); (3) the owner says "ready" and the soak starts at once, the chip still cold; (4) the owner watches the screen for its first five minutes (a dark screen is the finding to report; nothing to press); (5) the rest runs unattended. Question for the owner: is room temperature "cold" enough, or should a cooler room be tried once? for the owner's vote
  - **Queued (the owner, 2026-10-23): a Windows Mobile baseline and probe suite.** One program on the card, run by the owner from Windows Mobile (his community WM 6.1 build, not stock: its numbers labelled so), that runs every baseline the gate's lines want from Windows Mobile (the 3D and 2D speeds, the CPU's clocks and memory, the 4 KiB page hop for V7, the display) and the probes of Windows Mobile's own settings (the PXA's and the 2700G's registers as Windows Mobile leaves them, the voltage pins, and Dell's OpenGL ES driver's register writes while a 3D program runs, `tools/mmiotrace`), each into one log on the card. Built from `tools/wmbench`'s programs. After the overclock work.
  - **The give-up audit (the owner, 2026-10-23: "we don't give up until we can rule EVERYTHING out when it comes to performance").** Every conclusion that rests on a failed try, Dell's habit or a guess rather than proof, reopened for its "did we do it wrong?" test:
    1. **Divide 1** (the cores at the system clock): **Dell's own driver draws Gouraud right at divide 1** (`rp-wmprobe` module 1, 2026-10-23: 20 of 20 at 74.75 / 1 identical to divide 2); its state differs from ours in `M24CLK`'s divider (1, ours 2): run GC2M built (sha 1c00f45c); then our objects' own words (Gouraud TSP/ISP bits) against Dell's, which no register sweep touched.
    2. **2D and 3D at once, and the 32-bit screen** (P8, P15, P2X: a render goes wrong only with 3D, 2D and scan-out all busy; the 32-bit screen corrupts 32-bit renders, so ADR-0023 chose 16-bit screens and ADR-0030 makes the engines take turns): **the local memory's arbitration was never set.** Dell's driver reads `LocalMemoryPriority` (`0x99`) and `LocalMemoryTimeout` (10) from the registry and writes them to a register we never identified; Linux writes `DMCTRL` `0xD0303010` where Windows Mobile has `0x10303010` (the display's memory control). If either fixes the contention, 2D and 3D overlap (FT: 9% faster together) and the 32-bit screen comes back. Find the register in Dell's code, then a run with each.
    3. **ARGB8888 Gouraud's few words at 125.67-149.50 / 2** (GC2, GC2H): possibly the same contention (the 16-bit screen scanning out meanwhile), not the cores: GC2 under the arbitration settings of 2, and with scan-out stopped.
    4. **Render-to-texture** (P3, declined "the hardware has none"): only `0x650` and six render-table words were searched; the rest of the MBX block's 0x600-0x6FF and Dell's other jobs' words not.
    5. **The CPU bus to the 2700G** (`MSC1`: 14 clocks a beat gained 7%, 13 read wrong): only RDF was stepped; RDN, RRR and the other combinations not.
    6. **Zeroing memory by cache-line allocate** (hung the X51v once, "not isolated"): isolate it (one row a run, the owner present) before keeping it out.
    7. **The CPU's own overclock (C2, V3, V5: open, never started)**: Intel rates 624 MHz; the stepped test above it with the self-test, on the voltage patterns Dell already uses.
    - **How they're run (the owner, 2026-10-23: "all together in the same hardware test, to save time"):** item 1 in `rp-wmprobe` (Windows Mobile: Dell's driver lives only there); items 2-6 in one loader run, **GC3**, safest first (2, 3, 4, then 5 and 6, which can hang the bus: each step logged before it, the run resumable after the step that hung, the owner nearby for the end); item 7 its own watched run (a too-fast CPU can fail anywhere, so it isn't mixed with the 2700G's). Before GC3: Dell's driver read for where `LocalMemoryPriority` and `LocalMemoryTimeout` are written (no device). Order: `rp-wmprobe` module 1 (the owner), then the probe suite's other modules, then GC3, then item 7.
- [x] 4. P21 (phase 5): the 2D dither add (18.9 ms a full screen) against its alternatives (folded into the background, the 3D core) *(ticked 2026-10-22, the owner's ruling: dither once where static, 3D areas dither free, the 2D dither add only on changed 2D areas; compositor.md C2)*
- [ ] 5. Memory layout (phase 2): C4 and CPU V7 (page sizes, table placement, TLB lock-down; the hop with 4 KiB pages). *Built 2026-10-22, waiting for the device: `rp-probe` with `RP_PROBE_PAGES` (`RP_PROBE_PAGES=1 cargo xtask codegen-variant rp-probe pages`, unattended, ~1 min): the random hop over 8 MiB mapped as 1 MiB sections, 64 KiB and 4 KiB pages (`diag::with_window`), the second-level tables in SDRAM and in the internal SRAM, then the first-level table moved into the SRAM too, each with its D-TLB misses counted and the window checked to be the backing's own frames; `emu-probe` runs it under the loader (a mutant mapping each page one off is caught). TLB lock-down not in it yet.* *Runs 1-3 on the X51v 2026-10-22 (`probe-0068` to `0070`): **C4 ticked by the owner's ruling (ADR-0006 amendment: 4 KiB pages, the first-level table and the busiest second-level tables in the SRAM within 32 KiB, three hot kernel pages locked, 64 KiB pages lower priority)**; V7 left: the same hop timed under Windows Mobile.* *C3 and T2 ticked on the owner's shared SRAM layout (2026-10-22, ADR-0006 amendment); building it is in the backlog*
- [ ] 6. The rest of the 2700G's phase 4 and 5 lines: F6 (text: fewer operations), P9 with V5 (the video plane scaled and placed), P8 (= CPU T8: 2D and 3D at once), P4 (the TA's speed: which 3D path), P14 (the 3D core's limits), P15 (scan-out with both engines), P17 (refresh rates), P18 (status bits)
- [ ] 7. Together (phase 5): T1 (a pipeline, not turns), T3 (commands by DMA: the driver's choice), T4 (each unit's job). T1 and T4 are measured on **a minimal compositor test harness built inside the gate, as a benchmark only** (owner's vote, 2026-10-21: option a), not the real compositor; its frames are [workload-brief.md](design/workload-brief.md)'s W1-W9 and three measured-but-not-critical ones, with their targets (corrected and locked by the owner, 2026-10-22)
- [ ] 8. Phases 2, 4 and 5's architectural lines ticked **and the ratchet running** (owner's votes, 2026-10-21 and 2026-10-22): the owner checks the ledgers and lifts the gate. From then on performance is held by the ratchet, every day. Phase 7 (P11, V8: MVED1) and phase 9 (power and clocks for battery: C1, C2, C10, T7, V3, V5, V11, P10, V6 of the 2700G; P16 and CPU V4 declined) gate those phases

---

## Now: know the silicon first (owner, 2026-10-13)


The OS is designed around what the PXA270 and the 2700G do best, not
built for the CPU and retrofitted. Before the UI (Phase 5) and the CE
Personality's drawing (Phase 4) are designed:

- [x] **RE:** the MBX core's command interface, step by step on the device (clear, triangle, textured quad, blend), pulled forward from Phase 7 (`docs/research/imitation-audit.md`, item 69) *(all proven on the device by 2026-10-15: the 2D engine (fills, copies, per-pixel blending) and the 3D core (flat, Gouraud, textured in four formats, blending, depth); ADR-0022 to ADR-0024, docs/hw/2700g.md)*
- [ ] The CPU's unused features measured (performance monitor, cache locking, mini-data cache, SRAM, clocks and power states, memory timings: items 67, 68, 80). Memory timings follow the owner's brief (2026-10-15): find the X51v's SDRAM chips and their datasheet timings, and compare them with Dell's `MDCNFG`/`MDREFR` (probe 2026-10-11). Report only, with alternatives and a risk calibration; tighter timings fail by silently corrupting data, so a stress-test plan comes before any change, and the owner decides. Report done 2026-10-15: docs/research/memory-timing.md. Clocks: the first device run of the clock changes hung (most likely at the memory-clock halving); the next version waits on its branch
- [ ] The compositor/UI and GDI designs written GPU-first, with the CPU as fallback

---

## The alpha (owner, 2026-10-22)

**The alpha is the OS's basic functionality, a proof of concept:** the
device boots RetroPane's own kernel from the card and is usable as an
offline PDA through the shell (Phase 5's exit, at proof-of-concept depth),
with simple Windows Mobile apps running (Phase 4's exit). Bugs are
expected. Not in it: Wi-Fi (Phase 6), flash install, Bluetooth and the
release work (Phase 9). Estimated 2026-10-22 at the pace so far: about 3-4
months (late January to February 2027); with Wi-Fi about 4-6.

## Phase 0 — Foundations
**Goal:** a project anyone can clone, build and test, before any OS code exists.

- [x] Final name chosen (**RetroPane OS**). GitHub repo created: public, GPL-2.0-or-later (`Drakey83/retropane-os`)
- [x] `CONTRIBUTING.md`, `CLEANROOM.md`, code of conduct, issue/PR templates (with the four lenses)
- [x] Cargo workspace skeleton matching DESIGN §4.2, with `xtask` (HAL traits, mock, PXA27x MMIO, X51v memory map)
- [x] Custom Rust target `armv5te-x51v-none-eabi`; pinned toolchain
- [x] CI: fmt, clippy, lint-layers, host tests, cross-build, coverage floor
- [x] ADRs 0001–0004 written: Rust-first, QEMU fork, slot memory model, testing policy. *(The TLS library ADR moves to Phase 6, where it's needed.)*
- [x] ADR-0005 memory and storage architecture *(accepted 2026-10-10)*; code-size budgets enforced in `cargo xtask cross`; whole-device memory budget (`docs/memory-budget.md`: OS at most 12 MiB, apps at least 52 MiB) checked there too since 2026-10-12
- [x] **RE:** `tools/romdump`: unpacks X51v `.nb0` ROM images (IMGFS + LZX) and rebuilds driver DLLs as PE files. No device access needed
- [ ] *(Optional)* UART serial adapter for deep debugging. Not required: the cradle's USB is the main console from Phase 2
- [ ] **RE:** baseline benchmarks of stock Windows Mobile (current WM6.1 ROM) on your device (DoomGLES fps, boot time, memory, game load times), which become the floor to beat (DESIGN.md §1.0) *(CPU, memory, threads, timers, drawing and card speeds measured by `tools/wmbench`, 2026-10-13: docs/perf.md; DoomGLES fps and boot time still to do)*

**Exit:** `cargo xtask test` passes in CI on an empty skeleton; ROM dump in hand.

## Phase 1 — Emulator
**Goal:** an `x51v` machine in a QEMU 9.1 fork that boots a test kernel.

- [x] Fork QEMU 9.1 ([retropane-qemu](https://github.com/Drakey83/retropane-qemu); CI builds Linux + Windows releases, and `cargo xtask emu` downloads them, so no submodule is needed)
- [x] `x51v` machine: PXA270, memory map, UART console *(rp-v0.1.0)*, button matrix *(rp-v0.4.0)*, SD image *(with card detect and the lock switch, rp-v0.9.3)*, CompactFlash slot *(rp-v0.9.7)*, power button and headphone jack *(rp-v0.9.5, rp-v0.9.9)*, DMA *(fixed to serve requests asynchronously and chain descriptors, rp-v0.9.6)*
- [x] CPLD model (from `docs/hw/cpld.md`) *(rp-v0.3.0)*
- [x] TSC2046 touch screen, byte-accurate *(rp-v0.5.0)*
- [x] WM8750 audio *(on the I2C bus and I2S, output recordable for tests, rp-v0.9.8)*
- [x] 2700G display model: local memory, registers, graphics-plane scan-out *(rp-v0.2.0)*
- [x] Screenshot and input scripting for integration tests: `cargo xtask emu` verifies the test pattern pixel by pixel and presses buttons through the QEMU monitor
- [ ] **RE:** `tools/mmiotrace` working on Windows Mobile: traps and logs 2700G register accesses

**Exit:** a "hello world" binary prints to serial and draws a test pattern, in the emulator *and* booted from SD on the real device.
*Emulator half done (2026-10-10): see `docs/img/emulator-boot-log.png`.*

## Phase 2 — Kernel
**Goal:** a real kernel with threads, memory protection and full-speed clocks.

- [x] SD loader (a Windows Mobile app that hands off to our kernel; no flashing) *(HaRET boots RetroPane on the device since 2026-10-11; the resident loader then takes every image over USB, and updates its own files on the card: ADR-0012, ADR-0013)*
- [x] Physical frame allocator (`kernel/mm`, host-tested, benchmarked) *(2026-10-11)*; ADR-0006 (chip-specific memory design) accepted 2026-10-11
- [ ] On-screen debug text *(boot log done 2026-10-11)* and crash screen *(done 2026-10-12 in the loader, emulator-verified)* (using the display inherited from Windows Mobile), plus a crash log that survives reboot *(crash log done 2026-10-12 for images under the resident loader: back to the loader, report to the PC and onto the card as `crashlog.txt`)*
- [x] **USB device driver early:** CDC-ACM console over the stock cradle, with no custom PC drivers (ADR-0012). Working on the device 2026-10-11: the X51v is a driverless COM port on Windows 11, and the resident loader takes images and returns their output over it. Hands-off since the same evening: `cargo xtask load` re-finds the device through a Windows device restart (the pull-up is a fixed resistor)
- [x] Exception vectors (high), bring-up MMU (identity map: cached RAM, write-combined framebuffer, strict devices), caches, fault reporting *(2026-10-10)*
- [ ] FCSE slot model (ADR-0003) and demand-paged VM (ADR-0005), 624 MHz turbo plus frequency scaling *(VM layer done 2026-10-11 and running live in `boot/kernel`: own page tables, real page faults resolved by demand paging and copy-on-write, FCSE slots; ADR-0008 accepted. Kernel-space device map and per-process domain protection also live; reclaim (second-chance clock with emulated reference bits, compress or drop) live 2026-10-12; pinned memory, and region lists in per-process pages with a quota (ADR-0019, up to 1,632 regions per process) 2026-10-12)*
- [x] Interrupt controller, OS timer tick, idle mode (no busy-waiting) *(2026-10-10)*; unowned sources masked and reported *(2026-10-11)*
- [x] Interrupt ownership: drivers handle interrupts in their own thread via `IrqLine`, never in interrupt mode (ADR-0007, accepted and implemented 2026-10-11: mask-and-signal first-level handler, per-pin GPIO lines; hello sleeps until a button or the pen wakes it)
- [ ] Scheduler (256 priorities, priority inheritance) *(ADR-0010 accepted; scheduler core, context switch and tickless alarm done and live in `boot/threads` 2026-10-11, with user mode and SWI system calls, then preemption, user faults in the thread's context, lazy iWMMXt, interrupt threads and the KData page; all 13 tests pass on the real X51v 2026-10-12; the scheduler in the kernel image 2026-10-12, with threads preempted inside IPC calls (emulator); next: the ~12 µs device switch time vs the 2 µs target)*
- [ ] Processes, threads, sync objects, IPC, driver host process *(IPC 2026-10-12: rules in `kernel/ipc`, migrating calls between two user processes live in the emulator, ADR-0016 steps 1-2; a service manager handing out handles and a supervised host restarted after a crash, step 3 begun; processes started and ended at run time; lost-preemption bug fixed)*
- [ ] Compressed RAM and storage codec (ADR-0005 rules 4, 6) *(LZ4 block codec done 2026-10-11: safe decoder, page compressor, on-target benchmarks; compressed page store `kernel/mm::zpool`: 200 mixed pages in 39% of the RAM; wired into the VM and live in `rp-kernel`, handles stored in page-table entries)*
- [ ] PE loader (host-tested with synthetic PE files) *(format layer done 2026-10-11: `kernel/pe` parses, fills image-view pages, relocates per page and lists imports; verified on 12 real WM6 DLLs; ADR-0009 accepted; live in the kernel image 2026-10-12: two processes run from one PE image loaded page by page on demand, relocated, shared, W^X)*
- [x] Performance suite v1: context switch, IRQ latency, memcpy bandwidth *(2026-10-12: device numbers via `cargo xtask load rp-threads` and the probe; docs/perf.md)*

**Exit:** multiple processes running, with protected memory, on device and emulator; perf baseline recorded.

## Phase 3 — Core drivers
**Goal:** every "easy" (documented) device working natively.

- [x] Display (pxafb passthrough first, then native 2700G planes) *(the 2700G's own planes on the device since 2026-10-14: the Today screen is one picture composed by the 2D engine, double-buffered (ADR-0023); night-light gamma table found 2026-10-16)*
- [x] Touch (TSC2046 via SSP1 + GPIO), keypad buttons: drivers + emulator tests *(calibration UI still to do)*
- [ ] Backlight (PWM), power button *(both done 2026-10-12 in the emulator: `rp-drv-pxa-pwm`, `cargo xtask backlight`, a press fades the screen off/on. On the device 2026-10-16: the backlight taken over from Windows Mobile and dimmed to 1/432 (`rp-handover`, `rp-nightlight`); power button device run to do)*
- [ ] SD/SDIO with DMA and 4-bit mode: **SD, SDHC and SDXC** cards (beyond stock WM's 2 GB SD limit), plus a FAT32/exFAT file system *(polled, 4-bit, multi-block reads and writes with recovery for slow and failing blocks, hot swap and the lock switch; FAT32 read/create/replace (crash-safe), exFAT read and write (crash-safe, folders); the card as a USB drive (ADR-0014); folders; DMA for multi-block transfers, self-tested at start-up; all 2026-10-11/12. **On the device 2026-10-16** (`rp-storfast`): reads 4.2x faster by DMA (7.9 MB/s), writes 3.8x, a 430 MB soak with 0 errors, a card swap mid-soak; DMA writes on in the loader. To do: SDIO, the lock-switch step)*
- [ ] CompactFlash (PXA PC Card socket 1, DMA where possible) *(spec `docs/hw/cf.md` and the probe's socket readings 2026-10-12; driver `rp-drv-cf` (memory mode, ATA, write cache off) and the loader's `cf` command 2026-10-12 in the emulator; the card as the USB drive's second unit; **on the device 2026-10-16/17:** the card attaches (warm, 25 ms), reads by DMA, and shows in Explorer as the PC's second drive; to do on the device: the slot powering down when the card is pulled)*
- [ ] Audio (WM8750, I²S, DMA) *(**plays on the device since 2026-10-18** (docs/hw/audio.md): the codec found by its reset write over I2C (the X51v's controller can't do address-only transfers), its power (CPLD `0x1C` bit 1) and I2C gate (GPIO82), I2S by DMA at 48 kHz with I2S kept on through a session, the jack on GPIO83 and the speaker amplifier (CPLD `0x14` bit 1): three notes to the headphones and three to the speaker. **The audio service (ADR-0032, accepted 2026-10-18 with the owner's thirteen conditions): step S1 built 2026-10-18** (`services/audio`: 44.1 kHz on both sides, the power order, the 700 ms I2S restart guard, the jack by its edge latch; `rp-audio-s1` and `emu-audio-s1`; 52 of 52 mutants killed); **S1 PASSED (owner, 2026-10-19)** after runs 1-3, the A/B runs, Windows Mobile's own playback (`rp-wmtones`) and the speaker's measured response: the jack and power order work (VMID fast start), headphones play every note, the I2S guard works; the speaker's weak low notes are a hardware matter (the same under Windows Mobile; the owner's X50v will be compared). `rp-wmpower` measured 2026-10-19 (~10-35 mA for an idle powered codec); **the owner chose R5 option A**: the codec sleeps after 5 s of quiet with I2S kept running, woken early by a touch or key. **S1b (the codec's sleep) passed by the owner 2026-10-19** on the device (five sleeps with I2S running, wakes in 152 ms, the sound at the lift). **S2 (the mixer) built 2026-10-19** (roles, byte rings sized by the rule, the long ring rewritten ahead with the mix a function of output time, the 15 / 200 ms wake with a low-water mark, the limiter and dither, the one-table 44.1 kHz resampler; 63 host tests, 19 mutants killed, `emu-audio-s2` passes; **passed by the owner on the device 2026-10-19**). Then S3 (volumes and routing), S4 (the CE `waveOut` calls), S5 (recording), S6 (Bluetooth outputs). Still to do besides: interrupt-driven streaming, pop-free power sequences judged by ear; emulator: the amplifier's effect)*
- [x] USB client (serial gadget for development networking) *(CDC-ACM serial, and since 2026-10-12 a composite device with USB mass storage: ADR-0012, ADR-0014)*
- [x] **Hardware datasheet** *(2026-10-18: `docs/hw/datasheet.md`, the board on one page with a GPIO pinout, the CPLD's bits and drawings in `docs/hw/img/` (redrawn by `tools/hwmaps`); kept current with every hardware finding, same commit: docs/CONTRIBUTOR-BRIEF.md)*
- [ ] **RE:** full CPLD register map in `docs/hw/cpld.md` *(v0.1 from AxDroid facts; confirm and complete from Dell's OAL in `nk.exe`. Windows Mobile's register table read on the device 2026-10-16 and adopted at hand-over (`Cpld::adopt`); card-detect bit (0x04 bit 1) proven)*

**Exit:** touch, keys, SD, CF and audio pass hardware-in-the-loop tests.

## Phase 4 — CE Personality v1
**Goal:** simple unmodified WM5/6.x apps run.

- [ ] Compatibility corpus defined (the games catalogue, `docs/research/3d-games-catalogue.md`: DESIGN.md §1.0); import-table scanner produces an API backlog ranked by what the catalogue needs first *(scanner done 2026-10-12: `tools/apiscan` -> `docs/research/api-backlog.md` from the owner's ROM; today's ranking is the ROM's own 519 modules, no games: the catalogue's games still to add)*
  - [ ] Scan the catalogue's binaries on the card (they stay local; only interface facts are written: docs/CLEANROOM.md) and rank catalogue-first, as a games-ranked view beside the ROM's: `apiscan` needs a weighting or second-report option
- [ ] coredll (core subset), aygshell basics, GDI on the software backend
- [ ] GAPI (`gx.dll`)
- [ ] Registry and file-system semantics matching CE
- [ ] CAB installer (Windows Mobile's `wceload`): cabinet unpack, `_setup.xml` and `000` setup scripts, registry keys, shortcuts, setup DLLs in the sandbox; installs from the card first (ADR-0028 L2; the parsers exist: `rp-cab`, `rp-ce-setup`)

**Exit:** a set of simple WM5/6.x apps (calculator, a GAPI game, text editor) run unmodified.

## Phase 5 — Shell and UI
**Goal:** the "modern take on Windows Mobile" experience.

All design decisions (UI, apps, themes, weather, packs, game ports) are
gathered in [design/README.md](design/README.md).

- [ ] UI design spec and mock-ups approved (`docs/design/`) *before* code
- [ ] GWES window manager, compositor, theme engine (Modern + Classic)
- [ ] Screen calibration in Settings (owner, 2026-10-17): every X51v's panel has aged differently, so each owner dials in their own screen (black end, white end, mid-tones, warmth, backlight) with the patterns of `rp-display-cal` (docs/design/display-calibration.md); first as the dials, later a guided or automated tool. The result is the system's gamma table and backlight range. Planned shape (owner asked for automatic, 2026-10-17): the X51v can't see its own screen (no camera, no sensor facing the panel), so fully automatic needs a measuring device. Three layers: (1) defaults measured on the owner's X51v ship for everyone; (2) a one-minute guided step at install, one tap per question ("tap the faintest square you can see", "tap the box that blends in"), the curve computed from the taps, no dials; (3) optional fully automatic calibration with a PC colorimeter held on the screen, driven over the LegacySync USB link
- [ ] Today screen, Start menu, task switcher, notifications, SIP keyboard *(Today screen in the test image `rp-today` on the device: mock-up layout and VClouds' weather icons (T1, T2, 2026-10-17), Start menu, softkeys, D-pad focus; touch-to-screen median 19 ms. Next: wallpaper and settings from the card (T3, T4), which wait for the OS storage service)*
- [ ] Settings, file manager, notes

**Exit:** usable day to day as an offline PDA.

## Phase 6 — Networking
**Goal:** the built-in Wi-Fi on modern networks.

- [ ] **RE:** ACX100 trace analysis written up as `docs/hw/acx100.md`
- [ ] Native ACX100 driver; firmware extracted by `romdump`
- [ ] 802.11 MAC, scanning, WPA2-PSK in software
- [ ] WPA3-Personal in software: SAE (H2E, constant-time) and PMF; first, RE whether the firmware allows host-built management frames
- [ ] TCP/IP (IPv4/IPv6), DHCP, DNS, mDNS; TLS 1.3
- [ ] Winsock in the CE Personality

**Exit:** the device joins a WPA2 network and a WPA3-only network and does HTTPS on the real hardware.

## Phase 7 — 2700G acceleration (3D, 2D, video)
**Goal:** all 2700G blocks running on native drivers, at full speed.
*(RE starts in Phase 0; this phase is where the drivers land.)*

- [ ] 2D blitter driver; GDI and compositor accelerated *(2D renderer `rp-mbx-render` drawing the Today screen on the device since 2026-10-14; GDI to do)*
- [ ] MBX 3D: init → clear → triangle → textured → GLES 1.1 CL subset *(init, clear, triangle, textured, blending and depth proven on the device; our own renderer (ADR-0026) built and timed on the device; the GLES subset to do)*
- [ ] `libGLES_CL.dll` + EGL in the CE Personality on MBX
- [ ] Compositor moved to MBX at 60 fps
- [ ] MVED1 video decoder driver; media player
- [ ] Emulator models for all of the above (from the specs)

**Exit:** **DoomGLES runs unmodified, at least as fast as on stock Windows Mobile.**

## Phase 8 — Modern web and apps
- [ ] NetSurf port with TLS 1.3
- [ ] Bridge service (self-hostable) and Bridge mode in the browser
- [ ] Messaging (Matrix/XMPP/email), music player, modern media formats
- [ ] Broader CE compatibility (commctrl, DirectDraw, D3DM) as the catalogue needs it; beyond the catalogue, product by product as users report problems

## Phase 9 — Power, Bluetooth, flash install and release
- [ ] **RE:** power/battery IC; suspend/resume; battery gauge
- [ ] Bluetooth (BTUART) with a modern host stack *(owner 2026-10-18: keyboards and mice (HID) and headsets (A2DP stereo, HFP/HSP calls with the mic, AVRCP buttons): docs/design/peripherals.md; design ADR-0033 accepted 2026-10-18 (P1-P5 all A; routing and profile allowlist before P-S1); the radio is a TI BRF6150, Bluetooth 1.2, docs/hw/bluetooth.md)*
- [ ] USB host: keyboards and mice (HID) on the PXA27x host controller *(owner 2026-10-18; design ADR-0033 (R4: the role switch only when the PC link is idle or confirmed); Windows Mobile ships no USB host drivers on the X51v and VBUS on the connector is still to check (step P-S0): docs/hw/bluetooth.md)*
- [ ] Flash install via Dell's bootloader mode (reset button). The Dell bootloader region is **never written** (enforced by tests); recovery means reflashing a stock or LennySh ROM. Full ROM backup first
- [ ] Lock down the development loader before any public build (ADR-0017: pairing confirmed on screen, authenticated frames; or the loader left out of release images)
- [ ] Installer; v1.0 public release with SD and flash images
- [ ] Optional packs (ADR-0018, accepted): complete base system in flash; signed, verified packs (browser, codecs, fonts, language packs) on the card or in flash; clean card removal

## LegacySync: the Windows companion (ADR-0028, accepted 2026-10-15)
**Idea recorded 2026-10-15 (owner).** A Windows 11 program that does ActiveSync's jobs for RetroPane over the Dell cable: old Pocket PC `setup.exe` installers work again (a `CeAppMgr.exe` stand-in sends the ARM CAB to the device), then a `rapi.dll` for old PC tools, time sync, file browsing, encrypted backups, contacts/calendar/tasks sync (Outlook and CardDAV/CalDAV), and internet pass-through (off by default). Our own protocol over the USB link, not Microsoft's; partnerships with per-feature permissions. Needs Phase 4 (programs run, CABs install) first.
- [x] Research: ActiveSync's PC-side interfaces, from the owner's discs and PC (docs/legacysync/activesync-interfaces.md)
- [x] Shared CAB and setup-script parsers, device and PC (`rp-cab`, `rp-ce-setup`)
- [x] `ceappmgr.exe` stand-in: checks, chooses and stages the CAB with its plan (registration: option A chosen, backup/take-over/check built, not yet applied; docs/legacysync/ceappmgr-standin.md)
- [x] Device link session design, accepted 2026-10-15 (docs/legacysync/device-link.md: own WinUSB interface, Noise XX pairing with a 6-digit code then KK, signed partnership list in the built-in flash, the card as the SD-stage stand-in)
- [ ] **Device random number source** (docs/security.md principle 4): gates every key the device makes, so the link's device side waits for it. *(ADR-0029 and `rp-noise`'s entropy measurements merged into main 2026-10-18, `ac6f912`; still open: the restart test's healthy-row work and the generator itself)*
- [ ] X25519, ChaCha20-Poly1305 and list signing in `rp-crypto`; the transport-agnostic frame codec and Noise handshakes (no RNG needed)
- [ ] Sending to the device and installing (needs Phase 4's CAB installer)
- [ ] RAPI (L3), time, Explore, backup; then sync and pass-through

## After everything else: the Field Kit pack
**Idea recorded, specifics later** (ADR-0020, owner's initial ideas, 2026-10-13). The last thing built, after the browser and the rest of the roadmap: a pack of client-only network and utility tools (terminal, hex editor, password manager, encryption and hashes, serial console, crash log viewer; then SSH, IRC, network diagnostics, Wi-Fi scanner), each a separate program run one at a time from the card.

## At the end: a faster Quake 3 port
**Owner's request, 2026-10-14.** The Quake 3 port on the owner's card (Q3CE,
2008) "takes forever to start". A fresh port of the GPL engine (id's 2005
release or ioquake3; GPL-2.0, compatible with RetroPane) for the X51v,
drawn through RetroPane's 3D core work, with start-up time as a goal from
day one: Q3CE's start under Windows Mobile measured first as the baseline.
After the OS work; likely its own repository. Game data stays the owner's.

## Maybe, long after everything else: a modern screen
**Parked by the owner, 2026-10-17: a slight "maybe", only once everything above
is done.** Desk research on swapping the X51v's 3.7" Sharp panel for a cheap
modern one through our own flex adapter, and whether it could be a kit:
[research/screen-swap.md](research/screen-swap.md). Nothing planned; any
prototype would be on the spare X50v, never the owner's X51v.

---

## Tracking

Each checkbox becomes a GitHub issue under a milestone per phase. The RE track
uses the `re:` label, and every RE finding is merged as a spec in `docs/hw/`.
