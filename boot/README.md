# boot/

Bare-metal images and how they get onto the X51v. **Nothing here ever writes
the Dell bootloader region** (DESIGN.md §10c).

- `link.x`: the shared image layout. Images load at `0xA8008000`, 32 KiB into
  the X51v's SDRAM (which starts at `0xA8000000`, docs/hw/memory-map.md).
- `hello/`: the bring-up image: display test pattern plus an on-screen boot
  log, SD card, buttons and touch.
- `bench/`: deterministic benchmarks in the emulator (`cargo xtask bench`).
- `suite/`: the benchmark suite itself, shared by `bench` and `probe`.
- `kernel/`: the kernel image. So far it runs the VM layer live: its own
  page tables, real page faults resolved in an abort handler (demand-zero,
  file pages, copy-on-write), FCSE slot switching, and a full unmap.
  `cargo xtask emu` checks every step.
- `threads/`: the threads test image: scheduler, context switch, events,
  sleeping and the tickless idle alarm, live (one test image per subsystem,
  each with its own size budget).
- `probe/`: the real-device hardware probe. The X51v has no serial port, so
  it shows everything RetroPane needs to learn on screen, in 4 pages for a
  phone photo, each line ending in a checksum: CPU/clock/memory state, how
  Windows Mobile left the interrupts, GPIOs and 2700G, and the benchmark
  suite (including SRAM vs SDRAM latency and TLB-miss cost with the page
  table in SDRAM vs SRAM, for ADR-0006). `cargo xtask emu` boots it and
  verifies every checksum.

- `loadproto/` and `loader/`: the resident loader (ADR-0012) and its wire
  protocol. Started once by HaRET, it takes images from the PC over USB
  serial (`cargo xtask load`) and, since ADR-0013, files for the SD card
  (`cargo xtask put`, `cargo xtask install`): the loader, HaRET and its
  script are updated over the cable, nobody copies files.
- `noise/`: the random number generator's noise sources measured
  (ADR-0029): clock set-up, two-crystal and one-clock jitter, the touch
  controller's battery, temperature and spare inputs; raw samples to the
  card for NIST's tools (docs/hw/entropy.md).
- `light/`, `usb/`, `reconnect/`: first light, the USB link test and the
  reconnect experiment, each a bring-up image with its own budget.

## Booting on a real X51v from the SD card

`cargo xtask sdkit` builds `target/sdkit/`: HaRET (built from pinned sources in
WSL), its start-up script, `retropane.bin` (the probe image as a raw binary,
labelled with the commit it was built from) and a `README.txt` with the
steps. Copy the files to the SD card and run
`haret.exe` on the device. Nothing is written to flash; a soft reset returns to
Windows Mobile.

A RetroPane-native launcher replaces HaRET later in Phase 2.
