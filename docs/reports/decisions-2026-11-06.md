# Decisions waiting for the owner (deep-dive list, 2026-11-06)

Written while the owner was away; nothing here is decided. Tick a box to
rule. Each item's code is built on branch `fix/deep-dive-2026-11-06`
(not pushed).

## Item 1: the divide-1 cures

- [ ] **Accept ADR-0035 amendment E** (the cures follow the clock: on for
  every render into screen 1 at divide 1 above 74.75 MHz, off on die and
  at divide 2; R120's switch stays dropped).
  [docs/adr/0035-2700g-clocks.md](../adr/0035-2700g-clocks.md), "Amendment E".
- [ ] **Run the device check** ([run sheet](../research/sheets/divide1-cures-by-clock.md)):
  `rp-scene` divide-1 variant, P8 judged by DSIG. Owed; not made.
- [ ] **Which divide-1 rungs ship** stays amendment D's vote (Q1-Q3),
  unchanged by E.

## Item 5: the stack

- [ ] **Approve the loader's zeroed-RAM budget 36 -> 42 KiB** (`xtask/src/size.rs`):
  the two card LUNs moved from `main`'s frame into static slots, the
  loader's deepest stack path 87% -> 67% of its 32 KiB. The same bytes,
  counted in the zeroed RAM instead of the stack. Unticked, the LUNs go
  back on the stack and the loader stays on R149's list.

