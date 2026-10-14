# ADR-0004: Testing policy

- **Status:** Accepted (2026-10-10, written up with the Phase 0
  foundations, commit 34e8cd2; who accepted it is not recorded)
- **Date:** 2026-10-10
- **Last verified:** 2026-10-13 (against commit 3158b26)

## Context

Most of the work is reverse-engineered hardware support on a device that can't
run in CI. Without discipline, regressions would be found weeks later, on hardware.

## Decision

> **As built (note added 2026-10-13):** rule 1: `cargo xtask lint`
> checks `#![forbid(unsafe_code)]` only in library roots (`lint()` uses
> `lib_root`, `xtask/src/main.rs`; `xtask/src/policy.rs`), so the
> binary-only `boot/*` images are not checked; each carries the
> attribute anyway. Rule 7: `cargo xtask ci` runs fmt, clippy, lint, test
> and cross (`xtask/src/main.rs`), while CI also runs the `emu`, `bench`
> and `coverage` jobs (`.github/workflows/ci.yml`); `docs/testing.md`
> asks for both `cargo xtask ci` and `cargo xtask emu` before a commit.

1. **Layers are enforced:** only `hal/*` touches hardware (`cargo xtask lint`
   checks dependency direction and `#![forbid(unsafe_code)]` outside the HAL).
2. **Drivers are test-first** against `rp-hal-mock`, which records every
   register access. Recorded device traces can be replayed as test data.
3. **Every hardware fact has a test** that fails if the fact is wrong (spec tests).
4. **Coverage floor:** 80% lines for host-testable crates, enforced in CI.
5. **Bugs found on hardware** get a mock-based regression test before the fix.
6. **Performance is tested:** benchmarks for every "full speed" requirement;
   regressions block merges like failing tests.
7. **One command:** `cargo xtask ci` runs exactly what CI runs.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Every improvement is locked in by a test or benchmark |
| ✨ Modernize | Fuzzing and property tests for parsers and the network stack (from Phase 2) |
| 🗄️ Legacy support | The compatibility corpus is tracked over time (`tests/compat/`) |
| ⚡ Better / faster | The performance suite guards the "beat stock" goal |

## Alternatives considered

> **Flag (2026-10-13 truth pass):** the alternatives here are missing
> (this ADR has no such section); to be filled in by the owner's review.

## Consequences

Slightly more up-front work per feature, in exchange for much faster iteration
later and confidence that hardware bring-up doesn't regress.
