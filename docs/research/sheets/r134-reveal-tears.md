# Sheet: Reveal's torn refreshes (R134), 2026-10-30

**What it decides.** How Reveal, the entrance from the Today card,
changes the video window every frame without tearing. Run WG
(gpubench-0248/-0249) tore 15 and 10 of its 304 refreshes, and the owner
saw it ("can we chase down the tears though?"). Every other movement was
whole in every refresh: the fling (only `VSCADR` changes), Cascade
(`VSCTRL`, `SSSIZE`, `VSCADR`) and the 1-pixel turn (`VSADR` x, `VSCTRL`
width, `VSCADR`). **Reveal is the only one that also moves the window's
top, `VSADR` y.** No code and no device run until the owner's go.

## Rule 0

- **Windows Mobile and Dell (a finding):** never met it. Dell writes the
  video plane's registers only to switch it off and back on (REA #37,
  `0x211fe70`, `0x2120370`). It never moves or resizes the window per
  frame. Its one per-frame write, the `GSCADR` flip, goes in the VEVENT0
  interrupt at line 643 with `DUCTRL` = 1 (`0x0211d30c`).
- **The modern way:** atomic plane updates. Linux DRM's atomic commit
  writes a plane's place, size and address together into shadow registers
  that the hardware latches at vblank, so a frame never mixes old and new.
- **What the 2700G has toward that (R113, gpubench-0221):** `DSCTRL` bit
  24 (`UPDCNT`) *holds* plane writes (0 of 180 torn), but nothing we tried
  released them in the next refresh. What releases them is unknown.
- **Ours:** find the cause in one run, then use the cheapest clean way: a
  write order, a different write line, or bit 24 as a real shadow latch if
  its release is found. That would also give every future plane move a
  write that can't tear, at any line.

## The run (one image, unattended, ~2 minutes; `rp-gpubench` run WG-R)

Reveal's 8 grow-and-shrink cycles, each refresh judged by `DSIG` as run
WG did, in each way below. The first 16 torn refreshes of each way are
logged with their step (k from and to), the line each write landed on,
and which registers changed.

| Way | What | Answers |
|---|---|---|
| A | as run WG (the control): `VSCTRL`, `VSADR`, `SSSIZE`, `VSCADR` at lines 643-2 | the tear reproduced, and which steps |
| B | `VSADR` last | the order |
| C | `VSADR` first | the order |
| D | all four at lines 640-642 (the blanking's start) | whether 643-2 is too late for `VSADR` y |
| E | y alone moving (x, size and start fixed) | whether y alone tears |
| F | the height alone moving at Reveal's y steps (y fixed) | that it's y and not the size |
| G | `DSCTRL` bit 24 set, the four written, then each release R113 didn't try: bit 24 cleared in the blanking; `DUCTRL` = 1 twice; `DUCTRL` = 0 then 1 | a shadow latch for every plane move |

Also in the same image (V3 ruled ARGB4444 + dither, the owner,
2026-10-30): **way H**, the turn to the pixel and Reveal at ARGB4444,
where the start moves in 8-pixel steps and the window by 0-7 pixels, both
judged as above.

**Stop rule (written first):** one run. If any way of A-F is clean in all
304 refreshes and A tears, that way ships in the page layer. If only G is
clean, its release becomes the driver's plane write. If none is clean,
Reveal moves its top by a different means: the window's top kept at the
tabs, the clear rows above the card read from the picture's 120-line
margin. That costs nothing per frame and needs no y write, but the card's
top edge then comes from the picture, not the window. That is drawn as
mock-up frames for the owner before any device run.

**What overturns it:** tears that follow the step and not the write
(the same k in every way). Then it's the picture, not the timing: the
torn refreshes are photographed (the owner) and compared with each
state's reference.

## Run on the X51v (2026-10-30, gpubench-0250)

Ways A-F and H: 0 torn of 304 each (H's turn 0 of 256); G1-G3: 288 of
304 each (bit 24 changes the picture and no release shows the held
state; the owner saw the tearing at steps 9 and 10). By the stop rule's
own terms the control didn't tear, so no way "wins": the tear isn't the
write order, the line, or the top edge alone. What run WG had that this
run lacks is the lead. Run WG's Reveal now logs each torn refresh (its
step, the one before, the registers and the line of the write). One
more run of WG names the step if it tears again; if it doesn't, R134
closes as not reproduced, and the plane path's own Reveal is watched by
the owner when built. Details in `docs/hw/2700g.md` "Run WG-R".
