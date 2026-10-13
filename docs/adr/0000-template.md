# ADR-NNNN: Title

- **Status:** Proposed | Accepted | Superseded by ADR-XXXX
- **Date:** YYYY-MM-DD

## Context

What problem are we solving, under which constraints (64 MB, 624 MHz, no FPU, VIVT caches...)?

## Decision

What we will do.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | How it's measured and tested, and how a later iteration can improve it |
| ✨ Modernize | |
| 🗄️ Legacy support | |
| ⚡ Better / faster | |

## Hardware limit or choice?

For every behaviour inherited from Windows Mobile, Dell or the 2005 design
that this decision keeps: is the silicon forcing it (with the evidence), or
was it a software choice? A choice is open to replacement (docs/CONTRIBUTOR-BRIEF.md,
"imitation is the last resort").

## Alternatives considered

The 2-4 serious alternatives (faster, more modern, cleverer, the device's
parts working together), measured where that is cheap, and why each was
taken or rejected. Bounded to the size of the decision. If the decision
imitates the original, this section is where that is justified, ideally
with a measurement against the stock-WM baseline (docs/perf.md).

## Consequences

What becomes easier, harder, or must be revisited.
