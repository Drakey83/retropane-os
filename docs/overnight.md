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

- [ ] Review the diff against the charter, re-run CI and the emulator, merge.
- Iterate: scan cost measured in read commands before/after; the write-back
  FAT cache kept coherent; power-loss sweep still green; KData live test.
- Modernize: n/a beyond review; record what the worker reported.

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
