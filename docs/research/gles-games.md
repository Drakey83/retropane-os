# GLES games for the X50v/X51v (catalogue)

What each OpenGL ES game for the Axim's Intel 2700G actually calls, read
from its own executable's import table (no code run, no code copied). This
sizes the binary GLES layer for closed games (ADR-0026 D7, amended
2026-10-15): we implement the union of what owned games import, nothing
speculative. Open-source engines get native ports instead (D7 layer 3).

The wider library (every title, emulator and demo with evidence of 2700G
3D, including games the owner doesn't have, and what was checked and
excluded): [`3d-games-catalogue.md`](3d-games-catalogue.md).

Method: unpack the installer or read the installed files, list the PE
imports (`libGLES_CL.dll`, `libGLES_CM.dll`, `libEGL.dll`, `gx.dll`),
resolve ordinals against Intel's `libGLES_CL.dll` exports from the
owner's ROM (`local/drivers-a08/`). Files stay in `local/apps/` (not in
git).

## Findings that shape the GLES layer

- **Games import Intel's GLES DLL by ordinal number**, not by name
  (Enigmo: all 54 functions by ordinal). Our `libGLES_CL.dll` must export
  every function at **Intel's ordinal**, or such games fail to load.
- Intel's DLL also exports the EGL calls (Enigmo takes `egl*` from
  `libGLES_CL.dll`, not a separate `libEGL.dll`); Quake 3's port ships its
  own `libGLES_CM_NoE.dll` and `libEGL.dll` (the float Common profile).
- Compressed textures (`glCompressedTexImage2D`, PVRTC on this chip) are
  in use: the 3D driver needs PVRTC (TEX0 formats 5/7 are still undecoded,
  ADR-0024).

## The games

| Game | Source | Binary | GLES DLL | Notes |
|---|---|---|---|---|
| Enigmo (Aspyr, 2004) | closed | Axim X50 Game CD (`F:\Game\Games\EnA01En.exe`): installer resource 99/2, zlib, a second installer with a Windows CE CAB inside (68 files; game `00Enigmo.001`, old-style `000` setup script) | `libGLES_CL.dll` (fixed point), 54 imports by ordinal | also `gx.dll` (GAPI input only: `GXOpenInput`, `GXGetDefaultKeys`, `GXCloseInput`), FMOD CE for sound (`fmodce.dll`), `aygshell` |
| Geo Rally | closed | installed on the owner's card | `libGLES_CL.dll`, 48 | |
| Solterra | closed | installed on the owner's card | `libGLES_CL.dll`, 50 | found 2026-10-15, unknown to us before |
| Stuntcar Extreme (Fathammer) | closed | Axim X50 Game CD (`SCERC0B.exe`, InstallShield: not unpacked) | unknown | not installed |
| Virtual Pool Mobile | closed | installed build is v1.78, **software rendered** (GAPI, no GLES); Dell's CD trial is GAPI too | — | the GLES build is its own program, next row |
| Virtual Pool Mobile GLES (Celeris) | closed | installed on the owner's SD card (`\Program Files\Games\Virtual Pool Mobile GLES\VPMobileGLES.exe`, 760,832 bytes, built 2005-09-06) | `libGLES_CL.dll`, 48 by ordinal | `gx.dll` for input and suspend/resume only; `aygshell`, `ceshell`; no DLLs of its own (sound and Ogg video built in). Data in its own `.ark` packs |
| Call of Duty 2 Pocket PC (Activision) | closed | installed on the owner's SD card (`\Program Files\COD2\cod2.exe`, 350,720 bytes, built 2006-12-06; the program calls itself `cod2_gles`) | `libGLES_CL.dll`, 40 by ordinal | `gx.dll` incl. `GXOpenDisplay` (input, suspend/resume); data in `.pk2` packs; a `LOWRES_TEXTURES` option |
| Tony Hawk's Pro Skater 2 (Activision, 2005) | closed | installed on the owner's SD card (`\Program Files\THPS2\THPS2.exe`, 1,669,936 bytes, built 2005-10-27) | `libGLES_CL.dll`, 45 by ordinal | ships `fmodce.dll` (FMOD 3, 20 calls), `zlibce.dll`, and `x0__ar10dll.dll` (Intel compiler's float/maths runtime: our CE layer loads it like any DLL); `MMTimer.dll` timers; a Direct3D 8-style texture layer inside (`IDirect3DTexture8` names) |
| Toy Golf | closed | owner looking for it | to read | |
| Quake Mobile | Quake engine (GPL) | installer only on the card, not installed | to read | |
| DoomGLES, Doom2GLES, HereticGLES (Kokak) | open engines (GPL) | installed on the card | `libGLES_CL.dll`, 59 each | native ports planned (brightness control first)  Textures come from `.pvr` files whose header picks the format: on the card 443 PVRTC 4 bpp + 202 RGBA 5551, none 4444 (checked 2026-10-17): see design/display-calibration.md, Games |
| Quake III Arena (Q3CE) | open engine (GPL) | installed on the card | `libGLES_CL.dll`, 57 | its own `libGLES_CM_NoE.dll` (+ `libEGL.dll`) looks like a software fallback (it imports JIT helpers: `eglAllocExecutable`...); on the 2700G Q3CE uses Intel's DLL. Native port planned |
| Pocket Cube (Cube engine) | open engine | installed on the card | `libGLES_CL.dll`: game and server 35 each, its SDL port 14 (EGL) | |

## The union: 77 functions (owner's cards, 2026-10-15, +1 on 2026-10-18)

Ten programs scanned (the games above with GLES imports). Together they
import **76** functions from `libGLES_CL.dll`, all by ordinal: the binary
GLES layer's whole first target.

`eglChooseConfig eglCreateContext eglCreateWindowSurface eglDestroyContext
eglDestroySurface eglGetConfigAttrib eglGetConfigs eglGetCurrentContext
eglGetDisplay eglGetError eglInitialize eglMakeCurrent eglSwapBuffers
eglTerminate glActiveTexture glAlphaFuncx glBindTexture glBlendFunc
glClear glClearColorx glClearDepthx glClientActiveTexture glColor4x
glColorPointer glCompressedTexImage2D glCullFace glDeleteTextures
glDepthFunc glDepthMask glDepthRangex glDisable glDisableClientState
glDrawArrays glDrawElements glEnable glEnableClientState glFinish glFogx
glFogxv glFrontFace glFrustumx glGenTextures glGetError glGetIntegerv
glGetString glHint glLightModelx glLightModelxv glLightx glLightxv
glLineWidthx glLoadIdentity glLoadMatrixx glMaterialxv glMatrixMode
glMultMatrixx glNormalPointer glOrthox glPixelStorei glPolygonOffsetx
glPopMatrix glPushMatrix glReadPixels glRotatex glScissor glShadeModel
glStencilFunc glStencilOp glTexCoordPointer glTexEnvx glTexImage2D
glTexParameterx glTexSubImage2D glTranslatex glVertexPointer glViewport`

Used by one program only: `eglGetCurrentContext glDepthRangex
glLightModelx glStencilFunc glStencilOp glTexSubImage2D`.

**2026-10-18: Virtual Pool GLES, Call of Duty 2 and THPS2 added (77).**
Their 62 distinct functions are all in the 76 except **`glScalex`**
(Virtual Pool and THPS2), so the union is **77**. Every ordinal they import
resolves to a named export of Intel's `libGLES_CL.dll` (ROM A08): the
"keep Intel's ordinals" rule holds for them too. `glDepthRangex` now has a
second user (THPS2). No new DLL beyond what the table names; none of the
three loads GLES dynamically for its 3D calls.

Also found: two games draw through DirectDraw (`ddraw.dll`) instead: CE
personality 2D work, not GLES. Most of the owner's other games use GAPI
(`gx.dll`) or GDI.

## DoomGLES's GLES calls (59): the compat floor (the owner, 2026-10-23)

From the real DoomGLES Windows Mobile binary (`DoomGLES.exe`, release
B07: PE32 for Windows CE, ARM (machine `0x1C2`), built 2008-05-04; read
only, from the owner's download, nothing kept in the repo): **59 imports
from `libGLES_CL.dll`, all by ordinal**, every one a named export of
Intel's `libGLES_CL.dll` (ROM A08), all 59 within the union of 77. The
owner's ordinal list and the import table agree exactly:

1 `eglChooseConfig` 3 `eglCreateContext` 6 `eglCreateWindowSurface` 7 `eglDestroyContext` 8 `eglDestroySurface` 9 `eglGetConfigAttrib` 14 `eglGetDisplay` 15 `eglGetError` 17 `eglInitialize` 18 `eglMakeCurrent` 22 `eglSwapBuffers` 23 `eglTerminate` 26 `glActiveTexture` 27 `glAlphaFuncx` 28 `glBindTexture` 29 `glBlendFunc` 30 `glClear` 31 `glClearColorx` 32 `glClearDepthx` 34 `glClientActiveTexture` 35 `glColor4x` 37 `glColorPointer` 38 `glCompressedTexImage2D` 42 `glCullFace` 43 `glDeleteTextures` 44 `glDepthFunc` 45 `glDepthMask` 47 `glDisable` 48 `glDisableClientState` 49 `glDrawArrays` 50 `glDrawElements` 51 `glEnable` 52 `glEnableClientState` 53 `glFinish` 59 `glGenTextures` 60 `glGetError` 62 `glGetString` 63 `glHint` 65 `glLightModelxv` 66 `glLightx` 67 `glLightxv` 69 `glLoadIdentity` 73 `glMaterialxv` 74 `glMatrixMode` 75 `glMultMatrixx` 78 `glNormalPointer` 79 `glOrthox` 80 `glPixelStorei` 83 `glPopMatrix` 84 `glPushMatrix` 86 `glRotatex` 90 `glShadeModel` 94 `glTexCoordPointer` 95 `glTexEnvx` 97 `glTexImage2D` 98 `glTexParameterx` 100 `glTranslatex` 101 `glVertexPointer` 102 `glViewport`.

This is the floor the compatibility layer has to meet for DoomGLES (what
the binary needs from a Windows Mobile-compatible `libGLES_CL`); the
native driver isn't bound by Dell's limits. For the clocks' work: it
draws with `glShadeModel` and lighting (`glLightx`, `glLightxv`,
`glLightModelxv`, `glMaterialxv`), so Gouraud shading is on its path
(docs/hw/2700g.md, run GC2).

**HereticGLES (release B05b, built 2008-02-19, ARM; read the same way):
the same 59 ordinals, exactly.** Its data (`OpenGLES_Heretic_Datas`)
adds `.md2` models (Quake 2's format: the items, ammunition and
artifacts) beside the `.pvr` textures, drawn vertex-lit: Gouraud again.

## Enigmo's GLES calls (54)

EGL: `eglChooseConfig eglCreateContext eglCreateWindowSurface
eglDestroyContext eglDestroySurface eglGetConfigs eglGetDisplay
eglGetError eglInitialize eglMakeCurrent eglSwapBuffers eglTerminate`

GL: `glActiveTexture glAlphaFuncx glBindTexture glBlendFunc glClear
glClearColorx glClientActiveTexture glColor4x glColorPointer
glCompressedTexImage2D glCullFace glDepthMask glDisable
glDisableClientState glDrawArrays glDrawElements glEnable
glEnableClientState glFrontFace glGenTextures glGetError glGetIntegerv
glLightModelx glLightModelxv glLightxv glLoadIdentity glLoadMatrixx
glMaterialxv glMatrixMode glMultMatrixx glNormalPointer glOrthox
glPixelStorei glRotatex glScissor glTexCoordPointer glTexEnvx
glTexImage2D glTexParameterx glTranslatex glVertexPointer glViewport`

So one closed game already needs: fixed-function lighting, two texture
units, PVRTC upload, blending, alpha test, culling, scissor, depth mask,
matrices: the T5-T7 device tests (blending, filtering, depth) are on its
critical path.

## The owner's second batch (2026-10-18 evening, read-only)

105 installed programs (`\Games`, `\Program Files`) and 66 installers
(CABs unpacked into a scratch folder; cracked or keygen files and folders
skipped, as always) scanned for their graphics imports. **No new GLES
program**: GLES remains Enigmo, Geo Rally, Virtual Pool GLES, Call of
Duty 2 and THPS2 among the owner's games. **GAPI** (`gx.dll`, direct
frame-buffer drawing) in about 30 (Tomb Raider, Doom, Pocket UFO, Zuma,
Bejeweled 2, Arvale 2, Sky Force, ...); **DirectDraw** in two (Galaxies,
`cod6`); the rest draw through GDI. Tomb Raider (2002) predates Pocket PC
GLES: GAPI only. For RetroPane this makes the GAPI path (ADR-0026's
`GETGXINFO` answer) the one most of the library needs.

## Texture formats, checked on the owner's cards (2026-10-18, read-only)

- **DoomGLES** (`\Games\Pocket Doom\OpenGLES` on the CF card): 645 PVR v1
  files, 443 PVRTC 4 bpp, 202 RGBA 5551, none RGBA 4444
  (design/display-calibration.md, Games).
- **Enigmo** (`\Program Files\Enigmo\Data`, from Dell's Axim game CD):
  Pangea's own packs (`.bg3d` models, `.sprites`), big-endian. Their format
  fields name **PVRTC RGBA 4 bpp** (`0x8C02`) for the models
  (`Global.bg3d` 62, `Droplets.bg3d` 24) and most interface art (`Infobar`
  98), and plain **RGBA** for the font (110) and small sprites (likely:
  fields counted, the pack layout not decoded). `Enigmo.exe` uploads
  `0x8C02` through `glCompressedTexImage2D` (import ordinal 38) and the
  rest through `glTexImage2D` (ordinal 97) with the format and type from
  the data. So, as with DoomGLES, the muddy parts are compressed in the
  game's own data: no driver undoes PVRTC; better textures need new data.
- **Virtual Pool Mobile v1.78** as installed: GAPI (`gx.dll`), no GLES.
- **Virtual Pool Mobile, the copy the owner installed as "the GLES
  version" (2026-10-18; `VPMobile.exe` 696,320 bytes, built 2005-05-22, no
  version resource):** GAPI (`gx.dll`) only; it doesn't import
  `LoadLibrary`, and its folder holds no DLLs: it can't use GLES at all.
  It is **Dell's own bundled copy** (the companion CD's
  `EXTRAS\DEVICE\VirtualPoolMobilepmdelltrial.exe`, a trial, 2005-05-24),
  and it is fast on the X51v because of **Dell's GAPI**: Dell's `gx.dll`
  (8.7 KB) asks the display driver for escape `0x20000` (`GETGXINFO`,
  through `ExtEscape` with `QUERYESCSUPPORT` first), which returns a
  132-byte description of the 2700G frame buffer (address, stride, format),
  and uses `ChangeDisplaySettingsEx` for rotation. The game draws in its own
  software engine straight into that frame buffer; the 2700G's 3D core
  isn't used. **For RetroPane's CE personality:** answer `GETGXINFO` with
  our frame buffer and support the rotation, and every GAPI game gets the
  same direct path.
- **A GLES Virtual Pool did exist:** Celeris ran a beta of an **OpenGL ES
  version for the Axim X50v** in 2005, with the Xbox and PS2 releases'
  graphics (<https://www.eyeonmobility.com/2005/virtual-pool-beta-for-dell-axim-x50v/>;
  also listed among 2700G games: <https://www.etcwiki.org/wiki/Intel_2700g_games>).
  The owner knows the game as 3D-accelerated; the Dell CD's trial is not
  that build. Next: the owner looks for the GLES build (the beta or a later
  release, possibly the "v1.29" on the owner's list); its imports confirm
  it in a minute.

- **Found (2026-10-18): the GLES build is on the owner's SD card**
  (`VPMobileGLES.exe`, row above), and it is a real `libGLES_CL.dll`
  program. Its textures (read-only, from the card's `.ark` packs and the
  program's upload code): the loader knows four formats, RGB 565, ARGB
  4444, PVRTC 4 bpp RGB and PVRTC 4 bpp RGBA (`glCompressedTexImage2D`
  `0x8C00`/`0x8C02`). The six pool rooms hold **546 PVRTC 4 bpp opaque +
  61 PVRTC 4 bpp with alpha** (4 bits per pixel measured from the sizes);
  the interface pack `res.ark` holds **139 16-bit opaque + 24 16-bit with
  alpha**, which the upload code can only send as RGB 565 and ARGB 4444.
  Both in Celeris's own container (`0xABCD0001` rooms, `0xABCD0002`
  interface: 52-byte header, width, height, a type 3 opaque / 4 alpha).
  Movies are Ogg-in-AVI decoded on the CPU.
- **Call of Duty 2** (`.pk2` packs: tagged records, a name, zlib data;
  images are chunked `HEAD` + `DATA`, 16 bits per pixel): every texture
  goes up through `glTexImage2D`, never compressed. The upload code picks
  by the image's kind: opaque **RGB 565**, with alpha **ARGB 4444**; one
  more upload is 8-bit `GL_LUMINANCE` (128 or 256 wide). 492 images in the
  three packs (the 16 `IMMP` records in `maps_release.pk2` not decoded).
- **Tony Hawk's Pro Skater 2** (`All.pkr`: Neversoft's `PKR2` pack, 21
  folders, 3,274 files, zlib or stored): **927 `.PVR` textures, all PVRTC
  2 bpp** (legacy 44-byte header, type `0x0C`, twiddled, no mipmaps; 32x32
  498, 64x64 408, 128x128 15, 16x16 6), uploaded as `0x8C03` (PVRTC RGBA
  2 bpp). Uncompressed textures go through the game's Direct3D 8-style
  layer and up as **RGBA 5551** (`GL_RGBA` + `0x8034`) from its 1555
  formats; the pack's 1,477 `.BMP`s (931 8-bit palette, 533 24-bit, 13
  4-bit) are converted on the CPU before that.
- **What this means for the texture rule** (design/apps-and-packs.md §6a:
  RGB 565 dithered or better, never ARGB 4444 colour): legacy games keep
  their own data, so ARGB 4444 arrives from Virtual Pool's 24 interface
  pictures and Call of Duty 2's alpha textures, and PVRTC 2 bpp (the
  muddiest mode) is all of THPS2's world. Our GLES layer can't add detail
  that isn't there; what it can do is the same as for DoomGLES: hand the
  game what it asks for, apply the per-game picture curve, and leave
  better textures to new data packs (owner's choice per game).
