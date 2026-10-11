# Clean-Room and Reverse-Engineering Policy

This project reimplements software and drivers whose originals belong to
Microsoft, Intel, Imagination Technologies, Texas Instruments and Dell. To keep
the project legally sound and safe to distribute, every contributor follows
these rules. *This is not legal advice. When in doubt, ask in an issue before
acting.*

## 1. Never in the repository

- Source or binaries from Microsoft, Intel, Imagination, TI, Dell or HTC.
- ROM dumps, extracted DLLs, firmware blobs or disassembly listings of them.
- Leaked SDKs or driver source of any kind (e.g. "leaked" PowerVR DDKs).
- Copyrighted artwork, fonts or sounds from Windows Mobile.

## 2. Off-limits to contributors entirely

- Microsoft Windows CE **Shared Source** / Platform Builder source.
- The Microsoft **Device Emulator** shared-source release.
- Any leaked PowerVR driver source.

If you have read these, please don't work on the parts they cover
(kernel/CE APIs, or graphics, respectively).

## 3. Allowed references

- Public documentation: MSDN/Learn pages, published SDK headers and import
  libraries, books such as Boling's *Programming Windows CE*, datasheets
  (TSC2046, WM8750, PXA27x Developer's Manual).
- Open-source projects, as **references for facts** (register addresses, GPIO
  numbers, bit meanings): Linux `mbxfb`, AxDroid, acx-mac80211, KallistiOS
  (Dreamcast PowerVR2), CeGCC/mingw32ce headers. Don't copy code from projects
  with incompatible licenses. GPL-2.0-only code must not be copied into this
  GPL-2.0-or-later project.

## 4. How reverse engineering works here

Reverse engineering is done **only for interoperability**: to make our OS work
with the X51v's hardware and with existing WM5 and WM6.x applications.

```
 [Your own device]  →  romdump / mmiotrace  →  local traces & disassembly (never committed)
                                                     │
                                                     ▼
                                     Specification in docs/hw/  (facts only)
                                                     │
                                                     ▼
                                     Implementation from the spec + tests
```

1. **Observation.** Traces and analysis are done on binaries from the
   contributor's own device, kept locally.
2. **Specification.** Findings are written up as plain specifications: register
   maps, bit fields, command formats, sequences, timing. Each fact has a
   confidence level (`confirmed` / `likely` / `guess`) and cites the experiment
   that shows it. **No code, pseudo-code transcriptions or disassembly in specs.**
3. **Implementation.** Driver code is written from the spec. A recorded trace
   may be used as **test data** to check that our driver produces the same
   hardware behaviour, but never as a template to transcribe.

Where possible, the person (or AI session) writing a spec and the one
implementing it should be different. When the same person does both, the spec
must be merged first and reviewed on its own.

## 5. Firmware and user-supplied files

Some hardware needs proprietary firmware (the ACX100 Wi-Fi chip). The OS loads
it from a file that `tools/romdump` extracts from the **user's own device**. We
never distribute it. The same applies to WM5/6.x test applications in the
compatibility corpus: the repo stores only metadata (name, version, hash).

## 6. Names and trademarks

The project never calls itself "Windows", "Windows Mobile" or "Pocket PC" in
its name or branding. Describing compatibility ("runs Windows Mobile 5 and 6
applications") is fine.
