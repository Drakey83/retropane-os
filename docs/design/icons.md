# Icons: the set, packs and custom icons

Part of the design area: [README.md](README.md). Written 2026-10-30 after
the owner's rulings: "lets go with this, we can replace with new ones if i
find a set i like better with the correct license, these will just be a
stand in. also, we need to make sure custom icons is part of our
functionality." The icon list and the drawings: [icon-brief.md](icon-brief.md).
Weather icons have their own packs ([weather.md](weather.md)); this page is
every other icon the OS draws.

## The base set: Claude's drafts, as a stand-in (owner, 2026-10-30)

- **Ships as the base set** until the owner finds a set he likes better
  with a licence that allows it (CC BY 4.0, or written permission for
  shipping, packs and conversion: icon-brief.md). This replaces the earlier
  D5 note in [ui.md](ui.md) that the shipped set would not be Claude-drawn.
- **Source:** the vector masters in `tools/mockups/icon_drafts*.js` (about
  110 icons plus states), checked by `tools/mockups/icon_audit.py`; Juanje's
  SD and CF cards stay his.
- **Swapping it later costs nothing but the art:** the base set is just the
  first pack in the format below, so a better set replaces it file for file.
- **Its licence is the owner's call** (vote IC1 below).

## What a user can change

Four levels, each falling back to the one below, so nothing is ever left
without an icon:

1. **One item:** any shortcut, folder, program or desktop icon, from its
   tap-and-hold menu, **Change icon** (ui.md's menu). Picks from the current
   pack, any other installed pack, a picture on the card (PNG or ICO), or
   an icon inside a program (a legacy Windows Mobile `.exe` or `.dll`, as
   Windows XP allowed). **Reset icon** goes back.
2. **One file type:** in File Explorer, a file's Properties, Change icon for
   every file of this type.
3. **The whole set:** an **icon pack**, chosen in Settings, Themes &
   desktop, Icons. Packs come in the base (the stand-in), from the
   expansion pack on the card ([ADR-0018](../adr/0018-optional-packs.md),
   the open glossy sets such as Crystal Clear and Nuvola, LGPL, as
   optional packs only), or from the user.
4. **The base set**, which always has every icon.

A pack may cover only some icons: whatever it leaves out comes from the
base set (the same rule answers weather.md's open question for weather
packs: missing night icons fall back to day ones).

**Legacy programs join the theme.** A Windows Mobile program shows its own
icon by default, as on WM. A pack may also carry icons for known legacy
programs (keyed by the program's file name and a hash of its icon), so an
old app gets a matching glass icon when the user's pack has one; the
program itself is never changed.

## The pack format

The weather packs' rules, extended to named icons:

- A pack is a folder on the card, `Icons/<pack name>/`, holding
  `pack.txt` (name, author, link, licence, version; text through the
  strings table for any language) and one picture per icon.
- **Pictures are named by stable icon ids**, one per entry in
  icon-brief.md's list: `app.calendar.png`, `folder.music.png`,
  `type.pdf.png`, `place.trash.full.png`, `settings.storage.png`,
  `status.battery.50.png`, `badge.lock.png`, and so on; legacy programs as
  `legacy.<file name>.png`. The full id table is generated from the drafts'
  names, so the base set and the format can never drift apart.
- **One square master per icon,** transparent PNG, 256 px or larger. A
  maker may add hand-tuned small sizes (`app.calendar@32.png`) and dark-mode
  versions (`app.calendar.dark.png`) where the master does not read well;
  otherwise the device makes them.
- **Data only:** no code, no scripts. Checked like any input: size and
  count capped, decoded by our own validating PNG reader (and an ICO reader
  for user pictures and legacy programs: old 1, 4 and 8-bit icons with
  masks included, never refused for being old), refused if damaged
  ([security.md](../security.md); memory: legacy not locked out).
- **Makers' tool:** a PC command turns a folder of SVGs or PNGs into a pack
  and runs the same checks as the device, so a maker sees problems before
  copying it to the card.

## How it runs: decoded once, never while drawing

- **At install** (or when the user picks a picture), the shell decodes each
  picture once, scales it to the sizes the shell uses (desktop, lists,
  menus, status bar), makes the light and dark variants, and writes them
  into an **icon atlas** in the display's own pixel format, kept on flash
  or the card.
- **While running,** drawing an icon is a copy from the atlas (a 2D
  blit), with no PNG or ICO decoding and no scaling; only the icons on
  screen are paged in, so the 64 MB of RAM holds a few, not the whole set.
- **Changing a pack** rebuilds the atlas in the background with a progress
  bar; the old atlas stays in use until the new one is complete, so a
  failed or removed card never leaves the shell without icons.
- Sizes and memory are budgeted when the shell's icon sizes are fixed
  (the figures to measure: atlas bytes per pack, rebuild time on the X51v).

## Iterate, don't imitate (CLAUDE.md rule 0)

- **Windows Mobile 5:** no icon themes; each program shows its own icon,
  and changing one needed a third-party shell (SPB, Pocket Plus).
- **Windows XP:** Change Icon per shortcut, from `.ico`, `.exe` or `.dll`
  files; no whole-set themes without third-party tools.
- **Linux (freedesktop icon themes):** named icons, a fallback chain
  (`Inherits=`), many sizes, SVG drawn at run time.
- **Ours:** freedesktop's named ids and fallback chain for whole packs, plus
  XP's per-item Change icon (legacy program icons included), plus packs
  that dress legacy programs; but every picture is checked and converted
  **once** into an atlas in the display's format. **Why it wins here:** no
  SVG or PNG decoding while drawing (a 624 MHz CPU with no FPU), a small,
  checked parser surface for an internet-connected device, partial packs
  that can never leave a hole, and old programs that match the theme
  without being touched.

## Votes for the owner

- **IC1, the stand-in's licence:** the OS's own (GPL-2.0-or-later, simplest),
  or CC BY 4.0 (lets anyone reuse the icons elsewhere with credit).
- **IC2, legacy program icons:** shown as the program's own by default, with
  a pack able to replace them (proposed), or always the program's own.
- **IC3, user pictures:** PNG and ICO (proposed), or PNG only.
- **IC4, where Icons lives in Settings:** under Themes & desktop (proposed),
  or a page of its own.
- **IC5, a live Calendar icon:** the Calendar icon shows today's date
  (drawn over the icon at the atlas's draw time) instead of a fixed "30".
