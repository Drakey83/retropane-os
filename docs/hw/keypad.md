# X51v buttons (PXA27x keypad controller)

> Spec v0.1, 2026-10-10. Facts only, per [`../CLEANROOM.md`](../CLEANROOM.md).
> **Sources:** (M) Intel PXA27x Processor Family Developer's Manual, keypad
> interface chapter; (C) AxDroid board code (button matrix).

## Controller (M)

Base `0x41500000`; clock enable `CKEN` bit 19 (clock manager `0x41300004`).

| Register | Offset | Fields used |
|---|---|---|
| `KPC` | 0x00 | `[30]` AS (scan now), `[29]` ASACT (scan on activity), `[28:26]` MKRN = rows − 1, `[25:23]` MKCN = columns − 1, `[22]` MI (matrix interrupt; cleared by reading KPC), `[20:13]` MS7..0 (matrix scan lines), `[12]` ME (matrix enable), `[11]` MIE (matrix interrupt enable) |
| `KPDK` | 0x08 | direct keys |
| `KPMK` | 0x18 | matrix key (manual scan) |
| `KPAS` | 0x20 | `[31]` SO (scan on), `[30:26]` MUKP = keys pressed, `[7:4]` RP row, `[3:0]` CP column (0xF = none) |
| `KPASMKP0..3` | 0x28, 0x30, 0x38, 0x40 | pressed-key bitmaps: register *n* covers columns 2*n* (rows in `[7:0]`) and 2*n*+1 (rows in `[23:16]`) |
| `KPKDI` | 0x48 | debounce interval |

### What the controller remembers (M)

The scan bitmaps (`KPASMKP*`) and `KPAS` describe only the **latest**
automatic scan: what is held now. With nothing held, `KPAS` reads MUKP 0
and row and column 0xF. The one thing that outlives a key's release is
`KPC` MI, set by every press and every release and cleared by reading
`KPC`. So a press and release that both fall between two reads leave
"something moved" behind, **but not which button**: no register can tell
afterwards. Found 2026-10-13 when `rp-mbx` missed 2 of 5 presses in the
emulator; the emulator's keypad behaves the same way (`emulator.md`).

Consequences for software:

- Read with `Keypad::scan` (`rp_drv_pxa_keypad`): it acknowledges `KPC`
  first, then reads the bitmaps, then reads the latch again (re-reading if
  keys moved meanwhile), and returns the held set with the activity flag.
  Reading the bitmaps first could lose a press that lands between the two
  reads.
- `Keys` (below) counts "activity, held set unchanged" as a tap it
  couldn't see (`Keys::missed`); rp-today's report, rp-hello's serial log
  and rp-probe's footer show the count. That keeps a lost tap visible; it
  doesn't bring it back.
- Only a read **while the key is down** can name the button. In the OS,
  that is the keypad driver's thread, woken by the interrupt at high
  priority while the shell (user mode) is preempted: ADR-0007 rules 2-3
  and ADR-0010. The bare-metal test images have one loop that both draws
  and reads input, so a tap shorter than a redraw (rp-today: up to ~71 ms
  measured on the device) can still be lost there.

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

## As programs see them

`rp_drv_pxa_keypad::keys` gives each button its CE virtual-key code: the
D-pad `VK_UP`/`VK_DOWN`/`VK_LEFT`/`VK_RIGHT` and `VK_RETURN`; the program
buttons `VK_APP1` (Calendar), `VK_APP2` (Contacts), `VK_APP3` (Mail),
`VK_APP4` (Home), `VK_APP5` (Record), `VK_APP6` (wireless). The program
button numbering follows the usual Windows Mobile order: **likely**, to
confirm in the owner's Settings, Buttons. The D-pad repeats (500 ms, then
20 a second); program buttons have "press and hold" (1 s).

## Emulator mapping (retropane-qemu `x51v`)

Arrow keys and Enter = D-pad; F1 Calendar, F2 Contacts, F3 Mail, F4 Home,
F5 Record, F6 Wi-Fi.
