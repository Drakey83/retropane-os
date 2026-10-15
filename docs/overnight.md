# Overnight queue (2026-10-12)

Worked top to bottom, one item at a time, while the owner sleeps. Each item
is committed and pushed when `cargo xtask ci` and `cargo xtask emu` are
green, then ticked here with what was done to **iterate** and to
**modernize** it. The standard (docs/CONTRIBUTOR-BRIEF.md): every way the part
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
  which CPLD bit powers the slot (Dell OAL analysis: hard RE, done in bounded steps proven on the device).
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
27. **Processes and IPC: design**: **ADR-0016 accepted by the owner
    (2026-10-12) with a driver host per driver class.** Service calls migrate the calling thread into the server (CE's
    own model: fast, the caller's priority, drivers see their caller), on
    ports reached only through handles; bulk data moves page by page from
    owner to owner (no shared pages between slots: VIVT caches would alias
    them); notifications are events; the driver host supervises and
    restarts drivers; a service manager hands out ports per policy. Open
    questions for the owner at its end.

## Seventh tier (added 2026-10-12, same rules)

28. **Browse the card from the PC**: **done (emulator).** `cargo xtask ls
    [folder]` and `cargo xtask get <file> [local name]`: the loader sends
    `RPD` data lines before its reply (a listing entry each; a file as
    base64, a third bigger than the bytes rather than double as hex) and
    the reply carries the length and CRC-32, which the PC checks. FAT32 or
    exFAT (`AnyVolume::list`); refused while the PC writes through the
    drive; `get` never overwrites a local file. Base64 in `rp-loadproto`
    (3 tests: known values, any data in any pieces, rubbish refused).
    Emulator: the put test lists a new folder and fetches the 70 KB file it
    put, byte for byte. `read_back` takes a `dyn` callback now, so each file
    system's reader exists once in the loader (−2.4 KB).
    - **Iterate:** files come back checked end to end; nothing on either
      side is overwritten by accident.
    - **Modernize:** the card browsable from the PC's terminal over the
      same cable, besides the drive.
29. **Card check**: **done (emulator).** `card check` (and `cargo xtask
    card check`; part of `device-check`): every folder walked from the
    root, every cluster used marked once in a lent bitmap (a bit per
    cluster: 122 KB for the owner's 4 GB card, from the scratch area),
    then the FAT or the exFAT bitmap compared: cross-links and loops, bad
    chains, sizes against chains, lost clusters (counted, harmless), exFAT
    clusters used but marked free (dangerous) and bad entry sets, FAT32
    copies that differ. Folders read 16 entries at a time, nothing grows
    with a folder; reported, never fixed silently. 8 tests (clean after
    many writes, every kind of damage found on both, power cuts never
    damage, hostile volumes never panic). Emulator: the put test's check
    finds the FAT sector it was given damaged, then checks clean after the
    repair; the exFAT test checks clean with no lost cluster.
    `rp-loader` budget 112 -> 124 KiB (with item 28).
    - **Iterate:** the writers are now checked by separate code, not just
      read back; the owner's card can be checked for more weak FAT
      sectors.
    - **Modernize:** a built-in, read-only fsck that explains, rather than
      silently "repairing".

## Eighth tier (added 2026-10-12, same rules)

30. **SHA-256 for what the loader receives**: **done (emulator).**
    `rp-crypto` (`kernel/crypto`): SHA-256 from FIPS 180-4, no unsafe, no
    allocation, a 16-word message schedule ring; the published vectors
    (empty, "abc", both multi-block messages, a million "a") and any split
    of the input give the same digest. The loader acknowledges an image
    with `RPL OK sha256 <hex>` of what will run, and a file with the
    SHA-256 of what it read back from the card; `cargo xtask load` and
    `put` compare with their own (an older loader is accepted with a note).
    Emulator: the USB load test and both put tests check the fingerprints;
    an old loader fails them.
    - **Iterate:** what the PC knows it sent is proven to be what runs and
      what the card holds, not just "the CRC matched".
    - **Modernize:** a cryptographic fingerprint in the dev loop, and the
      first piece of the crypto the networked OS needs (TLS, signed
      images).
31. **The loader idles**: **done (emulator).** After 50 ms with nothing
    happening (no input, no drive command under way or new, nothing to
    send) the loop sleeps until the next 1 ms tick (the XScale's idle
    mode) instead of spinning; the tick is stopped and IRQs masked before
    an image starts. `drive status` reports the last second's idle share.
    Emulator: a waiting loader is 99% asleep and answers in 1-2 ms; the
    drive test now fails if 1 MiB takes over 8 s, which caught the first
    version napping between packets of a long read (17 s instead of 3):
    `Msc::busy` (new, tested) now keeps it awake mid-command.
    - **Iterate:** sleeps only when nothing is happening, so transfers keep
      their speed; the idle share is measured, not assumed.
    - **Modernize:** an idle device that actually idles: less heat and
      battery while docked and waiting.

## Ninth tier (added 2026-10-12, same rules)

32. **HMAC-SHA256 and the loader's security, designed**: **done; ADR-0017
    proposed, waits for the owner.** `rp-crypto`: HMAC-SHA256 (RFC 2104,
    all six full-tag RFC 4231 vectors) and `ct_eq`, a constant-time tag
    compare. The bench now checks SHA-256 and HMAC on the target (and
    fails if the check is missing, as for the memory and SD checks) and
    times them: 4 KiB hashed in ~253k instructions, a 64-byte HMAC in ~21k.
    `rp-bench` budget 30 -> 34 KiB. ADR-0017 (deferred by the owner until
    release, then required): pairing confirmed with a
    hardware button on the device (as phones do for USB debugging), the
    key kept in RAM only (nothing on the card to steal: the card is a
    drive), every later frame HMAC-tagged with a sequence number, the drive
    offered only after pairing.
    - **Iterate:** the loader's open door gets a design that fits this
      device's facts (no secure storage, no RNG, a card the attacker can
      read).
    - **Modernize:** RFC-checked crypto building blocks, timing-safe
      comparison, checked on the real instruction set.
33. **Deleting files**: **done (emulator).** `remove` on FAT32 (the short
    entry and the long-name entries in its sector in one write, those that
    spill into the sector before next, then the chain freed in both FATs
    and FSInfo) and exFAT (the entry set, file entry's sector first, then
    the bitmap), files or empty folders (`NotEmpty` otherwise); a power cut
    at any write leaves the file whole or gone, never damage (`check` after
    every cut). Loader `rm` (`cargo xtask rm`): refused on a locked card,
    while the PC writes, and for the files the device boots from. 3 tests;
    the emulator put test deletes a file, is refused the loader and a
    non-empty folder, and finds the file gone from the image.
    `rp-loader` budget 124 -> 128 KiB.
    - **Iterate:** deletion as crash-safe as writing, proven by the checker;
      no stray long-name fragments; the boot files protected.
    - **Modernize:** card housekeeping from the PC's terminal, with guard
      rails a modern tool has.

## Tenth tier (added 2026-10-12, same rules)

34. **Keys as Windows Mobile programs see them**: **done (emulator).**
    `rp_drv_pxa_keypad::keys`: each button's public CE virtual-key code
    (D-pad: arrows and Return; Calendar, Contacts, Mail, Home, Record and
    the wireless button: `VK_APP1`-`VK_APP6`, numbering "likely", to
    confirm against the owner's Buttons settings), D-pad auto-repeat
    (after 500 ms, 20 a second), and WM6's "press and hold" on the program
    buttons (one Hold after a second; the release marked so the press
    action doesn't also run). No button gets a job Windows Mobile didn't
    give it (owner's rule). 4 tests (codes, repeat with a late caller, hold
    once, independent keys across the clock wrapping). The hello image
    reports the events; the emulator holds the D-pad (repeats) and
    Contacts (hold). `rp-hello` budget 22 -> 24 KiB.
    - **Iterate:** buttons behave as on the owner's Axim, measured in
      milliseconds, with nothing new to learn.
    - **Modernize:** press-and-hold and repeat as one tested layer every
      program gets, instead of each app timing keys itself.

35. **Thumb for cold code, steps 1-2** (ADR-0015, accepted by the owner
    with a revert condition): **done (emulator).** Every HAL function with
    inline assembly marked `instruction_set(arm::a32)` (31 functions; a
    no-op in ARM builds); the global assembly was already `.arm`; the
    copy/fill loops use lr instead of r7 (Thumb's frame pointer), same
    speed (bench unchanged). A Thumb target; `xtask cross` builds the
    images in `THUMB_IMAGES` as Thumb (now the loader: 127.7 -> 94.2 KB)
    and puts them where everything downstream expects. Every emulator test
    passes with the Thumb loader; drive and put timings unchanged within
    noise. Findings: ARM-only instructions in Thumb functions are build
    errors, as planned; exception returns (`movs pc, lr`, `ldm ...^`)
    already restore the Thumb bit; system calls pass their number in r7,
    so the call stub stays ARM (it is).
    - **Iterate:** a third of the loader back without touching its speed;
      one list decides ARM or Thumb per image.
    - **Modernize:** mixed-ISA builds checked by the compiler, measured,
      reversible in one line.
    Device: try the Thumb loader in RAM first (`cargo xtask load
    rp-loader`, the card's loader untouched), then `cargo xtask install`.

36. **IPC, step 1: the rules** (ADR-0016, accepted): **done (host tests;
    the emulator test comes with step 2).** `rp-kernel-ipc`: ports opened
    by a server slot, per-process handle tables (a handle is an index into
    the caller's own table: other processes' handles can't be named),
    rights that can only shrink when handed on (`GRANT`), migrating-call
    frames with bounded nesting (4), a faulting server unwinding to its
    caller, restarts and port reuse bumping a generation so old handles
    fail as `Stale` instead of reaching a new server, exits closing ports
    and emptying tables; all fixed-size. 9 tests, one of them 100,000
    random operations from hostile processes checking a call only ever
    passes through a live handle with `CALL`. Step 2 (needs threads and
    processes in one image, the kernel's next step per ADR-0010): the
    context switch that carries a call (FCSE PID and domain switch, the
    per-thread server stack), its emulator test and a round-trip timing.
    Step 3: the service manager and the per-class driver hosts.
    - **Iterate:** CE's migrating calls with the checks CE lacked: no
      global handle namespace, rights, stale-handle safety after restarts.
    - **Modernize:** capability-style handles, verified against random
      hostile use before any of it runs.

37. **IPC, step 2: the call, live** (ADR-0016): **done (emulator).**
    `rp-kernel` runs two user processes (slots 5 and 6). The client calls
    the server 1,000 times through a port: each call migrates the thread
    (saves the caller's registers, clears all but the arguments, switches
    FCSE PID and DACR domain, enters the server on its own stack) and
    returns the sum. A server that faults mid-call unwinds to its caller
    with an error, a handle the client never had is refused, and each
    process keeps its own word at the same address. The emulator test
    requires all four `IPC ... OK` lines (absent without this step).
    `BENCH ipc_call_roundtrip`: ~12 µs per call (emulated real time).
    Size: handle tables are one word per handle and exist only for CE5's
    31 process slots (ADR-0016 said 63 processes; corrected: above slot 32
    is the shared-file area); test programs copied into a frame instead of
    a page-aligned page in the image; kernel load budget 38 -> 46 KiB,
    approved by the owner.
    - **Iterate:** CE's thread-migrating calls, with the server seeing
      only its arguments (never the caller's other registers), and a
      crashing service returning an error to its caller.
    - **Modernize:** user code never runs from a kernel-image page (copied
      into its own frame, W^X), a server can't touch its caller's memory
      (domains), and the tables are fixed, zero-initialised and half the
      size of a naive layout.

## Eleventh tier (added 2026-10-12, same rules)

38. **Calls carried on the caller's kernel stack** (ADR-0016): **done
    (emulator).** A migrating call now runs the server as a nested
    activation of the calling thread (`user::call_user`): the caller's
    registers wait in its own kernel stack frame, and the server's
    `SYS_RETURN` or fault resumes it (`user::resume_call`), dropping
    whatever the server left on the stack. The kernel keeps one word per
    thread (its innermost continuation) instead of a table of saved
    registers (at 128 threads and 4 levels that table would have been
    34 KiB). Each call depth gets its own server stack page, demand paged.
    New checks: a server calling itself 4 levels deep (`IPC nested OK`)
    and the fifth level refused with the error passed up unchanged
    (`IPC too deep OK`); the emulator test requires both.
    - **Iterate:** CE's thread migration, with nesting bounded and checked
      and kernel stack use bounded by it.
    - **Modernize:** no per-call kernel table to exhaust or leak through;
      a server's crash, return or refusal all resume the caller the same
      way, and the server never sees the caller's registers.

39. **The scheduler in the kernel image** (ADR-0010, ADR-0016): **done
    (emulator).** `rp-kernel` runs `kernel/sched`: the boot thread, an idle
    thread (tickless) and two client threads in slot 5, both calling the
    server in slot 6 1,011 times at once on a ~150 us quantum. Every system
    call is a scheduling point and interrupts from user mode preempt, so
    threads are switched inside calls (`IPC preempted in calls OK`, required
    by the emulator test); each switch restores the slot the thread runs in
    (the server's while in a call). User stacks per thread and call depth,
    demand paged. Every earlier check now has to pass for both clients.
    Memory: thread contexts moved onto each thread's own kernel stack (one
    word per thread, not ten: -4.6 KB, also in `rp-threads`); the caller's
    slot waits in its kernel stack frame, so the IPC crate no longer keeps
    caller frames (-512 B); ports without `Option` (-256 B); `install`
    refuses closed ports. Thumb for the start-up code was tried and grew
    the image (outlined helpers): kept ARM. Kernel budgets 46 -> 55 KiB
    code, 112 -> 120 KiB RAM, approved by the owner.
    - **Iterate:** CE's priority scheduler and thread migration in one
      kernel, with the switch state where the thread already is (its
      stack) instead of in tables sized for the worst case.
    - **Modernize:** preemption tested inside cross-process calls, no
      shared server stack between threads, nothing per-thread that a
      hostile process can grow.

40. **A process from a PE file** (ADR-0009): **done (emulator).** At
    start the kernel packages the test programs as a PE image (`.text`,
    `.rdata`, `.data`, `.reloc`; `kernel/pe`'s new no_std writer, which a
    host test checks byte for byte against the reference builder) in a RAM
    "file". No ARM PE toolchain is installed, so the file is built on the
    device, but every loading step is the real one. Both processes map it at
    0x100000, not its preferred 0x10000, each page with its section's
    rights. On a page's first fault the page cache loads it into a new
    frame (`Image::load_page`: fill, then relocate, including values
    straddling page edges), once, and both processes share it. The client
    follows two relocated addresses (a code literal to `.data`, `.data`'s
    pointer to `.rdata`) to a magic word. Checks: `PE image relocated` and
    `PE pages loaded on demand` (exactly 3 of 5 pages: never the headers or
    `.reloc`), required by the emulator test. W^X: an image with any
    writable+executable page is refused (`Image::writable_code`).
    `rp-kernel-pe` built for size. Kernel load budget 55 -> 65 KiB under
    the owner's new standing rule.
    - **Iterate:** CE's loader model (one image, mapped per process at the
      same address), with pages loaded only when touched and relocated per
      page, so an image's untouched parts cost nothing.
    - **Modernize:** W^X enforced at load, per-section rights, no kernel
      page ever user-visible, and every loader input bounded and treated
      as hostile.

41. **Reclaim** (ADR-0005): **done (emulator).** `Vm::reclaim`: a clock
    hand sweeps a process's pages. A resident page is *aged*: its cache
    lines are written back and its entry becomes a stored one recording
    the frame and whether it was writable, so the next touch is a cheap
    minor fault that maps it again (`Outcome::Referenced`). That emulates
    the accessed bit ARMv5 lacks, with no extra memory. A page still aged
    when the hand returns is evicted: frames the process owns (anonymous
    memory, private copies) are compressed; page-cache pages (code,
    constants) are let go and reload on demand; device windows are never
    touched. Aged entries use a size class the compressed store never has,
    so they can't be confused with compressed handles; `unmap` frees them
    like mapped pages; a page made read-only while aged comes back
    read-only. 5 host tests; live: 16 pages, one sweep ages all, 8 are
    touched (8 minor faults), the next sweep re-ages those 8 and compresses
    the other 8, and an evicted page comes back intact (`VM reclaim OK`,
    required by the emulator test). Fits the current budget.
    - **Iterate:** CE had no page reclaim beyond discarding code; this keeps
      the working set and squeezes only what wasn't touched for a sweep.
    - **Modernize:** second-chance clock with emulated reference bits (as
      modern kernels do on MMUs without them), compressed instead of
      swapped to slow flash, so no flash wear and no data leaves RAM.

43. **A whole-OS memory budget** (owner's question, 2026-10-12): **done.**
    `docs/memory-budget.md`: the 64 MiB split top-down, the OS's eight
    parts at most 12 MiB together, apps at least 52 MiB (what Windows
    Mobile leaves on the owner's X51v). `xtask cross` (so CI) checks it on
    every build (`check_allotments`): parts plus the apps' floor fit the
    device, each OS image counts in full against one part, a part's images
    fit its share; a unit test covers each failure. The kernel image's
    budgets use ~185 KiB of its 1 MiB part. No emulator test: it is a build
    check, and the build fails without it.
    - **Iterate:** WM never published a budget; this one is checked.
    - **Modernize:** the apps' floor is fixed and parts trade only among
      themselves, so OS growth has to be paid for inside the OS.

42. **Service manager and supervised hosts** (ADR-0016 step 3): **done
    (emulator).** Servers are entered with the caller's slot in `r0`, put
    there by the kernel. `SYS_GRANT` lets a server hand a narrower copy of
    a handle to another process. A host (slot 7) is supervised by a service
    manager (slot 8); both run from the shared PE image. Client 0 opens the
    host through the manager, calls it (7), makes it fault (`ERR_FAULT`); the
    kernel marks it *restarting* (new calls fail at once, calls already
    inside return `ERR_RESTARTED`, never the broken instance's answers) and,
    when the last thread leaves, drops and remaps its whole address space
    (code shared, data fresh: checked), bumps the port's generation and
    renews the manager's handle. The old handle is `Stale`; the client asks
    the manager again and the new instance answers (7). Checks `IPC service
    manager OK`, `IPC supervised restart OK`, required by the emulator test;
    a host test for the supervisor's renewal and `Rights::from_bits`. The
    emulator caught a race in my first test program (two threads sharing a
    data word): fixed by keeping per-thread state in registers. Kernel load
    budget 65 -> 76 KiB under the standing rule. Still to come (items 44,
    46): real drivers moved into per-class hosts, and processes created and
    ended by the manager.
    - **Iterate:** CE's `device.exe` hosted every driver in one process,
      where one crash took them all down; here a host is restarted alone
      and its clients reconnect.
    - **Modernize:** capability handles from a manager instead of global
      names, unforgeable caller identity, crash results never leak out of a
      faulted instance, and stale handles fail instead of reaching a new
      instance.

44. **Processes created and ended at run time**: **done (emulator).** A
    process table of 5 reusable records (memory follows the processes
    running, not the 31 slots); lookups never fall back to another
    process's space. `SYS_SPAWN` (the service manager only; the entry must
    lie inside the image's code): a free slot and record, the image mapped
    (code shared, the rest fresh), a handle to the manager only (always
    handle 0), a first thread. `SYS_EXIT`: ports closed, handles gone, the
    whole address space unmapped, domain and record freed, thread ended.
    `SYS_GRANT` refuses slots with no process (closes the open note in
    security.md). Test: the manager starts a worker twice; each sees zeroed
    memory (the second runs in the first one's slot and record and must not
    see its mark) and is refused a handle it was never given; after each
    exit every frame is back and the slot holds no handles or ports
    (`IPC spawn and exit OK`). Host tests: `reset_process`, `handle_count`,
    `ports_of`. Kernel load budget 76 -> 84 KiB under the standing rule.
    **Found on the way, a scheduler bug:** the emulator's preemption test
    failed once in a while. A quantum whose timer fired while the thread
    was inside a system call was lost (the kernel isn't preemptible, and
    nothing re-armed the timer), so a thread could keep the CPU for good;
    a hostile program could have timed its calls to do it on purpose. Fix:
    an interrupt in kernel mode leaves a preemption owed, paid on the way
    back to user mode from the system call or fault (the kernel image's
    per-call deadline check, a workaround, is gone). New check `USER
    preempted after a system call` (three quanta in the kernel, then a
    user spin with no calls; the other thread must run first); with the
    fix disabled it fails.
    - **Iterate:** CE's process model, with records reused instead of a
      fixed one per slot, and exits that provably leave nothing behind.
    - **Modernize:** only the manager may create processes, a new process
      starts with one capability, exit cleanup checked frame by frame, and
      "need-resched on return to user" as modern kernels do.

47. **ADR: kernel object memory** (for the owner's vote): **drafted:
    ADR-0019 accepted by the owner** (regions in per-process pages from
    the frame allocator, with a quota); built as item 48.
    Original scope: process records
    hold room for 64 regions each (1.6 KB), too few for some Windows Mobile
    apps (each memory reservation is a region) and wasted for small
    processes. Options: a shared region pool, regions in per-process pages
    from the frame allocator, or a small kernel heap; pluses and minuses.

45. **ADR draft: optional packs on the card** (owner's idea, 2026-10-12):
    **ADR-0018 accepted by the owner** (built in Phases 8-9). Recommends one
    complete base system in flash plus signed, verified packs that may live
    on the card, rather than two installs; notes the real limit is RAM,
    which files on the card don't save.
    Original scope:
    the base system always complete in flash; optional packs (browser,
    codecs, fonts, language packs, extra game compatibility) on the card or
    in flash; nothing needed to boot or recover lives only on the card;
    packs hashed and signed and refused if changed; programs from a pack
    closed cleanly before its card goes. For the owner's vote, with pluses,
    minuses and alternatives.

46. **A real driver in a host**: the display+input host process running
    the backlight (PWM) driver through a granted MMIO window and its
    interrupt as an event, called by a client through a port; a crash in
    the host restarts it without touching the kernel or other hosts.
    **Done (emulator), 2026-10-13; kernel budget waiting for the owner.**
    Owner's choices: compiled Rust (not assembly); sizes shown before any
    budget raise. `services/display-host` (rp-display-host) runs the real
    `rp-drv-pxa-pwm` driver in a user process: 1.2 KB of code, a 2 KB PE
    file built by `cargo xtask` (`userimg.rs`, ELF to PE) and embedded in
    the test kernel. PWM0's page is mapped into it uncached by the board's
    grant; the power button's interrupt (GPIO0, not the spare timer: the
    timer's block is the scheduler's) is granted to it alone
    (`SYS_IRQ_WAIT`). The test client sets 30 %, makes the host fault in a
    call, gets a new handle from the manager and reads 30 % back; a test
    thread makes the host's own code fault: two restarts, the host's
    blocked thread re-entering each time, the level kept in the hardware
    throughout. Then the emulator presses the power button twice: light
    off, light back on. Checks `IPC driver host served`, `IPC driver host
    restarted`, the button steps; three emulator mutants; host tests for
    the packer and the press logic. Found on the way: a press and release
    faster than the host could read the pin was lost (both edges arrive
    while the line is masked): the host now treats "up after up" as a tap
    (`button_press`). `cargo xtask emu-kernel` runs the kernel's check
    alone. Kernel image 78,572 -> 88,260 B (2 KB host image, ~5.3 KB
    mechanism, ~2.3 KB the test's checks), over the 82 KiB budget: **owner
    to approve**.
    - **Iterate:** CE's driver model (a driver in a host process, its
      interrupt as an event, `InterruptDone`), with one host per class.
    - **Modernize:** grants instead of `MmMapIoSpace` for anyone; a host
      restart that doesn't touch the kernel, other hosts or clients' other
      work; driver state kept in the device, so a restart is invisible.

48. **Process records per ADR-0019**: **done (emulator).** `AddressSpace`
    regions now live in `RegionList` (`kernel/mm/src/regions.rs`): 20-byte
    entries encoded by hand in 4 KiB pages from the frame allocator (no
    unsafe), taken as the list grows and given back as it shrinks, a quota
    of 8 pages (1,632 regions) per process. Records shrank from 1.6 KB to
    ~60 B; `rp-kernel` load 82,636 -> 76,672 B before the new check, and
    its budget ratchets 84 -> 82 KiB. Emulator check `VM many regions`
    (300 regions in one process, touched, unmapped, every frame back:
    fails at region 65 without it); host tests: a page-level model under
    30,000 random map/carve/protect operations with memory running out at
    list-page boundaries (mutation-checked: dropping either reservation
    fails it), 1,000 regions, wiped pages.
    - 🔁 Iterate: same API shape (`map`, `find`, `carve`, `protect`), now
      given the page source; `Vm` supplies its own, so most callers
      changed by one argument. All 82 earlier tests kept.
    - ✨ Modernize / better: kernel memory charged to the process with a
      quota; lookups and overlap checks by binary search (were linear
      scans); `protect` and `unmap` split in place, one piece at a time,
      instead of copying the whole region table onto a 4 KiB kernel stack;
      the reclaim hand finds its next page by binary search instead of
      rescanning every region per page; failures are atomic (room reserved
      before any change); freed list pages are wiped (they held another
      process's memory layout); exit hands list pages back.

## Twelfth tier: the design brief (owner, 2026-10-12; docs/design-brief.md)

49. **Pinned memory**: **done (emulator).** `Prot::pinned()`: a pinned
    region is never aged, compressed or evicted; `Vm::populate` loads its
    pages up front (private copies made now), so later touches never fault.
    Each space has a pin limit (64 KiB for apps; the kernel can raise it for
    trusted services such as the compositor), checked by `map` and
    `protect`; changing a pinned region's permissions keeps the pin. 2 host
    tests; live: 4 pinned pages loaded up front survive three full reclaim
    sweeps of the process, contents intact, with zero faults on touch
    (`VM pinned OK`, required by the emulator test). Fits the budget.
    - **Iterate:** CE had `LockPages` per call; here pinning is a property
      of the region, so the guarantee can't be forgotten halfway.
    - **Modernize:** like `mlock` with a per-process limit, so pinning
      can't be used to starve the device; the input path stays
      deterministic while everything else compresses.

50. **Boot time as a budget**: **done (emulator; device number next).**
    The entry macro records when each image starts (`rt::since_start`); the
    loader prints `BENCH boot_loader_ready` (entry to its waiting loop) and
    the kernel `BENCH boot_kernel_tables` (entry to its own page tables);
    `cargo xtask emu` fails if either is missing or over budget (loose
    budgets: emulator time is real time). The first measurement found the
    loader took **742 ms**: a bring-up beacon filled all 16 MiB of 2700G
    memory at four steps (now only the first MiB, where a scanned-out plane
    lies), and everything ran in turn after a 500 ms display-settle wait
    kept as insurance against an unexplained device hang. Now the GPIO, card
    set-up, DMA self-test and USB detach run inside that wait: **504 ms**,
    of which 500 is the settle. Next on the device: the real number, and
    whether the settle can go (find the hang's cause). Audio and CF already
    start on first use.
    - **Iterate:** boot time measured and enforced, not guessed; the first
      measurement paid for itself.
    - **Modernize:** start-up work overlapped instead of serialised; the
      remaining wait is named, explained and on the device list.

51. **Trace ring**: **done (emulator), step 1.** `rp-kernel-trace`: a fixed
    ring of 16-byte records (time, kind, two arguments), overwrite-oldest,
    all zero when empty; each record carries its sequence number written
    last, so a reader drops torn or garbled records instead of misreading
    them and knows exactly how many were lost; decoding works on a copy of
    the words (for the PC). 4 host tests. The kernel image records thread
    switches, page faults, calls and returns, restarts, process start and
    exit (2 KiB ring); after the run it checks the ring wrapped many times
    and kept the newest 128 intact and in order (`TRACE ring OK`, required
    by the emulator test) and measures a record (`BENCH trace_event`).
    Step 2 (item 54): the ring kept across a return to the loader and read
    over USB with `cargo xtask trace`.
    - **Iterate:** CE's CeLog needed a desktop tool connected while it ran;
      this is always on, bounded, and readable after a crash.
    - **Modernize:** self-validating records (no locks, no torn reads),
      like the flight recorders modern kernels keep.

54. **Trace over USB**: the ring placed where it survives the return to the
    loader (as the run log does), and `cargo xtask trace` fetching and
    printing it as a timeline.
    **Done (emulator), 2026-10-13.** `rp-kernel-trace::Persistent`: the
    same self-checking records in memory the caller hands over (a header,
    then as many records as fit). The trace region is 4 KiB right after the
    run log (`0xA8510000`, 255 records); the loader starts a fresh ring with
    each program and answers `trace` with the ring as it lies (one `RPD`
    line per four words); the program carries the ring on
    (`Persistent::resume`: the loader's if it started the run, else a new
    one). `cargo xtask trace` decodes it with the kernel's own decoder (torn
    records dropped, never misread) and prints a timeline in microseconds.
    `rp-threads` records its switches and start/end marks; `rp-kernel` moved
    to the same ring (128 -> 255 events kept, 2 KiB less zeroed RAM).
    Emulator check `trace` (`cargo xtask emu-trace`): rp-threads under the
    loader, then `trace`: 2,082 events, 254 switches kept, both marks, in
    order; mutant (no switches recorded) killed; host tests for the ring
    handed over, garbled dumps and the timeline. Not yet on the device: the
    ring after a hang and watchdog reset (SDRAM is kept through the reset,
    as the run log is), which is what makes it a post-mortem tool.
    - **Iterate:** CE's CeLog idea (always-on kernel events, read after the
      fact) at a fixed, tiny cost, without a separate collector.
    - **Modernize:** records that check themselves, so a crash mid-write
      can't produce a wrong timeline; one decoder for the device and the
      PC.

55. **Soft watchdog** (owner's device kept landing on Dell's restore
    prompt): **done (emulator); card-size tests split out as item 57.** The
    loader arms OS timer channel 4 (1 ms ticks) before starting a program;
    its settings survive the hand-off, so every image built on the runtime
    is covered with nothing to do. Past the limit, the runtime's interrupt
    handler raises `Fault::RunLimit` through the program's own fault
    handler, which returns to the loader with a crash report; the loader's
    crash screen says "The program ran past its time limit". Programs check
    in with `rt::keep_alive()` (rp-reconnect does, its only user so far).
    The hardware watchdog is now a backstop 30 s later, for a program that
    hangs with interrupts off. `limit <s>` (1-30) shortens the next run's
    limit for tests. New `rp-hang` test image; emulator test: back after
    its 2 s limit with a report and no reset (4 s in all); the hardware
    backstop test still resets a hang with interrupts off. Kit README
    updated (press Mail, never Contacts).
    - **Iterate:** WM had no run limit at all; ours no longer turns a slow
      program into a factory-reset scare.
    - **Modernize:** layered watchdogs (soft first, hard as backstop) as
      modern kernels do, with no per-program code needed.

56. **Intermittent: a reply's second USB packet stalls**: **fixed
    (host-tested); it was the tick, not USB.** Seen twice in the emulator,
    both times on a tone reply cut at exactly 64 bytes (one packet). The
    second time (2026-10-12) the serial log showed the loader printing no
    `RPLOAD1 READY` (once a second, unconditionally) for 20 s after the
    reply: the whole loop had stopped, right after the one `udc.poll()`
    that sent the first packet, in `wait_tick`. Cause: the 1 ms tick
    re-armed itself by reading the counter, then writing match 1 a few
    instructions later; a handler running almost a whole period late
    (QEMU's host thread pausing at the end of a tone, "Resetting rate
    control") let the counter pass the deadline in between. Match 1 is an
    equality compare, so the next tick came only when the counter wrapped:
    22 minutes. On the device the same window exists (an FIQ between the
    read and the write); a loader with the watchdog armed would then reset
    onto Dell's factory prompt. Fix (`OsTimer::arm1`, class: every match-1
    write): read the counter after writing, as `alarm` already did; if the
    deadline was missed and hasn't fired, move it ahead (doubling) and try
    again. `start` too. Guard test: an OS timer whose counter moves at
    every register access, every lateness around the edge and across the
    wrap (fails on the old code: late 997 ticks of 1000, 1 tick per
    access); a mutant keeps it biting. Not reproducible on demand in the
    emulator (it needs the host to pause inside a window of a few
    instructions), so the soak test this item planned can't fail without
    the fix; the host test is the check. Watchdog feeds (30 s ahead) and
    the run limit (counter reset after the match is set) don't have the
    window.

57. **Tests at the owner's card size**: emulator runs with a card image as
    large as the owner's (4 GB and 32 GB, sparse files), for the loader's
    long card commands, so slow-at-full-size problems are caught before the
    device.
    **Done (emulator), 2026-10-13.** `rp_fat::format::write_sparse` emits
    only an image's non-zero parts, and `Image::ballast` fills a card with
    allocated, chained clusters whose data is never written; xtask writes
    them into a sparse file (on Windows marked sparse before it is sized,
    so NTFS never fills in zeros: a 32 GB image takes a few MB).
    `cargo xtask emu-fullsize` (also in `cargo xtask emu`): a 4 GB card
    (4 KiB clusters, ~1 million) and a 32 GB card (16 KiB clusters, ~2
    million), each 90% ballast, FSInfo's hints "unknown" so `put` must scan
    the FAT. Emulator times (wall clock): ready 1.0 s, `card check` 9.1 /
    18.2 s, `put` past the ballast 3.2 / 5.4 s, check again, the last 4096
    blocks scanned, the drive's capacity and last block over USB. Mutant
    (card check's cluster bitmap sized for small cards) killed: "no room"
    at 4 GB, invisible on the 8-16 MB test cards. Formatter host test for
    sparse output and ballast. **Not modellable:** the device's time for
    these (the emulator's card has no per-block latency, docs/hw/emulator.md);
    `card check` reads both FATs (8-16 MB at 32 GB), so on the device it
    may take minutes and holds up the USB loop while it runs: worth a
    device run with the 32 GB card, and a chunked check if it is too slow
    (item 62 (c)'s rule).

52. **Display fast paths**: **done (emulator), iWMMXt next.** `rp-gfx`
    (`drivers/gfx`): rectangles clipped with saturating arithmetic, fills
    through `fill32`'s burst stores, copies and overlapping scrolls, the
    glass tint as a per-column precomputed gradient, alpha icons, photo rows
    with a 4x4 ordered dither, and dirty rectangles that merge as they're
    added and are widened to whole words so every flush takes `memcpy`'s
    burst path, writing the framebuffer only. Blends use the RGB565
    "spread" trick: three channels in one multiply, bit-exact with the
    per-channel reference (every alpha, tested). Emulator benchmarks
    (instructions, so work done): full-screen blend 3.08 vs 7.99 ms
    (2.6x less work), fill 0.96 vs 9.3 ms before `fill32` (9.7x), a
    clock-sized flush 0.0025 ms against 0.13 ms for the whole screen.
    On-target check `CHECK gfx` (fast paths against the reference on the
    CPU itself, a flush that writes nothing outside its rectangle),
    required by `cargo xtask bench`, which `cargo xtask emu` now runs too
    (so a regression past the baseline fails the suite).
    - 🔁 Iterate: one module for every pixel path (the console and crash
      screen move onto it as they're touched), each against a reference.
    - ✨ Modernize / better: compose in RAM, write-only flushes, dirty
      rectangles, word-aligned bursts, dithered photo conversion; no unsafe
      (pixel byte views added to `rp-hal-mem`); hostile rectangles draw
      nothing. `Dirty::add` rewritten as a loop when it measured 1.9 KB.
    - Next: real-device numbers (the probe runs the same suite), then
      iWMMXt for the blend and tint where the device shows the CPU, not
      memory, is the limit.

53. **Testing rules written down**: **done.** docs/testing.md (every
    change, fix the class, flakes are bugs, mutants, model tests, fuzzing
    from the day a parser lands, benchmarks as tests, the emulator tracks
    the device). `cargo xtask mutants [--emu]` (`xtask/src/mutants.rs`):
    11 deliberate bugs on security and crash-safety rules (region-list
    reservations and wiping, pin limit, double free, IPC rights, LZ4
    bounds, constant-time compare, pixel blend, dirty-rectangle alignment,
    and in the emulator the soft run limit's FIQ routing), all killed.
    - 🔁 Iterate: the manual "break it and watch it fail" checks from items
      48, 52 and 59 are now a list that runs on demand.
    - ✨ Modernize / better: each check is first run on the real code and
      must pass with at least one test (the first run found a mutant
      "surviving" only because its filter matched no test name); files
      with uncommitted changes are refused; every file is restored even on
      error; images are rebuilt after an emulator mutant so nothing built
      from a mutant can reach `cargo xtask load`; a unit test catches a
      mutant whose text no longer matches the code.
    - Still to add: file-system crash-path mutants (copy-then-swap) and the
      scheduler's owed preemption.

58. **SD throughput is ours to win** (measured 2026-10-12 on the owner's
    new 32 GB card): big transfers run at ~1.4-1.9 MB/s on both cards, a
    fraction of what the 4-bit bus carries, while the new card's single
    commands are 4.4x faster than the old card's; so the driver is the
    limit. Next: DMA for writes (on the device), larger multi-block
    commands, the bus clock checked, measured before/after on the device.
    **Progress (2026-10-13), emulator-verified, device run next.** Bus clock
    checked: right (19.5 MHz, the controller's maximum); ~80% of each block
    is overhead (docs/hw/sd.md "Speed"). A `card speed` loader command was
    built, then moved out (owner: iterate, not imitate): the probe measures
    it instead, at no cost to the loader (`sd_read_256_blocks` and the DMA
    rows `*_dma`, checked by `sd_dma_matches`). The drive now overlaps card
    and USB (two half-buffers; the SD DMA's wait loop sends the previous
    half: `Storage::read_blocks_while`, `SdCard::read_many_while`, the
    controller's `dma_wait` hook) and reads ahead after a bulk read
    (`drive status`: `read ahead`), dropped on any write, medium change or
    reset. Host tests: streams exactly, read-ahead hit, write and medium
    change drop it (two mutants), failure behind the front reported after
    the good blocks; emulator: sequential reads hit, the 1 MiB read across
    the slow block still exact. Loader 101 -> 112 KiB (owner-approved;
    104,476 B). **Owner at the Axim:** `cargo xtask load rp-probe` (speed
    rows FIFO vs DMA, 64 vs 256 blocks), copy a big file off the drive
    before/after; then DMA writes (`cargo xtask card dma writes`) and
    ACMD23 pre-erase, measured the same way.

60. **Plane blending at scan-out**: **done, proven on the device**
    (2026-10-12, findings in docs/hw/2700g.md "Measured on the owner's
    X51v"). Owner chose ARGB4444 with ordered dithering for the interface
    (`blendtest::to_4444_dithered`), confirmed again up close (C, D, E at
    20 s each: "E looks just as good"). The format stays one compositor
    setting (ARGB4444 dithered or ARGB8888), so it can be switched (even
    offered to the user) if the dither ever shows. `rp-blend` takes
    `RP_BLEND_STAGES` / `RP_BLEND_SECONDS` at build time to show chosen
    stages for longer. **Done in the emulator (2026-10-12): the OS's
    drawing path for the interface plane.** Owner chose a full-colour RAM
    layer (ARGB8888, 1.2 MB) over 16-bit + alpha or drawing in ARGB4444, so
    the dither always starts from 8 bits. `rp_gfx::layer`: fills, glass as
    a per-column gradient, icons and glyph masks drawn "over" (straight
    alpha, exact against a reference; short paths with an exact
    divide-by-255 in two lanes, only soft edges on glass divide), and
    `layer::flush` converting dirty rectangles to ARGB4444 with the
    approved dither (moved into rp-gfx; `blendtest` re-exports it) or
    copying ARGB8888 (`PlaneFormat`, the one setting). The dither is fixed
    to screen positions: a small flush writes exactly what a full one
    would (tested). `rp-blend` stage F draws E's scene through it in the
    scratch region (now 4 MiB) with a full clear flush then dirty flushes:
    byte for byte E's picture (host test from a garbage plane), all six
    stages pixel-exact in `cargo xtask emu-blend`, per-stage draw times in
    the run log for the device. `CHECK layer` on target; benchmarks
    (emulator instructions): dithered full-screen flush 9.5 ms vs 16.6 ms
    per-pixel reference, clock-sized 0.080 ms, glass 0.22 ms. Next: the
    device run (E against F by eye, F's draw time from the run log), a real
    wallpaper as 4:2:0 from a JPEG, iWMMXt for the dither (4 channels in
    one register), and a stage with no `VOVRCLK`. Earlier status:
    Steps 1-4 below are built: emulator rp-v0.10.0, `Display::adopt` in the
    loader and every image, `rp-blend`, `cargo xtask emu-blend` (three
    stages pixel-exact; screens in docs/img/emulator-plane-blend.png), two
    mutants. (Owner said go, 2026-10-12; replaces
    the "2D blitter fill" milestone, see docs/hw/2700g.md "2D drawing and
    plane composition"). The wallpaper lives in the 2700G's **video plane**
    (YUV), the interface in the **graphics plane** with per-pixel alpha,
    and the display controller mixes them while it scans out: no CPU and
    no extra memory traffic per frame for the glass look.
    Register facts (from mbxfb, B): each plane has a blend mode (none,
    inverse, global alpha, per-pixel alpha) and a layer slot (0 bottom,
    1 middle, 2 top); WM leaves graphics in slot 0, video in slot 1. Video
    needs `VOVRCLK` bit 0; planar chroma bases are in 8-byte units; write
    the chroma bases before `VSCTRL` (mbxfb: the chip hangs otherwise).
    Steps, each proven before the next:
    1. **Emulator model** (fork, `intel-2700g.c`): video plane (packed 4:2:2
       and planar 4:2:0, BT.601 colour), layer slots, the four blend modes,
       `BGCOLOR` behind everything, ARGB4444. Tag, bump `QEMU_RELEASE`.
    2. **Driver** (`mbx-display`): `VideoPlane`, `Layers` (slots + modes),
       ARGB4444; and **one `Display::adopt`** that every image and the
       loader use to take the screen, which also switches the video plane
       and cursor off. Found while planning: the 7 copies of "keep WM's
       plane" would leave a test's video plane covering the loader's
       screen after it returns (fix the class, not the instance).
    3. **`rp-blend` test image**: a generated wallpaper (bars, grey steps,
       a smooth sky, sharp edges) under a translucent glass panel with
       opaque text, in three labelled stages so a photo explains itself:
       A 4:2:2 + ARGB4444, B 4:2:0 + ARGB4444, C 4:2:0 + ARGB8888 (to judge
       ARGB4444's banding by eye). Writes the old register values and
       `VOVRCLK` to the run log and puts every register back before it
       returns. Uses 2700G memory above 1 MiB only (the loader's screen at
       offset 0 is never touched).
    4. **Emulator pixel check** (`cargo xtask emu`): each stage's screendump
       against a Rust reference of the documented blend maths (exact), plus
       sample points against the intended colours (catches both sides
       agreeing on something wrong, e.g. U and V swapped).
    5. **The Axim**: `cargo xtask load rp-blend`; the owner photographs the
       three stages. Questions the photos answer: is alpha 15 opaque (or
       is it the inverse)? limited or full-range YUV? does 4:2:0 show? is
       ARGB4444 good enough for glass? Findings into 2700g.md, the model
       corrected if the device disagrees.
    - Three questions. *Clever or copying?* WM never used these planes for
      its desktop (its GDI drew everything on the CPU); this moves the
      whole wallpaper-under-glass cost into the display hardware.
      *Size?* 4:2:0 wallpaper is 450 KiB vs 600 KiB as RGB565 (-25%), and
      it is what a JPEG decodes to anyway, so photos skip the colour
      conversion; ARGB4444 UI is 600 KiB, the same as today's plane; the
      test scene is generated, not stored (the image stays small).
      *Modern?* hardware composition with per-pixel alpha is how phone
      compositors (Android's hardware composer, iOS) do it today.

61. **The Windows build of the emulator can't receive images** (found
    2026-10-12 while releasing rp-v0.10.0): with the downloaded native
    Windows QEMU (`RETROPANE_WSL_DISTRO=none-for-test`), `emu-picture` and
    `emu-blend` time out with the loader printing `RPLOAD1 READY` again and
    again, so the frame sent on stdin never arrives. rp-v0.9.9 fails the
    same way, so it is older than this release. The WSL build and CI
    (Linux) are fine. Likely QEMU's Windows stdio chardev not reading a
    pipe; try `-serial pipe:` or a TCP serial in `xtask/src/emu.rs` for
    native Windows, then make the release check use it again.
    **Done (2026-10-13).** Not a Windows workaround: the harness's serial is
    a local TCP socket on every platform (`-serial tcp:...,server=on,
    wait=on`: QEMU holds the guest until the harness connects, so no early
    output is lost), one path for input and output, tested on Linux, WSL
    and native Windows. `RETROPANE_QEMU=release` runs any check on the
    published build for this platform; the release steps in
    docs/hw/emulator.md now require a full run with it. On the downloaded
    Windows build every serial check now passes (loader, watchdog, run
    limit, picture, blend, crash path), and the CF, kernel and full-size
    card checks too (native QEMU runs about twice as fast as the WSL
    build). That run found the next Windows-only fault, item 66. New
    shortcut `cargo xtask emu-usb`.

66. **Windows build of the emulator: no USB after an image returns**
    (found 2026-10-13 by item 61's release check). With the downloaded
    native Windows QEMU, an image loaded over USB runs and returns, the
    loader prints its run log on serial and `RPLOAD1 READY` again, but
    nothing more arrives on the USB socket, not even READY; reconnecting
    the socket doesn't help. So the emulated PC never re-enumerates the
    loader after it re-enables its UDC (`hw/misc/pxa27x-udc.c`: `H_ATTACH`
    on the UDE edge, then the script and the open requests). The first
    enumeration works; WSL and Linux CI are fine. Affects `emu-usb` and
    `emu-trace` (and the full suite stops there on Windows). Next: a
    Windows build of the fork with tracing around `UDCCR` writes, the
    attach timer and the open steps, compared with Linux.

62. **CF on the device: works first, breaks after a hand-over** (found
    2026-10-12, docs/hw/cf.md "Measured on the owner's X51v"). Fix the
    class, test first: (a) emulator: the CF card keeps its configured
    state across a loader hand-over, so CI fails like the device; (b)
    driver: reset the card (configuration register soft reset, then wait
    for ready) whenever a loader takes the socket, and re-check the CIS;
    (c) **no device wait may hold up the loader's USB loop** (a stuck CF
    stalled image transfers part-way; recovery was padding the frame so
    its CRC failed): bound each wait per loop pass or drop a dead card
    after one failed command, with a mutant that hangs a card; (d) only
    then the I: drive, read-only. Also found and fixed in the same
    session: `drive on` while the serial-only fallback layout is up now
    switches back to the serial + drive layout (it used to report "drive
    offered" with no drive possible). Diagnostics in the loader: `cpld`
    (registers as found), `cf` reports slot state, task file and sector
    0's ends.
    **Progress (2026-10-12 evening):** (b) and (c) done and unit-tested
    (hardware reset + READY wait before any access; a card that times out
    is dead after one timeout and the loader lets it go; failed slots are
    retried by the next `cf`); (c) **proven on the device** (a dying card
    no longer stalls the loader). But the device showed the real fault is
    different: the card works when taken, then within seconds everything
    reads 0 and READY drops to 0, and the reset doesn't bring it back;
    also on the first `cf` after Windows Mobile. Looks like the slot
    losing power (docs/hw/cf.md "Second session"). GPIO84 did not drop
    while GPIO36 was held, so GPIO36 = RESET is not confirmed. **Next:**
    fresh boot (owner at the Axim), `cf log` before and after `cf` to time
    READY's drop; then find what powers the slot. (a) waits until the
    mechanism is known, so the emulator models the real fault.
    **Solved (2026-10-12 night):** Dell's socket driver (analysed locally,
    facts in docs/hw/cf.md and cpld.md) powers the slot with CPLD `0x16`
    bit 1 + `0x10` bit 12 and resets the card with CPLD `0x18` bit 1;
    GPIO84 = slot powered; GPIO36 isn't CF at all. The loader now
    power-cycles the slot before `cf`: on the device the dead slot came
    back, Windows mounted the card and read 73 MB with no errors. First
    try turned the backlight off (CPLD `0x18` bit 2, cleared by our
    whole-register write: the CPLD doesn't read back); the loader now sets
    the backlight enable before its first write. Installed on the SD card.
    **Left:** (a) the emulator model (docs/hw/emulator.md rows), then a
    fork release; power the slot down on removal. CF icon done (owner
    renamed both cards "X51V SD" and "X51V CF" from Windows; icons show).
    **(a) done (2026-10-12, rp-v0.11.0):** the fork's CPLD is write-only
    latches with outputs (CF power, CF reset, backlight), reads give the
    measured status values; the CF slot starts unpowered and answers only
    through the power sequence; `0x18` bit 2 clear blanks the screen. The
    CF check (`cargo xtask emu-cf`) needs `cf soft` to fail and the
    backlight to stay on; two new emulator mutants (power-on removed,
    backlight enable removed) are killed. **Power-down done (2026-10-13,
    rp-v0.11.1):** the loader switches the slot off whenever no card is
    attached (removed, dead, failed attach); the fork's `cfslot` switch
    pulls the card out in tests, and the CF check needs GPIO84 to stay low
    on re-insert, then `cf` to take it again (mutant killed). Also fixed:
    `cf log`'s reply ended in a bare LF (a shell heredoc had turned its
    `\r\n` into a real newline). Left: try power-down on the device.

63. **Drive icons for the cards**: owner wants the "Memory cards" icons
    from puntodepartida.com (no licence stated: all rights reserved). For
    now: the owner's own cards only, files in `local/icons/` (never
    committed), shown via `autorun.inf` (`icon=`, `label=`). Owner is
    emailing the author for permission to ship them; if no answer, the
    repo uses the Crystal SVG pair (Everaldo Coelho, LGPL; Commons
    `Crystal128-sd-mmc-unmount.svg`, `Crystal128-compact-flash-unmount.svg`)
    with the licence notice next to them. My own drawn drafts were rejected
    (deleted). **SD card done (2026-10-12):** `card-sd.ico` (from the
    256 px PNG, 16-256 px sizes) and `autorun.inf` (`icon=` only) put on the
    owner's SD card; owner: "looks great". Files in `local/icons/` (also
    `autorun-cf.inf` + `card-compact-flash.ico` for the CF card, once item
    62 lets it attach).
    **Done (2026-10-14): permission granted.** The author (Juanje) replied
    "feel free to use them", with credit; selling isn't mentioned (the
    owner won't ask again). The owner put them in the base system, an
    exception to the packs-only rule. Icons and their record in
    `assets/cards/` (README.md: permission, credit, the gap, the Crystal
    fallback); `cargo xtask sdkit` builds `cards/sd` and `cards/cf`
    (icon + autorun.inf), `cargo xtask install` puts the SD pair on the
    card. File Explorer uses them once it exists.

64. **Remote reset back to Windows Mobile** (owner, 2026-10-12: wanted for
    more than the CF card). Today only the stylus reset (a hardware reset)
    boots Windows Mobile; the only reset software can cause is the
    watchdog's, and Dell's start-up code stops at its factory prompt after
    one (owner presses Mail). Bounded RE on the ROM dumps (`local/rom-*`):
    find where Dell's boot code chooses "prompt or boot Windows Mobile" and
    whether anything we control before a watchdog reset reaches it (RAM
    kept by self-refresh, `RCSR`/`PSPR`, a flash flag, the CPLD). If
    something does: `cargo xtask reboot-wm`, tested on the device. If
    nothing does: write that up as a dead end and stop. Clean-room rules
    apply (facts only into docs/hw).
    **Lead found (2026-10-13), device test next:** the factory prompt isn't
    in the `.nb0` images (it's Dell's bootloader in the flash boot block),
    but Windows Mobile's own reboot (`IOCTL_HAL_REBOOT` in the kernel's OAL)
    resets the board through GPIO99, not the watchdog: GPIO99 output,
    `PCFR` GPR_EN, GPIO15 low, CPLD `0x08` bit 1, GPIO99 high
    (docs/hw/README.md "Software reset"). `cargo xtask reboot-wm` does the
    same; emulator check covers the "no reset" path. **Owner at the Axim:**
    run it; Windows Mobile without the prompt = done (then model GPIO99 in
    the fork); prompt = dead end, written up. Also found: WM's CPLD shadow
    slots live in RAM from `0xA8001320` (cpld.md).

65. **Windows keeps a stale view after the loader writes the card**
    (found 2026-10-12): after `cargo xtask put`, Explorer and `dir` still
    showed the old root; `drive off` + `drive on` straight after didn't
    help either, only off, ~8 s, on. The loader's "medium changed" isn't
    enough for Windows to drop its cache. Class fix: after any loader
    write (`put`, `rm`, `install`), take the medium away long enough for
    Windows to notice, then give it back, with an emulator check that the
    PC's view matches the card afterwards.
    **Done in the emulator (2026-10-13), not yet seen on the device:** the
    fix is in `rp-usb-msc` itself, so every caller gets it: `medium_changed`
    now takes the medium away ("no medium") until the host has been told so
    twice and 2 s have passed (10 s ceiling with no host asking), then
    reports "medium changed" once. Unit tests for both paths; the drive
    check polls like Windows and needs away, changed, ready after `drive
    ro`, `drive rw` and the device's `put`, then the new file. **Device
    check:** `cargo xtask put` with Explorer open on the card: the drive
    should blink away for ~2 s and come back showing the new file. If
    Windows still shows the old root, raise `AWAY_MIN_MS` / `AWAY_SEEN`.

59. **No emulator-only image on the device**: **done (emulator).** Every
    image's budget now says what it does under the loader (`OnDevice`:
    returns, the loader, never returns), so a new image can't be added
    without saying; `cargo xtask load` refuses never-returning ones
    (`rp-bench`, `rp-kernel`, `rp-hello`, `rp-usb`, `rp-hang`) unless
    `--force`, naming `rp-probe` for device benchmarks. The soft run limit
    is routed to **FIQ**: it gets its own stack and vector, FIQ is unmasked
    once an image's vectors are in place, and the HAL no longer masks FIQ
    anywhere (IRQ entry, thread start, system calls, user mode), so a
    program spinning with interrupts off comes back with a report too.
    `rp-hang` now spins with IRQs off; its test passes, and fails with the
    limit routed to IRQ (checked). Unit test for FIQ routing. Kit README
    updated.
    - **Iterate:** the tool refuses the mistake instead of relying on
      memory; FIQ, which CE left to drivers, is the system's watchdog line.
    - **Modernize:** layered, uncircumventable-by-mistake watchdogs; only
      code that never starts our runtime can still reach the hardware reset.
    Original scope: `cargo xtask load` refuses
    images that never return to the loader (`rp-bench`, `rp-kernel`,
    `rp-hello`, `rp-usb`) unless forced; every image classified, a test
    that a new image must be. (Found 2026-10-12: rp-bench, which spins
    forever when done, was sent to the Axim and the hardware backstop reset
    it into Dell's prompt.) With the soft watchdog moved to FIQ, so a
    program spinning with interrupts off comes back too.

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

## Thirteenth tier: the imitation audit (owner, 2026-10-13; docs/research/imitation-audit.md)

The owner's hard rule (imitation is the last resort) applied to everything
built so far. Ranked in the audit; the "know the silicon" track (69, 80)
comes before the UI and GDI designs, which are written GPU-first.

67. **Own the clocks and power states.** Set 624 MHz turbo at start
    instead of inheriting WM's (seen at 208/520 and 624); drop to a low run
    frequency when idle and back on work; deep idle with SDRAM
    self-refresh; voltage scaling once the power IC is identified (RE of
    `PowerStateManager.dll`, `battdrvr.dll`); cheap timestamps from the CP14
    cycle counter. Find whether the clock explains the 12 us thread switch
    (target 2 us). Device runs, each measured.
68. **Memory and bus timings.** Measure and set the fastest stable SDRAM
    (`MDCNFG`, `MDREFR`, the "alternate memory clock" bit) and
    static-memory timings (`MSC1`: the 2700G's bus, framebuffer writes at
    ~41 MB/s), with a watchdog-guarded step-and-verify test.
    **Investigated 2026-10-15, report only** (docs/research/memory-timing.md):
    SDRAM is at the PXA's clock limit with 1.2%-margin CL2 headroom (keep);
    the `MSC1` experiment and the owner's chip markings are the open steps.
69. **MBX core bring-up, GPU-first.** From Dell's 3D drivers, step by step
    on the device: power the core, clear the screen with it, then a
    triangle, a textured quad, blending; each measured against the CPU.
    The emulator's model grows with each step. The compositor/UI ADR and
    the GDI design wait for its findings.
70. **Display without the CPU in the way.** Dirty-rectangle flushes by DMA
    descriptor chains; tear-free double buffering inside the 2700G's 16 MB
    with plane switches at vsync; hardware scrolling by moving the plane's
    start address (the console stops reading 2700G memory); the cursor
    plane as a free small overlay.
71. **iWMMXt pixel paths:** blend, dither, tint, `memcpy`/`memset`.
72. **Own the display set-up** (PLL, timings, plane) instead of adopting
    WM's; find the cause of the 2700G's 500 ms "settle".
73. **Interrupts and DMA in every OS driver** (DESIGN §11's own rule):
    completion interrupts, the CPU asleep during transfers, memory-to-
    memory DMA for page pre-zeroing; an emulator check that the CPU idles
    during a transfer. The development loader may keep polling.
74. **USB double-buffered endpoints and DMA**, so packets go back to back
    (towards full speed's ~1.1 MB/s).
75. **CF: timings from the card's own PIO mode** (IDENTIFY), READ/WRITE
    MULTIPLE, the card's interrupt, DMA from the socket window.
76. **Touch and light feel:** oversampled, filtered touch with pressure, a
    calibration screen, SSP DMA; backlight low steps by temporal
    dithering of the duty.
77. **CPLD shadow from WM's** (its shadow copies in RAM from `0xA8001320`)
    at hand-over, and the remaining slots mapped.
78. **ADR-0006's measured wins** (page tables and hot data in SRAM: 5.2x
    cheaper TLB misses measured; DMA pre-zeroing; large pages), and the
    CE5 layout limits re-examined with data (31 processes: CE's shared-file
    area, not FCSE's 64 PIDs; slot 1's XIP purpose doesn't apply here).
79. **Fix the documents that disagree with the code**: **done
    2026-10-13** (moved ahead of 69's later steps on review): the ten in
    the audit, plus docs/hw/2700g.md's "WM draws on the CPU" (measured
    wrong by `rp-wmgpu`) and the brief's example.
80. **CPU deep-dive:** the CP14 performance monitor first (to measure
    everything else), then cache locking for the IRQ path, scheduler and
    system calls, the mini-data cache for streaming data, SRAM for hot
    code and GPU command buffers, `PLD` in copies, write-buffer settings
    per region; each measured on the device before and after.

## Fourteenth tier: the shell (ADR-0021, owner 2026-10-13)

81. **Themes, a modern equivalent of Windows Mobile's** (owner, 2026-10-13;
    after the Today screen and the wallpaper picker). A RetroPane theme is
    a wallpaper, an accent colour, glass darkness and light/dark, bundled;
    the ten-wallpaper pack can ship as themes. Import of WM `.tsk` themes
    (a cabinet: the Today background becomes the wallpaper, the highlight
    colour the accent), so the owner's 65 old themes work; a cabinet parser
    is outside input, so hostile-file tests and bounded lengths (security
    first). Old WM apps get their system colours from the theme once GWES
    exists. Imported art stays the user's: never shipped.

82. **No "USB device not recognized", ever** (owner, 2026-10-13: "it
    looks sloppy if users end up seeing this"). The X51v's D+ pull-up is a
    fixed resistor (docs/hw/udc.md): whenever nothing answers USB while
    the cable is in (a test image under the loader today; in the product,
    early boot, suspend, a crash), Windows reports a malfunctioning
    device. First the untested lead: force SE0 through `UP2OCR`'s output
    override so the PC sees a clean unplug, and release it to replug
    (a short device experiment, `rp-reconnect` style). If it works: the
    loader unplugs before handing over and replugs on return, and the OS
    unplugs before any quiet period. If it can't: the OS answers USB from
    early boot and through suspend. Done when no reachable state shows the
    pop-up, checked on the device in each. Meanwhile, debugging never
    queries the device while a test image runs.

83. **The CF card's drive flaky in Explorer** (owner, 2026-10-15: a bare
    "USB Drive (J:)" with no size, or the label with a broken icon that
    fixes itself minutes later; "very flaky" since the drive was built).
    **Fixed in the emulator, device check pending.** Cause: the loader
    attached a CF card that was in at start-up only on its second detect
    poll (~1 s into the loop, after the pull-up), so Windows enumerated
    LUN 1 as "no medium", the attach then stalled every USB request for
    0.5-3.5 s, and `medium_changed` kept the new card away 2-10 s more.
    Each image run (the RNG restart loop: every ~9 s) did it again.
    Class fixes: (1) the card is attached before the pull-up, as the SD
    card; (2) `rp-usb-msc` skips the away time when the host last saw the
    unit empty or never asked (it has nothing cached), so an inserted or
    re-attached card is offered at once with a unit attention; (3) a card
    that fails to attach or stops answering is retried once per
    insertion (then idle, never a loop). Guards: emu-cf needs LUN 1 ready
    at the PC's first question; a unit test for the away skip; two
    mutants (both killed). Not covered by the emulator: the one retry
    (the fork can't make a card fail once). **Device check:** with a CF
    card in, `cargo xtask load` any returning image a few times with
    Explorer open: J: should come back each time with its label, size and
    icon at once; `cargo xtask cf log` should show no `dead` events.
