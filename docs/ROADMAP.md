# Roadmap

> Status: DRAFT v0.1. Phases overlap. The **reverse-engineering (RE) track runs
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
- [ ] **RE:** `tools/romdump`: dump your own X51v ROM from Windows Mobile and list its modules
- [ ] *(Optional)* UART serial adapter for deep debugging. Not required: the cradle's USB is the main console from Phase 2
- [ ] **RE:** baseline benchmarks of stock Windows Mobile (current WM6.1 ROM) on your device (DoomGLES fps, boot time, memory), which become the "at least as fast" target

**Exit:** `cargo xtask test` passes in CI on an empty skeleton; ROM dump in hand.

## Phase 1 — Emulator
**Goal:** an `x51v` machine in a QEMU 9.1 fork that boots a test kernel.

- [x] Fork QEMU 9.1 ([retropane-qemu](https://github.com/Drakey83/retropane-qemu); CI builds Linux + Windows releases, and `cargo xtask emu` downloads them, so no submodule is needed)
- [ ] `x51v` machine: PXA270, memory map, UART console *(done in rp-v0.1.0; 2700G and CPLD logged as unimplemented)*, SD image, keypad
- [ ] TSC2046 touch, WM8750 audio, CPLD model (from `docs/hw/cpld.md`)
- [ ] 2700G display planes model (from the `mbxfb` registers)
- [ ] Screenshot and input scripting over QMP for integration tests
- [ ] **RE:** `tools/mmiotrace` working on Windows Mobile: traps and logs 2700G register accesses

**Exit:** a "hello world" binary prints to serial and draws a test pattern, in the emulator *and* booted from SD on the real device.

## Phase 2 — Kernel
**Goal:** a real kernel with threads, memory protection and full-speed clocks.

- [ ] SD loader (a Windows Mobile app that hands off to our kernel; no flashing)
- [ ] On-screen debug text and crash screen (using the display inherited from Windows Mobile), plus a crash log that survives reboot
- [ ] **USB device driver early:** CDC-ACM console over the stock cradle, with no custom PC drivers
- [ ] Exception vectors, MMU, caches, FCSE slot model, 624 MHz turbo plus frequency scaling
- [ ] Interrupt controller, timers, scheduler (256 priorities, priority inheritance)
- [ ] Processes, threads, sync objects, IPC, driver host process
- [ ] PE loader (host-tested with synthetic PE files)
- [ ] Performance suite v1: context switch, IRQ latency, memcpy bandwidth

**Exit:** multiple processes running, with protected memory, on device and emulator; perf baseline recorded.

## Phase 3 — Core drivers
**Goal:** every "easy" (documented) device working natively.

- [ ] Display (pxafb passthrough first, then native 2700G planes)
- [ ] Touch (TSC2046), keypad, buttons, backlight (PWM)
- [ ] SD/SDIO with DMA and 4-bit mode: **SD, SDHC and SDXC** cards (beyond stock WM's 2 GB SD limit), plus a FAT32/exFAT file system
- [ ] CompactFlash (PXA PC Card socket 1, DMA where possible)
- [ ] Audio (WM8750, I²S, DMA)
- [ ] USB client (serial gadget for development networking)
- [ ] **RE:** full CPLD register map written up in `docs/hw/cpld.md`

**Exit:** touch, keys, SD, CF and audio pass hardware-in-the-loop tests.

## Phase 4 — CE Personality v1
**Goal:** simple unmodified WM5/6.x apps run.

- [ ] Compatibility corpus defined; import-table scanner produces a ranked API backlog
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
- [ ] TCP/IP (IPv4/IPv6), DHCP, DNS, mDNS; TLS 1.3
- [ ] Winsock in the CE Personality

**Exit:** the device joins a WPA2 network and does HTTPS on the real hardware.

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
