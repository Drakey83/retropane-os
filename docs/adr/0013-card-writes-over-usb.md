# ADR-0013: Writing files on the SD card over USB

- **Status:** Accepted (2026-10-11, by the project owner: "go")
- **Date:** 2026-10-11

## Context

The USB loader (ADR-0012) runs any image on the device with nobody at it,
but the loader itself, HaRET and its script live on the SD card, and the
only way to change them is a Bluetooth copy by the owner. RetroPane has no
SD write path yet: `rp-drv-sdcard` reads blocks (CMD17/18) and
`rp-drv-pxa-mmc` only drives the receive FIFO. The card is a 4 GB SDHC,
FAT32 (partition type `0x0C`, measured), with the kit files in its root.

## Decision

1. **SD writes in the drivers:** CMD24 (one block) and CMD25 + CMD12
   (several) in `rp-drv-sdcard`; the PXA27x MMC controller's transmit FIFO,
   `CMDAT.WRITE` and the program-done status in `rp-drv-pxa-mmc`. Tested
   against the mock bus and, end to end, in the emulator's SD card model.
2. **A minimal FAT32 writer (`kernel/fat`, `rp-fat`):** reads the boot
   sector, FAT and root directory (long names included); finds a file;
   follows its cluster chain. Replacing a file is **copy-then-swap**, not
   in-place: the new contents go into fresh clusters (one contiguous run
   when the card has one), the chain is linked in both FATs, then the
   file's directory entry is switched to the new chain and size with one
   512-byte sector write, and only then are the old clusters freed. Power
   lost at any moment leaves the complete old file or the complete new
   one, never a half-written loader. The contents are verified by reading
   them back through the same FAT code and checking the frame's CRC-32.
   Host tests use a FAT32 image built by the crate's own formatter
   (`std` feature), which xtask also uses for the emulator's card.
3. **A second frame type in the loader protocol:** `RPF1` = "write file":
   the name (length-prefixed) then the contents, under the usual length and
   CRC-32. The loader receives it into the image area, replaces the file,
   reads it back through the FAT code, checks the CRC, and replies
   `RPL OK <bytes> in <ms>, <n> clusters contiguous|scattered (<r> reads),
   read back in <ms>` (`<r>`: the write's read commands) or `RPL ERR <why>`. `cargo xtask put <file> [name]` sends one;
   `cargo xtask install` puts the kit's loader, `startup.txt`, `default.txt`
   and `haret.exe` on the card in one go.
4. **Emulator test:** the loader boots with a FAT32 card image, takes a file
   over the (emulated) USB port, and the test reads the image on the host
   and finds the new contents. On the device: `cargo xtask put` the loader
   itself, soft-reset, HaRET, and the new loader answers.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | The last manual step in the device loop goes away; the SD write path and FAT code arrive with tests on the host and in the emulator before the card is touched |
| ✨ Modernize | The card becomes something the PC updates over a standard USB serial link, no card readers, no Bluetooth |
| 🗄️ Legacy support | FAT32 as Windows Mobile formats it, so the card stays readable by Windows Mobile and any PC |
| ⚡ Better / faster | Multi-block writes from the start (ADR-0012's measurement: per-command cost dominates on this card); new contents allocated contiguously when possible, so HaRET's read of the loader is one run; the write's speed is reported in the reply and recorded in docs/perf.md |

## Alternatives considered

- **USB mass storage (expose the card to the PC as a drive):** no FAT code
  needed, but Windows Mobile's file system would also be absent while
  exposed, and the loader would need a second USB function. Worth doing
  later for users; not the shortest path for development.
- **A raw-sector "dd" over the loader with the PC doing the FAT work:** the
  PC would need the whole FAT logic anyway, and raw sector writes to a live
  card are easier to get wrong.
- **Replace in place (overwrite the file's own clusters):** no FAT changes,
  but a power loss mid-write leaves a torn loader on the card. Copy-then-swap
  costs the same code and never does.

## Status notes

- 2026-10-11: built. Host tests: SD writes against the fake card, the MMC
  transmit path against the mock bus, `rp-fat` (formatter, long names,
  contiguous and scattered replacement, full card, and the power-loss sweep:
  the disk fails at every write index in turn and the file always reads back
  whole as old or new). Emulator: `cargo xtask emu` boots the loader with a
  FAT32 card image, sends a 70,000-byte `retropane.bin` over the emulated USB
  port, gets `RPL OK ... contiguous`, a `not found` for a missing name, and
  the image on the host holds the new contents with `startup.txt` untouched.
  The probe lists the card's root directory, so the FAT reader meets the real
  card read-only first.
- 2026-10-11, on the owner's X51v: the probe read the card's FAT32 (4 KiB
  clusters, 972,800 clusters, files found); the new loader was sent to the
  old one as an image (`cargo xtask load rp-loader`: it moves itself home
  and takes over, no Bluetooth); `cargo xtask install` then wrote the
  loader, both scripts and HaRET to the card, each verified by read-back.
  Two device-only facts fixed on the way (docs/hw/sd.md): the PXA needs
  `CMDAT` stop-transmission on the CMD12 that ends a multi-block write, and
  the card stays busy after that CMD12, so the driver polls CMD13 until
  ready before the next command. Measured ~1.7 MB/s each way. **The last
  manual step in the device loop is gone.**

- 2026-10-12: files the card lacks are created (`Volume::create`/`put`:
  directory entry written last, long names, Windows' short names, root
  growth; power-loss sweep), so `put` and `install` need no hand copying.

## Consequences

- The loader grows (SD + FAT code); it's a test image, ratcheted freely
  (owner's rule, 2026-10-11).
- First code that writes the owner's card: a replaced file's old clusters
  are untouched until the new copy is complete and linked, the directory
  switch is a single sector write, and nothing else on the card is
  written. The first device use is on the loader file, which a Bluetooth
  copy can always restore.
- Not yet: creating new files or directories, timestamps (no clock yet),
  freeing space when the card is full.
- The FAT writer becomes the seed of the OS's file system support.
