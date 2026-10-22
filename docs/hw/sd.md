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
| Card detect | GPIO12, **active low (0 = card in)** | confirmed (probe with the owner's card in, 2026-10-12) |
| Write protect | GPIO27, high = locked | likely (reads 0 with the owner's unlocked card) |
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
| Its data was damaged too: 118 of 128 FAT entries impossible values; the mirror (second FAT) intact. Repaired from the mirror: 0.9 ms reads after | confirmed | `card scrub`/`card repair`/`card peek`, 2026-10-12 |
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

**Resolved (2026-10-12, owner approved the rewrite):** the block wasn't
just slow, its *data* was damaged. Compared with its mirror in the second
FAT (`cargo xtask card scrub` refused: 436 of 512 bytes differ), 118 of
the 128 entries held values no FAT entry can have (e.g. `0x23510080` on a
972,800-cluster card); the second FAT, which the boot sector marks as
mirrored (ExtFlags `0x0000`), held zeros (free clusters). `cargo xtask card
repair 10880` restored it from the mirror: verified by read-back, and its
read time went **795,659 µs → 900 µs** (rewritten into a healthy page).
Scanning the rest of the first FAT: blocks 9,374-15,074 all read in under
6 ms; the last 1,901 (to 16,975) outlasted the loader's run limit and the
device reset (the scan fed the watchdog only at its start; fixed). That
range is still to check. A damaged FAT sector on a card is a sign of
wear: keep a backup, and consider replacing it.

### File system

`cargo xtask card check` runs a read-only check of the card's file system
(FAT32 or exFAT) through the loader: cross-links, broken chains, sizes,
lost clusters, FAT copies that differ (as block 10880's did).


SDXC cards (64 GB and up) come as exFAT: `rp-fat` reads and writes those
too (folders included, growing as they fill),
through the allocation bitmap, storing a file
that fits one free run with no FAT chain; the loader's `put` and crash
reports mount whichever the card has (`AnyVolume`). Same crash-safety
order as below, plus exFAT's VolumeDirty flag for the duration.

The card is FAT32 (partition type `0x0C`, 4 KiB clusters on the owner's
card). `kernel/fat` (`rp-fat`) reads it and replaces files by copy-then-swap:
new clusters first, FAT links in every copy, then one sector write switches
the directory entry, then the old clusters are freed. The probe lists the
root directory read-only; the loader takes `RPF1` frames (`cargo xtask put`).

### Run logs on the card (the card log, 2026-10-20)

A test image's run log in SDRAM, and its copy in the internal SRAM, are
cleared by a hardware watchdog reset into Dell's prompt (run Z: the
2700G froze the bus): the runs that most needed their log lost it.

**First design, proven on the X51v (2026-10-20, rp-gpubench run H,
`local/rp-gpubench-runH1.txt`, `runH2.txt`, `local/cardlog/`):** the
image itself (`rp-cardlog`, built into rp-gpubench only) reserved
`rplog/<image>-NNNN.txt`, 256 KiB zero-filled in one run of sectors
(`reserve`, crash-safe like `put`), and wrote each log line into it **in
place**: one sector write a line, no directory or FAT change during the
run, so a reset at any moment leaves the file system whole and every line
written before it on the card (the text ends at the first zero byte). H1
returned and its card log matched the SDRAM log line for line (50 lines,
the closing line, 0 refused; slowest sector write 17.4 ms: keep log lines
out of timed parts). H2 logged 40 numbered lines and fired the PXA's
hardware watchdog (into Dell's restore prompt, Mail, the loader): the
loader had no run log left, and the card had every line up to the last
one before the reset, and no closing line.

**Every run now (owner's vote B, quality audit C24; proven on the X51v
2026-10-20, below):** the same file and the same in-place writes, but
the loader reserves the file (it has the file systems) for every image it
starts, named by `cargo xtask load` (`name <image>`, checked: 1-24 of
`a-z 0-9 - _`), the next number on either card. It reserves one on the SD
card and, when a CF card is in, one with the same name on the CF card
(`docs/hw/cf.md`), deletes this image's files beyond the newest 32 (8 a
run), and leaves the state in SDRAM (`rp_hal_x51v::cardlog`, at
`0xA8511000`). Every program then gets it with no code of its own: the
run log's writer (`devices::log_write`) puts each line on a card when it
ends, through the controller the loader set up (`rp_hal_x51v::cardwrite`: the
drivers' single-block write, CMD24 then CMD13, on the card the loader
identified, its address and bus width from the loader; a host test holds
it to the drivers' register writes; the CPU's own transfers, so the
program's DMA channels stay its own). Each sector is built **from the run log itself**
(sector `k` = bytes `k * 512..` of the run log, then of `0xA8520000` for
what a long run logs past the run log's 64 KiB): the card can't say
anything the run log doesn't. About 4.3 KiB of each image.

- **A line costs the sector's transfer, not the card's programming**
  (quality audit C59, 2026-10-22: the touch loop slowed from a sample
  every 7 ms to 11 ms with every sample logged, the slowest write 61.5 ms,
  90.3 ms in a later run).
  A program's writes return once the block is on the card (data done);
  the card's programming (the controller's programming done, then CMD13
  until it leaves `prg`) is waited for before the card's next command:
  the next line's write, `rt::take_mmc` / `devices::take_cf` (a program
  driving the card itself), and the return to the loader. A sector whose
  programming fails is found then: the card is given up on as for a
  refused write and that sector goes onto the other card. Lines back to
  back still wait for each other. The loader's own writes (its drivers)
  wait as before. After a reset the card has had seconds (Dell's boot or
  the loader's restart) to finish. *Built, emulator-checked; not yet
  measured on the X51v (the touch replay with the recorder on measures
  it).*
- **A program that takes the SD controller** (`rt::take_mmc`: rp-sdtime,
  rp-noise, rp-looks...) has it to itself: the log moves to the CF card
  from its next line, with a line saying so (`[card log: the SD card was
  taken by the program; continuing on the CF card]`). A CF test keeps it
  on SD; a program that takes both (rp-storfast), or an SD test with no
  CF card in, pauses it, with a line saying so; a card that refuses a
  write hands over the same way.
- **The loader closes it** when the program comes back (or, after a hang
  that kept the RAM, at its next start): what a pause held back goes onto
  a card, then `[card log closed: the run returned | crashed | did not
  return (closed at the next start); N sector writes, M refused, slowest
  T us, D bytes dropped]`. The closing line is in the run log the PC
  receives too. A file without it is a run that didn't come back and whose
  RAM didn't survive (a watchdog reset into Dell's prompt).
- **After a reset the loader writes a card only if its file still starts
  with the run's first bytes** (a card swapped meanwhile could hold other
  files there).
- `cargo xtask cardlog [image] [number]` fetches the SD card's part and
  the CF card's (`ls cf:rplog`, `get cf:rplog/...`), joins them byte for
  byte (each part holds its stretch at the same offsets, zeros elsewhere)
  and saves the whole log under `local/cardlog/`.

**On the X51v (2026-10-20, the loader installed, CF card in;
`local/cardlog/light-0001.txt`, `run-0001.txt`):** rp-light's card log
was the run log byte for byte (559 bytes, on the SD card, 17 sector
writes, 0 refused). rp-sdtime, the SD speed test, took the SD controller:
the log moved to the CF card with its line, and the two halves joined
into the run log byte for byte (4,694 bytes, 59 sector writes, 0
refused). The slowest sector write was **48 ms** (rp-light) and **141 ms**
(rp-sdtime, on the CF card or the SD card right after its tests), against
run H's 17.4 ms: a log line can cost tens of milliseconds, so timed parts
of a test log after they finish, never inside. Found on the way: the
image's name, sent as its own command before the image, was lost once
while Windows re-found the port after a run (the log was named `run`); it
now goes in the same transfer as the image, and the loader keeps the
bytes after a frame's end for the next frame (it had dropped the rest of
the USB read: the image never arrived).

`emu-cardlog` checks it on the emulator with both cards in: rp-light (on
SD, the CF file unwritten; numbered after 40 seeded files, the 8 oldest
beyond 32 deleted), rp-sdtime (moved to CF, the halves joined), rp-crash
(closed as crashed) and an image that hangs with interrupts off (the
watchdog reset, closed at the next start): each card log equals the run
log the PC received, word for word, and nothing outside `rplog/` changed
on either card. `emu-gpubench` checks every gpubench run's.

### DMA (M; emulator-verified; reads confirmed on the device 2026-10-15)

Multi-block transfers can move by DMA instead of byte-wide FIFO reads:
`CMDAT[7]` sends the controller's FIFO requests to the DMA controller,
request line 21 (receive FIFO, physical `0x41100040`) and 22 (transmit,
`0x41100044`). The channel (`hal/pxa27x/src/dma.rs`) runs a descriptor chain,
4 KiB per descriptor, 32-byte bursts of byte-wide FIFO accesses (`DCMD`
receive: increment target, flow-control by source; transmit: increment
source, flow-control by target), STOP on the last descriptor; done is
`DCSR` STOPINTR, failure BUSERR. The DMA engine owns the cache maintenance
(clean before, discard after a receive), so no caller can forget it, and
accepts only whole cache lines (32-byte aligned, multiples of 32 bytes): a
line shared with other data could be written back over what the device
wrote. Anything else, and single blocks, go through the FIFO as before.

The controller driver watches the card's error bits while the DMA runs (a
failing card never raises the FIFO requests the DMA waits for) and stops the
channel on any failure, so the existing recovery (CMD12, CMD13, retries,
block-by-block fallback) works unchanged.

Rollout, because nothing unproven may touch the owner's card: the loader
self-tests DMA at start-up (8 blocks read by DMA into a buffer pre-filled
with a pattern, compared with the same blocks read one by one); reads use
DMA only if they match. Writes stay on the FIFO until the owner runs
`cargo xtask card dma writes` for a session (a `put` is then verified by
read-back). `cargo xtask card dma` reports the verdict and the transfer
count. On the X51v (2026-10-15): "dma reads on (self-test passed), writes
off, 194 transfers".

## Speed (measured, real X51v)

One block per command took 3.85 ms on the owner's 4 GB SDHC card, far more
than the 0.2 ms the transfer itself needs at 19.5 MHz on one data line: the
card's per-command start-up dominates. RetroPane therefore switches every
card to the 4-bit bus (ACMD6, argument 2, then `CMDAT[8]` on data commands)
and reads runs of blocks with one CMD18, stopped by CMD12.

On the owner's 32 GB card (2026-10-12) 64-block reads ran at 0.27 ms per
block (~1.9 MB/s) where the data itself needs ~52 us at 19.5 MHz on four
lines: about 80% of each block is overhead, not the bus. The clock is
right (`CLKRT` 0, the controller's 19.5 MHz maximum). Prime suspect: the
receive FIFO is read a byte per bus access (the controller's FIFO is byte
wide; Linux reads it the same way). `rp-probe` now measures the parts
separately (item 58): `sd_read_block` (per-command cost),
`sd_read_64_blocks` / `sd_read_256_blocks` through the FIFO, and the same by
DMA (`sd_read_64_blocks_dma`, `sd_read_256_blocks_dma`, checked against
single-block reads by `sd_dma_matches`).

**The drive overlaps the card and USB** (item 58, emulator-verified): reads
use the buffer as two halves; while one goes to the PC, the next chunk is
read into the other by DMA, and the DMA's waiting loop sends the first
half's packets (`read_blocks_while`: useful work instead of spinning; the
card driver's retries and fallbacks run without it, unchanged). After a
bulk read the next chunk is read ahead and served if the PC reads on
(`drive status`: `read ahead N`); any write, medium change or reset drops
it. Only with DMA reads on (otherwise reading ahead would only add
waiting). Expected: the drive at the slower of card and USB instead of
their sum; to measure on the device.

## Off the end of the card (measured, real X51v, 2026-10-21)

`rp-sdtime`'s guard run (quality audit C32; `local/rp-sdtime-guard-run1.txt`),
the owner's 32 GB SDHC card (61,067,264 blocks, the loader's and the
driver's count agreeing):

- **Confirmed:** every read and write past the end refused by the driver
  (`read`, `read_many`, `write`, `write_many`, the block-device interface)
  and by the HAL's card-log writer in under a microsecond, no command sent
  (the read count unchanged); a run crossing the end refused whole.
- **New, not understood:** the last block read once, then the second read
  of it failed, and every read after (the last block again, block 0). The
  run didn't log the errors. A guess to test: the card reads ahead past
  its end and reports `OUT_OF_RANGE` in the next command's status, which
  the driver takes as that read's failure; each failing read here was
  again of the last block. The guard run's version 2 logs each result and
  the card's status (CMD13) through a sequence after a last-block read.
- **Version 2 (`local/rp-sdtime-guard-run2.txt`): confirmed.** Every
  failed read returned status `0x80000900`: `OUT_OF_RANGE` (bit 31) with
  the card in transfer state and ready. A read of the last block
  succeeds; the card raises `OUT_OF_RANGE` doing it and reports it in the
  next command's status, whatever that is (a read of block 5 failed after
  one of the last block; a CMD13 straight after showed `0x80000900` and the
  next read was clean). The driver now collects it with a CMD13 after any
  read ending on the last block (quality audit C46).
- **Version 3 (`local/rp-sdtime-guard-run3.txt`): that CMD13 is too early.**
  Sent straight after the last-block read it doesn't find the flag (the
  next read still fails); a CMD13 a few milliseconds later does, and
  clears it. The card raises OUT_OF_RANGE some time after the read
  (likely its read-ahead running off the end). How long: to measure.
  The guard run version 4 (G7) measures it: 20 last-block reads each
  followed by CMD13 every ~50 us for up to 50 ms, then single CMD13s after
  idle waits of 0.1-10 ms.
- **Version 4 (`local/rp-sdtime-guard-run4.txt`): measured, confirmed.**
  After each of 20 last-block reads (the driver's own CMD13 straight after
  it, clean), the bit showed at G7's second CMD13, **76 us after the read,
  20 of 20**; the CMD13 after it was clean. After idle waits of 0.1, 0.5,
  1, 2, 5 and 10 ms one CMD13 found it every time, the next clean. So it
  is raised by a time (under 76 us; a CMD13 takes ~13 us at full clock),
  and it stays until a command collects it. The driver now sends CMD13
  until it shows, 80 at most (~1 ms).
- **Version 5 (`local/rp-sdtime-guard-run5.txt`): the fix confirmed, 19
  checks, 0 BAD.** Every read after a last-block read succeeded; G7's
  CMD13s found no OUT_OF_RANGE left. Open note: in 5 of the 31 last-block
  reads the driver's wait ended without the bit (`end_unseen`) and no read
  failed after: either the card didn't raise it then, or it came later and
  one of G7's clean-up CMD13s took it. Which reads: not logged.
- **Version 6 (`local/rp-sdtime-guard-run6.txt`, 2026-10-21): every wait
  logged, 19 checks, 0 BAD.** 30 of 31 waits saw OUT_OF_RANGE: at the 2nd
  and 3rd CMD13 for G2's two reads, the 5th for G6's two, the 4th for all
  26 of G7's (each after 60 ms of idle CMD13s). **One ended unseen: the
  last-block read straight after G4's refused writes**; the reads after it
  (G5's block 0, G6's) succeeded, so the card didn't raise the bit for
  that read at all (a raised bit fails the next read, version 2). So the
  card doesn't always raise it, and when it does it shows within 5 CMD13s
  (~65 us), well inside the driver's 80. Version 5's five unseen were the
  same kind (nothing failed after them). Why that read didn't raise it is
  open (the only read that followed refused commands; harmless to the
  driver either way: an unraised bit needs no collecting).

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

## Polling slows the DMA (2026-10-22, `rp-probe`, X51v)

The driver's DMA wait read the DMA status and the controller's `STAT`
back to back. Pausing 200 spins between the polls made multi-block reads
by DMA 26-28% faster (256 blocks 22.31 -> 15.95 ms, 64 blocks 6.06 ->
4.49 ms; 50 spins gave half the gain, 1,000 no more). A DMA memory copy
polled the same way gains 0.7% only, so it's the controller's own status
reads during its transfer that hold it up. `pxa-mmc` pauses 200 spins;
the poll limits (passes) are backstops, stretched in time only.

