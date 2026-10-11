# X51v SD card slot (PXA27x MMC controller)

> Spec v0.1, 2026-10-10. Facts only, per [`../CLEANROOM.md`](../CLEANROOM.md).
> **Sources:** (M) Intel PXA27x Developer's Manual, MMC/SD/SDIO controller chapter;
> (S) SD Association, Physical Layer Simplified Specification; (C) AxDroid board
> code; (R) the user's ROM (`pxa27x_sdhc.dll`, `sdbus.dll`, `sdmemory.dll` in XIP).

## Board (C)

| Item | Value | Confidence |
|---|---|---|
| Controller | PXA27x MMC at `0x41100000`, clock `CKEN` bit 12 | likely |
| Card power | CPLD `0x16` bit 0 (docs/hw/cpld.md) | likely |
| Card detect | GPIO12 | likely |
| Write protect | GPIO27 | likely |
| Card types | SD, SDHC (the user's 4 GB card works under WM6.1); SDXC to be verified | confirmed for SDHC |

## Controller registers (M)

| Register | Offset | Fields used |
|---|---|---|
| `STRPCL` | 0x00 | `[0]` stop clock, `[1]` start clock (starts the queued command) |
| `STAT` | 0x04 | `[1]` response timeout, `[8]` clock enabled, `[11]` data done, `[12]` programming done, `[13]` end of command/response |
| `CLKRT` | 0x08 | card clock = 19.5 MHz >> n (n = 0..6) |
| `CMDAT` | 0x10 | `[1:0]` response type (0 none, 1 R1/R6/R7, 2 R2, 3 R3), `[2]` data enable, `[3]` write, `[5]` busy (R1b), `[6]` 80 init clocks, `[8]` 4-bit SD data bus |
| `RESTO` / `RDTO` | 0x14 / 0x18 | response / read timeouts |
| `BLKLEN` / `NUMBLK` | 0x1C / 0x20 | block length / count |
| `I_REG` | 0x2C | `[0]` data done, `[5]` RX FIFO request |
| `CMD` | 0x30 | `[5:0]` command index |
| `ARGH` / `ARGL` | 0x34 / 0x38 | argument high / low 16 bits |
| `RES` | 0x3C | response FIFO: nine 16-bit reads |
| `RXFIFO` | 0x40 | receive data, read byte by byte, 32 bytes per request |

**Response decoding:** read `v` = word 0, then for *i* in 0..4 read `w1`, `w2` and
form `resp[i] = v << 24 | w1 << 8 | w2 >> 8`, then `v = w2`. For 48-bit responses
only `resp[0]` (the card status, OCR, RCA or CMD8 echo) is meaningful.

**Command sequence:** write `CMD`, `ARGH`, `ARGL`, (`BLKLEN`/`NUMBLK` for data),
`CMDAT`, then `STRPCL` = start clock; wait for `STAT` end-of-response or timeout.

## Protocol (S)

CMD0 (with 80 init clocks) → CMD8 0x1AA (v2 cards echo it; v1 time out) →
repeat CMD55 + ACMD41 (HCS for v2) until OCR busy bit 31 is set; bit 30 = SDHC →
CMD2 → CMD3 (RCA in response bits 31:16) → CMD7 select → CMD16 512 (SDSC only) →
CMD17 reads a block (block address on SDHC, byte address on SDSC).

## Speed (measured, real X51v)

One block per command took 3.85 ms on the owner's 4 GB SDHC card, far more
than the 0.2 ms the transfer itself needs at 19.5 MHz on one data line: the
card's per-command start-up dominates. RetroPane therefore switches every
card to the 4-bit bus (ACMD6, argument 2, then `CMDAT[8]` on data commands)
and reads runs of blocks with one CMD18, stopped by CMD12.

## Emulator

QEMU's `pxa2xx_mmci` model with an `sd-card` on `-drive if=sd`. Its response FIFO
packing matches the layout above.
