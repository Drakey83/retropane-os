# ADR-0020: Field Kit, a network and utility toolkit pack

- **Status:** Proposed (idea recorded 2026-10-13, the owner's initial ideas;
  specifics later). **The last thing built**, after the browser (the first
  pack, ADR-0018) and the rest of the roadmap. Not for a vote yet: the
  decisions below are the starting point when the work begins.
- **Date:** 2026-10-13
- **Last verified:** 2026-10-13 (against commit 3158b26)

## Context

ADR-0018 makes optional parts of RetroPane into **packs**: one signed,
verified file each (Ed25519 manifest, per-page SHA-256, LZ4 per page),
installable to flash or the card, removable without breaking the base
system, carrying their own licence. The browser is the first pack
(ROADMAP Phases 8-9).

The owner wants a toolkit of network and utility tools as a pack too,
built last, once everything else is finished. Why it's worth having:

- **Dogfooding.** The owner is its first user. A serial console, a crash
  log viewer, a hex editor and a raw TCP tool are tools for developing
  RetroPane itself (once the shell runs on the device; until then the PC
  side, `cargo xtask`, does these jobs).
- **It exercises the pack system** (format, signing, install, card
  removal, its own licence) with many small programs, where the browser is
  one big one.
- **Half of it needs no network**, so it can grow in phases.
- **A story:** the retro PDA that ships with a field toolkit.

Constraints that shape it:

- **docs/security.md**: memory-safe parsers for all outside input; least
  privilege (unprivileged, sandboxed apps); **nothing listens by default**;
  modern cryptography only; signed updates. This pack's whole subject is
  networks, so it is where these rules are tested hardest.
- **64 MB of RAM, no execute-in-place** (ADR-0005): every tool runs from
  RAM; a tool's RAM matters, its flash size much less.
> **Correction (note added 2026-10-13):** the ACX100's firmware is not
> being reverse-engineered yet: there is no docs/hw/acx100.md, ROADMAP
> Phase 6's ACX100 RE item is unchecked, and only its power-up sequence is
> known (docs/hw/cpld.md, "Wi-Fi (ACX100) power-up").

- **The X51v's ports**: USB is device-only as far as we know (the PXA270
  has a USB host controller, but no host port is known on the X51v's
  connector); the FFUART exists (docs/hw/README.md: "no connector pins
  verified"); Wi-Fi is the ACX100 (Phase 6 driver, firmware's abilities
  still being reverse-engineered).

## Decision

1. **A toolkit pack, built last.** Working name **Field Kit** (owner's
   candidate, recommended). Others considered: *Workbench*, *Patch Bay*,
   *Pocket Lab*, *Toolroll*. Field Kit fits a device carried into the
   field and doesn't promise more than it is.
2. **Every tool is a client.** No tool opens a listening port, accepts an
   inbound connection, or runs a server, ever. UDP tools receive only
   replies on the socket they sent from. The system's "nothing listens by
   default" holds with the pack installed; the pack's manifest declares the
   capabilities it needs (below), and the system refuses any it doesn't
   declare.
> **Correction (note added 2026-10-13):** neither ADR-0016 nor
> docs/security.md principle 2 defines per-app storage; ADR-0016 gives
> process isolation (own slot and domain, handles, a service manager
> handing out ports). Per-app storage is not designed yet.

3. **Each tool is an unprivileged, sandboxed process** with its own storage
   (ADR-0016, docs/security.md principle 2). Tools reach the network,
   files, the serial port and the Wi-Fi scanner only through system
   services, each a capability granted by the manifest and shown to the
   user at install. No tool gets raw sockets, raw 802.11 frames, raw block
   devices or another app's data.
4. **Every protocol parser is memory-safe Rust**, bounded and tested with
   hostile input (truncated, oversized, malformed), never panicking on bad
   input. Anything adapted from elsewhere meets the same bar (decision 7).
> **Correction (note added 2026-10-13):** docs/security.md principle 4 is
> "Modern cryptography only"; it doesn't call for a single library. The
> system's crypto library today is `rp-crypto` (kernel/crypto), with
> SHA-256 and HMAC-SHA256 only.

5. **Shared building blocks, once.** One terminal emulator (VT100/xterm
   subset, monospaced font, scrollback in a fixed buffer) serves the local
   terminal, the serial console and the SSH client. One hex view serves the
   hex editor and the HTTP inspector. Crypto comes from the system's one
   reviewed library (docs/security.md principle 4), not per tool.
6. **Two phases** (contents below), both after the rest of the roadmap:
   Phase 1 tools need no networking; Phase 2 tools use the network stack,
   TLS 1.3 and Wi-Fi scanning, added as pack updates.
7. **The SSH client adapts an existing implementation; we don't write the
   protocol.** Recommended: **sunset**, a `no_std` Rust SSH library by the
   author of Dropbear, built for small devices. Fallback: Dropbear's client,
   in a process of its own. See "Alternatives considered: the SSH client".
8. **The pack has its own licence file** (ADR-0018). The pack's own code is
   the project's; adapted code (the SSH library, a font) keeps its licence,
   listed in the pack's licence file.

### Contents

**Phase 1: no networking needed (shippable first).**

| Tool | What | Capabilities | Notes |
|---|---|---|---|
| Local terminal | A command line for the pack's own tools and the user's files | user files (read/write) | **Not a system shell**: no access to system files, other apps' data, or raw devices (least privilege). Its commands are the pack's tools plus file basics |
| Hex editor | View and edit files byte by byte; search; go to offset; undo | user files (read/write) | Files only. Editing raw card or flash blocks would need a privileged capability: not in v1 (see open questions) |
| Password manager | Encrypted local vault; generator; copy with auto-clear | its own storage; clipboard | Key from the master password with a memory-hard KDF (budget below); AES-256-GCM or XChaCha20-Poly1305; auto-lock; vault never leaves the device unencrypted; no cloud sync in v1 |
| File encryption and hashes | Encrypt/decrypt a file with a password or key; SHA-256 (and SHA-512, BLAKE2/3) of files; compare | user files | Authenticated encryption only; hashing streams, constant memory |
| Serial console | A terminal on a serial line | serial port (user-started) | **Physical path uncertain**, see open questions. While open it accepts bytes from the other end: an inbound channel, so it runs only while the user has it open, with a visible indicator, never in the background |
| Crash log viewer | Reads the crash reports the system writes (today the loader's `crashlog.txt`), searches and copies them | system crash reports (read-only) | Read-only; reports may hold memory addresses, shown only to the device's user |

**Phase 2: after TCP/IP + TLS 1.3 (ROADMAP Phase 6).**

| Tool | Capabilities | Notes |
|---|---|---|
| SSH client | outbound TCP | Decision 7. Host keys pinned on first use and checked after; Ed25519 / ECDSA / RSA keys, modern algorithms only (curve25519, chacha20-poly1305 / AES-GCM, no SHA-1, no CBC); agent forwarding and port forwarding **off** (forwarding *from* the device would be a listening port) |
| IRC client | outbound TCP + TLS | TLS by default |
| Message board client | outbound TCP + TLS | Which protocols (NNTP? web forums through the browser's engine?) is an open question |
| Port scanner | outbound TCP | Connect-scan only (no raw SYN scan: no raw sockets); rate-limited; warns that scanning others' networks may be illegal |
| Ping, traceroute | system ICMP service | **Not raw sockets** (an unprivileged app can't have them): the network service offers "echo" and "path to" with replies routed back only to the asking app. Needs that service in the network stack |
| DNS / whois lookup | outbound UDP/TCP | DNS through the system resolver, plus a direct query mode |
| Subnet calculator | none | No network at all: could move to Phase 1 |
| Netcat-style tool | outbound TCP/UDP | **Connect mode only.** Listen mode (`nc -l`) is a listening port: excluded unless the owner decides otherwise (open question) |
| HTTP inspector | outbound TCP + TLS | Raw request builder, header and body viewer (hex view) |
| Wi-Fi scanner | Wi-Fi scan service | Networks, signal, channel, security type from the Wi-Fi service's scan results; no raw frames |

**Needs investigation (not promised).**

| Tool | Why uncertain |
|---|---|
| Packet sniffer | Needs the ACX100 to deliver frames in promiscuous or monitor mode, which its firmware may not do; and raw frames are a privilege the sandbox doesn't give apps. Investigate during the Wi-Fi bring-up (Phase 6 RE); if possible at all, it would be a separate capability the user grants explicitly, and might be an own-traffic-only capture instead |

### Budget (estimates, to be measured)

| Part | Flash (compressed) | RAM while running |
|---|---|---|
| Terminal emulator + font (shared) | ~60-100 KB | ~100-300 KB (scrollback) |
| Phase 1 tools, together | ~300-600 KB | each under ~1 MB, except the password manager's KDF |
| Password manager KDF | — | 16 MiB for a second or two while unlocking (Argon2id), on purpose |
| SSH client (library + crypto shared with the system) | ~150-300 KB | ~0.5-1 MB per session |
| Other Phase 2 tools | ~300-500 KB | each under ~1 MB |

The tools are separate programs, stored on the card (or in flash) and run
one at a time. Where a program is stored doesn't change its RAM: it is
loaded into RAM when started either way (no execute-in-place, ADR-0005),
and a tool that isn't running takes none. So RAM is not a constraint for
this pack: one tool at a time needs well under 1 MB, against ~64 MB. The
password manager's KDF briefly uses more on purpose (that is what makes a
stolen vault slow to crack), and with the tool running alone even 16 MiB
for a second or two is affordable.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Phase 1 first, small tools, each with hostile-input tests and emulator checks; tools added as pack updates, each measured (flash, RAM, start time from card vs flash) |
| ✨ Modernize | Client-only by design, capability manifest shown at install, memory-safe parsers, modern crypto only (no SHA-1, CBC, RSA < 2048), SSH with pinned host keys and no forwarding; a password vault with a memory-hard KDF; everything signed (ADR-0018) |
| 🗄️ Legacy support | Familiar tools (PocketPuTTY-era SSH, netcat, whois) on a 2005 PDA, with the terminal speaking the VT100/xterm subset old and new servers expect; Windows Mobile apps of the same kind keep running beside it through the CE Personality |
| ⚡ Better / faster | One terminal, one hex view and one crypto library shared by every tool; streaming hashes and encryption in constant memory; LZ4 packs load fast from the card |

## Alternatives considered

### The SSH client

SSH is a large binary protocol (transport, key exchange, user auth,
channels, many algorithm choices) with a long history of security bugs in
mature implementations (memory-safety bugs, state-machine confusion,
weak-algorithm downgrades, terminal escape injection; the 2023 "Terrapin"
prefix-truncation attack affected nearly every implementation at once).
Writing one ourselves would repeat those bugs. Options:

- **Hand-roll the protocol in Rust.** Full control and memory safety, but
  months of work and a guarantee of protocol-logic bugs others have already
  found and fixed (Terrapin was a design-level flaw; implementations only
  got it right through shared review). **Rejected.**
- **sunset (Rust, `no_std`, recommended).** Written for small devices by
  the author of Dropbear (decades of SSH experience), no allocator needed,
  memory-safe, client and server in one library (we use only the client).
  Small, fits our no_std world and our size budget. Risks: younger and
  less battle-tested than OpenSSH or Dropbear; its maintenance, licence and
  feature set (key types, agent, Terrapin countermeasure "strict KEX") must
  be checked when the work starts (open question 1). Mitigation: our own
  hostile-input tests at its boundary, fuzzing, and the sandbox.
- **Dropbear's client (C).** Small (a client of ~100-200 KB), embedded-
  proven for over twenty years, permissive licence. But C: memory-unsafe
  code parsing hostile network input, against docs/security.md principle 1,
  and a porting job onto our system-call layer. **The fallback** if sunset
  isn't ready: run it as its own process with only an outbound-TCP
  capability, so a bug in it reaches nothing else.
- **PuTTY's SSH core (C).** Client-only, very mature, ran on Windows Mobile
  (PocketPuTTY) and could even run through the CE Personality. C again,
  and tied to its own front-end and platform layers. Kept in mind as a
  CE-Personality option for users, not as our client.
- **OpenSSH (C).** The reference, the most reviewed. Large, deeply POSIX,
  with privilege separation built around a Unix process model we don't
  have; its notable recent CVEs were in the server (2024 "regreSSHion"),
  but the client is still a big C program. Too big a port for a client.
  Rejected.
- **russh / thrussh (Rust).** Memory-safe and featureful, but built on
  async std/tokio: a large runtime we don't have, and much bigger.
  Rejected unless a no_std path appears.

### Network tools that need privileges

- **Raw sockets for ping/traceroute/SYN scans.** Normal on Unix as root,
  but a raw socket can forge any packet: an unprivileged sandboxed app must
  not have one. Chosen instead: a narrow system ICMP service, and
  connect-only scanning.
- **Netcat listen mode.** A listening port, against principle 3. Excluded
  by default; if the owner wants it, the only acceptable form is a
  foreground, user-started, time-limited listen on one port with a visible
  indicator, closed when the app leaves the screen (open question 3).

## Consequences

- **Order:** the pack machinery arrives with the browser (ADR-0018, Phases
  8-9); Field Kit comes after everything else, so nothing on the roadmap
  moves for it.
- **New system pieces the pack depends on:** a capability system for apps
  (network outbound, user files, serial, clipboard, crash reports, Wi-Fi
  scan); a terminal emulator component; an ICMP echo/path service in the
  network stack (Phase 6); a Wi-Fi scan service (Phase 6).
- **The system crypto library grows** (ADR or extension of the TLS ADR):
  SSH's needs (curve25519, Ed25519, chacha20-poly1305, AES-GCM) overlap
  TLS 1.3's; the password manager adds Argon2id.
- **Security reviews:** each tool gets its row in docs/security.md's
  findings when it lands, and the pack's capability list is reviewed as a
  whole before release.
- **Legal:** the port scanner (and any sniffer) ships with a plain warning
  about scanning networks one doesn't own.

## Open questions (for the owner)

1. **SSH library:** confirm sunset when the work starts (maintenance,
   licence compatible with the pack's, strict-KEX/Terrapin fix, key types,
   size); else Dropbear sandboxed. Recommendation: sunset.
2. **Serial console path:** USB CDC to a PC (works today: the loader is a
   CDC device), making the Axim a terminal for the PC's side; or RS-232
   through the connector's FFUART pins (needs the pins confirmed and a
   level-shifting cable, Dell's or home-made); or both. The FFUART pins are
   worth a probe on the device.
3. **Netcat listen mode:** excluded (recommended), or allowed only in the
   restricted foreground form above?
4. **Password manager:** the KDF at 16 MiB (recommended: the tools run one
   at a time, so it can afford the stronger setting; the unlock time is
   measured on the device and the passes tuned to about a second). Also:
   import/export formats (KeePass KDBX?), and whether a vault may live on
   the card (encrypted, so allowed by default?).
5. **Hex editor on raw devices** (card sectors, flash): useful for OS work,
   but a privileged capability. Leave out (recommended), or a separate,
   explicitly granted "raw storage" capability, never for flash's boot
   region?
6. **Message board client:** which boards/protocols (NNTP, a specific web
   forum API, Matrix rooms)?
7. **Subnet calculator** to Phase 1 (it needs no network)? Recommended.
8. **Name:** Field Kit, or another?
> **Correction (note added 2026-10-13):** ADR-0018 (Consequences, added
> 2026-10-12) lets a pack carry its own licence for third-party content;
> it says nothing about the project's own code under another licence.

9. **Licence:** the pack's own code under the project's GPL-2.0-or-later,
   or a separate licence as ADR-0018 allows (for example to let others
   build on the tools)?

## For when the work starts

Plus: tools the owner needs for RetroPane's own development; the pack
system exercised by many small programs; a clear security posture
(client-only, sandboxed, capabilities shown, memory-safe) on the riskiest
kind of app.
Minus: new system services (capabilities, ICMP, Wi-Fi scan, terminal);
the SSH client depends on an outside library we must vet and keep updated.
