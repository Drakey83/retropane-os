# ADR-0016: Processes, the driver host, and IPC

- **Status:** Accepted (2026-10-12, by the project owner) with decision 5
  as amended: a driver host per driver class from the start (the owner
  chose the more isolated variant). Step 1 (`rp-kernel-ipc`, the rules)
  and step 2 (the migrating call live in `rp-kernel`, emulator-tested)
  done the same night; a call is carried as a nested activation on the
  calling thread's kernel stack (one word per thread for the innermost
  continuation, no table of saved registers); step 3 (service manager,
  driver hosts) next.
- **Date:** 2026-10-12
- **Last verified:** 2026-10-13 (against commit 3158b26)

> **Correction (note added 2026-10-13):** step 3 is no longer "next": the
> service manager and supervised hosts landed 2026-10-12 (commit cb2a468)
> and a real driver host on 2026-10-13 (item 46; see the implementation
> notes below).

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

> **As built (note added 2026-10-13):** a call carries 3 words (`r1`-`r3`,
> the handle in `r0`) and returns 1 word in `r0`, not 6 each way
> (`hal/user/src/lib.rs`: `call`, `SYS_CALL`, `SYS_RETURN`). There is no
> call timeout and no page-move system call yet (`kernel/ipc`,
> `hal/user`). Processes get their own ARM domain only while among the 13
> most recently run: ARMv5 has 16 domains, so the rest are *parked*
> (domain 15, never enabled) and retagged when they next run
> (`kernel/mm/src/domains.rs`). Drivers: only the display and input host
> exists (`services/display-host`, the backlight). The call round trip
> was measured in `rp-kernel` in the emulator (~12-17 us, docs/perf.md),
> not yet in `rp-bench` or on the device.

1. **A process is a slot plus a handle table.** Up to 31 processes (slots
   2-32 of CE5's layout, 32 MB each: FCSE has PIDs up to 63, but above
   slot 32 lies the shared-file area WM apps expect; corrected 2026-10-12), created from a PE image (ADR-0009) with a first
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

- 31 processes (CE5's layout, slots 2-32): taken as enough (CE5 allowed
  32); revisit if a real workload says otherwise.

  > **Note (2026-10-15):** re-examined (item 78):
  > docs/research/process-limit.md. The OS's own processes take ~11-13
  > slots, and no third-party program in the owner's corpus depends on
  > CE's slot layout. The options are with the owner for a vote.

## Implementation notes (step 3, 2026-10-12)

How decisions 2, 5 and 6 are carried out, within the decision above:

- **Caller identity.** A server is entered with the caller's slot in `r0`,
  put there by the kernel: a server can't be lied to about who called.
- **Handing out handles.** `SYS_GRANT(handle, slot, rights)` puts a copy of
  a handle the server holds (with `GRANT`) into another process's table,
  with the same rights or fewer, and returns its number there. The service
  manager answers "open service X" this way.
- **Supervision and restart.** A port may have a supervisor (the service
  manager). When its server faults inside a call, that call fails with
  `ERR_FAULT`, and the server is marked *restarting*: new calls fail at
  once, and calls already inside it return an error instead of anything
  the broken instance produced. When the last thread has left, the kernel
  resets the server's writable memory (it comes back zero-filled; code and
  constants stay, shared), bumps the port's generation (every handle taken
  before turns `Stale`) and renews the server's and the supervisor's own
  handles. Clients ask the manager again for a fresh handle. A port with
  no supervisor keeps the earlier behaviour: the faulting call fails, the
  server stays.

## Implementation notes (a real driver in a host, item 46, 2026-10-13)

- **Compiled user programs.** A program links `rp-hal-user` (the process
  entry, the system calls, device windows) and `hal/user/user.x`;
  `cargo xtask` turns the ELF into a PE file (`xtask/src/userimg.rs`:
  absolute words become `HIGHLOW` fix-ups, unknown relocation types are an
  error) before the kernel image is built, and the test kernel embeds it.
  The kernel loads its pages on demand straight from that read-only copy.
  The display host is 1.2 KB of code and needs no fix-ups at all.
- **Entry convention.** One entry per image: a thread starts there with
  `r0 = 0` and its role in `r1`; a call enters with the caller's slot in
  `r0` (2-32) and its arguments in `r1`-`r3`.
- **Device grants.** The board grants a host whole register pages: mapped
  uncached at a fixed address per device (`rp_hal_user::Device`), so the
  HAL can hand out windows safely by device, never by address. The kernel
  switches the device's clock on first (the clock unit is never granted).
- **Interrupts.** `SYS_IRQ_WAIT(line)`: only the process the line is
  granted to, never inside a call; it re-arms the line, waits, and
  returns the pin's level. The kernel masks and signals; the host does the
  rest (ADR-0007).
- **Restart covers a host's own threads.** A supervised host that faults
  in its own thread restarts like one that faults in a call. A thread of
  it blocked in the kernel when it restarts re-enters at the entry on
  fresh memory, by rewriting the saved user context its system call
  returns to (no kernel stack is abandoned); its other threads end.
- **Driver state in the hardware.** The backlight driver keeps no state of
  its own: the level is the PWM's duty register, read back. A restarted
  host carries on with no flicker and nothing to save. Prefer this for
  any device whose registers read back.
