# ADR-0017: Securing the development loader

- **Status:** Proposed (2026-10-12, overnight queue item 32; needs the
  project owner's approval before any code)
- **Date:** 2026-10-12

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
- **The device has hardware buttons and a screen**, which the attacker at
  the cable does not control.
- `rp-crypto` now has SHA-256 and HMAC-SHA256 (item 32), checked against the
  published vectors on the host and on the target.

## Decision (proposed)

1. **Pairing confirmed on the device, as Android's "Allow USB debugging?"**
   The first control frame from a PC is a pairing request carrying a
   256-bit key the PC generated (the PC has good randomness). The loader
   shows a prompt naming the key's SHA-256 fingerprint (the PC prints the
   same fingerprint) and waits for a hardware button (Calendar: accept;
   anything else or 30 s: refuse). Only an accepted key is kept, **in RAM
   only**: nothing at rest to steal; a reset forgets it (pair again, one
   button press per boot).
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
- **Better/faster:** one button press per boot; HMAC on a 70 KB frame is a
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
  every frame; the emulator tests press the button through the monitor.
- The first command after each boot needs the owner at the device.
- Open question for the owner: which button accepts (Calendar is proposed)?
