# RetroPane OS — Design Document

> Status: **living document**, kept in sync with the code (Phase 1 to 2:
> bring-up images boot in the x51v emulator; kernel work has started). See
> [ROADMAP.md](ROADMAP.md) for progress and [adr/](adr/) for decisions.
> Project name: **RetroPane OS** (chosen 2026-10-10).
> License: GPL-2.0-or-later.

---

## 1. Vision

A brand-new, open-source operating system for the Dell Axim X51v that feels like
Windows Mobile reimagined in 2026: the familiar Today screen, Start menu and
stylus-first UI, rebuilt with modern structure, modern networking and modern
security, while still running the software people loved on Windows Mobile 5 and 6.x.

It is a **clean-room reimplementation**, in the spirit of ReactOS (Windows NT)
and Wine — not a modified Microsoft OS. No Microsoft code is used or shipped.

### 1.0 The four lenses

Every design decision, ADR, pull request and review is judged through these
four lenses. A change should advance at least one and must not set back any:

| Lens | Question to ask |
|---|---|
| 🔁 **Iterate** | Can we measure it, improve it and test it again? Is there a benchmark or test that captures it, so the next iteration can beat it? |
| ✨ **Modernize** | Does it bring 2026 standards, security and UX to the device, not 2005 ones? |
| 🗄️ **Legacy support** | Do WM5/WM6.x apps (and users' muscle memory) still work, unchanged? |
| ⚡ **Better / faster** | Does it use the hardware more fully than the original did: faster, leaner, smoother? |

These lenses become required checkboxes in the PR template and a required
section in every ADR (Phase 0).

### 1.1 Goals

| # | Goal | Measure of success |
|---|------|--------------------|
| G1 | **Native drivers for ALL hardware — non-negotiable.** 3D (MBX), 2D blitter, Wi-Fi, touchscreen, SD and CompactFlash are hard requirements, reverse-engineered where no documentation exists; the rest (audio, keys, backlight, Bluetooth, battery/power, video decoder) follow | Every component in §2 has a driver we wrote and is hardware-accelerated where the hardware allows; no Windows driver DLLs loaded at runtime. Software fallbacks are development aids only and never count as "done" |
| G1b | **Efficient and full speed** | CPU runs at its full 624 MHz turbo clock under load (and scales down when idle); UI holds 60 fps on the GPU; budgets in §11a are met on real hardware |
| G1c | **Beat stock** | After drivers work, iterate (§11b) until RetroPane is measurably faster than the original WM drivers on the same apps |
| G2 | **WM5 + WM6.x application compatibility** | Unmodified WM5 and WM6.0–6.5 ARM `.exe`/`.dll` files run; DoomGLES is the flagship test |
| G3 | **Modern internet** | TLS 1.3, WPA2, IPv6, UTF-8, a usable modern web browser |
| G4 | **Modern take on the WM look** | Windows-esque, retro, modernised: recognisable but fresh |
| G5 | **Proper engineering** | Layered architecture, unit tests from day one, CI, documented decisions |
| G6 | **Anyone can try it** | Public GitHub repo; boots from SD card with no risk to the device; later, optional flash install |

### 1.2 Non-goals (for now)

- Phone-only WM6 Professional features (telephony, SMS radio APIs beyond stubs).
- Phone/GSM features (the X51v has no radio).
- Supporting other devices. The architecture should *allow* other PXA27x devices
  (e.g. the X50v), but we won't spend effort on them until the X51v works.
- Pixel-perfect cloning of Microsoft's UI or artwork (we design our own).

### 1.3 Guiding principles

1. **Layers talk through interfaces, not registers.** Only the HAL touches hardware.
   Everything above it is testable on a PC.
2. **Test before trust.** Every module ships with unit tests; every hardware fact
   ships with a test that would fail if the fact were wrong.
3. **Emulator first, hardware always.** Develop against the emulator, validate on
   the real X51v before calling anything done.
4. **Never brick.** SD-card boot is the default until a recovery path is proven.
5. **Write it down.** Hardware findings go into `docs/hw/` as specifications;
   architectural decisions go into `docs/adr/`.
6. **Fit the box creatively.** 64 MB RAM, 624 MHz, no FPU. Every design choice
   is judged against those numbers.
7. **Performance is a feature.** Measure from the first boot. Hardware acceleration
   (3D, 2D blitter, DMA, display planes, iWMMXt) is the default path, never an
   optional extra. Regressions in the performance tests block merges, the same
   way failing unit tests do.
8. **Zero cost, zero extra hardware.** Anyone with an X51v, its stock USB cradle
   or cable, the SD card already in it and a PC can use and develop RetroPane.
   No purchases, no soldering, no custom drivers. RetroPane installs as a folder
   on the existing SD card and boots from a launcher app in Windows Mobile. It
   overwrites nothing; a soft reset returns to Windows Mobile.

---

## 2. Hardware constraints (summary)

Full inventory with sources: [`research/hardware-inventory.md`](research/hardware-inventory.md).

| Resource | Value | Design consequence |
|---|---|---|
| CPU | Intel PXA270, XScale ARMv5TE, 624 MHz, single core | No Thumb-2, no atomics instructions beyond SWP; single-core simplifies locking |
| FPU | **None** (has iWMMXt SIMD) | Fixed-point everywhere it matters (graphics, audio, layout); iWMMXt asm for hot paths |
| Caches | 32 KB I + 32 KB D, **virtually indexed (VIVT)** | Context switches are expensive unless we avoid cache flushes → see §5.2 (slot memory model + FCSE) |
| RAM | 64 MB SDRAM (some units report 128 MB — verify) | Budget in §11 |
| Flash | M-Systems DiskOnChip G3 (~256 MB) | Bare-metal install target (late phase) |
| GPU | Intel 2700G (PowerVR MBX Lite + MVED1 video), 16 MB own memory | Undocumented 3D → reverse-engineering track (§8) |
| Display | 3.7" 480×640 VGA, resistive touch (TI TSC2046) | Stylus-first UI at 216 dpi |
| Wi-Fi | TI TNETW1100B (ACX100), 802.11b, memory-mapped on PC Card socket 0 | Software WPA2 crypto; firmware from user's own ROM |
| Audio | Wolfson WM8750 over I²S/I²C | Public datasheet |
| Expansion | SD/SDIO + CompactFlash Type II | Boot media; CF also a fallback Wi-Fi path |
| Glue | Unknown CPLD (16×16-bit regs at CS4) for power rails | Partially mapped by AxDroid; must finish RE |

---

## 3. Architecture overview

```
┌──────────────────────────────────────────────────────────────────────┐
│  Applications                                                        │
│  • Native apps (Shell, Browser, Notes, Media, Messaging, Settings)   │
│  • Legacy WM5/6.x apps (unmodified ARM PE binaries, e.g. DoomGLES)     │
├──────────────────────────────────────────────────────────────────────┤
│  CE Personality (Win32-for-CE API surface, clean-room)               │
│  coredll · aygshell · commctrl · winsock · libGLES_CL · EGL · ...    │
├───────────────────────────────┬──────────────────────────────────────┤
│  GWES (window manager,        │  System services                     │
│  input, GDI, compositor)      │  net stack · TLS · audio mixer ·     │
│                               │  storage/FS · registry · power ·     │
│                               │  graphics (GLES 1.1 CL engine)       │
├───────────────────────────────┴──────────────────────────────────────┤
│  Kernel  (scheduler · slot-based VM · IPC · PE loader · syscalls)    │
├──────────────────────────────────────────────────────────────────────┤
│  Drivers  (written against HAL traits — no raw register access)      │
│  display · mbx3d · touch · keypad · audio · sd · cf · acx100 · bt …  │
├──────────────────────────────────────────────────────────────────────┤
│  HAL  (register access, IRQs, DMA, clocks, GPIO, CPLD)               │
│   pxa27x/  (SoC)     x51v/ (board)     mock/ (host tests)            │
├──────────────────────────────────────────────────────────────────────┤
│  Boot  (SD loader launched from WM → later: flash bootloader)        │
└──────────────────────────────────────────────────────────────────────┘
         Real X51v hardware     |     x51v emulator (QEMU fork)
```

### 3.1 Layer rules (enforced in CI)

- A crate may depend only on crates in its own layer or below.
- Only `hal/*` crates may contain `unsafe` MMIO access. Everything else uses
  typed register interfaces from the HAL.
- Drivers depend on HAL **traits**, never on a concrete board. This is what makes
  them unit-testable with `hal/mock`.
- The CE Personality depends on kernel syscalls and services, never on drivers.

A small `xtask lint-layers` check parses `Cargo.toml` dependency graphs and fails
CI on violations.

---

## 4. Language, toolchain and repository layout

### 4.1 Language choices

| Area | Language | Why |
|---|---|---|
| Kernel, HAL, drivers, services, CE personality, GWES, shell | **Rust** (`no_std`) | Memory safety in a kernel; first-class unit testing via `cargo test` on the host; strong typing for registers |
| Boot stub, exception vectors, cache/MMU ops, iWMMXt hot loops | ARM assembly | Unavoidable |
| Ported third-party software (browser engine, TLS, codecs) | C, as upstream | Don't rewrite mature libraries; wrap them behind Rust interfaces |
| Emulator | C (QEMU fork) | See §9 |

**Toolchain:** custom Rust target `armv5te-x51v-none-eabi` (derived from the Tier-3
`armv5te-none-eabi`), built with `-Zbuild-std=core,alloc`. Pinned nightly in
`rust-toolchain.toml`. GCC/Clang arm-none-eabi for C components. No compiler
iWMMXt support can be assumed (recent GCC dropped it) — iWMMXt is hand-written asm.

Single core + no `LDREX/STREX` on ARMv5 → atomics are implemented by briefly
masking IRQs (`critical-section` crate with our own implementation).

### 4.2 Repository layout

```
x51v-os/
├── boot/            SD loader (Windows Mobile app that hands off to our kernel), later flash bootloader
├── kernel/          scheduler, VM, IPC, syscalls, PE loader
├── hal/
│   ├── api/         HAL traits (Gpio, Irq, Dma, Clock, Mmio, Cpld, ...)
│   ├── pxa27x/      SoC implementation
│   ├── x51v/        board: GPIO map, CPLD, memory map, variant detection
│   └── mock/        recording/scriptable fake hardware for host tests
├── drivers/         one crate per device (display, mbx3d, touch, keypad, audio, sd, cf, acx100, bt, battery)
├── services/        net (TCP/IP), tls, audio-mixer, fs, registry, power, gfx (GLES engine)
├── gwes/            window manager, GDI, input, compositor, theme engine
├── personality/ce/  clean-room CE API DLLs (coredll, aygshell, commctrl, ws2, libGLES_CL, ...)
├── shell/           Today screen, Start menu, task switcher, notifications
├── apps/            browser, notes, media, messaging, settings, file manager
├── third_party/     vendored C libraries (with their licenses)
├── emulator/        git submodule: QEMU fork with the `x51v` machine
├── tools/
│   ├── romdump/     dump user's own ROM + extract firmware (Wi-Fi) & test binaries
│   ├── mmiotrace/   Windows Mobile-side tracer for 2700G/ACX100 register traffic (RE)
│   ├── xtask/       build orchestration, image building, lint-layers
│   └── hwtest/      hardware-in-the-loop runner over serial
├── tests/
│   ├── integration/ emulator boot & system tests
│   ├── compat/      WM5/6.x app compatibility corpus (metadata only; binaries user-supplied)
│   └── golden/      reference screenshots/frames
└── docs/
    ├── DESIGN.md    (this file)
    ├── ROADMAP.md
    ├── CLEANROOM.md
    ├── adr/         Architecture Decision Records
    ├── hw/          hardware specifications written from RE (the "spec" side of clean room)
    └── research/
```

Everything builds from one command: `cargo xtask build --target x51v|emu`.

---

## 5. Kernel

### 5.1 Model

A **small hybrid kernel**, modelled loosely on Windows CE's own design so that
compatibility falls out naturally:

- Preemptive priority scheduler, 256 priority levels (CE has 256), priority
  inheritance on mutexes (CE semantics).
- Threads, processes, events, mutexes, semaphores, critical sections, waitable
  handles with `WaitForMultipleObjects` semantics built in.
- Drivers run in a privileged **driver host** process (like CE's `device.exe`),
  not inside the kernel proper. A crashing driver can be restarted without a
  reboot. Drivers get MMIO mappings only for the regions the board description
  grants them.
- Message-passing IPC for service calls, plus shared memory for bulk data
  (framebuffers, audio buffers, network packets).

### 5.2 Memory model — the creative core

ARMv5's VIVT caches mean that a normal "one address space per process" design
must flush caches on every context switch, which is slow. Windows CE 5 solved
this with a **single 4 GB address space split into 32 MB "slots"**, using the
XScale **FCSE** (Fast Context Switch Extension) to relocate the running process
into slot 0 without flushing caches.

We adopt the same model, for two reasons:

1. **Performance:** context switches cost a TLB/PID update, not a cache flush.
2. **Compatibility:** WM5 binaries assume CE5's memory layout (32 MB per process,
   shared DLL region at the top of the slot, memory-mapped files in a shared
   area). Matching it removes a whole class of compatibility bugs.

We improve on CE5 where it doesn't break compatibility:

- Per-process page permissions (CE5 let any process read others' slots).
  Legacy apps that poke other slots get a compatibility shim on request.
- W^X: code pages are never writable after load.
- A modern allocator (size-class slabs + a per-process heap) with fragmentation
  metrics exported for tests.

Decision recorded as [ADR-0003](adr/0003-ce5-slot-memory-model.md); physical memory is designed in [ADR-0005](adr/0005-memory-and-storage-architecture.md) and [ADR-0006](adr/0006-physical-memory-beyond-the-textbook.md), interrupt ownership in [ADR-0007](adr/0007-interrupt-ownership.md).

### 5.3 PE loader

Loads ARM PE/COFF images (`IMAGE_FILE_MACHINE_ARM` and `THUMB`), applies
relocations, resolves imports **by name and ordinal** (many CE apps import
`coredll` by ordinal — the ordinal table is a compatibility artefact we must
reproduce from public SDK import libraries/`.def` files, not from Microsoft
binaries). The loader is pure logic and is fully unit-tested on the host with
synthetic PE files.

---

## 6. CE Personality (legacy compatibility)

Clean-room reimplementations of the WM5/WM6.x system DLLs (WM6.x runs on CE 5.2, a superset of WM5's CE 5.1), written in Rust and exposed
with the CE ARM calling convention.

| DLL | Contents | Priority |
|---|---|---|
| `coredll.dll` | Kernel32/User32/GDI32 subset as merged in CE, CRT | P0 |
| `aygshell.dll` | Pocket PC shell APIs (SIP, menu bars, full-screen) | P0 |
| `commctrl.dll` | Common controls | P1 |
| `ws2.dll` / `winsock.dll` | Sockets, backed by our TCP/IP stack | P1 |
| `libGLES_CL.dll` + EGL | OpenGL ES 1.x Common-Lite (fixed-point) — what DoomGLES links | P0 for Doom |
| `gx.dll` | GAPI (most WM5 games use it) | P1 |
| .NET Compact Framework 2.0/3.5 apps | Many WM6-era apps are managed (.NET CF) code. **Path A:** the user installs Microsoft's own .NET CF runtime CAB, which runs on our personality like any native app (needs `mscoree`-level CE API coverage). **Path B (later):** a clean-room or Mono-based runtime | P2 |
| `ddraw.dll`, `d3dm.dll` | DirectDraw / Direct3D Mobile | P2 |
| `ole32`, `oleaut32`, `wininet`, `cellcore`… | As compatibility corpus demands | P3 |

**Initial compatibility and performance corpus** (the most demanding 2700G titles, chosen by the user):

| Title | Uses | Why it's in the corpus |
|---|---|---|
| DoomGLES: Doom | libGLES_CL, EGL, coredll, aygshell | Flagship. Baseline ~50–60 fps at VGA |
| DoomGLES: Doom II | same | Bigger maps, more stress |
| HereticGLES | libGLES_CL | Needs the 2700G; a second GLES engine variant |
| Cube (GLES port) | libGLES_CL | A different engine with fully 3D geometry |
| Quake 3 (Q3CE) | libGLES_CL | Heaviest title; load-time benchmark |

The original download sites are mostly gone (e.g. kokak.free.fr). The binaries
stay user-supplied and are never committed.

**Coverage is driven by a corpus, not guesses.** `tests/compat/` lists target
apps; a tool scans their import tables and produces a ranked list of
unimplemented APIs. That ranked list is the CE Personality backlog.

The personality is a **thin adapter**: it translates CE semantics into calls to
GWES and services. Native apps use the same services directly through a modern
Rust API, so legacy support never constrains the native design.

---

## 7. GWES, graphics and the shell

### 7.1 Graphics stack

```
 Legacy GDI apps ─┐          Legacy GLES apps (DoomGLES)
                  ▼                     ▼
              GDI engine          libGLES_CL / EGL
                  │                     │
                  ▼                     ▼
        ┌──────────── Compositor ────────────┐
        │ (window surfaces → final frame)    │
        └──────────────┬─────────────────────┘
                       ▼
           gfx service: backend trait
          ┌────────────┴─────────────┐
          ▼                          ▼
   mbx3d (2700G HW)          softgl (fixed-point software rasteriser)
```

- **Two interchangeable backends** behind one trait. The software rasteriser
  exists from day one: it lets the entire UI and GLES API be built and tested
  before the 3D hardware is reverse-engineered, gives the emulator something to
  draw with, and is the **reference implementation** — hardware output is
  tested against it (per-pixel tolerance) in golden-image tests.
- The compositor uses the 2700G's display planes/overlays (documented via the old
  Linux `mbxfb` registers) for zero-copy fullscreen apps and video.

### 7.2 Look and feel

"Windows-esque, retro, modernised":

- Keeps the WM vocabulary: Today screen, Start menu, title-bar clock, soft-key
  bar, stylus + D-pad navigation, SIP keyboard.
- Modern treatment: crisp vector icons at 216 dpi, a modern type ramp, dark mode,
  smooth GPU-composited transitions, notification centre, proper task switcher.
- A **theme engine** with at least two themes: *Modern* (default) and *Classic*
  (a faithful WM5-style homage, using our own artwork).
- Design spec lives in `docs/ui/` (to be written) with mock-ups before code.

---

## 8. The 2700G: 3D, 2D and video, via reverse engineering

All three undocumented 2700G blocks are **required**:

- **MBX 3D**: used by GLES, the compositor and the UI.
- **2D blitter**: used by GDI, scrolling, text and legacy apps.
- **MVED1 video decoder**: used for media playback.

This is the highest-risk part of the project and runs as a **parallel research
track starting in Phase 0**. The same method also applies to the ACX100 Wi-Fi
chip, the CPLD and the power IC.

### 8.1 What's known

| Area | Public documentation |
|---|---|
| Clocks, SDRAM controller, display planes, power, GPIO | ~90% — from the old Linux `mbxfb` driver (removed from mainline in 2020; retrievable from git history) |
| 2D blitter | none |
| MVED1 video decoder | none |
| MBX 3D (tile accelerator, parameter/display-list formats, texture layout, IRQs) | **none** |

The closest well-documented relative is the **Dreamcast PowerVR2 (CLX2)** —
same tile-based deferred rendering lineage. It provides the conceptual model;
we expect MBX formats to differ in detail.

### 8.2 Reverse-engineering method

1. **Acquire** — `tools/romdump` dumps the user's own X51v ROM and extracts
   `libGLES_CL.dll`, the display driver and the kernel-side MBX services DLL.
   These stay on the user's machine; they are never committed.
2. **Trace** — `tools/mmiotrace` runs on Windows Mobile on the real device. It unmaps the
   2700G register window and traps each access through the data-abort handler
   (the same technique as Linux `mmiotrace`), logging address/value/time to SD.
   We drive it with tiny known GLES programs (clear screen, one triangle, one
   textured quad, ...) so each trace isolates one feature.
3. **Analyse** — disassembly of the extracted DLLs is used to *explain* traces,
   not to copy code.
4. **Specify** — findings are written up as hardware specs in `docs/hw/mbx/`
   (register maps, command formats, init sequence), each with a confidence
   level and the trace that proves it.
5. **Implement** — `drivers/mbx3d` is written **from the spec only** (see
   `CLEANROOM.md`), test-first against `hal/mock` replaying recorded traces.
6. **Model** — the same spec drives an MBX model in the emulator, so the driver
   can run in CI.

Milestones:

- **3D:** first register-level init → first cleared screen → first triangle →
  textured triangle → GLES 1.1 CL conformance subset → **DoomGLES runs at full speed**.
- **2D:** fill → copy blit → transparent/ROP blits → GDI fully accelerated.
- **Video:** decoder init → first decoded frame → media player uses the hardware path.

The software rasteriser (§7.1) remains as the test reference and emulator
backend. It is **not** an acceptable shipping substitute for the hardware.

---

## 9. Emulator

Real hardware is slow to iterate on and can't run in CI, so we need an emulator.

**Decision (proposed — ADR-0002):** fork **QEMU 9.1**, the last release that
still contains PXA270 support (removed in 9.2), and add an `x51v` machine:

| Model | Source |
|---|---|
| PXA270 SoC, timers, IRQ, GPIO, MMC, UARTs, I²S, SSP | existing QEMU PXA code |
| TSC2046 touch | adapt QEMU's existing `ads7846` model |
| WM8750 audio | existing QEMU `wm8750` model |
| CPLD | new, from our `docs/hw/cpld.md` spec |
| 2700G display/planes | new, from `mbxfb` registers |
| 2700G MBX 3D, 2D blitter, MVED1 video | new, grows with the RE track |
| ACX100 Wi-Fi | new (later); bridged to host networking |
| Keypad, SD image, CF slot | adapt existing |

Alternatives considered: writing our own emulator in Rust (more control, much
more work), or using Microsoft's shared-source Device Emulator (wrong SoC, and
its license and clean-room concerns rule it out). QEMU is GPL-2, which matches our license.

The emulator is a **separate repo** included as a submodule, so it can live its
own life (and maybe be useful to other X50v/X51v hackers).

---

## 10. Networking and the modern internet

| Layer | Plan |
|---|---|
| Wi-Fi driver | Native `acx100` driver from our own spec. Prior art: the out-of-tree acx-mac80211 driver and the AxDroid Axim glue, used as **references for facts** only. |
| Firmware | The ACX100 needs TI firmware, which we can't redistribute. `tools/romdump` extracts it from the user's own device. |
| 802.11 / security | Our own 802.11 MAC layer (management, scanning, association). The hardware only does WEP, so **WPA2-PSK (CCMP) runs in software** (fine at 11 Mbps). WPA3-SAE is a stretch goal. |
| Known limitation | 802.11b only. Many modern routers disable 802.11b rates by default, and WPA3-only networks won't work. Settings will explain this clearly. The built-in ACX100 is required to work. Supporting **CF 802.11g cards** is an optional extra on top, not a substitute. |
| TCP/IP | `smoltcp` (Rust, BSD-0) or similar: IPv4/IPv6, DHCP, DNS, mDNS |
| TLS | TLS 1.3 + 1.2 via mbedTLS or wolfSSL (decide in ADR). X25519 + ChaCha20-Poly1305 preferred (no FPU/AES hardware needed). Mozilla CA bundle with updates. |
| Browser | **NetSurf** port (CSS, partial JS via Duktape) for direct browsing, plus a **"Bridge" mode**: an optional self-hostable companion service (on your PC or a server) that renders JavaScript-heavy sites with a headless browser and streams a simplified page or image tiles to the device (in the spirit of WRP/Browservice/Opera Mini). Both modes in one browser UI. |

### 10.1 Browser: on-device first (decided 2026-10-10)

**Principle: do as much as possible on the PDA's own hardware.** Off-device help
is a last resort, chosen per site and only when the device truly can't cope.

| Tier | What runs on the PDA | What runs off-device | When |
|---|---|---|---|
| **1. Native** (default) | Everything: networking, TLS 1.3, HTML/CSS parsing, layout, a modern JS engine (QuickJS-class), image decoding, rendering | Nothing | Always tried first |
| **2. Assist** | Layout and rendering in our own engine; user interaction | Only the JavaScript runs on a helper, which sends back the finished page structure (DOM snapshot) | Pages whose scripts exceed the device's RAM or CPU |
| **3. Bridge** | Display and input only | Full page rendering on a helper | Last resort, for heavy web apps |

Getting the most out of the hardware for Tier 1:

- **Use the 2700G's 16 MB of video memory** for page tiles, images and scroll
  buffers. That keeps them out of the 64 MB system RAM, and MBX composites and
  scales them.
- **Hardware video decode** via MVED1 for the formats it supports. Software
  decode with iWMMXt for the rest, where feasible.
- **iWMMXt-optimised** JPEG/PNG/WebP decoding, text rendering and crypto.
- **Fixed-point layout engine** (no FPU), with incremental and streaming
  parsing so pages render while they download.
- **Compressed memory plus SD/CF cache:** compressed RAM for background tabs,
  and an on-card cache for resources.
- **Memory-aware engine:** images are discarded and re-decoded on demand, with
  strict per-tab budgets so the browser never takes down the OS.
- **Per-site memory:** the browser remembers which tier each site needed and
  re-tries Tier 1 after updates. Every site that moves down a tier is a bug
  report for making the native engine better.

Being honest about the limits: no 64 MB device can run today's heaviest web
apps (YouTube, Gmail, Google Docs) entirely by itself. Tiers 2 and 3 exist only
to cover that gap.

---

## 10b. Connecting to a PC, and debugging, with no special hardware

**Principle: nobody, users or developers, should need a custom cable.**
Everything goes through the standard Dell USB sync cradle or cable and the SD
card, using **only drivers built into Windows 11, macOS and Linux**. No custom
PC drivers (which would also need expensive Microsoft code-signing).

| Function | How | PC driver |
|---|---|---|
| Install / update | SD card image; later an updater app | none |
| Debug console / logs | USB **CDC-ACM** serial over the cradle | built in (shows up as a COM port) |
| Networking via PC | USB **CDC-NCM / RNDIS** | built in |
| File transfer | USB **MTP** | built in |
| Companion app | "RetroPane Connect" for Windows 11 (later macOS/Linux): install, sync, logs, crash reports, Bridge host | uses the above |

Because nothing works over USB until our own USB device driver runs, the
**PXA USB device controller (UDC) driver is a Phase 2 priority**, delivered with
the kernel rather than with the other drivers. Before USB is up, or when a crash
takes it down:

1. **On-screen crash screen.** The loader inherits Windows Mobile's working display,
   so the very first kernel code can draw text. Panics show a RetroPane "blue
   screen" with the reason, registers and backtrace.
2. **Crash log that survives a reboot.** Panic data goes into a reserved RAM area
   that keeps its contents through a soft reset (SDRAM self-refresh). The next
   boot sends it over USB and saves it to SD.
3. **SD card logging** for anything longer.

### 10c. Install stages and the brick-proof rule

1. **Emulator.** The default for development.
2. **SD card.** A folder plus a launcher on the existing card. Windows Mobile stays
   in flash, and a soft reset returns to it.
3. **Flash install** (only when RetroPane is ready). Uses Dell's own **bootloader
   mode** (entered with the reset button), the same path the community used for
   ROM updates. See the user's own 2008 guide:
   https://www.youtube.com/watch?v=vEDvCklrNs0

**Brick-proof rule: RetroPane never writes to the Dell bootloader region.** The
installer and flash tools refuse to touch it, and tests verify that. With the
bootloader intact, any failed install is recovered by entering bootloader mode
and reflashing a stock Dell or LennySh ROM. Before the first flash, keep a full
backup of the user's current ROM.

A UART serial cable (3.3 V adapter on connector pins 10/11/12) stays supported
as an **optional** developer tool for the deepest bring-up work. It's never
required.

| Consumer | Budget |
|---|---|
| Kernel + driver host | 4 MB |
| Display: front/back buffers in 2700G memory (not system RAM) | 0 MB system RAM |
| GWES + compositor + shell | 6 MB |
| Services (net, TLS, audio, fs) | 4 MB |
| File cache | 4–8 MB (elastic) |
| **Available to apps** | **~40 MB** |

Budgets are enforced by tests: integration tests boot the emulator, run a
scripted session and fail if any component exceeds its budget.

## 11a. Performance requirements ("full speed")

| Area | Requirement |
|---|---|
| CPU clock | Kernel brings the PXA270 to **624 MHz turbo** at boot. Dynamic frequency scaling drops it when idle and returns to full speed within one scheduler tick under load |
| Memory bus | SDRAM and bus timings tuned to the fastest stable values (measured per unit, with safe defaults) |
| Caches | I/D caches, write buffer and branch target buffer on from the first instruction. The slot/FCSE model (§5.2) avoids cache flushes on context switch |
| Graphics | Compositor and UI on MBX at **60 fps**. Scrolling and text through the 2D blitter. Fullscreen apps scan out directly from display planes with zero copies |
| I/O | DMA for SD, CF, audio, Wi-Fi and display. No polling loops in drivers; everything interrupt-driven |
| Hot paths | Fixed-point maths; iWMMXt asm for memcpy/memset, blending, audio mixing, crypto (ChaCha20) and image decoding |
| Boot | Target under 5 s from loader to Today screen |
| Legacy apps | WM5/6.x apps run at least as fast as on stock Windows Mobile (benchmarked with the same apps on the same device) |

Each requirement has a matching benchmark in the performance test suite
(§12). Absolute numbers are recorded on real hardware at every milestone.

### 11b. Beyond stock: iterate until we beat the original drivers

Matching Intel's and Microsoft's drivers is the floor. The goal is to **beat
them**, because reverse engineering shows us what the hardware can really do,
not just what the 2005 drivers used. Once a driver works, it enters an
optimisation loop:

1. **Measure** against the stock baseline (same app, same device, same settings).
2. **Profile** to find where the time goes: CPU, GPU, bus, storage. Use the PXA270
   performance counters and our own trace tooling.
3. **Experiment** with what the old drivers didn't do: deeper command batching,
   smarter texture upload, DMA everywhere, better memory placement in the 2700G's
   16 MB, tighter interrupt handling.
4. **Lock in** every win with a benchmark, so it can never regress.

Stock baselines (user's X51v on the LennySh WM6.1 ROM, 2026-10-10):

| App | Stock behaviour | First improvement targets |
|---|---|---|
| DoomGLES (Doom, Doom II) | Full VGA resolution, **~50–60 fps** (estimated, no FPS counter) | Hold a locked 60 fps at VGA, then add effects. Add an FPS overlay so we can measure precisely |
| Quake 3 (Q3CE) | **Very slow loading**; roughly full speed once running | Load time: SD 4-bit mode at max clock, DMA, read-ahead, a better file cache, and faster texture upload to the 2700G |

---

## 12. Testing strategy

Testing is part of the architecture, not an afterthought.

### 12.1 Test pyramid

| Level | What | Where it runs | When |
|---|---|---|---|
| **Unit** | Every crate: pure logic, plus drivers against `hal/mock` | Host (x86-64 PC), `cargo test` | Every commit (CI) |
| **Spec tests** | Each documented hardware fact in `docs/hw/` has a test: replayed MMIO traces must produce the expected driver behaviour | Host | Every commit |
| **Property / fuzz** | PE loader, network stack, TLS glue, file system, font/image parsers | Host (`proptest`, `cargo-fuzz`) | Nightly CI |
| **Integration** | Boot the OS in the emulator, run scripted scenarios over the serial console, compare screenshots with golden images | Emulator in CI | Every PR |
| **Compat** | Launch each corpus app in the emulator; record pass/fail/crash; track it as a dashboard over time | Emulator (binaries user-supplied, not in CI) | Weekly / on demand |
| **Hardware-in-the-loop** | Same integration scripts on the real X51v over serial (`tools/hwtest`) | Real device | Before each milestone |
| **Performance** | Boot time, frame time, memory budgets, context switch cost | Emulator (relative), device (absolute) | Per milestone |

### 12.2 Rules

- A PR that adds behaviour adds tests. CI blocks merges below the coverage floor
  for non-HAL crates (target 80% line coverage for logic crates).
- Drivers are written **test-first** against `hal/mock`: the mock records every
  register access, so tests assert "the driver did exactly this sequence".
- Bugs found on hardware become a mock-based regression test before the fix.
- Golden images are generated by the software rasteriser and reviewed by a human.

### 12.3 CI (GitHub Actions)

`fmt` → `clippy` → `lint-layers` → host unit/spec tests → cross-build for
`x51v` → build emulator → emulator integration tests → publish build artefacts
(SD-card image) on tagged releases.

---

## 13. Naming

**Decided (2026-10-10): RetroPane OS.**

- **Retro:** the device and the WM heritage.
- **Pane:** a Windows-esque nod without the trademark.
- Together, the name reads as a modern take on a window.

**Release branding (2026-10-11): RetroPane 26.** The project is "RetroPane OS";
releases carry a two-digit year version, in the spirit of Windows Mobile 5 and 6
and of current year-numbered OS releases. "26" is short, reads as a version, and
naturally becomes 27 for the next major release, where "2026" would just look
dated.

A web search on 2026-10-10 found no software product with the name. Still to
check: a domain. The GitHub repository is `Drakey83/retropane-os`.

---

## 14. Legal and clean-room policy (summary)

Full policy: [`CLEANROOM.md`](CLEANROOM.md).

- No Microsoft, Intel, Imagination or TI code in the repository. Never.
- Microsoft's shared-source Windows CE code and the Device Emulator source are
  **off-limits** to contributors.
- Proprietary binaries (ROM contents, firmware, test apps) live only on the
  user's machine and are extracted from their own device by our tools.
- Reverse engineering happens **only for interoperability**, and its output is
  written up as specifications. Implementation is written from those specs.
- GPL-2.0-only code (e.g. AxDroid's Linux board files) is used as a *reference
  for facts* such as GPIO numbers. It is not copied, so our code stays
  GPL-2.0-or-later.

---

## 15. Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| MBX 3D / 2D / video RE takes much longer than hoped | High | High: they're hard requirements (G1) | Start the RE track in Phase 0, alongside everything else. Run small, isolated trace experiments. Specs are versioned, so progress is visible. The software backend lets UI and API work continue meanwhile (development only, not a substitute) |
| ACX100 Wi-Fi is hard to get stable (it never worked in AxDroid) | Medium | High: it's a hard requirement (G1) | Trace WM5's driver on the device. A USB-serial network link keeps development going in the meantime |
| Power/battery IC unknown → no suspend, poor battery life | High | Medium | Trace WM5 power management; accept "always on" in early releases |
| WM5/6.x API surface is larger than expected | High | Medium | Corpus-driven prioritisation; publish compatibility status honestly |
| PXA270 dropped from Linux/QEMU → fewer references | Certain | Low | Pin QEMU 9.1; keep LTS-kernel sources as reference |
| Device variance (64 vs 128 MB RAM, panel types, board revisions) | Medium | Low | Read board variant from the CPLD at boot; test on multiple units if contributors have them |
| Hardware failure (20-year-old battery and connectors) | Medium | Medium | Encourage contributors with spare units; the emulator reduces dependence on hardware |

---

## 16. Open questions for review

1. ~~Rust-first~~: **Decided (2026-10-10).** As fast as C, memory-safe, testable.
2. ~~QEMU 9.1 fork~~: **Decided (2026-10-10).** Fastest route to a working emulator.
3. ~~CE5-style slot memory model~~: **Decided (2026-10-10).** The most efficient design on ARMv5 VIVT caches (no ASIDs before ARMv6), and compatible with WM5.
4. ~~Bridge mode~~: **Decided (2026-10-10).** The browser does as much as possible on the PDA (§10.1). Assist and Bridge are per-site fallbacks of last resort.
5. ~~Serial cable~~: **Decided (2026-10-10).** Not required for anyone. The stock USB cradle with built-in PC drivers is the main link (§10b); UART is optional.
6. ~~Name~~: **Decided: RetroPane OS** (§13).
7. ~~RAM/ROM~~: The user's unit is an **Axim X51v** (fitted with a replacement X50v front bezel): 52.2 MB available (64 MB physical), running a WM 6.1 Classic community ROM (CE 5.2.20273, LennySh). So the unit has been reflashed before. **Open:** add WM6.1 apps to the compatibility target?
