# Sheet: internet over the dock's USB cable (R132), 2026-10-30

**What it decides.** How the X51v reaches the internet through the PC it is
docked to, before Wi-Fi exists, so the Weather app can fetch with the
owner's Tempest key (and Open-Meteo). The owner, 2026-10-30: "I want full
pass through of internet because we will use it a lot for testing if we are
going to wait till the end to do wifi"; later the same night: "I also want
internet to work over the dock so that I can test out my tempest API key".
ADR-0028 L4 already decided pass-through **yes, off by default**, turned on
per PC in LegacySync; this sheet decides **how**.

**No code until the owner's go** (the alternatives-sheet gate).

## Rule 0

- **Windows Mobile's way (a finding; Microsoft's CE 6.0 docs, "ActiveSync
  Desktop Pass-through (DTPT)"):** the USB cable carries an RNDIS network
  link (the device 169.254.2.1, the PC 169.254.2.2). On the device a
  Winsock layered provider sends each outbound TCP connect, and a name
  provider each DNS lookup, to the **DTPT server inside ActiveSync** (port
  5721), which opens the real connection and relays the bytes. Only
  outbound IPv4 TCP; one device at a time. Needs ActiveSync or WMDC on the
  PC, which Windows 11 no longer runs.
- **Dell's way: REA search owed** before the go (the sheet rule: an address
  or a logged no-result search in `local/re/rea-search-log.md`). Expected
  from the ROM's layout: Dell ships Microsoft's ActiveSync stack and no
  networking of its own over USB; to be confirmed, not assumed.
- **The modern way:**
  - **USB NCM + the PC's connection sharing.** Windows 10 1903+ and Windows
    11 bind a CDC-NCM device to their inbox `UsbNcm.sys` with no driver
    install (Microsoft's "Supported USB classes"; its source is
    `microsoft/NCM-Driver-for-Windows`). The device then needs a full TCP/IP
    stack, and the PC's Internet Connection Sharing: an admin setting that
    rewrites the PC's network (and often fights VPNs).
  - **A relay in user space** (Android's `adb reverse`, QEMU's user
    networking, `passt`): the PC program makes the connections for the
    device; no admin rights, no PC network change, it follows the PC's own
    VPN and proxy.
  - **End-to-end TLS 1.3 on the device** whatever carries it, so the PC
    sees only ciphertext.

## The alternatives

| | What | Wins | Loses |
|---|---|---|---|
| A. DTPT as Windows Mobile did | RNDIS + a connect/DNS relay in the PC | Proven design; old WM programs' expectations | RNDIS is deprecated on Windows 11 (no inbox binding for new devices); needs a TCP/IP stack on the device anyway for RNDIS |
| B. NCM + ICS | The device a full IP host behind the PC's sharing | Full internet: UDP, inbound, everything | An admin change to the owner's PC network; ICS breaks with VPNs; the device needs TCP/IP + DHCP + DNS before the first byte |
| C. NCM + a user-space NAT in LegacySync | As B, LegacySync translating (slirp/passt style) | Full IP, no admin | The largest build on both sides (a TCP/IP stack on the device, a NAT on the PC) |
| **D. Ours: a socket relay over our own USB function** | A vendor interface with Microsoft OS 2.0 descriptors (Windows 11 binds WinUSB itself, no driver install), streams multiplexed on it; LegacySync opens TCP connections and does DNS for the device; the device runs TLS 1.3 end to end | **No admin, no PC network change, no TCP/IP stack on the device** (the cable is already reliable and ordered); follows the PC's VPN/proxy; off by default and per partnership (ADR-0028 L4/L5); the smallest build that fetches Tempest and Open-Meteo | Outbound TCP only at first (DTPT's own limit). UDP comes as a relay channel when needed (Tempest's LAN broadcast on port 50222). Old WM programs that open raw Winsock get the same relay through the CE personality later |

**Recommended: D.** It is DTPT's idea (the PC makes the connections) made
modern: WinUSB instead of RNDIS, no ActiveSync, TLS ending on the device,
the trust and the switch in LegacySync.

## What it takes (each step: host tests, the emulator, a device run on "run it")

1. **Dell's REA search** (the gate) and the TLS choice: `embedded-tls`
   (embassy-rs, no_std, no allocator, TLS 1.3 client) against rustls
   (no_std needs an allocator). The deciding point is **certificate
   checking**: never ship a client that doesn't verify (`docs/security.md`).
   Measured: the handshake's time and stack on the PXA270.
2. **The device's USB function** next to MTP (ADR-0012's composite, U4):
   the vendor interface, MS OS 2.0 descriptors, the stream framing (open
   host:port, data, close, errors), fuzzed like every parser.
3. **LegacySync's relay**: the switch (off by default), the partnership
   check (L5), DNS and connects on the PC's own network, a log of what the
   device opened. Time sync on connect too (L4: TLS needs a right clock).
4. **TLS + a small HTTP/1.1 client** on the device; the Weather app's
   fetch of Tempest (`swd.weatherflow.com`, the owner's key from the device's
   key store, never the repo) and Open-Meteo, replacing the sample.
5. Device run: the owner's real forecast on the X51v.

## What overturns D

- The certificate check can't be made sound on the device in a reasonable
  size: then TLS waits, and nothing ships that sends the key in clear.
- Something needs UDP or inbound first (a LAN station): the relay grows a
  UDP channel, or C is reopened.

## The owner's ruling

- **R132's way: D** (the owner, 2026-10-30 late: "D"). Next: Dell's REA search (the sheet's step 1, its result written here), then the TLS choice, then the build.
