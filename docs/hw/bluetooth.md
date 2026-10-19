# X51v Bluetooth (TI BRF6150) and USB host: what Windows Mobile shows

> Spec v0.1, 2026-10-18. Facts only, per [`../CLEANROOM.md`](../CLEANROOM.md).
> **Sources:** (D) the owner's A08 ROM (`local/imgfs-a08`, `local/xip-a08`):
> the registry (`default.hv`), Dell's Bluetooth drivers' names and strings,
> TI's chip set-up script; read for facts, no code. (M) Intel PXA27x
> Developer's Manual. Nothing here has been run on the device yet.

## Bluetooth

| Fact | Confidence | Evidence |
|---|---|---|
| **The radio is a TI BRF6150** (single-chip Bluetooth **1.2**), ROM 2.12, **firmware 3.0.48** | confirmed (static) | (D) `TIInit_3_3_48.bts`: "BRF6150 2.12 ROM", "FW v3.0.48"; `BTHCI.dll` builds the file name `TIInit_%d_%d_%d.bts` from the chip's reported version |
| Bluetooth 1.2 means **no Low Energy, no Enhanced Data Rate, no Secure Simple Pairing**: every pairing on this radio is the old PIN pairing (E0 encryption) | confirmed (from the version; Bluetooth specification) | as above |
| It is wired to the PXA27x **BTUART**, used through Dell's `BTUartDMA.dll` (port `BTUart0`, DMA 6/7, SYSINTR 26) under Microsoft's stack (`BTHCI.dll`, the built-in transport, `COM5:`) | confirmed (static) | (D) registry `BuiltIn` transport → `BTHCI.dll`; `BTUart0` → `BTUartDMA.Dll`; interrupts.md |
| TI's script raises the link to **921,600 baud**; it also sets the radio's power tables, temperature thresholds and a voltage (VLDO) | confirmed (static) | (D) script comments |
| The script configures the chip's **PCM voice port** (clock as output, falling-edge polarity) and **"codec initial by I2C from BRF6150"** on its IO2/IO3 pins as SCL/SDA. So the radio may talk to an audio codec over I2C itself, and carry call audio over PCM | likely (TI's script; the board's wiring unknown) | (D) script comments |
| The audio driver's I2C lock `CI2C_Mutex_H` is shared with **another driver** (docs/hw/audio.md); the BRF6150's own I2C to a codec would be such a user | guess (fits; unproven) | audio.md, the script |
| `BTControl.dll` touches no hardware (its `BTC_PowerUp`/`BTC_PowerDown` are empty; it reads the registry, waits on an event and calls `BTHUTIL.dll` ordinal 1): **the radio's power and reset are in `BTuartDMA.dll`** | confirmed (code) | (D) static read, 2026-10-18 (ADR-0033 P-S0) |
| `BTHCI.dll` has a `WakeUpTimer` and `ResetDelay`: likely TI's low-power UART protocol (HCILL) and a reset wait | guess | (D) strings |
| Profiles in Dell's ROM: A2DP stereo (`bta2dp.dll`), headset/hands-free audio gateway (`bthasplugin.dll`), HID keyboards and mice (`bthhid.dll`, `bthidsvc.dll`, `kbdhid.dll`), phone book (`bthpbap.dll`), serial ports | confirmed (files) | (D) |

### The radio's power-up and power-down (static, `BTuartDMA.dll` `0x023C234C`, facts only)

Three small functions switch one CPLD bit each through Windows Mobile's
shared shadow (the same 10-way helpers as the other drivers: one sets, one
clears); the waits are busy loops on the OS timer.

**Power-up, in order:**

1. `CKEN` bit 7 on (the BTUART's clock).
2. GPIO42 and GPIO44 made inputs; 300 µs.
3. **CPLD `0x16` bit 3 set** (the radio's power, stage 1); 5 ms.
4. `OSCC` bits 1 and 2 set (the 32.768 kHz oscillator on and its output
   enabled), **GPIO10 made an output, function 1**: the PXA's 32 kHz clock
   output, most likely the radio's slow (sleep) clock.
5. **CPLD `0x10` bit 2 set** (power, stage 2); 300 µs.
6. GPIO42-45 to the BTUART's functions: 42 RXD (function 1), 43 TXD (2),
   44 CTS (1), 45 RTS (2); 300 µs.
7. **CPLD `0x18` bit 0 set: the radio out of reset** (the bit is the
   reset line, active low); 300 ms before the port is used.

**Power-down:** `0x18` bit 0 cleared (reset held); 300 µs; `0x10` bit 2
cleared; 300 µs; GPIO42-45 back to plain GPIOs driven low; 300 µs; `0x16`
bit 3 cleared; 50 ms; GPIO42 and 44 made outputs (low). The 32 kHz output
and the BTUART clock are left as they were.

| Fact | Confidence | Evidence |
|---|---|---|
| CPLD `0x16` bit 3 and `0x10` bit 2: the radio's two power stages; `0x18` bit 0: its reset (clear = held in reset) | likely (code; the roles from the order) | (D) `0x023C6F94`, `0x023C6FC0`, `0x023C6FEC` and their calls |
| GPIO10 (function 1, with `OSCC` TOUT_EN) feeds the radio a 32.768 kHz clock | likely (code; the PXA27x manual's GPIO10 output function) | (D), (M) |
| The BTUART on GPIO42-45 with hardware flow control (CTS, RTS) | confirmed (code) | (D) |

## USB host

| Fact | Confidence | Evidence |
|---|---|---|
| **Windows Mobile has no USB host drivers on the X51v**: only the function side (`usbfndrv.dll`, `SerialUSBFn.dll`); no OHCI, hub or class drivers | confirmed (static) | (D) file list and registry |
| The PXA27x has an OHCI host controller; its port 2 shares the sync connector's transceiver (`UP2OCR.HXS`), as the loader uses it to make the PC see an unplug | confirmed (device) | udc.md |
| **Whether the X51v can power a USB device** (5 V, VBUS, on the connector) is unknown: no driver ever needed it. If it can't, a powered hub or a cable with its own power is needed | unknown | to check: a multimeter on the connector with a host cable (owner), and the PXA27x's host power pins' functions as Windows Mobile leaves them (read-only probe) |

## Open questions

1. VBUS: can the X51v supply power on the connector, and is there a switch
   for it (a CPLD bit, a host power-enable pin)?
2. Is the BRF6150's PCM port wired to the WM8750 (which has one audio
   interface, already used by I2S) or unused?
3. ~~The BRF6150's power and reset lines~~ **answered (static, above):**
   CPLD `0x16` bit 3, `0x10` bit 2, reset `0x18` bit 0, the 32 kHz clock on
   GPIO10; to confirm on the device with the owner watching (board bits).
4. HCI transport framing (H4, or TI's HCILL on top) and the baud rate
   before the script runs.
