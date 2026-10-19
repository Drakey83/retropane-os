# X51v board CPLD

> Spec v0.2, 2026-10-12. Facts only, per [`../CLEANROOM.md`](../CLEANROOM.md).
> **Sources:** (C) AxDroid board and Wi-Fi glue code (working on X50/X51
> hardware in 2010); (A) strings in Dell's `AximSDK.dll` from the owner's
> ROM; (D) **Dell's Windows Mobile 6.1 drivers** (ROM A08, owner's device),
> analysed locally for which CPLD bits each driver switches and in what
> order (no code from them is in this repository); (P) RetroPane runs on
> the owner's X51v.
>
> As far as we know this is the first public map of the X50/X51 CPLD
> beyond AxDroid's Wi-Fi and SD bits. The 2005-2009 handhelds.org Linux
> port never found the CF slot's power, and no public port ever made the
> CF slot work.

The X50(v)/X51(v) board has two Xilinx CoolRunner-II CPLDs (handhelds.org
port wiki); software sees one 16-register block that switches power
rails, resets and enables for the board's peripherals, and reports a few
status bits.

## Address and access

| Item | Value | Confidence | Evidence |
|---|---|---|---|
| Physical base | `0x10000000` (PXA chip select 4) | confirmed | C; D (Windows Mobile maps it at `0xBB800000`); P |
| Size | 16 registers × 16 bits, offsets `0x00`-`0x1E` (2-byte stride) | confirmed | C; D (16-bit accesses at even offsets) |
| **Writes don't read back** | Reading an offset does not return what was written. `0x18` read `0x0000` while Windows Mobile had its backlight bit on; `0x16` read `0x0000` with the SD card powered. Some offsets read status inputs instead (`0x04`) | confirmed | P (2026-10-12: reads before RetroPane's first write: `00=0048 04=00a0 06=0002 14=0002`, the rest `0000`) |
| **Shared shadow convention** | Because writes don't read back, every Windows Mobile driver changes bits through one shared shadow copy per register (kept in shared memory, updated atomically) and writes the whole new value. A driver that writes a register without knowing the other drivers' bits switches their devices off | confirmed | D (the same set/clear helper in 17 drivers); P (see "Lesson" below) |

**Lesson (P, 2026-10-12):** RetroPane's CPLD driver kept its own shadow
starting at zero. Holding the CF card's reset (`0x18` bit 1) wrote `0x0002`
to `0x18`, clearing the backlight enable (bit 2) that Windows Mobile had
set: the panel went dark but kept its picture. Fixed then by setting the
backlight enable before the first write; fixed as a class on 2026-10-15
by seeding the shadow from Windows Mobile's own table ("Shadow at
hand-over" below).

## Shadow at hand-over (item 77, 2026-10-15)

| Fact | Confidence | Evidence |
|---|---|---|
| Windows Mobile's shared shadow is ten 32-bit slots in RAM at physical `0xA8001320`-`0xA8001347` (its drivers use the uncached view `0xAC001320`). Slot *i* holds register `0x02, 0x10, 0x12, 0x14, 0x16, 0x18, 0x1A, 0x1C, 0x1E, 0x08` for *i* = 0-9; offsets `0x00`, `0x04`-`0x06`, `0x0A`-`0x0E` have none | confirmed (code) | D (`Backlight`, `ddi`: the helper's 10-way switch and its address pool; the kernel's reboot uses the last slot for `0x08`) |
| Each change is an interlocked compare-and-exchange of the whole slot, then a 16-bit store of its low half to the register. The upper half of a slot is never set | likely | D (the helpers OR or clear the caller's 16-bit bits) |
| The LCD-type readout's enable (`0x1E` bit 3) and the reboot bit (`0x08` bit 1) are set only for a moment: Dell sets the enable, waits ~1 ms (a 1 ms sleep), reads, clears it at once; the panel type is read once and cached | confirmed (code) | D (`Backlight`); AxDroid (C) waits 1 ms before and after |
| The table's values on the owner's X51v at hand-over (after a soft reset into Windows Mobile, then the card's loader): `02=00FD 10=1000 12=000F 14=0008 16=0003 18=0004 1A=0006 1C=000C 1E=0000 08=0000`; it passed every check and was adopted, backlight (`0x18` bit 2), SD and CF power kept. The next program resumed RetroPane's table (`RPCS`, check valid) with the same values. The LCD-type readout reads `0` at 0, 1 and 2 ms (Sharp 3.7" VGA) | confirmed | P, 2026-10-16, `rp-handover` runs 1-2 (`local/rp-handover-run1.txt`, `-run2.txt`) |

**RetroPane (2026-10-15, `rp_drv_cpld::handover`):** every program takes
the CPLD with the table (`devices::take_cpld` hands out the register window
and the 40 bytes at `0xA8001320`, reserved in memory-map.md; `Cpld::adopt`):

1. **RetroPane's table** (word 0 = `"RPCS"`, words 1-8 the sixteen registers,
   word 9 an FNV-1a check of words 0-8): resumed as is. The loader adopts
   first; every image it starts finds its table, and every change is
   written through, so one program's CF reset can't undo another's SD
   power.
2. **Windows Mobile's table**, adopted when it passes every check: no slot
   above 16 bits, the backlight enable on (HaRET runs with the screen lit),
   neither momentary bit set. Nothing Windows Mobile left on is switched off
   by accident: the whole class behind the blackout, not just its bit.
3. **Neither** (the emulator's empty RAM, a damaged table): RetroPane's own
   known-good defaults, `DEFAULTS`: backlight enable and SD power on (the
   two bits RetroPane needs that are confirmed on the device), all else off
   until a driver that owns it sets it. In this case nothing is written:
   registers no driver writes keep what is latched.

**The flash-installed OS (the end state):** no Windows Mobile runs, so
there is no table to adopt and nothing is latched but the CPLD's power-on
state. One CPLD driver (one owner, in the kernel's driver host) keeps its
shadow in its own memory (`Own`), writes `DEFAULTS` to every register at
start-up so the shadow is true from the first instruction, and the drivers
that own the rest add their bits in their own power-up sequences (the
display's guessed sequence bits in `0x12`, `0x14` bit 3, `0x18` bit 3 come
from item 72's own display set-up, measured, not copied). Same code paths
as today's: only the seed differs. Before that build, the remaining guessed
bits need their device experiments (open questions below).

**LCD-type wait:** one wait before the read (Dell's ~1 ms; AxDroid's second
wait after it is dropped: nothing reads after it). The shortest working wait
can't be measured on the owner's X51v: its Sharp panel reads `0`, the same
as an enable that hasn't taken effect yet. Copied, tagged open: measure on a
Samsung panel (ID 2), if one turns up.

## Register bits

"Users" are the Windows Mobile drivers that switch the bit (D); the
function column is what the bit does where known, or a name-based guess
from its users.

| Offset | Bit | Function | Users (D) | Confidence | Evidence |
|---|---|---|---|---|---|
| `0x02` | `0x0040` | Wi-Fi power sequence step 1 (AxDroid writes it at `0x00`) | `pcc_socket`, `tiacxwln` | likely | D; C (offset `0x00`) |
| `0x04` (read) | `0x0002` | **CompactFlash card present when clear** | `pcc_socket` | likely | D; P (`0x00a0`, bit clear, with the owner's card in) |
| `0x08` | `0x0002` | set by the kernel's reboot just before its board reset (GPIO99); purpose unknown | `nk.exe` (OAL) | likely | D (README.md "Software reset") |
| `0x10` | `0x0004` | Bluetooth/serial power (guess) | `serial32`, `BTuartDMA` | guess | D |
| `0x10` | `0x0008` | Wi-Fi power sequence step 5 | `pcc_socket`, `tiacxwln` | likely | D; C |
| `0x10` | `0x1000` | **CompactFlash slot power, stage 2** | `pcc_socket` | confirmed | D; P (the slot powers up and stays up) |
| `0x10` | `0x2000` | Wi-Fi power sequence step 4 | `pcc_socket`, `tiacxwln` | likely | D; C |
| `0x10` | `0x8000` | cleared by the notification driver (LED or vibration, guess) | `OemNotify` | guess | D |
| `0x12` | `0x0001`-`0x0008` | display sequence bits (LCD power-up/down, guess) | `ddi`, `ddi_2700g` | guess | D |
| `0x14` | `0x0002` | **Speaker amplifier** (on only for the speaker; 70 ms to settle) | `wavedev` | **confirmed** | D; P (Windows Mobile measured, `rp-wmaudio`; `rp-tone` run 9 played on the speaker, 2026-10-18) |
| `0x14` | `0x0004` | peripheral / serial (guess) | `peripheral`, `serial32` | guess | D |
| `0x14` | `0x0008` | display (AxDroid: audio amplifier, set at boot). **Clearing this or `0x1A` bit 1 for 1 s wedged the device** (no check-ins after it; hardware watchdog into Dell's factory prompt; rp-unplug run 1, 2026-10-16, docs/hw/udc.md T4): never cleared outside a run built to find out which | `ddi`, `ddi_2700g` | guess | D; C; P (the hang) |
| `0x16` | `0x0001` | **SD card power** | `pxa27x_sdhc` | confirmed | D; C; P |
| `0x16` | `0x0002` | **CompactFlash slot power, stage 1** | `pcc_socket` | confirmed | D; P |
| `0x16` | `0x0004` | Wi-Fi power sequence step 3 | `pcc_socket`, `tiacxwln` | likely | D; C |
| `0x16` | `0x0008` | Bluetooth/serial (guess) | `serial32`, `BTuartDMA` | guess | D |
| `0x18` | `0x0001` | serial / Bluetooth (cleared by both; guess) | `serial32`, `BTuartDMA` | guess | D |
| `0x18` | `0x0002` | **CompactFlash RESET** (set = card held in reset) | `pcc_socket` | likely | D (held 100 ms after power-up); P (attach works through it) |
| `0x18` | `0x0004` | **Backlight enable** | `Backlight`, `ddi`, `ddi_2700g` | confirmed | D; P (clearing it darkened the panel, setting it lit it again, 2026-10-12) |
| `0x18` | `0x0008` | display (guess) | `ddi_2700g` | guess | D |
| `0x1A` | `0x0002` | battery / charger (guess) | `battdrvr` | guess | D |
| `0x1A` | `0x0008` | notification (guess) | `OemNotify` | guess | D |
| `0x1C` | `0x0001` | Wi-Fi power sequence step 2 | `pcc_socket`, `tiacxwln` | likely | D; C |
| `0x1C` | `0x0002` | **Audio codec (WM8750) power** | `wavedev` | **confirmed** | D; P (set by Windows Mobile while audio plays; RetroPane's sound with it set, 2026-10-18) |
| `0x1C` | `0x0008` | USB (guess; AxDroid: "USB power?") | `usbfndrv` | guess | D; C |
| `0x1E` | `0x0001`-`0x0004` | battery / charger (guess) | `battdrvr` | guess | D |
| `0x1E` | `0x0008` | write 1 to enable the LCD-type readout | `AximSDK`, `Backlight`, `battdrvr`, `ddi` | likely | C; D |
| `0x1E` (read) | `0x0003` | **LCD type**: 0 = 3.7" Sharp, 1 = 3.5" Sharp, 2 = 3.7" Samsung, 3 = 3.5" Sharp. Even = VGA (X50v/X51v), odd = QVGA | `AximSDK` | confirmed | C; A; P (reads 3.7" Sharp VGA on the owner's X51v) |

## Sequences

**LCD type detection:** set `0x1E` bit 3 → wait ~1 ms → read `0x1E & 3` →
wait ~1 ms → clear bit 3.

**CompactFlash slot power and reset** (D; works on the device, P): see
[`cf.md`](cf.md) "Slot power and reset".

**Wi-Fi (ACX100) power-up** (C, confirmed in the same order by D, which
uses offset `0x02` where AxDroid writes `0x00`): Wi-Fi reset GPIO89 high →
~1 ms → `0x02 |= 0x0040` → ~1 ms → `0x1C |= 0x0001` → ~5 ms (D; AxDroid
waits 50 ms) → `0x16 |= 0x0004` → ~1 ms → `0x10 |= 0x2000` → ~1 ms →
`0x10 |= 0x0008`. The ACX100 then responds at `0x2C000000` (PC Card socket
0 common memory), interrupting on GPIO107.

**Wi-Fi power-down** (D): `0x02 |= 0x0040` → clear `0x10` bit 3 → clear
`0x10` bit 13 → clear `0x16` bit 2 → ~5 ms → clear `0x1C` bit 0, ~1 ms
between steps.

## Open questions

- ~~What Windows Mobile leaves on at hand-over, register by register~~:
  measured (`rp-handover` 2026-10-16, "Shadow at hand-over"; HaRET leaves
  the table intact); the emulator's CPLD latches and `emu-cf` /
  `emu-handover`'s table use those values (rp-v0.18.0).
- The LCD-type readout's shortest wait (needs a panel whose ID isn't 0).
- The guessed rows (Bluetooth, audio, display, battery, notification):
  each needs a device experiment before a driver relies on it.
- `0x08` bit 1: set by the kernel's reboot before its board reset; what it does.
