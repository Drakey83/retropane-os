# Testing rules

How RetroPane is tested, in practice. [ADR-0004](adr/0004-testing-policy.md)
is the policy; this page is the working rules, added to as lessons come in.

## Every change

1. **Host tests** for the logic, in the crate (`cargo test`).
2. **An emulator check that fails without the change**, in `cargo xtask
   emu`, whenever the change runs on the device (a line it must print, a
   screen it must show, a check it must pass). Before relying on a new
   check, break the change and watch the check fail.
3. **`cargo xtask ci` and `cargo xtask emu` green, judged by exit status**,
   never by searching the output for "error" (a format failure prints only
   one line and once slipped through).
4. **Only then** commit and push, as separate commands: a `;` between a
   check and `git push` pushes even when the check failed.

## Bugs: fix the class

A bug is a symptom of a class. Fix the class, audit the code that could have
the same mistake, and add a guard test that fails on the original bug. The
Dell factory-restore prompt was reached three times by the same class (long
work not feeding the watchdog) before the class was fixed: never again.

**Flaky tests are bugs**, investigated, never rerun until green. The one
flaky test so far ("USER preempted") was a real scheduler bug: a quantum
lost when the timer fired inside a system call.

## One device run at a time

There is one docked Axim and several sessions or scripts that may want it.
Two runs at once lose a load, drop the USB link until the Axim is reseated,
and can leave a hung device in Dell's factory-restore prompt. So the tool
enforces it, not a convention: a "pause file" whose existence means busy
was deleted in passing on 2026-10-15 and two runs overlapped.

- **Every command that talks to the Axim holds the machine-wide lock**
  (`%LOCALAPPDATA%\RetroPane\axim.lock`, an OS file lock, so a killed or
  crashed holder releases it). It is taken at the first contact with the
  port (`usb::powershell`, the only way xtask reaches the device) and held
  until the command ends: a `load` keeps it across the send, the device's
  reconnect and the run log, but not across the build before it.
- **A second caller waits**, printing `device busy: <command> (pid, checkout)
  since <time>, waiting`; `--no-wait` (or `RP_AXIM_NO_WAIT=1`) fails at once
  instead. `RP_AXIM_WHO=<label>` names a loop in that line.
- **`cargo xtask axim`** says who holds it, if anyone.
- **Never talk to the port any other way.** A loop or script goes through
  `cargo xtask` commands; nothing spawns its own serial or PowerShell port
  code. `xtask/scripts/usb-restart.ps1` is only for the scheduled task a
  locked `load` runs; don't run it by hand while the Axim is busy. A unit
  test fails if any xtask file other than `usb.rs` spawns PowerShell or
  opens a serial port, or if `usb::powershell` stops taking the lock (both
  have mutants).
- **A loop releases the lock between runs**, so other sessions can take
  turns. Pausing someone else's loop is a message to that session, never a
  file.
- **Emulator runs never share ports.** Each `cargo xtask emu*` process
  reserves its own block of 20 local ports (serial, monitor, QMP, USB
  chardevs) from 45440 up, with an OS lock per block in
  `%LOCALAPPDATA%\RetroPane\emu-ports\` (`devlock::reserve_ports`), and
  skips a block where any port is already in use. Fixed ports made two
  sessions' runs fail with "Address already in use" on 2026-10-15. Every
  emulator port is an offset from `monitor_port()`; a test fails if a USB
  chardev number reaches past the block (it found `msc_args(13)` doing so
  the day it landed). `RETROPANE_MONITOR_PORT` still pins the ports by hand.
- **Emulator checks wait on the image, never on the wall clock.** A busy
  host gives the guest few instructions per second while its timers keep
  real time, so a fixed hold or sleep that is plenty on an idle PC falls
  short on a busy one: emu-today's 700 ms stylus holds gave the align
  screen too few samples with every core busy (234960 px off, 2026-10-15,
  on two emulator releases alike). Waits are on serial markers, the screen
  or device registers; a test keeps emu-today's presses that way. The
  guest's own deadlines suffer the same: QEMU's timers follow the host
  clock, so a starved guest saw its time jump past the loader's 30 s run
  limit while checking in every 5 ms of its own time. emu-today runs under
  `-icount` (guest time from instructions), as the cycle-exact checks do.
  So does the threads image: its 5 ms quantum once ended before the first
  thread reached its system call, and the preemption-order check saw `BS`.
  Nor is the guest's work timed on the PC's clock: the drive's 1 MiB read
  took 8.7 s against an 8 s limit with every core busy, so the check now
  reads the loader's own count of naps mid-command (`drive status`), the
  slowdown the limit was there to catch. Tests keep both, and allow one
  wall-clock bound (the idle loader's 500 ms answer, 26 ms under load).
  `RETROPANE_EMU_LOAD=1` runs any check with one spinner per WSL core
  (a fixed count, each ended by `timeout`), to prove it; the full suite
  passes under it.
- **A long device run:** `cargo xtask load` waits 120 s for the image's run log; `RETROPANE_LOG_WAIT_SECS=1500` waits longer (`rp-gpubench` run G takes up to ~20 minutes). If the PC gives up first, `cargo xtask log` fetches the log once the image is back at the loader.
- **Watching a check:** `RETROPANE_EMU_SHOW=1` opens the emulated screen
  in a window while any check runs (SDL; from WSL through Windows 11's
  own Linux windows), so a person can see what the check looks for. For
  watching only: the window passes stray key presses to the emulated
  X51v and slows its timing (`emu-gpubench`'s watch steps ended early
  with it, 2026-10-19), so a check that counts runs without it. Checks that judge the screen keep their last screendump beside it
  (`target/emulator/<check>-screen.<step>.ppm`).
- **A run ends only its own processes.** Two sessions' runs share WSL, so
  cleanup finds this run's emulator and load by the monitor port in their
  command lines. A `pkill -x qemu-system-arm` once ended another session's
  rp-blend mid-check (2026-10-15, "terminating on signal 15"); a test
  checks the patterns against both runs' command lines.
- **Every check stands alone in a fresh checkout.** Scratch files go
  through `scratch()` and raw binaries through `raw_image()`, which create
  their folders and build what they need; screendump makes its folder and
  reports QEMU's own complaint. A check once assumed `target/emulator`,
  which only the release download created, and several reused an earlier
  check's `rp-loader.bin`; a test fails on a hand-built path.

## Mutants: proving the tests bite

`cargo xtask mutants` applies deliberate bugs, one at a time, from the list
in [`xtask/src/mutants.rs`](../xtask/src/mutants.rs), and requires the named
tests to fail on each. `--emu` adds the slow ones that need the emulator.
`RETROPANE_MUTANT=<text>` runs only the mutants whose name holds the text
(a change's own, say `backtrace`).

- **Every security or crash-safety rule with a test gets a mutant**: the
  region list's atomic reservations and page wiping, the pin limit, double
  frees, IPC rights never growing, LZ4 refusing matches before the output,
  the constant-time compare's length check, the pixel blend, the dirty
  rectangles' word alignment, and the soft run limit's FIQ routing.
- **Run it before committing changes to those files**, and `--emu` before
  a device run that depends on a safety path it covers.
- The runner first checks each mutant's tests pass on the real code and
  actually run tests (a filter matching no test name otherwise reads as a
  surviving mutant, which is how the first run found one), refuses files
  with uncommitted changes, restores every file even on error, and rebuilds
  the images after an emulator mutant so nothing built from a mutant is
  left for `cargo xtask load`.
- A unit test fails if any mutant's text no longer matches the code once,
  so refactors can't silently retire a mutant.

## Model tests

For data structures the kernel depends on (frame allocator, region lists,
IPC tables, dirty rectangles), a random-operation test runs thousands of
operations against a simple model and compares after each one. Failure
injection goes where failures really happen: memory runs out at a list
page's boundary, not at random (the first version of the region test never
reached a boundary, and a mutant proved it).

## Fuzzing: from the day a parser lands

Every parser of outside data (FAT and exFAT volumes, PE images, USB
requests, the loader protocol, CF card data, later TLS and 802.11) gets a
random-input test the day it lands: thousands of hostile inputs from a fixed
seed, in the normal test suite, so CI repeats them exactly. The rule is no
panics, no unbounded loops, errors instead. Each run and its findings go in
[security.md](security.md). Coverage-guided fuzzing (`cargo fuzz` on the
host) is the next step for the network-facing parsers (Phase 6).

## Benchmarks are tests

`cargo xtask bench` runs in the emulator in deterministic mode (one
instruction a nanosecond), with on-target checks (`CHECK name OK`), and
fails on a regression past the committed baseline. `cargo xtask emu` runs it
too. The same suite runs on the real X51v in the probe image, so emulator
and device numbers come from the same code.

## The emulator tracks the device

Every hardware fact learned on the Axim is modelled in the emulator fork, or
listed as not modellable in [hw/emulator.md](hw/emulator.md). Only
emulator-tested images go to the device, and nothing may outlive the
loader's 30 s run limit.
