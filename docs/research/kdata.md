# The KData page: what Windows Mobile programs read directly

CE's SDK headers let programs read a fixed, user-readable kernel page
(`PUserKData`, `0xFFFFC800`) with inline macros instead of system calls.
RetroPane must provide the parts real programs read (ADR-0010 rule 8).
These facts come from scanning the owner's ROM (LennySh WM 6.1, A08 and L11
extractions in `local/`, 500 PE modules): every `0 - 0x3800` address
formation followed by a load, and every literal `0xFFFFCxxx` word in code.
Offsets only; no code is copied (CLEANROOM.md).

| Offset from `0xFFFFC800` | Meaning | Users in the ROM | Confidence |
|---|---|---|---|
| `+0x000` | Pointer to the current thread's TLS slot array | `coredll.dll` (375 sites, e.g. `TlsGetValue`) and 89 other modules (788 sites) | confirmed |
| `+0x008` | Current thread's ID/handle (`GetCurrentThreadId` macro: system-handle array at `+0x004`, index 1) | `mfcce300.dll`, `atlce300.dll`, `atlce400.dll`, `odysseyIMCE.dll` | confirmed (address); the array base and index follow CE's documented macro |
| `+0x00C` | Current process ID/handle (index 2 of the same array, `GetCurrentProcessId` macro) | none found in this ROM | likely |
| `+0x304`, `+0x388` | Kernel-information area (`+0x300`) entries | one module each (`odysseyIMCE.dll`, `appinstaller.dll`) | open: meaning unknown |

## The TLS array

| Fact | Value | Evidence |
|---|---|---|
| Slots | 64 (`TlsGetValue` refuses index ≥ 64) | `coredll.dll` |
| Words below slot 0 | per-thread fields at `-0x14` (249 uses in `coredll`), `-0x10` (2), `-0x0C` (5); slot 2 (`+0x08`) used 19 times | `coredll.dll` |

RetroPane ships its own `coredll`, so the words below slot 0 are ours to
define; only what apps inline is fixed: `+0x000` (and the TLS slots it points
to) and `+0x008`/`+0x00C`. The kernel rewrites them on every thread switch
(`kernel/sched::kdata`).

## Open questions

- The meaning of the two kernel-information entries used once each.
- Whether other apps (beyond this ROM) inline more of the page; re-run the
  scan on any app collection the compatibility work gathers.
