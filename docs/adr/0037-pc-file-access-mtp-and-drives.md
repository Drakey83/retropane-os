# ADR-0037: Files to the PC: MTP for Windows (and Linux), drives kept for every other computer

- **Status:** Accepted (2026-10-25, the owner: "I vote the defaults for
  Q1-Q5 (Q1 A, Q2 A, Q3 A, Q4 "Axim X51v", Q5 the PDA icon). Start
  building steps 1-5; ask me before anything runs on the Axim"). Not yet
  built; nothing works until proven on the X51v. Its first layer for the
  loader is built (ADR-0014 rule 3: the PC's view of the boot files).
- **Date:** 2026-10-25
- Reminders: R53 (this project), R51 (Windows shows a refused delete as
  done), R21 (the internal flash as a drive), R46 (the PDA icon), ADR-0014
  (USB mass storage), ADR-0036 (card formats), ADR-0013 (`rp-fat`).
- **Sources** (cited by section, never copied: both documents are licensed
  for internal use only and stay in `local/specs/`):
  - **[MTP]** USB-IF, *Media Transfer Protocol* revision 1.1, 6 April 2011
    (`local/specs/mtp-1.1/`), all 282 pages read 2026-10-25.
  - **[SIC]** USB-IF, *Still Image Capture Device Definition* revision 1.0,
    11 July 2000, with its errata of 16 March 2007
    (`local/specs/still-image-1.0/`), all 46 pages read 2026-10-25. This
    is MTP's USB transport: containers, class requests, the event pipe.
  - **[INF]** Windows 11's own MTP driver file on the owner's PC,
    `C:\Windows\INF\wpdmtp.inf` (read 2026-10-25): it binds
    `USB\MS_COMP_MTP` *and* `USB\Class_06&SubClass_01&Prot_01`.
  - **[MSOS]** Microsoft OS 1.0 descriptors (string 0xEE, extended
    compatible ID "MTP"): Microsoft Learn, "Microsoft-defined USB
    descriptors".
  - **[QEMU]** QEMU's `usb-mtp` device and its commit "fix
    usb_mtp_get_device_info so that libmtp on the guest doesn't complain":
    libmtp expects vendor extension ID 6, version 100, and treats
    0xFFFFFFFF as "a camera, not MTP".

## In plain words

Today the X51v shows its SD and CF cards to the PC as ordinary drives (USB
mass storage). The PC then writes the card's sectors itself, so when the
device has to refuse something (deleting a file RetroPane boots from), the
refusal reaches Windows only after Explorer has already shown the file
gone. On the X51v on 2026-10-25 that looked broken. The owner: no blink,
nothing that looks broken, and no support dropped.

With MTP the PC never touches the card's sectors: it *asks* the device for
each file and each change ("send me this file", "delete that one"), and
the device answers yes or no at once. Windows shows the X51v under "This
PC" as a portable device (like a phone), with SD Card, CF Card and later
Internal Storage inside it. The device keeps using its cards while the PC
browses, so nothing has to "own" a card any more.

Computers that don't speak MTP (Macs, mainly) get today's drives
automatically, a few seconds later.

## What others do (researched 2026-10-25)

- **Android (since 3.0/4.0): MTP.** Mass storage handed the PC the
  partition, so the phone couldn't use its own storage while plugged in;
  MTP keeps the file system the phone's (Dan Morrill, Android, via
  Engadget and OMG Ubuntu). An operation the device won't do gets a
  refusal (QEMU's MTP device answers "store read-only"). *Weak spots
  people report:* slow (a round trip per object), copies cut off by an
  unplug leave half files, Explorer showing stale folders, Macs needing a
  separate app, and its MTP interface only (the drive mode was removed).
- **Kindle: the system isn't on the drive** (only `/mnt/us` is shown). For
  our cards that means a second partition, which Windows Mobile can't show
  (first partition only, XDA reports): not usable while HaRET boots
  RetroPane.
- **Older Garmin GPS: hidden system files** (a curtain, not a lock).
- **Raspberry Pi Pico-style boot drives:** a made-up FAT drive that drops
  unwanted writes silently: the problem we had.

## What the specifications require (the facts the design rests on)

Each line: what the source says, then what it means here.

1. **Transport [SIC 4-7].** One interface with three endpoints (bulk IN,
   bulk OUT, interrupt IN), packets of at most 64 bytes (the PXA27x UDC is
   full speed: 64 is also its bulk maximum). Every phase travels in a
   container: 32-bit length, type (command, data, response, event),
   16-bit code, 32-bit transaction ID, then up to five 32-bit parameters
   or the data (all little-endian). A phase ends on a short or empty
   packet. One command at a time: the response comes before the next
   command [SIC 7.1].
2. **Class requests [SIC 5.2, errata 2007]:** Cancel (0x64, carries the
   transaction to cancel), Get Extended Event Data (0x65, optional),
   Device Reset (0x66: back to idle, session closed), Get Device Status
   (0x67: OK, busy, or cancelled with the stalled endpoints). Their
   `wIndex` is the interface number (the errata), which matters because
   ours is a composite device.
3. **Cancelling [SIC 7.2.1, annex B].** The PC cancels by stopping and
   sending Cancel, then polling Get Device Status until OK. The device
   cancels by stalling both bulk endpoints. A response is never
   cancelled.
4. **Suspend [SIC 4.5].** With remote wake-up off, events that happen
   while suspended are reported after resume as one UnreportedStatus
   event: no event queue is needed.
5. **Sessions and handles [MTP 3.4, 4.4].** One session (SIC devices are
   single-session). Object handles are 32-bit, chosen by the device,
   unique across all stores, valid only in their session, and **a deleted
   object's handle is never reused in the same session**. 0 and
   0xFFFFFFFF are reserved.
6. **Stores [MTP 5.2, D.2.4].** A store ID's top 16 bits name the
   physical store, the bottom 16 the logical one; bottom 0 means "no
   medium". An empty card slot is still listed, with bottom 0.
7. **Objects [MTP 5.3].** Every file and folder is an object; folders are
   "associations" of type 1 (generic folder); a root object's parent is
   0. ObjectInfo carries format, protection status, size (0xFFFFFFFF past
   4 GB), parent, name and dates. Strings are UTF-16 with a count byte,
   **at most 255 characters including the terminator** [MTP 3.2.3].
   Dates are `YYYYMMDDThhmmss`, optionally with a time zone [MTP 3.2.5].
8. **Protection status [MTP 5.3.1.3, B.2.3]:** 0x0000 none, 0x0001
   read-only (no delete, no change, no property change by the PC),
   0x8002 read-only data (properties such as the name may change), 0x8003
   not transferable. *Found: R53's note "read-only 0x8001 (Android)" is
   not an MTP value; corrected in R53.*
9. **Hidden and System properties [MTP B.2.13-14]** are hints for the
   PC's user interface only. The specification says that an object the
   device doesn't want the PC to have must not be listed at all.
10. **Properties [MTP 5.3.2].** A device offering object properties must
    offer at least: store, format, size, persistent unique ID, name, and
    parent (with folders). **The persistent unique ID** (128 bits) stays
    the same for as long as the object exists, across sessions, and is
    never reused [MTP B.2.15].
11. **Writing a file [MTP 5.3.4, D.2.12-13, E.2.4].** SendObjectInfo (or
    SendObjectPropList, which carries a 64-bit size) first: the device
    checks store, write access, free space, parent, name, *in that order*,
    reserves a handle and answers; SendObject with the bytes must come
    next. A failed SendObject keeps the description so the PC can retry.
    More bytes than announced is refused (Store_Full). Only one pending
    description per session.
12. **Deleting [MTP D.2.11]:** a protected object gets
    Object_WriteProtected; deleting a folder deletes what's inside, but
    keeps any folder holding an undeletable object (Partial_Deletion).
    The handle 0xFFFFFFFF means "everything on the device".
13. **Events [MTP 4.8, G]:** must be sent at once, during transfers too
    (the interrupt endpoint): ObjectAdded, ObjectRemoved (only for changes
    the PC didn't make), ObjectInfoChanged, StoreAdded/StoreRemoved
    (cards in and out), StorageInfoChanged, DevicePropChanged.
14. **Big files [MTP H]:** a data phase over 4 GB has length 0xFFFFFFFF;
    GetPartialObject takes 32-bit offsets [MTP D.2.27].
15. **Device identity [MTP 5.1.1].** The serial number is required and
    exactly 32 hex digits. The specification says the vendor extension ID
    is 0xFFFFFFFF [MTP 5.1.1.2], but Windows' MTP extension, libmtp and
    real devices use **6 (Microsoft), version 100, "microsoft.com: 1.0; "**
    [QEMU]; a device saying 0xFFFFFFFF is taken for a plain camera.
16. **How Windows binds it [INF].** Windows 11's driver binds either the
    standard class (0x06/0x01/0x01, no special descriptors) or any
    interface whose Microsoft compatible ID is "MTP" [MSOS]. *Verified on
    the owner's PC, not from memory.*
17. **Spec errors found** (we follow the summary tables): B.2.1's table
    gives a 4-byte property code; B.2.171 gives Media GUID the code
    0xDD71 (the summary says 0xDD72); appendix C's tables show every
    current value as 4 bytes; E.2.3's count field is 2 bytes in one
    column and 32-bit in the other; B.2.3 and 5.3.1.3 disagree on whether
    protection is settable as a property (we never let the PC set it).

## Decision (proposed)

> The way of the original, the way today's tools allow, and why ours wins
> (CLAUDE.md rule 0) is the table under "Better than Android" below; each
> claim there has the measurement that must prove it.

### Rules

1. **One responder, every store.** `drivers/usb-mtp` (`rp-usb-mtp`): the
   SIC transport and the MTP responder, hardware-independent and
   host-tested, over two traits like `rp-usb-msc`: the bulk-and-interrupt
   pipe, and a **`Stores`** trait (list, read, create, write, commit,
   delete, rename, move, copy, properties) that `rp-fat` implements for
   FAT12/16/32 and exFAT. SD is store `0x0001_0001`, CF `0x0002_0001`,
   the internal flash (R21) `0x0003_0001`; an empty slot is
   `0x000N_0000` [MTP 5.2.1, D.2.4].
2. **The loader serves MTP now** (R53 step 4), because the loader is
   where the cards are served until the OS proper exists; the same crate
   then moves into the storage service (phase 6) unchanged. This replaces
   the earlier "the loader keeps mass storage only".
3. **The USB layout (loader):** the composite device of ADR-0014 with the
   MTP interface in place of the drive interface: CDC serial (interfaces
   0-1, the development link) + MTP (interface 2: bulk OUT 0x04, bulk IN
   0x85, interrupt IN 0x86). The interface class is Q1's vote. The new
   layout gets **`bcdDevice` 26.02**: Windows records per device revision
   whether a device has Microsoft OS descriptors and never asks again
   [MSOS], and it has already recorded 26.00 and 26.01 as having none.
   The drive layout (today's 26.01) stays byte for byte as the fallback.
4. **Which computer gets what (Q1).** Default: whoever opens an MTP
   session within a few seconds gets MTP (Windows does so at once; Linux
   desktops through libmtp); a computer that opens none gets today's
   drives, re-offered automatically. A device setting "Connect to a
   computer as: Automatic / Portable device / Drive" overrides it in the
   OS proper (the loader: `cargo xtask drive mtp|msc|auto`). The wait is
   measured on Windows 11 and a Mac before it is fixed (not guessed).
   **Iterated 2026-10-25 (rule 0, the owner asked whether the defaults
   follow it):** a fixed wait is the slow, copied way. The device also
   offers Microsoft OS 2.0 descriptors (a BOS descriptor, `bcdUSB`
   0x0201), whose descriptor-set request only Windows sends; if the
   first run shows Windows sends it on every connection (not only the
   first), the device knows at enumeration that the PC is Windows and a
   Mac's drive needs no wait at all. The session wait then stays only as
   the safety net for hosts that send neither signal.
5. **Boot files (Q2).** Default: listed, protection 0x0001 (read-only),
   Hidden 1, System 1. Delete, overwrite, rename or move of one is
   answered **Object_WriteProtected (0x200D) at once**, before any data
   moves; Windows already knows from the protection status, so it warns
   before trying. The guard (`rp_fat::Guard`) stays the lock underneath:
   no path in the responder can write a guarded entry, chain or cluster.
6. **Every write is all or nothing.** SendObjectInfo / SendObjectPropList
   reserves the whole file **contiguously** at once (`rp-fat`'s `reserve`,
   generalised), under a hidden temporary name, so Store_Full or
   Object_Too_Large (a FAT32 file over 4 GB, a FAT16 file over 2 GB) is
   answered before a byte moves; SendObject fills it by DMA; only a
   complete transfer is committed (renamed into place, attributes and
   dates set, one directory write). A cancel, an unplug or a reset leaves
   no file and no lost space; the reservation is reclaimed at the next
   mount if the device lost power mid-copy (the disk check knows the
   temporary name).
7. **Object handles:** a compact table, assigned lazily as the PC lists
   folders (never a whole-card walk unless the PC asks for one), 16 bytes
   a handle (store, the entry's directory cluster and slot, its first
   cluster), never reused within the session [MTP 3.4.1], kept across
   moves [MTP D.2.25]. **Its size comes from a survey of the owner's
   cards** (`rp_fat::survey`, file and folder counts) with the margin
   set from those numbers and written down with them (no guessed
   multiplier), in static memory, never on the loader's stack (at 73% of
   its line). A full table answers Device_Busy and logs it, never a crash.
8. **Persistent unique IDs** (128 bits, the same in every session):
   volume serial, the object's first cluster, its creation time to 10 ms,
   and the store number. Stable across renames and same-card moves (the
   data doesn't move); empty files (no cluster) use their directory
   position instead. Objects RetroPane creates get creation times that
   never collide (bumped by 10 ms when needed), so an ID is never reused.
9. **Names:** UTF-16 both ways, checked against the card's own rules
   (FAT long names, exFAT's up-case table); the file-name property carries
   a regular expression of the legal characters [MTP B.2.7] so Windows
   refuses a bad name before sending. A name MTP can't carry (exFAT's
   255-character names, one over MTP's limit) is listed read-only under a
   shortened name and logged, never silently renamed on the card.
10. **Events:** card in or out = StoreAdded / StoreRemoved; a file the
    device itself writes, deletes or renames while the PC browses (the
    loader's `put`, a run log, later any app) = ObjectAdded /
    ObjectRemoved / ObjectInfoChanged; free space changed by the device =
    StorageInfoChanged; at most one UnreportedStatus after a suspend
    [SIC 4.5]. Changes the PC made raise none [MTP G.2.4].
11. **Operations offered** (in DeviceInfo [MTP 5.1.1.6]): GetDeviceInfo,
    OpenSession, CloseSession, GetStorageIDs, GetStorageInfo,
    GetNumObjects, GetObjectHandles, GetObjectInfo, GetObject,
    GetPartialObject, DeleteObject, SendObjectInfo, SendObject,
    MoveObject, CopyObject, GetObjectPropsSupported, GetObjectPropDesc,
    GetObjectPropValue, SetObjectPropValue (name only), GetObjectPropList
    (every depth, streamed: one pass counts, a second sends, so a whole
    folder tree costs one round trip and no RAM beyond the handles;
    Android stops at depth 1, which would have been a copy),
    SetObjectPropList, SendObjectPropList,
    GetDevicePropDesc, GetDevicePropValue, SetDevicePropValue (the
    initiator's version string; the friendly name, Q4).
    **Not offered:** PowerDown, ResetDevice's power side, SelfTest,
    InitiateCapture/OpenCapture, SetObjectProtection (the device decides
    protection), object references, Skip, GetThumb (no thumbnails until a
    picture decoder is in the storage service: noted, not dropped).
    **FormatStore (Q3 B)** only for a card that holds no RetroPane boot
    files, and only after the user confirms on the device: the screen
    names the card and its size, the Action key or an on-screen OK
    confirms (as Windows Mobile confirms), anything else or 30 s refuses
    (Access_Denied). The format is ADR-0036's (erase-block aligned, the
    SD Association's layout, not Windows'), its file-system parameter
    honoured when the card can take it; StoreRemoved then StoreAdded
    follow. The card RetroPane boots from is refused at once
    (Store_Read-Only). DeleteObject of handle 0xFFFFFFFF ("everything") is
    refused with Invalid_ObjectHandle: no single operation from a cable
    may empty every card.
12. **Device identity:** manufacturer "Dell", model "Axim X51v" (from the
    board, so an X50v port says X50v), device version the RetroPane
    version, serial number the 32-hex-digit form of the device's own ID
    (the same unit always gets the same Explorer entry); vendor extension
    6 / 100 / "microsoft.com: 1.0; " (fact 15); perceived device type 5,
    "personal digital assistant" [MTP C.2.43]; device friendly name and
    icon (Q4, Q5).
13. **Speed:** the data path is `rp-usb-msc`'s measured one (two 16 KiB
    halves, card DMA overlapping the USB): MTP must move files at least as
    fast as the drive does on the same card, measured in the ratchet
    (`cargo xtask perf`: MTP read and write MB/s beside the drive's), and
    a folder of 1,000 files must list in one round trip
    (GetObjectPropList depth 1) if Windows asks that way (measured: every
    operation Windows sends is logged on the device's card log during the
    first run).

### Security (docs/security.md checklist)

- **Hostile until checked.** Every container: length at least 12 and at
  most the phase's limit, type and code known, transaction ID matching,
  parameter count = (length - 12) / 4 at most 5 [SIC 7.1.2]; a command
  shorter than its own length stalls the pipe [SIC 7]. Every handle and
  store ID is looked up, never used as an index or address; every
  offset + length overflow-checked against the object; every dataset
  (ObjectInfo, property lists) parsed with bounds on every field, strings
  at most 255 characters, no embedded nulls [MTP 3.2.3]; a data phase that
  errors is still drained to its end before the error response
  [MTP 4.6]. DMA lengths come only from checked containers and never
  exceed a staging half (ADR-0014's audit, 2026-10-16).
- **Nothing the cable can do destroys the system:** no power-down, no
  format of a boot card and no format at all without the user's yes on
  the device, no "delete everything", no protection change, boot files
  read-only by construction (rule 5), the guard underneath.
- **Exposure:** in the loader (development), offered at once like the
  drive is today (docs/security.md, current exposures). In the OS proper:
  nothing is listed until the user allows it on the device, per
  connection (MTP has no authentication of its own [MTP 2.5]); until then
  GetStorageIDs returns no stores and the device shows "Allow access to
  your files?".
- **Fuzzing:** a 5,000-command random-input host test (as `rp-usb-msc`
  has), truncated and oversized containers, wrapping offsets, wrong
  transaction IDs, a SendObject longer than announced, cancels in every
  phase, a reset mid-transfer.

### Better than Android (rule 0: the original, the modern way, why ours wins)

| What | Android / Windows Mobile / the textbook | Ours | Proven by |
|---|---|---|---|
| Refusals | Android answers after the PC tries; WM had no MTP (ActiveSync) | Protection status up front, Windows warns before it tries; refusal at once | X51v run: delete `haret.exe` from Explorer |
| Interrupted copies | Half-written files left behind (common report) | Reserve, fill, commit: a cut-off copy leaves nothing | Emulator: cancel and unplug in every phase, card byte-identical; X51v: unplug mid-copy |
| Out of space | Found mid-copy | Answered before the first byte (whole-file reservation) | Host test; emulator with a nearly full image |
| Fragmentation | Files grow in pieces | Every file written contiguous (one extent, fastest to read and write) | `rp_fat::check` after a 200-file copy |
| Speed | MTP slower than mass storage (per-object round trips) | Same DMA path as our drive; folder listing in one round trip | Ratchet: MTP vs drive MB/s; list time for 1,000 files |
| Moves on the card | Often copy + delete through the PC | MoveObject = one directory change (instant, no data copied); CopyObject on the device, no USB traffic | Emulator timing; X51v: move a 20 MB file |
| Stale Explorer | Common (missed events) | Every device-side change and card swap pushed as an event | Emulator: `put` while browsing, card out and in |
| Thumbnails | Android shows picture thumbnails in Explorer | **Behind Android for now** (GetThumb not offered until a picture decoder is in the storage service, R54): an open gap, not a choice to keep | R54 |
| Macs | Need a separate app | Get the drive automatically (Q1) | Mac test (owner's or borrowed), else the emulator host without MTP |
| Names | Device names may be illegal on the card | Legal-name pattern published; checked on the device too | Host tests (every FAT/exFAT forbidden character, 255-character names) |

## Votes for the owner

**Voted 2026-10-25: every default (Q1 A, Q2 A, Q3 A, Q4 "Axim X51v", Q5
the PDA icon); then, the same day, the owner: "pick the ones that follow
my rules". Picked against the rules (iterate, not imitate; legacy not
locked out; security first; clever with no trade-off; keys behave like
Windows Mobile): Q1 A, Q2 A, Q4 and Q5 kept; Q3 changed to B.** Q3 A
refused something useful to stay simple (phones' way), while B gives the
PC a format that is better than Windows' own (ADR-0036's aligned
layout) with the user's consent on the device and the boot card never
touched: more, with nothing given up. The alternatives are kept below as
the record.

- **Q1. How a computer gets MTP or a drive.**
  - **A (default): MTP first, the drive if no MTP session comes.** The MTP
    interface uses the vendor class with the Microsoft compatible ID
    "MTP", which Windows binds [INF, MSOS] and libmtp recognises, but
    which a Mac ignores; no session within the measured wait means the
    device re-offers itself as today's drive. *Pros:* Windows and Linux
    get MTP, Macs get drives, nothing to set. *Cons:* a Mac waits a few
    seconds for the drives; relies on Microsoft's descriptor (cached per
    device revision, hence rule 3's 26.02).
  - **B: the standard class (0x06/0x01/0x01).** No Microsoft descriptor
    needed at all (Windows binds it, fact 16). *Cons:* a Mac binds it as a
    camera (Image Capture opens, photos only) and never falls back to the
    drive: Macs would need the device setting.
  - **C: the setting only, no automatic choice.** Simplest, but every Mac
    user has to find the setting.
  - *Rejected:* both at once (Windows would show a drive and a device, the
    drive with today's problem).
- **Q2. The boot files over MTP.**
  - **A (default): shown read-only, hidden and system** (rule 5). The user
    can still copy HaRET off as a backup; Windows warns before any change.
    Whether Explorer hides MTP objects marked hidden is measured on the
    first run.
  - **B: not listed at all** (the specification's own way to keep an
    object from the PC, fact 9). Cleanest, nothing to touch; but nothing to
    back up from Explorer either (the loader's tools still can).
- **Q3. Formatting a card from the PC (FormatStore).**
  - **A (default): never from the PC;** formatting stays the device's
    (ADR-0036's formatter), as the drive already refuses it.
  - **B: allowed for a card RetroPane doesn't boot from, after the user
    confirms on the device** (OS proper).
- **Q4. The name Windows shows** (device friendly name): **"Axim X51v"
  (default)**, "RetroPane", or "RetroPane on Axim X51v"; and whether
  renaming it in Explorer changes it on the device (default yes, kept on
  the device).
- **Q5. The device's icon in Explorer:** **R46's Icons-Land "PDA" icon
  (default),** served as MTP's device icon [MTP C.2.37] (its licence
  allows use in the product with credit; the About screen credits it), or
  Windows' generic portable-device icon.

## Open points to measure (not votes)

- Windows 11: which operations it sends and in what order (logged), the
  depth it asks of GetObjectPropList, whether it re-reads Microsoft's
  descriptor on every connection, whether Explorer hides objects marked
  hidden, whether it uses MoveObject/CopyObject or copies through the PC.
- The wait before falling back to the drive (rule 4), on Windows 11
  (must never fall back) and a Mac.
- The handle table's size from the card survey (rule 7).
- MTP speed against the drive's (rule 13).

## The plan (R53 steps 3-5; nothing runs on the Axim without the owner's go)

1. `rp-fat`: handle-addressed listing, streaming reserve-fill-commit,
   rename, move, copy, the temporary-name recovery in the disk check;
   host tests with planted bugs.
2. `rp-usb-mtp`: transport, responder, the property and event tables;
   host tests (each rule above, the fuzz run), mutants.
3. `pxa-udc`: the new layout, Microsoft OS descriptors (Q1 A), the
   interrupt endpoint, the fallback switch.
4. The QEMU fork: the UDC model's host learns MTP (bulk and interrupt
   pipes, the class requests) so `cargo xtask emu-mtp` lists, reads,
   writes, cancels, refuses a boot-file delete and sees a card swap
   (the emulator tracks the device).
5. The loader serves SD and CF over MTP; `cargo xtask ci` green.
6. The X51v with Windows 11 (asked first, steps given upfront): the open
   points above, the speed against the drive, an unplug mid-copy.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Every borrowed idea measured before it ships: the fallback wait, Windows' operation pattern, MTP's speed against our drive |
| ✨ Modernize | How every phone since 2011 talks to a PC, without phones' flaws: all-or-nothing copies, instant refusals, live events |
| 🗄️ Legacy support | Drives stay for every computer without MTP, automatically; the cards stay plain FAT/exFAT for Windows Mobile |
| ⚡ Better / faster | Same DMA path as the drive, contiguous files, moves without copying, folder listing in one round trip |

## Alternatives considered

- **A separate boot partition (Kindle-style):** rejected while Windows
  Mobile boots RetroPane (first partition only).
- **The blink (ADR-0014 rule 3's withdrawn amendment):** honest but looked
  broken; withdrawn by the owner.
- **MTP only (Android):** drops Macs and old tools: not an option (the
  owner: dropping support isn't an option).
- **Telling hosts apart by how they enumerate** (descriptor request order,
  sizes): fragile across Windows updates; a behaviour (opening a session)
  is the honest signal.

## Consequences

- The loader grows (the responder, the handle table in static memory):
  a test image, ratcheted with a logged reason; its stack stays under
  emu-cf's 75% line.
- Windows sees a new device revision once (26.02): the COM port number may
  change once; `cargo xtask` finds the port by VID/PID.
- `docs/adr-areas.md` gains `drivers/usb-mtp/` (0037, 0014) when the crate
  is created; `kernel/fat/` and `drivers/pxa-udc/` add 0037.
- Optional later, not dropped: thumbnails (GetThumb / representative
  samples) once a picture decoder lives in the storage service; contacts
  and calendar over MTP (vCard, iCalendar formats) for the rebuilt WM5
  apps; "send to PC" from the device (RequestObjectTransfer [MTP G.2.10]).
