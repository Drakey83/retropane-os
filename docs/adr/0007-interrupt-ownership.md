# ADR-0007: Interrupt ownership: no driver code runs in interrupt mode

- **Status:** Accepted (2026-10-11, by the project owner)
- **Date:** 2026-10-11

## Context

Drivers own their registers through `RegisterBus`, whose methods take
`&mut self`. That works only while one context touches a peripheral. Once
interrupts are handled, the obvious design (driver code in an interrupt
handler, sharing the device with main-line code) puts two owners on one bus.
That means critical sections, interior mutability in every driver, and
races that unit tests can't see.

What exists today: the HAL's first-level IRQ handler (`hal/pxa27x::rt`)
serves the kernel tick itself, and masks and records any other source
(`rt::unclaimed_irqs`). No driver runs in interrupt context yet. This ADR
settles the model before the first driver needs an interrupt.

Constraints: one core, ARMv5 with no atomics beyond `SWP`, and a kernel that
will schedule threads at 256 priorities (DESIGN §5.1). Windows CE drivers use
an interrupt service thread (IST) model (`InterruptInitialize`,
`WaitForSingleObject`, `InterruptDone`), and the CE personality must be able
to offer it.

## Decision

**Interrupt mode runs only kernel code, and that code never touches a
driver's device.**

1. **First-level handler (kernel/HAL, interrupt mode):** reads the pending
   sources, *masks* each one at the interrupt controller, marks it signalled
   and wakes whoever waits on it. It doesn't read or write the device. Fixed,
   tiny, the same for every source.
2. **Drivers handle interrupts in their own thread.** A driver gets an
   `IrqLine` token for its source, handed out once from the board description
   (like every other `take_*` handle). It calls `line.wait()` (sleep until
   signalled), services its device through its own `RegisterBus`, then
   `line.done()` (unmask). The driver's thread is the device's only owner, so
   `&mut self` holds by construction: no critical sections, no interior
   mutability, and drivers stay fully testable on the mock bus.
3. **Priority, not interrupt mode, gives latency.** A driver thread's priority
   decides how quickly it runs after its interrupt. Interrupt-to-thread latency
   becomes a tracked benchmark (performance suite v1).
4. **High-rate devices interrupt per buffer, not per byte.** SSP, audio, SD
   and the 2700G move data by DMA; the interrupt arrives when a buffer
   completes. That keeps interrupt rates low enough for the thread model
   everywhere.
5. **The only exceptions are kernel-owned:** the scheduler tick (OS timer) and,
   later, DMA-channel bookkeeping. Their state is owned by the kernel alone,
   written only in interrupt mode, and read with single aligned accesses.
6. **Unowned sources are masked and reported, never silently dropped**
   (already the case: `rt::unclaimed_irqs`).
7. **GPIO edge detection belongs to the interrupt layer, not to drivers.** On
   the PXA27x, GPIOs 2 to 120 share one interrupt (GPIO_X), so finding which
   pin fired means reading the GPIO edge-detect status. The kernel/HAL owns
   the edge-detect registers (rising/falling enables and status) as an
   extension of the interrupt controller, and hands out one `IrqLine` per pin.
   Drivers keep pin direction and level through their `GpioPin` handles, but
   never the edge-detect registers.

Before the scheduler exists, bare-metal images use the same API: `wait()`
idles the CPU until the source is signalled. Driver code written now doesn't
change when threads arrive.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Interrupt-to-thread latency and interrupt rate per device are benchmarks; DMA-per-buffer is measured against per-byte interrupts |
| ✨ Modernize | The split modern kernels converge on (threaded interrupt handlers), plus ownership enforced by Rust's type system instead of by convention |
| 🗄️ Legacy support | Maps directly onto CE's IST API for the CE personality: `InterruptInitialize` ↔ taking an `IrqLine`, `InterruptDone` ↔ `done()` |
| ⚡ Better / faster | Tiny fixed first-level handler (shorter interrupt-disabled windows than driver ISRs); no locking in drivers; fewer interrupts thanks to DMA |

## Alternatives considered

- **Driver code in interrupt handlers with critical sections:** familiar, but
  every driver needs interior mutability and careful locking, and the bugs
  are timing-dependent and invisible to host tests.
- **Split bus handles** (an "interrupt half" and a "thread half" of each
  device): workable, but every driver must decide which registers belong to
  which half, and many registers (status plus control) are needed by both.
- **CE-style installable ISRs** (driver-supplied code in interrupt mode): the
  flexibility isn't worth bringing driver code back into interrupt mode.

## Implementation (2026-10-11)

`hal/pxa27x::soc::Dispatch` holds claims and signals and runs the
first-level logic (host-tested against the mock bus); `soc::GpioEdges` owns
the GPIO edge registers. `rt::take_irq(n)` / `rt::take_gpio_irq(pin, edge)`
hand out `IrqLine`s with `take`, `wait` and `done`; `rt::wait_any_signal`
sleeps until any line fires or the tick advances, and `rt::stop_tick` lets an
idle image stop the tick entirely. `boot/hello` uses them: keypad and pen-down
interrupts wake it, and the tick runs only while a button or the pen is held.

## Consequences

- `RegisterBus` keeps `&mut self`; no trait changes are needed for interrupts.
- The HAL gains `IrqLine` (take-once per source) with `wait()`, `done()` and
  `is_signalled()`, plus the first-level handler logic. Host tests cover the
  handler's mask-and-signal behaviour against the mock interrupt controller.
- A very latency-sensitive device that can't use DMA would need an exception.
  None is known on the X51v. Checked for the touch screen, the most likely
  candidate: its only interrupt is pen-down (a GPIO). Sampling is polled SPI
  from the driver's thread, 9 bytes at 2.6 MHz or about 28 µs per sample, on a
  timer while the pen is down (about 0.6% CPU at 200 Hz). There are no
  per-sample interrupts, so the touch screen fits the thread model as is.
