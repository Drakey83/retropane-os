# RetroPane OS — Contributor Brief

---

You are contributing to RetroPane OS: a clean-room, open-source reimagining of
Windows Mobile for the Dell Axim X51v, written in Rust (`no_std`). It is NOT a
Windows Mobile clone. No Microsoft code, no Microsoft UI artwork, no
pixel-perfect imitation. Study Windows Mobile only for compatibility: the APIs
and behaviors that WM5/WM6.x apps depend on. Everything else, the
architecture, the internals, the look and feel, should be designed as if
building a new OS in 2026 that happens to run on 2005 hardware.

## Prime directive: iterate, don't imitate

Every change must advance at least one of these four lenses and set back none:

1. **Iterate:** it is measured, tested, and improvable. If there is no test or
   benchmark capturing it, the next iteration cannot beat it.
2. **Modernize:** it brings 2026 standards, security, and UX, not 2005 ones.
3. **Legacy support:** unmodified WM5/WM6.x ARM apps still run, and users'
   muscle memory still works.
4. **Better/faster:** it uses the hardware more fully than the original did.

At every new step (an ADR, a crate, a driver, an image, a protocol), answer
three questions in the plan and again in the summary (the owner's standing
request, 2026-10-11):

- Am I being clever and making this better, or just reverse engineering it
  to work the way it did?
- Am I keeping the file size down? (Measured size against the budget.)
- Am I doing anything to modernize it, wherever and whenever I can?

If an answer is "no", say so, and either fix it or explain why not.

"At least one lens" is the floor for merging, not the goal. The owner's
standard (2026-10-12): for every part, look for every way it can beat the
original (robustness, speed, size, safety, standards, user experience,
testability) and do as many as make sense; one token improvement doesn't
meet it. When a part is finished, report what was done to iterate and to
modernize it.

**Hard rule: imitation is the last resort** (the owner, 2026-10-13). How
Windows Mobile, Dell's drivers or the 2005 design did something is a
*finding*, never a *decision*. They didn't have today's tools, languages
or knowledge. For every part, and every decision inside it:

1. **Hardware limit or choice?** Ask it of every inherited behaviour. If
   the silicon itself forces it, say so with the evidence. If it was a
   software choice, it is open to replacement.
2. **Survey the better ways, bounded** (owner, 2026-10-13): name the 2-4
   serious alternatives (faster, more modern, cleverer, more efficient,
   and how the device's parts can work *together*: CPU, the 2700G's 3D
   core and display controller, DMA, the buses, power), measure where
   measuring is cheap, and time-box the exploration to the size of the
   decision. Stop when the silicon rules the others out, or a measurement
   or clear argument shows which is best. A survey that keeps generating
   alternatives has lost its bound.
3. **Then decide, imitation included**, and write down the alternatives
   rejected and why ("Hardware limit or choice?" and "Alternatives
   considered" in every ADR, plan and hardware write-up). The 2005
   engineers had the datasheets, schematics and Intel's engineers; their
   way is sometimes simply right. It has to be *beaten by a measurement*,
   not presumed worse: the stock-WM baseline (docs/perf.md) is the bar.

Example: Windows Mobile hands its screen fills to the 2700G (measured,
2026-10-13: the CPU stays free while fills finish at ~190 MB/s, where the
CPU's own writes reach ~41 MB/s). Dell's driver got that right, and
RetroPane, which fills on the CPU, is the one behind. Meanwhile
RetroPane's tickless timers beat WM's 1 ms tick ~60x: the inherited
choice lost there.

**Security is a design input, not a later pass.** RetroPane will be on the
internet. Read [`security.md`](security.md): every ADR gets a Security
section, every parser of outside input (USB, network, files, images) checks
every length and offset, never panics on malformed input, and has tests
with hostile input. Development-only exposures (the code-running loader)
are listed there and never ship.

**The emulator follows the device.** Every fact learned on the real X51v
that a model could reproduce goes into the emulator fork with a `cargo xtask
emu` boot that fails without it; what can't be modelled is listed with the
reason in `docs/hw/emulator.md`. The emulator is only trusted for what that
table says it models.

## The box (non-negotiable constraints)

- Intel PXA270 (XScale ARMv5TE), single core, 624 MHz. No Thumb-2, no atomics
  beyond `SWP`.
- **No FPU.** Fixed-point everywhere it matters (graphics, audio, layout).
  iWMMXt SIMD for hot paths.
- **64 MB RAM.** Every allocation is judged against this number.
- 32 KB I + 32 KB D caches, virtually indexed (VIVT). Avoid cache flushes the
  way you avoid allocations: design around them.
- Intel 2700G GPU (MBX Lite), 16 MB VRAM. GPU-composited UI, 2D blitter, DMA
  everywhere. Hardware acceleration is the default path, never optional.

## Architecture law

- Layers talk through interfaces, never registers. Only the HAL touches
  hardware. Everything above it must be testable on a PC.
- Layer order: HAL < Core (kernel, drivers, boot) < System (services, GWES) <
  Personality (CE compat) < Apps. Dependencies flow downward only. This is
  enforced by `cargo xtask lint`; do not work around it.
- `forbid(unsafe_code)` everywhere outside the HAL. Inside the HAL, every
  `unsafe` block gets a `SAFETY` comment explaining the invariant. No
  exceptions.
- Drivers are generic over HAL traits (`RegisterBus` etc.), never over
  concrete boards. If a driver can't be unit-tested against the mock bus, it's
  wrong.

## Concerns to honor (from code review)

1. **Design for the layers that don't exist yet.** The kernel, GWES, and CE
   personality are coming. When you add to the HAL or a driver, ask: will this
   trait still be ergonomic when the kernel depends on it? Do not over-fit the
   design to drivers-only usage.
2. **Plan the interrupt story now.** `RegisterBus` reads take `&mut self`,
   which will fight you when an ISR and main-line code share a peripheral.
   Every driver that will ever see interrupt context needs an ownership story
   (take-once statics, critical sections, or split bus handles) before the
   interrupt controller lands, not after. *(Settled in
   [ADR-0007](adr/0007-interrupt-ownership.md): no driver code runs in
   interrupt mode.)*
3. **Never silently discard.** Bitfield inserts, truncations, and fallbacks
   must fail loudly in debug (`debug_assert`) or be checked at compile time.
   Silent wrong values in driver code are unacceptable.
4. **Docs stay in sync with code.** If behavior changes, `DESIGN.md`, the
   ADRs, or the module docs change in the same commit. Stale docs are a bug.
5. **Test before trust.** Every module ships with unit tests. Every hardware
   fact ships with a test that would fail if the fact were wrong. The mock bus
   exists so driver tests assert exact register sequences; use it.

## Performance discipline

- Measure from the first boot. `cargo xtask bench` runs deterministic
  benchmarks against the committed baseline in `bench/baseline-emulator.json`;
  [`docs/perf.md`](perf.md) is the before/after log of every optimization. A
  performance regression blocks the merge like a failing test.
- Smooth and responsive on meager hardware is a design goal, not a hope.
  Prefer: DMA over CPU copies, fixed-point over float emulation, GPU
  composition over software blits, lazy work over eager work, static
  allocation over heap where it matters.

## Legacy without legacy thinking

Compatibility is a compatibility layer, not a design philosophy. The CE
personality exposes the Win32-for-CE surface so old apps run; the OS
underneath owes nothing to 2005 conventions. When in doubt, ask: "What would
this look like if the hardware were new and only the apps were old?" Then make
that run at 624 MHz.

## Definition of done

`cargo xtask ci` is green (fmt, clippy, architecture lint, unit tests, X51v
cross-build), docs updated, benchmarks not regressed, and the change advances
at least one lens without setting back any other.
