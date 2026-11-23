# Game Mode: building design B

**Status: started 2026-11-22** (the owner's go on design B, 2026-11-23 by
his calendar: "do your recommendations for all of these"; for this work:
"start this work in a new session, rule 0 all of it"). The design is
[game-mode-b.md](game-mode-b.md); this page is how it is built, in what
order, what blocks each step, and each step's rule 0 sheet (CLAUDE.md
rule 0: what Windows CE/WM, consoles, Android, iOS and Linux do, as
findings; the best way today's tools allow on this exact hardware; why ours
wins). The step list itself lives in the roadmap ("The showcase", step 8).

## Where the kernel stands, and what blocks Game Mode

`cargo xtask gate` (2026-11-22): **30 of 30 architectural lines ticked for
phases 2, 4 and 5** (lifted by the owner); phase 7 0 of 2 (2 partial),
phase 9 4 of 11 (2 partial, 5 open), each gating only its own phase. **No
open architectural line blocks Game Mode's design.** The one phase 9 line
it touches is **V6/P10** (the cursor plane and colour keys, not tried):
design B shows its "Keep holding to leave the game" countdown and "Game not
responding" choice "on the system's own plane"; that plane is the 2700G's
video plane, already proven to show RGB565 (**V5**, ticked), so P10 is not
needed. **T7** (clocks per moment against battery) would tune Game Mode's
clock profile later; nothing waits on it.

What does block it is work not yet built, not a gate line:

| # | Blocker | Where it stands | What waits on it |
|---|---|---|---|
| K1 | **Settled 2026-11-23: `rp-kernel` passed on the X51v (G5 run 4, below).** Before: **the kernel had never run on the X51v.** `rp-kernel` (VM, compressed RAM, scheduler, IPC, PE processes, supervised hosts, the display host) passes in the emulator only; Phase 2's exit ("multiple processes running, with protected memory, on device and emulator") is open. Only `rp-threads` (the scheduler's 13 tests) and `rp-quantum` ran on the device (2026-10-12, 2026-10-15) | Phase 2 | every device measurement of the freeze, the thaw and the switch; R270 (the game as a process, measured against the bare test image) |
| K2 | **Nothing of the OS runs as a process yet.** The shell (`rp-today`) and Half-Life's runner are bare test images under the loader; the audio and graphics services are libraries inside them, not driver hosts (only `services/display-host` is one) | Phase 2 "Processes ... driver host process", Phase 5 | Full Mode's suspend/resume calls, the exclusivity grants, the session test |
| K3 | **The large-allocation region (R363)**: ADR-0003 amended 2026-11-23; the reservation itself not yet in the kernel's address map | doable now (step G4) | the game's 27 MiB of modules and 26 MiB heap under the kernel |
| K4 | **The CE personality runs no WM5 program yet** (Phase 4: coredll, GAPI not built) | Phase 4 | design B's CE legacy test; LZ4's ratio on a CE app's heap (measurement 2's second half) |
| K5 | **The internal flash (DiskOnChip G3, CS0) can't be read or written by our code** (R6, R21) | R6 | measurement 1's flash half from our own driver; design B's "return record's copy in flash" |

## The measurements design B needs before code

| # | Measurement | Result (2026-11-22) | Source |
|---|---|---|---|
| 1a | SD write speed, large sequential, owner's card | **measured on the X51v with the full tier's own pattern (2026-11-23, `rp-sdtime` Game Mode run, `RP_SDTIME_GAMEMODE`, run 1):** a 30 MiB file preallocated in one run of clusters once: 8.5 s; written whole in 1 MiB DMA commands: **5,048-5,052 KiB/s (6.1 s for 30 MiB)** once the blocks had been written before, 3,444 KiB/s (8.9 s) the first time over fresh blocks; each MiB 135 ms (median), one stall a pass of 1.0-1.4 s (the card's own housekeeping); ACMD23 (pre-erase) made no difference once warm; read back by DMA and checked: **7,278 KiB/s (4.2 s)**, every block intact. So a 30 MiB full tier costs ~6 s out and ~4 s back, inside design B's 15 s each way, and LZ4's 2.0x makes it ~60 MiB of RAM. The two older runs' disagreement (4.9 MB/s run 1, 1.6-2.2 MB/s r7) fits the fresh-blocks cost seen here: a file written once is the fast case, so G8 keeps one file and writes over it | `local/rp-sdtime-gamemode-run1.txt`; earlier: `docs/research/storage-speed.md` "S2", `local/rp-storfast-run1.txt`, `local/rp-storfast-r7-log.txt` |
| 1b | Internal flash write speed | **not measurable by our code** (K5). Possible under Windows Mobile: a WM tool timing a file written to its built-in storage gives WM's driver's speed, a finding only (R365) | — |
| 2 | LZ4's ratio on real heap pages | **the shell's, measured in the emulator:** see below. A CE app's: blocked (K4) | `cargo xtask emu-heap-ratio` |
| 3 | Resident sizes from run logs | the 2700G's layout and every shell cache are measured (`docs/memory-budget.md`: screens 600 KiB each, work picture 1.17 MiB, background 1.17 MiB, image cache 384 KiB, glyph atlases 384 KiB, area cache 600 KiB: ~6.7 MiB of 2700G memory the game gets back); RAM: `rp-today` 933,224 B loaded + 102,304 B zeroed (the build's size check, 2026-11-22); the services as processes don't exist yet (K2) | `docs/memory-budget.md`, `cargo xtask` size check |
| 4 | The kernel's size | **`rp-kernel` 104,236 B loaded + 119,340 B zeroed RAM** (2026-11-22, inside its 112 / 120 KiB budget), against design B's and the memory budget's 1 MiB for the kernel image: today's kernel carries its own test programs, so the shipped one is smaller; the kernel's run-time memory (page tables, the store, stacks) is measured once K1 runs | the build's size check |

### Measurement 2: LZ4 on the shell's heap

`cargo xtask emu-heap-ratio` (2026-11-22, the pinned emulator rp-v0.33.0):
the Today screen started under the loader from the emulator's card, used
for ~25 s (the weather page, hourly, now, home, twice), then all 64 MiB
of the guest's SDRAM (on SDCS2 at `0xA8000000`) saved and every page
priced with the kernel's own codec (`rp_lz4::compress`) and the compressed
store's own size classes (64-byte granules, `MAX_OBJECT` 3,072 bytes).
The bytes are the image's own, so the ratio is the device's for this
session; what the emulator can't give is the owner's real weather and his
apps' data (R364 takes a dump on the X51v).

| | Pages | MiB |
|---|---|---|
| All of RAM | 16,384 | 64 |
| All zero (mostly never used: a process maps none of these) | 14,477 | 56.6 |
| Live (not zero) | 1,907 | **7.45** |
| of which one repeated word | 12 | 0.05 |
| of which compress into the store | 1,210 | 4.73 -> **1.05 in slots (4.5x; LZ4 alone 4.67x)** |
| of which don't compress (stay resident) | 685 | 2.68 |
| **Live pages kept in the fast tier** | | **3.73 (2.00x)** |

What the numbers mean:

- **The heap compresses 4.5x** where it compresses at all (the store's
  64-byte rounding costs little: 4.67x before it). The 1 MiB bands holding
  the shell's working data compress 6.7-9.9x.
- **The 685 pages that don't compress are code and already-packed data:**
  the band at `0xA8000000` (the loader's and the image's code: 1.16x) and
  the two bands at `0xA9A00000`-`0xA9BFFFFF` (1.01-1.02x, ~1.4 MiB: the
  size and place of the icon pack's file read whole). Under the kernel
  neither is frozen: code is a file mapping (let go, reloaded from the
  card on demand) and the icon pack's file is a cache design B drops in
  step 4. So for the frozen shell's own memory the right figure is ~4.5x,
  not 2x: design B's step 5 assumed "half the bytes if heap pages halve".
- **One-word pages are rare** (12 of 1,907 live pages, 0.6%): no rule for
  them. **Zero pages** can't be counted from a whole-RAM dump (most of RAM
  was never touched, and a process maps none of that); the kernel image's
  own process (`emu-kernel`'s `VM freeze` step) shows the rule at work:
  31 touched zero pages freed with nothing stored, against 16 marked pages
  compressed. The device dump (R364) counts them in a real session.
- **For design B's budget:** the shell's 7.45 MiB live, frozen fast-tier,
  keeps ~1.1 MiB (heap compressed) once its code is let go and its icon
  pack dropped; at the measured 183 us a page, compressing its 1,210 heap
  pages takes ~0.22 s on the X51v, inside step 5's 1.2 s.

## The build order

Each step: its rule 0 sheet, then an emulator test, then the device (when
K1 lets it). G1 to G4 are built, G6's protocol and G8's tier logic too (2026-11-23); the rest in this order.

### G1. Freeze a process's memory (built 2026-11-22)

`Vm::freeze(space, drop_file_pages)` in `kernel/mm` (design B step 5):
every page of a process in one pass. Owned pages (anonymous memory,
private copies, resident or already aged by the reclaim clock) are
compressed into the store; all-zero anonymous pages go back to demand-zero
with nothing stored; page-cache pages are let go (they reload on demand)
or kept; device windows and pinned regions are never touched. Thawing is
the fault path the reclaim clock already uses: lazy, a page at a time,
only what is touched.

- **What others do (findings):** Windows CE/WM has no freeze: under memory
  pressure the shell sends `WM_HIBERNATE`, then closes programs. Android
  freezes cached apps (the cgroup v2 freezer) and leaves their pages to
  zram's later LRU scans under pressure; iOS suspends apps and compresses
  their pages on pressure, then jetsam kills. Linux's zram (`ZRAM_SAME`),
  zswap (same-filled pages) and iOS's compressor all special-case pages
  of one repeated value. Consoles suspend the app in RAM untouched.
- **Best way today, on this hardware:** the process is stopped, so no
  page is "recently used": one pass, no aging faults, no LRU walk; the
  zero test reads a page only to its first non-zero word (cheap against a
  compression's 183 us, *measured*, `probe.lz4_compress_page`), and a zero
  page costs nothing at all, not even the store's smallest 64-byte slot.
  Measured on the shell's own pages (above) before choosing: the zero rule
  costs a word read per page and frees a frame with nothing stored (the
  kernel image's own process: 31 of 47 frozen pages); one-word pages are
  rare (12 of 1,907 of the shell's live pages), so they get no rule of
  their own (it would need a new page-table encoding).
- **Why ours wins:** nothing is killed (WM, Android, iOS); the memory is
  free before the game asks for it (Android and iOS compress only under
  pressure, while the game is already running); no partition is reserved
  when no game runs (consoles).
- **What overturns it:** a device run where the freeze's time per page
  (the kernel's `BENCH vm_freeze_process` line) times the process's pages
  misses design B's 1.2 s, or the thaw's faults show in the shell's first
  second back.
- **Tests:** host tests in `kernel/mm` (every page kind, the zero rule's
  edge: an all-zero *private copy* is compressed, never let go, since it
  would reload its file's bytes; aged pages; device windows; pinned pages;
  everything returned on unmap; two mutants checked to fail them);
  `emu-kernel`'s new `VM freeze` step (the kernel image's process frozen
  whole, every marked page thawed intact, pinned pages never faulting,
  the frames freed equal to the pages let go minus the store's growth).
- **Found on the way and fixed (the class, with guard tests):** reclaim's
  "let go" of a page-cache page left its page table's use count one high
  (the table was never freed after unmap: one frame lost per table), and
  `Vm::unmap` of a mapped device window computed a frame number below the
  RAM's base (an overflow in checked builds).

### G2. Freeze a process's threads (built 2026-11-23)

Take every thread of a frozen process off the run queues (and its timers
and waits held, not cancelled), so a frozen program can't run, touch a
page and undo the freeze; put them back as they were on the way out.
Rule 0: CE has `SuspendThread` per thread (a count, racy with waits);
Linux's cgroup freezer and Android's freezer stop a whole group at once
and let in-flight system calls finish; ours, the same at the scheduler
(ADR-0010), one flag per process checked where a thread is made ready,
so a freeze is one pass over the process's threads and a wake while
frozen is queued, not lost. Emulator test in `rp-kernel`.

**As built:** `Scheduler::freeze(slot, now)` and `thaw(slot)` in
`kernel/sched`, a bitmap of frozen slots (zero when none, so the empty
scheduler stays all-zero memory) and a `Frozen` thread state. A ready or
running thread of the slot leaves the run queues (the running one keeps
what was left of its quantum, as a preempted thread does); a blocked one
keeps its wait and deadline, and when the wait ends while frozen it
becomes `Frozen` instead of ready, so the wake is kept for the thaw. The
only cost on the hot path is one bit test in `wake`. Found while building
it: a frozen waiter at the head of a mutex's or an auto-reset event's queue
would take the mutex or the signal from a process that runs and hold it
until the thaw; it is passed over while anyone else waits (it keeps its
place), and `freeze` counts the mutexes a frozen thread owns while a
running process waits for them (`held`), for G5's switch to refuse or
report. Thawed threads go to the back of their queues in thread order
(CE's place for a thread that becomes ready). Host tests (6, four mutants
caught); `emu-kernel` freezes the IPC clients' process mid-run: 47 calls
before, none in 30 ms frozen, 151 in the first moments after the thaw,
both threads ready again, and every IPC result after it whole.

### G3. The return record (built 2026-11-23)

A region reserved at boot (256 KiB, the owner's ceiling, design B), in the
kernel's own area outside every slot: the frozen processes, their threads'
records, the suspended services' records, the full tier's page map; a
versioned, checksummed, bounds-checked format so a crashed game can't
corrupt it and a damaged record is refused, not followed. Rule 0: consoles
keep the system's state in a reserved partition of megabytes; Android
saves an app's state into a Bundle per app (often lost); ours, a quarter
megabyte, and only the record (the processes stay where they are).

**As built:** `kernel/gamemode` (`rp-kernel-gamemode`, pure logic, host
tested). A 32-byte header (magic, version, section count, body length,
sequence number, the body's hash, reserved bytes that must be zero, the
header's own hash) and typed sections (process, thread, service, tier
map), each length checked against the half before it is used and the
padding and flags checked zero; an unknown version or kind is refused.
The hashes are XXH32 (checked against xxHash's published vectors): no
1 KiB table as CRC-32's, a word at a time with the multiplier; the body's
is seeded with the sequence number, so an old body under a newer header
fails. The 256 KiB reserve is two halves: `commit` writes into the half
not holding the newest good record, clears its magic first and writes it
last, so a reset at any byte (the tests replay every prefix of a commit)
leaves the previous record the newest good one; a build that runs out of
room leaves its half empty. Host tests 9, seven mutants caught.
`emu-kernel` commits, fails a build, refuses a flipped bit and reads the
sections back on the ARM target. Kept non-generic (`commit` takes a
trait object): ~2.6 KB of code in the kernel; the kernel's load budget
went 112 -> 124 KiB for G1-G3 under the standing rule (115,584 B).
**For G5:** the kernel cleans the record's cache lines to RAM before it
writes the magic and again after, or a watchdog reset with RAM kept could
leave the magic in RAM ahead of the body (the D-cache is write-back); the
reserve's place (outside every slot) comes with G4's layout.

### G4. The large-allocation region (R363) and 1 MiB sections (built 2026-11-23)

At least 64 MB of address space outside the 32 MB slots for a Game Mode
port's modules and big blocks (ADR-0003 as amended 2026-11-23), mapped in
1 MiB sections where the memory is contiguous (ADR-0039 rule 1: the
PXA270's translation caches then cover the whole game with no page walks,
as the bare test image's 1:1 map does). Rule 0: CE5 maps its large
`VirtualAlloc`s there at 4 KiB pages; ours maps the game's in sections
because the game's frame time is the number that matters (to measure
against the bare image: R270).

**As built:** `layout::LARGE_BASE..LARGE_END` = `0x78000000..0x80000000`,
128 MiB at the top of CE5's shared area (where CE5 itself put large
`VirtualAlloc`s); CE programs' shared mappings keep the 822 MiB below it
(`layout::SHARED_END`), and the large window takes no shared writable file
mappings. It is its own `AddressSpace::large(domain)` in the game
process's ARM domain, so only that process reaches it. `Vm::map_sections`
adds a pinned, zero-filled region on megabyte boundaries and maps each
megabyte it can at once as one section of a free 1 MiB block of RAM,
zeroed; a megabyte with no free block is left to 4 KiB demand pages, so
fragmentation costs speed, never the allocation. Pinned: reclaim and
freeze never touch it. `unmap` refuses to split a section (nothing
changes), writes the caches back while it is still mapped, then frees
each block; `protect` and `retag` change a section whole. Host tests 4
(nine mutants caught); `emu-kernel` maps 4 MiB as 4 sections, writes and
reads each through its address and through RAM's own, takes no fault,
and gets every frame back. The frame-time gain against 4 KiB pages
(design B's reason for sections) is measured on the X51v once the kernel
runs there (K1, R270).

### G5. The kernel on the X51v (K1)

`rp-kernel` loaded by the resident loader as a test image (as `rp-threads`
was), its VM, IPC and supervision steps run on the device, its `BENCH`
lines (the freeze and thaw's among them) read from the run log. A device
run: the owner's go first.

**Device build (2026-11-23):** `RP_KERNEL_RETURN=1` builds `rp-kernel`
to return to the resident loader at the end (no wait for the power
button; the default build still halts for the emulator's direct boot),
and every line the kernel's checks print goes to the run log and the
card's log as well as the serial port (as `rp-threads` does). RAM stays
identity-mapped under the kernel's tables, so the loader's hand-back
works from them; a hang is caught by the loader's 30 s run limit (an FIQ
the kernel leaves to the runtime), not the hardware watchdog.
`cargo xtask emu-kernel-loader` runs that build under the loader in the
emulator: every check in the run log, back at the loader. Variant builds
now get the packaged user programs (`rp-display-host.pe`) in their own
folder (they were embedded empty, so IPC's setup failed in any variant).

**Run 1 on the X51v (2026-11-23, unattended, `local/rp-kernel-g5-run1.txt`,
sha `ddc113502cdbc8e7`):** the kernel built its tables ("VM tables OK, 12276
frames free") and panicked on its next line, "device not in the kernel
map": the line went to the card log, whose CF socket the X51v HAL
translated at the socket's base (`0x30000000`, its I/O space), which the
kernel's device map doesn't cover. It came back to the loader with its
report (no reset, no restore screen): the safety path worked. Fixed as a
class: each CF window translated at its own address, the CF socket's
attribute and common windows added to the kernel map, a host test that
every device window the HAL hands out is in the kernel map, and
`emu-kernel-loader` now runs with an SD and a CF card and requires the
kernel's log on both (it reproduces the panic without the fix; the first
emulator check had no cards, so the card log never ran).

**Run 2 (2026-11-23, `local/rp-kernel-g5-run2.txt`, sha `deca079088d4ac13`):
the kernel's first run past its tables on the X51v.** Every VM check
passed (zero page, file page, copy-on-write, FCSE, domains, fault loop,
compressed RAM, reclaim, pinned, many regions, unmap) and every Game Mode
one: `GAMEMODE full tier OK` (1 store frame out and back, 1 moved, 49
handles rehomed), `VM freeze OK`, `GAMEMODE return record OK` (XXH32 over
64 KiB in 511 us), `GAMEMODE large region OK` (4 MiB as 4 sections, 0
faults), and `THREADS freeze OK` (100 calls before, 0 during 30 ms frozen,
151 after; freeze 47 us, thaw 7 us). Then a system call touched a client's
`.data` page before the client had: the kernel's own fault path never
filled the image's page cache (the user path did), "I/O error", back at
the loader. Fixed as a class: one `resolve` for both fault paths, and a
kernel check that touches an unloaded image page before the clients run
(fails without the fix in the emulator). The device's numbers (the
`BENCH` lines) go into K1's row when the whole run passes.

**Run 3** (`local/rp-kernel-g5-run3.txt`): the new check failed on the
X51v though it passed in the emulator: the XScale's fault address is the
FCSE-relocated one (`0x0a103000`), the image's page fill took it as the
process's own and found no page; QEMU reports the unrelocated address.
Fixed in the one fault path, and the check gives it the relocated form
directly, so the emulator covers it (docs/hw/emulator.md, R368). Since
rp-v0.34.0 the emulator's FAR is the relocated one too (R368 done), so the
kernel's touch takes the device's path and the direct call checks the
process's own form (a prefetch abort's PC) instead.

**Run 4 (2026-11-23, `local/rp-kernel-g5-run4.txt`): `RETROPANE-KERNEL-OK`
on the X51v, back at the loader. K1 is settled.** Every VM, Game Mode
(full tier, freeze, return record, large region, threads freeze), IPC
(calls, server fault, foreign handle, nesting, isolation, preemption in
calls, PE image relocated and paged on demand, service manager,
supervised restart, spawn and exit, native slots, handle pages), trace
and driver-host (served, restarted) check passed; the deepest stack 6,440
of 16,384 bytes. The device's numbers are in docs/perf.md "The kernel on
the real X51v": a page compresses in 104 us and thaws in 43 us (design B
assumed 183 and 134), a whole 47-page process freezes in 3.8 ms, a
process's threads freeze in 49 us. G7 (the runner in the large region,
measured against the bare image, R270) is unblocked.

### G6. Exclusivity grants and the mode manager

The display, input and audio hosts take a grant token for one slot and
reset their device on revoke; the mode manager runs the switch, the 5 s
watchdog and the one way out (quit, crash, hang, force-close). The forced
way back (hold Power 3 s, the owner's ruling) is built after the check
design B asks for: what Windows Mobile itself does on a held Power on the
X51v (R366, a WM step for the owner's go). Waits on K2 for the real hosts;
the protocol and the watchdog are built and tested against stand-in hosts
in the emulator first.

**As built (protocol, 2026-11-23):** `services/mode` (`rp-mode`, pure
logic, host tests against stand-ins). Rule 0: Windows Mobile's GAPI
(`GXOpenDisplay`, `GXOpenInput`) gives a game the frame buffer and the
hardware keys with no revoke, so a crashed game leaves them as it left
them; Android's ANR fires only when input waits 5 s unanswered, so a game
spinning with nothing pressed is never caught; consoles reserve a system
button. Ours: a grant is a token of the game's slot and a generation
that only grows (never 0), so a call made under an old grant is refused
by a host that knows nothing of modes, and each host resets its device on
revoke; the game's first thread is held until the grants are out; the
watchdog runs on frames presented and system calls (a hang is caught
whether or not anyone presses a key), asks "Game not responding" once,
and a frame while it is up takes the question away; a game may set its
own deadline for a long load, clamped to 1-60 s; the millisecond clock
may wrap. Quit, crash, hang (close chosen) and the way back take one exit:
grants revoked in reverse (devices reset), the slot ended, the last frame
shown, the session thawed, the services resumed. A thaw that fails is
tried twice, then again each tick with the record kept, so the session is
never dropped; a way in that fails part way undoes its steps in reverse.
Host tests 8, 17 mutants caught (the 18th tested a check that can't fail in a game; the check went). **Still open:** the stand-ins become the real hosts with K2; the
service as a process in the emulator then; hold Power 3 s after R366.

### G7. Half-Life's runner as Game Mode's one program

The runner (ADR-0039) started by the kernel into the large region, its
modules in sections, measured against the bare test image (R270): frame
time and translation misses. Waits on G4 and G5 (both done).

**What the runner does that user mode can't (read from its source,
2026-11-23):** it runs today as a bare test image, privileged, with the
loader's 1:1 map, and its speed depends on that: device registers taken
whole (`rt::take_*`: the card, DMA channels, I2S, SSP, keypad, GPIO, PWM,
the 2700G's register blocks and its 16 MiB of memory, the CPLD); DMA by
the buffer's address as its physical one (`hal/pxa27x/src/dma.rs`); cache
maintenance (`cpu::clean_range`, `drain_write_buffer` at about 13 places,
some each frame); hot code locked into the I-cache (`unlock_icache`);
write-allocate sections (`rt::write_allocate_sections`); the CPU clock
(624 MHz and past it, `clocks::change`) and the 2700G's divide-1 bus
timing on CS3; a timer interrupt that pumps the screen (`set_tick_hook`,
`PUMP_HZ`) with interrupts masked around what it shares
(`irq_disable`); the cycle counter (`rt::cycles`, also the sound's
clock); the profiler (timer and event sampling); iWMMXt; the run limit
(`rp_image`).

**What others do (findings):** Windows Mobile ran a game in its user slot
with GAPI's frame buffer and keys; the 2700G's own GL ES library
(`libGLES_CL`) ran inside the game's process with the chip's registers
mapped in by `VirtualCopy`, and nothing took them back if the game died.
The PSP runs every game in user mode, driving its GPU's command lists
itself; cache write-back is a system call
(`sceKernelDcacheWritebackRange`). Linux gives a device's registers to user
space through UIO or DRM, and DMA addresses one buffer at a time from the
kernel (`dma_map_single`); ARM Linux's cache flush is a system call too.

**Ours (the plan):** the runner's own drivers run unchanged in the game's
process, as `libGLES_CL` did and as a PSP game drives its GPU, but each
device is a grant the kernel maps in and takes back (G6's revoke). What
user mode can't do comes from the kernel in the cheapest form for each:

| Need | Game Mode gives | Why this form |
|---|---|---|
| Device registers and the 2700G's memory | each granted window mapped into the game's domain, uncached (`Backing::Phys`), at the address the bare image uses where the layout allows | the drivers stay the bare image's code; a revoke is an unmap |
| DMA | the game builds its chain in its own addresses and hands it over in one system call; the kernel checks every descriptor against the game's memory and grants, splits it where pages aren't contiguous, writes the chain the controller runs and starts the channel; the controller's status page is mapped read only | the PXA270 has no IOMMU: a game holding the controller could copy over the kernel or the frozen session (the hole WM's in-process drivers and Linux's UIO leave); Linux's `dma_map_single` pays a call a buffer, this one a chain (first planned as a read-only section map with the controller mapped in: dropped for this reason before it was built) |
| Cache maintenance, write buffer drain | one system call (operation, address, length), checked against the game's own mappings | the PSP's and ARM Linux's form; the cost measured against the SWI's |
| Write-allocate | the large region mapped write-allocate when the game asks (a cache policy, XScale's X bit) | no call at all; the bare image's measured gain kept |
| I-cache lock | a system call locking the game's lines, unlocked by the kernel on every way out | a crashed game can't leave the cache locked |
| CPU clock, CS3 timing | a system call naming a profile (the mode manager's, ADR-0035's ladder), never a raw register | the SDRAM controller is the kernel's; a game can't set a clock the self-test hasn't proven |
| The screen pump's tick | a second game thread woken by a kernel timer; the masked sections become a lock between the game's threads | no game code in interrupt mode |
| Cycle counter, profiler | the OS timer's counter mapped read-only; the kernel samples the game's PC for the profiler | CP14 is privileged; a read stays one load |
| iWMMXt | the coprocessor enabled (CPAR) for the game's threads only, its registers saved on switch only when another thread uses it | Game Mode has one program, so almost never |
| The run limit | G6's watchdog; exit by `SYS_EXIT` | one way out |

**Why ours beats the originals:** WM's in-process driver had no revoke, so
a crash left the 2700G half-programmed; ours takes every grant back. A
game's DMA can reach only its own memory and its granted devices, which
neither WM nor UIO gives, for one call a chain where Linux pays one a
buffer. And the runner's code is the
bare image's, so G7d's measurement isolates exactly what the kernel costs
(sections, domains, FCSE, the tick). **For the owner (a result, not a
decision; nothing waits on it):** when the display host exists (K2), the
game keeps the 2700G directly under its grant, as above, rather than
calling the host per draw: one call is 27.6 us on the X51v and a frame
makes hundreds of draws.

**Steps:**

- **G7a (kernel side, emulator):** the game process with its large region
  and grants, the cache and DMA calls, and a small test game
  (`rp-game-probe`) started by `rp-kernel` that uses each and checks it.
  **G7a-2:** the tick thread's timed wait, iWMMXt (the coprocessor on for
  the game's threads, its registers saved on switch), the I-cache lock,
  the clock-profile call, write-allocate sections, the profiler.
- **G7b (the runner's user build):** `rp-hal-pxa27x`'s privileged calls
  routed to these calls in a `user` build, the runner and its modules
  linked into the large region. Planned with hl-40fps, which owns the
  runner.
- **G7c:** the kernel loads the runner and its modules from the card;
  `emu-xash` under the kernel.
- **G7d (R270):** the X51v: frame time and translation misses against the
  bare image, the same map and settings.

**As built (G7a, first part, 2026-11-23):** `rp-kernel-gamemode::dma`
(pure, host-tested): `translate` checks a game's chain in two passes (every
descriptor checked and split first, so a refusal anywhere writes nothing),
refuses interrupts, fly-by and compare, needs every memory byte to be the
game's (writable for a target) and every FIFO address inside a granted
window, splits at non-contiguous pages, links the kernel's chain and ends
it with STOP; `check_range` holds a cache call to the game's own pages (a
discard needs write access, at most 64 KiB a call). `rp-hal-user` gains
`SYS_CACHE`, `SYS_DMA`, `DmaDesc`, `Large` and the `Ost` and `DmaStatus`
windows; `rp-hal-pxa27x::cpu::invalidate_range` discards lines without
writing them back (edges cleaned first, as Linux does). `rp-kernel` starts
`rp-game-probe` in its own native slot after every other check, with 4 MiB
of large region in sections, the OS timer and the DMA status page read
only, and the card's channel; reads its chain a word at a time from its
own memory only; ends it on its report, taking everything back (the
channel aborted, the large region and slot unmapped). Host tests 10 in the
DMA module, 10 mutants caught. `emu-kernel` (rp-v0.34.0): `GAMEMODE game
program OK`, user mode in its slot, 4 sections, a cache call ~4 us in the
emulator (not the X51v's figure), the 64 KiB copy across a megabyte edge
~2.4 ms with every word right, 4 of 4 refused, every frame back;
`emu-kernel-loader` needs the line on both cards' logs too. Two faults of
the same class found on the way, each fixed as a class: **(1)** the DMA
HAL reached its registers by physical address without `devmap::va` (the
kernel had never used DMA; the bring-up map's identity hid it in every
image): fixed, and a host test now fails for any HAL module that names a
peripheral and reaches it by raw pointer without `va` (proven against the
unfixed file); **(2)** the kernel told the game apart by a slot number
fixed in advance, and the manager's workers take the first free native
slot, the same one: a worker loaded a page of the game's image, and could
have passed as the game in its calls. Now the kernel records the slot the
running game holds, takes it as `spawn` does, and every check (the calls,
the large region's faults, the image's pages) asks that record. **(3)** the
kernel's thread stacks are 4 KiB and a migrating call keeps
`on_syscall`'s frame at every nesting level: with `game_dma` inlined that
frame was 744 bytes (236 without), a client's stack came within 60 words
of its bottom, and the emulator crashed a different way each run (garbage
contexts, `not a process slot`, re-entered globals; first mistaken for two
other causes, R385 and R387, both closed by rerunning them with the fix).
The game's calls now keep their own frames, `rp-hal-pxa27x::thread` fills
every prepared stack with a guard word (checked at each switch, and the
depth read back), and `emu-kernel` fails unless the closest stack keeps
128 words never written (275 measured). **Still open:** G7a-2, then the
device run.

### G8. The full tier

A preallocated swap file on the card, its blocks found once at boot; the
frozen pages compressed first, then gathered into 1 MiB runs written by
DMA (measured with this exact pattern on the X51v, 2026-11-23: 30 MiB in
6.1 s at 5.0 MB/s over a file written before, 8.9 s the first time over
fresh blocks, read back in 4.2 s at 7.3 MB/s; 64 KiB writes 2.9 MB/s,
page-sized writes slower still), read back by DMA on the way out. The file
is made and written once at install, so every switch is the warm case. Rule 0: Linux's swap writes clusters of pages as reclaim finds them;
CE has no swap; ours writes only compressed bytes, in the card's fastest
size, once, at a moment when nothing else uses the card.

**As built (tier logic, 2026-11-23):** `rp-kernel-gamemode::tier`
(`TierMap::swap_out` / `swap_in`, a `Card` trait for the swap file) on
`rp-kernel-mm`'s new pieces: the compressed store keeps its frames in a
`FrameSet` (one bit a frame, 2 KiB for 64 MiB), `ZPool::detach` and
`attach` park it and take it back, `Frames::take` claims one chosen free
frame, `PageTables::rewrite_stored` and `Vm::rehome` point handles at a
frame that moved. What goes out is the store's own frames as they are
(already packed by size class, under 64 bytes wasted a page): no second
packing pass and no staging buffer, each run of 256 frames one 1 MiB
gather by DMA straight from them. The map is the set itself: a frame's
place in the file is its rank, so the record holds no entry a page (the
design's 8 B a page and the plan's ~2 B are both beaten: 2 KiB at most,
plus the list heads and a hash a run, ~2.4 KiB in all, one record
section). Coming back, each frame returns to the frame it left, so no
handle in any page table changes; a frame a kernel allocation kept
meanwhile comes back elsewhere (at most 64; more is refused before
anything changes) and is rehomed. Each run carries an XXH32 chained over
its frames, seeded with its number (a run read from the wrong place
fails); a run that fails is read once more, then the return is refused
with every frame given back and the map still out, so it can be tried
again (G6's tick does). A card that fails going out frees nothing.
Host tests 9 in the tier and 4 in `rp-kernel-mm`, 20 mutants caught (two that tested code that can't change the outcome went with that code). `emu-kernel`'s `GAMEMODE
full tier` runs it on the ARM target between the freeze and the thaw: the
store's frames out to RAM frames standing in for the card, every free
frame written over (the game), one store frame kept, the store back with
that frame moved and its handles rehomed, then the freeze's thaw reads
every page back. **Still open:** the swap file itself (made at install,
its blocks found at boot) and the SD card's gather/scatter DMA behind
`Card`, then the device run.

### G9. Full Mode's profile: suspend and resume

Every Full Mode service's `suspend`/`resume` (design B's table), the
shell's snapshot return (R153), caches dropped on entry. Waits on K2.

### G10. Design B's tests

The test table of design B, on the X51v: transitions in and out (median
of 10), RAM freed against the budget within 5%, the session intact
(hashes of frozen pages), crash, hang and force-close returns 10 times
each, Half-Life entered and left 10 times; the CE legacy test once K4 is
built.
