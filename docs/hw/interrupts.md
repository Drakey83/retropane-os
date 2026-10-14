# Interrupts: Windows Mobile's map of the X51v's lines

> **Source:** (D) the Windows Mobile 6.1 kernel's OAL (`nk.exe`, extracted
> from the owner's A08 ROM: XIP image at file offset `0x400`, loaded at
> `0x8C200000`, ROMHDR at `0x8C2C9324`), its interrupt handler and its
> enable/disable/done routines, analysed locally (2026-10-14); no code
> from it is here. Drivers from the ROM's registry (`default.hv`). Every
> GPIO edge setting below matches the registers as WM left them
> (`probe-2026-10-11.md`).

The OAL's handler is a chain on the PXA's `ICIP`; GPIOs 2-120 (ICIP bit
10) are tested in the order 11, 14, 13, 94, 12, 83, 84, 107, 96, 85,
86/87. SYSINTR is WM's logical interrupt number.

| SYSINTR | Source and trigger | Driver | Confidence |
|---|---|---|---|
| 1 | OS timer 0 (tick) | kernel | confirmed |
| 13 | RTC alarm | | confirmed |
| 16 | GPIO0 both edges, keypad, CPLD status bit 8 (via GPIO13, rising) | keyboard / power button | confirmed (driver likely) |
| 17 | GPIO94 falling (the handler also stops OS timer 1) | ? | confirmed |
| 18 | GPIO11 both edges | ? | confirmed |
| 19 | FFUART, DMA 8/9, CPLD bit 9 | serial (COM1) | confirmed |
| 20 | DMA 2/3 | audio | likely |
| 21 | CPLD bit 1 | CF socket status | confirmed |
| 23 | GPIO84 falling | CF card interrupt | confirmed |
| 24 | OS timer 1 | ? | confirmed |
| 25 | STUART, DMA 14/15 | IrDA | confirmed |
| 26 | BTUART, DMA 6/7 | Bluetooth | confirmed |
| **27** | **GPIO85 both edges** | **the 2700G (ddi_2700g)** | confirmed |
| 28 | GPIO107 falling | Wi-Fi (TI ACX) | confirmed |
| 30 | USB client, GPIO14 both edges, DMA 4/5 | USB function | confirmed (GPIO14 = cable: likely) |
| 31 | MMC, DMA 10/11 | SD | confirmed |
| 32 | I2C | ? | confirmed |
| 33 | I2S | audio | likely |
| 34 | GPIO86 or 87, both edges | ? | confirmed |
| 35 | GPIO83 both edges | headphone detect (`audio.md`) | confirmed (driver likely) |
| 36 | GPIO12 both edges | SD card detect (`sd.md`) | confirmed (driver likely) |
| 37 | CPLD bit 9 while CPLD `+4` bit 9 is set | ? | confirmed |
| 39 | GPIO96 both edges | ? | confirmed |

**The 2700G's line, GPIO85.** WM's enable makes it an input with both
edges armed; its handler only clears the GPIO85 latch (no 2700G register
read). WM's **disable makes GPIO85 an output driven low**: a later reader
sees it low and still, whatever the chip does. RetroPane makes it an
input before listening (`rp-mbx` stage 4e).

Unresolved: the SD driver's registry `ControllerIRQ` 23 and
`CardDetectIRQ` 32 look like raw IRQ numbers, not SYSINTRs (no
translation found).
