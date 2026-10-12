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
| `CMDAT` | 0x10 | `[1:0]` response type (0 none, 1 R1/R6/R7, 2 R2, 3 R3), `[2]` data enable, `[3]` write (data goes to the card), `[5]` busy (R1b), `[6]` 80 init clocks, `[8]` 4-bit SD data bus, `[10]` stop transmission (the CMD12 ending a multi-block **write**; without it the controller never issues it: command timeout, `STAT` = 0x142) |
| `RESTO` / `RDTO` | 0x14 / 0x18 | response / read timeouts |
| `BLKLEN` / `NUMBLK` | 0x1C / 0x20 | block length / count |
| `I_REG` | 0x2C | `[0]` data done, `[1]` programming done, `[5]` RX FIFO request, `[6]` TX FIFO request (the FIFO wants 32 bytes) |
| `CMD` | 0x30 | `[5:0]` command index |
| `ARGH` / `ARGL` | 0x34 / 0x38 | argument high / low 16 bits |
| `RES` | 0x3C | response FIFO: nine 16-bit reads |
| `RXFIFO` | 0x40 | receive data, read byte by byte, 32 bytes per request |
| `TXFIFO` | 0x44 | transmit data, written byte by byte, 32 bytes per request |

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

**Writes (ADR-0013, confirmed on the X51v 2026-10-11):** CMD24 writes one
block, CMD25 a run, stopped by CMD12 with `CMDAT[10]` set. Controller side:
`CMDAT` data enable + write, then fill `TXFIFO` 32 bytes at a time on each
`I_REG[6]` request; `STAT[11]` data done when the last byte went out,
`STAT[12]` programming done after a single block. **The controller's busy
wait on the stop does not cover the card's programming:** the command after
a CMD25 + CMD12 run got no response (`STAT` 0x142: response timeout,
transmit FIFO empty, clock on). RetroPane polls CMD13 after every write
until the card is ready for data and out of the `prg` state, which the
specification allows for up to 250 ms per block. The emulator's model needs
neither the stop bit nor the poll, so neither could be found there.

Measured on the owner's 4 GB SDHC card (4-bit bus, 19.5 MHz, through the
FAT layer, verified by read-back): 36,870 B in 36 ms, 142,848 B in 85 ms
(1.7 MB/s), read back at 1.7 MB/s. A first write after a stale FSInfo hint
took 3.7 s: the free-cluster scan read the FAT a sector per command
(1,490 sectors × 2.5 ms); the hint is kept current afterwards. The scan now
reads the FAT in runs through the loader's 64 KiB scratch (one sector
first, doubling to 128 per command), so the same scan is ~18 commands
instead of 1,490; what remains is the transfer itself (~0.75 MB of FAT,
roughly 0.4 s at the measured rate). The device number is still to be
measured (the loader's reply now counts the write's read commands).

### Reads that fail (measured on the owner's card, 2026-10-12)

The card as a USB drive (ADR-0014) reads far more of it than the loader
ever did, and found one weak block:

| Fact | Value | Confidence | Evidence |
|---|---|---|---|
| Block 10880 (inside the first FAT) takes 0.79 s to read alone; every other block measured takes ~4 ms | 794,241 / 794,457 us in two clean reads | confirmed | `cargo xtask card scan 10880 1` |
| A multi-block read (CMD18) that reaches it always ends in a read timeout (`STAT` 0x2941: bit 0) | 14/14, then 36/36 failures of the same 64-block range | confirmed | `cargo xtask drive status` |
| After such a timeout the card is still sending: the CMD12 that stops a timed-out read is unanswered (`STAT` 0x2142) and the card reports ILLEGAL_COMMAND (bit 22) in the next response | status 0x00400900 on good CMD17s | confirmed | `drive status` |
| Once provoked, single reads of the block also time out (0x2141), at full clock, a quarter and the minimum clock; clean single reads succeed at ~0.79 s, which looks like the controller's longest read wait whatever `CLKRT` is | 2/3 clean scans succeed, none after failed CMD18s | likely | `card scan`, `drive status` |
| The PXA27x's longest read timeout (`RDTO` = 0xFFFF) is about 0.8 s on this board | inferred from the two above | guess | to measure: time a read that the controller times out |

What the driver does (rp-drv-sdcard): ILLEGAL_COMMAND and COM_CRC_ERROR in
a response describe the *previous* command and no longer fail the current
one; after any failed transfer it sends CMD12 and polls CMD13 back to the
transfer state; CRC-type failures are retried, timeouts go straight to
reading block by block, a single block that times out is retried at a
quarter and at the minimum clock, and a range where a multi-block read
failed is remembered and read block by block from then on. None of that
rescues this block on this card; it does keep every other read working and
the card responsive.

**Open, for the owner:** the block can be *scrubbed*: read once while it
reads (a clean single read does), then written back unchanged, so the card
stores it in a fresh page and it reads fast again. That is a write to the
first FAT, so it waits for the owner (a card backup first). Or replace the
card: a block that slow is often the first sign of a wearing card.

### File system

The card is FAT32 (partition type `0x0C`, 4 KiB clusters on the owner's
card). `kernel/fat` (`rp-fat`) reads it and replaces files by copy-then-swap:
new clusters first, FAT links in every copy, then one sector write switches
the directory entry, then the old clusters are freed. The probe lists the
root directory read-only; the loader takes `RPF1` frames (`cargo xtask put`).

## Speed (measured, real X51v)

One block per command took 3.85 ms on the owner's 4 GB SDHC card, far more
than the 0.2 ms the transfer itself needs at 19.5 MHz on one data line: the
card's per-command start-up dominates. RetroPane therefore switches every
card to the 4-bit bus (ACMD6, argument 2, then `CMDAT[8]` on data commands)
and reads runs of blocks with one CMD18, stopped by CMD12.

## Emulator

QEMU's `pxa2xx_mmci` model with an `sd-card` on `-drive if=sd`. Its response FIFO
packing matches the layout above; writes through its transmit FIFO reach the
image file (`cargo xtask emu` replaces a file on a FAT32 card image through
the loader and checks the image afterwards). Upstream's model never issued a
stop-transmission command (`CMDAT[10]`) and left the previous command's
end flag set, so a driver written for the real controller saw a false
"done" and a card still waiting for data; the fork's model (rp-v0.8.0)
issues it once the transfer is finished, as the hardware does. The model
needs no CMD13 ready wait, so that finding is device-only.
