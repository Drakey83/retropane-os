# ADR-0018: Optional packs: a complete base system in flash, extras on the card

- **Status:** Accepted (2026-10-12, by the project owner; the owner's
  idea). Built with the installer and the first big pack (the browser),
  ROADMAP Phases 8-9.
- **Date:** 2026-10-12
- **Last verified:** 2026-10-13 (against commit 3158b26)

## Context

The owner asked whether RetroPane could give itself room to modernize by
putting some of the OS on the SD card when one is present, and installing a
smaller system when one isn't.

> **Correction (note added 2026-10-13):** the flash size (~256 MB
> DiskOnChip G3) is not yet confirmed on the device (docs/design-brief.md,
> "Flash").

What limits RetroPane is **RAM, not storage**. The X51v has about 256 MB of
built-in flash (DiskOnChip) and 64 MB of RAM. Its flash can't run code in
place (ADR-0005), so everything runs from RAM whatever it is stored on:
files on the card don't save RAM. The whole OS, browser included, should
take a few tens of MB of flash, well within the 256 MB.

But some parts are big, optional, or both: the web browser, media codecs,
fonts and language packs, larger Windows Mobile game support (3D), tools
for developers. A user with a small flash budget (the rest of flash is
theirs for files) may want those on the card. SD cards are also cheap and
large (RetroPane reads SDHC and SDXC, unlike stock WM's 2 GB limit).

Constraints that shape the answer:

- **A card can disappear** (pulled out, failed, swapped). Programs run
  straight from their files (pages load on demand, ADR-0009), so removing a
  card mid-run would pull code out from under a running program.
- **A card can be changed elsewhere.** It goes into PCs; FAT and exFAT have
  no permissions. Anything executable on it is untrusted until checked.
  The device will be on the internet (docs/security.md).

> **Correction (note added 2026-10-13):** the 1.2 ms per block was the
> owner's 4 GB card; the 32 GB SDHC card later measured 0.42 ms per block
> with 4-block commands and 0.27 ms with 64-block ones (docs/perf.md,
> 2026-10-12 SDHC row). The flash's read speed is not measured yet, so
> "flash is faster" is still unproven.

- **The card is slower for small reads.** 4-bit multi-block reads measured
  about 1.2 ms per 512-byte block on the owner's X51v; flash is faster for
  the small scattered reads of starting programs.

## Decision

1. **The base system is always complete in flash** and never needs the
   card: the kernel, drivers, shell, settings, PIM, Wi-Fi and networking,
   the Windows Mobile compatibility core, and recovery. Not "barebones":
   everything a user needs with no card in.
2. **Optional parts are packs.** A pack is one file (a small archive of
   images and data) with a manifest: its name, version, size, what it needs,
   and a SHA-256 of every file in it. Packs install either to flash or to
   the card; the user picks, and the installer suggests the card when flash
   is short.
3. **Nothing needed to boot or to recover lives only on a card.** The
   installer refuses to put base-system files there.
4. **Packs are checked before use.** Each file's SHA-256 is checked when it
   is first opened (the page cache verifies what it reads, page by page,
   against a per-page hash list, so a large file isn't read whole first).
   Releases sign the manifests (Ed25519, checked against a key in the base
   system); a changed or unsigned pack is refused, with the reason shown.
5. **Removing the card is handled.** When the card lock switch or card
   detect says the card is going, the OS closes the programs running from
   its packs first (asking to save, as on shutdown). If the card is pulled
   anyway, those programs end cleanly (a page that can't be read ends the
   process with a message), never the rest of the system.
6. **Packs are stored compressed.** Files in a pack are LZ4-compressed
   per page (the codec the kernel already has, ADR-0005 rule 4); the page
   cache decompresses as it loads each page. The card is slow and the CPU
   is fast, so fewer bytes read beats the decompression time (added
   2026-10-12 from docs/design-brief.md).
7. **Packs on the card are read-only to programs.** Their settings and data
   live in the user's storage, so a pack can be removed and reinstalled
   without losing anything.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Start with the browser as the first pack; measure start-up time from card vs flash and the RAM it takes (the same either way), then decide per pack where it defaults to |
| ✨ Modernize | Signed, verified, removable components, like modern app stores, but offline and owned by the user; card-removal handling that Android's early "apps on SD" lacked |
| 🗄️ Legacy support | Old WM programs and games installed to the card keep working: the loader reads them from the card through the same page cache, verified the same way (their own files aren't signed, so they are hashed at install and checked against that) |
| ⚡ Better / faster | Flash stays free for the user's files; packs on the card load a little slower the first time, with no cost while running (pages are in RAM) |

## Alternatives considered

- **Two installs, "full" with a card and "barebones" without.** Simpler at
  first, but the system would change when the card does: settings and
  programs could vanish with the card, and two configurations double the
  testing. Rejected in favour of one base plus packs.
- **Everything in flash, no packs.** The simplest and fastest; flash is big
  enough for the OS. It loses the user flash space the big extras would
  take, and the option to carry them on a card. Kept as the default for
  small packs.
- **The card as swap (RAM extension).** The real shortage is RAM, so this
  is tempting, but SD cards are slow for random writes and wear out, and
  swapped data would leave the device's RAM for a removable card (a privacy
  risk). Compressed RAM (ADR-0005) does this job instead.

## Consequences

- The installer, the page cache (per-page hashes) and the card-detect path
  gain pack support; a signing key is created for releases.
- Packs need a format (an archive the page cache can read in place) and a
  manifest format; both are small and specified in their own docs when
  this is accepted.
- Nothing changes in the kernel work now: packs land with the installer
  (ROADMAP Phase 9) and the first big optional part (the browser, Phase 8).
- (Added 2026-10-12, from an outside review of the UI direction.) Packs
  are also the boundary for **licensed third-party content**: art under
  someone else's terms (e.g. an icon set used with permission) ships only
  as a pack carrying its own licence and permission, never in the base,
  which always has a complete look of its own.
- (Added 2026-10-12.) **Anything the screen shows must not depend on the
  card.** A wallpaper picked from a card is converted into the user's
  flash storage when picked; the base has its own wallpaper and a
  drawn-in-code fallback. See docs/ui/README.md.

## For the vote

Plus: more flash left for the user, big extras carried on a card, signed
and checked, a system that never breaks when the card goes.
Minus: more to build (pack format, signing, card-removal handling), and
packs on the card start a little slower.
