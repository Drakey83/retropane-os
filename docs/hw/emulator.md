# The emulator and the real hardware

The emulator (`Drakey83/retropane-qemu`, a QEMU fork with X51v models) is
only useful while it behaves like the device. **Rule:** every fact learned
on the real X51v that a model could reproduce goes into the fork, with a
test in `cargo xtask emu` that fails without it; what can't be modelled is
listed here with the reason, so nobody trusts the emulator for it. Each row
names the finding, where it is written up, and the emulator's state.

| Finding (date) | Written up | Emulator |
|---|---|---|
| UDC: software sends the status IN for host-to-device control transfers; HXOE must stay set; one packet in flight; only the active interface's endpoints work; IN `UDCCSR` bit 6 = "not full" (2026-10-11) | `udc.md` | Modelled (rp-v0.7.0, `hw/misc/pxa27x-udc.c`), tested by the loader-over-USB boot |
| MMC: the CMD12 ending a multi-block **write** needs `CMDAT[10]` stop-transmission, else the controller never issues it (2026-10-11) | `sd.md` | Modelled (rp-v0.8.0): the model issues a stop command once the transfer is finished; upstream never issued it. Tested by the file-to-card boot |
| SD card: still busy programming after the CMD12 that stops a write; the next command gets no response unless the driver polls CMD13 (2026-10-11) | `sd.md` | Modelled (rp-v0.8.0): the emulated card answers the first CMD13 polls after a write with the `prg` state and "not ready for data", so a driver without the poll fails in CI as on the device (checked: with the poll removed, the suite reports the device's exact reply, `timeout on CMD24 stat 0x102`) |
| USB hosts wait 10 ms after a bus reset before the first request (USB 2.0 reset recovery) (2026-10-12, specification; found as an emulator race) | `udc.md` | Modelled (rp-v0.9.0). The driver also keeps a SETUP that arrives before it handles the reset, tested in `pxa-udc` with a mutation that makes it fail |
| A composite device's functions are told apart by interface class in the configuration descriptor; storage works with no program on the serial port (2026-10-12, how Windows binds IAD composites) | ADR-0014 | Modelled (rp-v0.9.0): descriptor-driven routing, Get Max LUN, the drive bridged to a second chardev. Tested by the drive boot |
| SD card: 3.85 ms per single-block read (per-command cost dominates) (2026-10-11) | `sd.md`, `perf.md` | Not modelled: the emulator's benchmarks are deterministic instruction counts (ADR-0007), not card latencies. Real-card numbers live in `perf.md` |
| XScale: enabling or disabling the MMU, the CPWAIT and the next instructions must share one 32-byte cache line (2026-10-11) | `README.md` (first facts) | Not modellable: QEMU has no cache-line pipeline effects. The HAL's aligned assembly is the only guard; keep the device run as the test |
| Windows Mobile leaves DMA channels and the UDC running at handoff; the 2700G needs ~0.5 s to settle (2026-10-11) | `README.md`, `2700g.md` | Not modelled: the emulator starts from reset, not from Windows Mobile's state. `_start` stops DMA/UDC on both |
| The X51v's D+ pull-up is a fixed resistor: the PC never sees the device disconnect (2026-10-11) | `udc.md`, ADR-0012 | Modelled in spirit: the emulated host re-enumerates when the loader re-enables the UDC; the Windows device-restart task exists only for the real cable |

## How to add a row

1. Record the fact in the block's spec (`docs/hw/<block>.md`) with its
   evidence, as always (`CLEANROOM.md`).
2. If a model can reproduce it: change the fork, add or extend an emulator
   boot in `xtask/src/emu.rs` that fails without the change, tag a release
   (`rp-vX.Y.0`), bump `QEMU_RELEASE` in `xtask/src/emu.rs`, re-record the
   bench baseline (`cargo xtask bench --update`: baselines are tied to the
   emulator version).
3. Add the row here either way.
