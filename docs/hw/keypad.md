# X51v buttons (PXA27x keypad controller)

> Spec v0.1, 2026-10-10. Facts only, per [`../CLEANROOM.md`](../CLEANROOM.md).
> **Sources:** (M) Intel PXA27x Processor Family Developer's Manual, keypad
> interface chapter; (C) AxDroid board code (button matrix).

## Controller (M)

Base `0x41500000`; clock enable `CKEN` bit 19 (clock manager `0x41300004`).

| Register | Offset | Fields used |
|---|---|---|
| `KPC` | 0x00 | `[30]` AS (scan now), `[29]` ASACT (scan on activity), `[28:26]` MKRN = rows − 1, `[25:23]` MKCN = columns − 1, `[22]` MI, `[20:13]` MS7..0 (matrix scan lines), `[12]` ME (matrix enable), `[11]` MIE (matrix interrupt enable) |
| `KPDK` | 0x08 | direct keys |
| `KPMK` | 0x18 | matrix key (manual scan) |
| `KPAS` | 0x20 | `[31]` SO (scan on), `[30:26]` MUKP = keys pressed, `[7:4]` RP row, `[3:0]` CP column (0xF = none) |
| `KPASMKP0..3` | 0x28, 0x30, 0x38, 0x40 | pressed-key bitmaps: register *n* covers columns 2*n* (rows in `[7:0]`) and 2*n*+1 (rows in `[23:16]`) |
| `KPKDI` | 0x48 | debounce interval |

## X51v button matrix (C)

7 rows × 3 columns, plus one direct key (power).

| Row \ Column | 0 | 2 |
|---|---|---|
| 0 | Wi-Fi | D-pad up |
| 1 | Record | D-pad down |
| 3 | Calendar | D-pad right |
| 4 | Contacts | D-pad left |
| 5 | Mail | D-pad center (Enter) |
| 6 | Home | — |

Column 1 and row 2 aren't used by any known button. Confidence: **likely**
(working AxDroid keymap). To confirm against Dell's keypad driver.

## Emulator mapping (retropane-qemu `x51v`)

Arrow keys and Enter = D-pad; F1 Calendar, F2 Contacts, F3 Mail, F4 Home,
F5 Record, F6 Wi-Fi.
