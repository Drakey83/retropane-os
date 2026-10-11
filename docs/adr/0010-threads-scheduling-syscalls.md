# ADR-0010: Threads, scheduling and the system-call boundary

- **Status:** Accepted (2026-10-11, by the project owner)
- **Date:** 2026-10-11

## Context

Phase 2 needs processes and threads (DESIGN §5.1: CE's 256 priorities,
priority inheritance, `WaitForMultipleObjects` semantics, interrupt service
threads). The pieces they sit on now exist: the VM with FCSE slots and real
page faults (ADR-0008), interrupt ownership where drivers service their
devices in threads (ADR-0007), and a single core with no atomics beyond
`SWP`.

Constraints and facts that shape the design:

- **One core, 624 MHz, VIVT caches, FCSE:** a process switch is a PID change
  plus a DACR write; nothing must flush caches or TLBs.
- **ARMv5 has `CLZ`:** finding the highest set bit is one instruction.
- **iWMMXt state is large** (16 × 64-bit registers plus control) and most
  threads never touch it. The XScale's Coprocessor Access Register can make
  the first use trap.
- **WM apps read a fixed "KData" page.** CE's public SDK headers (`kfuncs.h`)
  define macros that read `0xFFFFC800` directly (thread-local storage
  pointer, current thread and process IDs), so that page must exist,
  user-readable, with CE's layout, updated on every switch.
- **Page faults may need I/O** (an image view page from SD), so a faulting
  thread must be able to sleep inside the kernel.

## Decision

1. **O(1) scheduling with `CLZ`.** 256 priority levels (CE numbering: 0 is
   highest; apps default to 251). A 256-bit ready bitmap with an 8-bit summary
   finds the highest ready priority in two `CLZ`s; each level is a FIFO for
   round-robin within a priority (CE's default quantum, 100 ms, per thread).
2. **Tickless.** No periodic tick: the OS timer is programmed for the next
   event (quantum end or earliest sleeper). An idle system sleeps until an
   interrupt or a deadline, which is the dominant power state of a handheld.
3. **Cheap switches.** Same-process thread switch: registers only. Process
   switch: PID and DACR (ADR-0008's domains), still no cache or TLB work.
   **iWMMXt state is switched lazily:** coprocessor access is disabled for a
   thread that hasn't used it; its first iWMMXt instruction traps, and only
   then is state saved and restored. Context-switch time is a tracked
   benchmark (target: well under 2 µs on device).
4. **A non-preemptive kernel with per-thread kernel stacks.** Kernel code runs
   to completion or until it blocks; the first-level interrupt handler only
   marks threads ready (ADR-0007), and rescheduling happens on the way back
   to user mode. No locks inside the kernel on one core. Each thread has a
   4 KiB kernel stack (one frame, with an unmapped guard page) so it can block
   in a page fault or a wait; measured stack depth decides if 2 KiB will do.
5. **User faults run in the thread's context.** An abort from user mode saves
   the user registers, switches to the thread's kernel stack and calls the
   VM's fault handler there, which may block for I/O. (Today's kernel image
   resolves kernel-mode faults in abort mode; that stays for faults taken in
   the kernel.)
6. **System calls by `SWI` with the call number in a register** (r7), so the
   kernel never reads the instruction from user memory. Arguments in r0-r3
   and the user stack. Pointer arguments are checked against the caller's
   regions, and writes go through `prepare_user_write` (ADR-0008 rule 8). Only
   our own `coredll` makes system calls; apps call `coredll` as they always
   have.
7. **Kernel objects behind bounded per-process handle tables**: threads,
   processes, events, mutexes (priority inheritance, CE semantics),
   semaphores, critical sections (user-mode fast path, kernel only when
   contended), and waits on up to 64 objects (`MAXIMUM_WAIT_OBJECTS`). An
   `IrqLine` becomes a waitable object, so interrupt service threads are
   ordinary threads.
8. **The KData page** at `0xFFFFC000` (user read-only, kernel written),
   laid out as CE's public headers expect, updated on each switch: so
   `GetCurrentThreadId`, `TlsGetValue` and friends work as inlined by old
   apps.
9. **User stacks are demand-paged regions** with a guard page below (CE's
   default: 64 KiB reserved per thread, committed on touch), so a thread costs
   only the stack pages it uses.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Context-switch, interrupt-to-thread and syscall round-trip benchmarks; kernel stack depth measured to shrink stacks |
| ✨ Modernize | Tickless timing, lazy FPU-class state, threaded interrupt handlers, checked user pointers |
| 🗄️ Legacy support | CE priorities and quantum, CE wait semantics, priority inheritance, the KData page for inlined SDK macros |
| ⚡ Better / faster | O(1) scheduling in two instructions, no cache or TLB work on any switch, no iWMMXt save for threads that never use it, a CPU that sleeps instead of ticking |

## Alternatives considered

- **Periodic tick (CE's 1 ms):** simpler, but wakes the CPU 1000 times a
  second for nothing.
- **A preemptive kernel:** lower worst-case latency, but needs locking
  everywhere on a design whose kernel paths are short; interrupt work already
  runs in threads (ADR-0007).
- **One kernel stack with continuations (Mach-style):** saves memory per
  thread, but makes every blocking point hand-written state machines.
- **CE's trap-address system calls** (jumps to reserved addresses that fault):
  needed only for apps that bypass `coredll`; can be added in the personality
  layer if any are found.

## Consequences

- The kernel image grows a scheduler, a timer queue, handle tables and the
  user-mode entry/exit path; the HAL grows `SWI` and user-abort entry paths
  and lazy coprocessor switching.
- The KData layout must be taken from the public SDK headers and checked
  against real apps.

## Open questions

- 2 KiB or 4 KiB kernel stacks (measure).
- How often WM apps use iWMMXt (decides how hot the lazy-switch trap is).
