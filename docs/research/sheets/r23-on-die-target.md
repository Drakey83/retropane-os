# Sheet: is the on-die memory why screen 0 renders faster? (R23, test), 2026-10-29

**What it decides.** R114 (gpubench-0223) found no placement in the 2700G's
SDRAM changes a render's time; GC2DE found the textured frame 15% faster
into screen 0 (0xF000, inside the first 704 KiB) than into screen 1. The
public source calls those 704 KiB the 2700G5's on-die memory (R23). If the
target's memory is the cause, the copy fallback's renders (into screen 1)
could be faster only by rendering on die, which changes the memory map's
use of the on-die memory (screen 0, the lists on die, the planes).

## 1. Original way (Windows Mobile / Dell's product)
Windows Mobile's screen and GLES back buffers wherever the allocator gives
(REA #25-#31): no use of the on-die memory as such.

## 2. Dell's way (REA)
No sign of an on-die/off-die distinction in `ddi_2700g`'s allocator (one
arena, first fit, REA #30-#31). The datasheet (`local/datasheets/intel-2700g.pdf`)
to be read for the on-die memory's size and address in the same commit as
the result.

## 3. Modern way
Tile-based GPUs keep render targets in on-chip tile memory and write out
once; embedded GPUs with on-die SRAM put the hot target there. Training
knowledge, a claim.

## 4. The alternatives, and why ours wins
| | What | Wins | Loses |
|---|---|---|---|
| A. Assume it | Take the public source's 704 KiB as the reason | No run | A claim, not a measurement ("verify yourself") |
| **B. Ours** | Time the textured frame (and the other three) into targets wholly on die, ending exactly at 0xB0000, crossing it by 88, 216 and 304 KiB (15%, 36%, 51%), and wholly in SDRAM; lists in SDRAM for all (the on die kept free), the display's fetch off meanwhile; every render checked against the SDRAM one | The cause shown by a measurement: time rising with the share off die proves it, flat refutes it | One short run, the screen dark for it |

## 5. What would overturn it
- **On die overturned** if every placement times the same: then screen 0's
  speed in GC2DE came from something else (the lists on die beside it, or
  the display fetch), the next lead.
- **The test:** 7 places, 8 renders each, at 149.50 / 2.

**Asked of the owner:** the go (given 2026-10-29, "Yes, build it"); a
separate go for the run.

## Ran on the X51v, 2026-10-29 (gpubench-0224): proven
The textured frame 13.04 ms with its target wholly in 0x0-0xB0000, rising
3.7 us for every KiB of target past 0xB0000 to 15.29 ms wholly in SDRAM
(`docs/hw/2700g.md` "R23T"). The on-die memory is the cause of GC2DE's 15%,
and its edge is 0xB0000. What to do with it (screen 1's fallback frames
render in SDRAM; only ~104 KiB of the on die is left beside screen 0) is
the owner's: R116.
