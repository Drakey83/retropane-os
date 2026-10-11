# ADR-0004: Testing policy

- **Status:** Accepted
- **Date:** 2026-10-10

## Context

Most of the work is reverse-engineered hardware support on a device that can't
run in CI. Without discipline, regressions would be found weeks later, on hardware.

## Decision

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

## Consequences

Slightly more up-front work per feature, in exchange for much faster iteration
later and confidence that hardware bring-up doesn't regress.
