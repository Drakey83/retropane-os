# ADR-0010: Threads, scheduling and the system-call boundary

- **Status:** Accepted (2026-10-11, by the project owner); every rule
  implemented and live in `boot/threads` (2026-10-11, see Implementation
  status), moving into the kernel image next
- **Date:** 2026-10-11
- **Amended:** 2026-10-15: the time slice, measured on the device: a 20 ms
  quantum and a 1 ms wake slice replace CE's 100 ms ("Amendment: the
  time slice")
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

   > **Amended (2026-10-15):** the quantum is 20 ms, and a thread that
   > wakes gets the CPU within a 1 ms wake slice of a busy peer of its
   > priority, both measured on the device ("Amendment: the time slice").
   > Priorities, strict preemption and inheritance are unchanged.
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
| 🗄️ Legacy support | CE priorities and quantum (quantum since amended), CE wait semantics, priority inheritance, the KData page for inlined SDK macros |
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

## Amendment: the time slice (2026-10-15)

### In plain words

CE gives each thread 100 ms before another thread **of the same
priority** gets a turn. A touch or a frame tick wakes the program's
interface thread, but if any other program at the normal priority is busy
(a sync, a decoder, a game in the background), the interface thread
waits for the rest of that program's 100 ms. On the X51v that was up to
**102 ms from touch to screen**, and a 60 Hz animation drew 5 frames of
35. The 100 ms was copied from CE without measurement
(docs/research/imitation-audit.md, "Scheduler and system calls").

RetroPane now does two things:

1. **The wake slice.** A thread that wakes doesn't wait out a busy peer's
   quantum: it goes first, within 1 ms. To stay fair, the busy peer still
   gets at least as much CPU as the woken thread used in its last burst,
   so a thread that sleeps often can never take more than an equal share
   (what CE's round-robin gives). A thread whose last burst was a whole
   quantum or more is treated as busy and queues as in CE.
2. **A 20 ms quantum** instead of 100. This matters only for programs
   that never block (an old game's draw loop): they get the CPU back
   within 25 ms of a busy peer instead of 105.

Nothing changes between different priorities: a more urgent thread still
always runs first, priority inheritance is as before, and CE's numbers
mean what they meant.

### Measured (`rp-quantum`, the X51v, 2026-10-15)

`cargo xtask load rp-quantum`. For each policy, a busy user thread runs
at the interface thread's priority (251) all along. **Touch:** a timer
interrupt at 20-80 ms gaps stands in for a touch; a driver thread
(priority 100) passes it on; the interface thread draws a 5 ms frame
(calibrated passes over a 16 KiB buffer). **Animation:** a 5 ms frame
every 60 Hz period, for 600 ms. **Never-blocking loop:** 5 ms frames back
to back. **Busy pair:** two busy threads (16 KiB each, so together they
overflow the 32 KiB data cache) against one alone. Times in ms:

| Policy | Touch to screen, median / worst | 60 Hz frames of 35: drawn, on time | Never-blocking loop: worst gap | Busy pair's work |
|---|---|---|---|---|
| CE 100 ms (as built until now) | 73.5 / 101.7 | 5, 0 | 105.1 | 99.8% |
| CE 50 ms | 31.9 / 55.2 | 10, 0 | 55.1 | 99.8% |
| CE 20 ms | 25.1 / 25.1 | 24, 12 | 25.1 | 99.4% |
| CE 10 ms | 15.1 / 15.1 | 35, 35 | 15.1 | 98.9% |
| 100 ms + wake slice 0 | 5.1 / 5.1 | 35, 35 | 105.1 | 99.9% |
| 100 ms + wake slice 1 ms | 5.1 / 5.1 | 35, 35 (6.1 each) | 105.1 | 99.8% |
| **20 ms + wake slice 1 ms (chosen)** | **5.1 / 5.1** | **35, 35 (6.1 each)** | **25.1** | **99.5%** |
| 10 ms + wake slice 1 ms | 5.1 / 5.1 | 35, 35 | 15.1 (half the gaps) | 98.9% |

- With the wake slice, touch to screen is the frame's own 5 ms at every
  quantum: no waiting at all. The driver thread wakes the interface
  thread, which goes ahead of the busy one the driver preempted.
- **What a switch really costs:** the pair loses ~100 µs per quantum at
  every length (0.5% of 20 ms, 1.1% of 10 ms). The register switch is
  13 µs of that (`thread_switch`); the rest is the two working sets
  refilling the data cache and writing back dirty lines. Short quanta
  cost cache, not scheduler time.
- **The wake slice's own cost:** `thread_switch` (event ping-pong, 2,000
  switches) went from 86,708 to 88,868 ticks, 13.3 to 13.7 µs per switch
  (+0.33 µs; the same in two runs each; an inlined variant was no faster
  and 600 B bigger). The kernel image grows by 424 B of code and 16 B of
  RAM: the burst counter is 16 bits in 64-tick units and fills padding in
  the thread record.
- **A class fixed on the way.** Tickless, a thread alone at its priority
  runs on past its quantum's end. When a more urgent thread preempted it,
  it got a fresh quantum, so the next thread to wake waited the whole of
  it (CE mode's 73.5 ms median is that worst case; real CE ticks every
  1 ms and would average about half a quantum, ~55 ms). With the wake
  slice it now keeps its place with nothing left (found on the device:
  touch 10.2 ms at 20 ms + 1 ms before the fix, 5.1 after).
- The emulator runs the same image (`cargo xtask emu-quantum`, also in
  `cargo xtask emu`) to check that every policy runs. Its timing is the
  host's and it has no cache model, so its numbers are not the device's.

### Why these numbers

- **Wake slice 1 ms, not 0:** with a busy peer, every wake of an
  equal-priority thread hands the CPU over. The slice bounds a storm of
  wakes to ~1,000 hand-overs a second (at ~100 µs each with big working
  sets, ~10% of the CPU in the worst storm; with no slice it would be
  unbounded). The price is up to 1 ms on a frame tick (6.1 ms instead of
  5.1 of a 16.7 ms frame); touches pay nothing (they come through a
  driver thread).
- **20 ms, not 10:** both keep touch and animation perfect with the wake
  slice. They differ only for programs that never block: 10 ms gives a
  worst gap of 15 ms instead of 25, for twice the switch cost (1.1% vs
  0.5%), and still leaves half the gaps at 15 ms (5 ms frames split
  unevenly).
- **Not a foreground or input-owner boost:** the kernel would need to
  know the window manager's foreground (coupling) and change a thread's
  priority (CE semantics), and the wake slice already gets the input
  owner to the CPU in its frame's own time. A longer quantum for the
  foreground's never-blocking games is a possible later step once the
  window manager exists, if a game needs more than half the CPU against
  a busy peer.
- **Not a shorter quantum alone:** CE 10 ms gets animation right, but
  every touch still waits ~10 ms more, and it costs 1.1% always.

### The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | `rp-quantum` measures every policy on the device in ~20 s; host tests pin each rule (the wake slice, fairness, the driver path, a lone thread's quantum, CE mode); `thread_switch` tracks the scheduler's own cost |
| ✨ Modernize | Sleeper-fair wake preemption, as modern schedulers do (Linux's EEVDF lag, Windows' wake boosts), still tickless and O(1) |
| 🗄️ Legacy support | CE's 256 priorities, strict preemption, inheritance and FIFO order among busy peers are unchanged; `set_wake_slice(None)` restores CE's plain round-robin if a program turns out to need it |
| ⚡ Better / faster | Touch to screen 102 ms worst → 5.1 ms (the frame itself); 60 Hz animation next to a busy program 0 of 35 frames on time → 35 of 35; never-blocking games' worst gap 105 → 25 ms; for 0.33 µs a switch and 0.3% of a busy pair's work |

### Hardware limit or choice?

- The 100 ms quantum: **CE's choice**, now replaced.
- ~100 µs per switch with big working sets: **silicon** (32 KiB VIVT data
  cache, SDRAM line fills). It is why the quantum stays at 20 ms rather
  than 1-2 ms.
- The 13 µs register switch: **ours to improve** (ADR-0010's target is
  under 2 µs; items 67 and 80).

### Not yet

- CE's per-thread quantum (`CeSetThreadQuantum`, 0 = run to completion)
  isn't built; when the CE personality adds it, a thread with quantum 0
  must never be cut by the wake slice.
- Live wherever `kernel/sched` runs with its defaults: the kernel image
  (`boot/kernel`) and `rp-threads`, whose 13 tests all pass on the device
  with it.

## Open questions

- 2 KiB or 4 KiB kernel stacks (measure).
- How often WM apps use iWMMXt (decides how hot the lazy-switch trap is).
