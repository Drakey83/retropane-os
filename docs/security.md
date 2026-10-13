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
| **The resident loader runs any code sent over USB** (ADR-0012), and `RPF1` frames replace files on the card (ADR-0013) | The hands-off development loop | **Development only; the owner chose to keep it open until release (2026-10-12; ADR-0017 holds the lock-down design, a ROADMAP Phase 9 requirement).** Never part of a release image. Before any public build: either removed, or gated by a physical action on the device plus a signed-image check. Tracked here until then |
| USB mass storage exposes the whole card (ADR-0014) | Owner's request: the card as a drive | In the loader (development) the drive is offered at once and the SD card is **writable except what boots the device** (`rp_fat::Guard`, ADR-0014: the boot files' entries, chains and data and the card's layout can't be changed; read-only when no guard can be built); `drive ro` locks it. The CF card is read-only until `drive rw`. In the OS proper: only after the user allows it on the device, per connection, read-only by default for cards holding the system |
| Serial loader frames are CRC-checked, not authenticated | Integrity against line noise, not attackers | Same as the first row |
| The display host's test hooks: a call (`OP_TEST_FAULT`) and a thread role (`ROLE_TEST_FAULT`) that make it fault, so any client with its handle can restart it | The test kernel's supervision checks (item 46) | **Test kernel only.** The host image built for the OS proper must not have them (a build feature or a separate test image when the OS's hosts are built) |
| The loader's `trace` hands the last program's trace ring (thread numbers, addresses of faults and calls) to whoever holds the USB link | Debugging hangs and slow paths after the fact (item 54) | **Development only**, like the loader itself (first row). The OS proper keeps the ring for the user's own diagnostics, offered only with their consent |
| In the test kernel, a user fault in an **unsupervised** process outside a call still stops the whole system | The test image's way of failing loudly | Test kernel only; the OS proper ends the faulting process (and reports it) instead |

## Findings

| Date | Where | Finding | Status |
|---|---|---|---|
| 2026-10-12 | `rp-fat` mount | A forged FAT size or root cluster overflowed the sector arithmetic: a panic in test builds, reads of the wrong sectors in release | Fixed: the geometry is validated with checked arithmetic at mount; 1,500-card corruption test |
| 2026-10-12 | `rp-loadproto`, `rp-usb-msc`, `pxa-udc`, `kernel/pe` | Random-input tests added (20,000 / 5,000 / 52,000 / 5,000 cases) | No findings |
| 2026-10-12 | `rp-fat` exFAT | Names are read cut at 64 characters, so a file with a longer name matched a lookup of its first 64: a `put` of that shorter name would have overwritten the other file | Fixed: only names decoded whole (not cut, all ASCII) match; test `a_cut_long_name_never_matches_its_prefix`, which fails without the fix |
| 2026-10-12 | `rp-drv-cf` | 3,000 hostile cards (random CIS bytes, random IDENTIFY words) | No findings: I/O cards and bad CIS refused untouched, capacity taken only as the card states it and every transfer kept inside it, every wait bounded |
| 2026-10-12 | `rp-kernel-ipc` | 100,000 random operations from hostile processes (forged and cross-process handles, rights escalation, deep nesting, restarts, exits) | No findings: calls only through live handles with `CALL`; rights never grow; tables fixed-size |
| 2026-10-12 | `rp-crypto` HMAC-SHA256 | RFC 4231 vectors (all six full-tag cases), constant-time compare; checked on the target by the bench | No findings. ADR-0017 (proposed) designs the loader's pairing on top of it |
| 2026-10-12 | `rp-crypto` SHA-256 | Published test vectors (FIPS 180-2 appendix B, NIST examples, a million "a"); every split of the input | No findings; the loader now fingerprints what it runs and stores |
| 2026-10-12 | `rp-fat` check | 3,000 hostile volumes checked (FAT32 and exFAT) | No findings: bounded walks, the cluster bitmap sized before use, errors not panics |
| 2026-10-12 | `rp-fat` exFAT writes | Writing to hostile cards (1,500 corrupted images, boot checksum re-signed so the damage passes mount) and a power cut at every write of four kinds of put | No findings: errors, no panics or loops; no live cluster ever left marked free |
| 2026-10-12 | `kernel/pe` loading, kernel image | Images with a page both writable and executable are refused before mapping (W^X); each page maps with its own section's rights (code read+execute, constants read-only, data private copy-on-write), uncovered pages stay unmapped as guards; the image is never mapped from kernel memory (pages are loaded into fresh frames); relocation data bounded | No findings; host test `writable_code_is_found`, emulator checks `PE image relocated`, `PE pages loaded on demand` |
| 2026-10-12 | IPC supervision, kernel image | Servers learn their caller's slot from the kernel (unforgeable); `SYS_GRANT` only copies handles the granter holds with `GRANT`, never adding rights; a faulted supervised server's in-flight answers are dropped (`ERR_RESTARTED`), new calls fail while it restarts, and every pre-restart handle turns `Stale`; the restarted instance gets fresh memory | No findings; emulator checks `IPC service manager`, `IPC supervised restart`; host test `a_supervisor_keeps_its_handle_across_a_restart`. `SYS_GRANT` refuses slots with no live process since item 44 |
| 2026-10-12 | Scheduler (HAL `rt`), all images with user mode | A quantum ending while a thread was inside a system call was lost (no preemption in kernel mode, timer not re-armed): a thread, or a hostile program timing its calls, could keep the CPU indefinitely | Fixed: an interrupt in kernel mode leaves a preemption owed, paid on return to user mode; check `USER preempted after a system call`, which fails without the fix |
| 2026-10-12 | Processes, kernel image | Only the service manager may start processes; entries must lie in the image's code; a new process holds one handle (to the manager); exit closes its ports, drops its handles and returns every frame (checked) | No findings; check `IPC spawn and exit` |
| 2026-10-12 | Region lists, `kernel/mm` (ADR-0019) | Region storage moved from fixed tables into kernel pages charged to each process: a quota (8 pages, 1,632 regions) stops one process exhausting kernel memory with reservations; changes reserve room first, so a refusal (quota or no memory) leaves the regions untouched; list pages are wiped when freed, since they held the layout of a process's memory and a freed frame can become another process's page; never mapped for a process | No findings; host tests `region_operations_match_a_page_model_and_fail_atomically` (mutation-checked), `region_pages_are_wiped_before_they_go_back`; emulator check `VM many regions` |
| 2026-10-12 | Pinned memory, `kernel/mm` | Pinned memory can't be reclaimed, so it is a way to exhaust RAM: each space has a pin limit (64 KiB unless the kernel raises it for a trusted service), checked by `map` and `protect` | No findings; host test `pinning_is_limited_and_survives_protect` |
| 2026-10-12 | Loader, on the device | `card check` on the owner's 4 GB card read ~34,000 blocks (~40 s) without feeding the watchdog: a hardware reset into Dell's factory-restore prompt (the third such reset from the same class: long work not feeding the watchdog) | Fixed for the class: every file-system block read or write in the loader keeps the watchdog fed if armed (`Fed`), so progress is never taken for a hang and a real hang still is; the loader's other long commands audited (all fed); emulator check `keep-alive`. Next: a soft watchdog that returns to the loader instead of resetting (item 55) |
| 2026-10-12 | Soft watchdog, all images under the loader | A program past its run limit now faults back to the loader (crash report, no reset) instead of the hardware reset into Dell's factory-restore prompt, where a wrong button wipes the device; the hardware watchdog stays as backstop for hangs with interrupts off | Done; emulator tests `run limit` and `watchdog` |
| 2026-10-12 | Device runs, `cargo xtask load` | `rp-bench` (spins forever with IRQs off when done; emulator-only) was sent to the Axim and the hardware backstop reset it into Dell's factory prompt: the soft limit couldn't see a program with IRQs masked, and the tool didn't stop the wrong image | Fixed for the class: images classified in their budgets and never-returning ones refused by `load`; the soft limit routed to FIQ (never masked by RetroPane code), so IRQs-off spins come back too; mutation-checked emulator test |
| 2026-10-13 | Driver hosts, kernel image (item 46) | A driver in a user process gets only what the board grants it: one uncached page of device registers at a fixed address (the HAL offers windows only by grant, so safe user code can't build a pointer to another address; a window can never become cacheable or executable), and its interrupt, which only the granted process may wait on (others get `ERR_DENIED`), never from inside a call; the kernel enables the device's clock itself (the clock unit is never granted). A supervised host faulting in its own thread (not only in a call) now restarts it instead of stopping the system; its blocked thread re-enters at the entry on fresh memory. The ELF-to-PE packer reads its input as untrusted | No findings; emulator checks `IPC driver host served`, `IPC driver host restarted` and the power-button steps, each with a mutant (window off by a page, no re-entry, ungranted waiter); host tests `hostile_or_unexpected_input_is_refused` (packer) |

## Checklist for every ADR and parser

- What outside input does it accept, from whom?
- Every length, count and offset checked before use; arithmetic that can
  overflow uses checked/saturating forms.
- Malformed input produces an error, never a panic or a hang; tests prove it.
- What does it expose, to whom, and does the user consent?
- What runs privileged, and could it run unprivileged?
