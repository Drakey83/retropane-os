# ADR-0001: Rust first, on a pinned nightly with a custom ARMv5TE target

- **Status:** Accepted (2026-10-10, by the project owner; written up with
  the Phase 0 foundations, commit 34e8cd2)
- **Date:** 2026-10-10
- **Last verified:** 2026-10-13 (against commit 3158b26)

## Context

We are writing a kernel, drivers and a large compatibility layer for a
single-core ARMv5TE (XScale) CPU with no FPU and 64 MB of RAM, and want unit
tests on a PC for everything above the hardware.

## Decision

> **As built (note added 2026-10-13):** assembly is used for more than
> vectors, cache/MMU and iWMMXt: the thread switch (`rp_switch`,
> `hal/pxa27x/src/thread.rs`), user-mode entry and `SWI` system calls
> (`hal/pxa27x/src/user.rs`, `hal/user/src/lib.rs`), the boot hand-off
> (`hal/pxa27x/src/handoff.rs`) and the `ldm`/`stm` + `pld` copy and fill
> loops (`hal/mem/src/lib.rs`); all of it stays inside `hal/*`. No
> `critical-section` crate is used: IRQ-masked sections are the HAL's own
> (`irq_save` / `irq_restore` / `with_dispatch`, `hal/pxa27x/src/rt.rs`).
> A second custom target, `targets/thumbv5te-x51v-none-eabi.json`, builds
> cold images as Thumb (ADR-0015). No C code is in the tree yet
> (`third_party/` holds only its README).

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

> **Correction (note added 2026-10-13):** `opt-level=3` below is only the
> release profile's default; cold crates are built with `opt-level = "s"`
> (`[profile.release.package.*]` in `Cargo.toml`, ADR-0005 rule 8) and
> cold images as Thumb (ADR-0015).

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
