# ADR-0002: Emulator = fork of QEMU 9.1 with an `x51v` machine

- **Status:** Accepted
- **Date:** 2026-10-10

## Context

Iterating on real hardware is slow and can't run in CI. QEMU removed all PXA2xx
machines in 9.2 (deprecated in 9.0); 9.1 is the last release with PXA270
(`mainstone`) support.

## Decision

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
