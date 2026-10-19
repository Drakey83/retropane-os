# Keyboards, mice and headsets (USB and Bluetooth)

**Owner (2026-10-18):** full driver support for **keyboards and mice over
USB and Bluetooth**, and **Bluetooth headsets** (stereo listening, and
calls with the headset's microphone). **The design is
[ADR-0033](../adr/0033-keyboards-mice-headsets.md)** (proposed 2026-10-18,
votes P1-P5 waiting for the owner); the hardware facts found since are in
[hw/bluetooth.md](../hw/bluetooth.md) (the radio is a TI BRF6150,
Bluetooth 1.2; Windows Mobile has no USB host drivers). This page keeps
the original requirement and questions.

## What the X51v gives us (known, and to check)

| Part | Known | To check |
|---|---|---|
| USB host | The PXA27x's OHCI host controller; its port 2 shares the sync connector's transceiver with the device controller (`UP2OCR.HXS` switches it: measured 2026-10-16, docs/hw/udc.md, where the loader uses it to make the PC see an unplug) | Whether the X51v supplies VBUS (5 V) to a device on the connector (a board switch, a CPLD bit?) or a powered hub / Y-cable is needed; the cradle vs the cable; ports 1 and 3 wiring |
| Bluetooth | A built-in module on a PXA27x UART (BTUART, ROADMAP phase 9); its power and reset among the CPLD's bits (docs/hw/cpld.md) | The chip and its Bluetooth version (pairing: PIN-only before 2.1 or Secure Simple Pairing), its UART transport (H4/BCSP), baud rate and firmware set-up as Windows Mobile does it; whether its PCM/SCO audio is wired to the codec or carried over HCI |

## What the design must cover

1. **One input layer**: USB and Bluetooth keyboards and mice reach programs
   the same way (key maps, repeat, a pointer for the shell and for old CE
   programs through the CE personality), beside the touch screen and the
   hardware keys (whose jobs stay as Windows Mobile's: memory
   retropane-buttons-and-explanations).
2. **USB host**: the OHCI driver (interrupts, DMA-safe buffers), hubs, the
   HID class (boot protocol first, report descriptors after), power for the
   device, hot plug.
3. **Bluetooth host stack**: the UART transport and the module's
   start-up, HCI, L2CAP, SDP, pairing and bonding stored safely
   (docs/security.md: an internet- and radio-connected device), HID
   (keyboards, mice), **A2DP** with the SBC codec into the audio service,
   **HFP/HSP** for calls and the headset's microphone, AVRCP (headset
   buttons: play, pause, volume).
4. **The audio service** (ADR-0032, proposed; Bluetooth outputs are its step S6) routes
   sound to the speaker, the headphones or a Bluetooth headset, and takes a
   headset's microphone; one session rule (I2S on through a session) holds.
5. **Battery**: radio and USB power policies, measured.

## Modern options to weigh (owner: "the best way, with 2026 engineering and modern software")

- **The built-in radio is ~2005 (Bluetooth 1.2/2.0, to confirm): no
  Bluetooth Low Energy.** Most current keyboards, mice and earbuds are BLE
  (4.0+), and LE Audio (LC3) needs 5.2. The built-in radio still reaches
  classic devices: older keyboards and mice, and A2DP/HFP headsets (most
  current headsets keep both).
- **A USB Bluetooth 5.x dongle** on the USB host port would bring BLE
  keyboards, mice and earbuds, and perhaps LE Audio, through the same host
  stack (HCI over USB instead of the UART). Weigh: VBUS power, battery,
  which dongles (a short tested list).
- **The stack:** our own in Rust (small, memory-safe, no_std, fitted to
  64 MB, every parser hardened: docs/security.md) against porting one
  (Zephyr's or Android's Bluetooth host: Apache 2.0, C; BlueZ: GPL and
  Linux-bound; BTstack: non-commercial licence). Compare size, safety,
  licence, profiles covered, test suites we can reuse.
- **Codecs:** SBC (mandatory), then AAC or LC3 for better sound where the
  CPU budget allows (measured on the device).
- **Iterate:** each step proven on the device and modelled in the emulator
  before the next (the project's rules), with the four lenses (iterate,
  modernize, legacy support, better and faster) applied per choice.

**Order (proposed, for the vote):** the hardware checks above (read-only and
under Windows Mobile first), then the audio service design (headsets need
it), then the input layer and USB HID, then the Bluetooth stack.
