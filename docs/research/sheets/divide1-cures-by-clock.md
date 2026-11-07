# Run sheet: the divide-1 cures by the clock on the X51v (ADR-0035 amendment E, PROPOSED)

**Status: OWED. This run has not been made.** The rule is built and
checked in the emulator only (`cargo xtask emu-scene-d1`, 2026-11-06). The
owner was away when it was built. Nothing here is device proof.

## What it proves

That with the cures left to the clock (the default the amendment
proposes), every 3D render into screen 1 at 143.00 / 1 takes the underlay
and the alpha-0 overlay, and its refreshes are as right as with them held
on (R121's P5).

## The run

- Image: `rp-scene` divide-1 variant, phases P3 (149.50 / 2), P4 (2D
  flips), P5 and P6 (143.00 / 1, cures held on), P7 (cures held off: the
  control, some refreshes should be wrong), **P8 (143.00 / 1, every frame
  by the copy, cures by the clock)**.
- Command: `RP_SCENE_DIVIDE1=1 cargo xtask load rp-scene` (the loader up;
  `RETROPANE_LOG_WAIT_SECS=600`).
- About 3 minutes. Unattended: the image steps the clock, runs, puts
  Windows Mobile's clock back and returns.

## Pass

- P8: `screen1 N (with the cures N)` with N > 0 on both lines (back to
  back, one-off); `torn 0`; DSIG equal to the references on every judged
  refresh; `failed 0, recovered 0`.
- P7 (control): `with the cures 0`; some refreshes wrong is expected.
- `put back: Some(` and the run back at the loader.

## What overturns it

A torn or wrong refresh in P8 while P5 is clean: then the rung read at the
render is wrong (read too early around the clock step) and the switch
moves to `clock_changed`.

## Evidence

The run's report goes in `docs/reports/runs/` (item 2's rule), not in
`local/`.
