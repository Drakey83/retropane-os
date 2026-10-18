# GLES games for the X50v/X51v (catalogue)

What each OpenGL ES game for the Axim's Intel 2700G actually calls, read
from its own executable's import table (no code run, no code copied). This
sizes the binary GLES layer for closed games (ADR-0026 D7, amended
2026-10-15): we implement the union of what owned games import, nothing
speculative. Open-source engines get native ports instead (D7 layer 3).

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
| Virtual Pool Mobile | closed | installed build is v1.78, **software rendered** (GAPI, no GLES); the GLES build v1.29 is a CAB on the owner's CF card, not installed | (v1.29) to read | |
| Toy Golf | closed | owner looking for it | to read | |
| Quake Mobile | Quake engine (GPL) | installer only on the card, not installed | to read | |
| DoomGLES, Doom2GLES, HereticGLES (Kokak) | open engines (GPL) | installed on the card | `libGLES_CL.dll`, 59 each | native ports planned (brightness control first)  Textures come from `.pvr` files whose header picks the format: on the card 443 PVRTC 4 bpp + 202 RGBA 5551, none 4444 (checked 2026-10-17): see design/display-calibration.md, Games |
| Quake III Arena (Q3CE) | open engine (GPL) | installed on the card | `libGLES_CL.dll`, 57 | its own `libGLES_CM_NoE.dll` (+ `libEGL.dll`) looks like a software fallback (it imports JIT helpers: `eglAllocExecutable`...); on the 2700G Q3CE uses Intel's DLL. Native port planned |
| Pocket Cube (Cube engine) | open engine | installed on the card | `libGLES_CL.dll`: game and server 35 each, its SDL port 14 (EGL) | |

## The union: 76 functions (owner's cards, 2026-10-15)

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

Also found: two games draw through DirectDraw (`ddraw.dll`) instead: CE
personality 2D work, not GLES. Most of the owner's other games use GAPI
(`gx.dll`) or GDI.

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

