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
| K1 | **The kernel has never run on the X51v.** `rp-kernel` (VM, compressed RAM, scheduler, IPC, PE processes, supervised hosts, the display host) passes in the emulator only; Phase 2's exit ("multiple processes running, with protected memory, on device and emulator") is open. Only `rp-threads` (the scheduler's 13 tests) and `rp-quantum` ran on the device (2026-10-12, 2026-10-15) | Phase 2 | every device measurement of the freeze, the thaw and the switch; R270 (the game as a process, measured against the bare test image) |
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
K1 lets it). G1 is built; the rest in this order.

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

### G2. Freeze a process's threads

Take every thread of a frozen process off the run queues (and its timers
and waits held, not cancelled), so a frozen program can't run, touch a
page and undo the freeze; put them back as they were on the way out.
Rule 0: CE has `SuspendThread` per thread (a count, racy with waits);
Linux's cgroup freezer and Android's freezer stop a whole group at once
and let in-flight system calls finish; ours, the same at the scheduler
(ADR-0010), one flag per process checked where a thread is made ready,
so a freeze is one pass over the process's threads and a wake while
frozen is queued, not lost. Emulator test in `rp-kernel`.

### G3. The return record

A region reserved at boot (256 KiB, the owner's ceiling, design B), in the
kernel's own area outside every slot: the frozen processes, their threads'
records, the suspended services' records, the full tier's page map; a
versioned, checksummed, bounds-checked format so a crashed game can't
corrupt it and a damaged record is refused, not followed. Rule 0: consoles
keep the system's state in a reserved partition of megabytes; Android
saves an app's state into a Bundle per app (often lost); ours, a quarter
megabyte, and only the record (the processes stay where they are).

### G4. The large-allocation region (R363) and 1 MiB sections

At least 64 MB of address space outside the 32 MB slots for a Game Mode
port's modules and big blocks (ADR-0003 as amended 2026-11-23), mapped in
1 MiB sections where the memory is contiguous (ADR-0039 rule 1: the
PXA270's translation caches then cover the whole game with no page walks,
as the bare test image's 1:1 map does). Rule 0: CE5 maps its large
`VirtualAlloc`s there at 4 KiB pages; ours maps the game's in sections
because the game's frame time is the number that matters (to measure
against the bare image: R270).

### G5. The kernel on the X51v (K1)

`rp-kernel` loaded by the resident loader as a test image (as `rp-threads`
was), its VM, IPC and supervision steps run on the device, its `BENCH`
lines (the freeze and thaw's among them) read from the run log. A device
run: the owner's go first.

### G6. Exclusivity grants and the mode manager

The display, input and audio hosts take a grant token for one slot and
reset their device on revoke; the mode manager runs the switch, the 5 s
watchdog and the one way out (quit, crash, hang, force-close). The forced
way back (hold Power 3 s, the owner's ruling) is built after the check
design B asks for: what Windows Mobile itself does on a held Power on the
X51v (R366, a WM step for the owner's go). Waits on K2 for the real hosts;
the protocol and the watchdog are built and tested against stand-in hosts
in the emulator first.

### G7. Half-Life's runner as Game Mode's one program

The runner (ADR-0039) started by the kernel into the large region, its
modules in sections, measured against the bare test image (R270): frame
time and translation misses. Waits on G4 and G5.

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

### G9. Full Mode's profile: suspend and resume

Every Full Mode service's `suspend`/`resume` (design B's table), the
shell's snapshot return (R153), caches dropped on entry. Waits on K2.

### G10. Design B's tests

The test table of design B, on the X51v: transitions in and out (median
of 10), RAM freed against the budget within 5%, the session intact
(hashes of frozen pages), crash, hang and force-close returns 10 times
each, Half-Life entered and left 10 times; the CE legacy test once K4 is
built.
