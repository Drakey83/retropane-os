# X51v touchscreen (TI TSC2046 on PXA27x SSP1)

> Spec v0.1, 2026-10-10. Facts only, per [`../CLEANROOM.md`](../CLEANROOM.md).
> **Sources:** (D) TI TSC2046 datasheet; (M) Intel PXA27x Developer's Manual
> (SSP and GPIO chapters); (C) AxDroid board code (working touch on X50/X51).

## Wiring (C)

| Signal | Connection | Confidence |
|---|---|---|
| Data | PXA27x **SSP1**: GPIO23 SCLK, GPIO25 TXD, GPIO26 RXD (SSP1 alternate functions) | likely |
| Chip select | **GPIO24** as a plain GPIO output, active low (not the SSP frame signal) | likely |
| Pen interrupt | **GPIO94**, active low while touched | likely |
| Axes | X and Y are swapped relative to the screen | likely |
| Clock | ≤ 2.6 MHz (AxDroid ran it at 100 k samples/s × 26 clocks) | likely |

AxDroid's settings: internal reference with 100 µs settle, 150 µs input settle.

## TSC2046 protocol (D)

SPI mode 0. A conversion takes 24 clocks; with 8-bit frames that's three transfers:

1. Control byte: `[7]` S = 1, `[6:4]` A2..A0 channel, `[3]` MODE (0 = 12-bit,
   1 = 8-bit), `[2]` SER/DFR (0 = differential), `[1:0]` PD power-down mode.
   The device returns 0.
2. Returns a busy bit (0), then D11..D5.
3. Returns D4..D0, then three zero bits.

So `result = ((byte1 << 8 | byte2) >> 3) & 0xFFF`.

Channels (differential): **5 = X position, 1 = Y position, 3 = Z1, 4 = Z2**.

## PXA27x SSP (M)

SSP1 base `0x41000000`, clock enable `CKEN` bit 23.

| Register | Offset | Fields used |
|---|---|---|
| `SSCR0` | 0x00 | `[3:0]` DSS = data bits − 1, `[5:4]` FRF (0 = Motorola SPI), `[7]` SSE enable, `[19:8]` SCR: bit rate = 13 MHz / (SCR + 1) |
| `SSCR1` | 0x04 | `[3]` SPO clock polarity, `[4]` SPH clock phase (both 0 for mode 0) |
| `SSSR` | 0x08 | `[2]` TNF (transmit FIFO not full), `[3]` RNE (receive FIFO not empty), `[4]` BSY |
| `SSDR` | 0x10 | data FIFO |

## PXA27x GPIO (M)

Base `0x40E00000`. Banks of 32 pins: bank *n* = pins 32*n*..32*n*+31.

| Register | Banks 0-2 | Bank 3 |
|---|---|---|
| `GPLR` level | 0x00 + 4*n* | 0x100 |
| `GPDR` direction (1 = output) | 0x0C + 4*n* | 0x10C |
| `GPSR` set output high | 0x18 + 4*n* | 0x118 |
| `GPCR` clear output low | 0x24 + 4*n* | 0x124 |

## Emulator

retropane-qemu's `tsc2046` model implements the byte-level protocol above (not
QEMU's simplified ads7846), on SSP1 with GPIO24 chip select and GPIO94 pen
interrupt. Touches come from the emulator's absolute pointer (mouse button =
pen down), mapped linearly to raw values 200..3900 on both axes, with X and Y
swapped like the real panel.
