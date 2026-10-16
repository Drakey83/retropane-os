# The 31-process limit: where it comes from and how to raise it

**Status: decided and built (2026-10-15).** The owner's vote: option 1,
"RetroPane's services above 2 GB, old programs keep CE's 31 slots", and
yes to 255 threads. See "As built" at the end. (Item 78,
docs/research/imitation-audit.md, "Memory model".)

## In plain words

RetroPane can run 31 programs at once because it copied Windows CE 5's
map of memory: 32 MB per program, programs between 64 MB and 1 GB, and
above them up to 2 GB a big area for memory-mapped files. That map, not
the chip, sets the limit. The chip's trick that makes switching programs
cheap (FCSE, ADR-0003) allows 128 slots.

The limit matters more for us than for CE: RetroPane puts drivers and
services in their own processes for safety (ADR-0016). They use about a
third of the 31 slots before any app starts.

The scan of the owner's programs (below) found that **no third-party
program depends on where other programs live**. Only Windows Mobile's own
system programs do, and RetroPane replaces those. One game (Quake 3 CE)
needs the big mapped-file area to stay big.

## Where the limit comes from (the facts)

- `kernel/mm/src/layout.rs`: 32 MiB slots; slot 0 is the running process
  (FCSE relocates it there), slot 1 is CE's XIP DLL code, slots 2-32 hold
  processes, `0x42000000..0x80000000` is the shared mapped-file area, and
  the kernel's half starts at `0x80000000`. `kernel/ipc` copies the bounds
  (`FIRST_SLOT` 2, `LAST_SLOT` 32, checked at build time).
- **Hardware limit:** the FCSE process ID is 7 bits, so there are 128
  slots of 32 MiB, the whole 4 GiB. Slots 64-127 are today the kernel's
  half, so RetroPane can never have more than ~120 FCSE processes.
- **Choices:** the shared area's 30 slots (`0x42000000` up) and the
  kernel's 64 (`0x80000000` up) are layout choices, CE's and ours.
- **Other limits nearby:**
  - 128 threads in all (`kernel/sched::MAX_THREADS`, thread numbers are
    8 bits);
  - 32 port handles per process (`kernel/ipc::MAX_HANDLES`);
  - 13 ARM domains for process protection, recycled least-recently-run
    (`kernel/mm/src/domains.rs`). More processes than 13 in rotation
    cost a retag of 32 first-level entries plus a slot TLB flush when one
    gets its domain back.

## Who uses the slots

The OS's own processes (ADR-0016) are:

- driver hosts per class: storage, display and input, audio,
  connectivity, power (5);
- services: the service manager, graphics, the file system, the shell,
  the CE personality's window and message server, LegacySync's device
  side, and a few more (~6-8).

That is **~11-13 of 31**, leaving apps ~18-20. Windows Mobile 5 had the
same squeeze (its own ~15-20 system processes inside CE's 32), and "the
32-process limit" was a known WM complaint.

## Which old programs depend on the layout

Scan of the owner's corpus (2026-10-15): 617 ARM PE modules, 122 EXEs and
495 DLLs, from the A08, L11 and L16 ROMs and the third-party apps
(HereticGLES, Doom/Doom2 GLES, Quake 3 CE with its DLLs, Enigmo, fmodce).
Imports were resolved by ordinal through the ROM's `coredll` export
table. Full tables: the session notes' `process-limit-corpus-scan.md`.

| What | Who imports it | Third party |
|---|---|---|
| Cross-slot pointers and permissions (`MapCallerPtr` 42, `SetProcPermissions` 35, `GetCallerProcess` 24, `GetOwnerProcess` 26, `SetKMode` 20, `MapPtrToProcess` 5, `GetProcessIndexFromID` 9) | gwes, services, shell32, quickmenu, taskmgr, ROM drivers | none |
| `ReadProcessMemory` / `WriteProcessMemory` (4 / 3) | btrenamedir, felvop (L11 ROM), outlookenhancement, toolhelp | none |
| Toolhelp process lists (8) | taskmgr, switcherbar, 2700G display drivers, SQL CE | none |
| Mapped files (`CreateFileMappingW` / `MapViewOfFile` 49 each) | 11 ROM EXEs, 38 DLLs | **Quake 3 CE** |
| `VirtualAlloc` (29) | ROM | **Quake 3 CE**, its libEGL |
| `TerminateProcess` (32) | ROM | Quake 3 CE |

Image bases:

- all 122 ROM EXEs are at `0x10000`;
- 301 ROM DLLs are pre-relocated into slot 1 (285 with their relocations
  stripped, so they *must* load there: RetroPane keeps slot 1 for them);
- only 2 modules sit at or above `0x42000000` (avcodec and avutil in the
  L11 ROM, both relocatable);
- every third-party image is relocatable or at the standard base.

**So:**

- what depends on CE's slot numbers is Windows Mobile's own system
  software (gwes, shell, task managers, ROM drivers), which RetroPane
  replaces with its own;
- the one third-party constraint is Quake 3 CE's ~80 MB of file-backed
  "swap" views (two ~40 MB pools in its config). That needs a large
  shared area, not a particular slot.

**What an import scan can't see:** hard-coded slot addresses, pointers
computed from a process index, sizes and flags passed to `VirtualAlloc`
(CE5 put reservations over 2 MB in the shared area), or APIs looked up at
run time. Every option below keeps CE apps' own view (slot 0, slot 1,
`0x10000` bases) unchanged, so those risks are small. Each option also
gets a compatibility test with the owned games before it ships.

## The options

Costs are kernel RAM (64 MB device; the OS's share is 12 MiB,
docs/memory-budget.md). The tables that grow per slot today are fixed
arrays:

| Table | Per slot |
|---|---|
| IPC handles (32 × 4 B) | 128 B |
| Process record (ADR-0019) | ~60 B |
| Domain map | 1 B |
| **Total** | **~190 B** |

A running process costs the same in every option (kernel stacks, a
region page, second-level tables: ~16-24 KiB for a small one).

### A. Keep 31 (CE's layout as is)

- **Plus:** no work, nothing to retest.
- **Minus:** the OS's own processes take a third. Apps get ~18-20, fewer
  as services are added.
- **Memory:** none.

### B. Give part of the shared area to processes

Slots 2-47 hold processes (46). The shared area becomes `0x60000000` to
`0x80000000` (512 MB).

- **Plus:** a small change (two constants, the build-time check, the
  domain table). Quake 3 CE's 80 MB fit easily, and 512 MB of mapped
  files is far more than 64 MB of RAM can use.
- **Minus:** CE apps see a smaller shared area. Only a program that
  hard-codes `0x42000000`-`0x5FFFFFFF` would notice; none was found.
  Still 46, not more.
- **Memory:** +15 slots × ~190 B = **~2.9 KB** of fixed tables.

### C. Native processes above 2 GB (recommended)

Move the kernel's half up to `0xC0000000` (direct map, device windows;
the vectors and KData stay at the top). That frees FCSE IDs 64-95,
`0x80000000`-`0xBFFFFFFF`. RetroPane's own processes (driver hosts,
services, native apps built against `rp-hal-user`) live there. All 31 CE
slots, and the whole CE shared area, stay for Windows Mobile programs,
exactly as CE5 laid them out.

- **Plus:**
  - CE apps get all 31 slots and the full 1 GB shared area, so their
    view is byte-identical to CE5's;
  - 32 more slots for RetroPane's own (63 in all);
  - same FCSE speed: no cache or TLB work on any switch;
  - room to grow to ~94 by also taking slots 96-111 if the kernel packs
    tighter.
- **Minus:**
  - the kernel's virtual addresses move: constants in `kernel/mm::layout`
    and `rp_hal_pxa27x::devmap`, one emulator and device run;
  - native processes can't be addressed by CE-era APIs that take a slot
    index (none of ours will);
  - a kernel-mode CE driver that assumed CE's static kernel mappings at
    `0x80000000`/`0xA0000000` would break, but RetroPane doesn't run
    Windows Mobile's drivers.
- **Memory:** +32 slots × ~190 B = **~6 KB** as fixed arrays, or ~0 if
  the per-slot tables move into each process's own pages (as ADR-0019
  did for regions), paid only by processes that exist.

### D. Separate address spaces for native programs (no FCSE)

Each native program gets its own first-level table, so there is no slot
limit at all.

- **Plus:** unlimited processes; full 4 GiB views.
- **Minus:**
  - every switch to or from such a program must clean and invalidate the
    32 KiB VIVT caches and the TLBs. That is the cost ADR-0003 exists to
    avoid: the refill alone measured ~100 µs per switch with 16 KiB
    working sets (ADR-0010 amendment), plus the clean itself;
  - each program needs its own 16 KiB table, and only one fits in the
    fast SRAM (ADR-0006).
- **Memory:** 16 KiB per process (+ second-level tables), and the
  switch-time cost.
- **Not recommended.**

### E. Larger handle tables (with any of the above)

Today each process has 32 port handles in a fixed table. CE programs
expect hundreds of handles (events, files, windows' objects) once the
personality's kernel objects exist (ADR-0010 rule 7, not built). Move the
handle table into the process's own pages from the frame allocator, on
demand, with a quota: ADR-0019's pattern. A 4 KiB page holds 1,024
4-byte handles.

- **Plus:** small processes stay small; big ones get room; one process
  can't exhaust another's handles.
- **Minus:** one more lookup indirection.
- **Memory:** fixed tables shrink by 128 B per slot (−4 KB at 31). A page
  is paid only by a process that uses more than its first handles.

### Also: 128 threads

The scheduler numbers threads in 8 bits. At ~12 OS processes with 2-4
threads each, ~80 remain for apps. Raising it to 255 costs 24 B a thread
of scheduler table (+3 KB) and the kernel stacks of threads that exist;
beyond 255 needs 16-bit queue links (+~1.5 KB). This should be decided
together with the process limit.

## The four lenses (option C with E)

| Lens | Effect |
|---|---|
| 🔁 Iterate | Process count, per-process kernel memory and switch time measured before and after; the corpus scan re-run as the owner adds programs |
| ✨ Modernize | Per-process kernel tables in the process's own pages with quotas (as ADR-0019); native code no longer squeezed into a 1990s layout |
| 🗄️ Legacy support | CE programs see CE5's layout unchanged, with all 31 slots and the full shared area (Quake 3 CE's 80 MB of views included) |
| ⚡ Better / faster | 63 processes instead of 31 at FCSE speed (no cache or TLB work on switches); kernel RAM follows real use |

## The vote

Which way for the process limit?

1. **C + E (recommended):** RetroPane's own processes move above 2 GB.
   CE programs keep all 31 slots and CE5's exact layout, and native
   processes get 32 more. Handle tables grow on demand. About a day's
   work plus one device run.
2. **B + E:** shrink the shared area to 512 MB for 46 slots in all. The
   smallest change, but CE programs see a smaller shared area.
3. **A:** keep 31 for now and revisit when the services exist.

Separately: raise the thread limit from 128 to 255 (+3 KB), yes or not
yet?

## As built (2026-10-15, branch `claude/native-slots`)

**Better than option C as written: the kernel didn't have to move.**
Option C assumed the kernel used all of `0x80000000` up. Reading the
kernel image's map showed it uses only the device windows
(`0x90000000`-`0x9C1FFFFF`), RAM at its own address
(`0xA8000000`-`0xABFFFFFF`) and the top MiB (vectors, KData). So:

- **Native slots 96-126 sit at `0xC0000000`-`0xFDFFFFFF`**, empty until
  now: 31 more processes, 62 in all, with nothing relinked or moved and
  FCSE switching as before. (Option C's 32 would have needed the move;
  31 needs nothing.)
- **CE's slots 2-32 and the whole shared area are untouched**, so
  Windows Mobile programs see exactly CE5's layout.
- `layout::DIRECT_MAP_BASE` now says what the kernel does (RAM at its
  physical address), not the `0x80000000` it never used.
- **What runs where:** RetroPane's own processes take native slots: the
  service manager (96), the supervised host (97), the display host (98),
  and everything the manager starts. CE-slot test clients call them.

**Handle tables grow on demand (option E):**

- 8 handles sit inline in each process's record (32 bytes; every service
  so far needs one or two).
- Past that, the table takes 4 KiB pages of 1,024 handles from the frame
  allocator, the same source as region lists, up to 4 pages (4,104
  handles).
- A new page is cleared when taken and wiped when it goes back, which
  happens when the process exits.
- Handles are 16 bits now. Out of memory refuses the handle and changes
  nothing.

**255 threads:** the scheduler, the HAL's contexts and IPC's per-thread
table all cover every 8-bit thread number but 0. The kernel image checks
at build time that the three agree, and that IPC's slot ranges match the
layout's.

**Found and fixed on the way (each class, with a guard test):**

- **A native slot was taken for the shared area.**
  `AddressSpace::is_shared_area` tested `lo >= 0x42000000`. That would
  have let a native process map a writable shared file page in its own
  slot: the VIVT aliasing ADR-0008 rule 6 forbids. It now tests the
  shared area's real range; test `policy_w_x_and_shared_writes`.
- **IPC's per-thread table refused thread 128.** It had 128 entries
  indexed by thread number (1-128) and rejected `tid >= 128`. It is now
  sized and checked by number; test `every_thread_number_can_call`.
- **System-call arguments were truncated.** A handle or slot number from
  user mode was cut with `as u8`, so `0x105` named slot 5. Values past
  their width are now refused.
- **A kernel table indexed by slot number** (calls inside each server)
  was sized for CE's slots. All per-slot tables now go through
  `layout::slot_index`.

**Cost**, measured against main:

- `rp-kernel` +3.2 KiB code (inside its budget, no raise) and +3.3 KiB
  zeroed RAM, mostly the 127 more thread records;
- the IPC handle tables shrank, 3.9 KB → 2.9 KB for twice the slots;
- an out-of-line trampoline keeps the VM and IPC locking in one copy
  (one per call site had cost ~650 B each).

**Tests:**

- host: layout (dense slot indexes, ranges, FCSE reach), domains across
  both kinds, IPC (growth, the quota, out of memory, native tables,
  thread 255, random hostile operations over all 128 slot numbers and
  16-bit handles), scheduler (thread 255 queues, waits and wakes);
- emulator, `rp-kernel`: `IPC native slots OK` (the manager's code is
  mapped above `0xC0000000`) and `IPC handle pages OK` (a ninth handle
  costs exactly one frame, and exit gives it back), plus every earlier
  check with the services in native slots.
