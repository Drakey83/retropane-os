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
- **The game's files are the user's own**, looked for on the SD card, the
  CF card, then the internal flash's storage. Whether a `valve` folder fits
  the internal flash is not known yet: to measure against the owner's own
  copy and the flash left after the OS (readiness line 7).

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
