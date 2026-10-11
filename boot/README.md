# boot/

Bare-metal images and how they get onto the X51v. **Nothing here ever writes
the Dell bootloader region** (DESIGN.md §10c).

- `link.x`: the shared image layout. Images load at `0xA8008000`, 32 KiB into
  the X51v's SDRAM (which starts at `0xA8000000`, docs/hw/memory-map.md).
- `hello/`: the bring-up image: display test pattern plus an on-screen boot
  log, SD card, buttons and touch.
- `bench/`: deterministic benchmarks (`cargo xtask bench`).

## Booting on a real X51v from the SD card

`cargo xtask sdkit` builds `target/sdkit/`: HaRET (built from pinned sources in
WSL), its start-up script, `retropane.bin` (the hello image as a raw binary)
and a `README.txt` with the steps. Copy the files to the SD card and run
`haret.exe` on the device. Nothing is written to flash; a soft reset returns to
Windows Mobile.

A RetroPane-native launcher replaces HaRET later in Phase 2.
