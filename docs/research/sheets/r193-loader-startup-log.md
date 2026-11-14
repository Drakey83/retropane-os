# Sheet: R193, the loader's own start-up and USB log on the cards (rule 0, before code)

The owner's X50v (Windows Mobile 2003, started from the X51v's CF card by
HaRET) runs the loader but never enumerates on USB: `cable 0 configured no
open no`, `usb resets 30 setups 0`, the layout fallback alternating
(the owner's photo, 2026-11-13; docs/hw/udc.md "The X50v"). The loader
can't be reached over USB there, and today it writes a card log only for
an image's run (`card_log_begin`); its own start-up stays in RAM and on
screen. This sheet is the way to read the X50v's USB state back through
the X51v. The owner's "go" (2026-11-13) covers building it; the device run
waits for his go and "ready" as always.

## What it must record

- **Once, as found (before the loader writes anything):** `CKEN`, `PSSR`,
  `GPLR0-3`, `GPDR0-3`, every `GAFR` word (GPIO14, the X51v's cable-detect
  guess, and every other candidate pin with them), the GPIO edge and wake
  registers (`GRER`, `GFER`, `PWER`): `rp_hal_pxa27x::diag`'s whitelist,
  whose reads have no side effects.
- **The UDC's port registers** (`UDCCR`, `UP2OCR`, `UP3OCR`): read as soon
  as the loader holds the UDC and before its first write to it. Reading a
  PXA27x unit with its clock gated is not defined by the manual, and the
  X50v's question is exactly whether `CKEN` bit 11 was on; so they are read
  after `take_udc` turns the clock on (as the loader always has), and the
  log says whether the clock was on before.
- **The CPLD:** Windows Mobile's shared shadow table at `0xA8001320`
  (`0xAC001320` in Windows Mobile's map) raw, before `Cpld::adopt`, and
  the adoption's verdict (RetroPane's table, Windows Mobile's, or the
  defaults with the reason): Windows Mobile 2003 may keep no table there.
  Register `0x1C` as read on the bus (the latches may not read back), as
  in the table, and as the loader keeps it (the board record, quality
  audit C39), with bit 3 named: the lead (Dell's X50v `usbfndrv` sets it).
- **Then each USB event with its time:** bus resets, setups, configured,
  the PC's port open, the cable pin, the layout fallback, the "PC silent"
  replug; a heartbeat every 10 s with `UDCCR` and `UP2OCR`. For 5 minutes
  or until the PC configures the device, whichever first; a run that
  starts closes it first and opens its own as today.

## What others do (findings, not the answer)

- **Windows Mobile 5 / Dell:** the USB function driver's messages are
  debug zones (`DEBUGMSG`) and CeLog, read only with a kernel debugger over
  KITL, which needs the very link that fails; nothing is kept on a card.
  REA: neither the X50v's nor the X51v's `usbfndrv.dll` writes a file.
- **Linux:** `dmesg`, a RAM ring lost at reset; `pstore`/`ramoops` keep it
  across a warm reset in reserved RAM, read at the next boot, on the same
  device. `printk` to a console UART needs a serial cable (the X50v's
  FFUART is in the cradle connector: no cable here).
- **Modern flight recorders** (Android's `last_kmsg`, aircraft FDRs):
  append-only records to storage that survives the device, read elsewhere.
- **Ours until now:** the card log for an image's run (ADR-0013's writes,
  `hal/x51v` `cardlog`/`cardsink`), and the loader's own lines on screen
  and the FFUART only.

## The ways

| | Way | For | Against |
|---|---|---|---|
| A | **The loader's own card log in a run slot** (`rplog/log-SS.txt`, image name `loader`, reserved as a run's is, before the PC sees the device), lines queued in RAM and written a sector at a time when USB is quiet | readable through the X51v with today's `cargo xtask cardlog loader <n>` and `ls cf:rplog`; no FAT change after the reservation (R22), crash-safe (a reset keeps every written sector); the same bytes on both cards | one of the 32 slots per HaRET start; ~0.5 s more at the first start (a slot zeroed on each card) |
| B | A file put with `rp-fat`'s `put` at the end (copy-then-swap, crashlog.txt's way) | one whole file, one write | nothing on the card if the X50v hangs or loses power before the end; a FAT change under the PC each time. Rejected |
| C | A RAM log kept across a soft reset (`pstore`'s way), read by the next loader | no card writes | the X50v's next loader can't reach the PC either, and the CF card goes to the X51v with no RAM. Rejected |
| D | Every loader start (returns from images too) | the same log for every start | halves the run logs' 32-slot history and adds ~0.5 s to every `cargo xtask load` return, where the run's own log already says everything. Rejected: the first start after HaRET only |
| E | Write each line at once, as a run's card log does | nothing waits in RAM | a 1-140 ms card program between USB control requests could change the enumeration being diagnosed. Rejected for the USB events: queued, written when USB has been quiet 100 ms or the queue is half full |

**Proposed: A, first start only, queued writes (E's reason).** Why ours
beats the originals here: Windows Mobile's and Dell's logs need the link
that's broken; Linux's survive a reset but stay on the device; ours starts
before the loader's first register write, survives power loss sector by
sector, and is read on another device. Nothing the loader does to the
board changes: the code reads only (the owner's hard rule: CPLD `0x1C`
bit 3 stays as the loader keeps it; switching it is a later, watched step).

## The ADRs' rules (`docs/adr-areas.md`: boot/loader)

- **ADR-0012** rule 1: the `udc` layer stays polled and unchanged; the log
  reads its counters and `Udc::read_reg` (its bring-up reads) only. Rule 4:
  the development loop is unchanged (the log closes before any run).
- **ADR-0013** rule 2: the only file-system change is the slot's
  reservation, the run logs' crash-safe `reserve`; after it, sector writes
  inside the reserved file only.
- **ADR-0014** rule 3 (who owns the card): the reservation runs before the
  PC is shown the device, and each unit's first look after a loader start
  is answered "no medium" anyway (amended 2026-10-15); the writes stay
  inside a file that already exists (R22); the log closes when the PC
  configures the device, before the drive serves its first command.
- **ADR-0017** rule 4: the log holds registers and counters, no key or
  pairing state.
- **ADR-0036** rules 1-2: the reservation goes through `AnyVolume` (FAT12,
  16, 32, exFAT); the slot files are not boot files, so the guard is
  unchanged; a locked SD card gets no log.
- **ADR-0037** rule 2: MTP is unchanged; the log is closed before the PC
  configures any layout, so no store event is owed.

## How it is proven

- Host tests of the log's queue and sectors (edges: a sector's end, the
  file's end, a unit refusing, the queue full counted, both cards the
  same bytes, nothing after closing) and of the snapshot's decoding.
- The emulator: the full suite (`cargo xtask emu`, ~1 h), and a check
  that the emulator's card holds a `loader` entry with the snapshot.
- On the device, with the owner's go: `cargo xtask install cf` on the
  X51v, the X50v booted from that CF card, the log read back on the X51v.

## What would overturn it

- The X50v's loader never mounts the CF card (its socket set-up differs):
  then no log, and the screen's new line says so; the fallback is the
  SD-less path via the screen photo.
- The first start's extra ~0.5 s measurably delays the X51v's enumeration
  enough to matter: then reserve after the PC configures (the log kept in
  RAM until then).
- A log that changes the X50v's enumeration (E's risk): compare resets and
  setups against the photo's 30 / 0.
