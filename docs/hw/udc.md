# PXA27x USB device controller (UDC) on the X51v

The X51v's USB sync cable and cradle connect to the PXA27x's built-in USB 1.1
full-speed device controller. RetroPane drives it to appear on any PC as
standard USB classes (DESIGN.md §10b, ADR-0012).

Evidence key: **(M)** PXA27x developer's manual register model as used by the
Linux `pxa27x_udc` driver (read for facts only, as with the other reference
drivers); **(A)** AxDroid's X50/X51 board file; **(D)** measured on the
owner's X51v by `rp-probe` (`probe-2026-10-11.md`); **(T)** Toradex's PXA27x
DMA library documentation (its request-number table); **(U)** measured on the
owner's X51v by the item 74 work (`cargo xtask usbbench`, 2026-10-16).

## Board wiring

| Signal | Where | Meaning | Confidence | Evidence |
|---|---|---|---|---|
| USB cable detect | GPIO14, input (AxDroid's guess) | read **1** on the probe run (docked, as WM left it) but **0** under RetroPane while docked and enumerated. So it is not a plain cable-present level, or needs something WM sets up (a CPLD rail? a comparator?). Again 0 on 2026-10-16 with the PC configured (the loader's status line "cable 0 configured yes" after a soft reset), and every boot in the cradle since (owner, 2026-10-16): the loader's "cable" label shows this unproven reading, not a fault. WM enables both edges on it as an interrupt and as a wake source (`GRER0`/`GFER0`/`PWER` bit 14), so it does matter to WM | open | (A), (D), real X51v (2026-10-11) |
| D+ pull-up ("connect") | **none found: probably hard-wired.** **But a disconnect can be made without it (2026-10-16, `rp-unplug` run 3): the PXA27x's own host controller, given port 2's transceiver (`UP2OCR.HXS`), drives SE0 with a port reset and the PC sees an unplug; see Open questions, T4.** `rp-reconnect` (2026-10-11) tried, with the UDC enabled and the PC idle: GPIO2 low→high, GPIO2 as input→output high, `UP2OCR` DPPUE off→on, DPPDE on→off, CPLD `0x1C` bit 3 off→on, UDE off→on, and CPLD bit 3 + UDE together. Not one produced a bus reset or a SETUP within 3 s: the PC never saw a disconnect. `UP2OCR` read `0x00020020` with the UDC on (HXOE and bit 5). Next: forcing SE0 through `UP2OCR`'s output override, and a PC-side device restart | open | real X51v |
| (earlier guess) GPIO2, output, high | the PC enumerated the device with GPIO2 high, but **never noticed a disconnect** when GPIO2 went low (or the UDC was disabled): every run needed a cable replug. So GPIO2 is not (only) the pull-up control; AxDroid's board file left `gpio_pullup` unset too. Candidates: a CPLD bit (docs/hw/cpld.md), `UP2OCR.DPPUE` with the internal transceiver | open | real X51v (2026-10-11) |
| Transceiver | **port 2's, output enabled: `UP2OCR` = `0x00020000` (HXOE) as WM leaves it** | with HXOE cleared the PC saw the attach (a bus reset) but no SETUP ever arrived; with it set, enumeration worked. The driver sets it | confirmed | real X51v (2026-10-11) |
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
| `UP2OCR` | `0x020` | port 2 transceiver control: `[26:24]` SEOS (0 = on-chip transceiver; 2/4/6 = device controller through GPIO pads / external OTG transceiver / external charge pump; odd values the host controller), `[17]` HXOE transceiver output enable, `[16]` HXS output select (0 device, 1 host), `[7:6]` D−/D+ pull-up bypass, `[5:4]` D−/D+ pull-up enable, `[3:2]` D−/D+ pull-down enable, `[1:0]` charge pump. Reset: HXOE = HXS = 1, all resistors off. **WM leaves it `0x00020000`: on-chip transceiver, device output, and no on-chip D+ pull-up, yet the PC enumerates the device: the X51v has an external D+ pull-up resistor** (no software control found, see wiring) | confirmed (manual + measured) | PXA27x manual §12.5.2–12.6.3, real X51v |
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
| 0 | FS | FIFO needs service (IN: read the same as BNF in every probe state, (U)) |
| 1 | PC | packet complete; write 1 to clear. With two buffers it says only that *a* packet completed |
| 2 | TRN | Tx/Rx NAK |
| 3 | DME | DMA enable: the endpoint drives its DMA request. Every write sets or clears it, so writes made while DMA serves the endpoint must carry it |
| 4 | SST | a STALL was sent; write 1 to clear |
| 5 | FST | force STALL (preserve on writes) |
| 6 | BNF / BNE | IN endpoint: buffer not full; OUT endpoint: buffer not empty. Reads 0 while the endpoint isn't active, which once looked like "full" (it was the interface problem below, not this bit). Double buffered, IN: 1 while a buffer is free and no short packet waits (table below) |
| 7 | SP | short packet. IN: set by software to send a short packet. OUT: set by the hardware while the packet being read is shorter than the maximum ((U): thousands of 31-byte command blocks read by it, none misread) |
| 8 | FEF | flush the FIFO |
| 9 | DPE | data packet error |

Writes carry only the bits being acted on (plus DME/FST to keep them). An IN
packet of the full maximum size is sent when the FIFO fills; a shorter one
needs SP. An OUT packet is read from `UDCBCRx`/`UDCDRx`, then released with PC.

### `UDCCRx` (endpoint configuration)

| Bits | Field | Meaning |
|---|---|---|
| 0 | EE | endpoint enable |
| 1 | DE | double buffering: two packet buffers (accepted for the four 64-byte bulk endpoints, no EMCE, (U)) |
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

**Only the active interface's endpoints work** (confirmed on the real X51v):
with a CDC-ACM device's bulk pair in interface 1 the port opened but no
bulk data moved either way; the table therefore puts every endpoint in
interface 0 (the descriptors still say interface 1, which the host uses).
Linux's PXA27x driver does the same ("multiple interfaces are really broken
on the pxa") and gives each endpoint a distinct number.

## Behaviour

| Behaviour | Confidence | Evidence |
|---|---|---|
| **SET_ADDRESS, SET_CONFIGURATION and SET_INTERFACE are handled by the hardware.** Software sees a configuration-change interrupt (`UDCISR1[31]`), reads the new numbers from `UDCCR`, acknowledges with `UDCCSR0` AREN (plus ACM), and sets `UDCCR` SMAC | likely | (M) |
| GET_DESCRIPTOR, GET_STATUS and class requests (CDC line coding etc.) reach software as SETUP packets on endpoint 0 | confirmed | (M), real X51v: enumerated, 53 SETUPs answered |
| **Host-to-device requests (with or without a data stage): software sends the zero-length status IN by setting IPR** after clearing SA/OPC (and reading any OUT data). Measured: a driver that set IPR got the port opened by Windows (DTR arrived twice); one that didn't failed on opening with "a device attached to the system is not functioning". (A reading of (M)'s interrupt-driven code had suggested the opposite; the device decides.) | confirmed | real X51v (2026-10-11, two runs) |
| GPIO2 = 1 (output) and GPIO14 = 0 (input) while docked and enumerated under RetroPane, with WM's edge set-up still in place (`GRER0`/`GFER0` bit 14 set) | confirmed | probe via the loader, 2026-10-12 (`GPLR0` 0x057403FE, `GPDR0` 0xF3FB861C) |
| GPIO14 reads **0 with no USB host connected** and the device still charging from the cradle's power; with the cradle's USB reseated in the PC, the PC enumerated the device | likely (the reading during enumeration wasn't captured) | real X51v |
| Bus reset raises `UDCISR1[27]`; software flushes endpoint 0 (FTF, OPC) and returns to idle | likely | (M) |
| Enable order: interrupts masked, UDE clear, endpoint table written, UDE set, then EMCE checked | likely | (M) |
| **Composite device (ADR-0014):** endpoints 4 (bulk OUT) and 5 (bulk IN) carry USB mass storage in descriptor interface 2; like the serial data pair they sit in interface 0 of the hardware endpoint table (only the active interface's endpoints work). Device class 0xEF/02/01 with an IAD over interfaces 0-1, `bcdDevice` 26.01 so Windows re-enumerates | emulator; device pending | ADR-0014 |
| A bus reset's endpoint 0 clean-up (`FTF`, `OPC`) must not clear the `OPC` of a SETUP that has already arrived (`SA` set): that is the first request, lost. Real hosts wait 10 ms after a reset, so it's a race only with a slow driver poll or an impatient host | likely | emulator (rp-v0.8.0 model), driver test |
| **IN data waits until a PC program reads the port.** While the device is configured but no program has the port open, Windows issues no IN requests: whatever the device queues stays queued. A device that keeps writing (the loader's once-a-second READY) fills its own ring; after an hour idle, 36 stale lines and one torn one poured out when the port was next opened, and the reply to the first frame was cut short | confirmed | real X51v, 2026-10-12 (idle-put log) |
| **One packet in flight per endpoint** (single-buffered; with `DE` see "Throughput" below). The next IN packet may be written only after the previous one's packet-complete event (`UDCISR0` bit 0 for endpoint 0, `UDCCSRx.PC` for A–X), the moment an interrupt-driven driver would run. IPR/BNF alone don't prove a packet has left (a short packet doesn't fill the FIFO) | likely: (M)'s driver only writes from those events; a polled driver that didn't wait is the leading explanation for the "Device Descriptor Request Failed" seen on the device | (M), real X51v |

## Throughput: double buffering, DMA, interrupts (item 74)

Measured on the X51v (U) by the loader under test from RAM; numbers in
`perf.md`. Linux's driver uses neither ("No use is made of UDC's double
buffering either", and no DMA), so none of this came from it.

| Behaviour | Confidence | Evidence |
|---|---|---|
| **Double-buffered IN, `BNF`:** with `DE`, the mass storage IN endpoint (flushed between steps with `FEF`) read `UDCCSRx` 0x43 empty, **0x43 after one full packet** (FS, PC, BNF: a buffer free), **0x02 after two** (full), **0x02 after one short packet** (12 bytes + SP: nothing may go in behind it), 0x02 after a full and a short one. So `BNF` alone says when an IN write is safe | confirmed | (U) probe, 2026-10-16 (three runs, same values) |
| **Double-buffered OUT: the status and count can still be changing when a packet completes.** Reading `UDCBCRx` as soon as PC appeared gave a short count for a full packet: the driver released it unread and the bytes were lost (the PC's write completed, the device counted ~1 KB of a 921 KB frame). Fix: read status and count until two reads agree, and take a full packet's size from SP (clear = 64). Since then: 0 mismatches and 0 bad command blocks in 80 MiB; 0-6 unsettled reads per trial caught | confirmed | (U) trials 2-9 and the soak |
| **DMA request lines: endpoint 0 is request 24, slot A 25, slot x 24 + x** | confirmed for slots B-E | (T); (U): requests 26-29 moved data both ways, checked byte for byte |
| **OUT by DMA needs no CPU per packet:** with `DME` set and a channel armed on the FIFO (32-byte bursts, 32-bit width), 64 KiB of full packets arrived in 61 ms with the CRC right; afterwards `UDCCSRx` read 0x8 (DME only): the DMA reading a packet to its end releases it | confirmed (full packets; a short packet by DMA not tried) | (U) |
| **IN by DMA:** full packets go to the host by themselves as the DMA fills the buffers (both buffers kept full); 64 KiB in 57 ms, every byte checked; afterwards PC and TRN set. A partial last packet needs SP from software | confirmed (whole packets) | (U) |
| **The CPU keeps one IN packet in flight.** An early run streaming CPU writes into both IN buffers saw Windows time out and reset the device during drive reads (14 resets), but that run also had the OUT race above (6 mismatched counts), which explains resets by itself; streaming wasn't retried. The drive's blocks go by DMA (both buffers) and the CPU writes only command replies and the status, after the DMA's last packet (waiting for `BNF`, not for a packet-complete that may already have come) | likely | (U) |
| **The USB client interrupt is PXA27x source 11**; `UDCISR0` bit 2x is slot x's packet complete. With those enabled the waiting loader sleeps until the PC sends or takes a packet: idle round trip median 1,170 to 292-334 us | confirmed | (U) |
| **Windows moves at most 128 blocks (64 KiB) per READ(10)/WRITE(10)** to this full-speed drive | confirmed | (U) `udc stats` "largest transfer" |
| Windows sometimes refuses to reopen the serial port for a moment after a program closed it ("access denied"): xtask's scripts retry the open | likely | (U) the first of several back-to-back commands died right after finding the port |

## Open questions

### The first enumeration after a hand-over from Windows Mobile (2026-10-16)

**Symptom.** After a soft reset (Windows Mobile boots, HaRET starts the
card's loader) Windows often doesn't configure the serial + drive device:
the loader counts resets and SETUPs (2026-10-16: 11 resets, 34 setups;
2026-10-15 after a watchdog reboot: 27 resets, **0** setups) and after 25 s
falls back to the serial-only layout, which Windows takes at once. After an
image runs and returns, the composite device is configured every time.

**Windows' side** (Kernel-PnP and DeviceSetupManager logs, read without
admin; `Get-PnpDevice` arrival times):

| When | What Windows logged |
|---|---|
| 10-16 05:43:36 (a soft reset) | the composite "surprise removed, missing on the bus"; removal of its drive interface (`MI_02`) **vetoed** (open handles) |
| 05:43:48-05:45:29 | **7 cycles of "Unknown USB Device (Device Descriptor Request Failed)"** (`VID_0000&PID_0002`, Code 43), one every 15-17 s, each "failing" then "missing on the bus"; then nothing for 7 minutes: Windows stopped trying |
| 10-15 12:34, 12:39, 12:46, 13:04 | the same: the composite gone, then 6-7 descriptor-failure cycles ~15 s apart, then silence |
| 10-16 07:09:28-07:09:42 | Windows Mobile's own device (`VID_413C&PID_4011`, "Windows Mobile Remote Adapter") arrives and leaves (HaRET) |
| 07:09:42-07:12:03 | nothing logged: no descriptor-failure device, no RetroPane device (the loader's 11 resets / 34 setups fall in here) |
| 07:12:03 | serial-only `USB\VID_1209&PID_0001\RP26-0001` (COM3) arrives: the fallback |
| **07:12:15** | COM3 leaves and the **composite's two drives arrive in the same second**: after a disable/enable of the UDC (most likely `drive on`, which switches back from the fallback), Windows configured the composite at the first try |
| 07:12:48-07:13:14 | an image runs (the drives fail), the loader returns, the composite is configured again |

So the composite layout itself isn't what Windows rejects (07:12:15, and
every image return); what fails is the loader's **first** UDC set-up after
Windows Mobile owned the controller, and the fallback works because it
**re-enables the controller**, not because of the layout.

**More device evidence (2026-10-16, loader from main 98ae4fa, docked in
the cradle).** After soft resets, two outcomes. (a) **11 resets, 34 setups,
identical on two separate boots**, then the serial-only fallback; COM3
12-80+ s later. Counts that repeat exactly point at a fixed sequence
failing at a fixed step, not at timing luck. (b) Once: **0 resets, 0
setups** for 2+ minutes: the PC never touched the device until the owner
lifted the Axim out and re-docked it (a real disconnect), then COM3 in
16 s. That is hypothesis 2 below seen directly: Windows had given up on
the port before the loader came up.

A consequence for hypothesis 1: the fallback's disable/enable cures (a),
so the culprit can't be state that survives both an `enable` and the
loop's polling (`UP2OCR`, the `UDCCSRx` bits, `DRCMR`): those would still
be there at the second `enable`. What the first `poll` clears and the
second set-up never sees is the **pending status in `UDCISR0/1`** (and
anything the host itself remembers). `UP2OCR` stays the candidate for the
separate 0-setups case after a watchdog reboot.

**T1 result (2026-10-16 08:41-08:51, USB trace on the PC, Axim docked on
hub port 7).** `cargo xtask reboot-wm` reset the board (its first try on
the device: it works). Then:

| Time | Port 7 as the PC's hub saw it |
|---|---|
| 08:41:40.6 | **disconnect** (port status `0x100`: powered, nothing connected): a board reset takes the D+ pull-up away |
| 08:41:54.7 | connect (`0x101`); enumeration starts |
| 08:41:58.0 | enabled (`0x103`); Windows Mobile's "Remote Adapter" (`413C:4011`) configured |
| ~08:46 | the owner starts the loader from Windows Mobile (HaRET) |
| 08:46-08:51 | **nothing**: no status change, no reset, no enumeration, no transfer error on port 7. Windows kept listing Windows Mobile's device as present and working; the loader showed 0 resets, 0 setups |

So **Windows never sees HaRET's hand-over**: the pull-up stays (hard-wired),
and Windows Mobile's device was idle, so no transfer failed and nothing made
the hub reset the port. The loader is invisible until something on the PC
talks to the old device and fails (outcome a: the hub's recovery resets
reach the loader) or the cable is re-plugged (outcome b). A board reset is
different: it **does** drop the pull-up (14 s disconnect), so the pull-up's
supply is switchable by something on the board.

Revised reading of outcome (a): the hub's resets are its *recovery of
Windows Mobile's device*, which re-reads the descriptors and finds a
different device (`1209:0001` instead of `413C:4011`); a fixed recovery
script failing the same way each time fits the identical 11 / 34 counts,
and the serial-only device appearing 12-80 s later, on Windows' schedule,
fits Windows giving the old device up and enumerating afresh. Not traced
yet (this run was outcome b).

**Fix direction.** The loader must make the PC see a disconnect at start,
as a board reset does. Candidates, each checked from the PC's side now
that T1 gives an observable (port status, or Windows Mobile's device
leaving): forcing SE0 through `UP2OCR`'s output override (the open item in
the wiring table), and the board power bits a reset clears (CPLD; GPIOs
for the USB transceiver's supply). `rp-reconnect` (2026-10-11) already
ruled out GPIO2, `UP2OCR` DPPUE/DPPDE, CPLD `0x1C` bit 3 and `UDE`. A PC-side
port cycle (`IOCTL_USB_HUB_CYCLE_PORT`, `pnputil /restart-device`) works
too but needs admin each time: a fallback, not the fix.

**T4 design: `rp-unplug`, the disconnect hunt (built 2026-10-16,
`cargo xtask emu-unplug` passes; not yet run on the device).** A new test image (test images ratchet freely; no loader bytes),
started by the loader with `cargo xtask load`, built on `rp-reconnect`:
it enables the UDC itself, then for each candidate applies it for 1 s,
undoes it, and counts the bus resets and SETUPs in the next 3 s (a real
reconnect makes the PC's hub reset the port within ~0.1 s). Results to the
screen and the run log; the PC watches the port at the same time
(Windows' device list once a second, optionally the T1 trace). The image
feeds the watchdog between candidates (the run takes ~80 s; the default
limit is 30 s).

Windows Mobile's own USB driver maps only the controller (its registry
entry: `MemBase` and `IoBase` both `0x40600000`, ROM A08 `default.hv`), so
a disconnect Windows Mobile could make goes through the controller, the
port 2 transceiver, or the shared CPLD table. Candidates, safest first,
each restored before the next:

1. **SE0 from the PXA27x's own host controller.** Port 2's transceiver can
   be switched to the USB host controller (`UP2OCR.HXS` = 1). With its
   clock on (`CKEN` bit 10) and port 2 powered, a root-hub port reset
   (OHCI `HcRhPortStatus[1]`, "set port reset") drives SE0 for ~10 ms,
   which overpowers the 1.5 kOhm pull-up: the PC's hub reads SE0 for more
   than 2.5 us as a disconnect. Then `HXS` back to 0 and the UDC enabled
   afresh. Repeated a few times in the 1 s window. The one candidate that
   needs no unknown board wiring.
2. **`UP2OCR`'s other transceiver bits**: the D+/D- pull-up bypass bits
   (`[7:6]`) and the transceiver output enable off (`HXOE` = 0) while the
   UDC is off (2026-10-11 tried `HXOE` only with the UDC on).
3. **CPLD bits Windows Mobile has set at hand-over whose purpose isn't
   known**, one at a time, cleared for 1 s and set again: `0x1C` bit 2,
   `0x14` bit 3, `0x1A` bits 1 and 2, `0x02` bits 0, 2-5 and 7. Never the
   known ones: SD power (`0x16` bit 0), CF (`0x16` bit 1, `0x10` bit 12,
   `0x18` bit 1), backlight (`0x18` bit 2), display (`0x12`), the reboot
   bit (`0x08`). A board reset clears them all and does disconnect, so the
   pull-up's supply may be one of them.
4. Only if 1-3 find nothing (a separate run, its own plan): GPIO outputs
   Windows Mobile leaves high in GPIO function, excluding GPIO99 (board
   reset) and every pin a RetroPane driver uses.

A candidate that switches off something else (the screen, the card) is
undone 1 s later; the image keeps going and the run log names the step.
What "found" means: bus resets in the 3 s after the candidate on the
device **and** the device leaving and coming back on the PC. Then: the
loader gets that disconnect at start (its `DETACH` window, which today
does nothing), proven over five hand-overs from Windows Mobile, and the
emulator models the same switch (the fork's host sees the disconnect).

**Run 1 on the device (2026-10-16 09:29-09:31), and what changed.** The
image started at 09:29:56 and its UDC was configured by Windows at once.
Windows logged **no removal of the device until 09:31:22**: none of the
candidates tried before the hang (the host controller's SE0, the
transceiver off, CPLD `0x1C` bit 2, `0x14` bit 3, perhaps `0x1A` bit 1)
made the PC see a disconnect. At 09:31:22, ~87 s in, the hardware watchdog
reset the Axim into **Dell's factory-restore prompt** (the owner answered
No with the Mail button). The watchdog is fed for the run limit plus 30 s,
so the image stopped checking in ~27 s in: around clearing `0x14` bit 3 or
`0x1A` bit 1. A bus that stops answering (a chip powered off under a
screen write) can't be caught by the soft run limit. Windows Mobile then
booted over the run log, so the image's own lines were lost.

My error, and the class: `0x14` bit 3 is listed in `cpld.md` as "display
(guess)" and `0x1A` bit 1 as "battery / charger (guess)", yet the plan
called them "purpose unknown". A bit with any recorded owner, even a
guess, is not switched blind; a bit with none still risks the bus and is
only tried with the owner watching, one per run. Run 2's image has the two
transceiver candidates only (`0x1C` bit 2 already showed nothing), writes
the run log before the screen, and stays well inside the run limit (~15 s).
Still to learn from run 2: whether the host controller saw the port at all
(its port status before and after the resets is printed); if it never saw
the connection, its reset drives nothing and the candidate is untested.

**Runs 2 and 3 (2026-10-16): found.** Run 3 waited before each candidate
until the PC had configured the device and sent nothing for 2 s, and
stamped each with its time:

| Candidate | PC's reaction in the 3 s after (1 s before: 0 resets, 0 setups) | Windows |
|---|---|---|
| **Host controller SE0 on port 2** (6.5 s in) | **2 resets, 32 setups**: a full enumeration of a new device; configured again | **"surprise removed, missing on the bus" at 11:14:43.58**, the only removal of the run, within a second of the step (the image's own start, ~6 s earlier, was a recovery without a removal) |
| `UP2OCR` 0 with the UDC off (13.7 s in) | 2 resets, 1 setup, **not configured** 3 s later | nothing logged |

The host controller's port 2 read `0x00010101` before (connected: our own
pull-up against the PC's pull-downs) and `0x00110103` after (reset done,
enabled): it drove the reset, and the PC's hub took the SE0 as an unplug.
Run 2 showed the same (2 resets, 32 setups) but overlapped the image's own
start. So **the X51v can make the PC see a disconnect without a pull-up
switch**: switch port 2's transceiver to the PXA27x's host controller
(`UP2OCR` = `HXS | HXOE | DPPDE | DMPDE`), bring the controller up (out of
`FHR` reset, `FSBIR`, ports out of sleep standby, `HcRhDescriptorA.NPS`,
an HCCA in RAM before it goes operational), set port 2's reset, then
switch back (`UP2OCR` = `HXOE`, controller held in reset) and enable the
UDC. Code: `boot/unplug`, `Rig::off`/`Rig::on` for `HostSe0`.

The transceiver-off step is no use: the PC never re-configured within 3 s.

**The fix (built 2026-10-16, emulator-checked, proven on the device: step B below).**
`Udc::unplug` (`drivers/pxa-udc/src/unplug.rs`, the measured sequence,
host-tested) runs in the loader through one helper, `replug`, everywhere
the loader presents the device: at start (in place of the `DETACH` wait,
which drove GPIO2 and did nothing) and at both layout switches (the
fallback and `drive on`), which had the same flaw: the controller off and
on is invisible to the PC. +368 bytes (rp-loader 116,276 → 116,644 of
118,784); ~155 ms more at start, spent with the PC ready to ask at once.
The emulator models the hard-wired pull-up and the unplug (retropane-qemu
branch `udc-unplug`, not yet released): `emu-usb`, `emu-loader`,
`emu-handover` and `emu-unplug` pass; with the unplug taken out of the
loader (mutant) `emu-usb` fails, the image's run log never coming back
over USB. Step A on the device (2026-10-16 12:01): the new loader sent from RAM
over the card's one: Windows removed the old device at 12:01:23.4 and had
the composite back ~2 s later (drive read, `udc stats`: 2 resets), so the
fix does no harm; it doesn't prove the unplug itself (a hand-over with the
PC's port open made Windows drop the device before the fix too).

**Step B, proven on the device (2026-10-16 12:04-12:13; the new loader on
the card, sha `9573c6be`):** five hand-overs from Windows Mobile
(`reboot-wm`, then the loader started from Windows Mobile, docked, no
re-plug). Each time Windows logged Windows Mobile's device (`413C:4011`)
removed the moment the loader started (12:05:45.8, 12:07:36.8, 12:09:07.3,
12:10:59.2, 12:13:10.2) and had the composite configured 0-3 s later; the
loader counted 2 resets each time, no fallback, the drive read. Before the
fix (T1, same morning) Windows Mobile's device stayed "present" for 5+
minutes and the loader saw 0 resets. The unplug also writes `UP2OCR`
whole (`HXOE` only), so the transceiver state a watchdog reboot can leave
(the 0-setup case) no longer survives either.

**Ranked hypotheses** (as first written, before T1: hypothesis 2 is now
the observed mechanism; hypothesis 1 is demoted, as nothing on the PC even
tried the loader).

1. **State Windows Mobile leaves in the controller** (lead). Every failure
   followed Windows Mobile (two soft resets, one watchdog reboot); every
   success followed a controller the loader itself had set up or turned
   off (image return, fallback, `drive on`). `Udc::enable` resets only
   part of the block: it clears `UDE`, writes the endpoint table and adds
   `HXOE` to `UP2OCR`, but
   - leaves `UDCISR0`/`UDCISR1` as found: a configuration-change status
     left pending would make the first `poll` run `config_change` with no
     request from the host (writes `AREN`, sets `SMAC`), and a stale
     endpoint 0 packet-complete would let `InData` send its next packet
     early (the "one packet in flight" row above);
   - leaves every `UDCCSRx` (A-X) as found: `DME` and `FST` bits that
     Windows Mobile's own function drivers set;
   - leaves `UP2OCR`'s other bits: after a full reset (the watchdog case)
     the register resets to `HXOE | HXS` (`0x00030000`, the host controller
     selected). If Windows Mobile hadn't set it up again before HaRET ran,
     the `HXOE` check passes and the device still can't answer: **27 resets
     and 0 SETUPs is exactly the symptom of the transceiver not driving
     for the device** (measured 2026-10-11 with `HXOE` off).
   Same class as the OS timer channel and GPIO edge state that survived
   earlier hand-overs (`docs/hw/emulator.md`) and as the CF slot ("whatever
   an earlier owner left is not trusted").
2. **Windows' retry budget while nothing answers.** The pull-up is
   hard-wired, so the PC sees a device attached all through Windows
   Mobile's boot and HaRET, while the UDC is off. Windows tries,
   fails, retries every ~15 s and gives up after about 7 tries (05:43,
   10-15 12:34-13:05); a port given up on is only retried after a real
   reconnect (2026-10-15: a different PC port was needed). It explains the
   silences and the port change, not why tries that reach a working loader
   fail; it makes any first-try failure (hypothesis 1) much costlier.
3. **The old composite device not yet gone.** Windows logs the drive
   interface's removal as vetoed at almost every hand-over (open handles:
   Explorer, the indexer). A device instance with the same ID
   (`5250323630303031`) that hasn't been torn down can block the new one;
   the serial-only layout has another ID (`RP26-0001`) and isn't blocked.
   Fits ADR-0014's 2026-10-15 case (Explorer open, ~20 MB written just
   before, 13 resets / 91 setups); no veto was logged before the 07:09 case.
4. *Unlikely:* a descriptor the host rejects (the composite was configured
   at once at 07:12:15); the loop stalling during start-up (the CF slot's
   408 ms power cycle on a retry would cost one try, not 25 s).

**Test plan** (each step one device run, only on the owner's go; stop at
the first that settles it):

- **T1, PC side only, no new code.** Trace Windows' USB stack across one
  soft reset into the card's loader. In an *elevated* terminal:
  `logman create trace rp-usb -o %TEMP%\rp-usb.etl -ets -p Microsoft-Windows-USB-USBHUB3 0xffffffffffffffff 0xff`,
  then `logman update trace rp-usb -ets -p Microsoft-Windows-USB-UCX 0xffffffffffffffff 0xff`;
  soft reset the Axim (`cargo xtask reboot-wm` instead of the reset hole
  while docked: also that command's first try on the device) and wait for the loader's screen to say which layout
  Windows took; `logman stop rp-usb -ets`. The `.etl` is read without
  admin (`Get-WinEvent -Path ... -Oldest`). It shows, per reset, which
  request failed and how (timeout, STALL, short or garbled data) and
  whether Windows was still trying while the loader was up. Timeouts or bad
  data while the loader runs → hypothesis 1; no tries reaching the loader
  until the fallback → 2; enumeration done but no driver → 3.
- **T2, what Windows Mobile leaves.** Add the UDC to `rp-probe` (it runs
  from HaRET, as Windows Mobile left the hardware, as on 2026-10-11):
  `UDCCR`, `UDCISR0/1`, `UDCICR0/1`, `UP2OCR`, every `UDCCSRx` and
  `UDCCRx`, `DRCMR` 24-47. Once after a soft reset, once after a watchdog
  reboot. A test image: no loader bytes.
- **T3, the fix (proposed, not built).** `Udc::enable` takes the block
  from a known state instead of patching what it finds: `UP2OCR` written
  whole (`HXOE` only: Windows Mobile's own value), `UDCISR0/1` cleared
  (write all ones), every endpoint's `UDCCSRx` flushed with `DME` and
  `FST` off, all before `UDE`. A few dozen bytes in the loader at most
  (the `UP2OCR` read-and-test goes). Host test: a fake bus preloaded with
  the state T2 finds, enumeration must succeed; the mutant (old `enable`)
  must fail. Device proof: five soft resets in a row, the composite
  configured each time without the fallback, then one watchdog reboot.
- If T1 points at hypothesis 3 instead: Windows tears the old instance
  down once the last handle on its drive closes, so the fix is on the
  drive's side (how the loader lets Windows let go before it comes back);
  designed only after T1.

- Streaming CPU-written IN packets into both buffers without the OUT race
  (see above): would let command replies go back to back; low value now that
  bulk data goes by DMA.
- A short OUT packet by DMA (the end-of-receive signal): not needed while
  every DMA transfer is whole packets of a known length.

- GPIO14 polarity (cable present = high?) and whether the cradle and the cable
  behave the same: `rp-usb` shows the level live.
- Whether HaRET's "shutting down hardware" leaves the UDC enabled; RetroPane
  resets it either way (UDE off, then a fresh table).
- Whether the X51v routes the UDC through port 2 (`UP2OCR`): add `UP2OCR` to
  the probe if enumeration fails.
