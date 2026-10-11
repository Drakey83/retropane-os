# Contributing to RetroPane OS

Thanks for your interest! RetroPane is a clean-room OS for the Dell Axim X51v.
You can help with code, reverse-engineering findings, testing on real
hardware, or compatibility reports.

## Before you start

1. Read [`docs/DESIGN.md`](docs/DESIGN.md), especially **§1.0 The four lenses**,
   which every change is judged by:
   🔁 iterate · ✨ modernize · 🗄️ legacy support · ⚡ better/faster.
2. Read [`docs/CLEANROOM.md`](docs/CLEANROOM.md). It isn't optional: it's what
   keeps the project legal and safe to distribute.
3. Using an AI coding assistant? Give it
   [`docs/AI-CONTRIBUTING.md`](docs/AI-CONTRIBUTING.md) first: the project's
   identity, hardware constraints, architecture law, review concerns and
   definition of done in one brief. Human contributors will find it a useful
   summary too.

## What you need

- A PC (Windows, macOS or Linux) with [rustup](https://rustup.rs). The pinned
  toolchain installs automatically.
- Optional: a Dell Axim X51v, its stock USB cradle or cable, and an SD card.
  **No special hardware is required.**

## Build and test

```bash
cargo xtask ci        # everything CI runs
cargo xtask test      # host unit tests only
cargo xtask cross     # build for the X51v (ARMv5TE)
cargo xtask lint      # architecture rules (layering, unsafe policy)
```

## Rules of the road

- **Layers:** only `hal/*` touches hardware. Other crates must
  `#![forbid(unsafe_code)]` and depend only on the same or lower layers.
  `cargo xtask lint` enforces this.
- **Tests first:** new behaviour comes with unit tests. Drivers are tested
  against `rp-hal-mock`. Hardware bugs get a regression test before the fix.
- **Hardware facts** go into `docs/hw/` as specs with a confidence level
  (`confirmed` / `likely` / `guess`) and the evidence behind them.
- **Big decisions** get an ADR in `docs/adr/` (copy `0000-template.md`).
- **Performance:** no regressions; include before/after numbers when relevant.
- **Never commit** ROM dumps, DLLs, firmware, disassembly or other proprietary files.

## Reporting

Use the issue templates: bug report, hardware/RE finding, or app compatibility report.

By contributing you agree that your work is licensed under GPL-2.0-or-later.
