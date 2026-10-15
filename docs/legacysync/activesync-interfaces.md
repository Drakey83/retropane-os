# ActiveSync's PC-side interfaces (what old installers rely on)

Research for LegacySync ([ADR-0028](../adr/0028-legacysync.md)), milestone 1.
Facts only (docs/CLEANROOM.md): names, registry keys, file layouts and
behaviour observed on the owner's own discs and PC. No Microsoft code was
copied; the binaries and CABs examined stay in `local/` (gitignored).

Confidence: **confirmed** (seen in the owner's files or registry, cited),
**likely** (public documentation or one sample), **guess**.

## In plain words

An old Pocket PC `setup.exe` does three things that matter to us:

1. It asks Windows where ActiveSync lives (one of two registry keys).
2. It copies an **INI file** and one or more **CAB files** to a folder on
   the PC.
3. It runs ActiveSync's **Application Manager**, `CeAppMgr.exe`, with the
   INI's path. From there ActiveSync does everything: picks the right CAB,
   copies it to the device and starts the device's installer.

So a stand-in only has to answer step 1 with "here", accept step 3's
command line, and do the rest itself. No installer talks to the device
directly.

## Sources examined

| Source | What it is | Used for |
|---|---|---|
| Dell companion disc (`H:\`, owner's) | 19 installers in `EXTRAS\DEVICE`, two of them with their CAB beside them (`Aximdiag`, `CityTime`); `ACTIVESYNC\WWE\setup.exe` (ActiveSync 4.x) | How installers find `CeAppMgr.exe`; two real CABs |
| Dell game disc (`F:\`, owner's) | Enigmo (5 languages) and Stuntcar Extreme installers | Same installers as the CABs below |
| The owner's PC (2026-10-15) | WMDC 6.1 is installed (`C:\Windows\WindowsMobile`); five Dell installers had already been run through it today, leaving their INI + CAB on disk and their entries under `AppMgr\Apps` | Five more real CABs (Toy Golf, Stuntcar Extreme, Enigmo, GeoRally, Virtual Pool Mobile); what Application Manager records |
| Windows 11 itself | `rapi.dll` in `System32` and `SysWOW64` ("ActiveSync RAPI Backward Compatibility", 6.1.6965.0) | RAPI's export list |

Seven real device CABs in total (`local/legacysync/real/`).

## 1. How installers find `CeAppMgr.exe`

Two methods are in use, both **confirmed** in installer binaries:

| Method | Key | Seen in |
|---|---|---|
| A: ActiveSync's install folder | `HKLM\SOFTWARE\Microsoft\Windows CE Services`, value `InstalledDir` (REG_SZ, a folder); the installer runs `<InstalledDir>\CeAppMgr.exe` | Handmark Monopoly, Scrabble (Wise installers: `%cedir%\CeAppMgr.exe`) |
| B: App Paths | `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\CEAPPMGR.EXE`, default value = full path of the program | Dell `aximdiag.exe`; WMDC registers it too |

Both are 32-bit programs, so on 64-bit Windows they read the
**`WOW6432Node`** view: `HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows CE
Services` and `...\WOW6432Node\...\App Paths\CEAPPMGR.EXE` (**confirmed**:
the owner's WMDC wrote both views, and the 32-bit installers found it).
A stand-in must register in both views.

Other values installers and Dell's companion app read from the same key
(**confirmed** by name in `COMPANIONAPP.exe`; meaning **likely**):
`MajorVersion`, `MinorVersion`, `BuildNumber` (ActiveSync's version; WMDC
writes 6, 0, 6965). Some installers refuse to run on "too old" versions,
so a stand-in reports a recent one (6.1).

When neither key exists, the installers say so and stop ("Could not find
the ActiveSync software", "Windows CE Services are not found").

## 2. How `CeAppMgr.exe` is invoked

**Confirmed** (Monopoly, Scrabble):

```
"<InstalledDir>\CeAppMgr.exe" "<folder>\Monopoly.ini"
```

The argument is the INI's full path in double quotes. Scrabble runs it
twice, once per INI (the game, then a dictionary). **Likely** (public
documentation; `/REPORT` is a string in WMDC's `CeAppMgr.exe`): several
quoted INI paths may be given at once, and `/report` asks for error
reports. A stand-in accepts any number of quoted paths and ignores unknown
switches with a log line.

The installer usually doesn't wait for the result; some do. The stand-in
returns at once with exit code 0 once it has queued the install.

## 3. The INI file

Plain Windows INI, **confirmed** across 7 samples (`Diagnostic.ini`,
`Toy Golf.ini`, `Stuntcar Extreme.ini`, `EnigmoInstaller.ini`,
`unified_appmgr.ini`, `VPMobileFullPPC.ini`, plus Monopoly's names):

```ini
[CEAppManager]
Version   = 1.0
Component = Toy Golf        ; the name of the next section

[Toy Golf]
Description = Fathammer Toy Golf
CabFiles    = Fathammer Toy Golf.CAB    ; comma-separated, relative to the INI
Uninstall   = {ECC51961-...}            ; name used to uninstall
IconFile    = toygolf.ico               ; optional, PC side only
IconIndex   = 0                         ; optional
DeviceFile  = toygolf.exe               ; optional, the main program
```

Observed variations a parser must accept: spaces around `=` or none;
`IconIndex = -1`; section and key names in any case; `Component` naming a
section that comes later; CAB names with spaces; no `Uninstall`. **Likely**
(public docs): `Component` may name several components separated by commas,
each its own section.

**How the CAB is chosen** (correction to the ADR's wording): the INI does
not say which CAB is for which CPU. CAB file names like `*.ARM.CAB`,
`ARMV4`, `ARMV4I`, `SA` are only conventions. Application Manager opens
each CAB in `CabFiles`, reads the processor type and OS version range from
the setup script inside (section 4), and picks the one that fits the
device. This is **confirmed** by what it records: `ListCpuTypes` holds
`2577` for Virtual Pool Mobile, whose script says processor 2577, and `0`
for the others, whose scripts say 0.

## 4. What Application Manager records on the PC

**Confirmed** (the owner's registry after five installs), under
`HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows CE Services\AppMgr\Apps\<Provider> <AppName>`
(both names come from the CAB's setup script):

| Value | Type | Example (Toy Golf) |
|---|---|---|
| `AppName`, `Provider`, `Description` | REG_SZ | `Toy Golf`, `Fathammer`, `Fathammer Toy Golf` |
| `InstallDir` | REG_SZ | the INI's folder |
| `ListCabFiles` | REG_MULTI_SZ | `Fathammer Toy Golf.CAB` |
| `ListCpuTypes` | REG_MULTI_SZ | `0` (one per CAB) |
| `Data_<cab>` | REG_BINARY | 4 DWORDs: the script's min OS major, min minor, max major, max minor (`4, 20, 5, 99`) |
| `<cab>` | REG_MULTI_SZ | the script's unsupported-platform list (`HPC`, `JUPITER`) |
| `Uninstall`, `IconFile`, `IconIndex`, `DeviceFile` | from the INI | |

`AppMgr` itself holds `SilentInstall` (DWORD). WMDC's `CeAppMgr.exe`
also names `...\CurrentVersion\Uninstall` (Windows' own Add/Remove
Programs list) and the device folder `\Windows\AppMgr\Install`, where it
places the CAB before the device installs it (**likely**). Removal is
sent to the device as a provisioning document of type `UnInstall` naming
the app (**likely**, a string in `CeAppMgr.exe`).

LegacySync writes these too, so installers that check "is it already
installed?" and Windows' uninstall list behave.

## 5. Microsoft cabinet (MSCF) as used by device CABs

The container is Microsoft's public cabinet format ([MS-CAB]); what the
real device CABs use, **confirmed** on all 7:

- One cabinet, no previous/next cabinet (flags 0), no reserved areas.
- **All folders uncompressed** (type 0): CAB Wizard compresses only when
  asked. MSZIP (type 1) is the other kind device CABs may use (public docs);
  LZX (type 3) appears in the samples only inside an InstallShield PC
  installer, never a device CAB. Quantum: never seen.
- Every data block has a checksum, and **all 1,963 blocks verify** with:
  XOR of the block's data as little-endian 32-bit words (a 1-3 byte tail
  packed first byte highest), then the same over the 4 bytes of the
  block's two length fields, seeded with the first result.
- File names inside are mangled: up to 8 characters + `.NNN`, where NNN is
  the file's number in the setup script (`CityTime.001`, `0toygolf.004`);
  `.000` is the setup script and `.999` the setup DLL, if any.
- Sizes: 1.1 MB to 22.8 MB (Toy Golf), so the device side must stream
  folders, not load the CAB into RAM.

## 6. The setup script (`.000`, "MSCE")

All 7 real CABs use the binary `.000` script; **none** uses `_setup.xml`
(the XML form newer tools can produce; **likely** from public docs, its
format is Windows Mobile's provisioning XML: `Install`, `FileOperation`,
`Registry` characteristics). Layout, **confirmed** by decoding all 7
(little-endian):

**Header** (100 bytes):

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | `MSCE` |
| 4 | 4 | 0 |
| 8 | 4 | length of the script |
| 12 | 4 | 0 |
| 16 | 4 | 1 |
| 20 | 4 | processor type (0 = any, 2577 = StrongARM/ARM) |
| 24 | 16 | min OS major, min minor, max major, max minor |
| 40 | 8 | min build, max build (`0xE0000000` / `0xC0000000` / 0 seen as max) |
| 48 | 12 | counts: strings, directories, files, registry hives, registry values, shortcuts (6 x u16) |
| 60 | 24 | offsets of those six tables (6 x u32) |
| 84 | 12 | offset + length (u16 + u16) of the app name, the provider, the unsupported-platforms list |
| 96 | 4 | 0 |

**Tables**, each a run of variable-length entries:

| Table | Entry |
|---|---|
| Strings | id u16, length u16 (bytes, with NUL), text |
| Directories | id u16, length u16 (bytes), string ids (u16 each, ending with 0): the path's pieces, e.g. `%CE1%` + `CodeCity` |
| Files | id u16, directory id u16, unknown u16, flags u32, name length u16, name (with NUL) |
| Registry hives | id u16, root u16 (1 HKCR, 2 HKCU, 3 HKLM, 4 HKU), 0 u16, length u16, string ids ending with 0: the key path |
| Registry values | id u16, hive id u16, substitute u16 (1 = expand `%..%` in the value, **likely**), type+flags u32, length u16, name (with NUL) then data |
| Shortcuts | id u16, directory id u16 (where the shortcut goes), 0 u16, target id u16, target type u16 (1 file, 0 directory), length u16, string ids (the shortcut's name) |

Value types seen: `0x00000000` string, `0x00010001` DWORD, `0x00010000`
multi-string, `0x00000001` binary (**likely**, from the INF flags they
mirror); bit 1 (`0x2`) = don't overwrite an existing value (**likely**).
File flags seen: `0x40000000`, `0x40000002`, `0x80000000`; they mirror
INF `CopyFiles` flags (`0x2` don't skip, `0x10` no overwrite,
`0x40000000` shared/reference counted, `0x80000000` self-registering
DLL; **likely**).

**Path macros** used by the scripts (**confirmed** in the samples;
meanings **likely**, public docs): `%CE1%` Program Files, `%CE2%` Windows,
`%CE4%` StartUp, `%CE11%` Start Menu\Programs, `%CE17%` Start Menu, and
`%InstallDir%` (the app's folder, which the user may move to a card). The
full table (`%CE1%`-`%CE17%`) goes into the CE personality's installer.

A script can be *empty of tables* and still valid (GeoRally: 4 strings,
2 directories, 39 files, no registry, 1 shortcut).

**Version ranges matter**: GeoRally declares OS 4.0-4.99, but the X51v's
Windows Mobile 5 is CE 5.1, so a strict installer refuses it. The
stand-in shows such mismatches and lets the user go ahead.

## 7. RAPI (`rapi.dll`)

**Big finding:** Windows 11 still ships `rapi.dll` in both `System32` and
`SysWOW64` (file description "ActiveSync RAPI Backward Compatibility",
version 6.1.6965.0, product "Windows Mobile Device Center"). It is not a
known DLL. It imports `CoCreateInstance` (ole32) and `CEUTIL.dll`'s
`CeSvcOpenW`/`CeSvcGetStringW`: it is a thin layer over WMDC's RAPI2 COM
server rather than the transport itself (**likely** from its imports).

That gives L3 two routes (a decision for later, see the device-link note):

| | Our own `rapi.dll` beside each old tool | Our COM server behind Microsoft's `rapi.dll` |
|---|---|---|
| How | Put our DLL where the tool loads it first (its own folder) | Register LegacySync as the RAPI2 COM classes (public `rapi2.h` interfaces); Windows' own `rapi.dll` calls us |
| Old tools | Each tool's folder needs our copy | Every tool works unchanged |
| Risk | DLL placement per tool | WMDC's COM registrations, if present, must be replaced or coexist |

**Exports** (151 named, ordinals 27-179; names only). Every `Ce*` name
has an older `Peg*` twin for the same call (PegasusOS was CE 1.0's name):

- Session: `CeRapiInit`, `CeRapiInitEx`, `CeRapiUninit`, `CeRapiGetError`,
  `CeRapiFreeBuffer`, `RapiFreeBuffer`, `GetRapiError`, `CeRapiInvoke`,
  `CeCheckPassword`, `CeGetLastError`.
- Files: `CeCreateFile`, `CeReadFile`, `CeWriteFile`, `CeCloseHandle`,
  `CeSetFilePointer`, `CeSetEndOfFile`, `CeGetFileSize`, `CeGetFileTime`,
  `CeSetFileTime`, `CeGetFileAttributes`, `CeSetFileAttributes`,
  `CeCopyFile`, `CeMoveFile`, `CeDeleteFile`, `CeCreateDirectory`,
  `CeRemoveDirectory`, `CeFindFirstFile`, `CeFindNextFile`, `CeFindClose`,
  `CeFindAllFiles`, `CeGetTempPath`, `CeGetSpecialFolderPath`,
  `CeGetDiskFreeSpaceEx`, `CeGetStoreInformation`.
- Registry: `CeRegOpenKeyEx`, `CeRegCreateKeyEx`, `CeRegCloseKey`,
  `CeRegDeleteKey`, `CeRegDeleteValue`, `CeRegEnumKeyEx`,
  `CeRegEnumValue`, `CeRegQueryInfoKey`, `CeRegQueryValueEx`,
  `CeRegSetValueEx`.
- Databases (CE's property databases): `CeCreateDatabase(Ex)`,
  `CeDeleteDatabase(Ex)`, `CeOpenDatabase(Ex)`, `CeSeekDatabase`,
  `CeReadRecordProps(Ex)`, `CeWriteRecordProps`, `CeDeleteRecord`,
  `CeSetDatabaseInfo(Ex)`, `CeFindFirstDatabase(Ex)`,
  `CeFindNextDatabase(Ex)`, `CeFindAllDatabases`, `CeOidGetInfo(Ex)`,
  `CeMountDBVol`, `CeUnmountDBVol`, `CeFlushDBVol`, `CeEnumDBVolumes`.
- System and windows: `CeCreateProcess`, `CeGetSystemInfo`,
  `CeGetVersionEx`, `CeGlobalMemoryStatus`, `CeGetSystemPowerStatusEx`,
  `CeGetSystemMetrics`, `CeGetDesktopDeviceCaps`, `CeQueryInstructionSet`,
  `CeGetWindow`, `CeGetWindowLong`, `CeGetWindowText`, `CeGetClassName`.
- Shell: `CeSHCreateShortcut`, `CeSHGetShortcutTarget`.
- Sync: `CeSyncStart`, `CeSyncStop`.
- COM: `DllCanUnloadNow`, `DllGetClassObject`, `DllRegisterServer`,
  `DllUnregisterServer`.

Signatures are the public `rapi.h` ones (Windows Mobile SDK headers, an
allowed reference): Win32-style calls with wide strings, e.g.
`HANDLE CeCreateFile(LPCWSTR, DWORD, DWORD, LPSECURITY_ATTRIBUTES, DWORD,
DWORD, HANDLE)`; written up per function when L3 starts.

## 8. What this means for LegacySync

- The stand-in is a 32-bit-visible registration (both registry views) plus
  a program that accepts `"<ini>"...` and does Application Manager's job:
  read the INI, open each CAB, read its script's CPU and version range,
  pick one, hand it to the device.
- **Coexistence with WMDC**: the owner's PC has WMDC 6.1 installed, and
  its `CeAppMgr.exe` currently owns both keys. Taking them over changes
  the owner's registry; that needs the owner's OK and a way back (see
  [`ceappmgr-standin.md`](ceappmgr-standin.md)).
- The device side needs: MSCF uncompressed + MSZIP (no LZX so far), the
  `.000` script, the `%CEn%` folder table, and `_setup.xml` for newer
  CABs. Both parsers live in shared crates the PC and the device use.

## Open

- `_setup.xml`: no real sample yet. A WM5-era download (not from the Dell
  discs) would confirm the subset we parse.
- The 14 other Dell-disc installers (NSIS, InstallShield, Wise and unknown
  packers) keep their CABs inside compressed installers; running them is
  how their CABs come out. Not done here (it changes the PC); the five
  already run today cover four installer kinds. One installer kind
  opens without running: the X50 Game CD's Enigmo (`EnA01En.exe`) keeps
  a zlib stream in PE resource 99/2, which inflates to a second installer
  with the MSCF cabinet inside (found by the GLES-games session,
  2026-10-15). That CAB is byte-identical to the one WMDC staged, and
  `rp-cab`'s extraction of all 68 files matches 7-Zip's.
- CeAppMgr's exit codes and how installers react to them (**guess**: most
  ignore them).
