# Reminders: work parked until the right moment

## Standing rules

The owner's two most important rules (2026-10-24). They never close; the
hooks print them first, at every session start and before every device run
or test-image build.

- **Iterate, don't imitate.** What Dell, Windows Mobile, Linux or any 2005 design does is a finding, never the answer or a ceiling. For every inherited value or behaviour: is it a hardware limit or a choice? If a choice, find the better ways (our own software, the device's parts working together), measure them, and beat the original; keep the old way only where a measurement says it wins. **Be clever:** we have today's technology and software, AI included, that Dell didn't have in 2005: use it to make everything as good, as fast and as efficient as it can possibly be. **Modernize** everything that can be modernized, while keeping the neo-retro look.
- **Nothing is impossible or a limit without proof.** Never tell the owner something can't be done, or call a setting a hardware limit, until every idea and every "did we do it wrong?" check is ruled out by a measurement. Divide 1 was called unworkable, and it now runs 2.17x Windows Mobile's speed at 108.33 MHz: giving up early leaves performance on the table.

## The reminders

**Why this exists (the owner, 2026-10-24):** plans were made and then
dropped because nothing brought them back at the right time (12 found on
2026-10-24: ROADMAP step 3's give-up audit table). Anything parked, whether
the owner raised it or a session found it, goes on this list **in the same
reply that parks it**, with a trigger saying when it must come back up.

**How it comes back up (automatic, `.claude/settings.json`):**
- **Every session start** prints the open reminders (`tools/remind/remind.py
  session`).
- **Before a device run or a test-image build** (`cargo xtask load`,
  `codegen-variant`) the reminders whose trigger is `device-run`, or names
  that image (`image:rp-gpubench`), are printed: the moment to fold them
  into the run.
- **`cargo xtask ci`** checks this table's format and prints the open count.

**The rules:**
- **Trigger** is one or more of, separated by `;`: `session` (every session
  start), `device-run` (any device run), `image:<name>` (a run or build of
  that image), `after:<what>` (when that lands; shown at session start),
  `date:YYYY-MM-DD` (shown from that day). Every trigger is also shown at
  session start, so nothing waits on its trigger alone.
- **Status** is `open`, or `done <commit or run>`, or `dropped <the owner's
  reason>` (only the owner drops one). Rows are never deleted.
- Every summary to the owner names the reminders that came due.
- The owner can add, edit or drop rows himself.

| ID | What | Raised | Trigger | Status |
|---|---|---|---|---|
| R1 | Give-up audit item 2: 2D and 3D at once and the 32-bit screen, retested with the screen and the 3D target inside the safe region (FT2's test), with `LMCFG` and `DMCTRL` (GC3, rebuilt). **Built 2026-10-24: run FT3** (`RP_GPUBENCH_FT3`: the 3D target the scanned-out screen at 0, three clocks) and GC3 now runs FT3 under each memory setting | Claude, 2026-10-23; FT2 analysis 2026-10-24 | image:rp-gpubench; device-run | open |
| R2 | Give-up audit item 4: render-to-texture, the render block's other words (GC3T, built 2026-10-23, never run) | Claude, 2026-10-23 | image:rp-gpubench; device-run | open |
| R3 | Give-up audit item 5: every `MSC1` CS3 timing combination (RDN, RRR, RDF), not just RDF | Claude, 2026-10-23 | image:rp-gpubench; image:rp-cs3bus; device-run | open |
| R4 | Give-up audit item 6: isolate zeroing by cache-line allocate (hung the X51v once), one row a step, the owner nearby | Claude, 2026-10-23 | image:rp-probe; device-run | open |
| R5 | Give-up audit item 7: the CPU's overclock past 624 MHz (C2), its own watched run with the self-test | Claude, 2026-10-23 | session | open |
| R6 | The internal flash's timing (`MSC0`, where RetroPane is installed): Dell's value read, then a watched step test. **Designed 2026-10-24** (memory-timing.md section 9: the read strobe only, the write timing kept, the chip's ID checked each step) | Claude, 2026-10-20 | session | open |
| R7 | CF socket timings (docs/research/storage-speed.md: ~2x possible): the owner's decision. **Brief written 2026-10-24** (storage-speed.md: A, each card at its own rated speed, recommended) | Claude, 2026-10-15 | session | open |
| R8 | The CPLD's bus timing (buttons, power): Dell's value read, then a step test. **2026-10-24: proposed for decline** (memory-timing.md section 9: write-only latches can't be verified safely; microseconds a day): the owner's call | Claude, 2026-10-20 | session | open |
| R9 | One checklist of every Dell, Windows Mobile and Linux value the code uses, each tagged `// inherited:` with its test. **First sweep 2026-10-24: docs/research/inherited-values.md** (100 values: 11 copied and untested, 9 measured or ruled, the rest hardware facts); next: the tags in the code | Claude, 2026-10-20 | session | open |
| R10 | Dell's GLES register writes traced (`tools/mmiotrace`, `rp-wmprobe`'s missing module) | Claude, 2026-10-23 | after:the combined run (R1-R4) | open |
| R11 | Windows Mobile baselines: DoomGLES fps and boot time, a frame counter run by the owner (with Q3CE's settings tests, docs/research/quake3-on-the-x51v.md section 5) | Claude, 2026-10-13 | session | open |
| R12 | C39: what each test image changes on the board, and whether a crash puts it back | the owner, 2026-10-21 | device-run | open |
| R13 | The corner sweep at 117-133.25 / 1: also shift the whole scene a few pixels (tile seam or quad edge?), put one arm's target at `0x0` (option C's real address), reverse the tile order in one arm | Claude, 2026-10-24 | image:rp-gpubench | open |
| R14 | P8 / T8 ticked or not from R1's result (2D and 3D overlapping: FT measured 9% shorter frames) | Claude, 2026-10-24 | after:R1 | open |
| R15 | V5's rest: bit 27's meaning (C70) and the scaler coefficients' format, to compute a filter for shrinking (C73) | Claude, 2026-10-24 | image:rp-blend | open |
| R16 | Dell's free 2x on the graphics plane (`GDRCTRL` pixel and line doubling, bits 31 and 29): tried on the X51v | Claude, 2026-10-24 | image:rp-blend; device-run | open |
| R17 | V7: Windows Mobile's own hop programs on RetroPane with one word a hop (`rp-probe` `wmhop2`), then what beats Windows Mobile where we're behind (SRAM page tables, TLB lock, the open-row chase) | Claude, 2026-10-24 | image:rp-probe | open |
| R18 | docs/research/storage-speed.md's SD row still says 1 MiB reads fall back to PIO; fixed 2026-10-21 (`dmachain`, any length): the page updated with a measured number | Claude, 2026-10-24 | image:rp-storfast | open |
| R19 | P4: the TA against CPU-built lists, once the divide-1 work is done with `rp-glesfeat` (docs/design/p4-ta-speed.md, the owner's votes Q1-Q3) | the owner, 2026-10-24 | after:the divide-1 work | open |
| R20 | C's tearing design (ADR-0034 P1 = C): the core's tile write order measured, the render started behind the scan line, frames longer than a refresh | the owner, 2026-10-24 | image:rp-gpubench; image:rp-glesfeat | open |
| R21 | The PDA's internal storage (the DiskOnChip flash) shown to the PC as a USB drive beside the SD and CF cards. Read-only first; Dell's bootloader region never written (the flash-install rule); depends on reading the DiskOnChip at all (R6's read path) | the owner, 2026-10-24 | after:the flash read path (R6); image:rp-loader | open |
| R22 | The CF card as a USB drive is slow in Windows Explorer: the drive's icon, its capacity bar and a folder's contents all take a long time to appear. Measure what Windows reads on attach (sectors, their order, each command's time) against the card path's 4.1-5.2 MB/s, then fix the slow part (polled reads, per-command latency, a read cache for the FAT and root directory) | the owner, 2026-10-24 | image:rp-loader; device-run | open |
