# RetroPane OS

A new, open-source operating system for the **Dell Axim X51v**: a modern take
on Windows Mobile.

- **Native drivers for all of the hardware**, reverse-engineered where no
  documentation exists, including the Intel 2700G 3D, 2D and video blocks and
  the built-in Wi-Fi.
- **Runs Windows Mobile 5 and 6.x apps** unmodified. DoomGLES, HereticGLES,
  Cube and Quake 3 are the benchmark.
- **Modern internet:** TLS 1.3, WPA2, IPv6, and a browser that does as much as
  possible on the device itself.
- **Faster than stock:** full 624 MHz, GPU-composited UI, DMA everywhere, and the
  goal of beating the original drivers.
- **Zero extra hardware:** an X51v, its stock USB cradle, the SD card already in
  it, and a PC. It boots from the SD card and overwrites nothing.

> **Status: Phase 1 (emulator).** No usable OS yet, but RetroPane's first
> image boots in the X51v emulator and drives the Intel 2700G display. See the
> [roadmap](docs/ROADMAP.md).

<img src="docs/img/emulator-test-pattern.png" width="160" alt="Test pattern on the emulated X51v screen">

Everything is judged through four lenses: 🔁 **iterate** · ✨ **modernize** ·
🗄️ **legacy support** · ⚡ **better/faster**.

## Documents

- [Design document](docs/DESIGN.md)
- [Roadmap](docs/ROADMAP.md)
- [Architecture decision records](docs/adr/)
- [Clean-room and reverse-engineering policy](docs/CLEANROOM.md)
- [Hardware inventory and prior work](docs/research/hardware-inventory.md)
- [Contributing](CONTRIBUTING.md)

## Building

You need [rustup](https://rustup.rs). The pinned toolchain installs itself.

```bash
cargo xtask ci      # fmt, clippy, architecture lint, unit tests, X51v cross-build
cargo xtask emu     # boot the bring-up image in the X51v emulator and check the screen
cargo xtask qemu    # (optional) build the emulator locally in WSL for emulator development
```

## License

GPL-2.0-or-later. See [LICENSE](LICENSE).

RetroPane is an independent clean-room project. It is not affiliated with or
endorsed by Microsoft, Dell or Intel, and it contains no code from them.
"Windows Mobile" is a trademark of Microsoft, used here only to describe
compatibility.
