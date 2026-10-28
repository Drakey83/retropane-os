//! Image size budgets (ADR-0005 rule 7). Budgets only ever get tighter.

use std::path::Path;

/// A size budget for one bare-metal image.
pub struct Budget {
    /// Image name under `target/<target>/release/`.
    pub image: &'static str,
    /// Max bytes loaded from storage (code + read-only data + initialised data).
    pub max_load: u64,
    /// Max bytes of RAM not backed by the image (zeroed data + stack).
    pub max_zero: u64,
    /// What the image does when the resident loader starts it: part of the
    /// budget so no image can be added without saying (`cargo xtask load`
    /// refuses the ones that never return).
    pub device: OnDevice,
}

/// What an image does when the resident loader starts it on the device.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OnDevice {
    /// Returns to the loader when done (or crashes back to it).
    Returns,
    /// The loader itself (it replaces the running one).
    Loader,
    /// Never returns (an emulator test, or one HaRET starts directly):
    /// under the loader it would run into the watchdog. Refused by
    /// `cargo xtask load` unless forced.
    NeverReturns,
}

/// How `image` behaves under the loader (`None`: not one of ours).
#[must_use]
pub fn on_device(image: &str) -> Option<OnDevice> {
    BUDGETS.iter().find(|b| b.image == image).map(|b| b.device)
}

/// Current budgets.
///
/// Rules (ADR-0005 rule 7). **OS components** (what ships on the device,
/// starting with `rp-kernel`: RAM taken from apps) only tighten, except by
/// a raise the project owner approves, with the reason written next to the
/// budget. **Test and bring-up images** (never shipped) may be raised
/// without asking, with the reason written here (owner's decision,
/// 2026-10-11). Every raise is *ratcheted*: set to the measured size plus
/// about 10%, not to a round number with room to spare, so growth can't
/// accumulate unreviewed, and logged in docs/perf.md.
pub const BUDGETS: &[Budget] = &[
    Budget {
        image: "rp-hello",
        // 24 KiB until 2026-10-11: cold crates built for size cut it to 18 KB.
        // Raised 20 -> 22 KiB on 2026-10-11 (approved by the owner): shared
        // HAL start-up code grew (DMA stop, fault diagnostics, aligned MMU
        // transitions, user-fault path); 20.0 KiB measured, +~10%.
        // Ratcheted 22 -> 24 KiB on 2026-10-12 (test image): keys as Windows
        // Mobile programs see them (virtual keys, D-pad repeat, press and
        // hold); 23.2 KiB measured, +~4%.
        // Ratcheted 24 -> 26 KiB on 2026-10-12 (test image): the crash box
        // reads the 2700G's registers to draw on the plane actually on
        // screen (Windows Mobile's, at offset 0); 24.2 KiB measured, +~8%.
        // Ratcheted 26 -> 29 KiB on 2026-10-15 (test image): the CPLD's
        // shadow adopted from the shared table (Windows Mobile's at hand-
        // over, checked; item 77); 26.9 KiB measured, +~8%.
        // Ratcheted 29 -> 38 KiB on 2026-10-21 (test image): quality
        // audit C38, every crash through rp-image (the card-log line, the
        // crash box, the fault's word and SPSR). 34.4 KiB measured, +~10%.
        max_load: 38 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::NeverReturns,
    },
    // Ratcheted 24 -> 27 KiB on 2026-10-12 (test image): the SD driver's
    // recovery for slow and failing reads (block-by-block fallback, CMD12 +
    // CMD13 recovery, slow-clock retry), found on the owner's card;
    // 24.5 KiB measured, +~10%.
    // Ratcheted 27 -> 30 KiB on 2026-10-12 (test image): SD DMA (the DMA
    // channel driver and the controller's DMA path) and a 64-block read
    // row and its on-target check; 28.1 KiB measured, +~7%.
    // Ratcheted 30 -> 34 KiB on 2026-10-12 (test image): SHA-256 and HMAC
    // checked on the target and timed (opt-level 3: the hash is hot code
    // for TLS later); 32.9 KiB measured, +~3%.
    // Ratcheted 34 -> 46 KiB on 2026-10-12 (test image): the pixel paths
    // (item 52, `rp-gfx`) benchmarked and checked on the target: fills, the
    // glass tint, alpha icons, photo rows, framebuffer flushes, and the
    // per-channel reference they're compared with; 39.6 KiB measured, +~16%
    // (left there for the iWMMXt paths to come). Ratcheted 46 -> 54 KiB on
    // 2026-10-12 (test image): the interface layer (item 60 follow-on,
    // `rp_gfx::layer`) benchmarked and checked: glass, a glyph over glass,
    // dithered and 8888 flushes and the per-pixel reference; 48.2 KiB
    // measured with the slow "over" path kept out of line, +~12%.
    Budget {
        image: "rp-bench",
        // Ratcheted 54 -> 75 KiB on 2026-10-21 (test image): quality
        // audit C38, every crash through rp-image (the card-log line, the
        // crash box, the fault's word and SPSR). 67.8 KiB measured, +~10%.
        max_load: 75 * 1024,
        // Zeroed RAM ratcheted to 32 KiB on 2026-10-22 (test image): LZ4's
        // table kept in static memory as the kernel keeps it (8 KiB, ADR-0001
        // "Why, and what was fixed"); 27.7 KiB measured, +~15%.
        // Ratcheted 32 -> 48 KiB on 2026-10-26 (test image): the stack
        // 16 -> 32 KiB (it overflowed into `.bss`, quality audit C90).
        max_zero: 48 * 1024,
        device: OnDevice::NeverReturns,
    },
    // The kernel: frame-allocator bookkeeping for 48 MiB (60 KiB), page-table
    // bookkeeping (9 KiB), stacks including a 4 KiB abort stack.
    // Load budget raised 32 -> 38 KiB on 2026-10-11 (approved by the owner):
    // compressed RAM landed (LZ4 codec, compressed page store, fault-path
    // restore: ~8 KiB); the image crate is already built for size and its
    // printing trimmed. Ratcheted to the measured 34.9 KB + ~10%.
    Budget {
        image: "rp-kernel",
        // Raised 38 -> 46 KiB on 2026-10-12 (approved by the owner): IPC
        // (ADR-0016) landed: the system-call path, migrating call and
        // return, unwinding a faulted server (~6 KB), plus the IPC test
        // (~1.3 KB; its user programs are copied into a frame at start, not
        // a page-aligned page in the image). Measured 41.3 KiB + ~10%. The
        // handle tables are 4 KiB of zeroed RAM, sized to CE5's 31 process
        // slots, inside the existing zero budget.
        // Raised 46 -> 55 KiB on 2026-10-12 (approved by the owner): the
        // scheduler (ADR-0010) joined the kernel image: threads, events,
        // preemption and the idle loop, with two client threads in a
        // server at once. Measured 50.1 KiB + ~10%. Building the start-up
        // code as Thumb was tried and grew the image (helpers it inlined
        // were outlined), so it stays ARM.
        // Raised 55 -> 65 KiB on 2026-10-12 under the owner's standing rule
        // (a planned kernel subsystem lands: measured + ~10%, logged; ask
        // if one step adds over 8 KB): PE images loaded into processes
        // (ADR-0009): parse, demand page loads with relocation, W^X, the
        // page cache's fill (~5 KB), plus the test's image writer and
        // checks (~2.5 KB). `rp-kernel-pe` built for size (cold: once per
        // page per image). Measured 58.9 KiB + ~10%.
        // Raised 65 -> 76 KiB on 2026-10-12 under the owner's standing rule:
        // supervised driver hosts (ADR-0016 step 3): restart on fault with
        // in-flight calls failed, SYS_GRANT, caller identity, plus two more
        // test processes (a host and the service manager; their address
        // spaces are 1.6 KB each of initialised data). Measured 69.1 KiB +
        // ~10%. Follow-up: address spaces allocated per process at start,
        // not static (queued).
        // Raised 76 -> 84 KiB on 2026-10-12 under the standing rule: processes
        // started and ended at run time (SYS_SPAWN/SYS_EXIT, exit cleanup, a
        // reusable process table: 5 records of 1.6 KB, initialised data).
        // Measured 76.6 KiB + ~10%. Next: region storage from a shared pool
        // instead of 64 per record (ADR queued, item 47).
        // Lowered 84 -> 82 KiB on 2026-10-12: ADR-0019 landed (item 48):
        // regions in per-process pages, so each record shrank from 1.6 KB
        // to ~60 B (-6.0 KB), less the 300-region check (+1.8 KB).
        // Measured 76.7 KiB; the room goes back.
        // Raised 82 -> 95 KiB on 2026-10-13 (approved by the owner, +10%
        // rule): a real driver in a host (item 46): the display host's
        // image embedded (2 KB), device and interrupt grants, restarts of a
        // host's own threads, pages loaded straight from the embedded file
        // (~5.3 KB), and the test's checks (~2.3 KB). 88,260 B measured.
        // Raised 95 -> 112 KiB on 2026-10-21 under the standing rule
        // (a planned step adds under 8 KB: +6.6 KB): quality audit C38, every
        // crash through rp-image: the crash's line on the card (the card
        // log's writer, ~4 KB), the crash path and the fault's word and SPSR;
        // the kernel never takes the display, so no crash box. Measured
        // 100.9 KiB + ~10%.
        max_load: 112 * 1024,
        // Raised 100 -> 112 KiB with the load budget (approved by the owner):
        // the 12 KiB compression buffers live here, not on a kernel stack
        // (on the 4 KiB abort stack they overflowed into the frame
        // allocator).
        // Raised 112 -> 120 KiB on 2026-10-12 (approved by the owner): the
        // scheduler's thread and event tables (4.3 KiB). Already cut to
        // fit: thread contexts on each thread's own stack (512 B for 128
        // threads, not 5 KiB), the port table without `Option`, and callers'
        // slots kept in their kernel stack frames, not an IPC table.
        // Measured 112.9 KiB + ~6%.
        max_zero: 120 * 1024,
        device: OnDevice::NeverReturns,
    },
    // The display+input driver host (ADR-0016, item 46): a user process
    // embedded in the kernel image, so it counts against that budget too.
    // First budget = measured + ~10% (1,140 B of code, no data; its stack
    // is the kernel's mapping, not the image's).
    Budget {
        image: "rp-display-host",
        max_load: 1280,
        max_zero: 256,
        device: OnDevice::NeverReturns,
    },
    // Threads test harness (ADR-0010): scheduler tables (~4 KiB, .bss) and
    // the harness itself; one image per subsystem's test keeps each budget
    // honest instead of piling every test into one image.
    Budget {
        image: "rp-threads",
        // First budget = measured + ~10% (13.2 KiB load; 26.4 KiB zeroed:
        // boot stacks, scheduler tables, 128 thread contexts).
        // Raised 15 -> 17 KiB on 2026-10-11 (approved by the owner): the
        // user-mode preemption test (16.0 KiB measured, ratcheted +~9%).
        // Raised 17 -> 27 KiB on 2026-10-11 (approved by the owner): ADR-0010's
        // mutexes with priority inheritance (~2 KiB of scheduler code), the
        // user-fault path, lazy iWMMXt switching (~1.3 KiB of HAL code, in
        // this image only) and interrupts as waitable objects, plus their
        // live tests (23.9 KiB measured, +~10%).
        // Ratcheted 27 -> 32 KiB on 2026-10-12 (test image, owner's rule):
        // results teed into the run log and a return to the loader, so the
        // real X51v's switch and interrupt numbers reach the PC, plus the
        // shared crash path; 28.1 KiB measured, +~10%.
        // Ratcheted 32 -> 40 KiB on 2026-10-20 (test image): the card log for
        // every run the loader starts (rp_hal_x51v::cardsink, quality audit
        // C24: each line onto the SD or CF card the loader set up, ~4.5 KiB
        // of sector writes and the sync); 33.5 KiB measured, + ~10%.
        max_load: 40 * 1024,
        // Lowered 30 -> 27 KiB on 2026-10-12: thread contexts moved onto the
        // threads' own stacks (24.3 KiB measured, +~10%).
        // Ratcheted 27 -> 31 KiB on 2026-10-15 (test image rule): 255
        // threads instead of 128 (the owner's vote), the scheduler's and
        // HAL's per-thread tables; 27.9 KiB measured, +~10%.
        max_zero: 31 * 1024,
        device: OnDevice::Returns,
    },
    // Quantum test image (ADR-0010 amendment): touch latency, frame timing
    // and switch cost under each time-slice policy. First budget =
    // measured + ~10% (30.2 KiB load; 24.4 KiB zeroed: boot stacks,
    // scheduler tables, 128 thread contexts).
    Budget {
        image: "rp-quantum",
        // Ratcheted 34 -> 39 KiB on 2026-10-21 (test image): quality
        // audit C38, every crash through rp-image (the card-log line, the
        // crash box, the fault's word and SPSR). 34.7 KiB measured, +~10%.
        max_load: 39 * 1024,
        // Ratcheted 27 -> 31 KiB on 2026-10-15 (test image rule): 255
        // threads' tables; 27.3 KiB measured, +~10%.
        max_zero: 31 * 1024,
        device: OnDevice::Returns,
    },
    // Diagnostics only, never part of the OS: register names and report text
    // dominate. Built for size (opt-level "s").
    // Raised 40 -> 45 KiB on 2026-10-11 (approved by the owner): the run log
    // and return-to-loader path (ADR-0012), 40.9 KiB measured, +~10%.
    // Ratcheted 45 -> 53 KiB on 2026-10-11 (test image, owner's rule): the
    // card's FAT32 root listing (ADR-0013), 47.5 KiB measured, +~10%.
    // Ratcheted 53 -> 60 KiB on 2026-10-12 (test image): the PC Card
    // registers and CF line, and the exFAT reader for SDXC cards (the
    // probe's emulated card is exFAT); 54.6 KiB measured, +~10%.
    // Ratcheted 60 -> 70 KiB on 2026-10-12 (diagnostics image): the pixel
    // paths' benchmarks and on-target check (item 52), so the real X51v
    // measures them too; `Dirty::add` rewritten as a loop on the way
    // (-1.8 KB); 64.1 KiB measured, +~9%.
    // Ratcheted 70 -> 77 KiB on 2026-10-15 (test image): DMA transfers of
    // any length streamed through each channel's ring (storage speed-ups
    // item 1) on top of the DMA channel map; 70.0 KiB measured, +~10%.
    Budget {
        image: "rp-probe",
        // Ratcheted 77 -> 85 KiB on 2026-10-21 (test image): quality
        // audit C38, every crash through rp-image (the card-log line, the
        // crash box, the fault's word and SPSR). 77.1 KiB measured, +~10%.
        // Ratcheted 85 -> 96 KiB on 2026-10-22 (test image): the
        // performance gate's CPU benchmarks (cache walk, copy and prefetch
        // sweeps, SRAM patterns, DMA as a copy engine); 85.3 KiB measured.
        // Ratcheted 96 -> 100 KiB on 2026-10-22 (test image): the card log
        // no longer waits on a busy card (quality audit C59: sends, polls,
        // catches up at check-ins); 96.4 KiB measured.
        // Ratcheted 100 -> 112 KiB on 2026-10-22 (test image): the multiply
        // by multiplier size rows and LZ4's event counts (ADR-0001 "Why,
        // and what was fixed"); 102.3 KiB measured, +~10%.
        max_load: 112 * 1024,
        // Zeroed RAM ratcheted 20 -> 40 KiB on 2026-10-22 (test image): a
        // 32 KiB stack (the device's runs used 16,076-16,268 of 16 KiB and
        // one crashed); 35.7 KiB measured, +~12%.
        // Ratcheted 40 -> 48 KiB on 2026-10-22 (test image): LZ4's table kept
        // in static memory as the kernel keeps it (8 KiB); 43.7 KiB measured.
        max_zero: 48 * 1024,
        device: OnDevice::Returns,
    },
    // Bring-up: the USB link test (ADR-0012 U1): 17.9 KiB measured, +~10%.
    // Ratcheted 20 -> 22 KiB on 2026-10-12 (test image): the crash box
    // reads the 2700G's registers to draw on the plane actually on screen
    // (Windows Mobile's, at offset 0); 20.5 KiB measured, +~7%.
    // Ratcheted 22 -> 25 KiB on 2026-10-16 (test image): the UDC driver's
    // double-buffered endpoints, settled OUT reads and DMA hand-off (item
    // 74); 22.2 KiB measured, +~10%.
    Budget {
        image: "rp-usb",
        // Ratcheted 25 -> 34 KiB on 2026-10-21 (test image): quality
        // audit C38, every crash through rp-image (the card-log line, the
        // crash box, the fault's word and SPSR). 30.2 KiB measured, +~10%.
        max_load: 34 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::NeverReturns,
    },
    // Bring-up: the resident loader (ADR-0012 U3): 19.6 KiB measured, +~10%.
    // It must also stay inside its 1 MiB home (boot/loader/link.x).
    // Ratcheted 22 -> 41 KiB on 2026-10-11 (test image, owner's rule): SD
    // card writes and the FAT32 copy-then-swap (ADR-0013), ~37 KiB measured.
    // Ratcheted 41 -> 52 KiB on 2026-10-12 (test image): the SD card as a
    // USB drive (ADR-0014: composite descriptors, the Bulk-Only/SCSI stack,
    // 2.6 KiB of it in rp-usb-msc), 46.6 KiB measured, +~10%.
    // Ratcheted 52 -> 58 KiB on 2026-10-12 (test image): card diagnostics
    // over USB (`drive status`, `card scan`), which found the owner's slow
    // card block, and the USB layout fallback; 52.2 KiB measured, +~10%.
    // Ratcheted 58 -> 70 KiB on 2026-10-12 (test image): creating files on
    // the card (rp-fat: long names, ~N short names, root growth; ~5 KiB) and
    // the SD driver's slow-read recovery; 62.0 KiB measured, +~10%. When
    // rp-fat moves into the OS it counts against rp-kernel's strict budget.
    // Ratcheted 70 -> 78 KiB on 2026-10-12 (test image): card repair tools
    // (scrub, repair, peek), SD hot swap and the lock switch; 70.3 KiB
    // measured, +~10%.
    // Ratcheted 78 -> 87 KiB on 2026-10-12 (test image): card folders
    // (paths, mkdir, put creating parents) and the backlight command;
    // 79.0 KiB measured, +~10%.
    // Ratcheted 87 -> 96 KiB on 2026-10-12 (test image): exFAT reading and
    // writing for SDXC cards (put and crash reports on either file system,
    // ~16 KiB at opt-level 3); the loader is now built for size like the
    // other cold images, which took 11 KiB back; 89.8 KiB measured, +~7%.
    // Ratcheted 96 -> 104 KiB on 2026-10-12 (test image; never in a
    // release, docs/security.md): CompactFlash (driver, `cf`, the drive's
    // second unit) and exFAT folders; 98.2 KiB measured, +~6%.
    // Ratcheted 104 -> 112 KiB on 2026-10-12 (test image): audio (I2C,
    // the WM8750, I2S streamed from a DMA ring, the headphone jack) for
    // the `tone` command; 104.6 KiB measured, +~7%.
    // Ratcheted 112 -> 124 KiB on 2026-10-12 (test image): browsing the
    // card from the PC (`ls`, `get`) and the read-only file system check
    // (`card check`, FAT32 and exFAT); 118.0 KiB measured, +~5%.
    // Ratcheted 124 -> 128 KiB on 2026-10-12 (test image): SHA-256
    // fingerprints, idling, `rm`; 124.7 KiB measured. ADR-0015 (Thumb for
    // cold code, proposed) would give ~20 KiB of the loader back.
    // Lowered 128 -> 101 KiB on 2026-10-12: built as Thumb (ADR-0015,
    // accepted), 124.7 -> 92.0 KiB; measured + ~10%.
    // Raised 101 -> 112 KiB on 2026-10-13 (approved by the owner, +10%
    // rule): the drive's overlapped reads and read-ahead (item 58: the
    // card's DMA waits send the previous chunk to the PC); 104,476 B
    // measured. Room for DMA writes and ACMD23 next.
    // Raised 112 -> 116 KiB on 2026-10-16 (approved by the owner): the
    // CPLD's shared shadow table at hand-over (items 72/77: ~750 B, the
    // driver every program uses) and its diagnostics (`handoff`, the
    // shadow in `cpld`: ~830 B); 116,276 B measured after a 364 B trim.
    // The loader never ships (docs/security.md). A loader diet (diagnostics
    // whose questions are answered, e.g. the raw `cpld` dump) is queued for
    // the next time it needs room, before any further raise.
    // Lowered 116 -> 112 KiB the same day by the loader diet: the audio
    // check (`tone`, the WM8750, I2C, I2S streaming, the headphone jack;
    // 3.5 KiB) moved to its own test image, rp-tone; 112,680 B measured.
    // Raised 112 -> 124 KiB on 2026-10-20 (approved by the owner): the card
    // log for every run (quality audit C24): the loader reserves each run's
    // rplog/<image>-NNNN.txt on the SD and CF cards (rp-fat's reserve, the
    // cleanup of old ones), leaves the state for the program, closes the log
    // when the run comes back (or at the next start), and `ls`/`get` read the
    // CF card (`cf:`) for the half a program kept there; one copy of the file
    // systems for both cards. 121,224 B measured (Thumb).
    // Raised 124 -> 128 KiB on 2026-10-25 (approved by the owner): the CF
    // card's write timing (the owner's ruling A, R28: rule M for writes,
    // measured once per card on its own test sectors and kept in
    // rplog/cfwrite.txt), after the CF drive's guard and put (ruling A,
    // ADR-0014); 128,222 B measured (Thumb).
    // Ratcheted 128 -> 156 KiB on 2026-10-25 (test image, ADR-0013's rule;
    // ADR-0036): FAT12 and FAT16, the boot-file guard on every format
    // (exFAT's entry sets, bitmap and boot region), the formatter
    // (`rp_fat::mkfs`: FAT12/16/32 and exFAT, the up-case table's ranges)
    // and the `format` command; 145,926 B measured (Thumb), + ~9%.
    // 156 -> 244 KiB on 2026-10-25 (approved by the owner 2026-10-26, R55)
    // (ADR-0037, R53): the cards over MTP (`rp-usb-mtp`'s responder, its
    // card layer on rp-fat's node layer, the loader's glue and the keypad
    // for a format's yes); one disk type for the drive and MTP keeps rp-fat
    // built once (9 KB saved); 228,650 B measured (Thumb), + ~9%.
    Budget {
        image: "rp-loader",
        max_load: 244 * 1024,
        // Zeroed RAM 20 -> 36 KiB on 2026-10-25 (approved by the owner):
        // the loader's stack 16 -> 32 KiB, after the CF card's timing
        // searches at the attach overflowed 16 (17,376 B deepest; quality
        // audit C83); 36,656 B measured, the stack in its own 1 MiB home.
        max_zero: 36 * 1024,
        device: OnDevice::Loader,
    },
    // Test image: the crash path (panics on purpose); 6.6 KiB measured, +~10%.
    // Ratcheted 8 -> 14 KiB on 2026-10-12 (test image): it now draws the
    // crash box every image draws (the console and font), on a plane it
    // moves to offset 0 as Windows Mobile leaves it; 12.9 KiB measured, +~9%.
    // Ratcheted 14 -> 20 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 17.5 KiB measured, + ~10%.
    // Ratcheted 20 -> 23 KiB on 2026-10-21 (test image): its data-abort
    // variant (RP_CRASH_DATA) is 20,668 B with C38's crash path, caught by
    // the variant size check (quality audit C45); + ~10%.
    // Ratcheted 23 -> 26 KiB on 2026-10-22 (test image): built with frame
    // pointers and the backtrace walker (ADR-0001), its run-limit variant
    // (RP_CRASH_RUN_LIMIT) is 23,768 B; + ~10%.
    Budget {
        image: "rp-crash",
        max_load: 26 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Test image: the soft run limit (runs forever on purpose); 7.8 KiB
    // measured, +~10%; then 9 -> 10 KiB (2026-10-14): every image now
    // reports its deepest stack on the way back to the loader (formatting
    // linked in), 9.1 KiB measured.
    // Ratcheted 10 -> 20 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 17.4 KiB measured, + ~10%.
    Budget {
        image: "rp-hang",
        max_load: 20 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::NeverReturns,
    },
    // Test image: shows a UI concept picture on the real panel for 20 s,
    // then returns. Almost all of it is the 600 KiB picture (480 x 640
    // RGB565); 609.6 KiB measured, the picture fixed in size.
    Budget {
        image: "rp-picture",
        max_load: 640 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Test image: plane blending at scan-out (overnight item 60); the
    // scene is generated, not stored. 19.2 KiB measured, + ~10%.
    // Ratcheted 22 -> 34 KiB on 2026-10-12 (test image): stage F draws the
    // scene through `rp-gfx` (layer, glyph masks, dirty flushes: ~4 KiB of
    // the library, ~4 KiB of scene code) and times each stage's drawing;
    // 29.7 KiB measured, + ~14%.
    // Test image (item 68): steps the 2700G's chip-select timing and
    // verifies 2700G memory at each step. Budget set from the first build;
    // 2026-10-19 with a 2D fill check per step and the soak (the 2D
    // engine's driver): 18.2 KiB measured, + ~13%.
    // Ratcheted 21 -> 29 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 26.3 KiB measured, + ~10%.
    // Test image (reminder R8): the CPLD's bus timing, reads stepped and a
    // watched backlight blink. 30.1 KiB measured (2026-10-24), + ~13%.
    Budget {
        image: "rp-cpldbus",
        max_load: 34 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    Budget {
        image: "rp-cs3bus",
        // Ratcheted 29 -> 32 KiB on 2026-10-22 (test image): rp-image's
        // finish prints the deepest stack on serial when no loader is
        // there (quality audit C50); 29.1 KiB measured, + ~10%. Ratcheted
        // 32 -> 40 KiB on 2026-10-27 (test image): run 2's grid of every CS3
        // timing combination (reminder R3) and its screen label; 37.9 KiB
        // measured, + ~5%.
        max_load: 40 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Ratcheted 34 -> 40 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 39.5 KiB measured, + ~10%.
    Budget {
        image: "rp-blend",
        // Ratcheted 40 -> 47 KiB on 2026-10-21 (test image): quality
        // audit C38, every crash through rp-image (the card-log line, the
        // crash box, the fault's word and SPSR). 42.6 KiB measured, +~10%.
        // Ratcheted 47 -> 52 KiB on 2026-10-24 (test image): the scale
        // variant loads the scaler's filter (quality audit C73); 47.7 KiB
        // measured, +~10%.
        // Ratcheted 52 -> 62 KiB on 2026-10-25 (test image): run P9d, the
        // scaler filter computed for any scale (rp_drv_mbx_display::filter,
        // quality audit C73), its stripe pattern and steps; 56.3 KiB
        // measured, +~10%.
        max_load: 62 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Test image (ADR-0021): the shell's Today screen. Carries the 450 KiB
    // 4:2:0 wallpaper and ~33 KiB of glyphs (the OS will load both from
    // flash or the card instead). 538.5 KiB measured (450 KiB of it the
    // wallpaper), + ~10%. Zeroed RAM 20 -> 30 KiB (2026-10-14): its stack
    // is 24 KiB, not 16 (the shell's frame and the GPU renderer's tables
    // on it; 12.6 of 16 KiB used on the device), 26.1 KiB measured.
    // 592 -> 610 KiB (2026-10-15, test image): the Today screen on the
    // graphics service (ADR-0030: rp-graphics with the 3D renderer it owns,
    // ~10.7 KiB of code); 596.0 KiB measured, + ~10% of the code (the
    // wallpaper isn't code).
    // 610 -> 780 KiB (2026-10-17, test image): the six weather pictures of
    // the Today screen are baked in until the card can supply them (one
    // 175 x 120 and five 58 x 40, straight ARGB8888, 139 KiB); 599.4 KiB
    // measured without them, 727.0 KiB with, + ~7%.
    // 780 -> 900 KiB (2026-10-17, owner: the big icon looked small): the icons
    // are cropped to what is drawn and scaled to the mock-ups' boxes (the big
    // one 250 x 174, 170 KiB, the small ones 66 px wide); 833.8 KiB measured
    // in all, + ~8%. Zeroed RAM 30 -> 46 KiB (the same day): its stack 24 -> 40
    // KiB, 23.5 KiB of it used (96%) and a deeper path froze the device.
    // Zeroed RAM 46 -> 36 KiB (the same day, stack audit): 22 KiB of that
    // depth was `main`'s frame holding the long-lived objects; they are in
    // `rt::Slot`s in `.bss` now (19 KiB, counted here) and the stack is 12
    // KiB, 6.0 of it used (50%): 32.8 KiB in all, + ~10%. Then 36 -> 31 KiB:
    // the graphics service's 3D state (~5 KiB, unused here) is lent only by
    // programs that use 3D: 27.9 KiB, + ~11%. Then 31 -> 40 KiB: the owner's
    // 16-bit wallpaper comparison keeps its dither rows in `.bss` (7.5 KiB,
    // off the 12 KiB stack): 35.4 KiB, + ~13%.
    Budget {
        image: "rp-today",
        max_load: 900 * 1024,
        max_zero: 40 * 1024,
        device: OnDevice::Returns,
    },
    // Test image (item 69): the first GPU-drawn rectangle (MBX 2D engine),
    // with the display driver to adopt the screen and the CPU fill to time
    // against. 16.7 KiB measured (2026-10-13), + ~10%; then 19 -> 27 KiB
    // for the copy checks (step 2: off-screen, overlapping, colour-keyed,
    // timed), 23.7 KiB measured, + ~10%; then 27 -> 38 KiB for the DMA
    // timing and the past-16-MiB probe (ADR-0022 open questions 1-2), 34.0
    // KiB measured, + ~10%; then 38 -> 44 KiB for every way to copy down
    // over itself, checked and timed (2026-10-14), 39.1 KiB measured, + ~10%;
    // then 44 -> 50 KiB for the completion and interrupt-line stages (ADR-0022
    // question 5), 44.8 KiB measured, + ~10%.
    // Ratcheted 50 -> 59 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 57.3 KiB measured, + ~10%.
    Budget {
        image: "rp-mbx",
        // Ratcheted 59 -> 68 KiB on 2026-10-21 (test image): quality
        // audit C38, every crash through rp-image (the card-log line, the
        // crash box, the fault's word and SPSR). 61.7 KiB measured, +~10%.
        max_load: 68 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Test image (renderer step 1, ADR-0022): the blend-state and display-
    // event probe (2026-10-14): 37.4 KiB measured, + ~10%; then 42 -> 51
    // KiB for B6 (ADR-0023's unknowns: stride 0, 32 bpp blending and
    // scan-out, an ARGB4444 destination), 46.1 KiB measured, + ~10%.
    // Ratcheted 51 -> 57 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 55.0 KiB measured, + ~10%.
    Budget {
        image: "rp-mbx-blend",
        // Ratcheted 57 -> 65 KiB on 2026-10-21 (test image): quality
        // audit C38, every crash through rp-image (the card-log line, the
        // crash box, the fault's word and SPSR). 58.6 KiB measured, +~10%.
        max_load: 65 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Test image (ADR-0024 M4): the MBX 3D core's first triangle. 19.9 KiB
    // measured (2026-10-14), + ~10%; then 22 -> 25 KiB for version 2 (the
    // parameter memory set-up, the clock vote, two table tries), 22.6 KiB
    // measured, + ~10%; then 25 -> 29 KiB for version 7 (the textured
    // square's format sweep), 26.2 KiB measured, + ~10%; then 29 -> 33 KiB
    // for version 8 (8 forms of the square first), 30.1 KiB, + ~10%; then
    // 33 -> 36 KiB for version 22 (recovery after a timed-out render, the
    // alpha-source and filtering sweeps), 33.1 KiB measured; then 36 -> 42
    // KiB for version 24 (T10-T13: the interrupt line, depth across
    // renders, filtering and mipmaps, 32-bit and PVRTC textures), 39.3 KiB
    // measured, + ~7%; tightened 42 -> 35 KiB for version 26 (the clock
    // and 16-bit target stages; texture helpers gone), 31.2 KiB measured;
    // raised to 44 KiB for version 29 (option A's clock and CS3 bus-timing
    // steps: per-step flush, read-back, all-free-memory patterns, the
    // report lines), 41.0 KiB measured.
    // Test image (ADR-0026, 2026-10-18): the alpha test, depth "less"
    // without it and 8-bit grey textures; rp-mbx3d's proven set-up only.
    // 20.3 KiB measured, + ~15%.
    // Ratcheted 24 -> 32 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 28.7 KiB measured, + ~10%.
    // Ratcheted 32 -> 40 KiB on 2026-10-22 (test image): test M, the second
    // texture unit through the tile accelerator (the gate's P5); 35.1 KiB
    // measured, + ~10%.
    // Ratcheted 53 -> 61 KiB on 2026-10-23 (test image): test G3, Dell's
    // colour clear and the backgrounds at three divide-1 clocks; the G3
    // variant 55.4 KiB, + ~10%.
    // Ratcheted 46 -> 53 KiB on 2026-10-23 (test image): test G2, the TA's
    // choices one at a time through the direct path (per-tile lists, the
    // four-corner and clear objects); the G2 variant 48.1 KiB, + ~10%.
    // Ratcheted 40 -> 46 KiB on 2026-10-23 (test image): the on-screen
    // label (every device program shows its step) and test G, Gouraud at
    // the cores' divide 1 through the TA (the G variant, the larger build,
    // 41.6 KiB measured, + ~10%).
    // Ratcheted 74 -> 112 KiB on 2026-10-23 (test image): test G7 with the
    // emulator's frame embedded (our own scene, 30 KiB of region array and
    // objects); the variant 98.4 KiB, + ~10%. (The device's G7 variant
    // carries Dell's 177 KiB frame: a measurement variant, not held.)
    // Ratcheted 61 -> 74 KiB on 2026-10-23 (test image): test G5, Dell's
    // chip state copied under the loader, then halved (the state compared
    // word by word, the inherited way); the G5 variant with the emulator's
    // culprit 67.8 KiB, + ~10%.
    // Ratcheted 112 -> 128 KiB on 2026-10-24 (test image): test G15, the
    // CPU's operating point and core voltage per way (rp-hal-pxa27x's
    // clocks, the GPIO driver, the voltage codes) beside G14; the G15
    // variant with the emulator's frame 113.2 KiB, + ~10%.
    // Ratcheted 128 -> 140 KiB on 2026-10-27 (test image): the G17-G20
    // variants with the emulator's frame grew past 128 (128.5 KiB at most,
    // R58's run of every G test) as rp-mbx3d's TA set-up and quad streams
    // moved in (audit C81, P4); + ~9%.
    Budget {
        image: "rp-glesfeat",
        max_load: 140 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Ratcheted 44 -> 51 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 46.0 KiB measured, + ~10%.
    Budget {
        image: "rp-mbx3d",
        // Ratcheted 51 -> 58 KiB on 2026-10-21 (test image): quality
        // audit C38, every crash through rp-image (the card-log line, the
        // crash box, the fault's word and SPSR). 52.1 KiB measured, +~10%.
        max_load: 58 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Test image (ADR-0029): the RNG's noise sources measured, samples to
    // the card. 94.3 KiB measured (2026-10-15), + ~10%; most of it is
    // rp-fat's writes for both FAT32 and exFAT cards, as in the loader;
    // then 104 -> 117 KiB for interrupt-timestamped ticks (the IRQ
    // dispatch code), 106.5 KiB measured, + ~10%.
    // Ratcheted 117 -> 130 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 117.3 KiB measured, + ~10%.
    Budget {
        image: "rp-noise",
        max_load: 130 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Test image (ADR-0026): RetroPane's own 3D renderer through its API,
    // reproducing rp-mbx3d versions 19-23 (stages S1-S7). 50.8 KiB measured
    // (2026-10-15), + ~10%; then 56 -> 70 KiB for run 3 (S8, the "less"
    // exploration, and exact render times), 64.8 KiB measured, + ~8%; then
    // tightened 70 -> 66 KiB for run 4 (S8 done and removed), 60.0 KiB
    // measured, + ~10%; then 66 -> 72 KiB for S5's "less" and alpha-test
    // cases (ADR-0026 option a, 2026-10-18), 66.0 KiB measured, + ~9%.
    // Ratcheted 72 -> 83 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 74.6 KiB measured, + ~10%.
    Budget {
        image: "rp-mbx3d-api",
        max_load: 83 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Test image (the performance gate's P4, docs/design/p4-ta-speed.md):
    // the tile accelerator against the CPU binner, six content sets, six
    // ways; its quads made as they are wanted and its buffers in the SDRAM
    // pool, so the zeroed RAM is the runtime's alone. 55.5 KiB measured
    // (2026-10-25), + ~10%.
    Budget {
        image: "rp-tabench",
        max_load: 61 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Test image (docs/design/ui.md, the 3D compositor proposal's GPU
    // facts): run A (fill rate, layers, partial frames, rectangular
    // textures, uploads, frame-event watch tests) and run B (texture-word
    // probes). 95.9 KiB measured (2026-10-19), + ~10%; zeroed RAM 19.7 KiB
    // (the renderer's state), + ~12%. Ratcheted 2026-10-20 (test image,
    // the owner's rule): the run log on the card as the run goes
    // (rp-cardlog: the SD driver and FAT32 + exFAT, ~52 KiB of it the file
    // systems, to reserve the file; quality audit C24 has the smaller
    // design, the loader reserving it): 178.0 KiB measured, + ~10%; zeroed
    // RAM 23.7 KiB (+ the 4 KiB reservation scratch), + ~10%.
    // Lowered 196 -> 117 KiB on 2026-10-20: the card log is the loader's
    // now (every run keeps one, quality audit C24), so the image no longer
    // carries the SD driver and the file systems; 109.5 KiB measured,
    // + ~10%; zeroed RAM 26 -> 22 KiB (no reservation scratch).
    // Zeroed RAM ratcheted 22 -> 24 KiB on 2026-10-21 (test image): run
    // FF's 2D renderer frame (`FF_FRAME`), 22,840 B measured.
    // Ratcheted 117 -> 140 KiB on 2026-10-23 (test image): run GC3 carries
    // run FT2 and GC2's machinery in one image, 128,680 B measured.
    // Zeroed RAM 24 -> 32 KiB the same day: the stack 16 -> 24 KiB (runs
    // GC2H, GC2H2 and GC3 near 80% of 16), 28,408 B measured.
    // Ratcheted 140 -> 170 KiB on 2026-10-24 (test image): runs GC2O-GC2Q
    // (the tile orders, the one-pass check, the write-out search and the
    // climb above 133.25 / 1); GC2Q's variant 152,952 B measured, + ~10%.
    // Zeroed RAM ratcheted 32 -> 40 KiB on 2026-10-25 (test image): the
    // stack 24 -> 32 KiB (the GC2 soaks' shared code at 80% of 24, GC2Y
    // 19,928 B in the emulator), 34,912 B.
    // Ratcheted 170 -> 192 KiB on 2026-10-25 (test image): runs GC2AE and
    // GC2AF (R69: the source search, the display's fetch and the CPU's
    // status reads switched per run); GC2Q's write-out variant 175,248 B
    // measured, + ~10%.
    // Zeroed RAM ratcheted 40 -> 48 KiB the same day: the stack 32 -> 40
    // KiB (run GC2AG's second renderer, 29,256 B deep in the emulator).
    // Zeroed RAM ratcheted 48 -> 56 KiB on 2026-10-27: the stack 40 -> 48
    // KiB (run GC2BG runs the tile-order soak's plans inside its warm-up's
    // frame with the on-die renderer beside the first, 33,744 B deep in
    // the emulator); 52,968 B measured.
    // Loaded ratcheted 192 -> 196 KiB on 2026-10-27 (test image): run GC2BS
    // (the port's cures, the scene-3 forms, the paired proof) put the
    // biggest variant (GC2Q's write-out one) at 197,360 B.
    // Loaded ratcheted 196 -> 200 KiB on 2026-10-27 (test image): runs
    // GC2BT-GC2BV (the region array on the die, the diagonal's forms, the
    // seals) put the biggest variant at 201,716 B.
    // Zeroed RAM ratcheted 56 -> 64 KiB on 2026-10-27: the stack 48 -> 56
    // KiB (run GC2BT, 40,456 B deep in the emulator); 59,496 B expected.
    Budget {
        image: "rp-gpubench",
        max_load: 200 * 1024,
        max_zero: 64 * 1024,
        device: OnDevice::Returns,
    },
    // Test image (docs/research/storage-speed.md): the CF socket's timings,
    // the card's CIS and IDENTIFY, five read paths timed; report only.
    // 32.8 KiB measured (2026-10-15), + ~10%.
    // Ratcheted 37 -> 45 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 44.6 KiB measured, + ~10%.
    // Ratcheted 45 -> 50 KiB on 2026-10-24 (test image): the card log's
    // slots (reminder R22: `cardsink::taken` marks the CF card the image
    // takes); 46,124 B measured, + ~10%.
    Budget {
        image: "rp-cftime",
        max_load: 50 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Test image (docs/research/storage-speed.md): the SD card's registers
    // and speeds by size, DMA, pre-erase, clock code and FAT layer. 74.7
    // KiB measured (2026-10-15, most of it the FAT layer, which also makes
    // the test file), + ~10%.
    // Ratcheted 82 -> 94 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 84.9 KiB measured, + ~10%.
    Budget {
        image: "rp-sdtime",
        max_load: 94 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Device test (docs/research/storage-speed.md items 1-4): CF read paths
    // and attach cycles, SD reads by size, an SD DMA write soak across a
    // card swap and the lock switch. Provisional until measured.
    Budget {
        image: "rp-storfast",
        // Ratcheted 96 -> 108 KiB on 2026-10-21 (test image): quality
        // audit C38, every crash through rp-image (the card-log line, the
        // crash box, the fault's word and SPSR). 98.1 KiB measured, +~10%.
        // Ratcheted 108 -> 120 KiB on 2026-10-24 (test image): C4-C5, the
        // CF card's own timing and its margin (reminder R7); 111.8 KiB
        // measured once the size check read the real build (arm_release).
        // Ratcheted 120 -> 148 KiB on 2026-10-24 (test image): C7, the CF
        // write test (rp-fat built a second time, for the CF card) and C8,
        // the CPLD interrupt reads (R28, R24); 133.8 KiB measured.
        // Ratcheted 148 -> 164 KiB on 2026-10-25 (test image): C9-C13, the
        // CF write ladders, each write's phases timed, fresh sectors and the
        // rewrite cost, the CFA erase commands, the rewrite cost by size
        // (R28); 150.9 KiB measured.
        max_load: 164 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Test image (ADR-0025 S0-S2): the MVED1 video decoder's clock, reset
    // and frame conversion. 27.7 KiB measured (2026-10-14; 36.6 KiB with
    // Debug formatting in its log, replaced by plain hex), + ~10%.
    // Ratcheted 31 -> 40 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 36.1 KiB measured, + ~10%.
    // Ratcheted 40 -> 42 KiB on 2026-10-22 (test image): the card log
    // no longer waits on a busy card (quality audit C59: sends, polls,
    // catches up at check-ins); 40.7 KiB measured.
    Budget {
        image: "rp-mved",
        // Ratcheted 42 -> 50 KiB on 2026-10-25 (test image): the on-screen
        // step label from the first moment (the owner's label-the-screen
        // rule) and the results screen's prompt; 45.8 KiB measured, +~10%.
        max_load: 50 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Test image (ADR-0031, audit item 67): the PXA270's clocks taken,
    // every operating point measured, turbo and relock costs. 28.7 KiB
    // measured (2026-10-15), + ~10%; then 32 -> 36 KiB for version 3
    // (breadcrumbs on screen and in 2700G memory after run 1 never
    // returned), 31.9 KiB measured, + ~12%.
    // Ratcheted 36 -> 41 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 40.2 KiB measured, + ~10%.
    Budget {
        image: "rp-clocks",
        // Ratcheted 41 -> 51 KiB on 2026-10-21 (test image): quality
        // audit C38, every crash through rp-image (the card-log line, the
        // crash box, the fault's word and SPSR). 46.3 KiB measured, +~10%.
        max_load: 51 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Bring-up experiment: USB reconnect candidates (ADR-0012). Provisional.
    // Ratcheted 24 -> 34 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 30.2 KiB measured, + ~10%.
    Budget {
        image: "rp-reconnect",
        max_load: 34 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Bring-up experiment: the USB disconnect hunt (docs/hw/udc.md T4).
    // Provisional, as rp-reconnect.
    // Ratcheted 24 -> 34 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 29.7 KiB measured, + ~10%.
    Budget {
        image: "rp-unplug",
        max_load: 34 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Diagnostics only: the first-light image (14.3 KiB measured, +~10%).
    // Ratcheted 16 -> 18 KiB on 2026-10-12: the shared crash path (crash
    // reports marked for the loader, kept on the card); 16.1 KiB measured.
    // 18 -> 21 KiB on 2026-10-15: the hand-off line (what the 2700G waited
    // for, item 72) and the CPLD shadow adopted from the shared table (item
    // 77); 19.0 KiB measured, +~10%.
    // Ratcheted 21 -> 27 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 23.6 KiB measured, + ~10%.
    Budget {
        image: "rp-light",
        max_load: 27 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Diagnostics only: hand-over facts (items 72 and 77). Set at its first
    // build on 2026-10-15: 18.8 KiB measured, +~10%.
    // Ratcheted 21 -> 26 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 23.0 KiB measured, + ~10%.
    Budget {
        image: "rp-handover",
        max_load: 26 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Diagnostics only: the night-light experiment (item 76), for the
    // owner's eyes and ears. Set at its first build on 2026-10-15: 19.6 KiB
    // measured, +~10%.
    // Ratcheted 22 -> 28 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 24.4 KiB measured, + ~10%.
    // Ratcheted 28 -> 30 KiB on 2026-10-22 (test image): the card log
    // no longer waits on a busy card (quality audit C59: sends, polls,
    // catches up at check-ins); 28.5 KiB measured.
    Budget {
        image: "rp-nightlight",
        max_load: 30 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Diagnostics only: CF detect with the slot unpowered (the re-insert
    // fix, docs/hw/cf.md). Set at its first build on 2026-10-16: 18.3 KiB
    // measured, +~10%.
    // Ratcheted 21 -> 26 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 22.7 KiB measured, + ~10%.
    Budget {
        image: "rp-cfdetect",
        // Ratcheted 26 -> 29 KiB on 2026-10-21 (test image): quality
        // audit C38, every crash through rp-image (the card-log line, the
        // crash box, the fault's word and SPSR). 26.3 KiB measured, +~10%.
        max_load: 29 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Test image: the audio check, moved out of the loader (its `tone`
    // command) in the loader diet of 2026-10-16. Set at its first build:
    // 22.7 KiB measured, +~10%.
    // 2026-10-18: + the codec's power (CPLD 0x1C bit 1 through the shared
    // shadow) and its I2C gate: 26.1 KB measured, 25 -> 28 KiB; then run 3
    // (four probe ways, the speaker amplifier, the jack on GPIO83): 27.7 KiB
    // measured, 28 -> 31 KiB; then 31 -> 35 KiB for the I2C and DMA
    // diagnostics and the I2S off-time measurement (quality-audit B8),
    // 31.6 KiB measured.
    // Ratcheted 35 -> 41 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 36.7 KiB measured, + ~10%.
    Budget {
        image: "rp-tone",
        // Ratcheted 41 -> 46 KiB on 2026-10-21 (test image): quality
        // audit C38, every crash through rp-image (the card-log line, the
        // crash box, the fault's word and SPSR). 41.1 KiB measured, +~10%.
        max_load: 46 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Test image: audio service step S1 (ADR-0032), everything through
    // rp-audio. Set at its first build on 2026-10-18: 33.3 KiB measured,
    // +~10%. Then 37 -> 43 KiB for the speaker amplifier diagnostic
    // (`RP_AUDIO_AMP_TEST`, runs 2-3's unheard note): 38.7 KiB measured.
    // Ratcheted 43 -> 48 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 43.3 KiB measured, + ~10%.
    Budget {
        image: "rp-audio-s1",
        // Ratcheted 48 -> 54 KiB on 2026-10-21 (test image): quality
        // audit C38, every crash through rp-image (the card-log line, the
        // crash box, the fault's word and SPSR). 48.4 KiB measured, +~10%.
        max_load: 54 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Test image: audio service step S2 (ADR-0032), the mixer through
    // rp-audio on the X51v's wiring (rp-audio-x51v). Set at its first build
    // on 2026-10-19: 70.1 KiB measured, +~10%. Then 77 -> 90 KiB the same
    // night: the 48 kHz coefficient table (6.5 KB, read-only), the faster
    // block path and the measurement modes (cycle counts, the bench):
    // 81.4 KiB measured.
    Budget {
        image: "rp-audio-s2",
        // Ratcheted 90 -> 100 KiB on 2026-10-21 (test image): quality
        // audit C38, every crash through rp-image (the card-log line, the
        // crash box, the fault's word and SPSR). 90.8 KiB measured, +~10%.
        max_load: 100 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Test image: display calibration C1 (docs/design/display-calibration.md),
    // the gamma table judged by eye. Set at its first build on 2026-10-17:
    // 25.9 KiB measured, +~10%. Raised for the guided run 5 (six dials,
    // saturation redraw, steps): 28.8 KiB measured, +~10%.
    // Test image: the screen's patches for a colorimeter (`cargo xtask
    // measure`). Set at its first build on 2026-10-17: 17.3 KiB measured,
    // +~10%. 2026-10-17 (C6): + the 8 KiB program room the PC writes a
    // measurement program into (28.4 KB measured).
    // Ratcheted 30 -> 37 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 32.5 KiB measured, + ~10%.
    // Ratcheted 37 -> 39 KiB on 2026-10-22 (test image): the card log
    // no longer waits on a busy card (quality audit C59: sends, polls,
    // catches up at check-ins); 37.0 KiB measured.
    Budget {
        image: "rp-screen-measure",
        max_load: 39 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Test image: the refresh-rate test (display calibration C3). Set at
    // its first build on 2026-10-17: 24.1 KiB measured, +~10%.
    // Ratcheted 27 -> 32 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 28.4 KiB measured, + ~10%.
    // Ratcheted 32 -> 34 KiB on 2026-10-22 (test image): the card log
    // no longer waits on a busy card (quality audit C59: sends, polls,
    // catches up at check-ins); 32.7 KiB measured.
    Budget {
        image: "rp-refresh",
        max_load: 34 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Ratcheted 32 -> 40 KiB on 2026-10-20 (test image): the card log for
    // every run the loader starts (rp_hal_x51v::cardsink, quality audit C24:
    // each line onto the SD or CF card the loader set up, ~4.5 KiB of sector
    // writes and the sync); 35.3 KiB measured, + ~10%.
    // Ratcheted 40 -> 44 KiB on 2026-10-24 (test image): the card log sets
    // the CF socket to Windows Mobile's timing before each write (R7);
    // 40.1 KiB measured.
    Budget {
        image: "rp-display-cal",
        max_load: 44 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
    // Test image: the C6 looks test (two LZ4-packed photos, chart, ramps
    // under the measured tables). Set at its first build on 2026-10-17 to
    // the loader's own ceiling for a loaded image (928 KiB): the owner's
    // photos decide its size, and a test image's size doesn't matter.
    Budget {
        image: "rp-looks",
        max_load: 928 * 1024,
        max_zero: 20 * 1024,
        device: OnDevice::Returns,
    },
];

/// The device's RAM (docs/memory-budget.md).
pub const DEVICE_RAM: u64 = 64 << 20;
/// What the OS must leave for apps: at least what Windows Mobile leaves on
/// the owner's X51v (about 52 MB free).
pub const APP_RAM_FLOOR: u64 = 52 << 20;

/// A part of the OS and the RAM it may take, top-down
/// (docs/memory-budget.md). The images listed count against it with their
/// whole budgets (loaded + zeroed).
pub struct Allotment {
    /// What it is.
    pub part: &'static str,
    /// Its share of RAM.
    pub bytes: u64,
    /// The images built so far that belong to it.
    pub images: &'static [&'static str],
}

const MIB: u64 = 1 << 20;

/// The OS's parts: together at most `DEVICE_RAM - APP_RAM_FLOOR`.
pub const ALLOTMENTS: &[Allotment] = &[
    Allotment {
        part: "kernel image (code, data, static tables, stacks)",
        bytes: MIB,
        images: &["rp-kernel"],
    },
    Allotment {
        part: "kernel memory in use (page tables, compressed store, thread stacks)",
        bytes: MIB,
        images: &[],
    },
    Allotment {
        part: "driver hosts (storage, display+input, audio, connectivity, power)",
        bytes: MIB,
        images: &[],
    },
    Allotment {
        part: "service manager and system services (files, registry)",
        bytes: MIB,
        images: &[],
    },
    Allotment {
        part: "window manager (GWES), shell, Today screen",
        bytes: 3 * MIB,
        images: &[],
    },
    Allotment {
        part: "Windows Mobile libraries (coredll, aygshell, GDI, GAPI; shared code)",
        bytes: 2 * MIB,
        images: &[],
    },
    Allotment {
        part: "networking (TCP/IP, TLS 1.3, Wi-Fi)",
        bytes: 2 * MIB,
        images: &[],
    },
    Allotment {
        part: "headroom",
        bytes: MIB,
        images: &[],
    },
];

/// Checks the top-down budget: the parts and the apps' floor fit the
/// device, each image belongs to one part at most, and each part's images'
/// budgets fit inside it.
pub fn check_allotments(
    budgets: &[Budget],
    allotments: &[Allotment],
    device: u64,
    floor: u64,
) -> Result<(), String> {
    let total: u64 = allotments.iter().map(|a| a.bytes).sum();
    if total + floor > device {
        return Err(format!(
            "the OS's parts take {} KiB: with {} KiB kept for apps that is more than the device's {} KiB (docs/memory-budget.md)",
            total / 1024,
            floor / 1024,
            device / 1024
        ));
    }
    for a in allotments {
        let mut used = 0;
        for image in a.images {
            if allotments
                .iter()
                .filter(|o| o.images.contains(image))
                .count()
                > 1
            {
                return Err(format!(
                    "{image} is counted in two parts of the memory budget"
                ));
            }
            let b = budgets
                .iter()
                .find(|b| b.image == *image)
                .ok_or_else(|| format!("{image} is in the memory budget but has no size budget"))?;
            used += b.max_load + b.max_zero;
        }
        if used > a.bytes {
            return Err(format!(
                "{}: its images' budgets ({} KiB) exceed its share ({} KiB) of the memory budget (docs/memory-budget.md)",
                a.part,
                used / 1024,
                a.bytes / 1024
            ));
        }
    }
    Ok(())
}

/// Loaded and zero-initialised byte counts of an ELF32 image.
#[derive(Debug, PartialEq, Eq)]
pub struct Sizes {
    /// PROGBITS sections that occupy memory.
    pub load: u64,
    /// NOBITS sections that occupy memory (.bss, stack).
    pub zero: u64,
}

const SHF_ALLOC: u32 = 0x2;
const SHT_NOBITS: u32 = 8;
const SHT_SYMTAB: u32 = 2;

/// Measures an ELF32 little-endian image by its section headers.
pub fn measure(elf: &[u8]) -> Result<Sizes, String> {
    let u16_at = |o: usize| elf.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |o: usize| {
        elf.get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    if elf.get(0..5) != Some(&[0x7F, b'E', b'L', b'F', 1]) {
        return Err("not an ELF32 file".into());
    }
    let shoff = u32_at(0x20).ok_or("truncated ELF header")? as usize;
    let shentsize = usize::from(u16_at(0x2E).ok_or("truncated ELF header")?);
    let shnum = usize::from(u16_at(0x30).ok_or("truncated ELF header")?);
    let mut sizes = Sizes { load: 0, zero: 0 };
    for i in 0..shnum {
        let sh = shoff + i * shentsize;
        let kind = u32_at(sh + 4).ok_or("truncated section header")?;
        let flags = u32_at(sh + 8).ok_or("truncated section header")?;
        let size = u64::from(u32_at(sh + 20).ok_or("truncated section header")?);
        if flags & SHF_ALLOC == 0 {
            continue;
        }
        if kind == SHT_NOBITS {
            sizes.zero += size;
        } else {
            sizes.load += size;
        }
    }
    Ok(sizes)
}

/// Images allowed to link software floating-point helpers, with the reason
/// (the no-float rule: the PXA270 has no FPU, so a float operation costs
/// tens to over a hundred cycles; hot code uses fixed point, and floats
/// the 3D core consumes are built from integers or folded at compile
/// time). Empty: no image needs one (checked 2026-10-15).
const FLOAT_ALLOWED: &[(&str, &str)] = &[];

/// Whether `name` is a software floating-point helper (EABI or
/// compiler-rt names) or float formatting (`core::fmt` printing an `f32`
/// costs ~20 KiB of integer code: `rp-mbx3d` version 21, 2026-10-15).
fn is_float_helper(name: &str) -> bool {
    if name.contains("float_to_decimal") || name.contains("flt2dec") {
        return true;
    }
    if let Some(f) = name.strip_prefix("__aeabi_") {
        // Every EABI float routine starts with f or d (fadd, dmul, f2iz,
        // d2uiz, fcmpeq...) or converts to one (i2f, ul2d...); none of the
        // integer ones (uidiv, ldivmod, memcpy...) does.
        return f.starts_with('f') || f.starts_with('d') || f.ends_with("2f") || f.ends_with("2d");
    }
    name.starts_with("__float")
        || name.starts_with("__fix")
        || ["sf3", "df3", "sf2", "df2"]
            .iter()
            .any(|t| name.starts_with("__") && name.ends_with(t))
}

/// The software floating-point helpers an ELF32 image's symbol table names.
pub fn float_helpers(elf: &[u8]) -> Result<Vec<String>, String> {
    let u16_at = |o: usize| elf.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |o: usize| {
        elf.get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let shoff = u32_at(0x20).ok_or("truncated ELF header")? as usize;
    let shentsize = usize::from(u16_at(0x2E).ok_or("truncated ELF header")?);
    let shnum = usize::from(u16_at(0x30).ok_or("truncated ELF header")?);
    let section = |i: usize| -> Result<(u32, usize, usize, usize), String> {
        let sh = shoff + i * shentsize;
        let field = |o: usize| u32_at(sh + o).ok_or("truncated section header");
        Ok((
            field(4)?,
            field(16)? as usize,
            field(20)? as usize,
            field(24)? as usize,
        ))
    };
    let mut found = Vec::new();
    for i in 0..shnum {
        let (kind, off, size, link) = section(i)?;
        if kind != SHT_SYMTAB {
            continue;
        }
        let (_, str_off, str_size, _) = section(link)?;
        let strtab = elf
            .get(str_off..str_off + str_size)
            .ok_or("string table out of range")?;
        for sym in (off..off + size).step_by(16) {
            let name_at = u32_at(sym).ok_or("symbol out of range")? as usize;
            let name = strtab.get(name_at..).unwrap_or(&[]);
            let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
            let name = String::from_utf8_lossy(&name[..end]);
            if is_float_helper(&name) && !found.iter().any(|n: &String| *n == name) {
                found.push(name.into_owned());
            }
        }
    }
    Ok(found)
}

/// Checks every budget against the built images.
pub fn check(release_dir: &Path, images: &[&str]) -> Result<(), String> {
    check_allotments(BUDGETS, ALLOTMENTS, DEVICE_RAM, APP_RAM_FLOOR)?;
    // Every image needs a budget: an unbudgeted image would grow unseen.
    if let Some(missing) = images
        .iter()
        .find(|i| BUDGETS.iter().all(|b| b.image != **i))
    {
        return Err(format!(
            "{missing} has no size budget in xtask/src/size.rs (ADR-0005 rule 7)"
        ));
    }
    // Every image is measured before any over its budget fails the check:
    // one change that grows many images shows its whole cost at once.
    let mut over = Vec::new();
    for b in BUDGETS {
        let path = release_dir.join(b.image);
        let elf = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let s = measure(&elf)?;
        eprintln!(
            "    {}: {} B loaded (budget {}), {} B zeroed RAM (budget {})",
            b.image, s.load, b.max_load, s.zero, b.max_zero
        );
        if s.load > b.max_load || s.zero > b.max_zero {
            over.push(b.image);
        }
        let floats = float_helpers(&elf)?;
        if !floats.is_empty() && FLOAT_ALLOWED.iter().all(|(i, _)| *i != b.image) {
            return Err(format!(
                "{} links software floating point ({}): the PXA270 has no FPU. Use fixed point, or fold it at compile time, or add the image to FLOAT_ALLOWED with a reason",
                b.image,
                floats.join(", ")
            ));
        }
    }
    if !over.is_empty() {
        return Err(format!(
            "{} exceed{} {} size budget (ADR-0005). Shrink, or justify raising the budget in review",
            over.join(", "),
            if over.len() == 1 { "s" } else { "" },
            if over.len() == 1 { "its" } else { "their" }
        ));
    }
    Ok(())
}

/// Checks one build of `image` (a test image's variant, built in its own
/// folder by the emulator checks) against the image's budget, as
/// `cargo xtask load` would on the device day (quality audit C45: run FF
/// passed the emulator 312 B over and was refused at the Axim).
pub fn check_variant(elf_path: &Path, image: &str) -> Result<(), String> {
    let b = BUDGETS.iter().find(|b| b.image == image).ok_or_else(|| {
        format!("{image} has no size budget in xtask/src/size.rs (ADR-0005 rule 7)")
    })?;
    let elf = std::fs::read(elf_path).map_err(|e| format!("{}: {e}", elf_path.display()))?;
    let s = measure(&elf)?;
    if s.load > b.max_load || s.zero > b.max_zero {
        return Err(format!(
            "{} ({}) exceeds its size budget (ADR-0005): {} B loaded (budget {}), {} B zeroed RAM (budget {})",
            image,
            elf_path.display(),
            s.load,
            b.max_load,
            s.zero,
            b.max_zero
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal ELF32 with three sections: alloc PROGBITS, alloc NOBITS,
    /// and non-alloc (debug) PROGBITS.
    fn elf(sections: &[(u32, u32, u32)]) -> Vec<u8> {
        let shoff = 0x40usize;
        let mut d = vec![0u8; shoff + sections.len() * 40];
        d[0..5].copy_from_slice(&[0x7F, b'E', b'L', b'F', 1]);
        d[0x20..0x24].copy_from_slice(&(shoff as u32).to_le_bytes());
        d[0x2E..0x30].copy_from_slice(&40u16.to_le_bytes());
        d[0x30..0x32].copy_from_slice(&(sections.len() as u16).to_le_bytes());
        for (i, &(kind, flags, size)) in sections.iter().enumerate() {
            let sh = shoff + i * 40;
            d[sh + 4..sh + 8].copy_from_slice(&kind.to_le_bytes());
            d[sh + 8..sh + 12].copy_from_slice(&flags.to_le_bytes());
            d[sh + 20..sh + 24].copy_from_slice(&size.to_le_bytes());
        }
        d
    }

    #[test]
    fn counts_only_allocated_sections() {
        let d = elf(&[
            (1, SHF_ALLOC | 0x4, 1000),
            (SHT_NOBITS, SHF_ALLOC | 0x1, 500),
            (1, 0, 99_999),
        ]);
        assert_eq!(
            measure(&d).unwrap(),
            Sizes {
                load: 1000,
                zero: 500
            }
        );
    }

    #[test]
    fn rejects_non_elf() {
        assert!(measure(b"MZ\x90\x00").is_err());
        assert!(measure(&[0x7F, b'E', b'L', b'F', 1]).is_err());
    }

    #[test]
    fn the_os_leaves_apps_their_floor() {
        // The real table holds.
        assert_eq!(
            check_allotments(BUDGETS, ALLOTMENTS, DEVICE_RAM, APP_RAM_FLOOR),
            Ok(())
        );
        let b = [Budget {
            image: "k",
            max_load: 600 * 1024,
            max_zero: 500 * 1024,
            device: OnDevice::Returns,
        }];
        let a = |bytes, images: &'static [&'static str]| Allotment {
            part: "p",
            bytes,
            images,
        };
        // An image whose budget outgrows its part.
        assert!(check_allotments(&b, &[a(MIB, &["k"])], 64 * MIB, 52 * MIB).is_err());
        assert!(check_allotments(&b, &[a(2 * MIB, &["k"])], 64 * MIB, 52 * MIB).is_ok());
        // Parts that leave apps less than the floor.
        assert!(check_allotments(&b, &[a(13 * MIB, &["k"])], 64 * MIB, 52 * MIB).is_err());
        // Counted twice, or with no size budget.
        assert!(check_allotments(&b, &[a(2 * MIB, &["k"]), a(MIB, &["k"])], 64 * MIB, 0).is_err());
        assert!(check_allotments(&b, &[a(MIB, &["nope"])], 64 * MIB, 0).is_err());
    }

    #[test]
    fn images_that_never_return_are_known() {
        assert_eq!(on_device("rp-bench"), Some(OnDevice::NeverReturns));
        assert_eq!(on_device("rp-probe"), Some(OnDevice::Returns));
        assert_eq!(on_device("rp-loader"), Some(OnDevice::Loader));
        assert_eq!(on_device("nope"), None);
    }

    /// An ELF32 whose only sections are a symbol table naming `names` and
    /// its string table.
    fn elf_with_symbols(names: &[&str]) -> Vec<u8> {
        let mut strtab = vec![0u8];
        let mut symtab = vec![0u8; 16];
        for n in names {
            let at = strtab.len() as u32;
            strtab.extend_from_slice(n.as_bytes());
            strtab.push(0);
            let mut sym = [0u8; 16];
            sym[..4].copy_from_slice(&at.to_le_bytes());
            symtab.extend_from_slice(&sym);
        }
        let shoff = 0x40usize;
        let sym_off = shoff + 3 * 40;
        let str_off = sym_off + symtab.len();
        let mut d = vec![0u8; str_off];
        d[0..5].copy_from_slice(&[0x7F, b'E', b'L', b'F', 1]);
        d[0x20..0x24].copy_from_slice(&(shoff as u32).to_le_bytes());
        d[0x2E..0x30].copy_from_slice(&40u16.to_le_bytes());
        d[0x30..0x32].copy_from_slice(&3u16.to_le_bytes());
        // Section 1: the symbol table, linked to section 2; 2: strings.
        for (i, (kind, off, size, link)) in [
            (SHT_SYMTAB, sym_off, symtab.len(), 2u32),
            (3, str_off, strtab.len(), 0),
        ]
        .into_iter()
        .enumerate()
        {
            let sh = shoff + (i + 1) * 40;
            d[sh + 4..sh + 8].copy_from_slice(&kind.to_le_bytes());
            d[sh + 16..sh + 20].copy_from_slice(&(off as u32).to_le_bytes());
            d[sh + 20..sh + 24].copy_from_slice(&(size as u32).to_le_bytes());
            d[sh + 24..sh + 28].copy_from_slice(&link.to_le_bytes());
        }
        d[sym_off..str_off].copy_from_slice(&symtab);
        d.extend_from_slice(&strtab);
        d
    }

    #[test]
    fn float_helpers_are_found_and_integer_ones_are_not() {
        let e = elf_with_symbols(&[
            "__aeabi_memcpy",
            "__aeabi_uidivmod",
            "__aeabi_ldivmod",
            "__aeabi_fmul",
            "__aeabi_ui2f",
            "__aeabi_d2iz",
            "__addsf3",
            "__floatsisf",
            "_RNvNtNtCs1_4core3fmt5float32float_to_decimal_common_shortest",
            "main",
        ]);
        assert_eq!(
            float_helpers(&e).unwrap(),
            [
                "__aeabi_fmul",
                "__aeabi_ui2f",
                "__aeabi_d2iz",
                "__addsf3",
                "__floatsisf",
                "_RNvNtNtCs1_4core3fmt5float32float_to_decimal_common_shortest"
            ]
        );
        assert!(
            float_helpers(&elf_with_symbols(&["__aeabi_memset", "main"]))
                .unwrap()
                .is_empty()
        );
    }
}
