# The optimizations' backlog (payoff order)

The owner's restructure (2026-10-22, [gate-classification.md](gate-classification.md)):
the gate keeps only the architectural lines; every optimization (`opt` in
the two ledgers, [2700g-full-potential.md](2700g-full-potential.md) and
[cpu-full-potential.md](cpu-full-potential.md)) is here, ordered by payoff
against the device time it costs, and worked continuously from the top.
Each lands with its benchmark in the ratchet (`cargo xtask perf`,
[perf-floors.md](../perf-floors.md)). A line is **declined** when its
expected payoff is smaller than the device time it costs, with that reason
in its ledger row. An architectural line's leftovers once ticked appear as
`<line> (rest)`. `cargo xtask gate` checks this table holds every unticked
optimization exactly once.

Payoffs are estimates from the measurements so far (named), not results;
each is replaced by the measured number when the line lands. Device cost
counts runs on the X51v (U = unattended, W = the owner watching).

| Rank | Line | What's left | Payoff (estimate) | Device cost | Next step |
|---|---|---|---|---|---|
| 1 | CPU C6 | The mixer's cycles a frame with the media unit on, against `RP_AUDIO_PLAIN` | The filter's dot product 1.46x (probe `dot48`, 16.5 against 11.3 us a block): the mixer at 48 kHz below its 669 cycles a frame (4.7% of the CPU), roughly a fifth off; also 22.05/11.025 kHz | 2 U runs, ~5 min each; built | `rp-audio-s2` (`RP_AUDIO_QUICK`) twice, compare the cycles lines; then the other hot paths (colour conversion, T5's converters) |
| 2 | CPU C9 | Fat LTO and one codegen unit are already on (`[profile.release]`); what's open is **the image's opt-level reaching its libraries through LTO**: images built for size (`s`: the probe, kernel, loader) re-optimize the hot libraries they link at `s` (`rp_lz4::compress` 1,084 B at `s`, 1,100 B at 3; `decompress` 1,428 against 1,472), though `Cargo.toml` says those libraries keep level 3. The probe at `s` and at 3, A/B | If level 3 is faster, every hot library in an `s` image gains (the kernel's lz4, `mm`); the probe's own floors measure `s` code today | 4 U probe runs (each build twice, alternating) | Built 2026-10-22: `cargo xtask codegen-variant rp-probe probe-o3 'profile.release.package.rp-probe.opt-level=3'` (112,184 B loaded, a measurement variant); device: `cargo xtask load rp-probe` and `cargo xtask load target/variants/probe-o3/rp-probe.bin`, alternately, then the rows compared |
| 3 | CPU C8 | The event counters (cache and TLB misses, stalls) in the probe's rows | Indirect: every later line measured by cause, not only by time (it makes the rest cheaper) | 1 U probe run (the shipped probe: the same run as C9's A side) | Built 2026-10-22 (`rp_hal_pxa27x::pmu`, the probe's `pmu` section, emulator plumbing passes); device: `cargo xtask load rp-probe`, the `pmu_*` checks and `count` lines; then counts around more benches and into the ratchet |
| 4 | CPU C11 | SD's polling fix (a pause between DMA polls, reads -28%) checked in the other drivers that poll during DMA (CF, UDC) | Up to SD's 28% on CF reads, if CF's loop has the same effect; none if not | 2 U runs (`rp-storfast` with the CF card in) | Audited 2026-10-22: only CF's sector DMA wait (`rp_drv_cf::read_sector`) polls back to back (UDC, USB storage and I2S don't poll a DMA's end); `RP_CF_POLL_SPIN` added (default 0 = as before; emulator `emu-storfast` passes at 0 and 200). Device: `RP_CF_POLL_SPIN=0 cargo xtask load rp-storfast` and `RP_CF_POLL_SPIN=200 ...`, compare C2's "read 1024 KiB by DMA from the window" KiB/s; the default set from the result |
| 5 | CPU T2 (rest) | The SRAM staging wired into the OS's upload path | Uploads 34.8 to 32.2 ms a MiB (the bus's 32 MB/s) with the CPU free 7.4 ms of it (run E1e) | 1 U run (gpubench E) | The staging buffers' addresses in `docs/hw/memory-map.md`; the upload path borrows them |
| 6 | CPU C3 (rest) | The shared SRAM layout built (table, staging, kept log, stacks and run queues) | Holds the measured wins (TLB spills 5.2x cheaper with the table there) as more moves in | Emulator first, 1 U run | The layout in `memory-map.md`, one owner per range, a check that nothing overlaps |
| 7 | 2700G P13 | Lines and the brush palette on the 2D engine | Interface lines and patterns drawn with no CPU (GDI's `LineTo`, borders) | 1 U run (`rp-mbx-blend` B12, shared with 8-10) | B12: lines, brush palette, 2/4 bpp sources, Dell's remaining command words |
| 8 | 2700G P7 | 2 and 4 bpp sources through the palette, the brush palette | Smaller pictures for icons and glyphs (4 bpp half of 8) | in B12 | as rank 7 |
| 9 | 2700G V3 | 2 and 4 bpp sources (Dell's format table) | as rank 8 (the same run checks the claim) | in B12 | as rank 7 |
| 10 | 2700G V15 | Dell's 2D command words not yet proven | Each proven word usable; none expected to be fast in itself | in B12 | as rank 7 |
| 11 | 2700G P6 | The scaler (proven, 18.7 ms a full-screen 2x) used where the compositor zooms or makes thumbnails | Zoom and thumbnails at the engine's speed, not the CPU's | 0 (proven); waits for the compositor's code | Wire it when the compositor needs zoom |
| 12 | CPU V1 | The I-cache's and mini data cache's real sizes | Sizes for C5's tuning; on its own none | in a probe run (ranks 2-3) | A walk over 16-64 KiB of code and of mini-cached data |
| 13 | CPU C5 | The mini data cache for streaming data, line locking for hot loops | A few percent where streaming data evicts hot data (uploads, audio rings); unmeasured | 1-2 U probe runs | Mark a streaming buffer mini-cached, time a mixed workload |
| 14 | CPU V8 | DMA priorities under contention (SD or CF with audio) | Audio kept steady under heavy card traffic; nothing seen to slip yet | 1 U run | Audio ring and an SD soak together, by priority |
| 15 | 2700G P12 | DMA burst and width settings into 2700G memory | Small: the bus paces the 2700G at 30-33 MB/s whatever the DMA does | 1 U run | The burst sweep (as V8's) into local memory |
| 16 | CPU T5 | The 16-bit reorder timed | None expected: uploads are bus-limited, and the 32-bit reorder already outruns the bus (E1a, E1e) | 1 U run | Time it in gpubench E; likely declined then |
| 17 | 2700G V17 | Status bit 10's meaning | None known (2D waits stay polled; bit 3 seen) | 1 U run | Watch bit 10 across a TA job and a scan-out change |
| 18 | 2700G V16 | The plane formats left: indexed, inverse alpha, packed YUV | Indexed plane: a smaller screen buffer; packed YUV only with video (phase 7) | 1 W run (a picture to look at) | `rp-blend` with each format |
| 19 | 2700G V7 | Colour keys at scan-out | Cut-outs with no drawing; the compositor doesn't need them | 1 W run | With P10's run (phase 9) |
| 20 | 2700G P5 | The second texture unit: a mask times a picture in one pass | Effects in one pass instead of two; needs the TA path, which hangs (C53) and whose use is P4's (architectural) answer | Several runs, each hang needs a restart | After P4 and C53 |
| 21 | 2700G V9 | The second unit's words, fog bit 24, TSP bit 14 | as rank 20 | as rank 20 | as rank 20 |

Declined from the backlog: CPU C12 (the owner, 2026-10-22: battery only, no battery reading exists, as P16).
