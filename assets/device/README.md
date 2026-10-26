# The device icon

The Axim's own icon on the PC (Windows Explorer's "Axim X51v", served as
MTP's DeviceIcon: ADR-0037 Q5, reminder R60): Icons-Land's **"PDA"**, from
the Vista Hardware Devices set (iconarchive.com/show/vista-hardware-devices-icons-by-icons-land/PDA-icon.html).
Not RetroPane's own work and not under RetroPane's licence: used under
Icons-Land's demo licence, with credit.

| File | What it is |
|---|---|
| `pda.ico` | The icon exactly as Icons-Land made it: 9 sizes (16-256 px), 32-bit, 432,254 bytes, SHA-256 `e2fd161e408369c5651d3b0002c426ee2c19fa0ade279b1352aa639a84ebecc9` |
| `IconsLandDemoLicenseAgreement.txt` | The licence, as Icons-Land publishes it (the copy in their packs on softicons.com) |

## Licence

The owner chose the icon and read the licence on 2026-10-25 (R46). In
short (the text file is what counts):

- Free for personal and commercial projects.
- A backlink credit to http://www.icons-land.com on the About form.
- **Never modified in shape or colour:** the file ships byte for byte as
  made (`*.ico binary` in `.gitattributes` keeps git from touching it; a
  test holds its hash). No resizing, no repacking, no dropped sizes.
- Never sold, sublicensed or made available apart from RetroPane: it
  lives here, beside the licence, and goes only onto RetroPane's cards.

## Credit

Wherever RetroPane lists its credits (the About screen, the release
notes): "PDA icon by Icons-Land, http://www.icons-land.com".

## Where it's used

- `cargo xtask sdkit` puts it in the kit as `device.ico`, and
  `cargo xtask install` / `install-cf` put it on the cards (the SD card's
  root, the CF card's `RetroPane` folder), hidden from the PC and kept
  like the boot files.
- The loader serves it to Windows as the device's icon, read from the
  card in pieces as Windows asks (no copy in RAM), the SD card's first.
