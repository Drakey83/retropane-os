# Roadmap

> Status: living document, updated with each change. Phases overlap. The **reverse-engineering (RE) track runs
> in parallel from Phase 0**, because it's the critical path for the hardware
> requirements (3D, 2D, Wi-Fi).
> No dates yet. Each phase ends with a demo on the **real X51v**, not just the emulator.

```
Phase:   0     1     2     3     4     5     6     7     8     9
Build:  [fnd][emu][krn][drv][ce ][ui ][net][gfx][web][pwr/flash]
RE:     [rom dump · tracer · CPLD · 2700G 2D/3D/video · ACX100 · power IC ───────────────]
```

---

## Now: know the silicon first (owner, 2026-10-13)
The OS is designed around what the PXA270 and the 2700G do best, not
built for the CPU and retrofitted. Before the UI (Phase 5) and the CE
Personality's drawing (Phase 4) are designed:

- [ ] **RE:** the MBX core's command interface, step by step on the device (clear, triangle, textured quad, blend), pulled forward from Phase 7 (`docs/research/imitation-audit.md`, item 69)
- [ ] The CPU's unused features measured (performance monitor, cache locking, mini-data cache, SRAM, clocks and power states, memory timings: items 67, 68, 80)
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
- [ ] **RE:** baseline benchmarks of stock Windows Mobile (current WM6.1 ROM) on your device (DoomGLES fps, boot time, memory), which become the "at least as fast" target *(CPU, memory, threads, timers, drawing and card speeds measured by `tools/wmbench`, 2026-10-13: docs/perf.md; DoomGLES fps and boot time still to do)*

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

- [ ] Display (pxafb passthrough first, then native 2700G planes)
- [x] Touch (TSC2046 via SSP1 + GPIO), keypad buttons: drivers + emulator tests *(calibration UI still to do)*
- [ ] Backlight (PWM), power button *(both done 2026-10-12 in the emulator: `rp-drv-pxa-pwm`, `cargo xtask backlight`, a press fades the screen off/on; device runs to do)*
- [ ] SD/SDIO with DMA and 4-bit mode: **SD, SDHC and SDXC** cards (beyond stock WM's 2 GB SD limit), plus a FAT32/exFAT file system *(polled, 4-bit, multi-block reads and writes with recovery for slow and failing blocks, hot swap and the lock switch; FAT32 read/create/replace (crash-safe), exFAT read and write (crash-safe, folders); the card as a USB drive (ADR-0014); folders; DMA for multi-block transfers, self-tested at start-up (emulator; device run pending); all 2026-10-11/12. To do: SDIO)*
- [ ] CompactFlash (PXA PC Card socket 1, DMA where possible) *(spec `docs/hw/cf.md` and the probe's socket readings 2026-10-12; driver `rp-drv-cf` (memory mode, ATA, write cache off) and the loader's `cf` command 2026-10-12 in the emulator; the card as the USB drive's second unit; to do: device run, card power)*
- [ ] Audio (WM8750, I²S, DMA) *(I2C, codec and I2S drivers and the loader's `tone` 2026-10-12 in the emulator, the output recorded and its pitch measured in CI; DMA streaming the same day (CPU under 1% busy); to do: amplifier, headphone detect, device run)*
- [x] USB client (serial gadget for development networking) *(CDC-ACM serial, and since 2026-10-12 a composite device with USB mass storage: ADR-0012, ADR-0014)*
- [ ] **RE:** full CPLD register map in `docs/hw/cpld.md` *(v0.1 from AxDroid facts; confirm and complete from Dell's OAL in `nk.exe`)*

**Exit:** touch, keys, SD, CF and audio pass hardware-in-the-loop tests.

## Phase 4 — CE Personality v1
**Goal:** simple unmodified WM5/6.x apps run.

- [ ] Compatibility corpus defined; import-table scanner produces a ranked API backlog *(scanner done 2026-10-12: `tools/apiscan` -> `docs/research/api-backlog.md` from the owner's ROM; third-party apps still to add to the corpus)*
- [ ] coredll (core subset), aygshell basics, GDI on the software backend
- [ ] GAPI (`gx.dll`)
- [ ] Registry and file-system semantics matching CE

**Exit:** a set of simple WM5/6.x apps (calculator, a GAPI game, text editor) run unmodified.

## Phase 5 — Shell and UI
**Goal:** the "modern take on Windows Mobile" experience.

- [ ] UI design spec and mock-ups approved (`docs/ui/`) *before* code
- [ ] GWES window manager, compositor, theme engine (Modern + Classic)
- [ ] Today screen, Start menu, task switcher, notifications, SIP keyboard
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

- [ ] 2D blitter driver; GDI and compositor accelerated
- [ ] MBX 3D: init → clear → triangle → textured → GLES 1.1 CL subset
- [ ] `libGLES_CL.dll` + EGL in the CE Personality on MBX
- [ ] Compositor moved to MBX at 60 fps
- [ ] MVED1 video decoder driver; media player
- [ ] Emulator models for all of the above (from the specs)

**Exit:** **DoomGLES runs unmodified, at least as fast as on stock Windows Mobile.**

## Phase 8 — Modern web and apps
- [ ] NetSurf port with TLS 1.3
- [ ] Bridge service (self-hostable) and Bridge mode in the browser
- [ ] Messaging (Matrix/XMPP/email), music player, modern media formats
- [ ] Broader CE compatibility (commctrl, DirectDraw, D3DM) driven by the corpus

## Phase 9 — Power, Bluetooth, flash install and release
- [ ] **RE:** power/battery IC; suspend/resume; battery gauge
- [ ] Bluetooth (BTUART) with a modern host stack
- [ ] Flash install via Dell's bootloader mode (reset button). The Dell bootloader region is **never written** (enforced by tests); recovery means reflashing a stock or LennySh ROM. Full ROM backup first
- [ ] Lock down the development loader before any public build (ADR-0017: pairing confirmed on screen, authenticated frames; or the loader left out of release images)
- [ ] Installer; v1.0 public release with SD and flash images
- [ ] Optional packs (ADR-0018, accepted): complete base system in flash; signed, verified packs (browser, codecs, fonts, language packs) on the card or in flash; clean card removal

## After everything else: the Field Kit pack
**Idea recorded, specifics later** (ADR-0020, owner's initial ideas, 2026-10-13). The last thing built, after the browser and the rest of the roadmap: a pack of client-only network and utility tools (terminal, hex editor, password manager, encryption and hashes, serial console, crash log viewer; then SSH, IRC, network diagnostics, Wi-Fi scanner), each a separate program run one at a time from the card.

---

## Tracking

Each checkbox becomes a GitHub issue under a milestone per phase. The RE track
uses the `re:` label, and every RE finding is merged as a spec in `docs/hw/`.
