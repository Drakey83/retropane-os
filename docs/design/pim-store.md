# The organiser's store: one store, designed once (Rule 0)

Calendar, Contacts, Tasks and Notes all keep their records in one store
(`shell/pim/src/store.rs`, crate `rp-pim`). It is designed here once; the
four apps' sheets point at this page. Follows
[principles.md](principles.md); the look is not this page's subject.

## What it has to do

- Keep a few thousand small records (an appointment, a contact, a task, a
  note of up to 2,000 bytes) through resets, crashes and a battery pulled
  at any moment.
- Never hold up a frame: the X51v's cards write a 512-byte sector in about
  2 ms, but one `put` of a whole file held the loop 532 ms (run r184s3a),
  and a single sector write has taken 98 ms (gpubench-0007).
- Fit a 64 MB device: no database engine in the image, no index on the
  card to keep in step.
- Stay readable by later versions and by a PC tool.

## What others do (findings, not the answer)

| System | How it keeps organiser data | What that costs here |
|---|---|---|
| Windows Mobile 5 | `pim.vol`: the EDB property database (appointments, contacts, tasks); notes as `.pwi` files. Binary pages rewritten in place | An interrupted page write could corrupt the volume; the cure was ActiveSync's copy on the PC. A database engine in ROM |
| Windows Mobile 2003 | The object store in battery-backed RAM | Everything lost when both batteries ran out |
| Android, iOS | SQLite: a B-tree file with a write-ahead log or rollback journal | Sound, but the engine is about 500 KB of code, and every commit is several writes plus a sync (journal, pages, header) |
| Palm OS | Record databases in RAM, a record per entry, HotSync to the PC | RAM-only; simple and fast, lost with power |
| Lean Linux, Nextcloud, maildir | A file per entry (`.vcf`, `.ics`, a text note) | Readable anywhere, but on FAT every change allocates clusters and rewrites directory sectors: many writes per change, and FAT itself isn't crash-safe mid-update |
| Flash file systems (JFFS2, LittleFS), databases' logs | Append-only logs with checksums; the newest valid record wins; compaction when full | The right shape for flash: one small write per change, torn writes detected |

## The ways

| | Way | For | Against |
|---|---|---|---|
| A | **A checksummed log in two fixed files** (`RPPIM/A.RPP`, `B.RPP`, 1 MiB each, reserved once and never resized): a change is one group at the tail (usually one sector); the records live in RAM (256 KiB) with an index; a full slot is packed into the other, whose header is written last | One sector write per change (about 2 ms, never a frame); a power cut loses at most the change in flight; the file system is never touched after the files are made (the card log's proven trick, R22); about 700 lines, no engine | Capacity is fixed in v1 (256 KiB of records, 2,048 entries); a PC can't read it without a tool |
| B | SQLite, or a B-tree of our own | Queries, no fixed RAM image | Hundreds of KB of code and several writes per commit for a few thousand rows we sort in RAM anyway. Rejected |
| C | A file per entry (vCard, iCal, text) | PC-readable with no tool | Many FAT writes per change, each a chance to tear the directory; listing 1,000 files is a directory walk. Rejected for the store; kept as the export format (later list) |
| D | One file rewritten whole on each change (the Weather settings' record, grown) | Simplest | A 100 KB rewrite per keystroke-level change: hundreds of milliseconds, and the whole store at risk in each. Rejected |

**Chosen: A.** It was iterated twice before it defended itself:

1. *First draft:* records appended densely, several changes sharing a
   sector. **Rejected by the sheet's own rule:** rewriting the tail sector
   to add a record puts the records already in it at risk if that write
   tears. So each change starts on a sector of its own: a group never
   shares a sector with one already committed.
2. *Second draft:* the arena marked dead records in place. **Rejected:** a
   record could change between a group's sectors being written, and its
   checksum would no longer match. So the arena is append-only between
   packs, and "dead" lives only in the index.

## Why ours wins

- **Against Windows Mobile:** no page is ever rewritten in place, so no
  interrupted write can corrupt what was already safe; no PC is needed to
  recover. Proven by test: a power cut after *every* write of a change, and
  of a slot move, always loads the last complete state
  (`a_power_cut_loses_only_the_change_being_written`,
  `a_cut_during_the_move_keeps_the_old_slot_the_truth`).
- **Against SQLite:** one write per change instead of several, and about
  700 lines instead of an engine; sorting and search run in RAM at RAM
  speed. What SQLite would add (ad-hoc queries) no v1 app needs.
- **Against files per entry:** one sector write, no directory or FAT
  change, so the PC's cached view of the card stays right and FAT's own
  fragility never enters.
- **Hostile input:** a slot's header, every group and every record are
  checked before use (magic, generation, length against the slot and the
  arena, CRC-32, field lengths ending exactly at the record's end); junk
  or stale groups end the log and never panic
  (`junk_and_stale_groups_never_load_and_never_panic`).

## The format (for a later version or a PC tool)

- **Slot header** (sector 0): `RPPIMS01`, generation (u32), CRC-32 of
  those 12 bytes. The valid header with the higher generation is the
  truth.
- **Group** (from sector 1, each starting on a sector): `RPG1`, the slot's
  generation, payload length, CRC-32 of the payload; then the payload,
  zero-padded. A group of another generation, a bad length or CRC ends
  the log.
- **Record:** kind (1 appointment, 2 contact, 3 task, 4 note), flags (bit
  0: removed), body length (u16), id (u32), then fields: tag (u8), length
  (u16), bytes. Text is UTF-8; numbers are 4 bytes little-endian. Unknown
  tags are skipped, so fields can be added without a new version. The
  newest record of an id wins.

## On the device (rp-today, `shell/today/src/organiser.rs`)

- **The files** are found at start-up where they lie (`rp_fat`'s
  `extent`: nothing on the card changes), or made on a card's first run
  (`reserve`: 1 MiB of zeros each in one run of sectors, once ever; a slot
  must never show what its clusters held before, since an older store's
  groups would read as this one's). A slot file a PC left short or in
  pieces is made again; the other slot is kept.
- **The load** reads the picked slot 64 sectors (32 KiB) a pass of the
  image's loop while nothing moves, so a full slot never holds a frame;
  the apps show the records once it is over.
- **A save** is the change's sectors written in place, one a pass while
  nothing moves; what still waits is written before the image leaves. A
  card that refuses three writes stalls the store (the records stay in
  RAM, the run log says so) and is tried again each minute.
- **No card, or no free megabyte in one run:** the apps work in RAM for
  the run and the run log says nothing is saved.
- **Proof:** `organiser`'s host tests (FAT32 and exFAT card images: a
  restart, a full slot's move, a dead card, a short slot file, the shared
  card's other files) and `cargo xtask emu-apps` (the image loads sample
  records from the card, saves what the taps add, and shows them after a
  restart).

## Limits in v1, and the later list

- 256 KiB of records and 2,048 entries in all (about 1,500 contacts with
  room for the rest). A bigger arena is a constant; a store that pages
  from the card is the later design if anyone fills it.
- On the SD card only. The CF card as a second home, and a copy on both,
  are later.
- **Later:** export and import as vCard and iCal files (way C's good
  half); a PC tool that reads the slots; encryption with the device lock
  (docs/security.md); sync (LegacySync, ADR-0028).

## What would overturn it

- A card whose sector writes tear *across* sectors already written
  (no card seen does): then each change would need its own erase block.
- Users with more than a few thousand records: the paging design.
