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

- [ ] ADR: composite device (serial port for development + mass storage),
      bulk-only transport, the SCSI commands Windows, macOS and Linux use.
- [ ] Driver: the mass-storage function over the existing UDC driver, the
      SD card behind it as a block device.
- [ ] Emulator: the fork's built-in host enumerates the composite device and
      reads/writes the drive; a `cargo xtask emu` boot mounts the FAT32 image
      through it and checks a file.
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

- [ ] Only once the emulator test passes. If Windows' enumeration stalls,
      write down exactly what was seen (docs/hw/udc.md) and leave the rest
      for a Fable session instead of retrying blind.

## 4. FAT: create files (and directories' first step)

- [ ] `rp-fat` creates root-directory files (short names, with a long-name
      entry when needed), so `cargo xtask install` can put files the card
      doesn't have yet.
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
