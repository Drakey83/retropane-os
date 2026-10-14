# ADR-0010: Threads, scheduling and the system-call boundary

- **Status:** Accepted (2026-10-11, by the project owner); every rule
  implemented and live in `boot/threads` (2026-10-11, see Implementation
  status), moving into the kernel image next
- **Date:** 2026-10-11
- **Last verified:** 2026-10-13 (against commit 3158b26)

> **Correction (note added 2026-10-13):** not every rule is implemented
> (see the as-built note under Decision), and the scheduler has since
> moved into the kernel image (`boot/kernel`, step 6: `rp_kernel_sched`,
> `user::set_syscall_handler`, `rt::set_preempt_hook`).

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

> **As built (note added 2026-10-13):** rules 1-3, 5 and 8 are built as
> written (`kernel/sched`, `rp_hal_pxa27x::{thread,user,iwmmxt}`). The
> rest differ so far. Rule 4: kernel stacks are 4 KiB chunks taken
> back to back off the RAM pool (`boot/threads` and `boot/kernel`
> `kstacks`), with no unmapped guard page. Rule 6: no system call takes a
> pointer yet (`boot/kernel` `on_syscall`: `SYS_CALL`..`SYS_REPORT`), so
> no region check exists. Rule 7: `kernel/sched` has events, mutexes and
> single-object waits (`Scheduler::wait` on one `Eid`); no semaphores,
> critical sections or multi-object waits, and the per-process handle
> tables built are ADR-0016's port handles (`kernel/ipc`, 32 per
> process). Rule 9: user stacks are fixed 4 KiB buffers in
> `boot/threads`; in `boot/kernel` one zero-fill page per thread and call
> depth in a 256 KiB region (`stack_top`, `STACK_REGION`), with no guard
> pages. The kernel image runs the scheduler but maps no KData page and
> doesn't enable lazy iWMMXt (neither appears in `boot/kernel`).

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

## Implementation status

- **2026-10-11:** the scheduler core (`kernel/sched`: CLZ selection,
  round-robin with CE/NT preemption semantics, tickless deadlines, sleep
  queue, events; all-zero when empty so it lives in `.bss`), the context
  switch and thread start (`rp_hal_pxa27x::thread`), and the one-shot alarm
  (`rt::set_alarm`). Live in `boot/threads`: ping-pong between equal-priority
  threads, a sleeper waking on time, a tickless idle thread.
- **User mode and system calls (2026-10-11):** `rp_hal_pxa27x::user`: SWI
  entry saves the whole user context as a `Frame` on the thread's kernel
  stack, the handler may block or switch, exit restores it; `enter_user`
  drops a thread into user mode; a safe user-side `syscall` stub (number in
  `r7`). Live in `boot/threads`: two user threads print and yield through
  system calls (`ABABAB`), every frame checked to come from user mode.
- **Preemption (2026-10-11):** IRQs enter SVC mode with a full frame on the
  thread's kernel stack; quantum expiry switches user threads that never
  yield (`USER preempted OK`).
- **Mutexes with priority inheritance (2026-10-11):**
  recursive, CE semantics; waiters lend priority along
  chains of owners; most urgent waiter first; timed-out waiters withdraw
  their boost; an exiting owner hands its mutexes on. 9 host tests and a
  live priority-inversion test (`THREADS mutex inheritance OK`).
- **User faults in the thread's context (rule 5):** user-mode aborts build
  a system-call `Frame` on the thread's kernel stack and call
  `user::set_user_fault_handler`'s handler, which may block, fix and resume,
  or end the thread (`USER fault contained OK`).
- **Lazy iWMMXt (rule 3):** `iwmmxt::enable_lazy` turns media-unit access
  off (CPAR) for all but its owner and patches its own undefined-instruction
  entry into the vector page; a first use saves the owner's 152 bytes and
  loads the new thread's. 8 shared state slots (~1.2 KiB), not 128
  (`USER iWMMXt lazy switch OK`, mutation-checked). `wCSSF`/`wCASF` are
  write-1-to-clear, so restored as far as the hardware allows.
- **Interrupts as waitable objects (rule 7):** `rt::set_signal_hook` gets
  each claimed line that fires (after the first-level handler masked it);
  the kernel sets the event its interrupt-service thread waits on, and the
  thread services the device and calls `IrqLine::done`. Live with OS timer
  channel 2 (`soc::SpareTimer`, `THREADS interrupt thread OK`). Found on the
  way: the idle thread slept in `wait_tick`, which only a timer tick ends,
  so a device interrupt never got its thread scheduled; it now sleeps in
  `rt::idle_until_interrupt` (any interrupt ends it, with the "anything
  ready?" check made with IRQs masked).
- **The KData layout (rule 8), measured:** a scan of all 500 modules in the
  owner's ROM (docs/research/kdata.md) shows programs read `+0x000` (the TLS
  pointer: 89 modules) and `+0x008` (the current thread ID: MFC/ATL
  runtimes). `kernel/sched::kdata::switch_to` writes those words (and
  `+0x00C`, the process ID).
- **KData wired into the switch (rule 8, 2026-10-11):** every switch to
  another thread writes the next thread's view (`Scheduler::view`: three
  words) into the page. `rt::take_kdata_page` splits the bring-up map's top
  MiB into 4 KiB pages (a coarse table at `0xA80F2000`): the vector page,
  now kernel-only (the section had left it and the rest of SDRAM's first MiB
  user-writable), and KData at `0xFFFFC000` (frame `0xA80F1000`), user
  read-only. The kernel writes it through that one user address, so the
  VIVT cache never holds a second alias and a switch needs no cache
  maintenance. Live in `boot/threads` (`USER KData OK`): two user threads
  read the TLS pointer and thread ID from `0xFFFFC800` as the inlined macros
  do and keep their own value in TLS slot 0 across switches (6 checks
  against the kernel's view), and a user write to the page faults and
  changes nothing; both checks were mutation-tested (no write on the switch;
  a user-writable page). Choices where the ADR and kdata.md left room,
  the simplest consistent with the measured layout: the thread ID is the
  scheduler's thread number (1-128, never 0, as handles never are); the
  process ID is the process slot; each user thread's TLS array is the top
  256 bytes of its user stack with 8 words below slot 0 for our `coredll`
  (CE's uses -0x14..-0x0C), zeroed at creation, as CE places it; kernel
  threads have none (pointer 0). The kernel image keeps its own page tables
  (`PageTables`); it maps the same two pages (`mmu::high_pages`' layout)
  when the scheduler moves into it. Not yet checked on the device: the
  first coarse table a test image installs.

> **Correction (note added 2026-10-13):** since checked: all 13
> `rp-threads` tests, KData included, pass on the device (docs/perf.md,
> "Threads on the real X51v", 2026-10-12).

## Open questions

- 2 KiB or 4 KiB kernel stacks (measure).
- How often WM apps use iWMMXt (decides how hot the lazy-switch trap is).
