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

- [ ] `docs/hw/cf.md`: the PXA27x PC Card socket, the X51v's CF power (CPLD)
      and card-detect, from the PXA manual and the board sources, every row
      with its confidence and evidence.
- [ ] The probe reports the socket's state read-only (power, detect, the
      card's attribute memory if present). No driver writes yet.

## 6. GPIO14 / GPIO2

- [ ] A probe run that records both lines' levels and edges across docking,
      undocking-free events (USB reset, host open/close); settle what they
      are or narrow it down, docs/hw/udc.md updated.

## Security pass (owner's requirement, 2026-10-12)

- [x] `docs/security.md`: principles, threat model, current exposures,
      per-parser checklist; charter and memory updated.
- [ ] Hostile-input tests for every parser of outside input that exists:
      the loader protocol (`rp-loadproto`), FAT (`rp-fat`, a card can be
      hostile), PE (`kernel/pe`), USB setup packets (`pxa-udc`), and the
      mass-storage commands as they're written. No panics, no out-of-range
      access, overflow-safe arithmetic; findings fixed and listed.

## Second tier (when the above is done or blocked)

Each held to the same standard: every improvement that makes sense, tested
on the host or in the emulator first.

7. **Crash log that survives a reset** (ROADMAP Phase 2): faults and panics
   written to the reserved log area with a checksum, shown by the loader
   and sent to the PC at the next start, so a crash is never lost.
8. **Performance suite v1** (ROADMAP): context switch, IRQ latency, memcpy
   bandwidth, in the emulator baseline and on the device via the loader,
   numbers in docs/perf.md.
9. **exFAT reading** in `rp-fat`: SDXC cards (64 GB+) ship as exFAT; stock
   Windows Mobile can't read them at all. Host-tested with a formatter.
10. **SD card detect and write-protect** (GPIO12/GPIO27): driver support,
    emulator model, probe line.
11. **Backlight** (PWM): spec from the PXA manual and board sources,
    driver with brightness steps and a fade, a probe test that restores
    Windows Mobile's level afterwards.
12. **Compatibility corpus scanner** (ROADMAP Phase 3 start): a host tool
    that reads the import tables of WM6 executables (the `kernel/pe`
    parser already handles 12 real DLLs) and ranks the APIs RetroPane
    must provide.
13. **Docs sync**: ROADMAP and DESIGN brought up to date with tonight's
    work (SD writes, FAT, USB client, emulator models), coverage check.
