# ADR-0017: Securing the development loader

- **Status:** Proposed (deferred until release, 2026-10-12, by the project
  owner: "keep it open until release ... this needs to eventually be locked
  down once it's no longer in active development, but it'll just be a hassle
  if we do it now"). The design below stands as the plan; locking the loader
  down (or removing it from release images) is a release requirement in the
  ROADMAP (Phase 9).
- **Date:** 2026-10-12
- **Last verified:** 2026-10-13 (against commit 3158b26)

## Context

The resident loader (ADR-0012, ADR-0013, ADR-0014) runs any image a USB host
sends, writes files to the card, and shows the card to the PC as a drive.
docs/security.md lists it as a development-only exposure that must be
"removed, or gated by a physical action on the device plus a signed-image
check" before any public build. The owner wants security taken very
seriously: the device will be on the internet.

The threat here is **whoever is at the other end of the USB cable**: a PC
that isn't the owner's, a compromised PC, or someone with brief physical
access to the cradle. (Someone who takes the card out can replace the loader
itself; that is the physical-possession case, handled later by signed boot
images.)

Facts that shape the design:

- **Nowhere to keep a secret.** Flash is never written. The card is shown to
  the USB host as a drive, so a key file on it would be readable by the very
  attacker it is meant to stop.
- **No hardware random number generator** is documented for the PXA27x; good
  entropy on the device is not yet available.
- **The device has a touch screen**, which the attacker at the cable does
  not control.
- `rp-crypto` now has SHA-256 and HMAC-SHA256 (item 32), checked against the
  published vectors on the host and on the target.

## Decision (proposed)

1. **Pairing confirmed on the device, as Android's "Allow USB debugging?"**
   The first control frame from a PC is a pairing request carrying a
   256-bit key the PC generated (the PC has good randomness). The loader
   shows an on-screen prompt, "Allow this computer?", with the key's short
   fingerprint (the PC prints the same one) and Allow / Deny buttons on the
   touch screen, as a modern phone asks "Trust this computer?"; no hardware
   key is given a new job (the owner's rule: buttons behave as Windows
   Mobile's do). Deny, or 30 s without an answer, refuses. Only an accepted key is kept, **in RAM
   only**: nothing at rest to steal; a reset forgets it (pair again, one
   tap per boot).
2. **Every frame after pairing is authenticated**: an HMAC-SHA256 tag over
   the frame (with a sequence number, so a recorded frame can't be replayed),
   checked in constant time (`rp_crypto::ct_eq`). Unauthenticated frames get
   "not paired" and nothing else. A second USB host plugged in later can't
   inject anything without the key.
3. **The drive is offered only after pairing**, read-only until `drive rw`
   (an authenticated command): consent before storage is exposed, as
   docs/security.md requires of the OS proper.
4. **Status says so**: the loader's status lines show "paired" or "not
   paired"; the crash screen and logs never show the key.
5. **Release builds**: the loader is not part of them; OS updates will be
   signed (Ed25519), a separate ADR once there is an update path.

## The four lenses

- **Iterate:** "anyone at the cable can run code" becomes "only a PC the
  owner approved on the device, this boot".
- **Modernize:** the pattern phones use for USB debugging, with a modern MAC
  and replay protection.
- **Legacy support:** Windows Mobile had no such check at all.

> **Correction (note added 2026-10-13):** Windows Mobile's own SHA-256 on
> the device runs at ~8.5-9.3 MB/s (docs/perf.md, `sha256_4k`), so a
> 70 KB frame is ~8 ms; RetroPane's is not yet measured on the device
> (the ~62 instructions per byte is the emulator baseline,
> `bench/baseline-emulator.json`).

- **Better/faster:** one tap per boot; HMAC on a 70 KB frame is a
  few milliseconds on the XScale (bench: ~62 instructions per byte).

## Alternatives considered

- **A key file on the card**: readable through the drive by the attacker.
- **A key compiled into the loader**: the loader binary is on the card, so
  readable too, and one leak opens every device.
- **Signed images only**: protects against foreign code but not against a
  foreign PC reading or writing the card; signing needs a key on the PC
  anyway, and pairing covers both.
- **No change until release**: the device will be docked to a networked PC
  every day; "development only" is not a security boundary.

## Consequences

- `cargo xtask` gains pairing (key generation, fingerprint display) and tags
  every frame; the emulator tests tap Allow on the emulated touch screen.
- The first command after each boot needs the owner at the device (one
  tap).
