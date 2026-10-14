# ADR-0002: Emulator = fork of QEMU 9.1 with an `x51v` machine

- **Status:** Accepted (2026-10-10, by the project owner; written up with
  the Phase 0 foundations, commit 34e8cd2)
- **Date:** 2026-10-10
- **Last verified:** 2026-10-13 (against commit 3158b26)

## Context

Iterating on real hardware is slow and can't run in CI. QEMU removed all PXA2xx
machines in 9.2 (deprecated in 9.0); 9.1 is the last release with PXA270
(`mainstone`) support.

## Decision

> **As built (note added 2026-10-13):** the fork is the separate repo
> `Drakey83/retropane-qemu` (based on QEMU 9.1.3) but is **not** a
> submodule: GitHub Actions builds tagged `rp-v*` releases, and `cargo
> xtask emu` downloads the pinned one (`QEMU_RELEASE` in
> `xtask/src/emu.rs`, `rp-v0.11.3` at this commit); `cargo xtask qemu`
> builds a sibling checkout locally in WSL. `emulator/README.md` still
> describes the submodule. Touch uses a new `tsc2046` model
> (`hw/input/tsc2046.c` in the fork), not QEMU's `ads7846`; `wm8750` is
> reused as planned (`hw/arm/x51v.c`).

Fork QEMU 9.1 into a separate repo (`retropane-qemu`), included here as a
submodule under `emulator/` (Phase 1). Add an `x51v` machine reusing the
existing PXA270, `ads7846`/TSC2046 and `wm8750` models, plus new models written
from our `docs/hw/` specs: CPLD, 2700G display, then MBX 3D, 2D blitter, MVED1
and ACX100 as reverse engineering progresses.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Seconds per test cycle; integration tests and golden screenshots in CI |
| ✨ Modernize | Scriptable over QMP for automated tests |
| 🗄️ Legacy support | Can eventually boot the stock WM ROM to compare behaviour |
| ⚡ Better / faster | Relative performance tests in CI; absolute numbers on the device |

## Alternatives considered

- **Our own emulator in Rust:** full control, but months of work before it's useful.
- **Microsoft Device Emulator (shared source):** wrong SoC (S3C2410), a
  restrictive license, and off-limits under the clean-room policy.

## Consequences

We maintain a QEMU fork. It's GPL-2.0, compatible with our license.
