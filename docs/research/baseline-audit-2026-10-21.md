# Baseline audit of 2026-10-21: what was done, and where we disagree

The owner's brief (`baseline-audit-fix-prompt.md`, from the full-tree audit
at `022cec0`): fix the ten defects in order, file each as a quality-audit
item (C29-C38), give every commit its alternatives, prove hardware
behaviour on the device or the emulator, and report back, disagreements
included. This is that report.

**Where it stands:** all ten are fixed in code (defect 8 as its own step,
as the owner ruled). Every fix has host tests and mutants, and the emulator
suite passes. **None is proven on the X51v yet.** Four device runs are
built for the fixes that touch hardware (below); until they pass, those
fixes count as unproven.

## The ten defects

| # | Item | Commit | What changed |
|---|---|---|---|
| 1 | C29 | `825dbce` | One frame limit, `MAX_FRAMES = 2^14`, shared by the compressed-store handle, the aged-page payload and the frame allocator. The allocator refuses at compile time to own more frames than a stored page-table entry can name. Tests: the last frame round-trips through `l2_stored`/`l2_decode`, one past it is refused, `Frames<MAX_FRAMES + 1>` doesn't build. |
| 2 | C30 | `cd62490` | `protect()` upgrades now land: the fault path widens a mapping narrower than its region (Zero and shared-File pages, and `prepare_user_write`). **Sibling found:** release, reclaim and the aged restore judged a private page's owner by its write permission, so after a downgrade, unmap gave a process's private copy back to the cache and reclaim dropped it (data lost). Ownership now comes from the page cache (`PageCache::holds`). |
| 3 | C31 | `75b8686` | `restore()` decodes the running point through `Raw::point` (CCSR), the one decoder for read registers. Host test with CCCR and CCSR disagreeing. **New, found building the device run:** C42 (below). |
| 4 | C32 | `f3a9514` | One range check (`on_card`) at the top of `read`, `write`, `read_many` and `write_many`, before any command (the owner asked for all four). `Error::OutOfRange`, and `BlockError::OutOfRange` at the block-device interface. A capacity of 0 refuses everything. **Sibling found:** the HAL's own card-log writer (`cardwrite::sd_write`) had the same gap; it now takes the capacity the loader hands over. The loader on the card gets it at its next install. |
| 5 | C33 | `0969ed7` | Images over 32 MiB (one process slot) refused at parse, so every validated value is small. RVAs read from the image summed through one checked helper (`rva_at`). `page_protection`, `Relocations::apply` and `Exports::name_at` bounded. The fuzzer writes whole-word edge values and calls every entry point. 13 mutants. |
| 6 | C34 | `2559e33` | Bounds from the FAT32 specification: mount refuses a FAT over 2^21 sectors and over 0x0FFF_FFF5 clusters, so a FAT position times 128 can't overflow (check and USB write guard). Directories stop at 65,536 entries. **Sibling found:** the guard's `allows()` walked `lba..` past `u32::MAX`; a wrapping host write is refused. |
| 7 | C35 | `43bf48e` | `Ipc::thread_exit` ends a thread's call depth (the owner's per-thread hook). The kernel ends every thread through one `exit_thread` that pairs the scheduler's and the IPC's exits. The pairing and both lifecycles are written in the crate docs. |
| 8 | C38 | `511c9ee`, `4f3eff8` | Its own step (owner's ruling). One crash path for every image, `rp-image` (`boot/image`). Faults, panics, `crash` and `fail` run in a fixed order: the image's put-back step, the serial line, the crash box, the run log, the crash flag, then back to the loader. 38 images moved and every hand-rolled handler deleted. The kernel no longer spins. `cargo xtask lint` refuses a crash path outside `rp-image`, the loader and the HAL. `emu-crash` runs a panic, a data abort, a prefetch abort and an undefined instruction. **Found with it:** 47 silent stops in 20 images (the audit counted 8 images), unplug/reconnect reporting a crash as a clean finish, cfdetect and handover reporting a missing device as one. `4f3eff8` fixed a 15% `sd_read_block` regression the new panic handler exposed (an inlined formatted assert in the 8-bit register accessors). |
| 9 | C36 | `0bb1887` | Comments only (owner's ruling): the driver's register comments now follow `docs/hw/cpld.md`, with each bit's confidence. No code or value changed. |
| 10 | C37 | `1b94deb` | A command the FIFO refuses (full for 100 ms) drops the frame. It's counted in `fifo_full`, separately from `stalls` (hangs). Nothing more of the frame is queued and it isn't shown. The background, cached areas and next two frames are drawn again. The Today screen's run log and serial marker carry the count. **Sibling found:** before this, a frame missing commands was shown and marked whole. |

The 36 copies are gone. Mutants: 197 of 197 killed before C38 (172 host,
25 emulator); C38's 4 new ones: see the run with this report.

## Where I disagree with the reviewer

The owner's second reviewer read these and conceded all eight
(2026-10-21).

1. **Defect 1, the boundary:** the brief says the X51v's pool is exactly
   16,384 frames. It's 12,288 (the 48 MiB pool), so the bug can't be
   reached today. It's still real, and fixed (the owner said fix
   0-16383 too). The fix is a compile-time bound rather than the suggested
   runtime assert. A runtime assert is a no-op in release, and that's how
   this bug got in.
2. **Defect 2, eager upgrades:** the brief says apply upgrades in
   `protect()` straight away. I widened them lazily in the fault path. An
   eager walker would have to repeat the fault path's copy-on-write
   decision for every page, and it would still leave the fault path's
   spurious-fault loop for any other narrow mapping. The cost of lazy
   widening is one minor fault per page touched after an upgrade.
3. **Defect 3, "one word":** the one-word fix (`raw.ccsr`) would leave a
   second hand-written decode next to `Raw::point`, which is how the swap
   happened. Building its device run then found something the brief
   didn't cover (C42, below).
4. **Defect 5, the import walker:** the table and thunk sums the brief
   names can't overflow, because the read just before them refuses any
   range past `u32::MAX`. The IAT slot sum is the real one (it's never
   read), and the targeted test reaches it. All of them go through
   `rva_at` anyway, so the walker follows one rule.
5. **Defect 6, a step budget:** I used the specification's own limits
   instead of a step budget we'd pick ourselves. A budget of our choosing
   could be hit by a large card that is perfectly legal. The 2 MiB
   directory limit and the size-bounded file chains are limits no legal
   card exceeds.
6. **Defect 8, "then reset":** the brief says report the crash, then
   reset. The harness goes back to the loader with the crash flag set
   instead. A reset on the X51v lands on Dell's restore screen and costs
   the owner a hard reset by hand; the loader keeps the log and is ready
   for the next run. The audit also undercounted: 47 silent stops in 20
   images, not 8 images.
7. **Defect 9, the CPLD bits:** the reviewer was wrong here (the owner
   agreed). `cpld.md` already proves both bits on the device by sound, and
   the latches are write-only, so a read-back run can't measure anything.
   The driver's comments were stale; they're fixed. One more option: the
   audio service's bit constants could move into the driver so there is
   one source. That's a code change the ruling didn't ask for, so it's
   yours to call.
8. **Defect 10, count only:** counting alone wasn't enough. The refused
   command was silently lost, and if the engine caught up, the frame was
   shown and marked whole with commands missing. That frame is now
   dropped. Counting it without dropping it would have made the numbers
   honest while still showing the wrong picture.

## New items filed

- **C39** (open): on a crash, most images put back nothing they changed.
  Only clocks, cs3bus, gpubench, mbx3d and light register a put-back. The
  audio images leave the CPLD's speaker and codec bits set, for example.
- **C40, C41** (open): a picture in `local/` that arrives older than the
  build is missed; the emulator check runs whatever local build exists,
  not the pinned release (it cost an hour on rp-v0.26).
- **C42** (fixed, your option b): `clocks::restore` put back the running
  point but left a pending `CCCR` request armed when no relock was needed,
  while after a relock it wrote the running point's multipliers. It now
  always ends with `CCCR` naming the point running, both ways the same.
  Run F checks it, and an emulator mutant drops the write.
- **C44** (fixed): an abort's crash line always said SPSR 0 (found by run F).
- **C45** (fixed): emulator checks didn't size-check variants (found by run FF).
- **C46** (fixed again, your option a, waiting for guard version 5): reads failed after a read of the card's last block. Guard version 4 measured the card raising the flag 76 us after the read (20 of 20); the driver now polls CMD13 until it shows, ~1 ms at most.
- **C47** (fixed): 2D commands took any rectangle; on the device an inverted one writes past its picture.
- **C43** (open; your call: the faked reading first, the real stall
  decided after it): the FIFO-full device run (C37)
  fakes the FIFO's "full" reading. The only way known to really stall the
  2D engine is to stop its clock (`MBXCLK` enable 0). Dell's driver writes
  that on its way to 0x5, but whether the MBX block still reads with its
  clock off, or freezes the bus (a watchdog reset to Dell's screen), is
  unmeasured. If you want it tried, it would be one bit, with you
  watching.

## Found on the way: C17's cause

The full emulator suite on the C38 branch caught C17 again (one failed
audio ring end in `emu-audio-s2`, seen twice before and never with its
registers). This time the ring logged them: `DDADR` named the link
after piece 28 while `DSADR` was 96 bytes into piece 29. It was a torn
read: the channel loaded its next descriptor between the two register
reads. The class is fixed on the `device-runs` branch: every poll of a
DMA channel (the audio ring and the SD, CF and USB descriptor streams)
reads `DDADR` again after the others, and reads all of them again until
it hasn't moved. It has host tests, C17's own reading among them, and 2
mutants. It isn't proven on the device or by repeat emulator runs yet.

## The device runs: results (X51v, 2026-10-21)

| Run | Result | Log (`local/`) |
|---|---|---|
| guard, C32 | **Passed for C32:** every block off the card refused in under a microsecond with no command sent, by the driver's four calls, the block-device interface and the card-log writer; a run crossing the end refused whole; nothing written. **Found:** after the last block was read, the next reads failed (C46); version 2 logs why | `rp-sdtime-guard-run1.txt` |
| F, C31 and C42 | **Passed, 13 checks, 0 BAD:** a `CCCR` write alone changed nothing; after a deliberate data abort the crash path kept 624 MHz running (not the 208 MHz C31's bug would have chosen) and left `CCCR` naming it. The first device proof of C38's crash path. **Found:** an abort's crash line always said SPSR 0 (C44, fixed) | `rp-clocks-runF1.txt` |
| FF, C37 | **Passed, 11 checks, 0 BAD,** after the zeroed-RAM budget was raised 2 KiB (the emulator checks hadn't size-checked the variant: C45, fixed): the FIFO's level reads full on the device and a busy engine is never refused; with the level faked full, the frame is dropped, not shown, not a hang, and the next one is identical to a normal draw | `rp-gpubench-runFF1.txt` |
| guard version 2, C46 | **Diagnosed:** a read of the last block makes the card report `OUT_OF_RANGE` (`0x80000900`) in the next command's status; a CMD13 clears it. Fixed with your option (a); version 3 checks it | `rp-sdtime-guard-run2.txt` |
| P2L version 2 | **Cause found:** my test's own 2D fill, `(0, 280, 480, 40)` given as a size to a command that takes corners, wrote far past the screen on the device (the emulator drew nothing) and blackened the reference. C47: every 2D command now refuses such a rectangle. Renders 1-25 exact; 1 wrong bar pixel in 1,000, not attributable (the stray fill), version 3 lists them | `rp-gpubench-runP2L2.txt` |
| P2L version 3, ADR-0023 | **Passed:** 0 wrong or differing pixels in 1,000 32-bit renders under the 16-bit screen (307.2 million), 237 and 213 wrong bar pixels in the 32-bit controls. Vote B verified | `rp-gpubench-runP2L3.txt` |
| guard version 3, C46 | **The fix failed:** the driver's CMD13 straight after a last-block read comes before the card raises the flag; a CMD13 milliseconds later catches it. C46 open again: measure the delay first | `rp-sdtime-guard-run3.txt` |
| P2L, ADR-0023 | **Half measured:** 0 wrong bar pixels in 1,000 renders under the 16-bit screen, ~47 a render in the 32-bit controls. **The comparison half failed as a measurement:** the first render read all black by the 16-bit phase, a device-only effect; version 2 hashes it after every step and makes it again under the 16-bit screen | `rp-gpubench-runP2L1-card.txt` |

## The device runs as built

Each one is a variant of an existing test image, and each keeps its log on
the card. All four pass their emulator checks.

| Run | Proves | Image | Time | Touches |
|---|---|---|---|---|
| **FF**, C37 | FF1: 300 full-screen fills queued at once, so the real FIFO reads full and a busy engine is never refused. FF2: the 2D renderer on the real engine with the FIFO reading full for 150 ms: the frame is dropped, counted apart from hangs, and not shown. The next frame is whole and matches the same frame drawn without a drop. | `rp-gpubench`, `RP_GPUBENCH_FIFO_FULL` | under a minute | nothing outside the 2700G (no clock change) |
| **P2L**, ADR-0023 | 1,000 renders at 32 bits with the screen scanned out as dithered RGB565, each checked and compared with the first. Before and after, 5 control renders with the screen at 32 bits, where P2Y found errors. If the controls find none, the count can't tell us anything. | `rp-gpubench`, `RP_GPUBENCH_LONG_COUNT` | about 10 minutes; the screen counts | nothing outside the 2700G |
| **F**, C31 | `CCCR` set to ask for 2N 2 (a lower turbo, same memory clock) with no frequency change; the core counted to show nothing changed; the clocks saved in that state; then a deliberate data abort. The crash path's put-back must keep the running point, then write `CCCR` back as found. It's also the first device proof of C38's crash path. | `rp-clocks`, `RP_CLOCKS_FAULT` | seconds | `CCCR` only (no clock change; put back) |
| **guard**, C32 | Blocks off the end of the card refused by the HAL's card-log writer and by all four driver calls, with no command sent (the read count doesn't move, and each refusal takes microseconds). A run crossing the end is refused whole, and the last block reads and stays as it was. | `rp-sdtime`, `RP_SDTIME_GUARD` | seconds | nothing: every write it tries is off the card and carries the last block's own bytes |

None changes a clock, and none switches a CPLD or power bit.
