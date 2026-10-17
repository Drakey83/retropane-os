# ADR-0014: The SD card as a drive on the PC (USB mass storage)

- **Status:** Accepted (2026-10-12, by the project owner: overnight queue)
- **Date:** 2026-10-12
- **Last verified:** 2026-10-13 (against commit 3158b26)

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

> **As built (note added 2026-10-13):** point 4 changed with item 58.
> When the card reads by DMA (`Storage::overlaps`), the 32 KiB buffer is
> used as two 16 KiB halves: one half goes to the PC while the next 32
> sectors are read into the other, and the 16 KiB after a read are read
> ahead for the next command (`drivers/usb-msc/src/lib.rs`, `read_step`).
> Without DMA, and for writes, it is still one 64-sector command per
> buffer. No device figure for the SD drive's speed is logged in
> docs/perf.md: what is recorded is the emulator's 1 MiB read in ~2.8 s
> (perf.md) and the CF drive's ~0.6 MB/s on the device (docs/hw/cf.md).

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
   card; the CF card is LUN 1 (since 2026-10-12; attached on insertion since 2026-10-13), a second drive
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
   - after the device changes the medium (the loader writes a file itself,
     `cargo xtask put`, `rm`, card repair; a card in or out; `drive rw/ro`)
     the medium goes **away**: "medium not present" until the host has been
     told so twice and 2 s have passed (at most 10 s if it isn't asking),
     then **UNIT ATTENTION, medium may have changed** once, as when a card
     is pulled from a reader and pushed back. Windows drops its cached view
     only once it has seen the medium gone: a unit attention alone left
     Explorer showing the old root (item 65, 2026-10-12; amended
     2026-10-13). A unit the host last saw empty, or never asked about,
     skips the away time (nothing cached): the unit attention alone
     (2026-10-15, the CF card's flaky drive);
   - changes made while the loader isn't running (an image writing the
     card, the crash report the loader keeps on return, Windows Mobile):
     the X51v's pull-up is hard-wired, so Windows can re-attach without
     noticing the device went and keep its cached FAT and directories;
     writing them back lost a file (noise-ri.bin, 2026-10-15). So each
     unit's first poll, read or write after the loader starts that doesn't
     follow a READ CAPACITY (which every fresh mount sends first) is
     answered "no medium" and the medium goes away as above, with the card
     untouched; a fresh mount sees no delay (amended 2026-10-15). On the
     device (2026-10-15, loader from RAM, Explorer open on the card,
     drive read-write): after `rp-crash` the drive went away and came back
     in ~3-6 s, Windows' next write landed, and every root file hashed the
     same as before (only the new crash report differed). Windows logs
     each refusal as disk event 51 ("error during a paging operation"):
     expected, not a fault. Open: on the first try the device came back
     and Windows never configured the serial + drive layout (13 bus
     resets, 91 setups, then the serial-only fallback; a reseat didn't
     help, a soft reset into the card's loader did). Five later runs
     (main's and this loader, normal return and crash, read-only and
     read-write with Explorer open) didn't reproduce it; that try alone
     had Windows write and read ~20 MB through the drive just before.
     Seen again after soft resets (2026-10-16: 11 resets, 34 setups) and
     a watchdog reboot (27 resets, 0 setups); Windows' logs show the
     composite configured at the first try right after the controller was
     re-enabled (07:12:15), so the fallback works by re-enabling the UDC,
     not by its layout. Diagnosis, ranked hypotheses (the lead: state
     Windows Mobile leaves in the UDC that `Udc::enable` doesn't reset)
     and the test plan: `docs/hw/udc.md` "The first enumeration after a
     hand-over from Windows Mobile". A USB trace on the PC (T1,
     2026-10-16) then showed the mechanism: Windows never sees HaRET's
     hand-over (the pull-up stays, Windows Mobile's idle device stays
     "present"), so the loader is enumerated only via the hub's recovery
     or a re-plug; fixed 2026-10-16: the loader makes the PC see an unplug at start and
     at each layout switch (the PXA27x host controller drives SE0 on port
     2), proven over five hand-overs from Windows Mobile with the
     composite configured each time (`docs/hw/udc.md` T4);
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
- **Layout fallback (safety net):** if the PC keeps enumerating (bus
  resets) but never configures the composite device within 25 s, the loader
  offers the serial-only layout, byte for byte what it presented before this
  ADR (checked against the old source: same descriptors, revision 26.00,
  serial `RP26-0001`), and alternates while the PC keeps failing. No host
  can lock the development loop out, and a device run is safe with nobody
  at the Axim. Tested in the emulator with a host that refuses composite
  devices (`accept-composite=off`, rp-v0.9.1); with the fallback disabled
  the test fails.
- **Read-only until asked (2026-10-12):** the drive starts write-protected
  (MODE SENSE's WP bit, writes refused with DATA PROTECT sense); `cargo
  xtask drive rw` (a new `RPC1` command frame in `rp-loadproto`) makes it
  writable and `drive ro` locks it again, each followed by "medium changed"
  so the PC re-mounts with the new protection. The card holds the loader
  and HaRET, and Windows writes a mounted drive by itself (its System
  Volume Information folder), so nothing reaches the card unless asked.
  The emulator test checks both states and the switch.
- **Writable, boot files guarded (2026-10-12, owner's choice):** the SD
  card is now writable by default, like any USB drive (drag and drop,
  rename the volume, delete), except what boots the device. The PC writes
  sectors, not files, so `rp_fat::Guard` records the boot files'
  directory entries (name, attributes, first cluster, size), their chains
  in every FAT copy and their data clusters, plus the partition table and
  the boot sector's geometry, and refuses any write that would change
  them (DATA PROTECT, so Windows says "write protected"). Everything else
  passes: new files and folders, other files' changes and deletes, the
  volume label. Formatting the card is refused too. The guard is rebuilt
  from the card after start-up, a card change and anything the PC tool
  asks (`put`, `rm`, `install`, an image's run); when it can't be built
  (exFAT, a boot file in more than 8 pieces, unmirrored FATs) the card is
  read-only, as before. `drive ro` still locks it; `drive rw` also makes
  the CF card writable (it stays read-only by default until it stays
  attached, item 62). Device run: Windows wrote and deleted files; its
  delete of `default.txt` was refused on the card (Windows' cached view
  showed it gone until the drive was offered again, item 65); the boot
  files stayed identical to a backup.

> **Correction (note added 2026-10-13):** in the next note, block 10880
> did not stay unreadable: its data was damaged, and `cargo xtask card
> repair 10880` restored it from the second FAT the same day (read time
> 795,659 µs → 900 µs), which settled the scrub-or-new-card decision
> (docs/hw/sd.md, "Reads that fail").

- **Device run (2026-10-12, overnight):** Windows 11 enumerated the composite
  device first time: "USB Composite Device" with "USB Serial Device (COM4)"
  and "RetroPan SD card USB Device", read-only volume, 3,997,171,712 bytes
  (= the CSD capacity, 7,806,976 blocks). The serial port kept working
  alongside (every later `load`, `drive`, `card` command went through it).
  Reads worked (tens of MB) except one range: the owner's card has a weak
  block at 10880 inside the first FAT that reads in 0.79 s alone and never
  within a multi-block read (docs/hw/sd.md). Windows reads the whole FAT
  at mount, so it retried that range indefinitely and its storage stack
  stalled. Six driver fixes followed (sd.md "Reads that fail"), each with a
  host test, and the emulator now models the slow block; the block itself
  stays unreadable on this card once provoked. The drive was switched off
  for the night (`cargo xtask drive off`: "no medium") so the PC stays
  responsive; `cargo xtask drive on` offers it again. Decision for the
  owner: scrub the block (one rewrite of its own contents, after a card
  backup) or use another card.
- Added on the way: `cargo xtask drive status` (counters, last card error
  with the status word), `cargo xtask card scan <lba> <count>` (block by
  block, slowest read), `drive on|off`.

## Security

> **Correction (note added 2026-10-13):** a malformed CBW (wrong length
> or signature) gets no CSW at all: it is ignored and counted
> (`Stats::bad_cbw`), as Bulk-Only Transport expects, and the host's
> reset recovery follows (`take_command`; test
> `malformed_cbws_are_ignored_and_the_next_command_works`). Only
> well-formed commands that are unsupported or out of range fail with a
> CSW and sense data.

- **Input from the cable is hostile until checked.** The CBW parser and
  every SCSI command check signature, length, LUN, LBA and block count
  (with overflow-checked arithmetic: `lba + count` can't wrap past the
  card's end) before touching the card; a malformed or unsupported command
  is a failed CSW with sense data, never a panic, hang or out-of-range
  card access. Host tests feed it truncated, oversized and wrapping
  commands.

> **Correction (note added 2026-10-13):** not "always on": the loader
> offers the drive at start-up (`Lun::offered`, `boot/loader/src/main.rs`)
> and `cargo xtask drive off` withdraws it (status notes above).

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
