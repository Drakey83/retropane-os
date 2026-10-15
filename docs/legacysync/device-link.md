# The LegacySync session on the USB link (design for review)

ADR-0028 milestone 4. Status: **proposed, for the owner's review.**
Nothing here is built; nothing is sent to the Axim until the owner says go.

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

Development path either way: the frames below are transport-independent,
so the PC and device code can be built over the loader's serial port first
(magic `RPS1`, a test image answering) and moved to B's interface without
changing the protocol.

### K2. Protection on the wire

| | Authenticated only | **Authenticated and encrypted (recommended)** |
|---|---|---|
| What | HMAC-SHA256 tag on every frame (as ADR-0017) | ChaCha20-Poly1305 on every frame after an X25519 handshake |
| Stops | Faked or replayed frames from another PC or program | The same, plus anyone reading the cable (a USB sniffer, a hostile hub) learning contacts, backups or files |
| Pairing | The PC's key crosses the cable in the clear once: someone recording that moment can impersonate the PC forever | Keys never cross the cable; a 6-digit code on both screens defeats a man in the middle |
| Cost on the device | SHA-256: Windows Mobile's own reaches ~9 MB/s on the X51v | ChaCha20 is faster than SHA-256 on ARM; both far above the cable's ~1 MB/s |
| Work | Done in `rp-crypto` | X25519 and ChaCha20-Poly1305 added to `rp-crypto`, constant-time, against published test vectors |

Recommended: **Noise protocol `XX` with X25519, ChaCha20-Poly1305 and
SHA-256** (a published, analysed handshake; Signal and WireGuard use
Noise). First connection: the two sides learn each other's long-term
public keys, both screens show a 6-digit code derived from the handshake,
the user taps Allow on the Axim if they match. Later connections: Noise
`KK` (both keys known), no code, no tap (unless the partnership was
revoked).

This needs the device's random number source (security.md principle 4)
before any key is made; the PC contributes randomness too, but the
device's own must not be guessable.

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

The device's own long-term key pair needs a private key at rest, with the
same physical-possession limit; until the RNG and storage exist it can be
made fresh each boot (then every boot re-pairs, like RAM only).

### K4. What the user sees on the device

| Moment | Device shows |
|---|---|
| A new PC connects | "Allow this computer? Code 418 207" (same code on the PC), Allow / Deny on the touch screen; 30 s without an answer is Deny |
| After Allow | The permissions the PC asked for, each switchable: Install programs, Files, Sync, Time, Backup, Internet pass-through (off) |
| An install arrives | "Install Toy Golf (Fathammer, 22.8 MB) from LegacySync?", plus any notes from the plan (e.g. "made for Windows CE 4.x") |
| Settings | Partnerships: each PC's name, last seen, its permissions, Remove |

No hardware button gets a new job (owner's rule): all on the touch screen.

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

## Order of work (after the owner's review)

1. `rp-crypto`: X25519, ChaCha20-Poly1305 (vectors, constant time).
2. The frame codec and Noise handshake as a shared `no_std` crate, host
   tested with hostile frames (as `rp-loadproto` is).
3. PC side in LegacySync, device side in a test image, over the
   development serial port; emulator test end to end.
4. The vendor interface (K1 B) in `drivers/pxa-udc` and the emulator.
5. Device runs only when the owner says go.
