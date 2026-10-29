# Sheet: R20's proof run (test), 2026-10-29

**What it decides.** Whether the copy fallback (built 89ef87d3, the owner's
"Yes, copy variant") is tear-free and faster on the X51v, against the
two fixed fallbacks, before it is switched on. Also settles audit C93 (the
2D engine's write order inside a copy, which the copy's plan assumes).

## 1. Original way (Windows Mobile / Dell's product)
No tear test exists in the shipped drivers: Dell's code never reads the
display signature `DSIG` (REA log #13, #14, #21: `0x2184`, `0x184` in
`ddi_2700g`, `libGLES_CL`, `d3d_2700g`: no hit). Windows Mobile shipped
whatever the triple-buffered flip gave, unmeasured.

## 2. Dell's way (REA)
Dell avoids the question: full-screen GLES/D3DM renders off screen into
up to three pictures and flips in the VEVENT0 interrupt (`ddi_2700g`
0x0211d284 → 0x0211ce8c → 0x0211d30c: `GSCADR` = (old & 0xFF800000) |
addr/16, then `DUCTRL` 0x230C = 1; GLES `BackBuffers` 2, `VSyncLocked` 1,
0x022601d0). A windowed GLES surface with no back buffer renders straight
into the shown screen with no timing at all (0x0226d694; type-5 command
0x0211d1dc only counts a sync object). As a policy in this run Dell's flip
can't be tried as it is: rendering while screen 1 is shown breaks ADR-0034
rule 2. Its nearest legal form is option 1 (flip, copy back), which GC2DE
already measured.

## 3. Modern way
Scan-out CRCs per refresh, checked against a reference frame (the Linux
DRM CRTC CRC capture IGT uses for tear and flip tests), with flip
timestamps for latency. Training knowledge, a claim. Our `DSIG` is that
hardware: the display's per-refresh signature.

## 4. The alternatives, and why ours wins
| | What | Wins | Loses |
|---|---|---|---|
| A. Do nothing | Ship on the host model (22 tests, 11 mutants) | No device time | C93 unproven: a 2D engine that doesn't write top down would tear every copy; "proven on hardware" rule unmet |
| B. GC2DE's numbers only | Project, as the ruling did | No run | Projection only (23.5 / 26.6 ms); copy start inside the window never seen |
| C. Eyes only | Owner watches frames | Simple | Misses one-refresh tears; not counted |
| **D. Ours** | rp-gpubench run on GC2DE's harness at 149.50 / 2, the four game frames: three policies (copy fallback, blanking start, option 1) × one-off and back-to-back; every refresh's `DSIG` and screen 0's signature checked against the whole old/new pictures' | Counts tears, times to screen and next frame free, fps; failed/hung/undrawn counted (R96); C93 measured directly | ~10 min device time |

C93's part of D: the whole-screen copy begun at lines spread over its
window (557-637) and outside it (480, 520, 640), each refresh judged by
`DSIG`, plus rows' landing times sampled as GC2BZ did for the 3D core.

## 5. What would overturn each
- **Copy fallback (ours):** any torn refresh with the copy started inside
  its window, or rows landing out of top-down order → C93 fails: the copy
  goes back to a blanking start (ADR-0034's literal 640) until the order
  is known. Next frame free not below the blanking start's 27.7 ms → its
  only win is gone.
- **Blanking start:** tears on the checkerboard render from line 640
  (counted here) → it loses to the copy on tears too.
- **Option 1:** already beaten on next frame free (40.1 ms, GC2DE); it
  would win back only if the copy tears.
- **The test that tells them apart:** the same frames, the same asks,
  each policy back to back, judged by `DSIG` every refresh.

**Asked of the owner:** the go to build this run (rp-gpubench stage, its
host tests, the emulator check on rp-next-032); a separate go for the
device run itself.
