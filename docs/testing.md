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

## Mutants: proving the tests bite

`cargo xtask mutants` applies deliberate bugs, one at a time, from the list
in [`xtask/src/mutants.rs`](../xtask/src/mutants.rs), and requires the named
tests to fail on each. `--emu` adds the slow ones that need the emulator.

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
