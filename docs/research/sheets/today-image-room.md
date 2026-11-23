# Sheet: room in rp-today for settings, the keyboard and the radar, 2026-10-30

**What it decides.** How the UI test image `rp-today` makes room for the
Weather app's next steps (settings, the on-screen keyboard, the radar's
base map and sample frames, live touch): about 50-70 KB more.

**Measured (this build, `llvm-size`/`llvm-nm`):** 159 KB code + 734 KB
data = 892 KB loaded, against a 921 KB budget and the loader's 928 KiB
area (the end-address check of 8f7dca34). In the data: the wallpaper 460
KB (raw YUV 4:2:0, `include_bytes!`), the weather pictures 225 KB
(run-length packed, R129; the hero alone 140 KB), the fonts ~60 KB.

## 1. Original way (Windows Mobile)
The wallpaper a JPEG on the file system, decoded at every boot; icons ICO
resources in each program. No image-area limit: WM loads from its ROM
file system.

## 2. Dell's way (REA)
Not applicable: Dell shipped WM's way (no packed assets of its own found;
`imgfs-a08_*` hold BMP/ICO resources). Searched only the list of
`local/re/ghidra-c/` modules for an asset unpacker; none.

## 3. Modern way
PNG (a per-row predictor and deflate) for lossless pictures, WebP/AVIF
lossless where a decoder is affordable; assets streamed from storage.

## 4. The alternatives, and why ours wins
| | What | Wins | Loses |
|---|---|---|---|
| A. Raise the budget | | Nothing to build | The 928 KiB area is the loader's limit; 7 KB more at most |
| B. Fewer features in the test image | Settings or radar left out | | Not the owner's ask ("in full") |
| C. A JPEG wallpaper decoded at start (WM's way) | ~60 KB file, ~25 KB decoder | Smallest | Lossy: not the pixels the owner approved; slow on a CPU without FPU (~0.5-1 s) |
| D. Read assets from the SD card | | No image limit | Needs the storage service in the test image; the card is shared (tests write only their own folder) |
| **Ours: lossless packing, unpacked once at start into the RAM pool** | On the PC: each plane (Y, U, V; a picture's ARGB) run through PNG's Paeth predictor, then deflate. On the device: a small safe `no_std` inflate (fixed and dynamic Huffman, bounded by the output size) unpacks into the RAM pool at start, as R129 does for the icons | The same pixels as today, bit for bit (a host test compares); ~200 KB back for ~5 KB of code; the same decoder later takes radar frames and icon packs (one reviewed parser, security.md) | A few tens of ms at start (measured on the device); a parser of packed data (only our own build's bytes; still bounds-checked, fuzzed on the host) |

## 5. What overturns it
- **Unpacking over ~200 ms at start** on the X51v (run log line): keep
  the wallpaper raw and pack only the radar map and pictures.
- **Less than ~120 KB saved** (the PC packer prints it at build): JPEG (C)
  goes to the owner as the alternative, with photos of both.
- Host tests: unpacked bytes equal the original; damaged or short input
  stops cleanly with an error, never past the output; a fuzz run.

## Amendment, 2026-11-23 (R208): context modelling in place of PNG's way

The five app groups merged (66c6bb2f) put rp-today at 986,296 B, past its
933,888 B budget and the 950,272 B image area. Two cuts, nothing visible
given up; 917,824 B after them (16,064 B left), and the look check's
build (R237) 929,196 B, inside the budget again.

**The wallpaper, 138,807 -> 97,794 B, the same pixels.**
- Original / Dell: a JPEG on the file system (lossy, not the approved
  pixels); the wallpaper as a card file is ruled out by ADR-0018 ("anything
  the screen shows must not depend on the card") without the owner.
- Textbook (what this sheet chose first): PNG's Paeth filter and deflate.
  Our own deflate also measured 12.5 KB worse than `zlib -9` on it.
- Modern: JPEG-LS (LOCO-I), CALIC, FFV1, JPEG XL's modular mode: a
  median edge predictor and statistics kept per neighbourhood. JPEG XL's
  decoder is far too big for a resident image.
- Ours (`rp_inflate::plane`, `RPPL`): LOCO-I's median predictor, eleven
  contexts by local activity (log scale), an adaptive binary range coder
  (LZMA's, 11-bit probabilities), and two things fitted at build to this
  picture: the adaptation rate (tried 3-6, the smallest kept) and each
  plane's low bits that are always clear (the sunset's planes are 6-bit:
  coded without them, fewer decisions a byte). Measured: conditional
  entropy of the residuals ~98 KB, the coder 97,794 B; two more contexts
  (mantissa by activity, the last difference's size) gave under 100 B and
  were left out. About 2 KB of decoder.
- Cost: unpacking is 2.2x the old way's time on the PC (7.1 against 3.2
  ms); the X51v took 85 ms the old way. Not measured on the X51v yet
  (reminder row, device-run): the "wallpaper unpacked" line decides.
- **What overturns it:** over 200 ms on the X51v (this sheet's own rule)
  -> a faster binarisation or the old way back for the 39 KB.

**The card code built for size where it lands.** Code 665,320 -> 636,964 B
(the new decoder's ~2 KB in); data 320,976 -> 280,852 B. ADR-0005
rule 8 builds cold crates at opt-level `s` (rp-fat, rp-drv-sdcard), but a
generic instance is compiled at the opt-level of the crate that uses it:
rp-today's 3. Each generic function in rp-fat and rp-drv-sdcard, and in
rp-today's own card files (`kept`, `cardfiles`), now says
`#[optimize(size)]` itself (minsize: LLVM calls the EABI's unaligned
helpers, `__aeabi_uread4` and kin, now in rp-hal-pxa27x, the runtime every X51v image links; only mount, mkdir
and directory entries use them, the data moves by DMA). A test
(`generic_card_code_is_built_for_size_wherever_it_lands`) refuses an
unmarked one. Siblings checked: rp-drv-pxa-mmc's instances are 960 B;
rp-shell, rp-weather and rp-pim compile in their own crates.

Not taken: rp-today whole at `s` (-102 KB, but the 2D renderer's draw and
the graphics service's poll, the hot paths, shrink with it); the zone
names (Today's feed shows another zone's appointment in the device's);
`Debug` in log lines (run-log parsers read them).
