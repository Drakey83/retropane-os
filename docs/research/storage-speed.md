# Storage speed: CF and SD on the X51v (2026-10-15)

> **Report only.** Nothing here changes a driver; the owner decides what
> to change after seeing the numbers. Item 75 (CF) of the imitation audit,
> widened by the owner to the SD card ("make sure SD card speeds are also
> fully tested").
>
> **Sources:** (M) Intel PXA27x Developer's Manual, 280000-003 (memory
> controller 6.5.5 and Table 6-34; clocks Table 3-5 and `CCCR`; MMC
> controller 15.9.3); (CF) CF+ and CompactFlash specification, as
> reproduced in card makers' data sheets (Delkin C400, L5ENG00049);
> (SD) SD Physical Layer Simplified Specification (CSD, SCR, SD status,
> CMD6); (P) two report-only test images on the owner's X51v today,
> `rp-cftime` and `rp-sdtime` (boot/cftime, boot/sdtime); (D) Windows
> Mobile's own figures, `tools/wmbench` (docs/perf.md) and its OAL's
> clock table (read for hardware values only).

## The answer in one table

| | Today | Measured better path (no timing change) | Ceiling estimate | WM |
|---|---|---|---|---|
| **CF read, card to RAM** | **4.1 MB/s** (16-bit port, polled per sector) | 4.9 MB/s (32-bit loads from the data window, +18%); **5.2 MB/s by DMA** (+25%, CPU free for ~87% of each sector) | 9-12 MB/s with timings tuned to the card's own 80 ns rating (not tried) | 1.9 MB/s (file level) |
| **SD read, card to RAM** | 5.2 MB/s at 64 KiB commands by DMA; **1.9 MB/s at 1 MiB** (DMA falls back to PIO above 128 KiB) | 5.2 MB/s at every size above 64 KiB (longer DMA chain) | ~5.5 MB/s: the byte-wide FIFO and the card's latency, not the 9.6 MB/s bus | 3.0 MB/s (file level) |
| **SD write** | 1.9 MB/s (PIO: DMA writes are switched off) | **2.9 MB/s with DMA writes** (+52%, 0 errors, read-back exact) | ~3 MB/s: the card programs slower than the bus | 0.84 MB/s (file level) |
| **SD write, 4 KiB / one block** | 0.5-0.8 MB/s / **46 KB/s (10.8 ms a block)** | larger commands | - | - |
| **USB drive (PC sees)** | ~0.6 MB/s (CF), ~1 MB/s cap | - | USB full speed: ~1.1 MB/s | - |

Five things the measurements change:

1. **The CF socket is not why the CF drive is slow.** The card path reads
   at 4.1 MB/s today with Windows Mobile's timings; the drive's 0.6 MB/s
   end to end is the USB side (full speed caps it near 1 MB/s; item 74).
   Faster socket timing would make the PC's copy no faster at all. It
   matters for the OS reading the card itself (programs, media).
2. **WM's CF timings are generic, not the card's.** This card rates its
   common memory at **80 ns** (its CIS and IDENTIFY agree); WM programs a
   **260 ns** strobe cycle at 208 MHz, and a 529 ns one at its 104 MHz
   points: one table for every card, not derived from any card.
3. **Two CF wins need no timing change at all:** 32-bit loads from the
   card's data window and DMA from it. Both checked byte for byte against
   today's path over 512 KiB.
4. **SD is not "already maxed".** DMA writes are built but switched off
   (+52%), DMA silently gives up above 128 KiB (1 MiB reads run at a third
   of the speed), and single-block writes cost 10.8 ms each on this card,
   which is what the FAT layer's metadata writes are.
5. **Some ideas measured as worthless here:** a faster SD clock (the
   controller has none: the reserved clock code runs 12x *slower*), ACMD23
   pre-erase (no measurable gain), READ MULTIPLE on this CF card (it moves
   one sector per DRQ anyway), interrupts for CF reads (the card waits
   only 2% of the time).

## How it was measured

Two test images, each started by the loader under the Axim lock, run once
each on 2026-10-15 (logs kept in the session notes folder,
`rp-cftime-run1.txt` and `rp-sdtime-run1.txt`):

- **`rp-cftime`** reads the clocks and socket timings as found, powers and
  resets the CF card as the loader does, dumps the CIS (256 bytes) and all
  256 IDENTIFY words, times 1,024 single accesses per timing variant
  (made **longer** only, to calibrate each field's cost; restored after),
  then reads 1,024 sectors from LBA 0 five ways, each sector checksummed
  against today's path. **Nothing is written to the card's media.**
- **`rp-sdtime`** reads the SD card's CID, CSD, SCR, SD status and CMD6
  function table; times reads and writes at 4 KiB, 64 KiB and 1 MiB
  commands with and without DMA and ACMD23, each split into its phases;
  tries the controller's reserved clock code (reads only); times the FAT
  layer. Writes go **only into its own 2 MiB file `SDTIME.BIN`**, made
  through the FAT layer, checked contiguous, and only after every block
  matched its pattern; the same bytes are written back. Read back after:
  **0 of 4,096 blocks differ**. The file was removed afterwards
  (`cargo xtask rm`). Card check after the run: the root listing is
  entry for entry what it was before, and the loader kit files on the
  card (retropane.bin, haret.exe, startup.txt, default.txt,
  card-sd.ico, autorun.inf) match `target/sdkit`'s by SHA-256. Since
  then the image writes only inside its own folder `/rp-storage-bench/`
  and removes it at the end (the rule after another session's benchmark
  wiped a file on the card).

The core ran at 624 MHz turbo, 208 MHz run, with `CCCR` 0x02000310 (A = 1)
and `CLKCFG` B = 1 as Windows Mobile left it, so the **memory controller
clock CLK_MEM is 208 MHz** (M, Table 3-5: A = 1 makes CLK_MEM the system
bus clock, 208 MHz with B = 1). Measured, not assumed: each timing field
made longer costs what 208 MHz predicts (below).

# Part A: CompactFlash

## A1. The socket timings Windows Mobile set, decoded

Field meanings (M, Tables 6-34 and 6-35), in CLK_MEM cycles:

- **SET** (bits 6:0): address set-up before the strobe = SET + 2.
- **ASST** (bits 11:7): the strobe (nPOE/nPWE) = 3 x ASST + 6 for reads
  (3 x ASST + 5 for writes); nPWAIT is first sampled ASST + 2 cycles in.
- **HOLD** (bits 19:14): address hold after the strobe = HOLD.

At CLK_MEM 208 MHz (4.81 ns a cycle):

| Register | Value | SET / ASST / HOLD | Set-up | Strobe (read / write) | Hold | Cycle on paper | Measured per access |
|---|---|---|---|---|---|---|---|
| `MCMEM1` (common memory: the ATA registers) | 0x00020508 | 8 / 10 / 8 | 10 = 48 ns | 36 = 173 ns / 35 = 168 ns | 8 = 38 ns | 54 = **260 ns** | 444 ns (byte or half-word) |
| `MCATT1` (attribute memory: CIS, config) | 0x00020E08 | 8 / 28 / 8 | 10 = 48 ns | 90 = 433 ns / 89 = 428 ns | 8 = 38 ns | 108 = **519 ns** | 707 ns |
| `MCIO1` (I/O space, unused in memory mode) | 0x00020510 | 16 / 10 / 8 | 18 = 87 ns | 36 = 173 ns | 8 = 38 ns | 62 = **298 ns** | not used |
| `MECR` | 0x00000003 | two sockets, card inserted | | | | | |

**The decode is calibrated on the device**, not just read from the
manual (P): lengthening a field and timing 1,024 accesses gives each
unit's cost.

| Change (attribute memory, then common) | Extra per access | Predicted at 208 MHz |
|---|---|---|
| SET + 32 | +150 ns | 32 cycles = 154 ns |
| ASST + 3 | +47 ns | 9 cycles = 43 ns |
| HOLD + 16 | +74 ns | 16 cycles = 77 ns |
| ASST + 8 (common memory) | +114 ns | 24 cycles = 115 ns |

Every access also costs a **fixed ~185 ns** beyond its programmed cycle
(444 - 260 and 707 - 519): the CPU's uncached round trip through the
memory controller. It shrinks with wider or streamed access (below), not
with timing.

**Where WM's numbers come from:** the OAL's clock table (D) carries a
second set for its 91 and 104 MHz points: `MCMEM` 0x00010684 (SET 4,
ASST 13, HOLD 4: 55 cycles, **529 ns** at 104 MHz), `MCIO` 0x00010288; and
`MCMEM1` there is 0x000C0684, a HOLD of 48 against socket 0's 4 (a likely
table slip). Same card, same socket: 260 ns at one clock, 529 ns at
another. These are one generic table for any card, not a rating.

On paper WM's 208 MHz common-memory timing is even *tighter* than CF's
slowest (250 ns) mode in one respect: data is taken 46 cycles = 221 ns
after the address, against that mode's 250 ns address access time (CF).
It works because this card is far faster than 250 ns; a slow card (an
old Microdrive) could misread. Attribute memory is the opposite: 519 ns
against the 300 ns the specification asks.

## A2. What the card says it can do

The card: **"ULTIMATE CF CARD 8GB"** (CIS: KINGSTON), 16,250,976 sectors,
firmware 20070205.

| Source | Raw | Meaning | Confidence |
|---|---|---|---|
| CIS `CISTPL_DEVICE` (0x01) | `df 79 01 ff` | device speed code 7 = extended byte 0x79: mantissa 8.0 x 10 ns = **80 ns** | confirmed (CF/PC Card speed code) |
| CIS `CISTPL_DEVICE_OC` (0x1C) | `02 df 79 01 ff` | the same 80 ns at 3.3 V | confirmed |
| CIS `CISTPL_CFTABLE_ENTRY` 0 (memory mode) | `c0 c0 a1 01 55 08 00 20` | memory interface, uses READY and WAIT; no timing descriptor (the device speed above applies) | confirmed |
| IDENTIFY word 0 | 0x848A | CompactFlash | confirmed |
| word 47 | 0x8001 | READ/WRITE MULTIPLE: **at most 1 sector** a DRQ block | confirmed |
| word 49 / 53 | 0x0200 / 0x0003 | LBA; words 64-70 valid | confirmed |
| word 59 | 0x0100 | multiple-sector setting valid, 0 | confirmed |
| word 63 / 88 | 0 / 0 | no multiword or Ultra DMA reported (as the CF spec requires in PC Card modes) | confirmed |
| word 64 | 0x0003 | PIO 3 and 4 (True IDE) | confirmed |
| words 67 / 68 | 120 / 120 ns | minimum PIO cycle (True IDE) | confirmed |
| word 163 | 0x0492 | CF advanced True IDE: PIO 6 and MDMA 4 (80 ns class) | likely (CF 4.x layout) |
| word 164 | 0x001B | CF advanced PC Card modes: I/O and memory mode code 3 = **80 ns** (codes 0-3: 250, 120, 100, 80 ns), agreeing with the CIS | likely (CF 4.x layout; the CIS's 80 ns is confirmed) |

## A3. Actual against rated, per field

The card's ceiling is its 80 ns common-memory cycle. A setting that meets
it at 208 MHz, from Table 6-34's steps: **SET 1, ASST 2, HOLD 3** (set-up
3 = 14 ns, strobe 12 = 58 ns, hold 3 = 14 ns, cycle 18 = **87 ns**); a
middle step at the 120 ns mode: SET 2, ASST 5, HOLD 4 (cycle 29 = 139 ns).

| Field (common memory) | WM | For 80 ns | Headroom |
|---|---|---|---|
| Address set-up | 48 ns | 14 ns | 3.4x |
| Read strobe | 173 ns | 58 ns | 3.0x |
| Address hold | 38 ns | 14 ns | 2.7x |
| **Programmed cycle** | **260 ns** | **87 ns** | **3.0x** |
| Per access with the fixed ~185 ns | 444 ns | ~271 ns | 1.6x |
| Attribute memory | 519 ns | 300 ns (spec, all modes) | 1.7x, read a few hundred bytes once per attach: **not worth changing** |

The exact minimum set-up, strobe and hold for the 80 ns mode must be
taken from the CF specification's own tables before any change; the
data-sheet copy at hand lists only the 250 ns mode (ta(OE) 125 ns,
tsu(A) 30 ns, th(A) 20 ns), which WM's values meet with 1.4-1.9x margin.

## A4. The transfer path, measured

1,024 sectors (512 KiB) from LBA 0, each way, every sector compared with
path A's checksum (P):

| Path | Overall | Wait for DRQ | Data on the bus | Per half-word | Mismatches |
|---|---|---|---|---|---|
| **A** today: READ SECTORS, 16-bit data register | **4,148 KiB/s** | 2.4 ms (2%) | 114 us a sector | 447 ns | (reference) |
| **B** 32-bit loads from the data window 0x400-0x7FF | **4,880 KiB/s** (+18%) | 1.8 ms | 97 us | 378 ns | 0 |
| **D** DMA, memory to memory, from the data window, started at each DRQ | **5,175 KiB/s** (+25%) | 2.0 ms | 84 us | 330 ns | 0 |
| C / E READ MULTIPLE with B / D | not possible: the card's limit is 1 sector | | | | |

What each path is:

- **The data window.** In memory mode the CF card answers *every* address
  from 0x400 to 0x7FF with its data register (CF), so the CPU or the DMA
  controller can stream with incrementing addresses (load-multiple, or DMA
  bursts) instead of hammering one register. The PXA splits each 32-bit
  access into two 16-bit card cycles itself.
- **DMA without a request line.** The PXA has no DMA request for a PC Card
  socket, but a memory-to-memory transfer from the window works: started
  by the CPU once DRQ is up, it moved every sector exactly (P), no bus
  errors. The CPU is free for the 84 us the sector takes, ~87% of the
  sector time, where today it spins on every half-word. HAL:
  `Dma::arm_copy_from_device_window` (one descriptor per window).
- **The card's interrupt.** GPIO84 is the card's RDY/-BSY in memory mode:
  it read low in 587 of 1,612 polls while the card was preparing a
  sector (P). So a GPIO84 edge can wake the driver with no I/O mode (where
  the same pin becomes IREQ). For reads it saves nothing: the card waits
  ~2 us a sector, less than an interrupt costs. It is worth it for
  **writes** (flash programming takes milliseconds) and to let the CPU
  sleep; the read loop's speed comes from the data path, not the wait.
- **READ/WRITE MULTIPLE:** this card allows one sector per DRQ block
  (word 47), so it gains nothing here; other cards may allow more, and
  the driver can use what word 47 says.

**Estimated with tuned timings (not tried):** the programmed cycle falls
from 260 to 87 ns, the fixed overhead stays (it depends on the path):

| Path | Per half-word now | At 87 ns | Sector | Throughput |
|---|---|---|---|---|
| A (16-bit port) | 447 ns | ~271 ns | ~69 us | ~7 MB/s |
| B (window, 32-bit) | 378 ns | ~205 ns | ~53 us | ~9 MB/s |
| D (DMA from window) | 330 ns | ~157 ns | ~40 us | ~12 MB/s |

These are upper bounds: the card's own wait per sector (2 us at today's
pace) may grow when asked faster, and the fixed overhead is not proven
constant. At the middle (120 ns) step D would give about 9 MB/s.

## A5. Where the CF drive's 0.6 MB/s goes

USB full speed carries at most ~1.1 MB/s of bulk data, and the loader's
drive today manages ~0.6 MB/s (item 74, the USB session measures it). The
card delivers 4.1 MB/s, so ~85% of the drive's time is the USB side.
**No socket timing change speeds up a PC copy.** WM itself read this card
at 1.9 MB/s at file level with the same timings (docs/perf.md).

# Part B: SD

## B1. What the card says

The owner's 32 GB SDHC card, 61,067,264 blocks, FAT32 with 32 KiB clusters.

| Register | Raw | Meaning |
|---|---|---|
| CID | 8903034e43617264101930291200a13f | maker 0x89, "NCard", rev 1.0, made 2010-01: an unbranded card |
| CSD | 400e003253590000e8f37f800a40403f | CSD 2.0 (SDHC); TRAN_SPEED 0x32 = **25 MHz**; command classes 0x5B5 (includes switch, class 10) |
| SCR | 0245804300000000 | SD 3.0x; 1- and 4-bit bus; CMD20 and **CMD23** supported |
| SD status | speed class byte 0x04, AU 0x9, UHS grade 3 | **claims Class 10** (10 MB/s writes), 4 MiB allocation unit, U3 |
| CMD6 query | function group 1 = 0x8003 | default and High Speed (50 MHz) supported |
| CMD13 | 12 us a poll | |

The card's claims (Class 10, U3) are measured below at 19.5 MHz: writes
reach 2.9 MB/s by DMA, so whatever it is, it isn't the bus that holds
writes back.

## B2. Reads

`read_many` (the driver's path) over 4 MiB at each command size, then one
command split into its phases (P):

| Command | PIO | DMA | Per command (DMA) |
|---|---|---|---|
| 1 block (CMD17) | 476 KiB/s | (single blocks never use DMA) | 1,049 us |
| 4 KiB | 1,399 KiB/s | 2,696 KiB/s | 1,483 us |
| 64 KiB | 1,919 KiB/s | **5,168 KiB/s** | 12,382 us |
| 1 MiB | 1,899 KiB/s | **1,895 KiB/s: fell back to PIO** (0 transfers by DMA) | 540 ms |

Where the time goes:

- **Start-up a command: ~0.84 ms**, whatever its length (CMD18 + CMD12
  of 1 block: 1,122 us; 8 blocks: 3,124 us). This is the card's read
  access time (the CSD's TAAC says 1 ms). At 4 KiB it is 57% of the time;
  at 64 KiB 7%.
- **Per block, DMA: 96 us** against 52.5 us of 4-bit bus time at 19.5 MHz
  (1,024 data clocks + CRC and gaps). The rest is the byte-wide FIFO
  (each byte is its own DMA access) and the card's gap between blocks;
  the run can't separate the two.
- **Per block, PIO: 269 us**: the CPU reads the FIFO one byte at a time.
- **CMD12 after a read: 13 us.**
- **The 1 MiB cliff:** the HAL's DMA chain holds 32 descriptors of 4 KiB
  (128 KiB); a longer read is quietly done by PIO at a third of the speed.
- **Bus ceiling:** 19.5 MHz x 4 bits = 9.75 MB/s raw, ~9.6 MB/s of data.
  DMA reaches 5.2 MB/s at 64 KiB; the gap is the per-block 96 vs 52.5 us
  and the command start-up.

## B3. The clock: is 19.5 MHz really the top?

Yes. `MMC_CLKRT` divides a fixed 19.5 MHz (the 312 MHz peripheral PLL /
16, M 3.5.4 and 15.9.3): codes 0-6 are 19.5 MHz down to 304 kHz, code 7
is "reserved". Tried read-only (P): code 7 read 64 blocks correctly but
**12x slower** (222 ms against 17.5 ms), so it is a further division, not a
faster clock. The card's High Speed mode (CMD6, 50 MHz) needs a faster
clock than the controller has, so it gains nothing here.

## B4. Writes

1 MiB at each command size, raw (CMD25, CMD12, then CMD13 until ready),
the pattern written back over itself (P):

| Command | PIO | PIO + ACMD23 | DMA | DMA + ACMD23 |
|---|---|---|---|---|
| 4 KiB | 496 KiB/s | 739 KiB/s | 843 KiB/s | 770 KiB/s |
| 64 KiB | 1,905 KiB/s | 1,906 KiB/s | **2,900 KiB/s** | 2,889 KiB/s |
| 1 MiB | 1,951 KiB/s | 1,950 KiB/s | 1,944 KiB/s (PIO: over the DMA chain) | 1,945 KiB/s (PIO) |
| 1 block (CMD24) | **46 KiB/s, 10.8 ms each** | | | |
| driver `write_many`, 64 KiB | 1,897 KiB/s (its ACMD23, PIO) | | | |

Where the time goes, per 64 KiB command (DMA): data phase 20.3 ms (158 us
a block against 52.5 us of bus: the card holds busy between blocks while
it programs), CMD12 12 us, then CMD13 polls 1.45 ms. Per 4 KiB command:
3.2-3.9 ms data phase and 1.5 ms of polls: this card needs ~5 ms for any
small write.

- **ACMD23 (pre-erase):** no gain at 64 KiB and 1 MiB. The one change (4
  KiB PIO: 496 to 739 KiB/s) was the first test of the run, and only its
  CMD13 time differs (1,059 ms against 376 ms for every other 4 KiB
  test), so it is the card settling after the FAT layer had just written
  2 MiB, not pre-erase. With DMA it was 843 against 770. **Measured
  worthless on this card.**
- **Write-busy by interrupt instead of polling:** the polled part is
  small (1.45 ms a 64 KiB command, 7%); the card's busy is mostly inside
  the data phase, where DMA already leaves the CPU free if the driver let
  it do something else. The controller has the interrupts for both
  (`MMC_I_REG` programming done and data done) and the DMA channel its
  end interrupt: a CPU-time and battery win for the OS's driver (item
  73), **not a speed win**.
- **DMA writes:** built, and off by default since SD DMA was first proven
  on reads only. This run made 544 DMA writes, all correct (read back
  exact).

## B5. The FAT layer

| Operation | Speed | Note |
|---|---|---|
| Make a new 2 MiB file (`put`, PIO) | 1,131 KiB/s (1.81 s) | ~0.76 s over the raw write: finding free clusters with single-block FAT reads (~1 ms each) and metadata writes |
| Replace a 1 MiB file (`put`, PIO) | 1,678 KiB/s (610 ms) | ~90 ms over raw: about 8 single-block metadata writes at 10.8 ms |
| Remove it | 47.6 ms | ~4 single-block writes |
| `read_file` 2 MiB, 4 KiB scratch (PIO) | 1,379 KiB/s | 1.5% under the raw 4 KiB read |
| `read_file` 2 MiB, 64 KiB scratch (PIO) | 1,811 KiB/s | 6% under the raw 64 KiB read |

(The FAT runs were PIO, as the loader's writes are; reads would be ~5 MB/s
with DMA.) **The FAT layer's cost is its single-block I/O**: every
metadata write costs 10.8 ms on this card, every FAT sector read ~1 ms.

## B6. Against Windows Mobile

WM, 4 MB through its file system (D): **read 2,987 KB/s, write 840 KB/s.**
RetroPane: reads 5.2 MB/s at 64 KiB DMA commands (1.7x WM), writes 1.9
MB/s PIO (2.3x), 2.9 MB/s with DMA writes (3.5x); file level, `put` 1.7
MB/s (2x WM's writes).

# Alternatives considered

| Idea | Verdict | Why |
|---|---|---|
| **CF: tighter socket timings** (to the card's 80 ns) | **Promising, risky**: ~2x on the bus | 3x headroom per field (A3); needs the validation plan below. Gains nothing for the USB drive (A5) |
| CF: 32-bit loads from the data window | **Do**: +18%, measured, no timing change | the same card cycles, fewer CPU round trips |
| CF: DMA memory-to-memory from the window | **Do**: +25% and the CPU free | measured exact; needs a DRQ wait first (no request line) |
| CF: READ/WRITE MULTIPLE | Use when word 47 allows; **nothing on this card** | one sector a DRQ block |
| CF: the card's interrupt (GPIO84 RDY/-BSY) | For writes and idle; **not for read speed** | reads wait 2% |
| CF: I/O or True IDE mode | Not needed | memory mode has the data window and the busy line; I/O mode's IREQ is the same pin |
| CF: the attribute memory timing | Leave | read once per attach |
| **SD: DMA writes** | **Do**: +52% | built, measured exact |
| **SD: DMA for transfers over 128 KiB** (more descriptors, or 128 KiB pieces within one command) | **Do**: 1 MiB 1.9 -> ~5.2 MB/s reads | today's quiet PIO fallback |
| SD: larger commands, read-ahead | **Do** where the caller allows | 0.84 ms per read command, ~5 ms per small write |
| SD: FAT metadata in fewer, larger writes; FAT cached in RAM; free-cluster map | **Do** (file-system design) | single-block writes cost 10.8 ms |
| SD: faster card clock | **Impossible** | fixed 19.5 MHz; code 7 is slower |
| SD: High Speed mode (CMD6) | Nothing to gain | needs > 25 MHz |
| SD: ACMD23 pre-erase | **No gain measured** | keep it (harmless) or drop it; not a lever |
| SD: CMD23 instead of CMD12 | Negligible | CMD12 costs 13 us |
| SD: busy by interrupt | CPU/battery, not speed (item 73) | the polled part is 7% |

# Difficulty and risk

| Change | Difficulty | Risk | What could go wrong |
|---|---|---|---|
| SD DMA writes on | trivial (a flag) | low | a DMA fault mid-write leaves blocks inside the written range unknown; the driver already maps DMA errors to errors, the file system's ordering covers it. Measured: 544 transfers, 0 errors |
| SD DMA chain > 128 KiB | small (HAL) | low | a wrong descriptor count corrupts *reads* into RAM: read-back tests catch it at once |
| CF 32-bit window loads | small | low | another card that doesn't alias 0x400-0x7FF (the CF specification requires it in memory mode): checked per card at attach by comparing one sector both ways |
| CF DMA from the window | medium | low-medium | DMA reading a card that stops answering holds the bus (nPWAIT): the soft watchdog fires, as for the CPU today |
| **CF tuned timings** | medium | **high** | **too short a strobe returns wrong bytes with no error**: no CRC on the PC Card bus, no status bit. Corruption is silent and can land in the file system's own structures. Also: slower cards (Microdrives, old 250 ns cards) must keep slow timings, so timings must come from each card's CIS/IDENTIFY, never one table |
| SD/CF interrupts | medium (the OS's driver model) | low | latency, not data |

The asymmetry matters: SD has a CRC on every block, so a wrong setting
shows as an error; **the CF socket has none.** Anything that shortens the
CF timings must prove itself by reading known data, and must keep proving
it in use.

# Read-back validation and stress test (before any CF timing change)

1. **Per card, at attach:** read the CIS and IDENTIFY at WM's timings;
   derive the target timing from the CIS device speed and word 164 (the
   slower of the two), never faster than the card says; cards that say
   nothing keep the slow table.
2. **Known-data proof per step:** step from WM's timing toward the target
   one field at a time (hold, then set-up, then strobe), and at each step
   read the same 4 MiB at the old and new timing and compare **every
   byte**; also read the CIS and IDENTIFY again and compare. Stop at the
   first difference and back off two steps.
3. **Margin:** run at one step slower than the fastest that passed
   (never at the edge), and record the margin in the run log.
4. **Stress:** 30 minutes of mixed reads (sequential and random LBAs)
   with the CPU busy, the 2700G drawing and USB traffic (the buses
   share the memory controller), at hot and cold device temperature
   and on battery and on the charger (supply droop); 0 differences
   allowed.
5. **Writes:** only after reads pass, in a dedicated test file as
   `rp-sdtime` does (pattern written back over itself, verified first),
   then read back at WM's timing.
6. **Several cards:** at least three different CF cards (one slow), as
   WM's single table had to serve all of them.
7. **In use:** the driver keeps a CRC-free check: at each attach, one
   known sector read at both timings; a mismatch drops the card to WM's
   timing for good and logs it.
8. **The emulator** models the decode (PXA27x Table 6-34) and a card
   speed per CIS, so the derivation is tested on the host; the socket's
   electrical behaviour can't be modelled (docs/hw/emulator.md).

# Device results: the speed-ups built (2026-10-16, `rp-storfast`)

Run on the owner's X51v (main 03ef598, loader from RAM; the SD card the
owner's 32 GB "X51V SD", the CF card 8 GB; run log in
`local/rp-storfast-run1.txt`). Every number below is from
one run; nothing failed, nothing differed.

| Path | Before | After | |
|---|---|---|---|
| SD read, 1 and 4 MiB commands | 1,895 KiB/s (FIFO) | 7,940-7,981 KiB/s (DMA, one transfer each) | 4.2x |
| SD write, 1 MiB commands | 1,284 KiB/s (FIFO) | 4,891 KiB/s (DMA) | 3.8x |
| SD write, 64 KiB commands | 1,167 KiB/s (FIFO) | 2,897 KiB/s (DMA) | 2.5x |
| CF read, 4 MiB | 2,569 KiB/s (16-bit port) | 3,439 (32-bit window), 5,887 KiB/s (DMA from the window) | 2.3x |
| CF reset to ready | 100 ms pulse + ready wait | 28 ms in all (100 us pulse); 20 of 20 attach cycles good, slowest 28 ms | |

- **S1:** every size class from 2 blocks to 4 MiB by one DMA transfer, every
  byte checked (item 1: before it, reads over 128 KiB fell back to the FIFO).
- **S2 soak:** three rounds of 481 writes each (146,943 KiB per round,
  ~430 MB in all), every write read back by DMA and every fourth by the
  FIFO too: 0 write errors, 0 read errors, 0 blocks differ. So item 2 (DMA
  writes on by default in the loader) may go ahead.
- **S3 card swap:** the card pulled and put back mid-soak; the last writes
  read back exact and the soak went on. **Lock switch: not tested.** The
  X51v's slot covers the card's lock tab, so "slide it on with the card in"
  can't be done; the step must ask for card out, lock, card in.
- Only the test's own folder was written and it was removed; every root file
  of the shared card hashed the same before and after.

# The owner's decision: the CF socket's timings (reminder R7, 2026-10-24)

**Ruled: A (the owner, 2026-10-24): "there are probably going to be a lot of
slow CF cards used with this."** Each card runs at its own rated speed, so
A works both ways: a fast card runs faster than Windows Mobile's one table,
and a card rated slower than that table (the CF standard's slowest memory
mode is a 250 ns cycle; old cards and Microdrives sit there) gets a slower,
safe timing that Windows Mobile never gave it. Proven only by a device run
on the owner's cards.

**Built 2026-10-24.** `drivers/cf/src/timing.rs` turns the card's CIS rating
into the socket's fastest timing meeting the CF specification's minimums
with a 1.5x margin (`for_card`; the owner's 80 ns card: 29 memory clocks a
cycle against Windows Mobile's 54, 1.87x; a card rated above ~630 ns gets
the socket's slowest, the longest strobe it has). `CfCard::tune` reads the
rating at attach (`CISTPL_DEVICE_OC`, the 3.3 V tuple, before
`CISTPL_DEVICE`), hashes IDENTIFY and the first, middle and last sectors at
the timing it has, sets the card's timing, and keeps it only if both
re-reads hash the same; otherwise the old timing goes back and the card is
read again there (a card that ran out a wait at the new timing isn't left
dead if it answers at the old one). Host tests: a card that misreads or
hangs when too fast, one that dies mid-check, one unreadable before, the
clamp, the register's other bits kept. The device test is `rp-storfast`
C4-C5 (reads only): each read path at the card's timing, three passes, every
sector against the port's at Windows Mobile's, the speed before and after;
then, for the record and never kept, each field shortened a code at a time
to the first misread: the card's real margin against the 1.5x one. On the
emulator (a 150 ns card, timing not modelled) everything reads right and
Windows Mobile's `MCMEM1` comes back. The loader tunes each card at attach
once the device run proves it.

The free wins are in the driver since this report (32-bit loads, DMA:
`drivers/cf`, 5.2 MB/s). What's left is the socket's timing itself:
Windows Mobile programs one table for every card (a 260 ns strobe), and the
owner's card rates itself at 80 ns (its CIS and IDENTIFY agree).

- **A (recommended): each card at its own rated speed.** At insertion the
  driver reads the card's own timing from its CIS, programs the socket for
  it with a margin (say 1.5x the rating), and checks it: a sector read twice
  at the new timing must match its read at Windows Mobile's; if anything
  disagrees, or the CIS is unreadable, it falls back to the generic table and
  logs why. Plus: every card runs as fast as it says it can (this one ~2x:
  9-12 MB/s raw against 4.1 measured, estimate); an old slow card keeps a
  slow, safe timing; nothing depends on the owner's unit (the any-Axim
  rule). Minus: a card that lies about its speed could read wrong; the check
  at insertion and a write-verify soak in the device test cover that; one
  watched device run to prove it on the owner's two cards.
- **B: keep Windows Mobile's table.** Plus: nothing to test. Minus: every
  card at a third of what it can do.
- **C: one faster table for every card.** Plus: simple. Minus: an older
  card rated slower than the table would read wrong: the reason Windows
  Mobile picked a slow one.

The gain matters for the OS reading programs and media from the CF card,
not for the PC's copy over USB (full speed caps that near 1 MB/s).

# Copied values (the audit's tag)

| Value | From | Status after this report |
|---|---|---|
| `MCMEM1` 0x00020508 | WM | decoded: 260 ns cycle, 3x the card's need; generic table (A1). **Open: owner's decision** |
| `MCATT1` 0x00020E08 | WM | 519 ns against 300 ns; not worth changing |
| `MCIO1` 0x00020510 | WM | unused in memory mode |
| `MECR` 3 | WM | correct for two sockets, card inserted: a fact, not a choice |
| CF RESET held 100 ms (`drivers/cf`) | WM | the specification asks 10 us; a change to 1 ms is ready to make but **not made** (outside "report only"; needs the owner's go) |
| CPLD power step 10 ms, off 308 ms, on 100 ms (loader) | WM | board-specific, no specification value; the 10 ms step may be the slot's supply ramp before its bus buffers turn on (back-powering a card through its pins is the risk): not shortened without a scope. Open |
| SD 19.5 MHz | hardware | confirmed the top (B3) |
| SD DMA writes off | ours (caution) | measured safe, then soaked on the device (430 MB, 0 errors, 2026-10-16): switch on |

Totals: 8 values looked at; 2 confirmed as hardware facts, 2 settled as
not worth changing, 4 open (CF timing, CF reset hold, CF power waits, SD
DMA writes), each with numbers for the owner.
