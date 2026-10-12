# Security

RetroPane will be on the internet: Wi-Fi with WPA3, TLS 1.3, a modern
browser, messaging. A 2005 device with 2026 connectivity is exactly what
attackers look for, so security is a design input from the first line, not
a later pass (the owner's standing requirement, 2026-10-12). Every ADR has a
**Security** section; every part that parses outside input says so.

## Principles

1. **Memory safety by default.** Rust everywhere; `unsafe` only in the HAL
   (enforced by `cargo xtask lint`), each block justified. Parsers of
   outside input (USB, the network, files on a card, PE images) are safe
   code with every length and offset checked, and never panic on malformed
   input: a bad packet is an error reply, not a crash. They get tests with
   hostile input (truncated, oversized, overflowing arithmetic).
2. **Least privilege.** User processes run unprivileged with their own
   address space (ADR-0003, ADR-0008); drivers run in a driver host process,
   not the kernel, where the hardware allows; pages are no more permissive
   than needed (W^X: no page both writable and executable for user code;
   kernel pages unreachable from user mode, e.g. KData is user read-only and
   the vector page kernel-only, ADR-0010).
3. **Nothing listens by default.** No network service is open unless the
   user turns it on; USB functions that expose data need the user's consent
   on the device.
4. **Modern cryptography only.** WPA2/WPA3 (SAE with H2E, PMF), TLS 1.3 with
   certificate validation and an updatable root store; no WEP, no TKIP, no
   TLS 1.0/1.1, no SSLv3 fallback. Constant-time implementations for
   secrets (SAE, AES, ECDH). A real random-number source seeded from
   hardware noise before any key is made.
5. **Updates are signed.** The OS image and its updates are verified with a
   public key before they run or are installed; the Dell bootloader region
   is never written (DESIGN §10c), so a bad update can always be recovered.
6. **Legacy apps are contained.** Windows Mobile software was written with
   no security model; the CE Personality runs it as unprivileged processes,
   with the file and registry views it expects, and without access to other
   apps' data or the system's keys.
7. **Measured, not assumed.** Every finding about an attack surface is
   written down here, and fixed or listed as an open item.

## Threat model (who and how)

| Threat | Path | Defence |
|---|---|---|
| Remote attacker on the internet | Browser, TLS, messaging, any network service | Memory-safe parsers; nothing listening by default; TLS 1.3 with validation; unprivileged, sandboxed apps; signed updates |
| Attacker on the same Wi-Fi | 802.11 frames, ARP/DHCP/mDNS spoofing | WPA3-SAE + PMF where the firmware allows; WPA2-CCMP otherwise; strict parsers; mDNS off by default |
| Malicious legacy app | CE API calls | Unprivileged process, per-app storage, no raw hardware or kernel access |
| Someone with the device or a cable | USB, SD/CF card, the serial loader | Consent before exposing storage; development loader not in releases; signed boot image |
| Hostile files | Images, documents, PE executables on a card | Bounds-checked parsers, fuzzed |

## Current exposures (development builds)

| Exposure | Why it exists | Status |
|---|---|---|
| **The resident loader runs any code sent over USB** (ADR-0012), and `RPF1` frames replace files on the card (ADR-0013) | The hands-off development loop | **Development only.** Never part of a release image. Before any public build: either removed, or gated by a physical action on the device plus a signed-image check. Tracked here until then |
| USB mass storage exposes the whole card (ADR-0014) | Owner's request: the card as a drive | In the loader (development) the drive is offered at once but **read-only** until the PC sends `drive rw` (`cargo xtask drive rw`); the card holds the loader and HaRET, and Windows writes a mounted drive on its own. In the OS proper: only after the user allows it on the device, per connection, read-only by default for cards holding the system |
| Serial loader frames are CRC-checked, not authenticated | Integrity against line noise, not attackers | Same as the first row |

## Findings

| Date | Where | Finding | Status |
|---|---|---|---|
| 2026-10-12 | `rp-fat` mount | A forged FAT size or root cluster overflowed the sector arithmetic: a panic in test builds, reads of the wrong sectors in release | Fixed: the geometry is validated with checked arithmetic at mount; 1,500-card corruption test |
| 2026-10-12 | `rp-loadproto`, `rp-usb-msc`, `pxa-udc`, `kernel/pe` | Random-input tests added (20,000 / 5,000 / 52,000 / 5,000 cases) | No findings |
| 2026-10-12 | `rp-fat` exFAT | Names are read cut at 64 characters, so a file with a longer name matched a lookup of its first 64: a `put` of that shorter name would have overwritten the other file | Fixed: only names decoded whole (not cut, all ASCII) match; test `a_cut_long_name_never_matches_its_prefix`, which fails without the fix |
| 2026-10-12 | `rp-drv-cf` | 3,000 hostile cards (random CIS bytes, random IDENTIFY words) | No findings: I/O cards and bad CIS refused untouched, capacity taken only as the card states it and every transfer kept inside it, every wait bounded |
| 2026-10-12 | `rp-fat` exFAT writes | Writing to hostile cards (1,500 corrupted images, boot checksum re-signed so the damage passes mount) and a power cut at every write of four kinds of put | No findings: errors, no panics or loops; no live cluster ever left marked free |

## Checklist for every ADR and parser

- What outside input does it accept, from whom?
- Every length, count and offset checked before use; arithmetic that can
  overflow uses checked/saturating forms.
- Malformed input produces an error, never a panic or a hang; tests prove it.
- What does it expose, to whom, and does the user consent?
- What runs privileged, and could it run unprivileged?
