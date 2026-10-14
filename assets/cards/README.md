# Memory card icons

The SD and CompactFlash card icons, by **Juanje** (puntodepartida.com,
the "Memory cards" set). Not RetroPane's own work and not under
RetroPane's licence: used with the author's permission, with credit.

| File | What it is |
|---|---|
| `card-sd.png`, `card-compact-flash.png` | 256 × 256 RGBA, the source for every other size |
| `card-sd.ico`, `card-compact-flash.ico` | Windows icons (16-256 px), made from the PNGs; shown as the card's drive icon through `autorun.inf` |

## Permission

The project owner (Reid Carlisle) wrote to the author on 2026-10-12,
describing RetroPane as a hobby open-source operating system for the Dell
Axim X51v and offering credit. The author replied on 2026-10-14: "Sure,
feel free to use them! ... Just crediting me as you mentioned is perfect."
The reply is kept in the owner's mail.

What that covers: using the icons in RetroPane, the open-source project
described, with credit to the author. Resizing and converting them for a
screen or an icon file is part of using them.

**What it doesn't say:** anything about selling RetroPane or a device
with it. Anyone who sells it checks with the author first, or ships the
fallback (the Crystal SVG pair by Everaldo Coelho, LGPL, Wikimedia
Commons: `Crystal128-sd-mmc-unmount.svg`,
`Crystal128-compact-flash-unmount.svg`, with their licence notice).

## Credit

Wherever RetroPane lists its credits (the About screen, the release
notes), these icons are credited to Juanje, puntodepartida.com.

## Where they're used

- The cards' drive icons on the PC: `cargo xtask sdkit` puts each icon
  with its `autorun.inf` in the kit (`cards/sd/`, `cards/cf/`), and
  `cargo xtask install` puts the SD pair on the SD card.
- On the Axim: File Explorer's card icons, through the shell's asset tool
  at the sizes it needs, once File Explorer exists.

The owner put them in the base system on 2026-10-14, an exception to the
rule that third-party art ships only in packs (docs/ui/README.md): written
permission, two small images.
