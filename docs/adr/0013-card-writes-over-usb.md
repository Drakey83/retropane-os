# ADR-0013: Writing files on the SD card over USB

- **Status:** Proposed (2026-10-11)
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
2. **A minimal FAT32 writer (`kernel/fat`):** reads the boot sector, FAT and
   root directory; finds a file by its 8.3 name; follows its cluster chain.
   **Phase 1: replace in place.** A new version that fits in the clusters
   the file already has is written over them and the directory entry's size
   updated: no FAT changes at all, so the worst case (power lost mid-write)
   is that one file. **Phase 2: grow.** Allocate free clusters and extend the
   chain, updating both FAT copies, for files that outgrow their chain.
   Writes are verified by reading back. Host tests use a FAT32 image built
   by a small formatter in the tests themselves.
3. **A second frame type in the loader protocol:** `RPF1` = "write file":
   name, length, CRC-32, contents. The loader receives it into the image
   area, writes it to the card, verifies, and replies `RPL OK` / `RPL ERR`.
   `cargo xtask put <file>` sends one; `cargo xtask load --install` puts the
   kit's loader, HaRET and script on the card in one go.
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
| ⚡ Better / faster | Multi-block writes from the start (ADR-0012's measurement: per-command cost dominates on this card) |

## Alternatives considered

- **USB mass storage (expose the card to the PC as a drive):** no FAT code
  needed, but Windows Mobile's file system would also be absent while
  exposed, and the loader would need a second USB function. Worth doing
  later for users; not the shortest path for development.
- **A raw-sector "dd" over the loader with the PC doing the FAT work:** the
  PC would need the whole FAT logic anyway, and raw sector writes to a live
  card are easier to get wrong.

## Consequences

- The loader grows (SD + FAT code); it's a test image, ratcheted freely
  (owner's rule, 2026-10-11).
- First code that writes the owner's card: phase 1 only touches clusters
  the file already owns, and the first device use is on the loader file,
  which a Bluetooth copy can always restore.
- The FAT writer becomes the seed of the OS's file system support.
