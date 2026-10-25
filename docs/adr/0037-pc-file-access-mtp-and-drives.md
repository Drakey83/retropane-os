# ADR-0037: Files to the PC in the OS proper: MTP for Windows, drives kept for every other computer

- **Status:** Proposed (2026-10-25). The owner chose the direction ("do
  your proposed fix", after the research below); the design here is for
  his vote before any building. Its first layer for the loader is built
  (ADR-0014 rule 3: the PC's view).
- **Date:** 2026-10-25
- Reminders: R41 (Windows shows a refused delete as done), R21 (the
  internal flash as a drive), ADR-0014 (USB mass storage), ADR-0036 (card
  formats).

## In plain words

Today the X51v shows its SD and CF cards to the PC as ordinary drives (USB
mass storage). The PC then writes the card's sectors itself, so when the
device has to refuse something (deleting a file RetroPane boots from), the
refusal reaches Windows only after Explorer has already shown the file
gone. On the X51v on 2026-10-25 that looked broken: a delete that seemed to
work, a force-closed Explorer when we made Windows re-read, a "scan and
fix" prompt. The owner: no blink, nothing that looks broken, and no
support dropped.

Phones solved this a decade ago by not handing the PC the disk at all:
with MTP the PC *asks* the device for each file and each change, and the
device answers yes or no at once. RetroPane's OS does the same for
Windows, does it better than phones did, and keeps ordinary drives for
every computer that needs them.

## What others do (researched 2026-10-25)

- **Android (since Honeycomb/ICS) and newer Garmins: MTP.** Mass storage
  is block-level, so the PC owns the storage while it's mounted and the
  phone can't use it; MTP leaves the file system the device's and the PC
  requests files through it (Dan Morrill, Android, via Engadget and OMG
  Ubuntu). An operation the device won't do gets an answer, e.g. "store
  read-only" for a delete (QEMU's MTP device). *Weak spots:* Android's MTP
  only, so Macs (no built-in MTP) and old tools lost access; slow and
  flaky in practice (forum reports).
- **Kindle: the system isn't on the drive.** Over USB only the user area
  (`/mnt/us`) is exposed. For our cards this means a separate partition,
  which Windows Mobile can't show (it mounts a card's first partition
  only, XDA reports): not usable while HaRET boots RetroPane.
- **Older Garmin GPS: hidden system files** (seen only with Windows'
  "show protected operating system files"). A curtain, not a lock.
- **Raspberry Pi Pico-style boot drives:** a made-up FAT drive that
  ignores unwanted writes silently: the same problem we had.

## Decision (proposed)

1. **The loader and the card stage (built 2026-10-25, ADR-0014 rule 3):**
   the boot files are shown to the PC only as hidden + system + read-only
   (the loader changes those attribute bytes in the directory sectors it
   serves, and puts the real ones back on what the PC writes), the guard
   stays the lock. Garmin's idea, better: the card itself is never
   changed, Windows Mobile sees ordinary files, and a forced delete still
   can't remove anything.
2. **The OS proper: MTP for Windows, from the storage service.** Every
   object the PC sees is a file the device owns: a protected file is
   offered read-only (MTP's protection status), and a delete or overwrite
   of it is answered with "object write-protected" at once, so Explorer
   shows its error there and then and nothing goes stale. The device keeps
   using its storage while the PC browses (no "who owns the card").
3. **Mass storage kept for every other computer:** Macs, Linux, old
   Windows, anything with no MTP. Same guard and PC view as point 1.
4. **Which one a computer gets:** automatically, if a reliable signal
   exists. The candidate: Windows asks for Microsoft's OS string descriptor
   (index 0xEE) when it first meets a USB device, and Macs and Linux don't.
   Windows caches the answer per device, so whether it asks again on later
   connections is the open question, to measure before relying on it.
   Otherwise: a setting on the device ("Connect as: Portable device /
   Drive"), Portable device the default.
5. **Better than Android's MTP:** large transfers without per-object
   round trips (MTP's partial-object reads and writes), thumbnails and
   metadata from the device's own index, file system events pushed to the
   PC (so Explorer stays current without polling), and one code path for
   SD, CF and the internal flash (R21).

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Each borrowed idea gets a measurement before it ships (the 0xEE signal, transfer speed against mass storage) |
| ✨ Modernize | How every phone since 2011 talks to a PC, without phones' flaws |
| 🗄️ Legacy support | Mass storage stays for every computer without MTP; nothing loses access |
| ⚡ Better / faster | Refusals are instant and honest; the device and PC share storage; no Explorer re-read after a device change |

## Security

MTP input is hostile until checked, like the SCSI commands (ADR-0014):
every length, handle and offset bounded; object handles only ever name
files the storage service offers; protected objects can't be changed by
any operation. Exposure: only after the user allows it on the device, per
connection (as ADR-0014 says for the OS proper).

## Alternatives considered

- **A separate boot partition (Kindle-style):** rejected while Windows
  Mobile boots RetroPane (first partition only).
- **The blink (ADR-0014 rule 3's withdrawn amendment):** honest but looked
  broken; withdrawn by the owner.
- **MTP only (Android):** drops Macs and old tools: not an option (the
  owner: dropping support isn't an option).

## Consequences

- A storage service MTP responder (phase 6, with the OS's file layer),
  host-tested and emulator-tested (the fork's UDC model learns MTP's
  bulk pipes), then proven on the X51v with Windows 11.
- The loader keeps mass storage only (development; point 1).
