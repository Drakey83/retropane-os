# ADR-0001: Rust first, on a pinned nightly with a custom ARMv5TE target

- **Status:** Accepted
- **Date:** 2026-10-10

## Context

We are writing a kernel, drivers and a large compatibility layer for a
single-core ARMv5TE (XScale) CPU with no FPU and 64 MB of RAM, and want unit
tests on a PC for everything above the hardware.

## Decision

- The OS is written in **Rust** (`no_std`). Assembly is used only for vectors,
  cache/MMU operations and iWMMXt hot paths. Ported third-party code (browser
  engine, TLS) stays in C behind Rust interfaces.
- We use a custom target, `targets/armv5te-x51v-none-eabi.json`, derived from
  the Tier-3 `armv5te-none-eabi` with `cpu = xscale` and frame pointers kept
  for crash-screen backtraces. It's built with `-Zbuild-std`, so the toolchain
  is a **pinned nightly** (`rust-toolchain.toml`). Bumping it is a deliberate PR.
- ARMv5 has no `LDREX/STREX`, so the target reports no atomics.
  Synchronisation uses short IRQ-masked critical sections (single core). A
  `critical-section` implementation lands with the kernel (Phase 2).

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | `cargo test` on the host for every crate; CI on every commit |
| ✨ Modernize | Memory safety in kernel and drivers; current tooling |
| 🗄️ Legacy support | The CE ABI is exposed through `extern "C"` shims, so the language doesn't affect compatibility |
| ⚡ Better / faster | Same machine code quality as C (LLVM); LTO + `opt-level=3`; hand asm where it wins |

## Alternatives considered

- **C:** mature on ARMv5, but host testing and memory safety are much weaker.
- **Stable Rust only:** impossible without `build-std` for a Tier-3 target.

## Consequences

Nightly churn is possible, so we pin and bump deliberately. Contributors need
`rustup` (it installs the pinned toolchain automatically).
