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
| Stuntcar Extreme (Fathammer) | closed | same CD (`SCERC0B.exe`, InstallShield: not unpacked yet) | unknown | read from the owner's card once installed |
| Virtual Pool Mobile v1.29 (Celeris) | closed | owner's SD card (installed 2026-10-15) | to read | |
| Toy Golf, Geo Rally | closed | Dell X51v companion CD: owner looking for it | to read | |
| DoomGLES, Doom2GLES, HereticGLES (Kokak) | open engines (GPL) | `local/apps/` | `libGLES_CL.dll` (to confirm) | native port planned (brightness control first) |
| Quake III Arena (Q3CE, noctemware) | open engine (GPL) | `local/apps/Quake 3` | ships `libGLES_CM_NoE.dll` + `libEGL.dll` | native port planned |
| more on the owner's card | ? | installed 2026-10-15 | to read | some GLES titles were unknown to us |

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
