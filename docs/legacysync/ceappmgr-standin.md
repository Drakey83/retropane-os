# LegacySync's `CeAppMgr.exe` stand-in

ADR-0028 L2, milestone 3. Status: **built as a command-line program,
not registered.** Registration changes the owner's registry, so it waits
for the owner's choice below.

## In plain words

An old installer looks in two registry places to find ActiveSync's
Application Manager, then runs it with the path of an INI file
([activesync-interfaces.md](activesync-interfaces.md) §1-2). LegacySync
ships its own `ceappmgr.exe` that does Application Manager's job for
RetroPane. For installers to find it, those two registry places must
point at it.

## What the program does today

`pc/legacysync` builds `ceappmgr.exe` (crate `rp-legacysync`).

```
ceappmgr.exe [/report] "C:\...\Game.ini" [...]   what installers call
ceappmgr.exe --out <folder> "Game.ini"           stage into <folder>
ceappmgr.exe --print-registration                show the .reg (writes nothing)
```

For each INI, for each component in it:

1. Reads the INI (ANSI, UTF-8 or UTF-16; 64 KiB cap). CAB names must be
   plain names beside the INI: no `..`, drive or root.
2. **Checks every CAB whole** before choosing: every data block's checksum,
   the setup script parsed end to end (`rp-ce-setup`), and every file the
   script places present in the CAB. A CAB that fails is reported and
   skipped, never sent.
3. **Chooses** like Application Manager: by the processor and OS range in
   each script (ARM over older ARM over "any"; in-range versions first;
   MIPS/SH/x86 never), not by the file name.
4. **Stages** the chosen CAB and a readable `plan.txt` (what the device will
   do, with `%CEn%` folders shown as Pocket PC paths) in
   `%LOCALAPPDATA%\RetroPane\LegacySync\Outbox\<Provider App>\`.
5. Logs the call and its outcome to
   `%LOCALAPPDATA%\RetroPane\LegacySync\legacysync.log` (ADR-0028: every
   installer tried is logged with what it called).

On the owner's six real installers with INIs it stages all six; GeoRally and
Stuntcar Extreme get a note that their scripts say CE 4.x only (the X51v's
Windows Mobile 5 is CE 5.1; both were sold for WM5 Axims, so it is a note,
not a refusal).

Sending to the device waits for the LegacySync session on the USB link
([device-link.md](device-link.md)).

## Registration: what installers need

From `ceappmgr.exe --print-registration` (both registry views, because
the installers are 32-bit and read `WOW6432Node`):

| Key (under `HKLM\SOFTWARE` and `HKLM\SOFTWARE\WOW6432Node`) | Value |
|---|---|
| `Microsoft\Windows CE Services` | `InstalledDir` = LegacySync's folder; `MajorVersion` 6, `MinorVersion` 1, `BuildNumber` 6965 |
| `Microsoft\Windows\CurrentVersion\App Paths\CEAPPMGR.EXE` | default = full path of LegacySync's `ceappmgr.exe` |

Writing `HKLM` needs administrator rights once, at install time
(LegacySync's installer, not the stand-in itself).

## The decision: coexisting with WMDC (for the owner's vote)

The owner's PC has Microsoft's WMDC 6.1 installed today, and its
`CeAppMgr.exe` owns both keys (`C:\Windows\WindowsMobile`). Five Dell
installers already went through it this morning.

| | **A: LegacySync takes the keys, with a backup (recommended)** | B: LegacySync only where WMDC is absent | C: never register; drag the INI onto LegacySync |
|---|---|---|---|
| How | LegacySync's installer saves the current values to a `.reg` backup in its folder, then points both keys at itself; uninstalling restores the backup | Register only if no `CeAppMgr.exe` is registered; otherwise leave WMDC in charge | No registry changes; the user opens LegacySync and picks the INI the installer left behind |
| Old installers | Work unchanged, and reach RetroPane | On this PC they keep reaching WMDC, which can't talk to RetroPane | Run, then fail or reach WMDC; the user finishes by hand |
| WMDC | Its Application Manager is bypassed (the rest of WMDC still works); restored by uninstalling LegacySync | Untouched | Untouched |
| Risk | Two programs wanting the same keys; WMDC repair or reinstall would take them back (LegacySync checks on start and offers to re-take them) | On most Windows 11 PCs (no WMDC) the same as A; on the owner's PC no benefit | None, but the point of L2 is lost |
| Effort | Small: the backup/restore and the start-up check | Small | None |

**Recommended: A**, because on Windows 11 WMDC can't reach RetroPane
anyway, and the backup makes it fully reversible. A refinement for later:
LegacySync could hand an install to WMDC when the docked device is a real
Windows Mobile device rather than RetroPane, so one PC can serve both.

Not done without the owner's OK: writing these keys on the owner's PC,
including for a test. Until then the stand-in is tested by running it
directly (`--out`), which is how its tests and the six real installers
were checked.

## Open items

- **Console window**: run by an installer, the command-line program shows
  a console window for a moment. The first desktop version builds it as a
  Windows (GUI) program with a small "Ready for RetroPane: Toy Golf"
  window instead.
- **Waiting installers**: if some installer waits for `CeAppMgr.exe` and
  then deletes its INI and CABs, staging first (copying the CAB) already
  covers it. Not seen yet; every call is logged so it will show.
- **AppMgr records** (`AppMgr\Apps\<Provider App>`,
  activesync-interfaces.md §4): written at the same time as the keys
  (same decision), so installers that check "already installed?" behave.
- **Signing**: ADR-0028 wants the Windows program signed so SmartScreen
  doesn't block it; a release question, not a stand-in one.

## Security

- **Outside input**: the INI and the CABs come from old installers and
  the internet. The INI is size-capped and its CAB names can't leave its
  folder; CABs and scripts go through the hostile-input-tested parsers
  (`rp-cab`, `rp-ce-setup`: truncations, 20,000+ mutations each, mutants
  for the checksum, window and path rules); a CAB is checked whole before
  it is staged; nothing in a CAB is executed or unpacked on the PC.
- **What it exposes**: nothing. It listens on nothing and sends nothing;
  the device link and its pairing are designed separately.
- **Privilege**: runs as the user. Only the registration (once, by
  LegacySync's installer, after the owner's vote) needs administrator
  rights.
- **On the device** (Phase 4, noted here so it isn't lost): the setup DLL
  inside some CABs (`*.999`) is ARM code that runs at install time; the CE
  personality runs it in the app's sandbox, and the device asks the user
  before every install (ADR-0028 L2).
