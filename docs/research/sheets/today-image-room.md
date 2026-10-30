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
