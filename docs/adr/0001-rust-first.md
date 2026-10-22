# ADR-0001: Rust first, on a pinned nightly with a custom ARMv5TE target

- **Status:** Accepted (2026-10-10, by the project owner; written up with
  the Phase 0 foundations, commit 34e8cd2)
- **Date:** 2026-10-10
- **Last verified:** 2026-10-13 (against commit 3158b26)
- **Amended 2026-10-22 (the performance gate's C9; the owner's vote B):**
  frame pointers from "always" to "may-omit" in both target specs. See
  "Frame pointers, measured" below.

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

## Frame pointers, measured (2026-10-22, amendment accepted: B)

The target keeps frame pointers in every function "for crash-screen
backtraces". Nothing walks them: no backtrace was ever built (the crash
line reports the fault's address, word and SPSR). Measured on the X51v
(`rp-probe` built both ways, every self-check passing, docs/perf.md):

- **Frame pointers optional (`"frame-pointer": "may-omit"`):** lz4 2-6%
  faster, the photo row 7%, the frame allocator 3%, the TLB test 6%; the
  image 1.5 KB smaller; **nothing slower** (the SD reads that first looked
  6% slower were the driver's tight polling, since fixed).
- `cpu = xscale` (written above) isn't in the target spec, and measured
  makes byte-identical code: no gain either way.

**Options:** A, keep "always" (backtraces stay possible by walking frames,
when one is built); **B (recommended), "may-omit"**: 2-7% on compute and
smaller code everywhere; a backtrace, when wanted, comes from a debug
build with frame pointers on (one flag) or unwind tables, not every
build paying for it.

**Decision (owner, 2026-10-22): B, may-omit.** "Nothing walks them
today: no backtrace was ever built, the crash line reports the fault
directly." Both `targets/*.json` say `"frame-pointer": "may-omit"`; a
debug build that wants a backtrace turns them back on with one flag.

**Measured again (2026-10-22, the perf ratchet's floors; numbers in
docs/design/perf-ratchet.md):** the claim above, "nothing slower", is
wrong. `rp-probe` built just before and at the switch (1708939), two
runs each on the X51v: LZ4 compress 4.1-6.1% slower and the byte-wide
DMA copy 4.7% slower with may-omit, against the photo row 7.1%, LZ4
decompress 2.1% and the frame allocator 2.3% faster (quality audit A7).
The decision stands until the owner's next vote. **Options:** keep B
(take the new floors); go back to A (frame pointers everywhere: the
slower rows back, the faster ones and 1.5 KB lost); or keep B and find
why those two loops lose (the code each build makes for LZ4's compress
loop and the DMA copy's poll, compared), which could keep both gains (a
backlog optimization, device runs without a reset).

**The crash-screen backtrace is mode-dependent (owner, 2026-10-22).** B
stands. Frame pointers are opt-in per image, a list in xtask (like the
Thumb list), starting with `rp-crash` only: such an image is built in its
own target folder with `-C force-frame-pointers=yes --cfg rp_frame_pointers`
(the whole build, `core` included, so the chain is unbroken), and only
there is the walker compiled in. Every other image (the kernel, the loader,
and the benchmark images whose numbers are the perf ratchet's floors) is
may-omit, carries no walker code at all, and prints "no frame chain (built
may-omit)". Emptying the list recovers everything; nothing to remove. The
Thumb loader (frame pointer r7, chains broken by ARM/Thumb calls) stays
may-omit.

**Built (2026-10-22).** The list is `FRAME_POINTER_IMAGES` in xtask (`rp-crash`);
every image build (`cross`, the emulator's variants, `perf`,
`codegen-variant`) goes through one helper that gives a listed image the
flags and a target folder of its own, and `cross` refuses an image on both
the Thumb and the frame-pointer lists. The record layout was read from
`rp-crash` built that way, not assumed: every function, leaves included,
pushes `{.., r11, lr}` and points r11 at the saved r11, the return address
4 bytes above. In a frame-pointer build `_start` clears r11 before
`rp_main` (the chain's end) and every fault entry keeps the faulting code's
r11 (`rp_fault_fp`; the FIQ entry the user bank's, as FIQ has its own r11);
in every other build those pieces are empty, so no instruction moves. The
walk (`hal/pxa27x/src/backtrace.rs`) reads only word-aligned records wholly
in SDRAM, through `try_read`, each above the one before, at most 16, on the
first crash of a run only; its line goes to the serial port and the run log
(so the loader's crash screen and `crashlog.txt`), not the two-line crash
box: `CHAIN <call address>... start | cut | broken: fp <value> <why>`. Every
may-omit image prints `no frame chain (built may-omit)` instead: 31 bytes of
text and 36 to 68 bytes in `rp_image`'s report, nothing else changed
(compared function by function against the build before). `cargo xtask
symbols <image> <addr...>` names the addresses with the toolchain's
`llvm-nm` and `llvm-objdump --line-numbers`: the `llvm-tools` component has
no `llvm-symbolizer`, the tool this plan named. `emu-crash` follows all six
of `rp-crash`'s crashes (the panic, the data and prefetch aborts, the
undefined instruction, a system call with no handler, the run limit) to
`_start`; `emu-run-limit` requires the no-chain line from the may-omit
`rp-hang`. The chain's clean end is checked from the image, not the run
(`_start`'s `mov r11, #0` right before `bl rp_main`: the emulator's loader
happens to hand over r11 = 0, so a run alone can't tell). `RP_MAY_OMIT=1`
builds a listed image may-omit anyway (the X51v check of the no-chain
line: `RP_MAY_OMIT=1 cargo xtask codegen-variant rp-crash may-omit`).
Building it found quality audit C61 (the run limit's and the
SWI's reports carried no SPSR). **Proven on the X51v 2026-10-22** (the
owner present): the frame-pointer build's panic chained `panic_fmt`, `main`
at main.rs:150, `__rp_image_main`, `rp_main`, `_start` (the emulator's
chain, address for address); the may-omit build said only `no frame chain
(built may-omit)`. Both came back to the loader, their reports on the card.

