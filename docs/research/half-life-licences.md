# Half-Life in the OS: what the licences say

A finding for the owner's ruling (docs/design/half-life-readiness.md line
13), read 2026-11-15 from the sources fetched that day (Xash3D FWGS commit
707596f4, hlsdk-portable commit 89e83b92) and from this repository. **Not a
legal opinion:** what the texts say, what follows for how the port is
built and shipped, and the choices. The owner ruled on 2026-11-15 (below).

## The four things and their terms

| What | Whose | Terms (as read) |
|---|---|---|
| RetroPane OS and its port layer | ours | GPL-2.0-or-later (`Cargo.toml`, `LICENSE`) |
| The engine, Xash3D FWGS | Uncle Mike, FWGS and contributors | GPL version 3 or later: 384 of its own source files carry that header. It has no licence file at its top. **33 of its files are Valve's**, from the Half-Life SDK (`engine/eiface.h`, `cdll_int.h`, `studio.h` and the like): their header restricts use to "non-commercial enhancements to products from Valve LLC" |
| The game's logic (the Half-Life SDK, hlsdk-portable) | Valve | The "Half Life 1 SDK License" (its `LICENSE`, read whole): free to use "to develop a modified Valve game running on the Half-Life 1 engine"; the SDK and changes to it may be copied and distributed in source and object form "but only for free"; every distribution must carry the licence file, the copyright notice and its two disclaimers; commercial use needs Valve's word. (The engine's porting guide says the portable SDK's `public/build.h` is relicensed under the Unlicense so it can be shared with the engine.) |
| The game's files (the `valve` folder: maps, models, sounds) | Valve | Not redistributable; the user's own copy, as with every game in apps-and-packs.md |

## What follows

1. **Our code with the engine is allowed.** GPL-2.0-or-later code may be
   combined with GPL-3.0-or-later code; the combination is then under
   GPL version 3. So the port's renderer (`ref_rp`), its platform backend
   and whatever of RetroPane is linked into the engine's program go out
   under GPLv3 with their source, which RetroPane publishes anyway.
2. **The game's logic may not be linked into the engine as one program.**
   Valve's SDK terms (free only, no commercial use) are not compatible
   with the GPL, which allows no such added restriction. The engine's own
   guide says so: its maintainers do not support platforms that cannot
   load dynamic libraries, "as supporting full-static ports are violating
   the GPL license in various ways". Every Xash3D port keeps the game's
   logic a separate library the engine loads at run time, as the original
   game does. For RetroPane this settles half of readiness line 6: **the
   game's logic is a library of its own, loaded by our loader**, not one
   statically linked image. (The engine has a static mode,
   `platform/misc/lib_static.c`; it is the road the guide warns about.)
3. **"Only for free" reaches whatever ships the game's logic.** The SDK
   library, and the 33 Valve-headed files inside the engine, may be given
   away and not sold. RetroPane OS is free, so today nothing collides. It
   would if RetroPane (or an Axim with it installed) were ever sold with
   the Half-Life port inside.
4. **The engine itself carries Valve's files.** That mix (GPLv3 files
   beside files restricted to non-commercial use) is how Xash3D has been
   published since 2007 and how every port of it ships; Valve has not
   acted against it and has not blessed it in writing that I found. It is
   a standing risk of using Xash3D at all, not one RetroPane adds.
5. **In the base image or in a pack** changes nothing for the GPL (two
   programs on one card or in one flash image are an aggregate, each
   under its own terms). It changes two practical things: a base image
   holding the port must carry Valve's licence file and notices and the
   GPL's offer of source in the OS itself, and point 3 then binds the
   whole OS image, not one download.

## The owner's ruling (2026-11-15): choice A

**In the base OS image** (the owner, 2026-11-15: "the port should be a part of the OS, the additional game files should live on the sd/cf card or internal flash storage if itll fit").
What it binds, from the points above:

- The engine and the game's logic are two programs in the image, the
  logic loaded by our loader at run time (point 2).
- Valve's SDK licence file, its copyright notice and disclaimers, and the
  GPL version 3 notice with where the source is, are in the OS itself
  (the About screen and the image's licence texts) (point 5).
- RetroPane OS stays free of charge while the port is in it (point 3).
- **The game's files are the user's own**, looked for **on the internal
  flash first, then the SD card, then the CF card** (the owner, 2026-11-15:
  "look for them on internal flash first, if you give me the answer on
  weather itll fit or not").
- **Will they fit the internal flash? As far as is known, no; not
  measured.** The X51v's flash is a DiskOnChip G3 of about 256 MB in all
  (design-brief.md; its exact size is still to be confirmed on the device),
  shared with the OS, and RetroPane cannot read or write it yet (R21). A
  Half-Life `valve` folder is several hundred MB (from memory: the main
  `pak0.pak` alone is around 400 MB or more; **to be measured on the
  owner's own copy**, which is not on this PC yet). So the whole game on
  the flash would need its files made smaller (sounds and textures packed
  again, a project of its own, unproven) or only part of it there. What
  does fit on the flash whatever the answer: saves, settings and key
  bindings, which is why it is searched first.

## Can it be part of the base OS with no card at all? (the owner, 2026-11-15)

"ths os is to be treated as if there is not any expandable storage, so we
need to make sure all base stuff fits into the internal storage, i want
this port to be part of the base os experience but i need you to tell me
if that possible first."

And why (the same day): "I want to treat it like this because not everyone
will have an SD/cf card, if they do i want it to unlock more capabilities
but I still want all of the base stuff to work if not."

And, so it is not read literally: "dont treat it as if there is no
expandable storage, treat it so that we will count for no expandable
storage to make sure all base stuff works, and that we will offer more
functionality if there is expandable storage."

**What that means for this port** (my reading, his to correct): the port
is base, so it is in the image and starts with no card; a card is what
may unlock the full game if the user's copy cannot be made to fit the
flash. With no card and no game files on the flash, the port must still
behave as a finished part of the OS: say plainly that it needs the user's
Half-Life files, how big they are, where they can go (the flash over USB
if they fit, a card otherwise) and never show a crash or an empty screen.

What is known, and what is not:

- **The port itself: yes.** The engine and the game's logic are programs
  of a few MB (an estimate; not built yet), inside the OS's own budget on
  the flash.
- **The game's files can never be in the OS image**, whatever their size:
  they are Valve's, and each user must bring a copy he owns. So "part of
  the base OS" can only mean: the port is there from the first start, and
  it plays once the user has put his copy on the device.
- **Unchanged, that copy does not fit the internal flash.** The flash is
  about 256 MB in all (Dell's figure, the reviews of 2005 agree; its exact
  size and what the OS leaves free are not measured, and RetroPane cannot
  read or write the DiskOnChip yet, R21). A Half-Life `valve` folder is
  several hundred MB (about 500 MB by one guide of low quality; not
  measured: the owner's copy is not on this PC).
- **Made smaller, it may fit; unproven.** A PC tool that takes the user's
  own copy and packs it for the X51v (sounds at the rate and in the
  compression the device plays, textures at the size a 640 x 480 screen and
  16 MB of video memory can show, what single-player never loads left
  out) is the road nobody else needed. How small it gets is a
  measurement: the owner's `valve` folder broken down by kind (maps,
  sounds, models, textures), each packed, against the flash left after
  the OS. Until then neither "it fits" nor "it cannot" is honest.
- **So, today:** a base OS with the port in it is possible; the whole game
  on the internal flash with no card is **not known to be possible and not
  shown impossible**. It needs three things first: his `valve` folder (to
  measure), the DiskOnChip read and written (R21), the flash's free room
  after the OS.

## Hidden until a card brings the game (the owner, 2026-11-15, later)

"if hl game files wont naturally fit into the internal storage, I dont want
there to be any extra steps besides drag and drop, i want to hide any
mention of the port on the native os until an expandable memory card is
inserted onto the device."

- **No extra steps:** the user's `valve` folder is copied onto a card as
  it is (a card reader, or the Axim as a drive over USB). **No packing
  tool** (it would be a step): if the files do not fit the flash as they
  are, a card is where they go. The flash is still looked in first, so a
  copy that is there plays.
- **Hidden with no card:** nothing in the Start menu, Settings or anywhere
  else names the port; the programs are in the base image all the same.
  The licence notices stay in About (the code is shipped either way).
- **Shown on any card (ruled, 2026-11-15).** I proposed showing the entry
  only when a `valve` folder is found; he overruled it: "No I want them to
  be able to make a valve folder on the card." So the entry appears
  whenever a card is in. Opened with no game files, it is a finished
  screen, not a dead one: it says the game's files are the user's own,
  makes the `valve` folder on the card for him, and says to drop his
  Half-Life files into it from a PC. (My reading, his to correct: the
  folder is made when he opens the entry, never by a card merely going in,
  so nothing is written to a card unasked.)
- **A pop-up, his words (2026-11-15):** "when clicking on the icon that
  shows up there should be a pop up explaining about where and how to move
  the game files to where they should be." So the screen above is a
  pop-up over the Start menu, in the OS's own glass look: where the files
  go (the `valve` folder on this card, by its name as the PC shows it),
  how (a card reader, or the Axim as a drive over USB; drag the folder's
  contents in), and that the game starts from the same icon once they are
  there. Mock-ups for him before it is built.
- **Cost:** small. The shell already sees a card arrive and leave and
  loads programs when they are opened; one look for a folder at that
  moment decides one Start-menu entry.

## Where the files go: `Ports\valve` (the owner, 2026-11-15)

"lets have all of the port files be housed under the SD/CF root file in a
'Port' folder then 'valve' folder for this example so I:\Ports\valve is
where i put this valve folder just now ... doom will be I:\Ports\doom
... so on and so forth."

So every port's files sit in one folder, `Ports`, at the card's root, one
folder a game inside it (docs/design/porting-guide.md). For Half-Life the
`valve` folder the pop-up makes and names is `Ports\valve`; the search
order (flash, SD, CF) is unchanged. He wrote "Port" once and made the
folder as `Ports`: the folder he made is the name used.

## His copy, measured (2026-11-15, read from `I:\Ports\valve`, nothing written)

**509.2 MiB in 4,262 files: it does not fit the internal flash** (about
256 MB in all), so Half-Life plays from a card, as his ruling above
already allows. By kind: maps 213.8 MiB, pictures 59.5 (`.tga`), sounds
59.0 (`.wav`), music 50.7 (`.mp3`), texture files 44.1 (`.wad`), models
18.8, sprites 15.6. By place: `maps` 216.5, `media` 60.9, `sound` 59.1,
`halflife.wad` 36.2, `gfx` 28.8, `resource` 24.4. It is a Steam copy (the
music is MP3, there is no `pak0.pak`, and it carries Linux game libraries
our port does not use, 16.5 MiB). Two things follow: **its music needs the
MP3 decoder**, which is left out of the build today and is a download
(R256); and the maps alone are most of it, so the read speed of the card
is what decides loading times. The card has 27 GB free.

## The choices as they were put

- **A. In the base OS image** (what he asked for: the showcase, there from
  the first start). The engine and the game's logic as two separate
  programs in the image; Valve's licence text and the GPLv3 notice on the
  About screen; RetroPane stays free of charge while it ships them. The
  user adds the `valve` folder.
- **B. A pack**, downloaded with the other ports: the same two programs,
  the same notices, kept apart from the OS image, so the "only for free"
  term binds the pack alone.
- **C. Both:** in the image the owner builds for himself and shows, a pack
  for everyone else.

What would overturn this reading: a written statement from Valve or from
Xash3D's maintainers on embedding; a lawyer's reading of the SDK licence's
"modified Valve game" (whether a port to a new device is one); a newer
SDK licence text than the one in hlsdk-portable.
