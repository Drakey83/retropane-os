# ADR-0015: Thumb for cold code

- **Status:** Accepted (2026-10-12, by the project owner: "let's try it,
  with the understanding that we revert and do it all ARM if I manually
  test and doing it this way makes the OS feel too sluggish"). Steps 1 and
  2 done the same night: the loader is Thumb (127.7 -> 94.2 KB, every
  emulator test passing). Reverting is emptying `THUMB_IMAGES` in
  `xtask/src/main.rs`.
- **Date:** 2026-10-12

## Context

The XScale (ARMv5TE) runs two instruction sets: ARM (32-bit instructions)
and Thumb (16-bit, Thumb-1). Every RetroPane image is built as ARM today.
ADR-0005 makes memory the first constraint: 64 MB of RAM, a non-XIP flash,
everything loaded into RAM to run. Cold code (bring-up, file systems,
loader commands, settings, error paths) is most of an OS by size and a
small share of its time.

Measured on 2026-10-12: the file systems (`rp-fat`, FAT32 and exFAT, with
`AnyVolume`) and the CompactFlash driver, instantiated for a concrete
device and built as the loader builds them (opt-level "s", fat LTO):

| Instruction set | Code |
|---|---|
| ARM (today) | 30,421 B |
| Thumb | 21,083 B (−31%) |

The loader is ~98 KiB, mostly cold; Thumb would give back roughly 20 KiB
there, and proportionally across the OS.

Thumb-1 is slower per instruction for hot loops (fewer registers reachable,
no conditional execution, no `ldm`/`stm` with high registers), and it
cannot encode coprocessor instructions (`mcr`/`mrc`: caches, MMU, CP15),
so hot paths and every function with such inline assembly must stay ARM.
ARMv5TE interworks in hardware (`blx`), so the two mix freely at function
boundaries, and LLVM never inlines across them.

Ways to get there, tried or examined:

1. `-C target-feature=+thumb-mode` on the loader crate (per-package
   rustflags). Tried: rustc warns it is unstable and unsound and is being
   removed. Rejected.
2. `#[instruction_set(arm::t32)]` on cold functions (stable Rust): sound,
   but per function; large crates need it on every cold function, and a
   missed one stays ARM silently (only size suffers).
3. A Thumb target (`thumbv5te`, same spec otherwise) for cold images and
   cold crates, with `#[instruction_set(arm::a32)]` on every function with
   coprocessor assembly or a hot loop (the HAL's cache, MMU, vector and
   thread-switch code, `hal/mem`). A missed annotation is a build error
   (Thumb-1 can't encode `mcr`), so mistakes can't reach the device.

## Decision (proposed)

Option 3, in steps, each measured in the emulator's benchmarks (which
count instructions, so a slowdown shows) and only then on the device:

1. Annotate the HAL's assembly functions and `hal/mem` as `arm::a32` (a
   no-op on today's ARM builds), checked by a build of the HAL for the
   Thumb target in CI.
2. Build the loader for Thumb; compare size and the emulator's drive and
   put timings; device run only after the emulator passes, with the old
   loader kept on the card for recovery.
3. Then the probe and the kernel's cold crates; hot paths (scheduler, page
   allocator, LZ4, display fills) stay ARM by annotation, guided by the
   benchmarks.

## Consequences

- Images shrink by roughly a third of their cold code: memory back for
  apps, faster loads from the card.
- Two instruction sets to keep in mind; the annotations make the hot/cold
  split explicit and reviewable, and the build enforces the hard part.
- The loader is the device's lifeline: it changes last among the cold
  images' first steps, and only with a recovery copy on the card.
