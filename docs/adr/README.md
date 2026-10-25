# Architecture decision records

Every decision record, generated from the records themselves by `cargo xtask adr-index` (CI's `adrs` step fails when this page is stale: edit the records, then regenerate). The status is the first word of each record's status line; the record has the details (who decided, when, which options). New records start from [0000-template.md](0000-template.md).

| ADR | Title | Status | Date | Amendments |
|---|---|---|---|---|
| [0001](0001-rust-first.md) | Rust first, on a pinned nightly with a custom ARMv5TE target | Accepted | 2026-10-10 |  |
| [0002](0002-qemu-fork.md) | Emulator = fork of QEMU 9.1 with an `x51v` machine | Accepted | 2026-10-10 |  |
| [0003](0003-ce5-slot-memory-model.md) | CE5-style 32 MB slot memory model with FCSE | Accepted | 2026-10-10 |  |
| [0004](0004-testing-policy.md) | Testing policy | Accepted | 2026-10-10 |  |
| [0005](0005-memory-and-storage-architecture.md) | Memory and storage architecture: designed for 64 MB | Accepted | 2026-10-10 |  |
| [0006](0006-physical-memory-beyond-the-textbook.md) | Physical memory: designed for this chip, not the textbook | Accepted | 2026-10-11 | 2 |
| [0007](0007-interrupt-ownership.md) | Interrupt ownership: no driver code runs in interrupt mode | Accepted | 2026-10-11 |  |
| [0008](0008-virtual-memory-layer.md) | The virtual memory layer | Accepted | 2026-10-11 |  |
| [0009](0009-pe-image-loading.md) | Loading PE images: an image view in the page cache, relocated on demand | Accepted | 2026-10-11 |  |
| [0010](0010-threads-scheduling-syscalls.md) | Threads, scheduling and the system-call boundary | Accepted | 2026-10-11 | 1 |
| [0011](0011-memory-reclaim.md) | Reclaiming memory without accessed bits | Accepted | 2026-10-11 |  |
| [0012](0012-usb-device-link.md) | The USB device link (CDC-ACM first) | Accepted | 2026-10-11 |  |
| [0013](0013-card-writes-over-usb.md) | Writing files on the SD card over USB | Accepted | 2026-10-11 |  |
| [0014](0014-usb-mass-storage.md) | The SD card as a drive on the PC (USB mass storage) | Accepted | 2026-10-12 |  |
| [0015](0015-thumb-for-cold-code.md) | Thumb for cold code | Accepted | 2026-10-12 |  |
| [0016](0016-processes-and-ipc.md) | Processes, the driver host, and IPC | Accepted | 2026-10-12 |  |
| [0017](0017-securing-the-development-loader.md) | Securing the development loader | Proposed | 2026-10-12 |  |
| [0018](0018-optional-packs.md) | Optional packs: a complete base system in flash, extras on the card | Accepted | 2026-10-12 |  |
| [0019](0019-kernel-object-memory.md) | Where the kernel keeps per-process records | Accepted | 2026-10-12 |  |
| [0020](0020-field-kit-pack.md) | Field Kit, a network and utility toolkit pack | Proposed | 2026-10-13 |  |
| [0021](0021-shell-library-and-first-today-screen.md) | The shell library and the first Today screen | Accepted | 2026-10-13 |  |
| [0022](0022-graphics-stack.md) | The graphics stack: compositor, GDI layer and the MBX renderer | Accepted | 2026-10-13 | 1 |
| [0023](0023-mbx2d-renderer.md) | The MBX 2D renderer | Accepted | 2026-10-14 |  |
| [0024](0024-mbx-3d-core.md) | Bringing up the MBX 3D core | Accepted | 2026-10-14 |  |
| [0025](0025-mved1-video-decoder.md) | Bringing up the MVED1 video decoder | Accepted | 2026-10-14 |  |
| [0026](0026-own-mbx-3d-renderer.md) | Our own MBX 3D renderer | Accepted | 2026-10-15 |  |
| [0027](0027-2700g-memory-map.md) | The 2700G memory map | Accepted | 2026-10-15 |  |
| [0028](0028-legacysync.md) | LegacySync, the Windows companion for Pocket PC software | Accepted | 2026-10-15 |  |
| [0029](0029-device-random-numbers.md) | The device's random number generator | Accepted | 2026-10-15 |  |
| [0030](0030-graphics-service.md) | The graphics service: one owner for the 2D engine and the 3D core | Accepted | 2026-10-15 |  |
| [0031](0031-clocks-and-power-states.md) | RetroPane owns the CPU clocks, voltage and power states | Accepted | 2026-10-15 |  |
| [0032](0032-audio-service.md) | The audio service: one mixer, one owner of the codec | Accepted | 2026-10-18 |  |
| [0033](0033-keyboards-mice-headsets.md) | Keyboards, mice and headsets over USB and Bluetooth | Accepted | 2026-10-18 |  |
| [0034](0034-compositor.md) | The compositor: layers, frame modes and the frame budget | Accepted | 2026-10-19 | 1 |
| [0035](0035-2700g-clocks.md) | The 2700G's clocks: past Dell's, through a self-test on every device | Accepted | 2026-10-22 |  |
| [0036](0036-card-file-systems-and-formatting.md) | Card file systems (exFAT, FAT32, FAT16, FAT12) and formatting on the device | Proposed | 2026-10-25 |  |
| [0037](0037-pc-file-access-mtp-and-drives.md) | Files to the PC: MTP for Windows (and Linux), drives kept for every other computer | Accepted (votes: all defaults) | 2026-10-25 |  |

37 records: 33 accepted, 4 proposed, 0 superseded, 0 rejected.
