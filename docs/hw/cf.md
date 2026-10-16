# X51v CompactFlash slot (PXA27x PC Card socket 1)

> Spec v0.2, 2026-10-12. Facts only, per [`../CLEANROOM.md`](../CLEANROOM.md).
> **Sources:** (M) Intel PXA27x Developer's Manual, memory controller and
> PC Card chapters; (C) AxDroid board code (`pxa2xx_aximx50.c`, via
> `docs/research/hardware-inventory.md`); (CF) CompactFlash Association
> specification (attribute memory, CIS, True IDE / memory / I/O modes);
> (P) RetroPane's probe on the owner's X51v; (W) the handhelds.org Axim
> X50/X51 Linux port's wiki (archived 2009), volunteers' observations;
> (D) Dell's Windows Mobile 6.1 PC Card socket driver (`pcc_socket.dll`,
> ROM A08, owner's device), analysed locally for its hardware accesses and
> their order (no code from it is in this repository).
>
> **Status:** the CF slot works on the X51v: RetroPane powers it from off,
> resets the card and reads it as a disk (2026-10-12). As far as we know no
> system besides Windows Mobile ever has: the slot's power switch (two
> CPLD bits) and reset (a third) were unpublished.

The X51v's PXA27x drives two PC Card sockets: **socket 0** is the built-in
ACX100 Wi-Fi (docs/research/hardware-inventory.md), **socket 1** is the
CompactFlash Type II slot. A CF card is a small ATA disk (or an I/O card:
Wi-Fi, modem, Bluetooth), so the driver to come is a PC Card socket layer
plus an ATA block device; the USB drive (ADR-0014) then offers it as LUN 1
with no USB work.

## Board (C, P)

| Item | Value | Confidence | Evidence |
|---|---|---|---|
| Socket | PXA27x PC Card socket 1 | confirmed (code) | (C) |
| Card detect | GPIO13, **active low: 0 = card present** | confirmed | (C); reads 0 with the owner's CF card in the slot (P, owner, 2026-10-12); WM arms only the rising edge (removal) |
| Card present (second source) | CPLD `0x04` bit 1, **clear = card in** | likely | (D); (P) `0x00a0` with the card in |
| Slot powered / ready | GPIO84, high while the slot is powered | confirmed | (D) read with the presence bit as the socket's status; (W) "1 when CF Card is on"; (P) 0 with the slot off, 1 after the power-up below, stayed high while RESET was held |
| **Slot power** | CPLD `0x16` bit 1 (stage 1) and CPLD `0x10` bit 12 (stage 2) | confirmed | (D); (P) the slot comes up from off and stays up |
| **Card reset** | CPLD `0x18` bit 1, set = held in reset | likely | (D); (P) attach works through it |
| Not the reset | GPIO36: AxDroid called it CF reset; (W) calls it a charger/cradle signal; Dell's socket driver never touches it; holding it changed nothing (P) | confirmed not CF | (C), (W), (D), (P). Windows Mobile leaves it an output driven 0; RetroPane leaves it alone |
| Wi-Fi socket (0), for contrast | status from GPIO107 and a Windows Mobile Wi-Fi state flag; power via GPIO89 and four CPLD bits (`cpld.md`) | likely | (D) |

## Slot power and reset (D; confirmed on the device, P)

What Windows Mobile does when a card is in the slot (CPLD `0x04` bit 1
clear), and what RetroPane's loader now does before it touches the card:

| Step | Action | Then wait |
|---|---|---|
| 1 | Power off: clear CPLD `0x10` bit 12, ~10 ms, clear CPLD `0x16` bit 1 | 308 ms |
| 2 | Power on: set CPLD `0x16` bit 1, ~10 ms, set CPLD `0x10` bit 12 | 100 ms |
| 3 | Hold RESET: set CPLD `0x18` bit 1 | 100 ms (Windows Mobile); RetroPane: **100 µs** (2026-10-16) |
| 4 | Release RESET: clear CPLD `0x18` bit 1 | until GPIO84 is high |

Power-down is step 1 alone. The CPLD's registers are shared with other
devices (SD power, Wi-Fi, the **backlight enable in `0x18` bit 2**) and
don't read back, so each step must change only its own bit, keeping
everyone else's from a shadow copy (`cpld.md`).

**Why it failed before (P, explained):** Windows Mobile leaves the slot
unpowered or powering down when HaRET hands over; the card then ran on
what was left for a moment (it worked, then everything read 0 and GPIO84
fell, 120 ms after the driver's commands), and a re-inserted card got no
power at all (GPIO13 then read "no card"). Nothing RetroPane did to the
card itself could revive it; switching the slot on does.

## PXA27x side (M)

Memory windows, socket 1 (socket 0 is the same at `0x2xxxxxxx`):

| Space | Physical base | Use |
|---|---|---|
| I/O | `0x30000000` | True IDE / I/O-mode registers |
| Attribute memory | `0x38000000` | the card's CIS (what the card is), configuration registers |
| Common memory | `0x3C000000` | memory-mode registers / data |

Memory controller registers (base `0x48000000`):

| Register | Offset | Fields | Confidence |
|---|---|---|---|
| `MECR` | 0x14 | `[0]` NOS: two sockets; `[1]` CIT: card inserted (set by software) | likely (M) |
| `MCMEM0/1` | 0x28 / 0x2C | common memory timing: `[6:0]` SET, `[11:7]` ASST, `[19:14]` HOLD (in memory clocks) | likely (M) |
| `MCATT0/1` | 0x30 / 0x34 | attribute memory timing, same fields | likely (M) |
| `MCIO0/1` | 0x38 / 0x3C | I/O timing, same fields | likely (M) |

Pins: the PC Card strobes (`nPOE`, `nPWE`, `nPIOR`, `nPIOW`, `nPCE1/2`,
`nPSKTSEL`, `nPREG`, `nPWAIT`, `nIOIS16`) are GPIO alternate functions
(M); the probe records `GAFR*` as Windows Mobile set them, the reference
for the driver's set-up.

## As Windows Mobile left it (P, 2026-10-12)

| Register | Value | Decoded (fields above) |
|---|---|---|
| `MECR` | `0x00000003` | two sockets, card inserted (possibly just the built-in Wi-Fi on socket 0) |
| `MCMEM0`, `MCMEM1` | `0x00020508` | SET 8, ASST 10, HOLD 8 |
| `MCATT0`, `MCATT1` | `0x00020E08` | SET 8, ASST 28, HOLD 8 |
| `MCIO0`, `MCIO1` | `0x00020510` | SET 16, ASST 10, HOLD 8 |

In time at CLK_MEM 208 MHz (decode calibrated on the device,
docs/research/storage-speed.md): common memory 260 ns a cycle (444 ns a
CPU access), attribute 519 ns, I/O 298 ns. The owner's card rates its
common memory at 80 ns (CIS device speed, IDENTIFY word 164). GPIO84
follows the card's RDY/-BSY in memory mode (low while it prepares a
sector), and the common-memory window 0x400-0x7FF reads the data register
at every address (32-bit loads and DMA from it work: measured).

## Card side (CF)

- Attribute memory holds the Card Information Structure: tuples from
  offset 0, on even bytes only (`CISTPL_DEVICE` 0x01, `CISTPL_MANFID` 0x20,
  `CISTPL_FUNCID` 0x21: 4 = fixed disk, `CISTPL_CONFIG` 0x1A gives the
  configuration registers' base, `CISTPL_END` 0xFF).
- A storage card is set to True IDE or memory mode through its
  Configuration Option Register; then it speaks ATA (IDENTIFY DEVICE, READ /
  WRITE SECTORS, with LBA28 addressing up to 128 GiB), 512-byte sectors.
- Reset: pulse RESET (line unknown on the X51v, see Board), then wait for READY (GPIO84) before the
  first attribute access; the pulse and wait times come from (CF) when the
  driver is written (not yet recorded here).

## Driver (`drivers/cf`, `rp-drv-cf`; emulator-verified, works on the device)

Memory mode, the one every CF storage card must support: configuration
index 0 in the Configuration Option Register (after a soft reset through
its bit 7), the ATA task file at common memory 0x0-0x7 (data at 0x0 as
16-bit accesses, the other registers as bytes), device control at 0xE
(nIEN set: the driver polls). The CIS walk (even bytes, bounded to the
first 1 KiB) must find `CISTPL_FUNCID` = fixed disk and `CISTPL_CONFIG`
with the configuration registers' base; anything else (a Wi-Fi or modem
card) is left alone with nothing written to it. IDENTIFY DEVICE gives the
LBA28 capacity (words 60-61, word 49 bit 9) and the model (words 27-46);
SET FEATURES 0x82 turns the write cache off so a completed write is on the
card (the crash-safe file system order needs that), refused by cards with
no cache, which is fine. READ / WRITE SECTORS move up to 256 sectors a
command, polled per sector (DRQ), every wait bounded.

**Read paths (2026-10-16, docs/research/storage-speed.md):** in memory
mode the card answers its data register at every address of 0x400-0x7FF,
and the PXA splits a 32-bit access into two 16-bit card cycles, so sectors
can come off by 32-bit loads across that window (+18% measured) or by a
memory-to-memory DMA copy from it (+25%; the PXA has no request line for
the socket, so the copy starts once DRQ is up, one sector at a time).
Neither changes a socket timing. At attach the driver reads IDENTIFY
through the 16-bit port and again through each faster path, and uses the
fastest whose bytes match; afterwards the card must have no data left
(DRQ clear), and any left over is drained through the port, so a path
that moves too little can't leave the card mid-transfer for the next
command (found in the emulator, whose socket took one card cycle per
32-bit load: sector 0 came back as IDENTIFY's tail). Every read command
ends with the same check. The loader gives the CF card DMA channel 2;
`cf` reports the path (`reads by ...`). Writes still use the 16-bit port.

`devices::take_cf()` (board HAL) sets socket 1's timing registers to the
values Windows Mobile used and `MECR` to two sockets / card inserted, then
hands out the attribute and common windows. The loader's `cf` command
(`cargo xtask` control frame `cf`) identifies the card read-only, and only
when GPIO13 reads a card. Since 2026-10-13 the loader also attaches a card
on its own when GPIO13 settles to "in" (at start-up or on insertion); an
empty socket is never touched.

The USB drive's second unit (LUN 1, "CF card") is the CF card: attached
when it goes in (or was in at start-up), as the SD card is, read-only until
`drive rw` (as the SD card), and back to empty when GPIO13 says the card
left. Windows shows it as a second removable drive. `cf` reports an
attached card without taking it again (and without a medium change, which
would make Windows remount it). A card in the slot at start-up is attached
before the USB pull-up goes on (2026-10-15), so the PC's first question
finds it ready, as the SD card: attached later in the loop, it was "no
medium" while Windows enumerated, then away for seconds, and the attach
stalled the PC's requests meanwhile; Explorer drew a bare "USB Drive (J:)"
with no size, or the label with a broken icon for minutes. A card that
fails to attach or stops answering is taken again once on its own (one
retry per insertion, a second later), then stays idle until it is
re-inserted or `cf` is sent: never a retry loop, which would power-cycle a
broken card forever.

Taking the card (2026-10-12, item 62): the loader power-cycles the slot
(steps 1-2 above), then `CfCard::init` holds RESET (CPLD `0x18` bit 1)
for 100 µs (since 2026-10-16; Windows Mobile held it 100 ms, which no
source explains: the CF specification's minimum pulse, tw(RESET), is
10 µs, and card makers' data sheets agree), waits 20 ms, then waits up to 3 s for READY (GPIO84)
without touching either window (a card in reset or busy may hold the
bus), then reads the CIS. A card that runs out a wait during a transfer is
**dead**: every later transfer fails at once without touching it, and the
loader lets it go (an empty reader for the PC, so Windows stops
retrying); re-inserting it or `cf` takes it again from a reset. So a stuck card costs one
timeout, never a stalled USB loop. `cf soft` takes it the old way, with no
power cycle or reset (diagnostics); `cf log` reports GPIO84's changes and
the attach and drop times since start-up.

The slot is powered only while the loader holds a card, as Windows Mobile
does: once the card is removed (GPIO13), dropped as dead, or fails to
attach, the loader switches off the power it switched on (step 1 above;
checked every loop pass, so every way of letting go is covered). Power
Windows Mobile left on in an empty socket is left alone.
`cf log` shows it as `power off`.

Not yet: the card's interrupt, I/O-mode cards (Wi-Fi, modem, Bluetooth
CF cards).

The emulator (rp-v0.11.0) models the slot as the device has it: as
Windows Mobile hands it over (powered since rp-v0.18.0, its CPLD table as
measured by `rp-handover`), answering only while both power bits are set
and RESET is released, reset through
`0x18` bit 1, GPIO84 high while powered; `cargo xtask emu-cf` checks
that the card in the slot at start-up is attached on its own (so the power
sequence works), `cf` reports it without a remount, the backlight stays
on, and a card pulled out switches the slot off, with no READY before the
loader attaches it again on re-insertion (rp-v0.11.1)
([`emulator.md`](emulator.md)).

## Safety for experiments

An access to a socket window with the card unpowered, or before its timing
registers are set, can hold the bus until the card answers, which on a
device with nobody there means a watchdog reset into Dell's factory prompt.
So the probe only reads the memory controller and GPIO registers (always
safe); attribute memory is read only once power-on and detect are
understood, with the owner present.

## Measured on the owner's X51v (2026-10-12)

**Result (third session, power sequence above):** the slot, dead from an
earlier run (GPIO84 0), came up; the card identified ("ULTIMATE CF CARD
8GB", FAT32), Windows mounted it as a drive and read 73 MB (three files)
in 120 s, about 0.6 MB/s through the polled driver and USB full speed,
with no errors and GPIO84 high throughout. The first attempt turned the
backlight off (the reset write cleared `0x18` bit 2, `cpld.md` "Lesson");
fixed the same evening, the second attach kept it on.

The history below is how it was found; its readings were superseded by
the power sequence.

### First device runs

Card: "ULTIMATE CF CARD 8GB", 16,250,976 sectors, MBR with one FAT32
partition; reads and writes fine under Windows Mobile in the same slot.

| Run | Result |
|---|---|
| First loader after Windows Mobile (HaRET start), `cf` | **Works**: IDENTIFY, write cache off, sector 0 read whole (`33 c0 8e d0 ...`, `55 aa`), status `0x50` after |
| A loader that had been reloaded over USB several times, `cf` | IDENTIFY and SET FEATURES work, but sector 0 comes back with its end zeroed (`00 00` for `55 aa`) and status reads `0x00` after; every later read fails (Windows retried sector 0 endlessly) |
| A loader started by another loader that had used the socket, `cf` | `not a storage card`: the CIS reads back as garbage |
| GPIO13 with the card in | low (so low = card in, as the driver assumes) |
| Card pulled out and pushed back in while the loader ran (card fully seated) | GPIO13 then reads **high** ("no card") and stays high: either GPIO13 isn't a plain detect line (the card's ready line would go quiet unpowered), or removal cuts the slot's power and only Windows Mobile restores it. Either way the loader must power and reset the slot itself. Also: a slot whose attach failed is never retried (`CfSlot::Failed` sticks), even after a re-insert |
| CPLD registers, read before RetroPane's first write | `00=0048 04=00a0 06=0002 14=0002`, the rest 0, including `0x16`: our only write (`0x16 = 0x0001`, SD power) clears nothing Windows Mobile set, so the CF slot keeps Windows Mobile's power |

Second session (2026-10-12 evening, loader with the hardware reset):

| Run | Result |
|---|---|
| Old loader, first `cf` after Windows Mobile (fresh boot) | **Fails too** this time: sector 0 ends in zeros, status `00`. So the fault is not (only) a hand-over |
| That loader, then `cargo xtask load rp-loader` | Transfer froze at 24,517 / 97,800 bytes (Windows retrying the dead card's reads held up the USB loop: item 62 (c), seen again) |
| Fresh boot, new loader, `cf` (GPIO36 held 1 ms, then released) | **Works**: FAT32 partition, status `50`. But GPIO84 stayed **high** while GPIO36 was held, so GPIO36 is not shown to be RESET (or 1 ms is too short to see READY drop) |
| Seconds later, Windows' first read of the CF unit | Failed: the card stopped answering; the loader let it go and **stayed responsive** (the stall fix works) |
| `cf` again | `timeout`: GPIO84 now reads **0** (was 1), and stays 0 for 3 s after the reset pulse |

Timed with `cf log` (fresh boot, new loader, 2026-10-12): READY 1 at
start; `cf` attached the card at 135,887 ms (reset, IDENTIFY, write cache
off, sector 0 read whole, status `50`); **READY fell to 0 at 136,007 ms,
120 ms later, before the PC touched the card**; Windows' first read then
timed out and the card was dropped at 138,353 ms (the loader stayed
responsive). So the card dies on its own right after the driver's burst
of commands, not because of the PC's traffic.

**Public record (W):** the handhelds.org Axim X50/X51 Linux wiki
(archived 2009, `DellAximX50Hardware`, observations by the port's
volunteers) lists **GPIO84 "= 1 when CF Card is on"**, i.e. the slot's
power status, not (only) the card's READY; and **GPIO36** as an OS-driven
output "set to 1 when charger/cradle power is connected", not CF reset
(contradicting AxDroid; our device run, where holding GPIO36 changed
nothing on GPIO84, fits the wiki better). The same page has SD detect
the wrong way round against our measurement (GPIO12), so it is a lead,
not proof. Its status table lists PCMCIA/CF as working on no model: no
public port ever powered the slot, so the power switch itself is
unpublished. Read with GPIO84 = power, the timing above says the slot's
**power turns off** about 120 ms after the driver's commands.

**Reading at the time:** the slot losing power shortly after RetroPane
started using it, which Dell's socket driver then confirmed (the power
sequence above). A finding that stays (a bug class, not CF only): while the
card was stuck, image transfers over USB stalled part-way (the loader's
loop waited on the card's long timeouts), so a dead card could lock the PC
out of the loader. Waits on a device must never hold up the USB loop; a
card that times out is now dropped after one timeout.

## Open

| Question | How to settle |
|---|---|
| GPIO13 polarity (present = 0 or 1?) | probe with and without a card (owner: one swap). Hint (P): WM arms only GPIO13's *rising* edge (`GRER0` bit 13 set, `GFER0` bit 13 clear); if that is the insertion event, present = 1 and the 0 read on 2026-10-12 means an empty slot |
| The reset set-up time after RESET is released (20 ms here, as Windows Mobile; data sheets give 20 ms or 100 ms for tsu(RESET)) | GPIO84 doesn't fall during reset on the X51v, so the card's own busy can't be seen there; settle from the CF specification's table, or by measuring how soon after release the task file answers with BSY |
| ~~Which CPLD bit powers the slot~~ | settled: `0x16` bit 1 + `0x10` bit 12 (D, P) |
| ~~Power the slot down when the card leaves~~ | done (2026-10-13, emulator-tested): the loader switches it off whenever no card is attached. **Seen on the device 2026-10-16: it breaks re-insertion.** The owner pulled the card and pushed it fully back in; GPIO13 then read "no card" and stayed so (`cf`: "no card (GPIO13 high)"), as on 2026-10-12: the detect line needs the slot powered. A soft reset (Windows Mobile powers the slot) brought it back. **Settled 2026-10-16 (`rp-cfdetect`, `local/rp-cfdetect-run2.txt`):** with the slot off, a pull-out set both GPIO13 and CPLD `0x04` bit 1 to "no card"; on the push back in, the CPLD bit returned to "card" while GPIO13 stayed "no card", even after the slot was powered again for 1 s. **Fix (main, same day):** the loader detects the card by the CPLD bit (`cf_card_in`; GPIO13 only without a CPLD), so slot power stays off when empty and re-insertion is seen. **Device check passed (same day):** with the fixed loader running, the owner pulled the card and pushed it back in; the loader counted both changes and attached the card again on its own (`cf`: "was card", detect changes 2), no soft reset. Emulator: GPIO13's stickiness is fork work (emulator.md)  **Boot time (same day):** the loader's start-up stages (`handoff`) showed the CF attach taking 457 of the 585 ms before the display: the 308 ms power-off and 110 ms power-up of a slot Windows Mobile had left powered. Now a slot found powered (both CPLD stages on, READY high) with its card in gets the RESET pulse only (a hard reset by itself); a slot the loader switched off, or a card inserted since, still gets the power cycle. `cf log` shows `W` for a warm attach. Emulator-guarded (`emu-cf`: the CF stage under 200 ms at start-up; mutant). Device check pending. |
| A card misread at start-up, now and then (2026-10-16) | Same loader, same card, two boots: the first time Windows found no partition table on the CF unit (one "FAT16" partition from offset 0, the whole card: what Windows does when sector 0 lacks `55 aa`), the next time it mounted the FAT32 partition at 32,256. The loader's own `cf` read sector 0 right both times. Fits the 2026-10-12 reads of sector 0 "with its end zeroed". To settle: when it happens, read the CF disk's sector 0 raw on the PC (admin, read-only: the owner allowed it) and compare with `cf`'s dump; then the drive's first CF reads (timing after attach, the read path) |
| Whether WM's `CIT` reflects socket 1 | probe with and without a card |
