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

## Phase 0 — Foundations
**Goal:** a project anyone can clone, build and test, before any OS code exists.

- [x] Final name chosen (**RetroPane OS**). GitHub repo created: public, GPL-2.0-or-later (`Drakey83/retropane-os`)
- [x] `CONTRIBUTING.md`, `CLEANROOM.md`, code of conduct, issue/PR templates (with the four lenses)
- [x] Cargo workspace skeleton matching DESIGN §4.2, with `xtask` (HAL traits, mock, PXA27x MMIO, X51v memory map)
- [x] Custom Rust target `armv5te-x51v-none-eabi`; pinned toolchain
- [x] CI: fmt, clippy, lint-layers, host tests, cross-build, coverage floor
- [x] ADRs 0001–0004 written: Rust-first, QEMU fork, slot memory model, testing policy. *(The TLS library ADR moves to Phase 6, where it's needed.)*
- [x] ADR-0005 memory and storage architecture *(accepted 2026-10-10)*; code-size budgets enforced in `cargo xtask cross`
- [x] **RE:** `tools/romdump`: unpacks X51v `.nb0` ROM images (IMGFS + LZX) and rebuilds driver DLLs as PE files. No device access needed
- [ ] *(Optional)* UART serial adapter for deep debugging. Not required: the cradle's USB is the main console from Phase 2
- [ ] **RE:** baseline benchmarks of stock Windows Mobile (current WM6.1 ROM) on your device (DoomGLES fps, boot time, memory), which become the "at least as fast" target

**Exit:** `cargo xtask test` passes in CI on an empty skeleton; ROM dump in hand.

## Phase 1 — Emulator
**Goal:** an `x51v` machine in a QEMU 9.1 fork that boots a test kernel.

- [x] Fork QEMU 9.1 ([retropane-qemu](https://github.com/Drakey83/retropane-qemu); CI builds Linux + Windows releases, and `cargo xtask emu` downloads them, so no submodule is needed)
- [ ] `x51v` machine: PXA270, memory map, UART console *(rp-v0.1.0)*, button matrix *(rp-v0.4.0)*, SD image
- [x] CPLD model (from `docs/hw/cpld.md`) *(rp-v0.3.0)*
- [x] TSC2046 touch screen, byte-accurate *(rp-v0.5.0)*
- [ ] WM8750 audio
- [x] 2700G display model: local memory, registers, graphics-plane scan-out *(rp-v0.2.0)*
- [x] Screenshot and input scripting for integration tests: `cargo xtask emu` verifies the test pattern pixel by pixel and presses buttons through the QEMU monitor
- [ ] **RE:** `tools/mmiotrace` working on Windows Mobile: traps and logs 2700G register accesses

**Exit:** a "hello world" binary prints to serial and draws a test pattern, in the emulator *and* booted from SD on the real device.
*Emulator half done (2026-10-10): see `docs/img/emulator-boot-log.png`.*

## Phase 2 — Kernel
**Goal:** a real kernel with threads, memory protection and full-speed clocks.

- [ ] SD loader (a Windows Mobile app that hands off to our kernel; no flashing) *(HaRET kit ready: `cargo xtask sdkit` boots the on-screen hardware probe; awaiting first run on the device)*
- [x] Physical frame allocator (`kernel/mm`, host-tested, benchmarked) *(2026-10-11)*; ADR-0006 (chip-specific memory design) accepted 2026-10-11
- [ ] On-screen debug text *(boot log done 2026-10-11)* and crash screen (using the display inherited from Windows Mobile), plus a crash log that survives reboot *(crash log done 2026-10-12 for images under the resident loader: back to the loader, report to the PC and onto the card as `crashlog.txt`)*
- [x] **USB device driver early:** CDC-ACM console over the stock cradle, with no custom PC drivers (ADR-0012). Working on the device 2026-10-11: the X51v is a driverless COM port on Windows 11, and the resident loader takes images and returns their output over it. Hands-off since the same evening: `cargo xtask load` re-finds the device through a Windows device restart (the pull-up is a fixed resistor)
- [x] Exception vectors (high), bring-up MMU (identity map: cached RAM, write-combined framebuffer, strict devices), caches, fault reporting *(2026-10-10)*
- [ ] FCSE slot model (ADR-0003) and demand-paged VM (ADR-0005), 624 MHz turbo plus frequency scaling *(VM layer done 2026-10-11 and running live in `boot/kernel`: own page tables, real page faults resolved by demand paging and copy-on-write, FCSE slots; ADR-0008 accepted. Kernel-space device map and per-process domain protection also live; next: reclaim)*
- [x] Interrupt controller, OS timer tick, idle mode (no busy-waiting) *(2026-10-10)*; unowned sources masked and reported *(2026-10-11)*
- [x] Interrupt ownership: drivers handle interrupts in their own thread via `IrqLine`, never in interrupt mode (ADR-0007, accepted and implemented 2026-10-11: mask-and-signal first-level handler, per-pin GPIO lines; hello sleeps until a button or the pen wakes it)
- [ ] Scheduler (256 priorities, priority inheritance) *(ADR-0010 accepted; scheduler core, context switch and tickless alarm done and live in `boot/threads` 2026-10-11, with user mode and SWI system calls, then preemption, user faults in the thread's context, lazy iWMMXt, interrupt threads and the KData page; next: the scheduler in the kernel image)*
- [ ] Processes, threads, sync objects, IPC, driver host process
- [ ] Compressed RAM and storage codec (ADR-0005 rules 4, 6) *(LZ4 block codec done 2026-10-11: safe decoder, page compressor, on-target benchmarks; compressed page store `kernel/mm::zpool`: 200 mixed pages in 39% of the RAM; wired into the VM and live in `rp-kernel`, handles stored in page-table entries)*
- [ ] PE loader (host-tested with synthetic PE files) *(format layer done 2026-10-11: `kernel/pe` parses, fills image-view pages, relocates per page and lists imports; verified on 12 real WM6 DLLs; ADR-0009 accepted)*
- [ ] Performance suite v1: context switch, IRQ latency, memcpy bandwidth

**Exit:** multiple processes running, with protected memory, on device and emulator; perf baseline recorded.

## Phase 3 — Core drivers
**Goal:** every "easy" (documented) device working natively.

- [ ] Display (pxafb passthrough first, then native 2700G planes)
- [x] Touch (TSC2046 via SSP1 + GPIO), keypad buttons: drivers + emulator tests *(calibration UI still to do)*
- [ ] Backlight (PWM), power button
- [ ] SD/SDIO with DMA and 4-bit mode: **SD, SDHC and SDXC** cards (beyond stock WM's 2 GB SD limit), plus a FAT32/exFAT file system *(polled SD/SDHC init + block reads done: drivers/sdcard, drivers/pxa-mmc; emulator test reads the MBR)*
- [ ] CompactFlash (PXA PC Card socket 1, DMA where possible)
- [ ] Audio (WM8750, I²S, DMA)
- [ ] USB client (serial gadget for development networking)
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
- [ ] Installer; v1.0 public release with SD and flash images

---

## Tracking

Each checkbox becomes a GitHub issue under a milestone per phase. The RE track
uses the `re:` label, and every RE finding is merged as a spec in `docs/hw/`.
