# Sheet: R113, plane flips written mid-scan (test, then a fix), 2026-10-29

**What it decides.** Whether our 2D flips tear today, and how every write
of a plane address (`GSCADR`, `VSCADR`) should be timed. W1 measured that a
`GSCADR` write takes effect at once; `rp-mbx-render` writes it whenever a
2D frame's fence lands (`Mbx2dRenderer::show_picture`, `flip_wait` `None`
everywhere). Not proven to tear on the X51v: a reading of code against W1.

## 1. Original way (Windows Mobile / Dell's product)
Flips only in the blanking, from the VEVENT0 interrupt (W2: 1,534 flips,
all on line 643).

## 2. Dell's way (REA)
`ddi_2700g`: the display ISR 0x0211d284 (SYSINTR 27, GPIO85) clears
`DINTRS` bit 2 and, with flips queued, runs 0x0211ce8c, which writes the
flip by 0x0211d30c: `GSCADR` = (old & 0xFF800000) | addr/16, then
0x0211d250: read `DUCTRL` (0x230C, discarded), `DUCTRL` = 1. VEVENT0 is
enabled in `DINTRE` (0x217C) only while flips are queued (0x0211cf88).
`DVECTRL` is never written: VEVENT0 stays at 643. `DSCTRL`'s update fields
(bits 16-19 `UPDWAIT`, 24 `UPDCNT`, 25 `UPDINTCNT`, 26 `UPDSYNCCNT`, Linux
mbxfb's names) are set per panel type by the mode set 0x02126c90; the
X51v's are 0 (`DSCTRL` 0x8000002A). REA log #3-#8.

## 3. Modern way
Display registers double-buffered in hardware and latched at vblank: the
driver writes any time, the hardware applies at the frame's start (DRM
atomic commits). Training knowledge, a claim.

## 4. The alternatives, and why ours wins
| | What | Wins | Loses |
|---|---|---|---|
| A. Do nothing | Flip when the fence lands | No work | One torn refresh per 2D flip, if W1 holds |
| B. Dell's | VEVENT0 interrupt flips the queued address | Proven (W2), no CPU wait | Needs the GPIO85 interrupt routed to the display host (our service is polled); a 51 us window (lines 643-644) |
| C. Poll VEVENT0's status before the write | Wait for the bit, then write | No interrupt plumbing | The CPU waits up to a refresh (16.6 ms) per flip, or the flip slips a refresh |
| D. Modern: a hardware latch | If `DSCTRL`'s update bits + `DUCTRL` = 1 hold the write to the next frame | Write at any line, no wait, no interrupt; fixes the class at the root | Untried; may do nothing on this panel type |
| **Ours** | Measure A first, then try D; take D if it latches, else B for flips (C until the interrupt is routed) | Fixes only what is proven broken, by the cheapest mechanism that works | One short device run |

## 5. What would overturn each, and the test
- **The test** (rp-gpubench stage): two whole pictures with known `DSIG`;
  (1) `GSCADR` written at lines 100, 320, 600 for 60 refreshes each, every
  refresh's `DSIG` read: torn = neither picture's. (2) The same with
  `DSCTRL` bit 24, then 25, then 26, then `UPDWAIT` 1, one at a time, each
  followed by `DUCTRL` = 1, `DSCTRL` restored to 0x8000002A after each.
  One setting per step, the owner watching (a 2700G display register, not
  a board bit; reversible, but the screen may blank until restored).
- **A overturned** if (1) shows no torn refresh: W1's finding was
  something else, R113 closes.
- **D chosen** if a setting moves every flip to the refresh boundary (no
  torn refresh, the new picture from the next refresh); **D overturned**
  if none does.
- **B over C** when the interrupt reaches the display host; until then C.

**Asked of the owner:** the go to build the stage; a separate go for the
device run.
