# ADR-0012: The USB device link (CDC-ACM first)

- **Status:** Proposed (2026-10-11; built ahead of acceptance at the owner's request, "start on the USB link")
- **Date:** 2026-10-11

## Context

Every change tested on the real X51v so far has cost the owner 5–10 minutes:
build, Bluetooth the file over, move it onto the card, run HaRET, photograph
four pages, and have the photos checksum-verified. Bring-up of every driver
still to come (power, audio, Wi-Fi, 3D) needs hundreds of such runs. DESIGN
§10b already decides the answer: **no custom cable for anyone**. The stock
Dell sync cable/cradle and drivers built into Windows 11, macOS and Linux are
the link, and the UDC driver is a Phase 2 priority.

Facts (docs/hw/udc.md):

- The PXA27x UDC is USB 1.1 full speed (12 Mbit/s, about 1 MB/s of bulk data
  in practice): a 40 KB image moves in well under a second.
- The hardware answers SET_ADDRESS, SET_CONFIGURATION and SET_INTERFACE by
  itself. The endpoint table (endpoint number, type, direction, configuration,
  interface) is fixed before the UDC is enabled. Software handles descriptor
  requests and class requests on endpoint 0, and moves data through FIFOs.
- GPIO2 switches the D+ pull-up (the host sees the device only when it's on),
  and GPIO14 detects the cable. Both were in the state WM uses on the probe
  run.
- The emulator has no UDC model yet.

## Decision

1. **A `rp-drv-pxa-udc` driver** in two layers:
   - `usb`: hardware-independent USB device logic. It parses SETUP packets,
     answers standard and CDC requests, and holds the descriptors as `const`
     byte arrays (no allocation, nothing built at run time). It is fully
     tested on the host.
   - `udc`: the PXA27x register layer, generic over `RegisterBus` +
     `RegisterBus8` like every driver. It is **polled first** (`poll()` called
     from the main loop or a thread), then moved onto an interrupt line
     (ADR-0007) once it works on hardware. It is tested on the host against a
     **simulated UDC with a scripted host** that enumerates the device the
     way Windows does.
2. **The first function is CDC-ACM** (a virtual COM port). Windows 10/11 bind
   the built-in `usbser.sys` to it with no driver or INF file, and macOS and
   Linux do the same with their built-in drivers. Device class 0x02, one
   communication interface (an interrupt IN notification endpoint) and one
   data interface (bulk IN + bulk OUT, 64-byte packets). Composite
   NCM/MTP functions come later (DESIGN §10b), on the same driver.
3. **USB IDs:** pid.codes' open-source vendor ID `0x1209` with its testing
   product ID `0x0001` during development, *to be verified on pid.codes and
   replaced by an assigned product ID before any release*. No purchase is
   needed (pid.codes assigns free IDs to open-source hardware projects).
4. **The development loop, in steps:**
   - **U1 (this change):** `rp-usb`, an SD image that enumerates as a COM port,
     echoes what it receives and shows its USB state on screen (cable detect,
     resets, configuration, bytes each way). `cargo xtask usb` finds the port
     by VID/PID and talks to it. *First device test needs the owner once.*
   - **U2:** every image's console goes to USB as well as the screen.
   - **U3:** a resident loader: the PC sends a new image over USB, the
     loader checks it (length + CRC-32) and jumps to it. Together with the
     PXA watchdog (an automatic reset after a hang) and HaRET started from
     WM's StartUp folder, most device runs then need nobody at the device.
   - **U4:** interrupt-driven, DMA, composite NCM (networking) and MTP (files).

## Implementation status (2026-10-11)

- **U1 built, device test pending:** `rp-drv-pxa-udc` (15 tests against a
  simulated host), `rp-usb`, `cargo xtask usb`.
- **U3 built, emulator-tested:** `rp-loadproto` (framing + CRC-32),
  `rp-loader` (moves itself to `0xA8400000`, reserved in boot/link.x and
  memory-map.md), `handoff::launch` (caches and MMU off, as HaRET leaves
  them), `cargo xtask load`. `cargo xtask emu` boots the loader as a raw
  binary away from home, sends it first light over the serial port, and
  checks first light runs. Over USB it needs U1 to work on the device first.
- **Run log + return (in place of U2's per-image USB console):** an image
  started by the loader also writes its output to a 64 KiB RAM log at
  `0xA8500000`, and when it finishes or crashes it jumps back to the loader
  (`devices::return_to_loader`), which sends the log to the PC ("RPL LOG" …
  "RPL END") once the PC has reopened the port. Images need no USB code, and
  `cargo xtask load <image>` prints the run's output. `rp-light` does this
  (its 2700G register walk, answered on the device, was removed to fit its
  budget); the emulator test checks the whole round trip.
- **Not yet:** the run log and return path in the other images (`rp-probe`
  needs ~1 KB it doesn't have in its budget), the watchdog for hangs, and
  starting HaRET from WM's StartUp folder.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Turns a 5–10 minute manual device run into seconds, unattended for most runs. The `usb` layer is host-tested exhaustively, the register layer against a simulated host; the first device run checks the remaining facts (GPIO14 polarity, HaRET's UDC state) on screen |
| ✨ Modernize | Plug-and-play on Windows 11, macOS and Linux with their built-in drivers: no WMDC, no ActiveSync, no signed drivers. Stock WM needed WMDC, which no longer works properly on Windows 11 (seen on the owner's PC) |
| 🗄️ Legacy support | The same Dell cable and cradle. Later, an ActiveSync-compatible RNDIS function is possible on the same driver if anyone needs desktop sync |
| ⚡ Better / faster | ~1 MB/s versus Bluetooth OBEX's ~70 KB/s and a manual copy. The descriptors are `const` data in flash, and the driver keeps a few hundred bytes of buffers in `.bss` (Memory first) |

## Alternatives considered

- **UART cable (connector pins 10/11/12, 3.3 V):** simpler software, but
  needs a cable nobody has and a level-correct adapter. It stays optional
  (DESIGN §10b), not part of the plan.
- **RNDIS/NCM first** (networking, then telnet or HTTP): more useful in the
  end, but far more code before the first byte moves. CDC-ACM is the smallest
  thing that gives a console and a loader.
- **Bluetooth (OBEX/SPP):** works today for file pushes, but needs pairing,
  a Bluetooth stack in RetroPane, and is ~15× slower.

## Consequences

- One more first-device-run unknown (does it enumerate?), shown on screen so
  a single photo answers it.
- The endpoint table is fixed at enable time. Changing the function set later
  (adding NCM/MTP) means a new table and a re-enumeration, which is the normal
  USB behaviour anyway.
- `cargo xtask` gains a serial-port dependency on the PC side, written with
  the standard library only (Windows `\\.\COMn` files), so no new crates.
- A UDC model in the emulator (retropane-qemu) is still worth having for CI.
  Until it exists, the simulated host in the driver tests stands in.
