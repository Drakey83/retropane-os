# ADR-0028: LegacySync, the Windows companion for Pocket PC software

- **Status:** Accepted (2026-10-15, by the project owner: L1-L5 as
  recommended). Work starts in its own session.
- **Date:** 2026-10-15

## In plain words

Many Pocket PC programs, games especially, were never just a file you copy
to the device. They came as a Windows `setup.exe`: you ran it on the PC,
and **ActiveSync** carried the program over the cradle and installed it on
the PDA. ActiveSync also did much more: it synced Outlook contacts,
calendar, tasks and email, let you browse the device's files from the PC,
backed it up, set its clock, shared the PC's internet connection, and gave
other PC programs a remote API (RAPI) to reach the device.

Microsoft dropped ActiveSync (Windows XP) and its successor Windows Mobile
Device Center (Vista/7); on Windows 10 and 11 WMDC is unsupported and
often won't install or connect. So today those `setup.exe` files often
can't run at all.

**LegacySync** is a Windows program we write that does ActiveSync's jobs
for RetroPane: old installers think ActiveSync is there, and everything
ActiveSync did works again, over the same Dell cable and cradle, on
Windows 11.

## What ActiveSync did (the feature list to cover)

| Feature | What it did | Who uses it |
|---|---|---|
| Add/Remove Programs | `setup.exe` hands an INI file to ActiveSync's Application Manager (`CeAppMgr.exe`), which picks the CAB for the device's CPU, copies it over and runs the device's installer | almost every commercial program and game |
| RAPI | a Windows DLL (`rapi.dll`) that PC programs call to read and write device files, the registry, start programs | installers, backup tools, remote-control tools, utilities |
| Explore | the device's files in Windows Explorer | users |
| Sync | Outlook contacts, calendar, tasks, notes, Inbox email, Pocket IE favorites, a synced Files folder | users |
| Backup and restore | the whole device to a file on the PC (ActiveSync 3.x) | users |
| Time sync | the device clock set from the PC on connect | everything time-based |
| Pass-through | the device on the internet through the PC | browsing, email over the cradle |
| File conversion | Word/Excel files converted to Pocket Word/Excel on copy (ActiveSync 3.x) | Pocket Office users |
| Partnerships | which PC the device trusts and syncs with | all of the above |

## Decisions (for the owner's vote)

### L1. What talks to the device

| | A: the real ActiveSync protocol on the device | **B: LegacySync over our own link (recommended)** |
|---|---|---|
| How | RetroPane acts like Windows Mobile: the USB networking function ActiveSync expects, then Microsoft's sync and RAPI services over it; Microsoft's ActiveSync/WMDC on the PC | LegacySync on the PC speaks a RetroPane protocol over the USB link we already have (ADR-0012, ADR-0014); it presents ActiveSync's interfaces to old PC software |
| Windows 10/11 | needs WMDC, unsupported and often broken | a normal Windows program, no special drivers |
| Work | the protocol must be reverse-engineered (only parts are documented), then the whole Windows Mobile service side implemented | our protocol is ours; the work is in the compatibility shims on the PC side |
| Security | Microsoft's 2005 protocol, its trust model and bugs | ours (docs/security.md): pairing, permissions, nothing listening by default |
| Mac/Linux later | no | possible (the device side is the same) |

Why B: the point is old **PC-side** software (installers, RAPI tools)
working on today's Windows. That software only needs ActiveSync's
**PC-side interfaces** (the Application Manager, `rapi.dll`, its registry
keys), not Microsoft's wire protocol. LegacySync provides those
interfaces and is free to talk to RetroPane however works best.

### L2. Installing programs (the first feature)

**Recommended.** LegacySync registers itself as `CeAppMgr.exe` (the
registry keys installers look for, so they believe ActiveSync is there),
reads the installer's INI, picks the ARM CAB (`ARMV4`, `ARMV4I`, `ARM`
names), and sends it to RetroPane. On the device, the CE personality
installs it as Windows Mobile's `wceload` did: unpack the CAB (Microsoft's
cabinet format), follow its setup script (`_setup.xml`, or the older `000`
file), place files, write registry keys, make shortcuts. Setup DLLs inside
some CABs (native ARM code run at install time) run inside the CE
personality's sandbox like any other program. The device asks the user
before every install: "Install Toy Golf (from LegacySync)?"

> **Correction (note added 2026-10-15):** the INI doesn't say which CAB is
> for which CPU, and names like `ARMV4` are only conventions. Application
> Manager reads the processor type and OS range from each CAB's setup
> script and picks by those; LegacySync does the same
> (docs/legacysync/activesync-interfaces.md §3). Also, all seven real CABs
> on the Dell discs use the `.000` script; none uses `_setup.xml`.

### L3. RAPI

**Recommended, second.** A `rapi.dll` of our own, with Microsoft's
exported functions (file, directory, registry, process calls), placed
where old PC tools find it. Each call becomes a request over our link;
the device answers with the user's permissions, and the first connection
from a new PC needs the device's approval (a partnership, L5).

### L4. Sync, backup, time, pass-through, conversion

| Feature | Recommendation |
|---|---|
| Explore | yes: a LegacySync file window, and the device already appears as a USB drive for its card (ADR-0014) |
| Time sync | yes, on connect (cheap; fixes the RTC drift TOTP and TLS care about) |
| Backup and restore | yes: the device's files and settings to a file on the PC, restorable, encrypted with a key the user chooses |
| Contacts, calendar, tasks | yes, **modernized**: sync with Outlook *and* with modern services (CardDAV/CalDAV: Google, iCloud, Nextcloud), not only desktop Outlook as ActiveSync did; the device keeps the Pocket Outlook formats legacy apps read |
| Email | through the device's own mail (Phase 8) rather than Outlook's Inbox sync; LegacySync can still copy Outlook's folders for users who want that |
| Pass-through | yes, **off by default**: the device online through the PC (useful before Wi-Fi works, and on networks that refuse 802.11b), only when the user turns it on in LegacySync |
| File conversion | no: modern formats instead (the device opens .docx/.xlsx-lite or plain text); revisit if users ask |

### L5. Trust (partnerships)

**Recommended:** the first time a PC connects, the device shows a code
and asks; LegacySync stores a key; later connections are checked with it.
A PC without a partnership can do nothing beyond the USB drive (ADR-0014).
Like ActiveSync's partnerships, but with modern crypto and per-feature
permissions (install, files, sync, pass-through).

## Implementation status (2026-10-15)

- **Research**: [docs/legacysync/activesync-interfaces.md](../legacysync/activesync-interfaces.md),
  from the owner's Dell discs and PC: how installers find `CeAppMgr.exe`
  (two registry keys, read through `WOW6432Node`), its command line, the
  INI, Application Manager's records, the MSCF and `.000` layouts (seven
  real CABs), and the finding that Windows 11 still ships a `rapi.dll`
  (a layer over WMDC's RAPI2 COM server), which opens a second route for L3.
- **Shared parsers** (device and PC, `no_std`, no heap, hostile-input
  tested, mutants): `rp-cab` (`personality/ce/cab`: cabinets, uncompressed
  and MSZIP, streamed) and `rp-ce-setup` (`personality/ce/setup`: `.000`
  and `_setup.xml` to a validated install plan).
- **L2's PC half**: `pc/legacysync`'s `ceappmgr.exe` stages the right CAB
  and a readable plan for all six of the owner's real installers with
  INIs ([ceappmgr-standin.md](../legacysync/ceappmgr-standin.md)).
  Registration: option A (the owner, 2026-10-15): LegacySync's installer
  backs up the lookups' current values (WMDC's, on the owner's PC), takes
  them over, and its uninstaller restores them. The backup, take-over and
  check exist (read-only); not applied to the owner's registry yet.
- **Device link**: designed, for review
  ([device-link.md](../legacysync/device-link.md)): its own USB interface,
  a Noise handshake with a code on both screens, encrypted frames,
  partnerships on the card guarded from the drive.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Each feature ships separately (install first); every installer tried is logged with what it called |
| ✨ Modernize | Windows 11 without WMDC; CardDAV/CalDAV beside Outlook; encrypted backups; per-feature permissions |
| 🗄️ Legacy support | The old `setup.exe` installers and RAPI tools work unchanged |
| ⚡ Better / faster | Our USB link at full speed, no Microsoft protocol layers in between |

## Hardware limit or choice?

ActiveSync's protocol is a Microsoft software choice; the cable and cradle
(USB 1.1, ~1 MB/s) are the only fixed part, and LegacySync uses them as
they are.

## Alternatives considered

- **A in L1 (the real protocol):** rejected above: Windows 10/11 support
  depends on unsupported Microsoft software, and the RE is large.
- **No PC side at all, CABs only:** users would extract CABs from
  installers by hand (tools exist) and copy them to the card; the device
  installs CABs anyway (L2's device half). Kept as the fallback and the
  first milestone, but not the answer for most users.
- **Support only games:** the RAPI and sync features matter to the
  productivity side of the Pocket PC library too.

## Dependencies and order

1. Device: the CE personality runs programs (Phase 4) and installs CABs
   (L2's device half; testable from the card first, no PC needed).
2. LegacySync's first version: the `CeAppMgr.exe` stand-in and the CAB
   transfer over the USB link.
3. RAPI (L3), then time, Explore, backup; sync and pass-through after the
   device's own PIM apps and networking (Phases 5, 6).

The Windows program is new ground for the project: a desktop app (Rust,
built in CI), signed so Windows SmartScreen doesn't block it.

## Consequences

- RetroPane gets a PC companion, with its own release and signing.
- The CE personality gains a CAB installer and the registry and shortcut
  layout installers expect.
- The USB link (ADR-0012) gains an authenticated session for LegacySync
  beside the developer loader, which never ships to users.
