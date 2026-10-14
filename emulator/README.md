# emulator/

The emulator is `Drakey83/retropane-qemu`, a fork of QEMU 9.1.3 with an `x51v` machine (ADR-0002). It is not a submodule (corrected 2026-10-13): GitHub Actions builds tagged `rp-v*` releases, `cargo xtask emu` downloads the release pinned in `xtask/src/emu.rs` (`QEMU_RELEASE`), and `cargo xtask qemu` builds a local checkout (`../retropane-qemu`) in WSL, which `emu` then prefers. What it models: `docs/hw/emulator.md`.
