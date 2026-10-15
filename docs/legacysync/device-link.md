# The LegacySync session on the USB link (design for review)

ADR-0028 milestone 4. Status: **accepted by the owner, 2026-10-15**: K1 B,
K2 encrypted (Noise XX then KK), K3 on the card, K4 as described, with
the owner's review points worked in (named Noise patterns, the RNG
dependency, a signed partnership list, the email scope, transport-agnostic
framing). Nothing here is built; nothing is sent to the Axim until the
owner says go.

## In plain words

LegacySync on the PC and RetroPane on the Axim need a private, trusted
conversation over the Dell cable: "here is Toy Golf, may I install it?",
later "what time is it", "copy this file", "back up". Today the cable
carries the **developer loader**, which runs anything it's sent and never
ships to users. LegacySync needs its own channel that:

- exists in the OS proper, with or without the loader,
- only talks to a PC the user approved on the device (a partnership,
  ADR-0028 L5), with a code shown on both screens the first time,
- can't be read or faked by anything else on the cable,
- asks the user on the device before anything is installed or exposed.

## What is already there

- The X51v is a USB 1.1 composite device: CDC-ACM serial (the loader's
  link) + mass storage (the card as a drive), built-in Windows drivers,
  ~1 MB/s (ADR-0012, ADR-0014).
- The loader's frames: `RPL1` (run an image), `RPF1` (write a file),
  `RPC1` (a command), each magic + length + CRC-32 (`boot/loadproto`).
- ADR-0017 (proposed, deferred to release) designs the loader's pairing:
  a PC key accepted by a tap on the device, kept in RAM only, HMAC-SHA256
  on every frame. `rp-crypto` has SHA-256, HMAC-SHA256 and a constant-time
  compare.
- No hardware random number generator is documented for the PXA27x
  (ADR-0017); docs/security.md requires one, seeded from hardware noise,
  before any key is made on the device.

## Decisions for the owner

### K1. Which USB function carries it

| | A: the loader's serial port, new magic `RPS1` | **B: its own USB interface, driverless WinUSB (recommended)** |
|---|---|---|
| How | Same CDC-ACM COM port; frames starting `RPS1` go to the LegacySync service | A third interface in the composite device: one bulk IN + bulk OUT pair, vendor class, with the Microsoft OS 2.0 descriptors that make Windows 8.1+ bind its built-in WinUSB driver (no INF, no driver install) |
| In release images | The serial port must stay for LegacySync, so "the loader never ships" means a serial port that is LegacySync-only | No serial port at all; the loader's CDC interface simply isn't in release images |
| PC side | COM port numbers change, other programs can open the port | LegacySync opens the device by its USB IDs; no COM port to confuse anyone |
| Mac/Linux later | Works (serial) | Works (libusb) |
| Work | None on the device's USB side | Descriptor work in `drivers/pxa-udc` (the UDC has endpoints to spare: 5 of 23 used); an emulator model of the new interface (docs/hw/emulator.md rule) |

Development path either way: the PC and device code are built over the
loader's serial port first (magic `RPS1`, a test image answering) and
moved to B's interface later without changing the protocol. That only
works if the protocol is **transport-agnostic from the first line**, so
the serial phase can't bake in assumptions:

- **The codec sees bytes, not reads or packets.** It is a `no_std` crate
  fed arbitrary byte slices (`push(&[u8])`) that hands out whole frames;
  it never assumes one read is one frame, as a serial port can split or
  join anything.
- **No meaning in packet boundaries.** A frame ends where its length
  field says, never at a short USB packet or a zero-length packet; a
  frame may span many bulk packets and a packet may hold the end of one
  frame and the start of the next.
- **No meaning in serial line behaviour** either: no line endings, no
  timeouts as delimiters, no reliance on the port being opened or closed
  (ADR-0012: on this hardware the PC doesn't even see the device leave).
- **Session state lives above the transport.** A cable pull or a
  transport swap ends the session the same way (keys forgotten, the PC
  reconnects); the install's resume-by-offset sits in the messages, not in
  the link.
- **Guard test:** the codec's tests feed the same frame stream split at
  every byte boundary, merged into one slice, and cut into 64-byte
  "packets", and must produce identical frames each way.

### K2. Protection on the wire

| | Authenticated only | **Authenticated and encrypted (recommended)** |
|---|---|---|
| What | HMAC-SHA256 tag on every frame (as ADR-0017) | ChaCha20-Poly1305 on every frame after an X25519 handshake |
| Stops | Faked or replayed frames from another PC or program | The same, plus anyone reading the cable (a USB sniffer, a hostile hub) learning contacts, backups or files |
| Pairing | The PC's key crosses the cable in the clear once: someone recording that moment can impersonate the PC forever | Keys never cross the cable; a 6-digit code on both screens defeats a man in the middle |
| Cost on the device | SHA-256: Windows Mobile's own reaches ~9 MB/s on the X51v | ChaCha20 is faster than SHA-256 on ARM; both far above the cable's ~1 MB/s |
| Work | Done in `rp-crypto` | X25519 and ChaCha20-Poly1305 added to `rp-crypto`, constant-time, against published test vectors |

Noise is a framework, not one handshake, so the patterns are named (the
security properties differ by pattern):

| When | Protocol name | Why this pattern |
|---|---|---|
| First connection (pairing) | `Noise_XX_25519_ChaChaPoly_SHA256` | Mutual authentication with **no prior shared secret**: each side sends its long-term public key, encrypted, during the handshake. On its own that is trust-on-first-use; the 6-digit code closes the gap: both screens show a code taken from the handshake hash, which a man in the middle can't make match on both ends (one guess in a million per attempt; the device allows 3 failed pairings, then waits a minute) |
| Every later connection | `Noise_KK_25519_ChaChaPoly_SHA256` | Both long-term keys already known from pairing: mutual authentication from the first message, no code, no tap. A PC whose key isn't in the partnership list can't complete it |

Both: fresh ephemeral keys every session, so recording traffic and later
stealing a long-term key doesn't reveal past sessions (forward secrecy);
the Noise prologue carries `RPS1` and the protocol version, so a
downgrade or a frame from another protocol fails the handshake. The
6-digit code is the first 20 bits of an HMAC-SHA256, keyed with the
handshake hash, over the label "LegacySync pairing", shown as 6 decimal
digits. Revoking a partnership (Settings) removes the PC's key, so its
next `KK` fails and it must pair again through `XX`.

### K2's dependency: the device's random number source (schedule risk)

Checked 2026-10-15 on `main` (49061cb): **there is no RNG.** No code
gathers entropy, docs/security.md principle 4 only requires one, and the
ROADMAP has no item for it. Every key in K2 (the ephemeral keys each
session, the device's long-term key at pairing) needs it, and the PC's
randomness can't stand in: the device's half of each handshake must be
unguessable to a hostile PC.

So it is now **a ROADMAP item in its own right, ahead of the LegacySync
link**, with its own design (sources on this hardware to measure: jitter
between independent clocks (the CPU cycle counter, the 3.25 MHz OS timer,
the 32.768 kHz RTC oscillator), touch and battery ADC noise,
audio ADC noise from the WM8750; a seeded ChaCha20-based generator,
reseeded, refusing to hand out keys until enough entropy is in). The
link's device side does not start before it exists; the PC side, the
frame codec and the crypto primitives (X25519, ChaCha20-Poly1305) can.

### K3. Where the device keeps partnerships

ADR-0017 keeps the loader's key in RAM only (re-approve each boot). A
partnership should survive a reset (ActiveSync's did). What it stores is
the **PC's public key** and the permissions given, never a secret, so
being readable is fine; being *writable* by a PC is not (a PC with write
access to the drive could add itself).

| | RAM only (tap once per boot) | **On the card, protected from the drive (recommended)** |
|---|---|---|
| How | As ADR-0017 | A partnership file in RetroPane's system folder; the mass-storage function's write guard (`rp_fat::Guard`, which already protects the boot files, ADR-0014) refuses writes to it from the PC |
| Survives reset | No | Yes |
| Risk | None at rest | Someone with the card in their hands can edit it: the physical-possession case, the same as replacing the OS, handled by signed boot images later |

**Signing the partnership list** (owner's review): the device signs the
list (Ed25519, or an HMAC-SHA256 tag) whenever it writes it, and checks
it at boot; a list that fails is treated as empty (every PC pairs again)
and the user is told. This is built in from the start because it is
cheap, but how much it protects depends on **where the signing key
lives**, and it has to be said plainly:

| Where the key lives | What signing stops |
|---|---|
| On the card, beside the list (SD-card stage, DESIGN §10c stage 2) | Corruption, and edits by anything that doesn't read the key. **Not** someone with the card: the key is on the same card, so they can re-sign their edit. That is circular, and the note doesn't claim otherwise |
| In the device's flash (flash-install stage 3; outside the Dell bootloader region, which is never written) | Card edits: taking the card out no longer gives the key. Someone with the whole device and the tools to read its flash still can |
| Derived from the user's lock-screen PIN (if they set one) through a slow password hash | Card edits even in the SD-card stage, without flash. The list can't be checked until the PIN is entered, so LegacySync pairs only after unlock |

Recommended: build the signing now with the key on the card (it costs
almost nothing and the format won't change), move the key to flash at
stage 3, and offer the PIN-derived key when RetroPane gets a lock screen.
The device's long-term Noise key has the same storage question and gets
the same answer; until the RNG and storage exist it can be made fresh
each boot (then every boot re-pairs, like RAM only).

### K4. What the user sees on the device

| Moment | Device shows |
|---|---|
| A new PC connects | "Allow this computer? Code 418 207" (same code on the PC), Allow / Deny on the touch screen; 30 s without an answer is Deny |
| After Allow | The permissions the PC asked for, each switchable: Install programs, Files, Sync, Time, Backup, Internet pass-through (off) |
| An install arrives | "Install Toy Golf (Fathammer, 22.8 MB) from LegacySync?", plus any notes from the plan (e.g. "made for Windows CE 4.x") |
| Settings | Partnerships: each PC's name, last seen, its permissions, Remove |

No hardware button gets a new job (owner's rule): all on the touch screen.

## Legacy programs never meet the security (owner's rule, 2026-10-15)

The point of LegacySync is that old programs work. So the protection in
K2 and K3 sits **only between LegacySync and RetroPane**, two programs we
write; it is never asked of old software:

```
old setup.exe / RAPI tool  --(ActiveSync's own 2005 interfaces, unchanged)-->  LegacySync on the PC
LegacySync on the PC       --(K1-K3: paired, encrypted)---------------------->  RetroPane
```

- An old installer sees exactly what it saw in 2005: the registry keys,
  `CeAppMgr.exe` with its INI, later `rapi.dll` with its old calls. It
  never needs modern crypto, signing, a new SDK or any update.
- Old CABs are not held to modern rules: unsigned CABs install (Windows
  Mobile's own signing is not required), a CAB whose script names an
  older Windows CE gets a note, not a refusal (GeoRally, Stuntcar), and
  old encodings and INI quirks are accepted.
- **What is refused, and why, is kept short**, and each refusal says what
  happened in plain words and goes into the log:
  - *Damaged*: a CAB that fails its own checksums or whose script points
    at files it doesn't contain. Installing it would install garbage.
  - *Dangerous*: a file path climbing out of its folder (`..\..\Windows`).
    No real CAB seen does this; it only shows up in attacks.
  - *Not understood yet*: a `_setup.xml` feature or a compression type we
    haven't met (LZX). This is a gap in RetroPane, not the program's
    fault. It is reported as "not supported yet" with the program's name
    in the log, so it gets added, rather than half-installed.
- Every real program that is refused is a bug report against RetroPane.
  The parsers have already been loosened once for real data (CityTime's
  registry key holds quotes), and that is the rule going forward: real
  programs win, as long as the device stays safe.

## Scope: what "ActiveSync's jobs" covers, and email

The K4 permissions (Install, Files, Sync, Time, Backup, Internet) cover
what **desktop** ActiveSync did over the cable. Email needs separating,
because two different things were both called ActiveSync:

| | Desktop ActiveSync's email | Exchange push email ("Direct Push", Exchange ActiveSync) |
|---|---|---|
| What it was | Outlook's Inbox copied to Pocket Inbox while docked | The device talking **directly to the Exchange server** over Wi-Fi or GPRS, mail pushed as it arrived; desktop ActiveSync was only used to set it up |
| Runs on | The PC, over the cable | The device, over the network |
| In this plan | Covered: ADR-0028 L4 (the device's own mail, plus an optional copy of Outlook folders by LegacySync) | **Not LegacySync's job.** It belongs to the device's mail app (ROADMAP Phase 8, "Messaging") |

So the claim is narrowed: **LegacySync does desktop ActiveSync's jobs.**
Exchange push mail is a device feature and is **not in scope here**.
Whether RetroPane's mail app speaks the Exchange ActiveSync protocol is
an open Phase 8 decision for the owner. Microsoft 365 and many company
servers still offer it, and it is a large piece of work (its own binary
XML, server policies, remote wipe) next to IMAP/SMTP. The same goes for
ActiveSync-protocol contacts and calendar; L4's CardDAV/CalDAV route
covers Google, iCloud and Nextcloud.

## The frames

Every frame, both directions:

| Bytes | Field |
|---|---|
| 4 | `RPS1` |
| 1 | kind: 1 handshake, 2 message |
| 1 | flags: bit 0 = payload is LZ4-compressed (messages only) |
| 2 | 0 (reserved, must be 0) |
| 4 | payload length (≤ 64 KiB + 16), little-endian |
| n | payload: a Noise handshake message, or an encrypted message whose AEAD covers the 12 header bytes and uses a per-direction counter as nonce (a replayed, reordered or dropped frame fails) |

A frame that fails any check ends the session (the device forgets the
session keys and shows nothing new); the PC reconnects. Bytes before
`RPS1` are skipped, as the loader does.

Messages (inside the encryption): a 2-byte type, a 4-byte request id, then
fields; each request gets one reply with the same id. First set:

| Message | Fields | Permission |
|---|---|---|
| `hello` | protocol version, device name, OS version reported (5.1), processor (2577), platform (`PocketPC`), free space per store | any partner |
| `time.set` | UTC time, PC's time zone | Time |
| `install.offer` | app, provider, size, SHA-256 of the CAB, the plan's notes | Install |
| `install.answer` (device) | accepted / declined (the user's tap) | |
| `file.chunk` | offset, bytes (≤ 32 KiB), optionally LZ4 | Install, Files |
| `install.done` | | Install |
| `install.result` (device) | installed / refused + why | |

Install flow: offer, the user taps Accept on the device, the CAB arrives
in chunks into a staging folder on the card (resumable after a cable pull
by offset), the device checks its SHA-256 and the whole CAB
(`rp-cab`, `rp-ce-setup`, the same parsers LegacySync used) **before**
installing anything, installs, reports. Nothing is installed from a
partial or unverified CAB.

**Compression**: measured on the owner's seven real CABs (zlib level 1,
32 KiB chunks, as a stand-in for LZ4): apps shrink to 49-54%, games only
to 85-98% (their data is already compressed). So the flag is per chunk,
set only when it saves at least an eighth; LZ4 because the device already
has a fast safe decoder (`rp-lz4`). At ~1 MB/s, Toy Golf takes ~23 s
either way; Dell's Diagnostic CAB halves.

## Beside the developer loader

- The loader never sees LegacySync frames and LegacySync never sees the
  loader's: separate interfaces (K1 B), or separate magics on the
  development serial port (K1 A / the development path).
- Release images have no loader (docs/security.md, ADR-0017), so nothing
  that runs unpaired code exists there; LegacySync's session is the only
  PC channel besides the consented drive.
- The loader's own pairing (ADR-0017) stays separate: a developer
  approving their PC for the loader doesn't make it a LegacySync partner,
  and the reverse.

## Security summary

- **Outside input**: every frame from the PC is hostile until checked:
  length capped before reading, reserved bits zero, AEAD verified before
  parsing the payload, message fields bounded; CABs parsed by the
  fuzz-tested `rp-cab` and `rp-ce-setup`.
- **Exposes**: nothing until the user allows a PC on the device; then
  only the features they switched on, revocable in Settings.
- **Listens**: the USB interface only while a cable is in, and only the
  handshake until paired. Nothing on any network.
- **Privilege**: the session service is an unprivileged process; installs
  run through the CE personality's installer with the app's sandbox.
- **Depends on**: the device RNG (security.md principle 4) before keys are
  made; X25519 and ChaCha20-Poly1305 in `rp-crypto`.

## Order of work (accepted 2026-10-15)

0. **The device RNG** (its own design and ROADMAP item; K2's dependency
   above). Gates every step that makes a key on the device.
1. `rp-crypto`: X25519, ChaCha20-Poly1305 (vectors, constant time), and
   Ed25519 or HMAC signing for the partnership list.
2. The frame codec (bytes in, frames out; the split/merge/packet guard
   test from K1) and the Noise `XX`/`KK` handshakes as a shared `no_std`
   crate, host tested with hostile frames (as `rp-loadproto` is).
3. PC side in LegacySync; device side in a test image over the
   development serial port, once step 0 exists; emulator test end to end.
4. The vendor interface (K1 B) in `drivers/pxa-udc` and the emulator.
5. Device runs only when the owner says go.

Steps 1 and 2, and the PC side of 3, don't need the RNG and can go first.
