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

**Count (`cargo xtask gate`, 2026-10-22): phases 2, 4, 5: 8 of 30
architectural lines ticked (17 partial, 5 open)**; phase 7: 0 of 2; phase 9:
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
- [ ] 3. The 2700G's clocks (phase 5: P1, P2, V1, V2, V12, V13, V14): soaks warm and cold at the cores' divide 1 (run G4: 3D 1.81x, 2D 1.89x, short checks on one unit), the per-device self-test, an ADR for the owner's vote
- [ ] 4. P21 (phase 5): the 2D dither add (18.9 ms a full screen) against its alternatives (folded into the background, the 3D core)
- [ ] 5. Memory layout (phase 2): C4 and CPU V7 (page sizes, table placement, TLB lock-down; the hop with 4 KiB pages). *C3 and T2 ticked on the owner's shared SRAM layout (2026-10-22, ADR-0006 amendment); building it is in the backlog*
- [ ] 6. The rest of the 2700G's phase 4 and 5 lines: F6 (text: fewer operations), P9 with V5 (the video plane scaled and placed), P8 (= CPU T8: 2D and 3D at once), P4 (the TA's speed: which 3D path), P14 (the 3D core's limits), P15 (scan-out with both engines), P17 (refresh rates), P18 (status bits)
- [ ] 7. Together (phase 5): T1 (a pipeline, not turns), T3 (commands by DMA: the driver's choice), T4 (each unit's job). T1 and T4 are measured on **a minimal compositor test harness built inside the gate, as a benchmark only** (owner's vote, 2026-10-21: option a), not the real compositor
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
