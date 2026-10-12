# ADR-0014: The SD card as a drive on the PC (USB mass storage)

- **Status:** Accepted (2026-10-12, by the project owner: overnight queue)
- **Date:** 2026-10-12

## Context

Docked, a 2026 device shows its storage in the PC's file manager. Windows
Mobile never did: WM 6.1 appears to the PC as an ActiveSync (RNDIS) device,
and files move through ActiveSync or by pulling the card. RetroPane's
loader (ADR-0012) is a USB serial port; ADR-0013 added file writes through
it (`cargo xtask put`). The owner asked for the SD card (and later the CF
card) to appear under "Devices and drives" on Windows 11.

What exists: the PXA27x UDC driver (`drivers/pxa-udc`), SD block reads and
writes with multi-block commands (`drivers/sdcard`, measured 1.7 MB/s each
way on the owner's card), `BlockDevice` in `rp-hal-api`, the emulator's UDC
model with a built-in host that enumerates like Windows.

## Decision

1. **A composite device**: the CDC-ACM serial port (interfaces 0 and 1,
   grouped by an Interface Association Descriptor) **plus** a USB Mass
   Storage interface (interface 2: class 0x08, SCSI transparent command set,
   Bulk-Only Transport) with its own bulk pair (endpoints 0x04 OUT, 0x85
   IN). Device class 0xEF/0x02/0x01 ("uses IADs"): Windows 10/11, macOS and
   Linux bind their built-in serial and storage drivers, no INF. The serial
   port keeps working while the drive is mounted, so `cargo xtask load` and
   `put` work at the same time; nothing switches modes.
   - The PXA27x only runs endpoints of the active interface (docs/hw/udc.md),
     so the hardware endpoint table puts all five endpoints in interface 0,
     as it already does for the serial port's data pair.
   - `bcdDevice` goes 26.00 → 26.01 so Windows enumerates the new layout
     afresh instead of reusing what it cached for the serial-only device.
   - The serial number becomes 16 upper-case hex digits (`5250323630303031`,
     "RP260001" in hex), as the Bulk-Only Transport specification requires;
     Windows uses it to give the drive the same letter every time.
2. **`drivers/usb-msc` (`rp-usb-msc`)**: the Bulk-Only Transport and SCSI
   command set, hardware-independent and host-tested, over two small traits:
   a bulk pipe (one packet in, one packet out, one in flight) and the
   storage behind each logical unit (`BlockDevice` plus size, presence and
   write protection). Logical units, not one fixed device: LUN 0 is the SD
   card; the CF card becomes LUN 1 when its driver exists, a second drive
   with no USB work.
   Commands: TEST UNIT READY, REQUEST SENSE, INQUIRY (with the serial
   number VPD page), READ CAPACITY(10), READ FORMAT CAPACITIES,
   READ(10)/WRITE(10), VERIFY(10), MODE SENSE(6)/(10) (write-protect bit),
   PREVENT/ALLOW MEDIUM REMOVAL, START STOP UNIT (eject), SYNCHRONIZE
   CACHE(10); anything else fails cleanly with ILLEGAL REQUEST sense.
   Class requests: Get Max LUN, Bulk-Only Mass Storage Reset.
3. **Who owns the card.** Windows caches a mounted file system, so the
   device must not change the card behind its back:
   - the drive appears as **removable**: Windows then uses "quick removal"
     (no write caching), so the loader disconnecting to run an image never
     loses a write that Windows reported done;
   - **eject** in Windows (START STOP UNIT) hands the card back to the
     device: the LUN reports "medium not present" until the device has
     changed the card and offers it again;
   - after the loader writes a file itself (`cargo xtask put`), the LUN
     reports **UNIT ATTENTION, medium may have changed** on the next command,
     so Windows drops its cache and re-reads the volume, as when a card is
     swapped in a reader;
   - the loader refuses a `put` while the PC wrote to the card in the last
     2 s (`RPL ERR card busy: the PC is writing`), so the two never
     interleave writes.
4. **Data path**: a 32 KiB buffer (the loader's scratch area), filled or
   drained with one multi-block card command per 64 sectors, streamed as
   64-byte packets. Full-speed USB (12 Mbit/s) caps the drive at about
   1 MB/s; the card (1.7 MB/s) isn't the limit. Measured and logged.
5. **Emulator**: the UDC model's built-in host reads the configuration
   descriptor like a real host, finds the CDC data and mass-storage
   interfaces by class, routes each bulk pair by endpoint number, sends Get
   Max LUN, and bridges the storage pipe to a second chardev. The serial
   "open" requests no longer block the rest: a drive works with no program
   on the serial port, as on Windows. A `cargo xtask emu` boot speaks
   Bulk-Only Transport over that chardev, mounts the FAT32 card image with
   `rp-fat` *through the USB drive*, reads a file, replaces another, and
   meanwhile sends a `put` over the serial port and expects the UNIT
   ATTENTION.

## Status notes

- 2026-10-12, emulator: built. `rp-usb-msc` (Bulk-Only Transport, the SCSI
  commands above, two logical units possible; 13 host tests including a
  5,000-command random-input run, wrapping LBAs, wrong directions and
  lengths, bad LUNs, card errors mid-transfer); the UDC driver's composite
  descriptors, mass storage endpoints and class requests (18 tests); the
  SD driver reads the card's capacity from its CSD. The emulator's UDC
  model (retropane-qemu rp-v0.9.0) learned the device's layout from its
  descriptors and bridges the drive; `cargo xtask emu` mounts the card
  image *through the drive* with `rp-fat`, reads a file, replaces another
  over SCSI, sees a serial `put` refused while the PC writes ("card busy")
  and accepted 2.5 s later, then "medium changed" on the drive, then both
  files right; junk transfers, an out-of-range and a wrapping read fail
  cleanly. The image on the host holds both writes afterwards.
- Found on the way, in the emulator: card set-up after the pull-up made the
  host's first request time out (fixed: the card is set up first); the
  emulated host sent its first request 0.1 ms after a bus reset instead of
  USB's 10 ms, and a driver handling the reset afterwards lost the request
  (fixed in the model, and the driver now keeps a SETUP that has already
  arrived). Both would have bitten the real device.
- Device run: pending (overnight queue item 3).

## Security

- **Input from the cable is hostile until checked.** The CBW parser and
  every SCSI command check signature, length, LUN, LBA and block count
  (with overflow-checked arithmetic: `lba + count` can't wrap past the
  card's end) before touching the card; a malformed or unsupported command
  is a failed CSW with sense data, never a panic, hang or out-of-range
  card access. Host tests feed it truncated, oversized and wrapping
  commands.
- **Exposure.** In the development loader the drive is always on, like the
  loader itself (docs/security.md: development only). In the OS proper the
  card is offered only after the user allows it on the device for that
  connection, as phones do.
- **No silent writes.** Write protection is honoured; device-side writes and
  PC writes never interleave (rule 3), so neither side can be tricked into
  writing over the other's view of the file system.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Composite, so development and the drive coexist; ownership rules that make corruption impossible by construction instead of "please eject first"; LUNs ready for CF; every command host-tested and the whole stack tested end to end in the emulator before the device |
| ✨ Modernize | The device shows up as a drive on any 2026 PC with no driver and no sync software: drag and drop files, like every phone and camera since; compliant serial number so the drive keeps its letter |
| 🗄️ Legacy support | The card stays plain FAT32 (or exFAT later), readable by Windows Mobile and any PC |
| ⚡ Better / faster | Multi-block card transfers through a 32 KiB buffer, so the USB link, not the card, is the limit; the loader's serial path is unchanged |

## Alternatives considered

- **MTP** (what phones moved to): no exclusive ownership problem, but a much
  larger protocol, an object model on top of the file system, and slower;
  worth revisiting for the OS proper, where the device keeps using its card.
- **Mode switch** (serial *or* storage): simpler descriptors, but every
  switch is a re-enumeration, and the hands-off loop would lose its port
  whenever the drive is wanted.
- **Exposing the FAT volume as files over the serial port** (an `xtask`
  file manager): no Windows integration at all, which is the point.

## Consequences

- The loader grows (USB descriptors, the storage stack, the buffer is in
  the scratch area so RAM is not static); a test image, ratcheted with a
  logged reason.
- Windows sees a new device once (new `bcdDevice` and serial): the COM port
  number may change once. `cargo xtask` finds the port by VID/PID, so
  nothing in the tools changes.
- Overnight (owner asleep) the real card is only read through the drive;
  Windows writing to it is left for when the owner is there.
