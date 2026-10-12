# ADR-0016: Processes, the driver host, and IPC

- **Status:** Accepted (2026-10-12, by the project owner) with decision 5
  as amended: a driver host per driver class from the start (the owner
  chose the more isolated variant)
- **Date:** 2026-10-12

## Context

DESIGN §5.1 asks for processes, a privileged **driver host** (as CE's
`device.exe`) that can restart a crashed driver without a reboot, and
"message-passing IPC for service calls, plus shared memory for bulk data".
What exists: FCSE slots and real page faults (ADR-0003, ADR-0008), threads,
the O(1) scheduler, priority inheritance and `SWI` system calls with
bounded handle tables (ADR-0010), interrupt service threads (ADR-0007), the
PE loader (ADR-0009). Drivers today are libraries linked into test images.

Facts that shape the design:

- **One core, VIVT caches, FCSE.** A process switch is a PID and DACR write,
  no cache flush (ADR-0010 rule 3). But the same physical page seen at two
  virtual addresses is two sets of cache lines: **shared memory between
  slots aliases** and goes incoherent unless every access is fenced by cache
  maintenance. CE avoided this by putting shared things in a global region
  at one address.
- **CE's own calls migrate threads.** A CE API call into another process
  (PSL: GWES, the file system, `device.exe`) runs on the caller's thread,
  which switches its process context for the call. CE drivers rely on it:
  they see the caller's thread and process, run at its priority, and are
  written reentrant.
- **WM binaries call by import** (`coredll`), which the CE personality
  (Phase 4) turns into RetroPane calls: whatever IPC is chosen sits under
  every CE API call, so a call's cost is the OS's cost.
- **Security matters** (docs/security.md): the device will be on the
  internet; a compromised app must not reach drivers or other processes'
  memory it wasn't given.

## Decision (proposed)

1. **A process is a slot plus a handle table.** Up to 63 processes (FCSE
   PIDs 1-63, 32 MB each), created from a PE image (ADR-0009) with a first
   thread. Its pages are visible to it only (per-process page permissions
   through ARM domains: a process's slot is in its own domain, enabled in
   DACR while it runs), improving on CE5, where any process could read any
   slot.
2. **Service calls migrate the calling thread.** A server registers entry
   points on a **port** (a kernel object, reached only through a handle). A
   call (`SWI`, port handle, operation, up to 6 words in registers) switches
   the caller's thread into the server's slot and domain, on a per-thread
   server stack, runs the entry point, and switches back with up to 6 words
   of result: no scheduler decision, no second thread, the caller's priority
   (so no priority inversion through a server), a cost of two context
   writes. CE compatibility comes for free (drivers see their caller).
   Servers are reentrant (Rust's `Sync` services; locks where they share
   state). A server that faults during a call fails that call (the caller
   gets an error) and is restarted by its supervisor; a call can carry a
   timeout.
3. **Bulk data moves, it isn't shared.** A call may carry whole pages: they
   leave the caller's slot and appear in the server's (or back), cleaned
   from the cache on the way (a 4 KiB page is 128 lines). One owner at a
   time: no aliasing, no lifetime bugs, zero copies, the shape Rust's
   ownership already has. Buffers two parties use at once (framebuffers,
   audio rings, DMA buffers) live in the global shared region at one
   address, mapped to their one user process, or to a device.
4. **Notifications are events.** Asynchronous signals (data ready, a card
   inserted) use the existing waitable objects (ADR-0010), not message
   queues: nothing to fill up, nothing to drop.
5. **A driver host per driver class, each driver a supervised service in
   its host** (amended by the owner: isolation from the start). Classes:
   storage (SD, CF, the file systems' block layer), display and input
   (2700G, keypad, touch, backlight), audio, connectivity (USB, Wi-Fi,
   Bluetooth), power. A fault in one class can't reach another's memory.
   Drivers get MMIO windows only for the devices the board description
   grants them, and their interrupts as waitable objects (ADR-0007). A
   driver that faults is restarted from its entry point; its in-flight calls
   fail. Each host costs a slot and its stacks (on the order of 64 KiB
   resident), measured as the hosts are built.
6. **No global names: a service manager hands out ports.** Processes start
   with a handle to the service manager only; it gives port handles per
   policy (which process may open which service), which is where the CE
   personality's `CreateFile("COM1:")` and `DeviceIoControl` land.

## The four lenses

- **Iterate:** CE's thread migration, kept for what it gets right (speed,
  caller identity, priority), with what CE lacked: per-process memory
  protection, supervised restartable drivers, handles instead of global
  names, page moves instead of shared-memory hacks.
- **Modernize:** capability-style access (only what you were handed), Rust
  ownership as the model for data in flight, failure that is an error
  value, not a hang.
- **Legacy support:** the CE personality maps PSL calls, `device.exe`
  semantics and `DeviceIoControl` one-to-one onto ports and migrating
  calls.
- **Better/faster:** a call is two context writes and a register copy; no
  copying of bulk data; no cache flushes anywhere.

## Alternatives considered

- **Rendezvous IPC between threads (L4/seL4 style).** Clean, but each call
  is two scheduler switches and the server's own priority (inversion unless
  donated), and CE drivers expecting the caller's thread would need a shim.
- **Message queues (Mach, QNX pulses).** Buffers to size and overflow,
  copies in and out, more memory; notifications are better served by
  events.
- **Shared memory between slots.** Aliasing on VIVT caches: correct only
  with cache maintenance on every handover, which page moves do once and
  precisely.
- **Drivers in the kernel.** Faster by a context write, but a driver bug
  takes the system down; DESIGN §5.1 rules it out.

## Consequences

- The kernel gains: process objects, ports, the call/return path with
  per-thread server stacks, page moves, domains per process.
- Servers must be reentrant; the Rust service framework makes that the
  default.
- Measurement plan: call round-trip time and a 64 KiB page-move transfer in
  `rp-bench` (emulator: instructions; device: time), against the ADR-0010
  thread switch.

## Open questions

- 63 processes (FCSE's limit): taken as enough (CE5 allowed 32); revisit
  if a real workload says otherwise.
