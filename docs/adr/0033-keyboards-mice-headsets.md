# ADR-0033: Keyboards, mice and headsets over USB and Bluetooth

- **Status:** **Accepted (owner, 2026-10-18): P1 A, P2 A, P3 A, P4 A,
  P5 A**, with conditions that land before P-S1 (below: the radio routing
  policy and the per-transport profile allowlist, R10 and R11, and the
  owner's fourteen fixes, all written in the same day). R1-R9 are
  recommendations under the standing go-ahead, open to veto.
- **Date:** 2026-10-18

## In plain words

You asked for full support for keyboards and mice over USB and Bluetooth,
and Bluetooth headsets (docs/design/peripherals.md). Two findings from
Dell's ROM (docs/hw/bluetooth.md) shape everything:

1. **The built-in Bluetooth radio is a TI BRF6150: Bluetooth 1.2 from
   2005.** It still talks to most keyboards, mice and headsets made today
   that keep "classic" Bluetooth, but it has **no Bluetooth Low Energy**
   (what most new mice, many keyboards and nearly all earbuds use) and
   **only the old PIN pairing**, which is weak by today's standards.
2. **Windows Mobile never used USB host on the X51v** (no drivers at all),
   and **we don't yet know whether the Axim can power a USB device**. A
   modern Bluetooth 5 USB dongle would fix point 1, if USB host works.

Five questions are yours (P1-P5). The rest has a recommendation.

## The owner's decision (2026-10-18)

| Vote | Chosen | The owner's reason |
|---|---|---|
| P1 radios | **A: both** | Built-in for classic devices; a dongle when plugged in for Low Energy and secure pairing |
| P2 software | **A: our own, in Rust** | Parser bugs were the big Bluetooth attacks; ours are memory-safe by construction. Slowest to the first keyboard, worth it |
| P3 PIN pairing | **A: keyboards, mice and headsets only, with the plain notice** | Files and networking wait for the secure dongle |
| P4 mouse | **A: an iPad-style pointer** | Shows on mouse movement, hides on touch; right click = tap-and-hold menu; the wheel scrolls |
| P5 keyboard | **A: modern shortcuts** | The hardware buttons keep their Windows Mobile jobs; layouts, key repeat and Caps Lock lights included |

**Conditions, all before P-S1:** the radio routing policy (R10), the
per-transport profile allowlist (R11), and the owner's fourteen fixes:

1. The stack's work split honestly in two phases (R2).
2. Radio selection: built-in by default for classic, the dongle for
   LE-only devices, a per-device override, never both connectable (R10).
3. Audio prefers the built-in radio, the dongle is for LE input (R10); a
   plugged-in dongle and the USB PC link exclude each other (Consequences).
4. BTstack's licence corrected; the real blocker is C parsers (P2).
5. The PIN-pairing notice names everything a bond carries (P3).
6. R6 enforces R11's allowlist per transport (R6, R11).
7. A software pointer if the hardware cursor plane fails (P4).
8. Two tiers of shortcuts, global and contextual (P5).
9. SBC encoding's CPU cost measured on the device before A2DP is done (R5).
10. Call audio through the audio service, so the microphone permission
    holds (R5).
11. The USB role switch: host mode only when the PC link is idle or you
    confirm; a notice on every switch (R4).
12. P-S2 and P-S3 don't wait on P-S0's USB power question (Steps).
13. The KNOB wording stays as it is (R6; kept on purpose).
14. The dongle's power draw measured at P-S6 if the Axim powers it (R7).

## Context

| Fact | Consequence | Evidence |
|---|---|---|
| Radio: TI BRF6150, Bluetooth 1.2, firmware 3.0.48, on the BTUART at 921,600 baud, set up by TI's script `TIInit_3_3_48.bts` | Classic profiles only (HID, A2DP, AVRCP, HFP/HSP); no LE, no EDR (A2DP fits in the basic rate: SBC at high quality is ~330 kbit/s of ~720); PIN pairing only | bluetooth.md |
| TI's script is TI's binary, needed at every start of the radio | It can't go into the repository; RetroPane loads **the copy already in the device's own Windows Mobile ROM** (or the owner's backup of it), checked against a known hash | bluetooth.md; CLEANROOM.md |
| The BRF6150 may have its own I2C to the codec and a PCM voice port | Call audio may have a hardware path; to settle before HFP | bluetooth.md open question 2 |
| USB: OHCI host controller; port 2 shares the sync connector with the device side (the loader already switches it, `UP2OCR.HXS`) | Host and device can't run at the same time on that connector: a keyboard plugged in means no USB link to the PC until it's removed | udc.md |
| VBUS (5 V to a device) unknown | If the Axim can't power it, a powered hub or a powered host cable is needed | bluetooth.md open question 1 |
| The 2.4 GHz band is shared with the ACX100 Wi-Fi | Bluetooth audio and Wi-Fi together may stutter; coexistence lines unknown | hardware-inventory.md |
| ADR-0032: the audio service runs at 44.1 kHz with outputs as `Output`s | A2DP's SBC takes 44.1 kHz directly: **no conversion for Bluetooth headsets** | ADR-0032 A1 |

## The owner's votes

### P1. Which Bluetooth radio(s)

| | **A: both: built-in for classic, a USB dongle when plugged in (recommended)** | B: built-in only | C: dongle only |
|---|---|---|---|
| Classic keyboards, mice, A2DP headsets | Yes (built-in) | Yes | Yes |
| Low Energy keyboards, mice, earbuds; modern secure pairing | Yes, with the dongle | **No** | Yes |
| Extra hardware | Optional dongle (and maybe a powered hub, P-S0) | None | Always a dongle |
| Work | Two phases (R2): classic Bluetooth over both transports first, then the Low Energy stack (dongle only), a second stack in all but name | Less (classic only) | Both phases, minus the TI start-up |
| Risk | The dongle path depends on USB host power (unknown) | Leaves most new devices out | Useless if USB power fails |

### P2. Whose Bluetooth software

| | **A: our own, in Rust (recommended)** | B: port Zephyr's host stack (C, Apache 2.0) | C: BTstack (C) |
|---|---|---|---|
| Safety | Memory-safe parsers on the radio's attack surface (the BlueBorne and BrakTooth bug classes are parser bugs) | Mature, but C parsers we'd have to harden | Mature, C |
| Size for 64 MB | Only the profiles we use; measured | Moderate; built for microcontrollers (good fit) | Small |
| Licence | Ours | Fine | Free for non-commercial use (BlueKitchen's terms), so a hobby OS likely needs no commercial licence; **the real blocker is C parsers on the attack surface** |
| Classic profiles (HID, A2DP, HFP) | We write them | **Zephyr's classic support is thin** (LE first) | Complete |
| Time to first keyboard | Longest | Medium | Short |
| Testing | Against Google's open **Bumble** stack as a scripted peer (R8) | Same | Same |

### P3. The old PIN pairing on the built-in radio

Every device paired through the BRF6150 uses Bluetooth 1.2's PIN pairing:
anyone recording the pairing nearby can work out the key. Over the dongle
(P1-A) pairing is modern.

| | **A: allow it for keyboards, mice and headsets, with a plain notice; nothing else (recommended)** | B: allow everything | C: refuse it |
|---|---|---|---|
| What works on the built-in radio | Input and audio | Also file transfer, serial, networking | Nothing (dongle only) |
| Plus | The devices you asked for work without a dongle; the data that matters most (files, network) only over secure links; legacy devices aren't locked out | Most compatible | Strongest |
| Minus | Someone who recorded the pairing can read everything that bond ever carries: typing, **passwords**, **voice calls** | Files and network on a weak link | The built-in radio is wasted |

The notice under A (owner's fix 5), shown before every PIN pairing:
**"Pair somewhere private. Anyone who recorded this pairing can read
everything this device ever sends over it: your typing, your passwords
and your voice calls."**

### P4. A mouse on a touch-screen OS

| | **A: a pointer while the mouse moves, hidden at the first touch (recommended)** | B: no pointer (as Windows Mobile; the mouse acts like the stylus) |
|---|---|---|
| How | As iPads do: a small pointer appears with mouse movement, fades when idle, vanishes when you touch the screen. Left click = tap; **right click = tap-and-hold menu**; the wheel scrolls | Clicks land where the pointer would be, invisible |
| Plus | Usable; hover highlights possible later; old programs still see stylus taps | Nothing new to draw |
| Minus | One sprite to draw (the 2700G's display controller has a hardware cursor plane, `HCCTRL` and friends: named in 2700g.md, untested; if it works, no redraw cost). **If it fails testing: a software sprite through the 2D blitter, one small blit per motion event** (owner's fix 7) | Unusable without seeing where you are |

### P5. Keyboard shortcuts

| | **A: modern shortcuts, the hardware keys unchanged (recommended)** | B: plain typing only |
|---|---|---|
| How | Start/Windows key opens Start; Alt+Tab switches programs; Ctrl+C/X/V/Z (contextual tier, below); media keys (play, volume) go to the audio service; Esc = back/cancel; arrows and Enter drive the D-pad's actions. The Axim's own buttons keep Windows Mobile's jobs (memory: no new button jobs) | Keys type; nothing else |
| Plus | A keyboard makes the device feel like a small laptop | Simple |
| Minus | A shortcut list to learn (shown in Settings) | Wastes the keyboard |

**Two tiers** (owner's fix 8):

- **Global**, always taken by the shell, whatever has focus: Start
  (Windows key), Alt+Tab, the media keys, Esc as back.
- **Contextual**, Ctrl+C/X/V/Z: passed through to the focused program
  (old programs get them as ordinary keys, as on a desktop), and handled
  by the shell only when one of the shell's own text fields has focus.

Also part of A: keyboard **layouts** (US default; others selectable),
**key repeat** settings, and Caps/Num lock lights driven back to the
keyboard.

## Recommendations (engineering)

### R1. One input layer, one HID parser for both transports
USB HID and Bluetooth HID carry **the same report descriptors and
reports**, so one hardened parser (no_std Rust, fuzzed, size-capped)
serves both; boot protocol as the fallback for simple keyboards and mice.
Keyboards produce key events beside the Axim's keypad; mice produce
pointer events beside touch; old programs see ordinary key and stylus
input through the CE personality, so they work unchanged.

### R2. The Bluetooth host stack, by layers, in two phases
"One stack, two transports" understates it (owner's fix 1). **Phase 1,
classic Bluetooth (BR/EDR) over both transports:** the built-in radio
and, when present, a dongle. **Phase 2, the Low Energy stack (dongle
only):** its own link layer handling, ATT, GATT, the Security Manager and
HID over GATT, in practice a second stack beside the first that shares
only the transports, HCI and the bond store. Each phase is planned and
measured as its own piece of work.

A transport trait (**UART H4** for the BRF6150 with TI's start-up script
and its low-power UART protocol if confirmed; **USB** for a dongle), then
HCI, L2CAP, SDP, pairing and bonding; on top: **HID** (classic), **A2DP**
(AVDTP + an SBC encoder in fixed point, iWMMXt) as an ADR-0032 `Output`,
**AVRCP** (headset buttons: play, pause, volume, which become media
keys), **HFP/HSP** (calls and the headset's microphone, for VoIP later).
Low Energy (GATT, HID over GATT) only on a dongle; **LE Audio (LC3)** only
on a dongle that supports it, later.

### R3. Where it runs
The Bluetooth host stack in **its own process** inside the connectivity
class (ADR-0016 D5), not shared with Wi-Fi or USB: it parses whatever a
nearby stranger transmits. USB host and HID in the connectivity host.

### R4. USB host
An OHCI driver (interrupt-driven, DMA-safe buffers from the shared
region), hub support (needed for a powered hub), the HID class, and
**the connector's role**: device mode by default (PC link); host mode when
a host cable or hub is detected (how: P-S0), never both. **The switch
policy** (owner's fix 11): host mode is taken only when the PC link is
idle (no transfer, no sync, no loader command in flight), otherwise only
after you confirm on screen; **the shell shows a notice on every switch**,
either way.

### R5. Audio over Bluetooth (with ADR-0032)
A2DP output at 44.1 kHz SBC, high-quality setting (bitpool 53), no rate
conversion; the headset's volume follows A3's per-output level (AVRCP
absolute volume where the headset supports it). HFP: narrowband 8 kHz
(the 1.2 radio has no wideband speech); the microphone under ADR-0032 A5's
permission. **Before A2DP is called done, two measurements on the device:
the fixed-point SBC encoder's CPU cost (owner's fix 9) and Wi-Fi
coexistence.**

**Call audio's route** (owner's fix 10): the headset's voice (SCO) is
carried **over HCI, through the audio service**, so ADR-0032 A5's
microphone permission applies to it like any other microphone. The
BRF6150's hardware PCM port (if it is wired to the codec: bluetooth.md
open question 2) is used only if HCI-routed SCO can't be made to work,
and then the gap (audio that bypasses the service and its permission
check) is documented and shown in Settings.

### R6. Security
- Discoverable only while you're pairing, for two minutes; never by
  default.
- Bonding keys stored encrypted with the device's own key (ADR-0029's
  random numbers), never readable by programs.
- HID input accepted only from bonded devices; a "new keyboard" must be
  confirmed on screen (the attack where a device pretends to be a
  keyboard and types commands).
- **The profile allowlist per transport (R11) is enforced here**, in the
  stack, since the profiles live in it: the built-in radio runs only HID,
  A2DP, AVRCP and HFP/HSP (owner's fix 6); a connection asking for
  anything else on that transport is refused before its parser runs.
- The minimum encryption key size enforced wherever the controller lets
  the host check it (the KNOB attack); BIAS-style downgrade checks. (The
  owner's review kept this wording on purpose, fix 13: on Bluetooth 1.2
  the host may not be able to check, and the ADR shouldn't claim more.)
- Every parser fuzzed in CI (descriptors, SDP, L2CAP, AVDTP).

### R7. Power
The radio stays off until a device is bonded or you open Bluetooth;
connected and idle, the BRF6150's low-power UART mode (if confirmed); a
USB device's power is the hub's, not the Axim's battery, unless P-S0 shows
VBUS. Measured per step; **if the dongle runs from the Axim's battery,
its draw is measured at P-S6** (expected 50-100 mW, set against the power
budget; owner's fix 14).

### R8. Testing and the emulator
The emulator's BTUART connects to **Bumble** (Google's open-source
Bluetooth stack in Python, Apache 2.0) acting as the controller and as
virtual keyboards, mice and headsets: every layer is tested in CI without
a radio. USB host: the fork's OHCI model with QEMU's USB keyboard and
mouse. Device runs (owner present) with a real keyboard, mouse and
headset at each step.

### R10. Radio routing (owner's condition)
With both radios present, each device goes to one radio by a fixed
policy, never by chance:

| Device | Radio | Why |
|---|---|---|
| Headsets (A2DP, AVRCP, HFP/HSP) | **The built-in radio** (owner's fix 3); the dongle only if the headset is Low Energy only (LE Audio, later) | Classic A2DP works there; it leaves the dongle's air time to input and doesn't make audio depend on USB power |
| Classic keyboards and mice | **The built-in radio by default** (owner's fix 2) | Works with no extra hardware |
| Low Energy keyboards and mice (HID over GATT) | **The dongle** (the built-in radio can't) | — |
| File transfer, networking | **The dongle only** (P3) | Secure pairing required |

- **A per-device override in Settings** (owner's fix 2): a classic
  keyboard can be moved to the dongle, for its secure pairing.
- **Never both connectable for the same device:** a device is bonded to
  one radio at a time (its bond belongs to that radio's address); moving
  it removes the old bond before the new pairing.
- When the dongle is pulled, its devices disconnect; they don't fall back
  to the weak pairing silently.

### R11. A profile allowlist per transport (owner's condition)
Each radio has a fixed list of profiles it will run, enforced in the host
stack (a connection asking for anything else is refused before its
parser runs):

| Transport | Allowed | Refused |
|---|---|---|
| Built-in BRF6150 (PIN pairing only) | **HID (classic), A2DP, AVRCP, HFP, HSP** (owner's fix 6); SDP only as the service lookup those profiles need to connect, advertising nothing else | OBEX (file transfer, phone book), PAN/BNEP (networking), RFCOMM serial ports, everything else |
| USB dongle (secure pairing) | The above, plus HID over GATT (Low Energy), and later, each when it's built: OBEX, PAN, LE Audio | Anything not built and reviewed |

The allowlist is data in one place (not checks scattered through the
stack), tested by a fuzz target that offers every profile on each
transport, and a mutant that removes one entry.

### R9. Old programs
Keyboards and mice reach old programs as normal input (R1). Windows
Mobile's own Bluetooth programming interfaces (Winsock `AF_BTH`, `Bth*`
calls) are lower priority, done if a program the owner uses needs them.

## Steps

0. **Before P-S1:** R10 and R11 designed in detail and tested in the
   host stack's skeleton (the owner's fourteen fixes are written in).
1. **P-S0 facts (read-only and owner checks):** VBUS on the connector (a
   multimeter, owner); the host power pins as Windows Mobile leaves them;
   static read of `BTControl.dll`/`BTHUTIL.dll` (the radio's power and
   reset lines); the PCM question; the host cable or hub to use.
2. **P-S1 USB host and a USB keyboard** (if P-S0 finds power, or with a
   powered hub): OHCI, hub, HID boot protocol; the input layer (R1); P4
   and P5.
**P-S2 and P-S3 don't depend on P-S0's USB power question** (owner's
fix 12): the built-in radio needs no USB, so built-in Bluetooth goes
ahead whatever P-S0 finds; only P-S1 and P-S6 wait on it.

3. **P-S2 the BRF6150 starts:** power-up, TI's script from the device's
   own ROM, HCI at 921,600 baud; Bumble in the emulator.
4. **P-S3 L2CAP, SDP, pairing (P3), Bluetooth HID:** a Bluetooth keyboard
   and mouse.
5. **P-S4 A2DP and AVRCP:** a headset as an ADR-0032 output (after audio
   S2, the mixer).
6. **P-S5 HFP/HSP:** the headset's microphone.
7. **P-S6 the USB Bluetooth dongle:** the same stack over USB; Low Energy
   keyboards, mice and earbuds; LE Audio later.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Each step a device run with a real device; Bumble makes every layer testable in CI |
| ✨ Modernize | Low Energy and secure pairing through a dongle; a real pointer; modern shortcuts; headset buttons as media keys |
| 🗄️ Legacy support | Old programs get keyboard and mouse as ordinary input; classic devices work on the built-in radio; legacy PIN pairing allowed where it's safe enough (P3) |
| ⚡ Better / faster | One parser for two transports; no conversion for Bluetooth audio (44.1 kHz end to end); a hardware cursor if it tests out |

## Hardware limit or choice?

| Limit | Silicon or choice? |
|---|---|
| No Low Energy, PIN pairing only on the built-in radio | **Silicon** (Bluetooth 1.2) |
| No USB host in Windows Mobile | **Choice** (Dell shipped no drivers); the controller exists |
| Host and device can't share the connector at once | **Wiring** (one transceiver, port 2) |
| VBUS | Unknown (P-S0) |

## Alternatives considered

- **Windows Mobile's Microsoft stack and Dell's drivers:** closed binaries;
  RetroPane loads no Windows drivers (DESIGN.md G1).
- **Linux's BlueZ:** GPL and built around the Linux kernel; not portable to
  our kernel.
- **Android's stack (Fluoride/Gabeldorsche, Apache 2.0):** complete, but
  large C++ built for phones with far more memory.
- **No Bluetooth on the built-in radio, dongle only:** P1-C, in the vote.

## Consequences

- TI's start-up script is read from the device's own ROM at run time; a
  device whose ROM was wiped needs the owner's backup.
- The connectivity host grows a Bluetooth process (its memory budget set
  at P-S2).
- ADR-0032's `Output` gains A2DP and HFP outputs; AVRCP feeds media keys.
- P-S0 needs the owner (a multimeter on the connector) before USB host can
  be designed in detail.
- **A plugged-in USB dongle and the USB link to the PC exclude each
  other** (one connector, one transceiver: host or device, never both).
  While the dongle is in, there is no LegacySync, no loader link and no
  USB drive; Bluetooth headsets keep working on the built-in radio.
