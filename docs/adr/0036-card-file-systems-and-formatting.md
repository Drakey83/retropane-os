# ADR-0036: Card file systems (exFAT, FAT32, FAT16, FAT12) and formatting on the device

- **Status:** Proposed (2026-10-25). Built and checked on the PC on the
  owner's instruction ("build what I asked ... plan first, then build and
  check on the PC; ask me before anything runs on the Axim"); the votes
  below are open, each built with the default marked **(default)** so a
  different vote changes a constant, not the design.
- **Date:** 2026-10-25
- Reminders: R44 (formatting), R26 (old cards: FAT16, exFAT's first real
  card), R29 (the formatter's design points), R21 and R45 (later users).

## In plain words

Today RetroPane's file code (`rp-fat`) opens FAT32 and exFAT cards only,
and only the PC can format a card. Old SD and CF cards (2 GB and smaller)
usually come FAT16, and the smallest FAT12, so RetroPane can't open the
owner's 1 GB SanDisk CF card at all. This ADR adds FAT12 and FAT16 to the
same code, gives exFAT cards the boot-file guard that FAT32 cards already
have (so an exFAT card can be a writable drive on the PC too), and adds a
formatter that runs on the X51v and writes any of the four formats to the
SD or the CF card, laid out the way the SD Association's own formatter
does it (aligned to the card's erase blocks), not the way Windows does.

## Context

- `rp-fat` (ADR-0013): FAT32 read and write (copy-then-swap, crash-safe
  order, long names, folders, delete, disk check) and exFAT read and write
  (`exfat.rs`: contiguous files, bitmap allocation, entry-set checksums).
  The FAT32 mount accepts only volumes with FAT32's markers (no fixed root
  directory, a 32-bit FAT size).
- The boot-file guard (ADR-0014 point 3, ruling A): the PC writes sectors,
  so `rp_fat::Guard` records the boot files' entries, chains and data
  clusters and refuses any write that changes them. It is FAT32 only: an
  exFAT card is offered **read-only**, a FAT16 card not at all as a
  RetroPane volume (the PC still sees its sectors).
- The formatters that exist are host-only (`format::build`,
  `exfat::format::build`) and build whole images in memory for tests and
  the emulator: no FAT12/16, no alignment, the exFAT one writes a partial
  up-case table and no backup boot region.
- The owner's direction (R44, 2026-10-25): storage free of Windows Mobile
  anchors, the best format wherever no hardware blocks it; the current
  cards stay FAT32 while HaRET starts RetroPane from Windows Mobile.
  Windows Mobile 5/6 (Windows CE 5.x) has no exFAT, which came with CE 6.0
  (a claim: verified on the X51v when the exFAT card is first put in under
  Windows Mobile), so a card HaRET boots from must stay FAT.

## Decision

### 1. One volume type, four formats

`rp_fat::Volume` opens FAT12, FAT16 and FAT32; `AnyVolume` adds exFAT.
Every operation (find, read, put, mkdir, rm, extent and reserve for the run
logs, disk check, the guard) works on all four.

- **The type is decided as Microsoft's FAT specification says**, by the
  data-cluster count (under 4,085: FAT12; under 65,525: FAT16), except
  that a volume carrying FAT32's markers (no fixed root, a 32-bit FAT
  size) is FAT32 whatever its size, as Linux does, so the tiny FAT32 test
  images keep working.
- **FAT12/16's fixed root directory** is directory number 0 (cluster 0,
  which `..` entries already use for the root): read and written in place,
  never grown (a full root is `NoRoom`, as on any FAT16 system).
- **FAT entries of 12 and 16 bits**: the FAT cache holds a two-sector
  window, so a FAT12 entry that straddles two sectors is read and written
  in one place; end-of-chain and bad marks are mapped to FAT32's values
  inside the cache layer, so the rest of the code is unchanged.
- **FSInfo** only on FAT32 (FAT12/16 have none); FAT12/16 FATs are always
  mirrored.

### 2. The boot-file guard on every format

- **FAT12/16:** as FAT32 (ADR-0014's rules unchanged), with FAT12/16's own
  layout bytes (the boot sector's bytes 11-35; the label and serial after
  them may change), the fixed root directory's sectors judged like any
  directory sector, and 12-bit entries checked bit by bit (a write that
  covers half of a straddling entry is judged on the half it covers).
- **exFAT (new):** the PC may not change:
  - the partition table, and the boot region's layout: the main and backup
    boot sectors' bytes 0-105 and 108-111 (everything but VolumeFlags and
    PercentInUse, which Windows updates on its own) and boot sectors 1-11
    whole (their checksum excludes those three bytes, so it doesn't move);
  - a protected file's FAT entries (when it is chained), its data
    clusters, and its clusters' bits in the allocation bitmap (they stay
    set: the space can't be given away);
  - a protected file's entry set: the file entry's type and attributes and
    the set's size, the stream entry whole (flags, length, first cluster),
    the name entries whole; the timestamps and the set checksum may
    change (Windows may update times);
  - a protected folder's entry set: type, attributes, first cluster, name
    (its length and chaining may change as Windows adds files to it).
  Formatting the card from the PC is refused (the boot region is
  protected), as on FAT32.
- **When a guard can't be built** (a boot file in more than 8 pieces,
  unmirrored FATs, a damaged card) the card stays read-only, as today.

### 3. A formatter that runs on the device (`rp_fat::mkfs`, no_std)

`mkfs::plan(capacity, options)` computes the layout and checks it;
`mkfs::write(dev, &layout, scratch)` writes it through any `BlockDevice`
in multi-block runs, with no memory beyond the caller's scratch. The
host-only builders stay for tests and the emulator, and a host test checks
that every layout `mkfs` writes is mounted, written and checked by
`rp-fat` itself.

- **What it writes:** an MBR with one partition of the right type
  (FAT12 `0x01`, FAT16 `0x06`, FAT32 `0x0C`, exFAT `0x07`), the boot
  sector (FAT32 also its backup and FSInfo; exFAT its whole boot region
  and the backup with checksums), every FAT zeroed and its first entries
  set, the root directory, and for exFAT the allocation bitmap and
  Microsoft's recommended up-case table (spec section 7.2.5.1: 5,836
  bytes, checksum `0xE619D30D`), kept as 119 ranges (about 700 bytes) and
  expanded as it is written, byte-exact (host test).
- **Crash order:** the old boot sector is cleared first (a half-formatted
  card is never mounted as its old file system over a new FAT), the boot
  sector written last.
- **Checks before writing anything** (hostile and mistaken inputs, the
  testing rules): the cluster count must suit the format (FAT12 under
  4,085, FAT16 4,085-65,524, FAT32 at least 65,525, exFAT within its
  limits), the cluster size a power of two within the format's range, the
  card large enough; otherwise an error that says which and what would
  fit, and nothing is written.

### 4. Choices (the owner's votes; each built with its default)

**Q1. Which format by default (when the command says `auto`)?**
- **A (default): the SD Association's rule by size:** FAT12 to 64 MiB,
  FAT16 to 2 GiB, FAT32 to 32 GiB, exFAT above. Pros: what every camera
  and phone expects of a card that size; old cards stay readable
  everywhere. Cons: FAT32 on a 4-32 GB card is slower to mount than exFAT
  (Windows reads the whole FAT: 16 s on the 8 GB CF card, R42).
- B: exFAT for every card over 32 MiB. Pros: the fastest format here
  (a small bitmap instead of a large FAT, contiguous files with no chain);
  Cons: Windows Mobile and old PCs (before XP SP2 + update) can't read it,
  so HaRET can't boot from such a card.
- Either way `format` can name any format the size allows.

**Q2. Cluster size by default?**
- **A (default): the SD Association's sizes** (16 KiB for FAT12/16 cards
  over 64 MiB, 32 KiB for FAT16 at 1-2 GiB and FAT32, 32 KiB for exFAT to
  512 GiB, 128 KiB above). Pros: aligned to the card's pages; a small FAT
  (32 KiB clusters make the 8 GB CF card's FAT 1 MB instead of 7.7 MB).
  Cons: small files take more space (a 1-byte file takes 32 KiB).
- B: Windows' sizes (4 KiB for FAT32 to 8 GB). Pros: less slack. Cons: the
  big FAT and slow mounts we measured.

**Q3. Alignment?**
- **A (default): the partition start and the data area aligned to the
  erase unit:** 4 MiB on cards over 1 GiB, smaller on smaller cards (the
  largest power of two at most capacity / 256, at least the cluster
  size); the FAT padded so the data area starts on that boundary. Pros:
  each cluster sits inside one erase block; the SD Association's
  formatter does this, Windows doesn't. Cons: up to 4 MiB unused.
  The SD card's own AU size (its SD status, ACMD13) is not read yet: a
  follow-up (it replaces the guess, the code takes the value as input).
- B: Windows' 1 MiB start, data area unaligned.

**Q4. A card that holds boot files (the SD card's loader and HaRET, the
CF card's `RetroPane/`)?**
- **A (default): kept.** The loader reads the boot files into RAM,
  formats, writes them back and reads each back (copy, then verify), and
  the PC tool first saves them to `local/` too. `wipe-boot` formats them
  away, only when typed. Pros: the card still boots after a reformat; two
  copies exist the whole time. Cons: a power cut mid-format loses the
  card's copy (the PC's is kept).
- B: refuse any card with boot files.
- **exFAT on a card with boot files is refused** unless `wipe-boot` (or a
  future `unbootable`): Windows Mobile can't read exFAT, so HaRET couldn't
  start RetroPane from it.

### 5. Where it lives now: the loader (development), the OS later

- Loader command `format <sd|cf> <auto|fat12|fat16|fat32|exfat>
  [cluster <KiB>] [label <name>] [keep-boot|wipe-boot] [plan]`; `plan`
  prints the layout and writes nothing. `cargo xtask format <sd|cf> <fs>
  [...]` prints the plan, saves the boot files to `local/`, and formats only
  with `--yes`.
- ADR-0014's rules hold: the lock switch is obeyed (a locked SD card is
  never formatted), a format while the PC wrote in the last 2 s is
  refused (`card busy`), and after it the drive goes away and comes back
  ("medium changed") with its guard rebuilt.
- In the OS: Settings and File Explorer's "Format card" (phase 6), with the
  same `mkfs`, a confirmation, and never the card RetroPane runs from
  without a warning (R29).

## Status notes

- 2026-10-25, built and checked on the PC: FAT12/16 in `rp-fat` (host
  tests: a FAT12 entry straddling two sectors against the specification's
  packing, end and bad marks on every width, the fixed root's 512 entries,
  cards filled to the last cluster, 1,000 hostile cards each); the guard on
  every format (19 tests: what Windows may do passes, deleting, replacing,
  renaming or formatting a boot file is refused on all four, half a FAT12
  entry, FAT16's fixed root, exFAT's flags and timestamps against its names,
  bitmap bits, chains and boot region); `mkfs` (every format mounted,
  written and checked clean, the FAT type at the cluster-count edges as
  mount decides it, a format cut at every write leaves nothing mountable,
  the up-case table byte for byte); the loader's `format` and `cargo xtask
  format`. **Emulator (`cargo xtask emu-format`):** SD FAT32 -> FAT16 (the
  three boot files, `autorun.inf` and its icon kept and read back) -> exFAT
  (`wipe-boot`) -> FAT32 at 1 KiB clusters, CF -> FAT12 -> exFAT; exFAT
  refused while boot files are on a card; the PC's write over a boot file
  refused on FAT16, exFAT and the CF card; every card check clean. On the
  way: loader commands raised from 32 to 64 bytes (`rp-loadproto`); the
  loader's budget 128 -> 156 KiB (test image, logged in `xtask/src/size.rs`).
  **Not yet proven on the X51v** (the owner's rule): roadmap step 6.
- 2026-10-25, **proven on the X51v** (the owner present; loader from RAM,
  the main cards untouched and checked after): the old SanDisk 1 GB CF
  card, FAT16 as it came, opened by RetroPane for the first time (R26),
  formatted FAT16 on the device (541 ms, 4 MiB aligned); the spare 4 GB SD
  card formatted **exFAT** on the device (304 ms), the kit put on it, its
  boot files guarded: Windows' delete of `haret.exe` refused on the card
  (byte-identical after), Windows' photo written beside it; then FAT32
  with the six kit files kept and read back (1,092 ms). `chkdsk` (Windows'
  own checker) found no problems on either card after RetroPane's formats
  and Windows' writes; both drive icons shown through `autorun.inf`.
  Layouts and timings: docs/hw/sd.md, docs/hw/cf.md. Found: Explorer shows
  a refused delete as done (R50, the owner's ruling A), the serial-only
  fallback after a long run of card writes (R49), big `get`s failing at
  50 s (R48).

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | One file system crate for every format, the same guard rules for all; every layout the formatter writes is checked by our own reader on the host, in the emulator and on a real card |
| ✨ Modernize | exFAT becomes a full drive format (written, guarded); cards are formatted the way the SD Association specifies (erase-unit alignment) instead of the way Windows does |
| 🗄️ Legacy support | FAT12 and FAT16 cards (old SD and CF cards) open and write; no card is locked out for its age |
| ⚡ Better / faster | Large aligned clusters shrink the FAT 8x on the 8 GB CF card (mounts faster on the PC, R42); exFAT's bitmap and chainless files; the formatter streams with no extra memory |

## Security

Every value read from a card is hostile until checked (docs/security.md):
the FAT12/16 mount checks the BPB (sector size, power-of-two clusters,
root entries inside the volume, FAT size covering every cluster) with
overflow-checked arithmetic, as FAT32's does; directory walks stay bounded.
The formatter parses nothing from the card except, for `keep-boot`, the
files it reads back through the checked reader. The guard is judged per
sector, so a split or mixed write can't slip a protected byte through.

## Alternatives considered

- **A RetroPane flash format with MTP to the PC** (R44): the best format
  for flash in principle, but no PC or camera reads it, and MTP replaces
  the drive (ADR-0014). Kept as a later option for internal flash (R21),
  where no other device reads the medium anyway.
- **Format only on the PC:** no code on the device, but the owner asked
  for it on the device, and Windows can't align to erase units or format
  FAT32 over 32 GB.
- **A separate crate per format:** more code twice (directory walking,
  names, copy-then-swap); FAT12/16/32 differ only in the FAT entry width
  and the root directory.

## Consequences

- The loader grows (FAT12/16, the exFAT guard, the formatter, the up-case
  ranges): a test image, ratcheted with a logged reason.
- The OS gets one storage layer for SD, CF and, later, the internal flash
  (R21) and its icon (R45).
- Device proof needed (the owner's rule: nothing works until proven on the
  X51v): R26's spare SD card formatted exFAT on the device, the kit put on
  it, Windows refused a delete of a boot file; the 1 GB SanDisk CF card
  (FAT16) opened by RetroPane; a FAT16 format read by Windows.
