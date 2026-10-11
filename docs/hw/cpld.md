# X51v board CPLD ("FPGA")

> Spec v0.1, 2026-10-10. Facts only, per [`../CLEANROOM.md`](../CLEANROOM.md).
> **Sources:** (C) AxDroid board and Wi-Fi glue code (working on X50/X51
> hardware in 2010); (A) strings in Dell's `AximSDK.dll` from the user's ROM.
> Dell's own CPLD code lives in the kernel's OAL (`nk.exe`), which `AximSDK`
> reaches through `KernelIoControl`; analysing it is the next step.

## Address and access

| Item | Value | Confidence | Evidence |
|---|---|---|---|
| Physical base | `0x10000000` (PXA chip select 4) | confirmed | C (maps `PXA_CS4_PHYS`) |
| Size | 16 registers × 16 bits, offsets `0x00`–`0x1E` (2-byte stride), 0x20 bytes | likely | C |
| Read-back | Unknown. AxDroid keeps a shadow copy of written values and never reads the power registers back, which suggests they may be write-only | guess | C |

## Register bits

| Offset | Bit(s) | Function | Confidence | Evidence |
|---|---|---|---|---|
| `0x00` | `0x0040` | Wi-Fi power sequence step 1 | likely | C (Wi-Fi glue) |
| `0x10` | `0x2000` | Wi-Fi power sequence step 4 | likely | C |
| `0x10` | `0x0008` | Wi-Fi power sequence step 5 | likely | C |
| `0x14` | `0x0008` | Set at boot (audio amplifier power, per AxDroid notes) | guess | C |
| `0x16` | `0x0001` | **SD card power** | likely | C (MMC set_power) |
| `0x16` | `0x0004` | Wi-Fi power sequence step 3 | likely | C |
| `0x1C` | `0x0006` | Set at boot (unknown) | guess | C |
| `0x1C` | `0x0001` | Wi-Fi power sequence step 2 | likely | C |
| `0x1C` | `0x0008` | USB power? | guess | C (comment "USB power?") |
| `0x1E` | `0x0008` | Write 1 to enable the LCD-type readout | likely | C |
| `0x1E` | `0x0003` (read) | **LCD type**: 0 = 3.7" Sharp, 1 = 3.5" Sharp, 2 = 3.7" Samsung, 3 = 3.5" Sharp. Even = VGA (X50v/X51v), odd = QVGA | likely | C (from `AximSDK.dll`); A (strings "Sharp", "Samsung", "3.7 TFT", "3.5 TFT") |

## Sequences

**LCD type detection:** set `0x1E` bit 3 → wait ~1 ms → read `0x1E & 3` → wait ~1 ms → clear bit 3.

**Wi-Fi (ACX100) power-up:** Wi-Fi reset GPIO89 high → 100 µs → `0x00 |= 0x40` →
1 ms → `0x1C |= 0x01` → 50 ms → `0x16 |= 0x04` → 1 ms → `0x10 |= 0x2000` →
1 ms → `0x10 |= 0x08` → 100 ms → GPIO89 low. The ACX100 then responds at
`0x2C000000` (PC Card socket 0 common memory), interrupting on GPIO107.

**Wi-Fi power-down:** `0x00 |= 0x40` → 1 ms → clear `0x10` bits 3 and 13, `0x16`
bit 2, `0x1C` bit 0 → GPIO89 high.

## Open questions

- Read-back behaviour, and the reset value of every register.
- Remaining bits: CF power, Bluetooth power and reset, backlight enable, charger.
- Confirm all of the above from Dell's OAL code in `nk.exe`.
