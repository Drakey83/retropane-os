# Overnight queue (2026-10-12)

Worked top to bottom, one item at a time, while the owner sleeps. Each item
is committed and pushed when `cargo xtask ci` and `cargo xtask emu` are
green, then ticked here with what was done to **iterate** and to
**modernize** it. The standard (docs/AI-CONTRIBUTING.md): every way the part
can beat the original, not one token improvement.

**Device rules overnight:** only images that passed the emulator go to the
Axim; nothing that could outlive the loader's 30 s run limit (a watchdog
reset lands on Dell's factory-restore prompt, which needs the owner to press
Mail; if that happens, device work stops and emulator/docs work continues);
nothing writes flash; card writes only through copy-then-swap.

## 1. Merge the worker's branch: FAT free-space scan in runs, KData wiring

- [x] Reviewed (the HAL page-table change: one-shot guard, old mapping's
      cache lines cleaned first, vectors kept on the same physical page),
      merged as 6db35fc; CI and the emulator green on main. On the Axim the
      merged loader writes a file in 7–9 card reads in total; the stale-hint
      scan (3.7 s once) drops from ~1,490 to ~18 reads by the emulator's
      measure (25× fewer in the host test), to confirm on the card when a
      stale hint next occurs.
- **Iterate:** the free-space scan reads the FAT in doubling runs through
  the caller's scratch (no new buffers, no heap), the cache stays coherent
  without extra flushes, the power-loss sweep runs both ways, and the reply
  now reports card reads so every write is measured; KData is written
  through its one user address so a thread switch needs no cache
  maintenance, checked by a live test with two mutations that make it fail.
- **Modernize:** card operations are measured and gated in CI; KData's page
  is user read-only (verified by a faulting write), and the same change
  closed a hole that left the vector page and the first MiB of SDRAM
  user-writable.

## 2. ADR-0014: the SD card as a drive in Windows (USB mass storage)

- [x] ADR-0014 written and accepted, with a Security section.
- [x] `drivers/usb-msc` (Bulk-Only + SCSI, LUNs), composite descriptors and
      the mass storage endpoints in `pxa-udc`, capacity from the CSD in
      `sdcard`, the loader runs the drive next to the serial port.
- [x] Emulator rp-v0.9.0: descriptor-driven routing, the drive bridged to a
      second chardev; `boot_loader_drive` mounts the card through USB with
      `rp-fat`, reads and writes both ways, checks the ownership rules and
      hostile input. Two real bugs found on the way (ADR-0014 status).
- Done so far against the plan: driverless class, composite, ownership by
  construction (busy refusal, medium-changed, eject), multi-block 32 KiB
  transfers, card absence as "not present", stable serial, LUN-ready for CF.
  Write-protect is plumbed (`Storage::read_only`, MODE SENSE, refused writes)
  but not yet wired to the switch: GPIO27's polarity is unconfirmed, so it
  waits for item 10's probe run rather than risk a wrong read-only card.
- **Iterate:** composite instead of a mode switch; corruption made impossible
  by construction (the device and PC never write the card at once, the PC is
  told when the device changed it, eject hands it back) rather than "please
  eject first"; every SCSI command host-tested including 5,000 random
  commands; the whole stack tested end to end through the emulated drive
  before the device; two latent USB timing bugs found and fixed for the real
  hardware on the way; LUNs ready for the CF card with no USB work.
- **Modernize:** the card shows up as a drive on any 2026 PC with no driver
  or sync software (Windows Mobile never offered it at all); standard
  compliant serial so it keeps its drive letter; hostile-input-proof parser
  per docs/security.md; storage exposure noted as development-only until the
  OS asks the user for consent.
- Improvements planned, all of them:
  - driverless on Windows 11, macOS and Linux (standard class, no INF);
  - composite, so `cargo xtask load` keeps working while the drive is
    mounted, no mode switch;
  - the card is handed to the PC only while it's mounted, and taken back on
    eject (the device never writes a card the PC owns: no corruption);
  - multi-block reads/writes straight from the USB buffer to the card, no
    staging copy, so it runs near the card's measured 1.7 MB/s;
  - write-protect switch honoured (GPIO27) and reported as read-only;
  - card removal reported as "medium not present", not an I/O error;
  - a proper volume name and serial so Windows remembers the drive;
  - the protocol layer is generic over any block device, so CF (and later
    the internal flash) become extra drives with no USB work;
  - size: ~6-8 KiB in the loader, measured and logged.

## 3. Device run of item 2

- [x] Enumeration worked first time on Windows 11 (composite: COM4 + "RetroPan
      SD card", read-only volume, the right capacity), with a layout
      fallback in place first so nobody could get locked out.
- [ ] **Blocked on the card, not the code:** a weak block at 10880 in the
      first FAT (0.79 s alone, never inside a multi-block read) makes
      Windows' mount retry forever. Six driver fixes (sd.md "Reads that
      fail") keep everything else working; the block itself needs a scrub
      (one rewrite after a backup) or another card: the owner's call. The
      drive is **off** for the night (`cargo xtask drive on` to offer it).
- **Iterate:** a fallback layout so an unattended device run could never
  lock the loop out; diagnostics over the serial port that turned "it's
  slow" into a block number and a status word in minutes (`drive status`,
  `card scan`); an SD driver that now recovers from timeouts, previous-
  command error bits, stuck transfers and slow blocks, learns weak ranges,
  and an emulator that reproduces the slow block.
- **Modernize:** the drive is read-only until asked and can be withdrawn
  like an empty reader (`drive off`), so the PC is never stuck on it; the
  card's health problem is reported, not hidden.

## 4. FAT: create files (and directories' first step)

- [x] `rp-fat` creates root-directory files: `create`, and `put` (replace or
      create), used by the loader's file frames, so `cargo xtask put` and
      `install` create what the card lacks. 7 new host tests (20 in all),
      the emulator test creates "RetroPane loader.lnk" through the loader
      and finds it by long and short name in the image.
- **Iterate:** crash-safe order proven by a power-loss sweep (no file or
  the whole file, never a broken one; the other files untouched); a full
  root directory grows; a full card gives the clusters back; names FAT
  forbids are refused before anything is written.
- **Modernize:** long names exactly as Windows writes them, Windows'
  `~1`/`~2` short names, and lower-case 8.3 names stored with Windows NT's
  case flags (no long-name entries needed), so the card reads naturally on
  any 2026 PC.
- Improvements planned: crash-safe order (clusters, FATs, then the
  directory entry last: a power loss leaves either no file or the whole
  file); long-name entries Windows displays correctly; name collisions
  resolved the way Windows does (`~1`, `~2`); power-loss sweep extended.

## 5. CF card groundwork

- [x] `docs/hw/cf.md`: socket 1, detect/ready/reset GPIOs, the PXA27x
      windows and timing registers, the card side (CIS, ATA), safety rules
      for experiments, open questions.
- [x] The probe reports the PC Card timing registers and a decoded CF line;
      run on the Axim: both sockets set up identically by Windows Mobile
      (MCMEM 0x00020508, MCATT 0x00020E08, MCIO 0x00020510), MECR 3, detect
      GPIO13 = 0, ready = 1, reset driven 0. Attribute memory deliberately
      not read with nobody present (a bus stall would mean Dell's prompt).
- **Needs the owner:** is a CF card in the slot? (settles detect polarity);
  which CPLD bit powers the slot (Dell OAL analysis: Fable-level RE).
- **Iterate:** the spec separates what's confirmed on the device from what
  the manual says, and the probe captures the state the driver must
  reproduce, before any driver code exists.
- **Modernize:** planned as LUN 1 of the USB drive (no USB work), with
  LBA28 cards up to 128 GiB (stock Windows Mobile handled far less).

## 6. GPIO14 / GPIO2

- [x] Recorded docked + enumerated: GPIO2 = 1 (output), GPIO14 = 0 (input),
      WM's both-edge interrupt on GPIO14 still armed (docs/hw/udc.md).
- [ ] **Needs the owner:** the same probe with the Axim *out* of the cradle
      (one undock, `cargo xtask load rp-probe` needs USB, so: undock, tap
      HaRET with the probe kit, photograph page 2's GPIO lines). That pair
      of readings settles GPIO14.

## Security pass (owner's requirement, 2026-10-12)

- [x] `docs/security.md`: principles, threat model, current exposures,
      per-parser checklist; charter and memory updated.
- [x] Hostile-input tests for every parser of outside input:
      `rp-loadproto` (20,000 random stream chunks), `rp-usb-msc` (5,000
      random commands), `pxa-udc` (50,000 random SETUP packets plus 2,000
      through the whole driver), `kernel/pe` (5,000 mutated images through
      every entry point), `rp-fat` (1,500 corrupted cards, looping FAT and
      root chains, out-of-range clusters, absurd geometry).
- **Found and fixed:** `rp-fat` trusted the card's FAT size and root
  cluster: forged values overflowed the sector arithmetic (a panic in test
  builds, silently wrong sectors in release). Mount now validates the whole
  geometry with checked arithmetic, so every cluster the code later trusts
  is in range. Listed in docs/security.md.
- **Iterate:** every parser now has a test that throws garbage at it, so a
  future change that reopens a hole fails CI.
- **Modernize:** the threat model's "hostile files" and "someone with a
  cable" rows are now backed by tests, not intentions.

## Card repair (owner approved, 2026-10-12)

- [x] `cargo xtask card scrub|repair|peek`: the weak block 10880 turned out
      to hold *damaged* FAT data (118 of 128 entries impossible values),
      with an intact, live mirror in the second FAT. `card repair` restored
      it under strict evidence rules (mirroring on, the mirror's entries all
      valid, at least 90% of the differing entries invalid in the damaged
      copy), verified it: reads in 0.9 ms instead of 796 ms.
- [ ] **Device reset:** a scan of the FAT's last 1,901 blocks outlasted the
      run limit (the scan fed the watchdog only once; fixed: it feeds per
      block). The Axim is on Dell's factory prompt until the owner presses
      Mail; then: re-scan blocks 15,074-16,975, then `cargo xtask drive on`
      and check Windows mounts the drive.
- **Iterate:** never write from belief: compare with the card's own backup,
  judge the evidence, refuse when it's ambiguous (the tool refused twice
  before the rules matched the facts), verify after.
- **Modernize:** a self-healing path for FAT damage that stock Windows
  Mobile would have needed a PC and `chkdsk` for.

## Second tier (when the above is done or blocked)

Each held to the same standard: every improvement that makes sense, tested
on the host or in the emulator first.

7. **Crash log that survives a reset** (ROADMAP Phase 2): **done.**
   `devices::crash_to_loader` (one shared path for faults and panics):
   message into the run log, run marked crashed, straight back to the
   loader, which keeps the report on the card (`crashlog.txt`, crash-safe
   write, before anything else) and sends it to the PC; `cargo xtask load`
   fails with the reason. Hangs (watchdog) are saved the same way. New test
   image `rp-crash` (6.6 KiB) panics on purpose; `cargo xtask emu` checks
   the PC report, no watchdog trip, and the file on the card. On the Axim:
   the panic came back in about a second, report on the PC and the card.
   - **Iterate:** a crash costs a second instead of a watchdog reset and a
     trip through Dell's prompt (which needed the owner); one crash path
     for every image instead of copies; tested end to end.
   - **Modernize:** crash reports persist and reach the developer by
     themselves, like modern crash reporting, instead of a photo of the
     screen.
8. **Performance suite v1** (ROADMAP): **device numbers in.** `rp-threads`
   tees its results into the run log and returns to the loader, so `cargo
   xtask load rp-threads` gives the real X51v's figures: all 13 tests pass
   on hardware for the first time; switch ~12 µs (an open lead against the
   2 µs target), interrupt-to-thread ~8 µs, sleep lateness ~17 µs; memcpy
   ~95 MB/s already measured. docs/perf.md.
   - **Iterate:** the numbers come from the device in seconds, so the
     switch-time lead can be chased one suspect at a time.
   - **Modernize:** honest measurement instead of assumptions: the switch
     misses its target and says so.
9. **exFAT reading** in `rp-fat`: **done.** `rp_fat::exfat` from the
   published specification: boot-region checksum and every field checked at
   mount, entry-set checksums, contiguous and FAT-chained files, zeros past
   the valid data length; a host formatter builds test cards; 5 tests incl.
   2,000 hostile cards. The probe falls back to exFAT and its emulated card
   is now exFAT ("exFAT 4096 B/cluster", file listed).
   - **Iterate:** checksums verified (boot region and every entry set), so a
     damaged card is refused or the damaged entry skipped, not misread;
     separate module so only images that read cards pay for it.
   - **Modernize:** SDXC cards up to 2 TB readable; stock Windows Mobile
     stopped at FAT and 32 GB.
10. **SD card detect and write-protect** (GPIO12/GPIO27): **done.** From
    the owner's probe: detect active low (0 = card in), lock high = locked.
    The loader reads both once a second (debounced: two readings must
    agree), hot-swaps (a card put in is initialised on the kept controller,
    `SdCard::try_init` hands the controller back on failure; one taken out
    is "no medium" to the PC), and honours the lock everywhere (USB drive
    writes, `put`, `install`, scrub, repair). Emulator rp-v0.9.3 wires both
    lines; `boot_loader_locked` checks a locked card refuses a file and
    stays unchanged (the emulated card itself stays writable, as a real one
    does, so ignoring the switch fails the test).
    - **Iterate:** hot swap without a restart; debounced switches; a card
      that answers is used even if the switch reads oddly (a wrong polarity
      can't hide it).
    - **Modernize:** the lock switch respected as users expect from any
      card reader; the drive tells the PC when the card changes.
11. **Backlight** (PWM): **done in the emulator; device run pending (the
    Axim is on Dell's prompt).** docs/hw/backlight.md; `rp-drv-pxa-pwm`
    (4 tests): perceptual levels, a floor above 0, full duty at 100,
    16-step fades, Windows Mobile's period kept (audible-whine reasoning
    recorded); `cargo xtask backlight [0-100]`; emulator rp-v0.9.4 models
    PWM0-3, and `boot_loader_backlight` checks the registers via the
    monitor (new `Monitor::query`).
    - **Iterate:** levels that look linear, smooth fades, no accidental
      black; the existing period respected; registers checked by a test.
    - **Modernize:** brightness controllable from the PC; the fade and the
      perceptual scale are what users expect of a 2026 device.
12. **Compatibility corpus scanner** (ROADMAP Phase 3 start): **done.**
    `tools/apiscan` (host tool on `rp-kernel-pe`): import tables of every PE
    file in a corpus, ordinals resolved through the corpus's own export
    tables (unresolved 34,893 -> 3,259 once the XIP region's `coredll` is
    in), ranked by importing modules into `docs/research/api-backlog.md`:
    519 modules of the owner's ROM, 4,361 functions; `coredll` leads with
    1,315 functions used by 432 modules (`LocalFree`, `memset`, the stack
    cookie pair, `CloseHandle`, `memcpy`, the registry...). Host-tested
    (no emulator part: it's a PC tool). Clean-room: interface facts only.
    - **Iterate:** the CE personality gets built in the order real
      programs need it, measured, not guessed; re-runnable on any corpus
      (third-party apps next).
    - **Modernize:** a data-driven compatibility plan instead of a
      reimplementation of everything WM had.
13. **Docs sync**: ROADMAP and DESIGN brought up to date with tonight's
    work (SD writes, FAT, USB client, emulator models), coverage check.
