# A modern screen for the Axim X51v? (desk research, 2026-10-17)

> **Status: parked, a long-term "maybe" (owner, 2026-10-17).** Something to
> look into only after everything else on the roadmap is done, and even
> then only perhaps. Nothing here is planned or started; the picture
> problems that prompted it are handled in software
> ([design/apps-and-packs.md](../design/apps-and-packs.md) §6a).

The owner's question: could a small, cheap modern LCD (the kind sold for
retro handhelds and the Raspberry Pi) replace the X51v's original 3.7"
Sharp screen? What would the easiest conversion be, how hard is it, and
could it become a kit to sell?

**Desk research only.** Nothing was bought, and the Axim was not touched.
About 2-3 hours of searching: enough to answer the question, not a full
survey of the market.

Confidence: **confirmed** (a datasheet, a Linux driver or this repository
says so), **likely** (strong evidence, not proven), **guess** (my
estimate), **measure** (only the real hardware can answer).

Sources, cited by letter in the tables:

| Letter | Source |
|---|---|
| (S) | Sharp, *LS037V7DW01 LCD Module, Product Specifications*, LCP-06008A, 17 May 2006 (mirror: <https://www.beyondinfinite.com/lcd/Library/Sharp/LS037V7DW01.pdf>). The sibling of the Axim's LS037V7DD05; no DD05 datasheet was found. Facts only are used here; the document itself is copyrighted and is not copied into the repository |
| (D) | Sharp, *LS037V7DD05 technical literature*, LCP-04006A, 3 March 2004, marked **tentative** (found by the owner, 2026-10-30; a local copy in `local/datasheets/`, not in the repository: the document is copyrighted and forbids copying, so only facts are used here). The Axim's own panel: section 1a |
| (L) | Linux `drivers/gpu/drm/panel/panel-sharp-ls037v7dw01.c` (<https://raw.githubusercontent.com/torvalds/linux/master/drivers/gpu/drm/panel/panel-sharp-ls037v7dw01.c>) |
| (F) | Focus LCDs, *E35GG-I-CW350-N* spec, rev 1.0, 2022-06-15 (<https://focuslcds.com/wp-content/uploads/Specs/E35GG-I-CW350-N_Spec.pdf>) |
| (K) | Kingtech, *PV03502LZR40D* specification, V0, 2019-02-18 (<https://www.kingtechdisplay.com/uploads/file/20241217/ips-3.5-inch-480-640-transflective-tft-lcd-module.pdf>; product page <https://www.kingtechdisplay.com/products/ips-3-5-inch-480-640-transflective-tft-lcd-module.html>) |
| (A) | Linux patch for the Anbernic RG35XX Plus rev 6 panel, NV3052C, 2024-11 (<https://lkml.iu.edu/hypermail/linux/kernel/2411.3/00035.html>); linux-sunxi wiki (<https://linux-sunxi.org/Anbernic_RG35XX_Plus>) |
| (W) | Waveshare 3.5inch DPI LCD wiki (<https://www.waveshare.com/wiki/3.5inch_DPI_LCD>) |
| (R) | This repository: `docs/hw/2700g.md`, `docs/hw/cpld.md`, `docs/hw/backlight.md`, `docs/hw/touch.md` |
| (E) | Shop listings, quoted in the text where used (prices move; treat them as rough) |

## The short answer

**Possible, and a fun project, but not a weekend job and not much of a
business.** The easiest real conversion is a **3.5" 480x640 portrait panel
with an 18-bit RGB interface** (Kingtech PV03502LZR40D class) on a small
custom flex adapter that carries a tiny microcontroller to send the new
panel's start-up commands. It plugs into the Axim's existing screen
connector: no soldering on the Axim, nothing changed on its board.

The catch: the panels that are clearly *better* than the Sharp (twice as
bright, four times the contrast) are **landscape 640x480** panels, which
need RetroPane to scan out sideways and would show Windows Mobile and
Dell's boot and recovery screens scrambled. The portrait panel keeps
those working but gains much less (better contrast and viewing angle,
about the same or lower brightness).

Prototype on the spare **X50v** when it arrives, never on the owner's X51v.

## 1. The original screen (Sharp LS037V7DD05)

The X51v's screen is sold as Sharp **LS037V7DD05** for the X50v/X51v
(eBay listing, about US$15, new old stock:
<https://www.ebay.de/itm/282155876012>). No DD05 datasheet is public. Its
sibling **LS037V7DW01** has one (S) and a mainline Linux driver (L); they
share the 3.7" VGA CG-Silicon glass, so DW01 facts are a strong guide,
**not** proof for DD05. The X51v also shipped with a 3.7" **Samsung**
panel on some units (the CPLD's LCD-type readout has a code for it; the
owner's reads 0 = Sharp 3.7" VGA) (R: cpld.md).

| Item | Value (DW01) | Confidence | Source |
|---|---|---|---|
| Size, pixels | 3.7" (9.4 cm), 480 x 640, portrait | confirmed | S |
| Active area | 56.16 x 74.88 mm (2.21 x 2.95 in) | confirmed (DW01) | S |
| Module outline | 65.0 x 89.2 x 4.4 mm, touch panel included | confirmed (DW01) | S |
| Panel type | transflective CG-Silicon TFT, normally black, readable in sunlight by reflection | confirmed | S |
| Connector | 51 pins, 0.3 mm pitch, mates with Hirose **FH23-51S-0.3SHAW(05)** (bottom contact) on the host board | confirmed (DW01) | S; Hirose FH23 listings (<https://www.snapeda.com/parts/FH23-51S-0.3SHAW(06)/Hirose%20Electric/view-part/>) |
| Colour data | **18-bit** RGB (6 bits per colour, R5-R0, G5-G0, B5-B0); also an 8-colour mode | confirmed | S |
| Signals | CLKIN, HSYNC, VSYNC, DEN (data enable) | confirmed | S |
| Control pins | **INI** (power-on control, high = on), **RESB** (reset, low ≥ 20 µs), **MO** (low = VGA, high = QVGA), **LR** / **UD** (scan direction, high = normal), **SMPSYNC** / **SMPDATA** (which clock edge samples sync and data) | confirmed | S; L |
| Spare-looking pins | pin 2 NC, pin 4 (no function given), pin 51 **ID1 = ground** | confirmed (DW01) | S |
| Logic supply | VCC 3.3 V (3.0-3.6), about 45 mA typ., 75 mA max; logic levels 3.3 V CMOS | confirmed | S |
| Bias supplies | made **on the panel's own flex** (a DC-DC converter on the FPC); the host supplies only 3.3 V | confirmed | S |
| Pixel clock | 39.7 ns typ. (25.2 MHz), 38-41.7 ns | confirmed | S |
| Timing on the X51v | 23.5625 MHz, 480 x 640, H 88/64/16, V 1/2/1, ~55 Hz | likely | R (2700g.md, Dell table + AxDroid) |
| Power sequence | VCC on → clock and syncs running → INI high; off: INI low → keep syncs 5 frames → VCC off | confirmed | S |
| Backlight | white LED string on pins 1 (LED+) and 3 (LED−): **27.9 V typ. at 16 mA** (31.5 V max at 20 mA), 446 mW, so about 8-9 LEDs in series | confirmed (values); likely (LED count) | S |
| Brightness, contrast | 165 cd/m² typ. at 16 mA; contrast 200:1 typ.; viewing 80° each way (contrast ≥ 5); NTSC 35% | confirmed | S |
| Touch layer | 4-wire resistive, **part of the module** (touch active area 57.4 x 76.1 mm), its four wires (T1-T4) on the **same 51-pin flex** | confirmed (DW01) | S |
| Touch controller | TI TSC2046 on the X51v board, on PXA SSP1 | likely | R (touch.md) |

**What this means for a swap:**

- The X51v's host side gives the panel **3.3 V, 18 data lines, clock,
  syncs, DEN, INI, RESB, a constant-current LED supply (~28 V) and the
  four touch wires**, all through one 51-pin, 0.3 mm connector.
- The backlight driver (boost converter) is on the **Axim's board**, not
  on the panel: the panel only has the LEDs. Dimming is the PXA's PWM0,
  enable is CPLD `0x18` bit 2 (R: backlight.md). Whether the board's
  driver is constant-current (most likely: an LED boost driver) and what
  its maximum output voltage is: **measure**.
- **Touch comes with the screen.** A new screen without a resistive layer
  means no touch at all; the kit must bring its own.
- The CPLD's LCD-type readout very likely reads ID pins on this same
  connector (pin 51 ID1 is grounded on the Sharp, and the owner's reads
  type 0). An adapter must keep the ID pins exactly as the Sharp has them,
  or Windows Mobile (and RetroPane) would think a different panel is
  fitted. **likely**; measure.

### Spare lines for a serial (SPI) set-up?

Modern panels need a few dozen to ~150 start-up commands over a 3-wire
SPI before they show anything (see section 3). The X51v connector has no
SPI. The candidates for borrowing are **MO, LR, UD, SMPSYNC, SMPDATA** and
pins 2/4. On the Sharp these are set-once straps, so on the X51v board
they are most likely tied high or low with resistors, **not** driven by a
GPIO or the CPLD. INI and RESB are driven (most likely by the CPLD
"display sequence" bits in `0x12`, `0x14` bit 3, `0x18` bit 3: R,
cpld.md, guessed rows). So: **probably no spare lines**. Check with a
meter on the spare X50v: each pin's level and whether it changes during
Windows Mobile's suspend and resume.

That's why the adapter below carries its own microcontroller: it doesn't
need any spare line at all, only INI and RESB as its "go" signals.

### What must be measured on the real flex (can't be found online)

1. The DD05's pin order: is it the DW01's 51-pin order? (Count the pins,
   check GND and 3.3 V positions with a meter, power off.)
2. Which of MO/LR/UD/SMPSYNC/SMPDATA are tied, and to which level.
3. Which ID pins the CPLD reads, and their levels.
4. Whether all 18 data lines carry signal, or only 16 (the 2700G has a
   16- or 18-bit output: R, 2700g.md; ramps band visibly on the device).
5. The backlight driver's open-circuit voltage and its regulated current
   (meter in series with the LED string, at full and lowest brightness).
6. The Samsung variant's connector and pinout, if a Samsung-panel Axim
   turns up.
7. The bezel opening's size and the depth behind the panel.

## 1a. The DD05's own datasheet (found by the owner, 2026-10-30)

The owner found the DD05's datasheet (D). **It is a different kind of
module from the DW01**, so the DW01 rows above are wrong for the Axim
wherever they differ:

| Item | DD05 | Confidence | Source |
|---|---|---|---|
| What the module is | the glass with its source driver on the glass (COG), the gate driver in the glass, the LED backlight, the touch layer: **no timing controller and no power converter** ("it isn't composed control circuit") | confirmed | D |
| Connector | **61 pins** (rev A: pin 60, ID2, changed from ground to not connected), plus the touch flex (T1-T4) | confirmed | D |
| What the host must drive | the gate driver's clock and start pulse (CLS, SPS), the source driver's start pulses, latch and clock (SPL/SPR, LP, DCLK), 18 data lines, the polarity inversion (REV), VCOM (±2.5 V swing on +0.1..+2.1 V) and CsCOM, analog-switch and source controls (ASC, SSC), MOD | confirmed | D |
| Supplies the host makes | **five rails**: +3.3 V (logic), +5 V (analog), +8 V and −4 V (the gate driver), −8 V (VEE), in a set order: 3.3 V, then the signals with a white picture, then +5/+8/−4, then MOD and −8 V; off in reverse, the white picture shown 2+ refreshes before | confirmed | D |
| Modes on pins | **CO**: 262,144 colours (18-bit) or **8 colours**; **MO**: VGA or **QVGA (240 x 320)**; **U/L**, **LBR**: scan direction | confirmed | D |
| Line and pixel rate | VGA: line 35.5-39.5 kHz, pixel clock 19.4-28 MHz; QVGA: 17.9-19.9 kHz, 5.0-6.8 MHz. The datasheet's own measuring set-up is **60 Hz with a 25.175 MHz clock**, which is our 60 Hz timing (`DISPPLL` `0xFC5`, 25.19 MHz); Windows Mobile's ~55 Hz is inside the range too | confirmed | D; R (2700g.md) |
| Panel's own draw | about 5.5 mA at 5 V, 2.3 mA at 3.3 V and ~1.4 mA from the ±8/−4 V rails typ.: **about 45 mW** | confirmed (tentative values) | D |
| Backlight | LEDs **32.4 V typ. at 15 mA** (37.8 V max at 20 mA), **486 mW**: about ten LEDs in series, so **the backlight draws ~10x what the panel does** | confirmed (values); likely (count) | D |
| Response | rise 30 ms typ. (60 max), fall 50 ms typ. (100 max): **slow**; a full black-to-white change spans several 60 Hz refreshes | confirmed (tentative) | D |
| Brightness, contrast | 85 cd/m² typ. at 15 mA (tentative), contrast 65:1 lit, 2.5:1 by reflection, 6% reflectance | confirmed (tentative) | D |
| Temperature | operating −10 to 60 °C at the panel's surface, storage −20 to 70 °C | confirmed | D |

**What it changes for a swap:**

- **The Axim's board must hold a timing controller and the panel's power
  supplies** between the 2700G (which sends ordinary RGB with syncs and a
  data enable) and this connector. So the 61-pin connector carries
  driver-level signals, not a plain RGB interface. An adapter there would
  have to turn the gate and source pulses back into RGB: hard. The easier
  tap is upstream, the 2700G's RGB output before that controller, or
  replacing the controller's output stage. Where the controller sits and
  what it is: **measure** (board photos; never opening the owner's unit,
  "Any Axim").
- **The DW01 rows on INI, RESB, the single 3.3 V supply, the DC-DC on the
  flex, the 51-pin Hirose connector and the pins borrowable for SPI don't
  apply to the Axim.** The borrowable-pin candidates on the DD05 are CO,
  MO, U/L, LBR and the NC pins (2, 19, 60), and they are most likely
  straps on the board as well.
- The backlight voltage is higher than the DW01's (32 V against 28 V), so
  a replacement's LEDs must match about ten in series, or a new driver.

**What it says about the X51v as it is (not only a swap):**

- **The backlight is the screen's power, by about ten to one.** For R133
  (the GPU only while the screen moves), the backlight's level and
  time-out matter more to the battery than the panel's refresh.
- **The panel's response is slow** (30 ms + 50 ms typical). Fast flings
  and short fades smear on this glass whatever the frame rate. That is a
  physical limit of the glass, so motion design should lean on eased,
  not-too-fast movement. Measurable with the owner's camera if needed.
- **An 8-colour and a QVGA mode exist on the glass.** If the board
  drives CO or MO (from a CPLD bit or a GPIO) rather than strapping them,
  an 8-colour, lower-power standby picture (a clock face) would be
  possible. **Measure**, only with the CPLD bits' owners known and the
  owner watching (the no-blind-board-bits rule).

## 2. What the X51v side gives us

| Item | Fact | Confidence | Source |
|---|---|---|---|
| Who drives the panel | the Intel 2700G's display controller, parallel RGB | confirmed | R (2700g.md) |
| Timings | fully programmable by RetroPane's own driver: `DHT*`/`DVT*` porches and sync, `DSCTRL` sync polarities, 12-bit fields (up to 4095 per value) | confirmed | R (2700g.md, Linux `mbxfb` field layouts) |
| Pixel clock | from the 2700G's own PLL (`PIXCLK`, `PIXCLKDIV`); 23.56 MHz today | likely | R |
| Colour depth to the panel | 16 or 18 bits (not yet known which) | likely | R |
| Scan orientation | the plane is read out row by row in the panel's order; the 2700G's blitter can rotate 90/180/270° while copying (measured) | confirmed (blitter) | R |
| X50v | the same 3.7" VGA panel and the same 2700G | likely | Wikipedia (<https://en.wikipedia.org/wiki/Dell_Axim>); the LS037V7DD05 listing names X50v and X51v |

**Under RetroPane, any timing a modern panel wants is ours to set.**
Under **Windows Mobile** (and Dell's boot, flashing and factory-reset
screens) the timing is Dell's fixed 480 x 640 portrait set. A replacement
panel that can't take Dell's timing shows a broken picture whenever
Windows Mobile or Dell's screens run. This matters today: RetroPane still
starts from Windows Mobile, and the owner reads Dell's factory prompt
after a watchdog reset.

## 3. Candidate panels

All of these are 3.3 V logic (no level shifting) and parallel RGB. MIPI
DSI-only panels (for example Newhaven NHD-3.5-640480EF, BuyDisplay
ER-TFT035-9 with NV3051) are ruled out: the 2700G has no DSI, and a
RGB-to-DSI bridge (Toshiba TC358762/778 class) adds cost, power and a
second chip to start. SPI-only panels (320x480 ILI9488 and similar) are
ruled out: too few pixels and too slow.

| | A. Kingtech PV03502LZR40D | B. NV3052C 640x480 IPS (Focus E35GG-I-CW350-N; RG35XX Plus spares) | C. Waveshare 3.5" DPI LCD (Pi) | D. New Sharp LS037V7DD05 (baseline) |
|---|---|---|---|---|
| Size, pixels | 3.5", **480 x 640 portrait** | 3.5" (3.45"), **640 x 480 landscape** | 3.5", 640 x 480 landscape | 3.7", 480 x 640 portrait |
| Active area | 53.57 x 71.42 mm (95% of the Sharp's width and height) | 70.08 x 52.56 mm, i.e. 52.56 x 70.08 turned upright (94%) | not published (3D file only) | 56.16 x 74.88 mm |
| Outline (upright) | 63 x 85 x 2.9 mm | 63.84 x 76.84 x 3.27 mm | a Pi HAT board, not a bare panel | 65.0 x 89.2 x 4.4 mm |
| Fits inside the Sharp's outline? | yes, 1.5 mm thinner | yes, 1.1 mm thinner | no (a board, plus cap. touch glass) | same part |
| Type | transflective, wide view (80° all round, contrast > 10) | transmissive IPS (75° all round, contrast > 10) | IPS | transflective CG-Si |
| Brightness, contrast | **120** cd/m² typ., **300:1** | **350** cd/m², **800:1** | not published | 165 cd/m², 200:1 |
| Interface | **18-bit** RGB + 3-wire SPI | 24-bit RGB + 3-wire SPI | DPI666 (18-bit) from the Pi header | 18-bit RGB, no SPI |
| Controller, start-up | Himax **HX8363-A**: SPI commands needed (count unknown, typical 20-40) | **NV3052C**: ~145 register writes in Linux's RG35XX Plus rev 6 entry | made by the board; not described | none (INI and RESB only) |
| Connector | 40-pin FPC (pitch not in the datasheet text) | 54-pin, 0.5 mm (see the note below) | Pi 40-pin header | 51-pin 0.3 mm |
| Backlight | 8 white LEDs in series, **25.6 V at 20 mA** | 6 white LEDs in series, **17.5 V at 20 mA** | from the board, 5 V | 27.9 V at 16 mA |
| Resistive touch | **4 touch pins on its own flex** (XL/YD/XR/YU): a touch version is planned for; not confirmed it's sold | touch pins on the flex marked "not connected"; no touch | capacitive (Goodix, I2C) | built in |
| Works with Dell's timing (Windows Mobile) | **likely** (portrait, same width; needs its init first, and its porch limits checked) | **no**: landscape scan; WM's picture scrambled | no | yes |
| Price | not published; manufacturer quote (MOQ unknown). Guess US$15-30 | spares US$15-30 for the RG35XX panel (eBay, e.g. <https://www.ebay.de/itm/295897899588>); Focus: quote | ~US$34-45 (eBay, Eckstein) | ~US$15 |
| Datasheet | yes, sparse (timing and absolute ratings pages are images) | yes (F) + NV3052C datasheet + Linux init tables (A) | wiki only | DW01 only (S) |
| Confidence | confirmed (datasheet) except price | confirmed (F, A) | confirmed (W) | confirmed (S) |

Note on B's connector: the Focus datasheet names Molex 512965494 as
the mating connector (54 positions, 0.5 mm pitch, likely); listings of the
RG35XX spare call it a 54-pin panel. Whether the RG35XX spare has the
Focus pin order: **measure** (they share the NV3052C and size, not
necessarily the flex).

Others seen and not chosen: **Crystalfontz CFAF480640A-035T** (3.5"
480x640 transflective, HX8363-A, the same class as A) is **discontinued**
(<https://www.crystalfontz.com/product/cfaf480640a035t>); Crystalfontz's
current 480x640 parts are ~US$72-84, too dear for a kit. **Ortustech
COM35H3P08ULC** (3.5" 480x640 transflective "Blanview", excellent in
sunlight) seen once at US$77 plus shipping: lovely, too dear and hard to
find. **TPO TD035STEE1** (3.5" 480x640) seen at ~US$38 used; not IPS.
No **3.7" 480x640** modern panel was found: the size seems to have died
with the PDAs. Taller 480x800 panels (ST7701S, 3.5"-4") are the wrong
shape (too narrow or too tall for the opening).

**Easiest: A.** It is portrait 480 x 640 like the Sharp, 18-bit like the
Sharp, its LED string wants almost the same voltage (25.6 V vs 27.9 V),
and it has resistive-touch pins on its flex. It needs only an SPI start-up
and a pin remap.

**Best picture: B.** Twice the brightness, four times the contrast, true
IPS, 24-bit, very cheap as handheld spares. But it is landscape-native;
the NV3052C has no frame memory, so it can't turn the picture itself, and
turning it in the adapter would need a frame buffer (an FPGA plus RAM,
far beyond "cheap"). It only works with RetroPane scanning out 640 x 480
sideways (RetroPane draws upright and rotates in the renderer, which the
2700G's blitter can do), and Windows Mobile and Dell's screens would be
unreadable. It is also **transmissive**: worse than the Sharp in direct
sunlight, even at 350 nits.

## 4. The adapter

One part, plugging between the Axim's board connector and the new panel.
Nothing on the Axim is cut or soldered.

**What it does:**

1. **Pin remap**: a 0.3 mm 51-finger flex tail into the Axim's FH23
   connector at one end, a connector for the new panel's flex at the
   other (A: 40-pin; B: 54-pin 0.5 mm).
2. **Colour bits**: A takes the 18 lines straight. For B's 24 bits, wire
   each colour's two low bits to copies of its two top bits (R7..R2 =
   R5..R0, R1..R0 = R5..R4), so full white is full white: the same
   "repeat the top bits" trick the 2700G uses to widen RGB565 (ADR-0023).
   No logic needed.
3. **Panel start-up**: a tiny microcontroller on the adapter (a WCH
   CH32V003 or ST STM32C0 class part, a few tens of cents; guess) powered
   from the panel's 3.3 V. It watches the X51v's **INI** and **RESB**: when
   INI goes high it releases the panel's reset, sends the start-up
   commands over its own 3-wire SPI (a few ms), then lets the picture
   through; when INI goes low it sends "sleep in". Windows Mobile,
   RetroPane and Dell's screens all just see "a panel": **no spare lines,
   no driver change**. The firmware is ours, open, and updated through
   two test pads (SWD). It talks to nothing but the panel, so it adds no
   attack surface (it has no input from the host but two wires).
4. **ID pins**: tie them as the Sharp does, so the CPLD still reads
   type 0. (Later, a different code could tell RetroPane "modern panel
   fitted": only if a free ID code is safe for Windows Mobile. Measure
   first.)
5. **Backlight**: both A (25.6 V, 8 LEDs) and B (17.5 V, 6 LEDs) are
   series strings at 20 mA. If the X51v's driver is constant-current
   (likely), it drives either directly at its ~16 mA, and the brightness
   control keeps working. If it is not, or its over-voltage limit is
   wrong for B, the adapter carries its own small LED boost driver (TI
   TPS61160 class) fed from the X51v's LED supply's PWM. **measure** first.
6. **Touch**: the adapter routes four touch wires from a resistive layer
   to the Axim's T1-T4. For A, from its own touch pins if a touch version
   is bought; otherwise from a separate 4-wire resistive overlay (3.5",
   tape-bonded to the panel). The X51v's TSC2046 keeps working unchanged;
   RetroPane's calibration screen (item 76) covers the new size.

**Mechanics:**

- Both candidates are smaller than the Sharp in every direction (A: 2 mm
  narrower, 4.2 mm shorter, 1.5 mm thinner). The Sharp's 1.5 mm of
  thickness is room behind the panel for the adapter's small rigid part
  (microcontroller, a few capacitors, maybe the boost driver) without
  touching the case. guess, check on the X50v.
- The visible picture is about 5% smaller each way (3.5" vs 3.7"), so a
  thin black border shows inside the bezel: a printed or laser-cut mask
  (a shim that also centres the panel) hides it.
- A separate touch overlay adds ~1 mm: A with an overlay is ~3.9 mm,
  under the Sharp's 4.4 mm. Keep Sharp's rule: a 0.3-0.7 mm gap between
  bezel edge and touch surface, cushion only on the overlay's edge (S, §13).
- B is landscape: its flex leaves on a long side; turned upright it
  leaves at the side, not the bottom where the Axim's connector is. The
  adapter flex must fold around: **measure** the space.

**Rough cost (guess):**

| Item | Prototype (5) | Small batch (50) |
|---|---|---|
| Flex PCB, 2 layers, with stiffener at the tail | US$25+ for 5 at JLCPCB (<https://jlcpcb.com/blog/flex-pcb-cost-pricing-guide>) | US$3-6 each |
| Assembly (parts on the flex) | US$30-60 set-up | US$2-4 each |
| Microcontroller, connector, passives | US$2-3 | US$1-2 |
| Optional LED boost driver | US$1 | US$0.50 |

A rigid-flex (rigid island plus flex tail in one) is neater but costs
several times more at these quantities; a plain flex with a stiffener
under the parts is enough. The 0.3 mm tail with fingers that fit the
FH23's thickness is the hardest part to get right: order a test tail
first. Hirose's drawing gives the cable thickness (not found in this
search: check before layout).

## 5. Verdict

| Question | Answer |
|---|---|
| Easiest candidate | **A, Kingtech PV03502LZR40D class**: portrait, 18-bit, similar LED string, touch pins |
| Difficulty | **weeks**, not a weekend: one flex design and its re-spin, microcontroller firmware (start-up table from the HX8363-A datasheet and the vendor), bench bring-up with the panel on a Pi or ESP32 first, then fit and finish. For B add the RetroPane landscape scan-out (days) |
| Risk to the device | moderate: opening the Axim, a 0.3 mm connector and a fragile flex, a 28 V LED supply next to 3.3 V logic (a short kills the panel or worse). **Prototype on the spare X50v only.** No X51v run until the X50v has worked for weeks |
| Real gain, A | better contrast (300 vs 200:1) and colour at angles; still transflective; **slightly dimmer** (120 vs 165 nits). Modest |
| Real gain, B | much brighter (350) and far higher contrast (800:1), true IPS: a clear step up indoors. Worse in direct sun; Windows Mobile and Dell's screens unreadable |
| Colour depth | stays at the 2700G's 16 or 18 bits either way: a 24-bit panel shows no more colours, only its better contrast and angles |
| Compared with doing nothing | the owner's main complaint (DoomGLES dark, details blending) is mostly the game's 4-bit textures; RetroPane's own Doom and the gamma table fix that on the **existing** screen. A new panel helps viewing angle and contrast, not that |
| Cheapest improvement | a new LS037V7DD05 (~US$15) if the owner's panel or backlight has aged: zero design work |

## 5b. Later candidates (2026-10-17, owner's finds; desk research)

The owner found three more panels after the first survey. Measured against
the owner's aged Sharp (C6 colorimeter runs: **57 cd/m2 at full backlight,
22:1, white 13,600 K**, docs/design/display-calibration.md), not only the
new Sharp's datasheet.

| | C. DisplayModule DMKTT0350VGNA | D. "3.5 inch 640x480 RGB" listings | **E. Yada/ADIA YT035G003 (leading)** |
|---|---|---|---|
| Source | datasheet V1.0.0, 2025-12-18 (owner's copy) | shop listings (Youritech, Shenzhen LCD Mall), US$8-20 | product page <https://www.tftlcd-displays.com/sale-13551372-3-5-300cd-m2-industrial-tft-display-18-bit-rgb-interface-for-pos-device.html>, spec table only (no datasheet there) |
| Pixels | 480 x 640 **portrait** | 640 x 480 **landscape** | 480 x 640 **portrait** |
| Interface | 2-lane MIPI DSI (80-500 Mbit/s a lane) | 24-bit parallel RGB + SPI set-up | **18-bit parallel RGB** + SPI set-up (ILI9806E, no frame memory) |
| Driver | ILI9806E | NV3052C / ST7701S class (likely) | ILI9806E (same glass family as C) |
| Brightness, contrast | 900 cd/m2, 600:1, NTSC 51.7%, white x 0.303 y 0.339 | IPS on some (unverified) | 300 cd/m2 typ., 400:1 min, MVA (wide, even viewing) |
| Backlight | 6 white LEDs, 18 V at 20 mA | varies | 6 white LEDs |
| Module | 61.00 x 81.75 x 2.6 mm, no touch | ~3.5" | 60.00 x 82.90 x 2.5 mm; resistive touch offered by the supplier |
| Active area | 53.28 x 71.04 mm | ~3.5" | 53.28 x 71.04 mm (Sharp: 56.16 x 74.88) |
| Price | US$69 | US$8-20 | US$11-12, minimum 10; samples offered |

**C, MIPI (portrait):** needs an RGB-to-DSI bridge on the adapter (Solomon
SSD2828 or Toshiba TC358768A/TC358778 class: parallel RGB in, DSI out,
set up over SPI or I2C) plus the adapter's microcontroller to set up the
bridge and send the panel's start-up list at every power-on and wake (the
connector has no SPI). Timing fits RetroPane (25.19 MHz and 60.3 Hz inside
16.2-26.3 MHz and 57-63 Hz); Windows Mobile's 56.4 Hz is just under 57
(likely works, to test). About 275-300 Mbit/s a lane at 60 Hz. The
18-bit source is padded to 24 by the bridge, so dithering stays. The
start-up list is **not** in the datasheet (ask the vendor). Matched
100-ohm differential traces on the flex.

**D, landscape parallel RGB: not recommended.** Turning the glass doesn't
turn the data: the panel scans 480 lines of 640 pixels, the 2700G sends 640
lines of 480, and these panels can mirror but not swap rows and columns (no
frame memory). Every frame would have to be rotated before it leaves:
Windows Mobile, Dell's boot and recovery screens unreadable, RetroPane
drawing everything sideways (2D engine rotation unknown) and legacy
direct-to-screen games (GAPI, DirectDraw) needing emulated rotation. Still
needs the set-up microcontroller and a touch layer. Only for an Axim that
would run RetroPane alone, forever.

**E, YT035G003: the easiest real path found so far.** Portrait and 18-bit
parallel RGB is exactly what the Axim already sends (no bridge, no
rotation; Windows Mobile and Dell's screens unchanged), 18x the contrast of
the owner's panel and about 5x its brightness, MVA viewing angles, and the
supplier offers a resistive touch panel and custom flex cables. Still
needed: the adapter's microcontroller for the ILI9806E's SPI start-up list
(option A's design), and from the supplier: the full specification
(pinout, timing, power sequence, connector), **the start-up code for 18-bit
RGB mode**, the backlight's voltage and current, colour figures. The
listing is inconsistent (its headline says 320x480 "SPI RGB"; its table
480x640, "SPI+SPI 18BIT"): confirm the exact part before ordering.

**For C and E alike:**
- **Backlight:** the Axim's own LED boost driver regulates current, so a
  6-LED, ~18 V string should run from it as is (the Sharp's is 8-9 LEDs,
  ~28 V); **measure** its current and open-circuit voltage first (a new
  boost converter is likely unnecessary).
- **Touch:** neither module has one built in (E's supplier can laminate
  one); the Sharp's touch layer comes with the Sharp.
- **The CPLD's LCD-type ID** must read as the Sharp's (type 0) through the
  adapter.
- **Fit:** the active area is about 1.4 mm smaller each side and 1.9 mm top
  and bottom than the Sharp's: a thin black mask or frame; depth fits (2.5-2.6
  mm plus a touch layer, under the Sharp's 4.4); where the adapter sits:
  measure on the spare X50v.
- **First steps** when the swap comes off the parking list: ask the
  supplier (E) for the datasheet and RGB start-up code; measure the
  Axim's backlight current and bezel depth on the spare X50v; order E's
  samples with the resistive touch option.

## 6. As a kit

**Bill of materials (guess):** panel US$15-30, adapter (assembled flex)
US$6-12, touch overlay US$3-8, bezel mask/shim US$1-3, packaging and a
spudger US$2. **About US$30-55** a kit at 50; a sale price of US$70-100
to cover failures, returns and time.

**Who buys it:** Axim X50v/X51v owners in the retro-PDA crowd (forums
such as HPC:Factor, retro-handheld and PDA subreddits). A small, enthusiast
market: tens of kits, maybe low hundreds over years (guess). Version B
only suits people running RetroPane, which shrinks it further until
RetroPane is the installed OS.

**X50v and X51v:** the same Sharp panel is sold for both (eBay listing
above) and both have the 2700G, so one kit very likely fits both (likely;
confirm on the spare X50v). The **Samsung-panel** units may have a
different connector: one more thing to check before selling.

**Licensing and IP:**

- The kit is our own hardware design and firmware. Nothing from Dell
  (code, drivers, files) goes into it, per `docs/CLEANROOM.md`.
- Sharp's datasheet is copyrighted ("no part may be reproduced"). We
  use its facts (pin functions, voltages); we don't copy the document
  or its drawings.
- Panel start-up tables: write ours from the controller's datasheet and
  the panel maker's notes. Linux's NV3052C tables are GPL-2.0: copying
  them makes our firmware GPL (fine if we publish it, but a decision to
  make on purpose).
- No Dell or Axim logos on the kit; "for Dell Axim X50v/X51v" as a fit
  description only.
- Selling electronics in the US/EU: an internal accessory like this
  probably falls outside most radio/EMC certification as a part, but CE
  (EU) and RoHS questions come up; ask before selling into the EU (guess).

**Support burden:** high for the size of the market. Every buyer opens a
20-year-old handheld with brittle clips and a 0.3 mm connector; some will
break flexes or bezels and ask for help. Clear photo instructions, a
spare tail in the box, and "Windows Mobile works" (A) vs "RetroPane only"
(B) stated up front.

## Next steps, if the owner wants to go on (not started)

1. When the X50v arrives: open it, photograph the panel and flex, and
   measure the seven items in section 1 (meter only, no power-on with the
   panel disconnected).
2. Ask Kingtech for a price, MOQ, the 40-pin pitch and a touch version
   (an email the owner sends; no account or order made here).
3. Bench-test the chosen panel on a Raspberry Pi or ESP32-S3 (both drive
   parallel RGB and SPI) before any adapter is drawn.
4. Then the adapter flex, on the X50v only.

## Summary

**Recommendation:** a fun project worth doing on the spare X50v, not a
real business: the easiest swap is a 3.5" 480x640 portrait panel
(Kingtech PV03502LZR40D class) on our own flex adapter with a tiny
microcontroller doing the panel's start-up; a big picture upgrade needs a
landscape IPS panel that only RetroPane can drive.

**Top 3 risks:**

1. **Unknowns on the flex:** the DD05's real pin order, the ID pins, the
   backlight driver's voltage limit and whether all 18 colour lines are
   wired can only be measured; any of them can change the design.
2. **The best panels are landscape:** brighter IPS panels scramble
   Windows Mobile and Dell's boot and recovery screens; the portrait
   panel that keeps them working is only a modest upgrade (and dimmer).
3. **Device and support risk:** a 0.3 mm connector, a 28 V LED supply
   and brittle 20-year-old cases: prototype only on the X50v, and as a
   kit the support load is large for a market of tens of buyers.
