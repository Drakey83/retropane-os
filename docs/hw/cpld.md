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
set: the panel went dark but kept its picture. The driver now sets every
bit the loader depends on (the backlight enable) before its first write
to a shared register.

## Register bits

"Users" are the Windows Mobile drivers that switch the bit (D); the
function column is what the bit does where known, or a name-based guess
from its users.

| Offset | Bit | Function | Users (D) | Confidence | Evidence |
|---|---|---|---|---|---|
| `0x02` | `0x0040` | Wi-Fi power sequence step 1 (AxDroid writes it at `0x00`) | `pcc_socket`, `tiacxwln` | likely | D; C (offset `0x00`) |
| `0x04` (read) | `0x0002` | **CompactFlash card present when clear** | `pcc_socket` | likely | D; P (`0x00a0`, bit clear, with the owner's card in) |
| `0x08` | — | written through the shared helper (no driver seen using it) | — | — | D |
| `0x10` | `0x0004` | Bluetooth/serial power (guess) | `serial32`, `BTuartDMA` | guess | D |
| `0x10` | `0x0008` | Wi-Fi power sequence step 5 | `pcc_socket`, `tiacxwln` | likely | D; C |
| `0x10` | `0x1000` | **CompactFlash slot power, stage 2** | `pcc_socket` | confirmed | D; P (the slot powers up and stays up) |
| `0x10` | `0x2000` | Wi-Fi power sequence step 4 | `pcc_socket`, `tiacxwln` | likely | D; C |
| `0x10` | `0x8000` | cleared by the notification driver (LED or vibration, guess) | `OemNotify` | guess | D |
| `0x12` | `0x0001`-`0x0008` | display sequence bits (LCD power-up/down, guess) | `ddi`, `ddi_2700g` | guess | D |
| `0x14` | `0x0002` | audio (guess: amplifier) | `wavedev` | guess | D |
| `0x14` | `0x0004` | peripheral / serial (guess) | `peripheral`, `serial32` | guess | D |
| `0x14` | `0x0008` | display (AxDroid: audio amplifier, set at boot) | `ddi`, `ddi_2700g` | guess | D; C |
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
| `0x1C` | `0x0002` | audio (guess) | `wavedev` | guess | D |
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

- What Windows Mobile leaves on at hand-over, register by register: the
  registers don't read back, and Windows Mobile's shared shadow copies sit
  in its RAM at an address not yet resolved to physical. Knowing it would
  let RetroPane start its shadow from Windows Mobile's exact state instead
  of the bits it knows it needs.
- The guessed rows (Bluetooth, audio, display, battery, notification):
  each needs a device experiment before a driver relies on it.
- `0x08`: reachable through Windows Mobile's helper, no user found.
