# PXA27x USB device controller (UDC) on the X51v

The X51v's USB sync cable and cradle connect to the PXA27x's built-in USB 1.1
full-speed device controller. RetroPane drives it to appear on any PC as
standard USB classes (DESIGN.md §10b, ADR-0012).

Evidence key: **(M)** PXA27x developer's manual register model as used by the
Linux `pxa27x_udc` driver (read for facts only, as with the other reference
drivers); **(A)** AxDroid's X50/X51 board file; **(D)** measured on the
owner's X51v by `rp-probe` (`probe-2026-10-11.md`).

## Board wiring

| Signal | Where | Meaning | Confidence | Evidence |
|---|---|---|---|---|
| USB cable detect | GPIO14, input | high on the probe run; both edges enabled as an interrupt and as a wake source (`GRER0`/`GFER0`/`PWER` bit 14) by WM | confirmed (pin), likely (polarity: high = cable present) | (A), (D) |
| D+ pull-up ("connect") | GPIO2, output | driven high by WM on the probe run; the host sees the device only while the pull-up is on | likely | hardware inventory, (D) (output, high) |
| UDC clock | `CKEN` bit 11 | on as WM left it | confirmed | (D) `CKEN 0x00D91AA3` |
| Data pins | dedicated USB client pins | no `GAFR` setting needed | likely | (M) |

## Registers (base `0x40600000`)

| Register | Offset | Meaning | Confidence | Evidence |
|---|---|---|---|---|
| `UDCCR` | `0x000` | control: `[0]` UDE enable, `[1]` UDA active (read), `[2]` UDR resume, `[3]` EMCE endpoint memory configuration error, `[4]` SMAC switch endpoint memory to the active configuration, `[7:5]` AAISN, `[10:8]` AIN, `[12:11]` ACN active configuration/interface/alternate (read), `[16]` DWRE, `[31]` OEN. Writes must only carry OEN, SMAC, UDR and UDE (read-modify-write with the others masked off) | likely | (M) |
| `UDCICR0` | `0x004` | interrupt enables, 2 bits per endpoint 0, A–O: `[0]` packet complete, `[1]` FIFO error | likely | (M) |
| `UDCICR1` | `0x008` | the same for P–X, plus `[27]` reset, `[28]` suspend, `[29]` resume, `[30]` SOF, `[31]` configuration change | likely | (M) |
| `UDCISR0` / `UDCISR1` | `0x00C` / `0x010` | interrupt status, same layout as the enables; write 1 to clear | likely | (M) |
| `UDCFNR` | `0x014` | frame number | likely | (M) |
| `UP2OCR` | `0x020` | port 2 transceiver control (pull-ups/downs, `[16]` HXS host/device select) | likely; X51v use unknown | (M) |
| `UDCCSR0` | `0x100` | endpoint 0 control/status, see below | likely | (M) |
| `UDCCSRx` | `0x100 + 4x` | endpoint x = 1..23 (A..X) control/status | likely | (M) |
| `UDCBCRx` | `0x200 + 4x` | bytes in the OUT FIFO (`[9:0]`) | likely | (M) |
| `UDCDRx` | `0x300 + 4x` | FIFO data: 32-bit accesses for whole words, 8-bit accesses for the remainder | likely | (M) |
| `UDCCRx` | `0x400 + 4x` | endpoint x configuration, see below | likely | (M) |

### `UDCCSR0` (endpoint 0)

| Bit | Name | Meaning |
|---|---|---|
| 0 | OPC | OUT packet (or SETUP) complete; write 1 to clear |
| 1 | IPR | IN packet ready: set by software to send a short (incl. zero-length) packet; a full 16-byte FIFO is sent without it |
| 2 | FTF | flush the transmit FIFO |
| 3 | DME | DMA enable |
| 4 | SST | a STALL was sent; write 1 to clear |
| 5 | FST | force STALL |
| 6 | RNE | receive FIFO not empty |
| 7 | SA | SETUP active: the FIFO holds an 8-byte SETUP packet; write 1 (with OPC) to clear once read |
| 8 | AREN | ACK response enable (acknowledges the status stage of SET_CONFIGURATION / SET_INTERFACE) |
| 9 | ACM | ACK control mode |

Endpoint 0's FIFO is 16 bytes. A SETUP packet is detected by SA, OPC and RNE
all set, read as two 32-bit words, then acknowledged by writing SA | OPC.

### `UDCCSRx` (endpoints A–X)

| Bit | Name | Meaning |
|---|---|---|
| 0 | FS | FIFO needs service |
| 1 | PC | packet complete; write 1 to clear |
| 2 | TRN | Tx/Rx NAK |
| 3 | DME | DMA enable (preserve on writes) |
| 4 | SST | a STALL was sent; write 1 to clear |
| 5 | FST | force STALL (preserve on writes) |
| 6 | BNE / BNF | buffer not empty (OUT) / not full (IN) |
| 7 | SP | short packet: set by software to send a short IN packet |
| 8 | FEF | flush the FIFO |
| 9 | DPE | data packet error |

Writes carry only the bits being acted on (plus DME/FST to keep them). An IN
packet of the full maximum size is sent when the FIFO fills; a shorter one
needs SP. An OUT packet is read from `UDCBCRx`/`UDCDRx`, then released with PC.

### `UDCCRx` (endpoint configuration)

| Bits | Field | Meaning |
|---|---|---|
| 0 | EE | endpoint enable |
| 1 | DE | double buffering |
| 11:2 | MPS | maximum packet size |
| 12 | ED | direction: 1 = IN |
| 14:13 | ET | type: 1 isochronous, 2 bulk, 3 interrupt |
| 18:15 | EN | USB endpoint number |
| 21:19 | AISN | alternate setting |
| 24:22 | IN | interface number |
| 26:25 | CN | configuration number |

The endpoint table is fixed while the UDC is enabled: every endpoint, with the
configuration, interface and alternate setting it belongs to, is written
before UDE is set, and EMCE reports a table the hardware can't fit.

## Behaviour

| Behaviour | Confidence | Evidence |
|---|---|---|
| **SET_ADDRESS, SET_CONFIGURATION and SET_INTERFACE are handled by the hardware.** Software sees a configuration-change interrupt (`UDCISR1[31]`), reads the new numbers from `UDCCR`, acknowledges with `UDCCSR0` AREN (plus ACM), and sets `UDCCR` SMAC | likely | (M) |
| GET_DESCRIPTOR, GET_STATUS and class requests (CDC line coding etc.) reach software as SETUP packets on endpoint 0 | confirmed | (M), real X51v: enumerated, 53 SETUPs answered |
| **Host-to-device requests (with or without a data stage): the hardware sends the status stage (zero-length IN) itself** once SA/OPC are cleared and any OUT data read. Software must not set IPR for them: a driver that did made Windows report "a device attached to the system is not functioning" when opening the port | confirmed | real X51v (2026-10-11); (M) never sets IPR there |
| GPIO14 reads **0 with no USB host connected** and the device still charging from the cradle's power; with the cradle's USB reseated in the PC, the PC enumerated the device | likely (the reading during enumeration wasn't captured) | real X51v |
| Bus reset raises `UDCISR1[27]`; software flushes endpoint 0 (FTF, OPC) and returns to idle | likely | (M) |
| Enable order: interrupts masked, UDE clear, endpoint table written, UDE set, then EMCE checked | likely | (M) |

## Open questions

- GPIO14 polarity (cable present = high?) and whether the cradle and the cable
  behave the same: `rp-usb` shows the level live.
- Whether HaRET's "shutting down hardware" leaves the UDC enabled; RetroPane
  resets it either way (UDE off, then a fresh table).
- Whether the X51v routes the UDC through port 2 (`UP2OCR`): add `UP2OCR` to
  the probe if enumeration fails.
