# Overnight queue (2026-10-12)

Worked top to bottom, one item at a time, while the owner sleeps. Each item
is committed and pushed when `cargo xtask ci` and `cargo xtask emu` are
green, then ticked here with what was done to **iterate** and to
**modernize** it. The standard (docs/AI-CONTRIBUTING.md): every way the part
can beat the original, not one token improvement.

**Device rules overnight:** only images that passed the emulator go to the
Axim; nothing that could outlive the loader's 30 s run limit (a watchdog
reset lands on Dell's factory-restore prompt, which needs the owner to press
Mail; if that happens, device work stops and emulator/docs work continues);
nothing writes flash; card writes only through copy-then-swap.

## 1. Merge the worker's branch: FAT free-space scan in runs, KData wiring

- [x] Reviewed (the HAL page-table change: one-shot guard, old mapping's
      cache lines cleaned first, vectors kept on the same physical page),
      merged as 6db35fc; CI and the emulator green on main. On the Axim the
      merged loader writes a file in 7–9 card reads in total; the stale-hint
      scan (3.7 s once) drops from ~1,490 to ~18 reads by the emulator's
      measure (25× fewer in the host test), to confirm on the card when a
      stale hint next occurs.
- **Iterate:** the free-space scan reads the FAT in doubling runs through
  the caller's scratch (no new buffers, no heap), the cache stays coherent
  without extra flushes, the power-loss sweep runs both ways, and the reply
  now reports card reads so every write is measured; KData is written
  through its one user address so a thread switch needs no cache
  maintenance, checked by a live test with two mutations that make it fail.
- **Modernize:** card operations are measured and gated in CI; KData's page
  is user read-only (verified by a faulting write), and the same change
  closed a hole that left the vector page and the first MiB of SDRAM
  user-writable.

## 2. ADR-0014: the SD card as a drive in Windows (USB mass storage)

- [x] ADR-0014 written and accepted, with a Security section.
- [x] `drivers/usb-msc` (Bulk-Only + SCSI, LUNs), composite descriptors and
      the mass storage endpoints in `pxa-udc`, capacity from the CSD in
      `sdcard`, the loader runs the drive next to the serial port.
- [x] Emulator rp-v0.9.0: descriptor-driven routing, the drive bridged to a
      second chardev; `boot_loader_drive` mounts the card through USB with
      `rp-fat`, reads and writes both ways, checks the ownership rules and
      hostile input. Two real bugs found on the way (ADR-0014 status).
- Done so far against the plan: driverless class, composite, ownership by
  construction (busy refusal, medium-changed, eject), multi-block 32 KiB
  transfers, card absence as "not present", stable serial, LUN-ready for CF.
  Write-protect is plumbed (`Storage::read_only`, MODE SENSE, refused writes)
  but not yet wired to the switch: GPIO27's polarity is unconfirmed, so it
  waits for item 10's probe run rather than risk a wrong read-only card.
- **Iterate:** composite instead of a mode switch; corruption made impossible
  by construction (the device and PC never write the card at once, the PC is
  told when the device changed it, eject hands it back) rather than "please
  eject first"; every SCSI command host-tested including 5,000 random
  commands; the whole stack tested end to end through the emulated drive
  before the device; two latent USB timing bugs found and fixed for the real
  hardware on the way; LUNs ready for the CF card with no USB work.
- **Modernize:** the card shows up as a drive on any 2026 PC with no driver
  or sync software (Windows Mobile never offered it at all); standard
  compliant serial so it keeps its drive letter; hostile-input-proof parser
  per docs/security.md; storage exposure noted as development-only until the
  OS asks the user for consent.
- Improvements planned, all of them:
  - driverless on Windows 11, macOS and Linux (standard class, no INF);
  - composite, so `cargo xtask load` keeps working while the drive is
    mounted, no mode switch;
  - the card is handed to the PC only while it's mounted, and taken back on
    eject (the device never writes a card the PC owns: no corruption);
  - multi-block reads/writes straight from the USB buffer to the card, no
    staging copy, so it runs near the card's measured 1.7 MB/s;
  - write-protect switch honoured (GPIO27) and reported as read-only;
  - card removal reported as "medium not present", not an I/O error;
  - a proper volume name and serial so Windows remembers the drive;
  - the protocol layer is generic over any block device, so CF (and later
    the internal flash) become extra drives with no USB work;
  - size: ~6-8 KiB in the loader, measured and logged.

## 3. Device run of item 2

- [x] Enumeration worked first time on Windows 11 (composite: COM4 + "RetroPan
      SD card", read-only volume, the right capacity), with a layout
      fallback in place first so nobody could get locked out.
- [ ] **Blocked on the card, not the code:** a weak block at 10880 in the
      first FAT (0.79 s alone, never inside a multi-block read) makes
      Windows' mount retry forever. Six driver fixes (sd.md "Reads that
      fail") keep everything else working; the block itself needs a scrub
      (one rewrite after a backup) or another card: the owner's call. The
      drive is **off** for the night (`cargo xtask drive on` to offer it).
- **Iterate:** a fallback layout so an unattended device run could never
  lock the loop out; diagnostics over the serial port that turned "it's
  slow" into a block number and a status word in minutes (`drive status`,
  `card scan`); an SD driver that now recovers from timeouts, previous-
  command error bits, stuck transfers and slow blocks, learns weak ranges,
  and an emulator that reproduces the slow block.
- **Modernize:** the drive is read-only until asked and can be withdrawn
  like an empty reader (`drive off`), so the PC is never stuck on it; the
  card's health problem is reported, not hidden.

## 4. FAT: create files (and directories' first step)

- [x] `rp-fat` creates root-directory files: `create`, and `put` (replace or
      create), used by the loader's file frames, so `cargo xtask put` and
      `install` create what the card lacks. 7 new host tests (20 in all),
      the emulator test creates "RetroPane loader.lnk" through the loader
      and finds it by long and short name in the image.
- **Iterate:** crash-safe order proven by a power-loss sweep (no file or
  the whole file, never a broken one; the other files untouched); a full
  root directory grows; a full card gives the clusters back; names FAT
  forbids are refused before anything is written.
- **Modernize:** long names exactly as Windows writes them, Windows'
  `~1`/`~2` short names, and lower-case 8.3 names stored with Windows NT's
  case flags (no long-name entries needed), so the card reads naturally on
  any 2026 PC.
- Improvements planned: crash-safe order (clusters, FATs, then the
  directory entry last: a power loss leaves either no file or the whole
  file); long-name entries Windows displays correctly; name collisions
  resolved the way Windows does (`~1`, `~2`); power-loss sweep extended.

## 5. CF card groundwork

- [x] `docs/hw/cf.md`: socket 1, detect/ready/reset GPIOs, the PXA27x
      windows and timing registers, the card side (CIS, ATA), safety rules
      for experiments, open questions.
- [x] The probe reports the PC Card timing registers and a decoded CF line;
      run on the Axim: both sockets set up identically by Windows Mobile
      (MCMEM 0x00020508, MCATT 0x00020E08, MCIO 0x00020510), MECR 3, detect
      GPIO13 = 0, ready = 1, reset driven 0. Attribute memory deliberately
      not read with nobody present (a bus stall would mean Dell's prompt).
- **Needs the owner:** is a CF card in the slot? (settles detect polarity);
  which CPLD bit powers the slot (Dell OAL analysis: Fable-level RE).
- **Iterate:** the spec separates what's confirmed on the device from what
  the manual says, and the probe captures the state the driver must
  reproduce, before any driver code exists.
- **Modernize:** planned as LUN 1 of the USB drive (no USB work), with
  LBA28 cards up to 128 GiB (stock Windows Mobile handled far less).

## 6. GPIO14 / GPIO2

- [x] Recorded docked + enumerated: GPIO2 = 1 (output), GPIO14 = 0 (input),
      WM's both-edge interrupt on GPIO14 still armed (docs/hw/udc.md).
- [ ] **Needs the owner:** the same probe with the Axim *out* of the cradle
      (one undock, `cargo xtask load rp-probe` needs USB, so: undock, tap
      HaRET with the probe kit, photograph page 2's GPIO lines). That pair
      of readings settles GPIO14.

## Security pass (owner's requirement, 2026-10-12)

- [x] `docs/security.md`: principles, threat model, current exposures,
      per-parser checklist; charter and memory updated.
- [x] Hostile-input tests for every parser of outside input:
      `rp-loadproto` (20,000 random stream chunks), `rp-usb-msc` (5,000
      random commands), `pxa-udc` (50,000 random SETUP packets plus 2,000
      through the whole driver), `kernel/pe` (5,000 mutated images through
      every entry point), `rp-fat` (1,500 corrupted cards, looping FAT and
      root chains, out-of-range clusters, absurd geometry).
- **Found and fixed:** `rp-fat` trusted the card's FAT size and root
  cluster: forged values overflowed the sector arithmetic (a panic in test
  builds, silently wrong sectors in release). Mount now validates the whole
  geometry with checked arithmetic, so every cluster the code later trusts
  is in range. Listed in docs/security.md.
- **Iterate:** every parser now has a test that throws garbage at it, so a
  future change that reopens a hole fails CI.
- **Modernize:** the threat model's "hostile files" and "someone with a
  cable" rows are now backed by tests, not intentions.

## Card repair (owner approved, 2026-10-12)

- [x] `cargo xtask card scrub|repair|peek`: the weak block 10880 turned out
      to hold *damaged* FAT data (118 of 128 entries impossible values),
      with an intact, live mirror in the second FAT. `card repair` restored
      it under strict evidence rules (mirroring on, the mirror's entries all
      valid, at least 90% of the differing entries invalid in the damaged
      copy), verified it: reads in 0.9 ms instead of 796 ms.
- [ ] **Device reset:** a scan of the FAT's last 1,901 blocks outlasted the
      run limit (the scan fed the watchdog only once; fixed: it feeds per
      block). The Axim is on Dell's factory prompt until the owner presses
      Mail; then: re-scan blocks 15,074-16,975, then `cargo xtask drive on`
      and check Windows mounts the drive.
- **Iterate:** never write from belief: compare with the card's own backup,
  judge the evidence, refuse when it's ambiguous (the tool refused twice
  before the rules matched the facts), verify after.
- **Modernize:** a self-healing path for FAT damage that stock Windows
  Mobile would have needed a PC and `chkdsk` for.

## Second tier (when the above is done or blocked)

Each held to the same standard: every improvement that makes sense, tested
on the host or in the emulator first.

7. **Crash log that survives a reset** (ROADMAP Phase 2): **done.**
   `devices::crash_to_loader` (one shared path for faults and panics):
   message into the run log, run marked crashed, straight back to the
   loader, which keeps the report on the card (`crashlog.txt`, crash-safe
   write, before anything else) and sends it to the PC; `cargo xtask load`
   fails with the reason. Hangs (watchdog) are saved the same way. New test
   image `rp-crash` (6.6 KiB) panics on purpose; `cargo xtask emu` checks
   the PC report, no watchdog trip, and the file on the card. On the Axim:
   the panic came back in about a second, report on the PC and the card.
   - **Iterate:** a crash costs a second instead of a watchdog reset and a
     trip through Dell's prompt (which needed the owner); one crash path
     for every image instead of copies; tested end to end.
   - **Modernize:** crash reports persist and reach the developer by
     themselves, like modern crash reporting, instead of a photo of the
     screen.
8. **Performance suite v1** (ROADMAP): **device numbers in.** `rp-threads`
   tees its results into the run log and returns to the loader, so `cargo
   xtask load rp-threads` gives the real X51v's figures: all 13 tests pass
   on hardware for the first time; switch ~12 µs (an open lead against the
   2 µs target), interrupt-to-thread ~8 µs, sleep lateness ~17 µs; memcpy
   ~95 MB/s already measured. docs/perf.md.
   - **Iterate:** the numbers come from the device in seconds, so the
     switch-time lead can be chased one suspect at a time.
   - **Modernize:** honest measurement instead of assumptions: the switch
     misses its target and says so.
9. **exFAT reading** in `rp-fat`: **done.** `rp_fat::exfat` from the
   published specification: boot-region checksum and every field checked at
   mount, entry-set checksums, contiguous and FAT-chained files, zeros past
   the valid data length; a host formatter builds test cards; 5 tests incl.
   2,000 hostile cards. The probe falls back to exFAT and its emulated card
   is now exFAT ("exFAT 4096 B/cluster", file listed).
   - **Iterate:** checksums verified (boot region and every entry set), so a
     damaged card is refused or the damaged entry skipped, not misread;
     separate module so only images that read cards pay for it.
   - **Modernize:** SDXC cards up to 2 TB readable; stock Windows Mobile
     stopped at FAT and 32 GB.
10. **SD card detect and write-protect** (GPIO12/GPIO27): **done.** From
    the owner's probe: detect active low (0 = card in), lock high = locked.
    The loader reads both once a second (debounced: two readings must
    agree), hot-swaps (a card put in is initialised on the kept controller,
    `SdCard::try_init` hands the controller back on failure; one taken out
    is "no medium" to the PC), and honours the lock everywhere (USB drive
    writes, `put`, `install`, scrub, repair). Emulator rp-v0.9.3 wires both
    lines; `boot_loader_locked` checks a locked card refuses a file and
    stays unchanged (the emulated card itself stays writable, as a real one
    does, so ignoring the switch fails the test).
    - **Iterate:** hot swap without a restart; debounced switches; a card
      that answers is used even if the switch reads oddly (a wrong polarity
      can't hide it).
    - **Modernize:** the lock switch respected as users expect from any
      card reader; the drive tells the PC when the card changes.
11. **Backlight** (PWM): **done in the emulator; device run pending (the
    Axim is on Dell's prompt).** docs/hw/backlight.md; `rp-drv-pxa-pwm`
    (4 tests): perceptual levels, a floor above 0, full duty at 100,
    16-step fades, Windows Mobile's period kept (audible-whine reasoning
    recorded); `cargo xtask backlight [0-100]`; emulator rp-v0.9.4 models
    PWM0-3, and `boot_loader_backlight` checks the registers via the
    monitor (new `Monitor::query`).
    - **Iterate:** levels that look linear, smooth fades, no accidental
      black; the existing period respected; registers checked by a test.
    - **Modernize:** brightness controllable from the PC; the fade and the
      perceptual scale are what users expect of a 2026 device.
12. **Compatibility corpus scanner** (ROADMAP Phase 3 start): **done.**
    `tools/apiscan` (host tool on `rp-kernel-pe`): import tables of every PE
    file in a corpus, ordinals resolved through the corpus's own export
    tables (unresolved 34,893 -> 3,259 once the XIP region's `coredll` is
    in), ranked by importing modules into `docs/research/api-backlog.md`:
    519 modules of the owner's ROM, 4,361 functions; `coredll` leads with
    1,315 functions used by 432 modules (`LocalFree`, `memset`, the stack
    cookie pair, `CloseHandle`, `memcpy`, the registry...). Host-tested
    (no emulator part: it's a PC tool). Clean-room: interface facts only.
    - **Iterate:** the CE personality gets built in the order real
      programs need it, measured, not guessed; re-runnable on any corpus
      (third-party apps next).
    - **Modernize:** a data-driven compatibility plan instead of a
      reimplementation of everything WM had.
13. **Docs sync**: **done.** ROADMAP (SD loader, USB client, perf suite
    ticked; scheduler, SD/FAT/exFAT, CF, backlight progress noted) and
    DESIGN §10b (mass storage row, the loader's card updates, the crash log)
    brought up to date. Coverage: every new crate has host tests and its
    parsers hostile-input tests (security pass).
    - **Iterate:** the plan documents say what is true tonight, so the next
      session (or contributor) starts from facts.
    - **Modernize:** the PC-side story is now standard classes only:
      serial, mass storage, and later NCM/MTP; no sync software.

## Third tier (added 2026-10-12, same rules)

14. **Emulator release bump**: **done.** rp-v0.9.4 (SD switches, PWM) in
    `QEMU_RELEASE`, bench re-recorded; GitHub CI now runs the locked-card
    and backlight tests too.
    - **Iterate / modernize:** the public CI covers every emulator test the
      local runs do; nothing silently skipped on GitHub.
15. **FAT subdirectories** in `rp-fat`: **done.** Paths with `/` or `\`
    (`.`/`..` and more than 16 levels refused), find/read/create/replace in
    folders, `mkdir` (first cluster with `.`/`..` written and linked before
    the entry), `put` creating missing parents like `mkdir -p`, listings
    per folder, short names unique per folder; one shared entry writer and
    one shared lookup for files and folders. 3 new tests (33 in all):
    folders end to end, power loss at every write of a put that makes two
    folders and a file, a folder pointing back at the root (corrupt, not a
    loop). Emulator: the loader puts "Apps/Sub Folder/hello.txt" on a card
    with no such folders and the image holds it.
    - **Iterate:** the same crash-safety proof extended to folders; loops
      and absurd depths refused; less code for both kinds of entry.
    - **Modernize:** `cargo xtask put file Apps/Game.exe` organises the card
      like any modern device, folders made as needed.
16. **exFAT writes**: **done (emulator).** `ExVolume::put` replaces and
    creates root-directory files: free space from the allocation bitmap; a
    file that fits one free run is stored with no FAT chain (exFAT's
    contiguous files), else chained through the FAT; data and bitmap first,
    then one sector write switches the directory (in place when the file
    and stream entries share a sector, else a new set then the old one
    deleted; new sets at the end of the directory are written
    last-sector-first behind the end marker), then the old clusters freed;
    VolumeDirty set for the duration; name hashes computed as Windows
    looks names up. `AnyVolume` lets the loader's `put` and crash reports
    use either file system. Tests: 7 new (power cut at every write of four
    kinds of put, hostile cards, fragmentation, sector boundaries, a cut
    long name that matched its prefix: a real bug, fixed). Emulator:
    `boot_loader_put_exfat`. Independent check still to do: `fsck.exfat`
    (exfatprogs, not installed in WSL; installing needs the owner).
    - **Iterate:** contiguous files need no FAT writes at all; fewer reads
      than FAT32 (8 vs 12 for the same put); the directory walk checks set
      checksums as entries stream past instead of buffering whole sets;
      the loader is built for size (−12%), paying for the feature.
    - **Modernize:** SDXC cards (64 GB+) get the same crash-safe tools as
      SD/SDHC; the dirty flag tells the next PC to check the card after a
      cut, as modern file systems do.
17. **Power button**: **done in the emulator; device run pending.** GPIO0,
    resting at 0 with WM watching both edges (probe). The loader takes the
    resting level at start and counts a 30 ms-debounced change from it as a
    press (polarity needn't be known); a press fades the screen off, the
    next back on at the level it had. Emulator rp-v0.9.5: an `x51v-button`
    model on GPIO0 pressed through `qom-set`; the backlight test presses it
    twice and checks PWM0 (0, then 60%). The planned long-press "back to
    Windows Mobile" was dropped: the only software reset on the PXA27x is
    the watchdog, which lands on Dell's prompt; recorded here instead.
    - **Iterate:** polarity-proof by design; debounced; the screen comes
      back exactly as it was.
    - **Modernize:** press for screen off/on with a fade, as on any phone.
18. **SD DMA**: **done in the emulator; device run pending.** A DMA channel
    in the HAL (descriptor chains, 4 KiB each; cache maintenance inside it;
    whole cache lines only) behind a small `DmaEngine` trait (arm, poll,
    abort), so the MMC driver stays safe code tested against a fake engine
    (6 new tests: routing, refusals, card errors mid-transfer, command
    timeouts, bus errors, per-direction switches). Multi-block reads and
    writes use it; single blocks and refused buffers use the FIFO. The
    loader self-tests DMA reads at start-up and keeps writes on the FIFO
    until `card dma writes`. Emulator rp-v0.9.6 fixes three model faults
    found on the way (see `docs/hw/emulator.md`). Bench: `sd_read_4_blocks`
    13,635 → 1,250 ns of CPU work; new `sd_read_64_blocks` and an on-target
    check that DMA reads equal single-block reads; the bench now fails on
    any failed on-target check, not just the memory one.
    - **Iterate:** the CPU is free while the card transfers (a 32 KiB read
      costs it ~13 µs of work); errors end a DMA at once instead of hanging;
      the rollout is gated by a self-test, so unproven hardware behaviour
      can't corrupt the card.
    - **Modernize:** scatter-gather DMA with the cache rules enforced by the
      type that owns the channel, the way modern kernels' DMA APIs do;
      measured, not assumed (instruction-count bench plus a device baseline
      row in the probe).

## Fourth tier (added 2026-10-12, same rules)

19. **CompactFlash**: **done in the emulator; device run pending.**
    `rp-drv-cf`: memory mode (every CF storage card has it), CIS checked
    (only fixed disks taken; I/O cards untouched), soft reset, IDENTIFY
    (LBA28 capacity, model), write cache off, READ / WRITE SECTORS (256 per
    command), all waits bounded; 8 tests against a simulated card
    (including 3,000 hostile cards and a bad sector). Board HAL
    `take_cf()` (Windows Mobile's socket timings, `MECR`). Loader `cf`
    command: identify, read-only, only when GPIO13 says a card is in.
    Emulator rp-v0.9.7: QEMU's PC Card socket and CF card model wired into
    the x51v machine, detect on GPIO13; `boot_loader_cf` identifies a
    16 MB card, and the put test checks an empty slot is reported, not
    touched. The card is the USB drive's second unit (LUN 1, "CF card"):
    an empty reader until `cf` attaches it, read-only until `drive rw`,
    empty again when it leaves the slot; the emulator test reads it as the
    PC would, is refused a write while read-only, writes after `drive rw`
    and finds the write in the card image.
    - **Iterate:** the write cache goes off, so the file systems' crash
      safety holds on CF too (Windows Mobile's driver leaves it on);
      non-storage cards are recognised from the CIS and left alone; the
      socket is never touched unless the detect line says a card is there.
    - **Modernize:** one `BlockDevice` for SD and CF, so the same file
      systems, the USB drive and the tools work on both; both cards show
      on the PC as drives, as on any modern device with card slots; CF
      cards past WM's limits (LBA28: up to 128 GiB).
20. **exFAT folders**: **done (emulator).** Paths with `/` or `\` on
    exFAT as on FAT32: `find`, `for_each_in`, `mkdir`, and `put` making
    missing parents. Folders are made as Windows makes them (one cluster,
    no FAT chain); `mkdir` zeroes and marks the cluster before the entry
    that makes the folder exist. Every directory routine now takes the
    directory it works in (one run, or a chain bounded by the volume), so
    a folder pointing back at the root is an error, not a loop. 4 new
    tests (paths, a full folder, a power cut at every write of a put
    making two folders, a looping folder; the power-cut test fails with
    the order reversed). Emulator: the exFAT put test now puts into new
    folders and finds the file in the image. `rp-loader` budget 96 ->
    104 KiB (with item 19).
    - **Iterate:** one code path for the root and every folder; folder
      creation crash-safe by order; corrupt folders bounded.
    - **Modernize:** SDXC cards organise into folders like any other card
      (`cargo xtask put Apps/Game.exe` works on both file systems).
    A full folder answers "no room" (folders don't grow yet: 4 KiB+
    clusters give 40+ files per folder, SDXC's usual 128 KiB, 1,300+).
21. **Thumb for cold code**: **measured; ADR-0015 proposed, adoption waits
    for the owner** (it changes how every image is built). The file
    systems and CF driver as the loader builds them: ARM 30,421 B, Thumb
    21,083 B (−31%); roughly 20 KiB back in the loader. Per-crate
    `+thumb-mode` was tried and rejected (rustc: unstable, unsound, being
    removed). Proposed: a Thumb target for cold code with the HAL's
    coprocessor assembly and hot loops marked ARM, where a missed mark is a
    build error, not a device fault; steps measured in the emulator first.
22. **Audio**: **first sound done in the emulator; device run pending
    (with the owner there).** `rp-drv-pxa-i2c` (polled master, bounded,
    a refused transaction always ends in a stop), `rp-drv-wm8750` (found
    at either address, playback set-up, volume in 1 dB steps, changes at
    zero crossings), `rp-drv-pxa-i2s` (PXA clock master, 48/44.1 kHz) with
    `Tone` (fixed-point sine, accurate to 0.1%). Loader `tone <hz> <ms>`.
    Emulator rp-v0.9.8: the WM8750 and I2S models wired into the x51v
    machine with a recordable output; `boot_loader_tone` records
    `tone 440 500` to a wav and measures 439.6 Hz for 0.496 s. 11 new host
    tests. Spec: `docs/hw/audio.md`. DMA streaming is the next step.
    - **Iterate:** the codec is found rather than assumed (the address
      isn't documented), volume steps are even to the ear, and the test
      listens to the output instead of trusting the register writes.
    - **Modernize:** sound tested in CI like everything else, from the
      register writes to the pitch of what comes out.

## Fifth tier (added 2026-10-12, same rules)

23. **Audio streaming by DMA**: **done (emulator).** `DmaEngine` takes a
    `Port` (request line, FIFO address, access width); `DmaRing` loops a
    buffer (the last descriptor points at the first) and reports where it
    reads; channels 0 (SD) and 1 (audio), each with its own chain (32
    descriptors, so two channels cost the RAM one did). `stream` refills
    the half the engine has left; 2 new tests (every frame in order then
    silence; a stuck engine reported). The loader's `tone` now streams:
    500 ms in 517 ms with the CPU busy under 1% (polled: 100%); recorded
    pitch 439.5 Hz. Bench unchanged (SD on the new API).
    - **Iterate:** the CPU is free while sound plays; the ring's cache rule
      is enforced by handing the engine the bytes written; stalls are
      errors.
    - **Modernize:** double-buffered DMA audio, the shape of every modern
      audio stack, measured in CI.
24. **Headphone detect**: **done (emulator); polarity to confirm on the
    device.** The loader reads GPIO83 each second (debounced over two
    reads), routes the codec (`Wm8750::route`: headphones mute the speaker
    pair; unplugging brings it back) and logs each change with the raw
    level. Emulator rp-v0.9.9: the jack as a switch on GPIO83; the tone
    test plays to the speaker, plugs headphones in, and the next tone goes
    to them. 1 new codec test. `rp-loader` budget 104 -> 112 KiB (audio).
    - **Iterate:** the speaker mutes itself when headphones go in, as
      expected of any device; unknown polarity handled by logging levels,
      not guessing silently.
    - **Modernize:** automatic output switching, tested end to end.
25. **Growing folders**: **done.** FAT32 already grew every folder (root
    included). exFAT now does too: a zeroed cluster marked used first; the
    root links it through the FAT; a folder stays one run when the next
    cluster is free, or its run is chained through the FAT first (ignored
    while its entry still says "no FAT chain"); then one sector write of
    the folder's own entry (size, chain flag) makes it part of the folder.
    New sets that spill into sectors not there yet grow the folder before
    anything is written. Tests: folders growing in one run, by a chain,
    and the root; a power cut at every write of a put that grows a folder,
    with free space full of valid-looking entries, so a cluster that isn't
    zeroed shows up (the test fails with the zeroing removed; an earlier
    version of it didn't, and was fixed).
    - **Iterate:** folders keep exFAT's contiguous layout when they can;
      growth is crash-safe by order; the test proves the zeroing matters.
    - **Modernize:** no folder size limit on SDXC cards.

## Sixth tier (added 2026-10-12, same rules)

26. **Crash screen**: **done (emulator).** The loader draws it: every
    image comes back to the loader after a crash (or, after a hang, through
    the watchdog), so nothing broken has to draw. A red band says "The
    program crashed" or "The program stopped responding" and where the
    report is (the card, or the PC); below, the end of the program's log,
    whole lines chosen to fit (no scrolling a screen per line of a long
    log), the `PANIC`/`FAULT` line in yellow. `rp_drv_mbx_display::crash`
    (2 host tests); the emulator's crash test screenshots the screen and
    checks the band, its text and the log.
    - **Iterate:** one crash screen for every image, drawn by the program
      that is known to work; the cause stands out.
    - **Modernize:** a readable crash report on the device itself, as any
      modern OS shows, besides the PC and the card.
27. **Processes and IPC: design**: **ADR-0016 proposed; waits for the
    owner.** Service calls migrate the calling thread into the server (CE's
    own model: fast, the caller's priority, drivers see their caller), on
    ports reached only through handles; bulk data moves page by page from
    owner to owner (no shared pages between slots: VIVT caches would alias
    them); notifications are events; the driver host supervises and
    restarts drivers; a service manager hands out ports per policy. Open
    questions for the owner at its end.

## Seventh tier (added 2026-10-12, same rules)

28. **Browse the card from the PC**: `cargo xtask ls [folder]` and
    `cargo xtask get <file> [local name]` through the loader, read-only,
    FAT32 or exFAT, with the transfer checked (CRC) end to end.
29. **Card check** (`card check`): a read-only consistency check of the
    card's file system, as a modern fsck: FAT32 chains (loops, cross-links,
    clusters used but marked free, lost clusters), exFAT (files against the
    allocation bitmap, set checksums), reported, never "fixed" silently; run
    by the emulator tests after every write test.

Device items waiting for the Axim (Dell prompt: owner presses Mail).
**First, one command:** `cargo xtask device-check` runs every read-only
check below in one go (drive counters, the SD DMA self-test verdict, the
backlight, the unscanned blocks 15,074-16,975 in 400-block commands) and
writes `target/device-check.txt`; `cargo xtask device-check --with-owner`
adds the CF card and a tone (someone at the device: it makes sound).
scan blocks 15,074-16,975, drive on + Windows mount check, backlight and
power button on the device, thread-switch time investigation, SD DMA on the
device (`cargo xtask card dma` after boot: the self-test verdict; then
`card dma writes` and a `put`, which verifies by read-back), exFAT image
check with `fsck.exfat` (needs exfatprogs in WSL: the owner installs it).
