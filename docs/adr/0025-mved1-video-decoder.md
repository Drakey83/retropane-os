# ADR-0025: Bringing up the MVED1 video decoder

- **Status:** Proposed (draft, 2026-10-14; needs the owner's decision and
  the owner's go for every device run)
- **Date:** 2026-10-14

## In plain words

The 2700G has a third engine besides the 2D engine and the 3D core: a
**video decoder** ("MVED1", called M24 by the drivers). Windows Mobile uses
it to play WMV video. DESIGN.md G1 asks for native drivers for all
hardware, and nothing about this unit was public. A static look at
Windows Mobile's two video DLLs (docs/hw/2700g.md, "The MVED1 video
decoder") has now found where it is, how it is switched on, how it is fed,
and how it says it's done. The split is the classic one for 2005: **the
CPU reads the video file and works out each block; the chip does the
heavy arithmetic** (the 8 × 8 inverse transform and the motion
compensation that copies and blends blocks from the previous picture),
writing finished pictures straight into the 2700G's memory, where the
display's video plane shows them without a copy.

This ADR proposes **one short test image, `rp-mved`**, that switches the
unit on and checks the findings one at a time, ending with a picture the
chip built from commands we wrote. Its most useful single answer is
whether the chip's 8 × 8 transform is the MPEG one: if it is, the same
unit can play **MPEG-4 (Xvid/DivX) and MPEG-2**, not only Windows Media.

## Context

- **Found statically (docs/hw/2700g.md, 2026-10-14):** registers at
  `0x0FFE3000`; two write-only 4 KiB command windows at `0x0FFF0000`
  (commands) and `0x0FFF1000` (coefficients or residuals), both inside the
  register window RetroPane already maps; the clock in `M24CLK` (system
  `0x050`, bit 0 enable; Windows Mobile idles at `0x2`, off; on is most
  likely `0x3`); reset `0x00` = 1 then 0, acknowledge `0x88` = 3; FIFO
  levels in `0x50`; status `0x80` (bit 1 frame done, bit 0 conversion
  done), acknowledged by writing the bits to `0x88`; capabilities in
  `0xA0`. Frames are planar 4:2:0, stride a multiple of 256, the two
  chroma components side by side in one plane. Addresses and strides are
  written divided by 8. A frame-conversion job (4:2:0 to YUY2 or to a
  3-byte format) needs only registers.
- **The work split:** CPU: parsing, variable-length decoding,
  dequantisation, vector and DC/AC prediction, WMV9's small transforms
  (8 × 4, 4 × 8, 4 × 4), most likely the loop filter. Chip: inverse scan
  and the 8 × 8 inverse transform from run-level pairs, motion
  compensation, reconstruction. Completion is polled; no interrupt is used.
- **Reads that are safe:** only `0x50`, `0x80`, `0xA0` (the only ones
  Dell's library reads). Reading anything else in the 2700G's register
  block may hang the bus (four WM probes froze, docs/hw/2700g.md).
- **Not known:** the meaning of `0xA0`'s bits, the chroma order, the
  vector units, the transform's arithmetic, the scan each of the four
  scan modes selects, timings, whether reads with the clock off hang.
- **Non-negotiables (owner):** native drivers, no Windows Mobile DLLs at
  runtime, no device run without the owner's go, security noted in every
  ADR and parser, emulator gaps listed.

## Decision (proposed)

Build `rp-mved`, a loader test image in the style of `rp-mbx3d`, with four
stages. Each stage prints what it read, stops on the first surprise, has a
timeout on every wait, and puts back what it changed (`M24CLK` to the
value it found, the decoder reset). It draws its results on the screen
for the owner's photo and logs them over USB as before.

1. **S0, clock and identity (reads three registers).** Read `M24CLK`
   (expect `0x2`); set it to `0x3`; only then read `0xA0`, `0x50`, `0x80`.
   Expected: FIFOs empty (`0x50` levels 0), status 0, `0xA0` bits 0-1 set
   in some combination.
2. **S1, reset.** `0x00` = 1, `0x00` = 0, `0x88` = 3; read `0x50` and `0x80`
   again.
3. **S2, conversion (registers only, no command stream).** A 64 × 64
   test frame in 2700G memory well away from the visible plane (luma a
   gradient, the two chroma halves different constants), converted to
   YUY2 (`0x30`-`0x3C`, `0x48` = 1); poll `0x80` bit 0 (timeout 100 ms,
   timed); acknowledge; compare with a host-computed YUY2. This proves the
   clock, the address units and the completion bit, and shows the chroma
   order. Then mode 3 (3 bytes a pixel), to see what that format is.
4. **S3, motion compensation, no residual.** Frame size, reference and
   output (`0x20`, `0x24`, `0x10`, `0x14`, `0x90` = 0); for each macroblock
   of a 64 × 64 frame: set-up, position, the three-word block with a zero
   vector, for luma and chroma; `0x78000000`; poll `0x80` bit 1. Expected:
   the output equals the reference. Then vectors of 4 and 2 (one pixel and
   half a pixel if the units are quarter pixels) to learn the units and the
   interpolation. **S4, the transform:** with `0x90` = 1, intra blocks with
   one coefficient each (DC; then single AC coefficients at several runs,
   in each of the four scan modes), the output compared with two host
   references: WMV9's integer transform and the IEEE 1180 IDCT.

S0-S2 are one device run (the first test); S3 and S4 a second, after
reporting. A host-tested, pure-Rust command encoder (`rp-drv-mved`, as
`rp-drv-blit2d` and `rp-drv-mbx3d`) builds S3's and S4's words and the
reference pictures.

**Time box:** the image and encoder in one working session; each run's
write-up in one more. If S2 fails in a way the static findings can't
explain, stop and report before going on.

**Afterwards (its own ADR):** a video service that owns the unit (one
client at a time, as Dell's driver enforces), a decoder for whichever
formats S4 makes worthwhile, and the media player on the ROADMAP.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Every finding is checked by a stage that prints its evidence; the encoder is host-tested against reference pictures; each run decides the next |
| ✨ Modernize | A video service owns the unit; apps never touch its registers (on Windows Mobile the app's own process wrote them). If the transform is MPEG's, the unit plays formats WM never used it for |
| 🗄️ Legacy support | WMV9 (WM5/6 media files) is the format the unit was built for; the same path later backs DirectShow-style apps |
| ⚡ Better / faster | The chip does the per-pixel work: the CPU keeps only parsing, so the stylus and audio stay responsive during playback; frames land where the video plane scans them out, with no copy |

## Three questions

- **Clever or copied?** Copied: facts about the hardware only. Beyond WM:
  the MPEG-4/MPEG-2 question, a service instead of per-app register
  access, and the conversion job (unused by WM's decoder) measured as a
  possible YUV converter for photos and the wallpaper.
- **Size:** `rp-mved` is a test image (budget measured + ~10%, set when it
  exists). Frames live in the 2700G's 16 MiB, not system RAM. The 256-byte
  stride costs memory: a 320 × 240 frame takes a 512-byte stride, 180 KiB;
  three frames for playback are 540 KiB at 320 × 240 and 1.6 MiB at
  640 × 480 (stride 768).
- **Modern:** the CPU/accelerator split is how phone video worked until
  fixed-function H.264 blocks; H.264 itself is out of reach of this unit.

## Hardware limit or choice?

| Behaviour | Limit or choice | Evidence |
|---|---|---|
| Parsing and variable-length decoding on the CPU | Hardware: the unit takes run-level pairs, not bitstreams | command and data words (docs/hw/2700g.md) |
| WMV9's 8 × 4, 4 × 8, 4 × 4 transforms on the CPU | Hardware, most likely: the unit's header encodes the sizes, but Dell's decoder sends those as residuals; S4 can try one | (A) |
| Completion polled | Choice by Dell's library; whether the unit can interrupt is unknown | (A) |
| One client at a time | Choice (Dell's driver); a sensible one for a single unit, kept | (A) |
| Rotation through the 3D core | Choice (no rotation in the decoder or the video plane) | (A) |
| Only WMV formats use it on WM | Choice, as far as known; S4 decides whether MPEG is possible | (A) |

## Alternatives considered

1. **Software decoding only** (the PXA270 at 624 MHz with WMMX): MPEG-4 SP
   at 320 × 240 is feasible but uses most of the CPU, VGA is not, and the
   unit stays idle. Kept as the fallback for formats the unit can't help
   with (and as the reference the chip is tested against).
2. **The MBX 3D core for motion compensation** (each block a textured
   quad): hundreds to thousands of quads a frame, no inverse transform,
   and the 3D core is wanted for the interface. Rejected.
3. **The unit as Windows Mobile uses it (WMV only):** the minimum; what
   S0-S3 prove.
4. **The unit for MPEG-4 Part 2 and MPEG-2 as well** (recommended, if S4
   shows an MPEG-compatible transform): the same commands, a different CPU
   front end; a much larger library of playable files.

Loading `wmv9decoder2700G.dll` or `pvrvadd.dll` is ruled out (no WM DLLs
at runtime).

## Security

- **Video files are untrusted input.** The bitstream parser runs on the
  CPU, in Rust, with every length, count and vector checked; malformed
  streams are errors, never hangs; it is fuzzed on the host. (Checklist,
  docs/security.md.)
- **The unit is a bus master inside the 2700G's memory**, which also holds
  the visible screen. The driver computes every address and stride
  itself from validated sizes; vectors are clamped so no block reads
  outside its reference frame (what the chip does with an out-of-frame
  vector is unknown until measured); a garbled stream can then at worst
  produce a garbled picture in its own frames.
- **No app gets the registers.** On Windows Mobile every process using
  `pvrvadd.dll` had the decoder's registers and windows mapped into it;
  in RetroPane only the video service does.
- The test image is development-only and ships nowhere.

## Emulator

Not modelled (docs/hw/emulator.md row added). To model once S2/S3 pass:
the register file (`0x50` always empty, `0x80` set by jobs, `0x88`
clearing it, bus errors for other reads, as for the MBX), the conversion
job, and motion compensation without residuals, so `cargo xtask emu`
checks `rp-mved`'s pictures pixel by pixel.

## Consequences

- If S0-S2 pass, the unit is usable for colour conversion at once, and
  the decoder path (S3, S4) is a matter of the command words.
- If the transform is WMV9-only, MPEG-4 stays a CPU job and the unit
  serves WMV files.
- The 2700G's register map gains a third engine; `M24CLK` joins `MBXCLK`
  in the power service's clock votes.

## Open questions (for the owner)

1. Is video worth this now, or after the GLES work (ADR-0024)? The first
   run is short (S0-S2).
2. If both work, which first: WMV9 (Windows Mobile's own files) or MPEG-4
   (most video files people have)?
3. The go for the first device run, when the image is ready.
