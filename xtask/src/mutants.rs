//! `cargo xtask mutants [--emu]`: the mutants each test suite must kill
//! (docs/testing.md).
//!
//! A mutant is one deliberate bug: a line of real code changed to what a
//! plausible mistake would leave. Each names the tests that must catch it.
//! The runner applies one at a time, runs those tests, expects them to fail
//! (the mutant is *killed*), and puts the file back. A mutant that survives
//! means a test that doesn't test what it claims: the run fails.
//!
//! Safety: a file is only mutated if git says it has no changes (so a crash
//! mid-run can always be undone with `git checkout`), the original is put
//! back by a guard even if a step errors, and each mutant's text must occur
//! exactly once (a stale entry is reported, never guessed at).

use std::path::{Path, PathBuf};
use std::process::Command;

/// How a mutant is checked.
pub enum Check {
    /// `cargo test` with these arguments must fail.
    Host(&'static [&'static str]),
    /// This emulator check (after a cross build) must fail. Slow: only with
    /// `--emu`.
    Emu(fn(&Path) -> Result<(), String>),
}

/// One deliberate bug.
pub struct Mutant {
    /// What it breaks, in a few words.
    pub name: &'static str,
    /// The file, from the workspace root.
    pub file: &'static str,
    /// The exact original text (must occur once).
    pub from: &'static str,
    /// What the mistake leaves instead.
    pub to: &'static str,
    /// The tests that must kill it.
    pub check: Check,
}

/// `Display::on_screen` ignoring the registers: the mistake the crash
/// boxes made before (drawn where nothing shows on the device).
const CRASH_BOX_FROM: &str = "        self.scanned_out(vram_size)\n            .unwrap_or(Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565))";
const CRASH_BOX_TO: &str =
    "        let _ = vram_size;\n        Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565)";

/// The list. Every security or crash-safety rule with a test gets a mutant
/// here, so the test is known to bite.
pub const MUTANTS: &[Mutant] = &[
    Mutant {
        name: "I2C: address-only writes let through (the X51v's controller can't finish them)",
        file: "drivers/pxa-i2c/src/lib.rs",
        from: "        if last == 0 {
            return Err(I2cError::Unsupported);
        }
",
        to: "",
        check: Check::Host(&[
            "-p",
            "rp-drv-pxa-i2c",
            "an_address_only_write_is_refused_without_touching_the_bus",
        ]),
    },
    Mutant {
        name: "I2C: STOP sent with START on a one-byte write",
        file: "drivers/pxa-i2c/src/lib.rs",
        from: "        if let Err(e) = self.send(addr << 1, ICR_START) {",
        to: "        if let Err(e) = self.send(addr << 1, ICR_START | ICR_STOP) {",
        check: Check::Host(&[
            "-p",
            "rp-drv-pxa-i2c",
            "no_write_ever_puts_start_and_stop_on_one_byte",
        ]),
    },
    Mutant {
        name: "I2C: a controller left busy after the stop isn't reset (SCL held low)",
        file: "drivers/pxa-i2c/src/lib.rs",
        from: "        if self.regs.read32(reg::ISR) & ISR_UB != 0 {
            self.reset();
        }
",
        to: "",
        check: Check::Host(&[
            "-p",
            "rp-drv-pxa-i2c",
            "a_controller_still_busy_after_the_stop_is_reset",
        ]),
    },
    Mutant {
        name: "renderer: the depth mode written into the alpha-test bits again (2026-10-15's mistake)",
        file: "drivers/mbx3d-render/src/lib.rs",
        from: "        None => m3::ISP_ALWAYS,
",
        to: "        None => d.depth.map_or(m3::ISP_ALWAYS, |dp| (dp.mode & 7) << 29),
",
        check: Check::Host(&[
            "-p",
            "rp-mbx3d-render",
            "--features",
            "model",
            "the_depth_mode_never_reaches_the_alpha_bits",
        ]),
    },
    Mutant {
        name: "xtask: two emulator runs given the same ports",
        file: "xtask/src/devlock.rs",
        from: "        if file.try_lock().is_ok() && (start..start + len).all(is_free) {
",
        to: "        if (start..start + len).all(is_free) {
",
        check: Check::Host(&[
            "-p",
            "xtask",
            "concurrent_emulator_runs_get_their_own_ports",
        ]),
    },
    Mutant {
        name: "xtask: a port script run without the Axim lock (two runs overlapped)",
        file: "xtask/src/usb.rs",
        from: "    crate::devlock::ensure_held()?;
",
        to: "",
        check: Check::Host(&["-p", "xtask", "only_usb_powershell_reaches_the_device"]),
    },
    Mutant {
        name: "xtask: a busy Axim lock taken as free",
        file: "xtask/src/devlock.rs",
        from: "            Err(std::fs::TryLockError::WouldBlock) => {}
",
        to: "            Err(std::fs::TryLockError::WouldBlock) => break,
",
        check: Check::Host(&["-p", "xtask", "second_caller_waits_until_the_first_is_done"]),
    },
    // (Removed 2026-10-19: "today: an idle sleep past the run limit". Its
    // mutant took out the run-limit cap on the idle wait, but that wait has
    // since been capped at 50 ms too (the pen line looked at every 50 ms),
    // so either bound alone keeps the image checking in: the mutant could
    // no longer fail. The rule stands, held twice over.)
    Mutant {
        name: "shell: a notice shown without marking its area (never redrawn)",
        file: "shell/core/src/lib.rs",
        from: "        self.notice = Some((what, self.now.wrapping_add(NOTICE_MS)));
        self.dirty(NOTICE);",
        to: "        self.notice = Some((what, self.now.wrapping_add(NOTICE_MS)));",
        check: Check::Host(&["-p", "rp-shell", "incremental_frames_match_full_redraws"]),
    },
    Mutant {
        name: "renderer: commands drawn without clipping to the dirty rectangle",
        file: "drivers/gfx/src/render.rs",
        from: "            Cmd::Fill { r, color } => self.layer.fill(rect(r).intersect(&clip), color),",
        to: "            Cmd::Fill { r, color } => self.layer.fill(rect(r), color),",
        check: Check::Host(&[
            "-p",
            "rp-gfx",
            "nothing_outside_the_dirty_rectangles_changes",
        ]),
    },
    Mutant {
        name: "display: adopt keeps a left-over video plane over the screen",
        file: "drivers/mbx-display/src/lib.rs",
        from: "        if let Some(p) = self.scanned_out(vram_size) {\n            layers::solo(&mut self.regs);\n",
        to: "        if let Some(p) = self.scanned_out(vram_size) {\n",
        check: Check::Host(&["-p", "rp-drv-mbx-display", "adopt_keeps_wm_plane"]),
    },
    Mutant {
        name: "crash box drawn at 0x60000, not on the plane on screen (off screen on the device)",
        file: "drivers/mbx-display/src/lib.rs",
        from: CRASH_BOX_FROM,
        to: CRASH_BOX_TO,
        check: Check::Host(&["-p", "rp-drv-mbx-display", "wm_plane_at_offset_0"]),
    },
    Mutant {
        name: "crash box drawn at 0x60000 (emulator: rp-crash's box on WM's plane)",
        file: "drivers/mbx-display/src/lib.rs",
        from: CRASH_BOX_FROM,
        to: CRASH_BOX_TO,
        check: Check::Emu(crate::emu::boot_loader_crash),
    },
    Mutant {
        name: "display reference: per-pixel alpha read inverted",
        file: "drivers/mbx-display/src/scanout.rs",
        from: "                    3 => s.a,",
        to: "                    3 => 255 - s.a,",
        check: Check::Host(&["-p", "rp-drv-mbx-display", "every_stage_shows"]),
    },
    Mutant {
        name: "region list: a middle split skips its reservation",
        file: "kernel/mm/src/space.rs",
        from: "                // A hole in the middle: one more region, reserved first.\n                self.list.reserve(rp, 1)?;\n",
        to: "",
        check: Check::Host(&["-p", "rp-kernel-mm", "region_operations"]),
    },
    Mutant {
        name: "region list: protect skips its reservation",
        file: "kernel/mm/src/space.rs",
        from: "        self.list.reserve(rp, 2)?;\n        self.split_at(rp, start)?;",
        to: "        self.split_at(rp, start)?;",
        check: Check::Host(&["-p", "rp-kernel-mm", "region_operations"]),
    },
    Mutant {
        name: "region list: freed pages not wiped",
        file: "kernel/mm/src/space.rs",
        from: "        self.mem.zero(page);\n        let f = self.mem.phys_frame(page);",
        to: "        let f = self.mem.phys_frame(page);",
        check: Check::Host(&["-p", "rp-kernel-mm", "region_pages_are_wiped"]),
    },
    Mutant {
        name: "pin limit not checked on map",
        file: "kernel/mm/src/space.rs",
        from: "u64::from(r.len) > u64::from(self.pin_limit)",
        to: "u64::from(r.len) > u64::MAX",
        check: Check::Host(&["-p", "rp-kernel-mm", "pinning_is_limited"]),
    },
    Mutant {
        name: "frame allocator accepts a double free",
        file: "kernel/mm/src/frames.rs",
        from: "        if t & USED == 0 {\n            return Err(Error::NotAllocated);",
        to: "        if false {\n            return Err(Error::NotAllocated);",
        check: Check::Host(&["-p", "rp-kernel-mm", "frames::"]),
    },
    Mutant {
        name: "IPC grant can add rights the granter lacks",
        file: "kernel/ipc/src/lib.rs",
        from: "if !e.rights().contains(Rights::GRANT) || !e.rights().contains(rights) {",
        to: "if !e.rights().contains(Rights::GRANT) {",
        check: Check::Host(&["-p", "rp-kernel-ipc"]),
    },
    Mutant {
        name: "LZ4 accepts a match reaching before the output",
        file: "kernel/lz4/src/lib.rs",
        from: "        if offset == 0 || offset > o {",
        to: "        if offset == 0 {",
        check: Check::Host(&["-p", "rp-lz4"]),
    },
    Mutant {
        name: "constant-time compare ignores length",
        file: "kernel/crypto/src/lib.rs",
        from: "    if a.len() != b.len() {\n        return false;\n    }\n    let diff",
        to: "    let diff",
        check: Check::Host(&["-p", "rp-crypto", "tags_compare_whole"]),
    },
    Mutant {
        name: "pixel blend weights the destination wrong",
        file: "drivers/gfx/src/lib.rs",
        from: "    unspread((spread(src) * a + spread(dst) * (32 - a)) >> 5)",
        to: "    unspread((spread(src) * a + spread(dst) * (31 - a)) >> 5)",
        check: Check::Host(&["-p", "rp-gfx", "the_one_multiply"]),
    },
    Mutant {
        name: "dirty rectangles not widened to whole words",
        file: "drivers/gfx/src/dirty.rs",
        from: "    let x0 = r.x & !1;",
        to: "    let x0 = r.x;",
        check: Check::Host(&["-p", "rp-gfx", "dirty"]),
    },
    Mutant {
        name: "tick re-armed without checking the counter hasn't passed it (a 22-minute freeze)",
        file: "hal/pxa27x/src/soc.rs",
        from: "        self.arm1(next, period);",
        to: "        self.0.write32(ost_reg::OSMR1, next);",
        check: Check::Host(&["-p", "rp-hal-pxa27x", "a_late_tick"]),
    },
    Mutant {
        name: "soft run limit not routed to FIQ (IRQs-off hangs reset the device)",
        file: "hal/pxa27x/src/rt.rs",
        from: "    i.route_fiq(crate::soc::irq::OST_4_11, true);",
        to: "    i.route_fiq(crate::soc::irq::OST_4_11, false);",
        check: Check::Emu(crate::emu::boot_loader_run_limit),
    },
    Mutant {
        name: "CF slot never powered on (the card reads nothing, as on the device)",
        file: "boot/loader/src/main.rs",
        from: "        cpld.set_cf_power(true, &mut BusyWait);",
        to: "",
        check: Check::Emu(crate::emu::boot_loader_cf),
    },
    Mutant {
        name: "CPLD shadow not seeded from Windows Mobile's table (a CF reset darkens the screen)",
        file: "drivers/cpld/src/handover.rs",
        from: "        Ok(v) => (v, Source::Adopted),",
        to: "        Ok(_) => ([0; REGS], Source::Adopted),",
        check: Check::Host(&[
            "-p",
            "rp-drv-cpld",
            "bits_windows_mobile_left_on_survive_our_writes",
        ]),
    },
    Mutant {
        name: "the loader's CPLD shadow not seeded from Windows Mobile's table in RAM",
        file: "drivers/cpld/src/handover.rs",
        from: "        Ok(v) => (v, Source::Adopted),",
        to: "        Ok(_) => (DEFAULTS, Source::Adopted),",
        check: Check::Emu(crate::emu::boot_loader_cf_alone),
    },
    Mutant {
        name: "CPLD defaults without the backlight enable (no table: a CF reset darkens the screen)",
        file: "drivers/cpld/src/handover.rs",
        from: "    v[reg::CTL18 / 2] = crate::BACKLIGHT_ENABLE;",
        to: "",
        check: Check::Host(&[
            "-p",
            "rp-drv-cpld",
            "defaults_keep_the_backlight_through_a_cf_reset",
        ]),
    },
    Mutant {
        name: "CPLD shadow changes not written through to the shared table (the next program loses them)",
        file: "drivers/cpld/src/handover.rs",
        from: "        *self.words = encode(&self.values);",
        to: "",
        check: Check::Host(&[
            "-p",
            "rp-drv-cpld",
            "adopting_takes_the_table_over_and_writes_changes_through",
        ]),
    },
    Mutant {
        name: "medium change reported as a unit attention only (Windows keeps its stale view)",
        file: "drivers/usb-msc/src/lib.rs",
        from: "                u.attention = false;\n                u.away = Some(None);",
        to: "                u.attention = true;",
        check: Check::Host(&[
            "-p",
            "rp-usb-msc",
            "a_device_side_change_takes_the_medium_away",
        ]),
    },
    Mutant {
        name: "driver host's device window mapped a page off (the PWM unreachable)",
        file: "boot/kernel/src/main.rs",
        from: "                        start: base + rp_hal_user::Device::Pwm0.va() as u32,",
        to: "                        start: base + rp_hal_user::Device::Pwm0.va() as u32 + 0x1000,",
        check: Check::Emu(crate::emu::boot_kernel),
    },
    Mutant {
        name: "a restarted host's blocked thread resumes its old context",
        file: "boot/kernel/src/main.rs",
        from: "            if take_reentry(me) {",
        to: "            if false && take_reentry(me) {",
        check: Check::Emu(crate::emu::boot_kernel),
    },
    Mutant {
        name: "any process may wait for the driver host's interrupt",
        file: "boot/kernel/src/main.rs",
        from: "        if from != DISPLAY_SLOT || f.arg(0) != 0 || IPC",
        to: "        if f.arg(0) != 0 || IPC",
        check: Check::Emu(crate::emu::boot_kernel),
    },
    Mutant {
        name: "rp-threads records no thread switches (the trace over USB shows none)",
        file: "boot/threads/src/main.rs",
        from: "            trace(Kind::Switch, u32::from(cur.0), u32::from(next.0));",
        to: "",
        check: Check::Emu(crate::emu::boot_loader_trace),
    },
    Mutant {
        name: "card check's cluster bitmap too small for a full-size card (fine on test cards)",
        file: "boot/loader/src/main.rs",
        from: "card_check(&mut lun, seen, scratch, &mut status);",
        to: "card_check(&mut lun, &mut seen[..64 * 1024], scratch, &mut status);",
        check: Check::Emu(crate::emu::boot_loader_full_size),
    },
    Mutant {
        name: "a medium change keeps the blocks read ahead (stale data served)",
        file: "drivers/usb-msc/src/lib.rs",
        from: "    pub fn medium_changed(&mut self, lun: usize) {\n        self.ahead = None;",
        to: "    pub fn medium_changed(&mut self, lun: usize) {",
        check: Check::Host(&["-p", "rp-usb-msc", "a_medium_change_drops_the_read_ahead"]),
    },
    Mutant {
        name: "a unit without overlap reads after one with it leaves the front on the second half (the PC saw the CF drive as zeros)",
        file: "drivers/usb-msc/src/lib.rs",
        from: "                if !overlap {
                    self.front = 0;
                }
",
        to: "",
        check: Check::Host(&[
            "-p",
            "rp-usb-msc",
            "a_unit_without_overlap_sends_what_it_read",
        ]),
    },
    Mutant {
        name: "a host's view from before the drive started is trusted (its old root directory written back)",
        file: "drivers/usb-msc/src/lib.rs",
        from: "        if !unit.vouched && matches!(op, 0x00 | 0x25 | 0x28 | 0x2A | 0x2F | 0x35) {",
        to: "        if false {",
        check: Check::Host(&[
            "-p",
            "rp-usb-msc",
            "a_view_from_before_the_drive_started_is_sent_away_first",
        ]),
    },
    Mutant {
        name: "a host's view from before the loader's restart writes over an image's file (emulator)",
        file: "drivers/usb-msc/src/lib.rs",
        from: "        if !unit.vouched && matches!(op, 0x00 | 0x25 | 0x28 | 0x2A | 0x2F | 0x35) {",
        to: "        if false {",
        check: Check::Emu(crate::emu::boot_loader_stale_view_alone),
    },
    Mutant {
        name: "a write uses the blocks read ahead instead of dropping them",
        file: "drivers/usb-msc/src/lib.rs",
        from: "        if !write\n            && let Some((unit, first, n)) = ahead",
        to: "        if let Some((unit, first, n)) = ahead",
        check: Check::Host(&[
            "-p",
            "rp-usb-msc",
            "read_ahead_serves_the_next_read_and_a_write_drops_it",
        ]),
    },
    // (Removed 2026-10-19: "CF card not attached on insertion". Since the
    // retry of an inserted card left idle (once per insertion, within a
    // second; 2026-10-15) it is attached either way, so the mutant could no
    // longer fail. Held twice over.)
    Mutant {
        name: "CF card in at start-up attached only after the drive is offered (the PC's \"USB Drive\" with no size)",
        file: "boot/loader/src/main.rs",
        from: "        if cf_in {\n            cf_attach(&mut cf, &mut gpio, cpld.as_mut(), false);\n        }\n",
        to: "",
        check: Check::Emu(crate::emu::boot_loader_cf),
    },
    Mutant {
        name: "a medium arriving where the host saw none is kept away for seconds",
        file: "drivers/usb-msc/src/lib.rs",
        from: "            if u.shown || u.away.is_some() {",
        to: "            if true {",
        check: Check::Host(&[
            "-p",
            "rp-usb-msc",
            "a_medium_arriving_where_the_host_saw_none_is_offered_at_once",
        ]),
    },
    Mutant {
        name: "CF card detected by GPIO13 again (it sticks at no card after a removal: a re-inserted card is never seen)",
        file: "boot/loader/src/main.rs",
        from: "            Some(c) => c.cf_present(),",
        to: "            Some(_) => !gpio.read(CF_DETECT),",
        check: Check::Emu(crate::emu::boot_loader_cf_alone),
    },
    Mutant {
        name: "CF slot power-cycled at start-up although it was found powered (0.4 s of every boot)",
        file: "boot/loader/src/main.rs",
        from: "if cpld.cf_powered() && gpio.read(CF_READY) {",
        to: "if false && cpld.cf_powered() && gpio.read(CF_READY) {",
        check: Check::Emu(crate::emu::boot_loader_cf_alone),
    },
    Mutant {
        name: "CF slot left powered after the card is removed",
        file: "boot/loader/src/main.rs",
        from: "            cpld.set_cf_power(false, &mut BusyWait);\n            self.powered = false;",
        to: "            self.powered = false;",
        check: Check::Emu(crate::emu::boot_loader_cf),
    },
    Mutant {
        name: "cab: a DEFLATE match reaching before the window is followed",
        file: "personality/ce/cab/src/inflate.rs",
        from: "            if distance > *pos || *pos + len > end {",
        to: "            if *pos + len > end {",
        check: Check::Host(&[
            "-p",
            "rp-cab",
            "a_match_into_the_history_works_and_before_it_fails",
        ]),
    },
    Mutant {
        name: "cab: data block checksums ignored",
        file: "personality/ce/cab/src/lib.rs",
        from: "            if csum != 0 && checksum(&h[4..8], checksum(data, 0)) != csum {",
        to: "            if csum != 0 && checksum(&h[4..8], checksum(data, 0)) != csum && false {",
        check: Check::Host(&["-p", "rp-cab", "a_changed_byte_fails_the_checksum"]),
    },
    Mutant {
        name: "cab: a file running past its folder's end accepted short",
        file: "personality/ce/cab/src/lib.rs",
        from: "        if done < end {
            Err(Error::Truncated)
        } else {
            Ok(())
        }",
        to: "        Ok(())",
        check: Check::Host(&["-p", "rp-cab", "a_file_past_its_folder_end_is_truncated"]),
    },
    Mutant {
        name: "ce-setup: `..` accepted in an install path",
        file: "personality/ce/setup/src/lib.rs",
        from: "        let dots = names && (piece == b\".\" || piece == b\"..\");",
        to: "        let dots = false;",
        check: Check::Host(&[
            "-p",
            "rp-ce-setup",
            "bad_references_and_paths_are_refused_up_front",
        ]),
    },
    Mutant {
        name: "ce-setup: a script accepted before its whole plan is checked",
        file: "personality/ce/setup/src/lib.rs",
        from: "        script.plan(|_| Ok(()))?;\n        Ok(script)",
        to: "        Ok(script)",
        check: Check::Host(&[
            "-p",
            "rp-ce-setup",
            "bad_references_and_paths_are_refused_up_front",
        ]),
    },
    Mutant {
        name: "ce-setup: a DOCTYPE in _setup.xml skipped instead of refused",
        file: "personality/ce/setup/src/xml.rs",
        from: "                // DOCTYPE, ENTITY, CDATA: refused.\n                return Err(Error::BadXml);",
        to: "                self.i = self.find(at + 2, b\">\")? + 1;",
        check: Check::Host(&[
            "-p",
            "rp-ce-setup",
            "setup_xml_outside_the_subset_is_refused",
        ]),
    },
    Mutant {
        name: "pxa-udc: a full OUT packet sized by the byte count (lost when it reads short)",
        file: "drivers/pxa-udc/src/lib.rs",
        from: "        let count = if csr & reg::SP == 0 {",
        to: "        let count = if false {",
        check: Check::Host(&["-p", "rp-drv-pxa-udc", "a_lagging_byte_count_loses_no_data"]),
    },
    Mutant {
        name: "pxa-udc: the CPU takes OUT packets meant for the DMA controller",
        file: "drivers/pxa-udc/src/lib.rs",
        from: "        if self.dma[SLOT_OUT] {\n            return;\n        }\n",
        to: "",
        check: Check::Host(&[
            "-p",
            "rp-drv-pxa-udc",
            "an_endpoint_under_dma_is_left_to_the_dma_controller",
        ]),
    },
    Mutant {
        name: "usb-msc: a reset leaves the abandoned command's DMA running",
        file: "drivers/usb-msc/src/lib.rs",
        from: "        if self.phase == Phase::Command && (self.send_dma || self.recv_dma.is_some()) {",
        to: "        if false {",
        check: Check::Host(&["-p", "rp-usb-msc", "a_reset_aborts_the_transfer_in_flight"]),
    },
    Mutant {
        name: "usbbench: a bench folder holding someone else's files used anyway",
        file: "xtask/src/usbbench.rs",
        from: "                if e.file_name() != BENCH_FILE {",
        to: "                if false {",
        check: Check::Host(&["-p", "xtask", "the_bench_folder_keeps_off_other_files"]),
    },
    Mutant {
        name: "night light: the picture steps leave the gamma table at full (the picture never dims)",
        file: "boot/nightlight/src/main.rs",
        from: "                let t = scale_gamma(&table, step.picture);",
        to: "                let t = scale_gamma(&table, 256);",
        check: Check::Emu(crate::emu::boot_loader_nightlight),
    },
    Mutant {
        name: "display cal: a dial change never reaches the gamma table (every setting looks like the original)",
        file: "boot/display-cal/src/main.rs",
        from: "                    table(d, &found)\n                };",
        to: "                    found\n                };",
        check: Check::Emu(crate::emu::boot_loader_display_cal),
    },
    Mutant {
        name: "tone: the codec's I2C gate (GPIO82) never opens: the codec is never found (the first device run)",
        file: "boot/tone/src/main.rs",
        from: "            self.0.write(I2C_GATE, open);",
        to: "            self.0.write(I2C_GATE, false);",
        check: Check::Emu(crate::emu::boot_loader_tone),
    },
    Mutant {
        name: "looks: the strength dial never reaches the picture (every strength shows the original)",
        file: "boot/looks/src/main.rs",
        from: "                draw(fb, kind, photo, &t, 0..H);",
        to: "                draw(fb, kind, photo, &found, 0..H);",
        check: Check::Emu(crate::emu::boot_loader_looks),
    },
    Mutant {
        name: "screen measure: a program's table change never reaches the gamma table",
        file: "boot/screen-measure/src/main.rs",
        from: "                    if s.table != table {",
        to: "                    if false && s.table != table {",
        check: Check::Emu(crate::emu::boot_loader_screen_measure),
    },
    Mutant {
        name: "screen measure: a damaged program is shown anyway",
        file: "boot/screen-measure/src/program.rs",
        from: "        if fnv1a(&room[HEADER..end]) != u32_at(room, 16) {",
        to: "        if false && fnv1a(&room[HEADER..end]) != u32_at(room, 16) {",
        check: Check::Host(&["-p", "rp-screen-measure", "the_device_refuses_damage"]),
    },
    Mutant {
        name: "loader: a new loader sent as an image is reported as a hang (crashlog.txt overwritten)",
        file: "boot/loader/src/main.rs",
        from: "                        devices::log_mark_handover();",
        to: "",
        check: Check::Emu(crate::emu::boot_loader_usb_alone),
    },
    Mutant {
        name: "shared OS timer line never dispatched (a claimed channel 5-11 interrupt fires forever)",
        file: "hal/pxa27x/src/rt.rs",
        from: "    let other = pending & !(1 << crate::soc::irq::OST1);",
        to: "    let other = pending & !(1 << crate::soc::irq::OST1 | 1 << crate::soc::irq::OST_4_11);",
        check: Check::Emu(crate::emu::boot_loader_noise),
    },
    Mutant {
        name: "audio: the I2S restart guard skipped (a switch-on within 645 ms is dropped)",
        file: "services/audio/src/lib.rs",
        from: "        if since >= RESTART_GUARD_MS {",
        to: "        if since < u32::MAX {",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "up_soon_after_down_waits_out_the_guard_and_is_taken",
        ]),
    },
    Mutant {
        name: "audio: a jack edge while down forgotten (the next power-up wouldn't know)",
        file: "services/audio/src/lib.rs",
        from: "            self.jack_dirty |= edge;",
        to: "            let _ = edge;",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_jack_edge_while_down_powers_nothing_and_is_read_at_the_next_up",
        ]),
    },
    Mutant {
        name: "audio: going down leaves the speaker amplifier on",
        file: "services/audio/src/lib.rs",
        from: "        self.amp_off();
        let (r, bus) = codec.power_down();
        self.codec = Codec::Down(bus);
        self.back_down();",
        to: "        let (r, bus) = codec.power_down();
        self.codec = Codec::Down(bus);
        self.back_down();",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_route_failing_after_the_amplifier_takes_it_back_off",
        ]),
    },
    Mutant {
        name: "audio: to headphones, the amplifier off before the speaker pair is muted",
        file: "services/audio/src/lib.rs",
        from: "        if to == Output::Speaker {\n            self.amp_on();\n        }\n        let r",
        to: "        if to == Output::Speaker {\n            self.amp_on();\n        } else {\n            self.amp_off();\n        }\n        let r",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_jack_edge_while_up_reroutes_after_the_debounce",
        ]),
    },
    Mutant {
        name: "audio: the jack edge no longer debounced",
        file: "services/audio/src/lib.rs",
        from: "        if now.wrapping_sub(since) < JACK_DEBOUNCE_MS {",
        to: "        if now.wrapping_sub(since) < 0 {",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_jack_edge_while_up_reroutes_after_the_debounce",
        ]),
    },
    Mutant {
        name: "GPIO: taking one pin's edge clears every latched edge",
        file: "drivers/pxa-gpio/src/lib.rs",
        from: "        self.regs.write32(off, bit);\n        true",
        to: "        self.regs.write32(off, u32::MAX);\n        true",
        check: Check::Host(&[
            "-p",
            "rp-drv-pxa-gpio",
            "an_edge_is_taken_once_and_cleared_alone",
        ]),
    },
    Mutant {
        name: "WM8750: the reference switched off before the outputs",
        file: "drivers/wm8750/src/lib.rs",
        from: "        step(self.set(reg::PWR2, 0));\n        step(self.set(reg::PWR1, 0));",
        to: "        step(self.set(reg::PWR1, 0));\n        step(self.set(reg::PWR2, 0));",
        check: Check::Host(&[
            "-p",
            "rp-drv-wm8750",
            "power_down_mutes_then_outputs_then_the_reference",
        ]),
    },
    Mutant {
        name: "WM8750: the set-up moves VMID to playback without waiting for it to charge",
        file: "drivers/wm8750/src/lib.rs",
        from: "        wait_us(VMID_CHARGE_US);
",
        to: "",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "the_jack_is_read_only_once_the_set_up_has_settled",
        ]),
    },
    Mutant {
        name: "audio: the jack line falling with our own power-down taken as a jack change",
        file: "services/audio/src/lib.rs",
        from: "        // the emulator's model, 2026-10-18.)
        let _ = self.board.take_jack_edge();
",
        to: "        // the emulator's model, 2026-10-18.)
",
        check: Check::Host(&["-p", "rp-audio", "our_own_power_down_is_not_a_jack_change"]),
    },
    Mutant {
        name: "WM8750: VMID started on the slow 50k divider (S1 run 1: the jack read 'out' with headphones in)",
        file: "drivers/wm8750/src/lib.rs",
        from: "const PWR1_FAST_START: u16 = 0x1C0;",
        to: "const PWR1_FAST_START: u16 = 0x0C0;",
        check: Check::Emu(crate::emu::boot_loader_audio_s1),
    },
    Mutant {
        name: "lint: an inherited: tag without its source accepted (D1)",
        file: "xtask/src/policy.rs",
        from: "if !item.trim().is_empty() && !source.trim().is_empty() =>",
        to: "if !item.trim().is_empty() =>",
        check: Check::Host(&[
            "-p",
            "xtask",
            "inherited_tags_are_counted_and_malformed_ones_refused",
        ]),
    },
    Mutant {
        name: "audio: the codec's sleep switches I2S off too (the 645 ms trap on every wake)",
        file: "services/audio/src/lib.rs",
        from: "        self.board.pins_down();
        self.board.codec_power(false);
        // The jack line falls with the supply: our edge",
        to: "        self.link.stop();
        self.board.pins_down();
        self.board.codec_power(false);
        // The jack line falls with the supply: our edge",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "the_codec_sleeps_after_the_quiet_time_and_the_link_keeps_running",
        ]),
    },
    Mutant {
        name: "audio: the codec sleeps before the quiet time is up",
        file: "services/audio/src/lib.rs",
        from: "        if quiet < SLEEP_AFTER_MS {",
        to: "        if quiet < SLEEP_AFTER_MS / 2 {",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "the_codec_sleeps_after_the_quiet_time_and_the_link_keeps_running",
        ]),
    },
    Mutant {
        name: "audio: a touch's wake doesn't restart the quiet time",
        file: "services/audio/src/lib.rs",
        from: "    pub fn up(&mut self) -> Result<Powered, AudioError> {
        self.last_active = self.board.now_ms();
",
        to: "    pub fn up(&mut self) -> Result<Powered, AudioError> {
",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_wake_or_a_sound_restarts_the_quiet_time",
        ]),
    },
    Mutant {
        name: "audio: the codec's sleep leaves the speaker amplifier on",
        file: "services/audio/src/lib.rs",
        from: "        self.amp_off();
        let (r, bus) = codec.power_down();
        self.codec = Codec::Down(bus);
        self.board.pins_down();",
        to: "        let (r, bus) = codec.power_down();
        self.codec = Codec::Down(bus);
        self.board.pins_down();",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "the_codec_sleeps_after_the_quiet_time_and_the_link_keeps_running",
        ]),
    },
    Mutant {
        name: "audio: the jack line falling with the codec's sleep taken as a jack change",
        file: "services/audio/src/lib.rs",
        from: "        // The jack line falls with the supply: our edge, not a jack change.
        let _ = self.board.take_jack_edge();
",
        to: "",
        check: Check::Host(&["-p", "rp-audio", "our_own_sleep_is_not_a_jack_change"]),
    }, // Audio S2, the mixer (ADR-0032 R2-R4).
    Mutant {
        name: "mixer: a rewrite starts at the DMA position, not past the margin",
        file: "services/audio/src/mixer.rs",
        from: "let floor = align_up(self.hw + MARGIN, SNAP);",
        to: "let floor = align_up(self.hw, SNAP);",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_click_starts_within_the_margin_while_music_is_mixed_far_ahead",
        ]),
    },
    Mutant {
        name: "mixer: a rewrite doesn't put the chain's state back (limiter, filter, dither jump)",
        file: "services/audio/src/mixer.rs",
        from: "        self.state = chain::State::load(st);",
        to: "        let _ = chain::State::load(st);",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_rewrite_sounds_exactly_as_if_the_sound_had_been_known_all_along",
        ]),
    },
    Mutant {
        name: "mixer: rings sized without the DMA ring's lead (condition 4)",
        file: "services/audio/src/mixer.rs",
        from: "let out = RING_FRAMES + MARGIN + SNAP;",
        to: "let out = MARGIN + SNAP;",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "rings_are_sized_for_the_longest_lead_and_a_smaller_one_is_refused",
        ]),
    },
    Mutant {
        name: "mixer: a commit larger than the room taken (writes over unplayed sound)",
        file: "services/audio/src/mixer.rs",
        from: "if frames > s.cap - (s.write - s.release) {",
        to: "if false {",
        check: Check::Host(&["-p", "rp-audio", "a_program_can_only_spoil_its_own_stream"]),
    },
    Mutant {
        name: "mixer: a stale stream handle reaches the slot's next owner",
        file: "services/audio/src/mixer.rs",
        from: "            .get_mut(usize::from(id.slot))?\n            .as_mut()\n            .filter(|s| s.generation == id.generation)",
        to: "            .get_mut(usize::from(id.slot))?\n            .as_mut()",
        check: Check::Host(&["-p", "rp-audio", "a_program_can_only_spoil_its_own_stream"]),
    },
    Mutant {
        name: "mixer: the next wake timed from before the mixing, not after it",
        file: "services/audio/src/mixer.rs",
        from: "let ahead_ms = to_ms(self.mixed_to.saturating_sub(now + MARGIN));",
        to: "let ahead_ms = to_ms(self.mixed_to.saturating_sub(self.hw + MARGIN));",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_wake_that_takes_time_to_mix_still_wakes_again_in_time",
        ]),
    },
    Mutant {
        name: "mixer: mixes past where a stream's data ends (it runs dry at every long wake)",
        file: "services/audio/src/mixer.rs",
        from: "let target = align_down((self.hw + lead).min(data), BLOCK as u64)",
        to: "let target = align_down((self.hw + lead).min(data.max(u64::MAX)), BLOCK as u64)",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_ring_of_exactly_the_rule_never_runs_dry_across_interval_switches",
        ]),
    },
    Mutant {
        name: "mixer: a music player topping up in small writes taken as interactive (condition 13)",
        file: "services/audio/src/mixer.rs",
        from: "&& close && queued <= SHORT_QUEUE_MS",
        to: "&& close",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_player_trickling_small_writes_over_a_deep_queue_is_not_interactive",
        ]),
    },
    Mutant {
        name: "mixer: the short interval ends when a click's stream closes (no hold)",
        file: "services/audio/src/mixer.rs",
        from: ".is_some_and(|t| now_ms.wrapping_sub(t) < HOLD_MS);",
        to: ".is_some_and(|_| false);",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "the_short_interval_holds_after_a_click_stream_has_closed",
        ]),
    },
    Mutant {
        name: "mixer: the engine moving before the first wake counted late (streams re-anchored)",
        file: "services/audio/src/mixer.rs",
        from: "if core::mem::take(&mut self.starting) {",
        to: "if core::mem::take(&mut self.starting) && false {",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "the_engine_moving_before_the_first_wake_is_not_a_late_wake",
        ]),
    },
    Mutant {
        name: "chain: the limiter aims at full scale (a growing peak reaches the backstop)",
        file: "services/audio/src/chain.rs",
        from: "((i64::from(TARGET) << 16) / peak) as i32",
        to: "((i64::from(CEIL) << 16) / peak) as i32",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "two_notes_beating_into_an_overload_are_held_without_clipping",
        ]),
    },
    Mutant {
        name: "chain: -32768 taken for an overload (bit-exact sound limited)",
        file: "services/audio/src/chain.rs",
        from: "m.max(if s < 0 { -(s + 256) } else { s })",
        to: "m.max(s.abs())",
        check: Check::Host(&["-p", "rp-audio", "negative_full_scale_is_not_an_overload"]),
    },
    Mutant {
        name: "chain: the release rounds down and stalls short of unity (never bit-exact again)",
        file: "services/audio/src/chain.rs",
        from: "* i64::from(RELEASE) + 0xFFFF) >> 16) as i32;",
        to: "* i64::from(RELEASE)) >> 16) as i32;",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "an_overload_is_ridden_down_within_a_block_and_released_in_about_100_ms",
        ]),
    },
    Mutant {
        name: "chain: no hold before the release (a low note's next peak clips)",
        file: "services/audio/src/chain.rs",
        from: "        } else if self.hold > 0 {\n            self.hold -= 1;\n            from\n",
        to: "",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_low_note_overloaded_is_held_between_its_peaks",
        ]),
    },
    Mutant {
        name: "chain: dither on every sample (a lone 44.1 kHz stream not bit-exact, silence hisses)",
        file: "services/audio/src/chain.rs",
        from: "if s & 0xFF == 0 {",
        to: "if false {",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_16_bit_signal_at_unity_comes_out_bit_exact",
        ]),
    },
    Mutant {
        name: "chain: the speaker's filter on the headphones too",
        file: "services/audio/src/chain.rs",
        from: "        if speaker {",
        to: "        if true {",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "the_speaker_filter_cuts_the_bass_and_keeps_the_rest",
        ]),
    },
    Mutant {
        name: "roles: an old program's media paused on unplug (it can't be: muted instead, A2)",
        file: "services/audio/src/role.rs",
        from: "Role::Media if legacy => Unplug::Mute,",
        to: "Role::Media if legacy => Unplug::Pause,",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "unplugging_pauses_media_mutes_old_media_and_moves_the_rest",
        ]),
    },
    Mutant {
        name: "mixer: the block path taken across a volume ramp (a step instead of the ramp)",
        file: "services/audio/src/mixer.rs",
        from: "if !(at + RAMP <= b || at + 1 >= end) {",
        to: "if false {",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "the_block_path_gives_exactly_what_the_frame_path_gives",
        ]),
    },
    Mutant {
        name: "mixer: the block path reads before the ring's oldest frame",
        file: "services/audio/src/mixer.rs",
        from: "if i0 < self.write.saturating_sub(self.cap) + left {",
        to: "if false {",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "the_block_path_gives_exactly_what_the_frame_path_gives",
        ]),
    },
    Mutant {
        name: "resampler: downsampling's scale left out of the coefficients (sums overflow 32 bits)",
        file: "services/audio/src/resample.rs",
        from: "    if r.gain == 32_768 {
        c
    } else {
        ((c as i64 * r.gain) >> 15) as i32
    }",
        to: "    c",
        check: Check::Host(&["-p", "rp-audio", "the_sum_fits_32_bits"]),
    },
    Mutant {
        name: "resampler: whole-number steps read the wrong table points (the stride's start)",
        file: "services/audio/src/resample.rs",
        from: "            ((OUT - phase) * p / OUT) as usize,
        ))",
        to: "            ((OUT - phase) * p / OUT) as usize + 1,
        ))",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "the_48k_table_and_the_strides_are_the_computed_coefficients",
        ]),
    },
    Mutant {
        name: "resampler: 48 kHz's right wing read from the wrong row",
        file: "services/audio/src/resample.rs",
        from: "Some((&T48[k], &T48[147 - k]))",
        to: "Some((&T48[k], &T48[k]))",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "the_block_path_gives_exactly_what_the_frame_path_gives",
        ]),
    },
    Mutant {
        name: "resampler: 48 kHz not scaled down (6% loud, overshoot)",
        file: "services/audio/src/resample.rs",
        from: "OUT as i64 * 32_768 / rate as i64",
        to: "32_768",
        check: Check::Host(&["-p", "rp-audio", "a_sine_converts_cleanly_from_every_rate"]),
    },
    // Quality audit C5 (2026-10-19): guards of the parsers that read
    // untrusted input, each with the test that must catch it.
    Mutant {
        name: "loadproto: a frame larger than the buffer stored anyway",
        file: "boot/loadproto/src/lib.rs",
        from: "if self.len > dest.len() {",
        to: "if false {",
        check: Check::Host(&[
            "-p",
            "rp-loadproto",
            "too_big_is_refused_before_any_byte_is_stored",
        ]),
    },
    Mutant {
        name: "loadproto: a damaged payload taken (the CRC not compared)",
        file: "boot/loadproto/src/lib.rs",
        from: "if crc32(&dest[..f.len]) == rx.expected_crc() {",
        to: "if true {",
        check: Check::Host(&["-p", "rp-loadproto", "a_damaged_payload_is_refused"]),
    },
    Mutant {
        name: "loadproto: a file name longer than its frame taken",
        file: "boot/loadproto/src/lib.rs",
        from: "if n == 0 || n > NAME_MAX || 1 + n > self.len {",
        to: "if n == 0 || n > NAME_MAX {",
        check: Check::Host(&[
            "-p",
            "rp-loadproto",
            "a_file_frame_with_a_bad_name_is_refused_by_the_split",
        ]),
    },
    Mutant {
        name: "loadproto: base64 data after its padding taken",
        file: "boot/loadproto/src/b64.rs",
        from: "if pad > 0 && c != b'=' {",
        to: "if false {",
        check: Check::Host(&["-p", "rp-loadproto", "rubbish_is_refused_not_misread"]),
    },
    Mutant {
        name: "loadproto: base64 decoded past the end of its buffer",
        file: "boot/loadproto/src/b64.rs",
        from: "if o + bytes > out.len() {",
        to: "if false {",
        check: Check::Host(&["-p", "rp-loadproto", "rubbish_is_refused_not_misread"]),
    },
    Mutant {
        name: "pe: more sections than the table holds taken",
        file: "kernel/pe/src/lib.rs",
        from: "if count > MAX_SECTIONS {",
        to: "if false {",
        check: Check::Host(&["-p", "rp-kernel-pe", "mutated_images_never_panic"]),
    },
    Mutant {
        name: "pe: sections out of order or overlapping taken",
        file: "kernel/pe/src/lib.rs",
        from: "if s.rva < prev_end || end > self.image_size.next_multiple_of(PAGE) {",
        to: "if end > self.image_size.next_multiple_of(PAGE) {",
        check: Check::Host(&["-p", "rp-kernel-pe", "rejects_malformed_images"]),
    },
    Mutant {
        name: "pe: an image for another CPU taken (x86)",
        file: "kernel/pe/src/lib.rs",
        from: "if u16_at(headers, opt)? != 0x10B || !matches!(machine, MACHINE_ARM | MACHINE_THUMB) {",
        to: "if u16_at(headers, opt)? != 0x10B {",
        check: Check::Host(&["-p", "rp-kernel-pe", "rejects_malformed_images"]),
    },
    Mutant {
        name: "lz4: a match before the start of the output taken (the slow path)",
        file: "kernel/lz4/src/lib.rs",
        from: "if offset == 0 || offset > o {",
        to: "if offset == 0 {",
        check: Check::Host(&["-p", "rp-lz4", "rejects_hostile_blocks"]),
    },
    Mutant {
        name: "lz4: a match before the start of the output taken (the fast path)",
        file: "kernel/lz4/src/lib.rs",
        from: "if offset == 0 || offset > o2 {",
        to: "if offset == 0 {",
        check: Check::Host(&["-p", "rp-lz4", "hostile_input_never_panics"]),
    },
    Mutant {
        name: "lz4: a match running past the output's end taken",
        file: "kernel/lz4/src/lib.rs",
        from: "        if end > dst.len() {\n            return Err(Error::Overflow);\n        }\n        copy_match",
        to: "        copy_match",
        check: Check::Host(&["-p", "rp-lz4", "rejects_hostile_blocks"]),
    },
    Mutant {
        name: "lz4: the fast path's room check short of its largest sequence",
        file: "kernel/lz4/src/lib.rs",
        from: "o + 14 + 18 <= dst.len()",
        to: "o + 14 <= dst.len()",
        check: Check::Host(&["-p", "rp-lz4", "hostile_input_never_panics"]),
    },
    Mutant {
        name: "ipc: a handle granted with rights its holder doesn't have",
        file: "kernel/ipc/src/lib.rs",
        from: "if !e.rights().contains(Rights::GRANT) || !e.rights().contains(rights) {",
        to: "if !e.rights().contains(Rights::GRANT) {",
        check: Check::Host(&["-p", "rp-kernel-ipc", "rights_never_grow"]),
    },
    Mutant {
        name: "ipc: a call through a handle without the CALL right",
        file: "kernel/ipc/src/lib.rs",
        from: "        if !e.rights().contains(Rights::CALL) {
            return Err(Error::NoRight);
        }
",
        to: "",
        check: Check::Host(&["-p", "rp-kernel-ipc", "rights_never_grow"]),
    },
    Mutant {
        name: "ipc: an old handle reaches the port that took its place (no generation check)",
        file: "kernel/ipc/src/lib.rs",
        from: "if p.generation != e.generation() {",
        to: "if false {",
        check: Check::Host(&[
            "-p",
            "rp-kernel-ipc",
            "an_exited_server_closes_its_ports_and_reuse_cant_be_reached_by_old_handles",
        ]),
    },
    Mutant {
        name: "ipc: calls nest without bound",
        file: "kernel/ipc/src/lib.rs",
        from: "if d >= MAX_DEPTH {",
        to: "if false {",
        check: Check::Host(&["-p", "rp-kernel-ipc", "nesting_is_bounded_and_unwinds"]),
    },
    Mutant {
        name: "ipc: a handle table grows past its quota of pages",
        file: "kernel/ipc/src/lib.rs",
        from: "if p >= HANDLE_PAGES {",
        to: "if false {",
        check: Check::Host(&["-p", "rp-kernel-ipc", "tables_are_bounded"]),
    },
    Mutant {
        name: "fat guard: the MBR written with a new partition table",
        file: "kernel/fat/src/guard.rs",
        from: "return lba == 0 && s[PARTITIONS] == self.partitions;",
        to: "return lba == 0;",
        check: Check::Host(&[
            "-p",
            "rp-fat",
            "a_format_or_a_new_partition_table_is_refused",
        ]),
    },
    Mutant {
        name: "fat guard: the boot sector reformatted (its geometry not compared)",
        file: "kernel/fat/src/guard.rs",
        from: "return s[GEOMETRY] == self.geometry;",
        to: "return true;",
        check: Check::Host(&[
            "-p",
            "rp-fat",
            "a_format_or_a_new_partition_table_is_refused",
        ]),
    },
    Mutant {
        name: "fat guard: a boot file's chain relinked",
        file: "kernel/fat/src/guard.rs",
        from: "Some(next) => v == next,",
        to: "Some(_) => true,",
        check: Check::Host(&["-p", "rp-fat", "a_boot_files_chain_cannot_be_relinked"]),
    },
    Mutant {
        name: "fat guard: a boot file's chain end moved",
        file: "kernel/fat/src/guard.rs",
        from: "Some(FAT_MASK) => v >= EOC_MIN,",
        to: "Some(FAT_MASK) => true,",
        check: Check::Host(&["-p", "rp-fat", "a_boot_files_chain_end_cannot_be_moved"]),
    },
    Mutant {
        name: "fat guard: a boot file's data written over",
        file: "kernel/fat/src/guard.rs",
        from: "if files.iter().any(|f| f.holds(cluster)) {",
        to: "if false {",
        check: Check::Host(&[
            "-p",
            "rp-fat",
            "a_protected_sector_inside_a_long_write_is_caught",
        ]),
    },
    Mutant {
        name: "fat guard: a boot file renamed (its directory entry's name not compared)",
        file: "kernel/fat/src/guard.rs",
        from: "e[..12] == f.name_attr",
        to: "true",
        check: Check::Host(&[
            "-p",
            "rp-fat",
            "boot_files_cannot_be_deleted_replaced_or_renamed",
        ]),
    },
    Mutant {
        name: "mm: a frame range added over frames already owned",
        file: "kernel/mm/src/frames.rs",
        from: "if (start..end).any(|f| self.owns(f)) {",
        to: "if false {",
        check: Check::Host(&[
            "-p",
            "rp-kernel-mm",
            "add_range_rejects_overlap_and_overflow",
        ]),
    },
    Mutant {
        name: "mm: a frame freed twice (or one it never handed out)",
        file: "kernel/mm/src/frames.rs",
        from: "        if t & USED == 0 {
            return Err(Error::NotAllocated);
        }
",
        to: "",
        check: Check::Host(&["-p", "rp-kernel-mm", "rejects_double_and_interior_frees"]),
    },
    Mutant {
        name: "mm: a small page mapped over an existing mapping",
        file: "kernel/mm/src/pagetable.rs",
        from: "if t[i] != 0 {",
        to: "if false {",
        check: Check::Host(&["-p", "rp-kernel-mm", "never_overwrites_a_mapping"]),
    },
    Mutant {
        name: "mm: a section mapped over an existing mapping",
        file: "kernel/mm/src/pagetable.rs",
        from: "if *l1 != 0 {",
        to: "if false {",
        check: Check::Host(&["-p", "rp-kernel-mm", "sections_for_the_kernel_direct_map"]),
    },
    Mutant {
        name: "mm: a large page mapped over existing small pages",
        file: "kernel/mm/src/pagetable.rs",
        from: "if t[first..first + 16].iter().any(|&e| e != 0) {",
        to: "if false {",
        check: Check::Host(&[
            "-p",
            "rp-kernel-mm",
            "a_large_page_never_lands_on_small_pages",
        ]),
    },
    Mutant {
        name: "mm: a process given the kernel's or the shared domain",
        file: "kernel/mm/src/domains.rs",
        from: "for c in FIRST..=LAST {",
        to: "for c in 0..=LAST {",
        check: Check::Host(&[
            "-p",
            "rp-kernel-mm",
            "reserved_domains_never_go_to_processes",
        ]),
    },
    Mutant {
        name: "usb-msc: a read or write past the card's end (or wrapping) let through",
        file: "drivers/usb-msc/src/lib.rs",
        from: "if lba.checked_add(count).is_none_or(|end| end > blocks) {",
        to: "if false {",
        check: Check::Host(&[
            "-p",
            "rp-usb-msc",
            "out_of_range_and_wrapping_requests_never_touch_the_card",
        ]),
    },
    Mutant {
        name: "usb-msc: a transfer the host asked for differently (length, direction) carried out",
        file: "drivers/usb-msc/src/lib.rs",
        from: "if u64::from(self.host_len) != bytes || (count > 0 && self.host_in == write) {",
        to: "if false {",
        check: Check::Host(&[
            "-p",
            "rp-usb-msc",
            "a_mismatched_transfer_is_refused_without_card_access",
        ]),
    },
    Mutant {
        name: "usb-msc: a write to a write-protected unit carried out",
        file: "drivers/usb-msc/src/lib.rs",
        from: "if write && read_only {",
        to: "if false {",
        check: Check::Host(&[
            "-p",
            "rp-usb-msc",
            "write_protection_is_honoured_and_reported",
        ]),
    },
    Mutant {
        name: "usb-msc: a command block without its signature (or the wrong length) taken",
        file: "drivers/usb-msc/src/lib.rs",
        from: "if n != CBW_LEN || u32::from_le_bytes([c[0], c[1], c[2], c[3]]) != CBW_SIGNATURE {",
        to: "if n != CBW_LEN {",
        check: Check::Host(&[
            "-p",
            "rp-usb-msc",
            "malformed_cbws_are_ignored_and_the_next_command_works",
        ]),
    },
    Mutant {
        name: "cf: a transfer past the card's end sent to it",
        file: "drivers/cf/src/lib.rs",
        from: "Some(end) if end <= self.sectors => Ok(n),",
        to: "Some(_) => Ok(n),",
        check: Check::Host(&[
            "-p",
            "rp-drv-cf",
            "transfers_past_the_end_are_refused_before_the_card_sees_them",
        ]),
    },
    Mutant {
        name: "cf: a card still in reset accessed (its READY line not waited for)",
        file: "drivers/cf/src/lib.rs",
        from: "        if lines.ready() {
            return Ok(ResetSeen {",
        to: "        if true {
            return Ok(ResetSeen {",
        check: Check::Host(&[
            "-p",
            "rp-drv-cf",
            "a_card_that_never_comes_out_of_reset_is_never_accessed",
        ]),
    },
    Mutant {
        name: "xtask: the emulator's clean-up ends another run's emulator (a port prefix)",
        file: "xtask/src/emu.rs",
        from: "'qemu-system-arm .*-monitor tcp:127.0.0.1:{monitor_port}[,]'",
        to: "'qemu-system-arm .*-monitor tcp:127.0.0.1:{monitor_port}'",
        check: Check::Host(&["-p", "xtask", "cleanup_ends_only_this_runs_processes"]),
    },
    Mutant {
        name: "xtask: the emulator's clean-up leaves an emulator that ignores SIGTERM (C15)",
        file: "xtask/src/emu.rs",
        from: "pkill -KILL -f {mine}; {wait}; ",
        to: "",
        check: Check::Host(&["-p", "xtask", "cleanup_ends_only_this_runs_processes"]),
    },
    Mutant {
        name: "xtask: an emulator check without its own recorder plays sound on the PC",
        file: "xtask/src/emu.rs",
        from: "    if !extra.contains(&\"-audiodev\") {",
        to: "    if false {",
        check: Check::Host(&["-p", "xtask", "the_emulator_never_plays_sound_out_loud"]),
    },
    Mutant {
        name: "xtask serve: an unknown path answered with a kit file",
        file: "xtask/src/serve.rs",
        from: ".map_or(Route::NotFound, |f| Route::File(f))",
        to: ".map_or(Route::File(FILES[0]), |f| Route::File(f))",
        check: Check::Host(&["-p", "xtask", "only_the_kit_files_are_served"]),
    },
    Mutant {
        name: "fat: a FAT12/16 volume (a fixed root directory) taken for FAT32",
        file: "kernel/fat/src/lib.rs",
        from: "            || le16(&buf, 17) != 0 // FAT32 has no fixed root directory
",
        to: "",
        check: Check::Host(&["-p", "rp-fat", "rejects_what_is_not_fat32"]),
    },
    Mutant {
        name: "fat: a new name ending in a dot taken (Windows can't open it)",
        file: "kernel/fat/src/lib.rs",
        from: "        || name.ends_with(b\".\")
",
        to: "",
        check: Check::Host(&["-p", "rp-fat", "refuses_names_fat_does_not_allow"]),
    },
    Mutant {
        name: "cab: an over-subscribed Huffman code taken (its codes overlap)",
        file: "personality/ce/cab/src/inflate.rs",
        from: "            if left < 0 {",
        to: "            if false {",
        check: Check::Host(&["-p", "rp-cab", "over_subscribed_code_is_refused"]),
    },
    Mutant {
        name: "cab: a literal written past the output limit",
        file: "personality/ce/cab/src/inflate.rs",
        from: "            if *pos >= end {",
        to: "            if *pos >= window.len() {",
        check: Check::Host(&["-p", "rp-cab", "output_limit_is_enforced"]),
    },
    Mutant {
        name: "udc: a configured device (or a PC still retrying) unplugged",
        file: "drivers/pxa-udc/src/unplug.rs",
        from: "        if configured || resets != self.resets {",
        to: "        if resets != self.resets {",
        check: Check::Host(&[
            "-p",
            "rp-drv-pxa-udc",
            "a_pc_that_talks_or_a_configured_device_is_never_unplugged",
        ]),
    },
    Mutant {
        name: "udc: the host controller run without our communication area (it writes memory there)",
        file: "drivers/pxa-udc/src/unplug.rs",
        from: "    uhc.write32(HCCA, core::ptr::addr_of!(hcca) as usize as u32);",
        to: "    let _ = &hcca;",
        check: Check::Host(&[
            "-p",
            "rp-drv-pxa-udc",
            "the_host_controller_never_runs_without_our_communication_area",
        ]),
    },
    Mutant {
        name: "legacysync: a cab name with `..` (outside the INI's folder) used on the PC",
        file: "pc/legacysync/src/stage.rs",
        from: "            .any(|p| p.is_empty() || *p == \".\" || *p == \"..\")",
        to: "            .any(|p| p.is_empty() || *p == \".\")",
        check: Check::Host(&[
            "-p",
            "rp-legacysync",
            "cab_names_escaping_the_ini_folder_are_refused",
        ]),
    },
    Mutant {
        name: "udc: no retry for a PC silent after the plug-in (the loader waits forever)",
        file: "drivers/pxa-udc/src/unplug.rs",
        from: "        self.wait_ms = (self.wait_ms * 2).min(Self::MAX_MS);
        true",
        to: "        self.wait_ms = (self.wait_ms * 2).min(Self::MAX_MS);
        false",
        check: Check::Emu(crate::emu::boot_loader_silent_pc),
    },
    Mutant {
        name: "today: GPIO24 driven as the touch chip's select while still SSP1's frame (every tap reads 0, as on the X51v)",
        file: "shell/today/src/main.rs",
        from: "        gpio.borrow_mut().set_function(TS_CS, 0);",
        to: "",
        check: Check::Emu(crate::emu::boot_loader_today),
    },
    Mutant {
        name: "sdcard: a card echoing the wrong CMD8 check pattern taken as v2",
        file: "drivers/sdcard/src/lib.rs",
        from: "Ok(r) if r[0] & 0xFFF == CMD8_ARG => true,",
        to: "Ok(_r) => true,",
        check: Check::Host(&["-p", "rp-drv-sdcard", "bad_cmd8_echo_is_rejected"]),
    },
    Mutant {
        name: "dma chain: a chunk longer than a descriptor's length field taken",
        file: "hal/pxa27x/src/dmachain.rs",
        from: "            || self.chunk > 0x1FFF
",
        to: "",
        check: Check::Host(&["-p", "rp-hal-pxa27x", "invalid_shapes_are_refused"]),
    },
    Mutant {
        name: "dma chain: a destination step other than the chunk (or 0) taken",
        file: "hal/pxa27x/src/dmachain.rs",
        from: "            || (self.dst_step != 0 && self.dst_step != self.chunk)
",
        to: "",
        check: Check::Host(&["-p", "rp-hal-pxa27x", "invalid_shapes_are_refused"]),
    },
    // The audio ring's end (ADR-0032 A6, dmaring).
    Mutant {
        name: "ring: old STOP cleared before the new one is set (ADR-0032 A6)",
        file: "hal/pxa27x/src/dmaring.rs",
        from: "        eng.link(k as usize, self.link(k) | STOP);\n        if let Some(j) = self.end {\n            eng.link(j as usize, self.link(j));\n        }",
        to: "        if let Some(j) = self.end {\n            eng.link(j as usize, self.link(j));\n        }\n        eng.link(k as usize, self.link(k) | STOP);",
        check: Check::Host(&[
            "-p",
            "rp-hal-pxa27x",
            "every_frame_played_is_the_one_written_for_its_time_or_silence",
        ]),
    },
    Mutant {
        name: "ring: the race not waited for (left to stop short) (ADR-0032 A6)",
        file: "hal/pxa27x/src/dmaring.rs",
        from: "            Look::Stopping(c) if c == k => return RingEnd::Running,",
        to: "            Look::Stopping(_) => return RingEnd::Running,",
        check: Check::Host(&["-p", "rp-hal-pxa27x", "dmaring"]),
    },
    Mutant {
        name: "ring: an engine stopped just after the new end started again (a lap) (ADR-0032 A6)",
        file: "hal/pxa27x/src/dmaring.rs",
        from: "        if r == if k + 1 == self.pieces { 0 } else { k + 1 } {",
        to: "        if false {",
        check: Check::Host(&["-p", "rp-hal-pxa27x", "dmaring"]),
    },
    Mutant {
        name: "ring: the ring's end read as past it (ADR-0032 A6)",
        file: "hal/pxa27x/src/dmaring.rs",
        from: "        if at == self.len {\n            Some(0)\n        } else if at < self.len {",
        to: "        if at < self.len {",
        check: Check::Host(&["-p", "rp-hal-pxa27x", "dmaring"]),
    },
    Mutant {
        name: "ring: an end in the engine's own piece taken (goes round) (ADR-0032 A6)",
        file: "hal/pxa27x/src/dmaring.rs",
        from: "            Look::Moving(c) if c == k => {",
        to: "            Look::Moving(c) if c == k && false => {",
        check: Check::Host(&["-p", "rp-hal-pxa27x", "dmaring"]),
    },
    Mutant {
        name: "ring: a stop mid-piece taken as a stop at a piece's end (ADR-0032 A6)",
        file: "hal/pxa27x/src/dmaring.rs",
        from: "            if !at.is_multiple_of(self.piece) {\n                return Look::Bad;\n            }\n            return Look::Stopped(at / self.piece);",
        to: "            return Look::Stopped(at / self.piece);",
        check: Check::Host(&["-p", "rp-hal-pxa27x", "dmaring"]),
    },
    // The mixer's end of the ring (ADR-0032 A6 step 2, audit E3/E4).
    Mutant {
        name: "mixer: the end's rule dropped (lead < ring - one piece), the old cap back (ADR-0032 A6)",
        file: "services/audio/src/mixer.rs",
        from: "            .min(self.max_end())\n",
        to: "            .min(self.hw + RING_FRAMES - MARGIN)\n",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_ring_cut_in_larger_pieces_still_keeps_the_end_s_rule",
        ]),
    },
    Mutant {
        name: "mixer: the rest of the end's piece not cleared (audit E4) (ADR-0032 A6)",
        file: "services/audio/src/mixer.rs",
        from: "        self.clear(from, to, dma);\n        self.set_end(dma, from);",
        to: "        let _ = to;\n        self.set_end(dma, from);",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_shortening_rewrite_then_a_stall_plays_no_stale_tail",
        ]),
    },
    Mutant {
        name: "mixer: after a stop the sound carries on past the margin, not where the engine stopped (ADR-0032 A6)",
        file: "services/audio/src/mixer.rs",
        from: "            self.mixed_to = if self.ended {\n                self.piece_end(at)\n            } else {\n                floor\n            };",
        to: "            self.mixed_to = floor;",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_stall_longer_than_the_ring_is_silence_never_a_replay",
        ]),
    },
    Mutant {
        name: "mixer: the ring loops until the first wake's end (an engine faster than the mixer laps it) (ADR-0032 A6)",
        file: "services/audio/src/mixer.rs",
        from: "        self.set_end(dma, align_up(self.hw + MARGIN, self.piece));\n",
        to: "",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_ring_started_with_sound_waiting_has_its_end_from_the_start",
        ]),
    },
    Mutant {
        name: "mixer: the end's piece not kept past the engine by the margin (ADR-0032 A6)",
        file: "services/audio/src/mixer.rs",
        from: "            if align_down(self.mixed_to, self.piece) >= now + MARGIN || next > self.max_end() {",
        to: "            if true {",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_shortening_rewrite_on_a_slow_cpu_keeps_the_end_ahead_of_the_engine",
        ]),
    },
    // The end moving during a long mix (ADR-0032 A6).
    Mutant {
        name: "mixer: the end moved only after the whole mix (the race waits for a long mix) (ADR-0032 A6)",
        file: "services/audio/src/mixer.rs",
        from: "            if self.ended
                && b.is_multiple_of(self.piece)
",
        to: "            if false
                && b.is_multiple_of(self.piece)
",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "the_race_on_a_slow_cpu_costs_about_two_pieces_mixing_not_the_whole_mix",
        ]),
    },
    // The end moved mid-mix only to a stopped engine (ADR-0032 A6).
    Mutant {
        name: "mixer: the end moved under a playing engine with every piece mixed (the mixer tied to the engine) (ADR-0032 A6)",
        file: "services/audio/src/mixer.rs",
        from: "                && b > self.piece_end(self.end_frame)\n                && dma.stopped()\n",
        to: "                && b > self.piece_end(self.end_frame)\n",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_restart_does_not_tie_the_mixer_to_the_engine",
        ]),
    },
    // After a stall: the write pattern and the dry count (ADR-0032 A6 run 2).
    Mutant {
        name: "mixer: a refill growing its queue counted as a game's small writes (music taken for interactive after a stall) (ADR-0032 A6 run 2)",
        file: "services/audio/src/mixer.rs",
        from: "        let level = queued <= s.last_queued_ms + ms / 2;\n",
        to: "        let level = true;\n",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "after_a_stall_music_stays_music_and_carries_on_in_one_gap",
        ]),
    },
    Mutant {
        name: "mixer: a dry stretch counted when mixed, not when played (a refilled stretch reported dry) (ADR-0032 A6 run 2)",
        file: "services/audio/src/mixer.rs",
        from: "            if !s.dry_seen && s.starved_at.is_some_and(|at| at < hw) {",
        to: "            if !s.dry_seen && s.starved_at.is_some() {",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "after_a_stall_music_stays_music_and_carries_on_in_one_gap",
        ]),
    },
    Mutant {
        name: "mixer: a stream that really runs dry never counted (ADR-0032 A6 run 2)",
        file: "services/audio/src/mixer.rs",
        from: "                s.dry_seen = true;\n                self.stats.starved += 1;",
        to: "                s.dry_seen = true;",
        check: Check::Host(&[
            "-p",
            "rp-audio",
            "a_stream_that_runs_dry_carries_on_where_it_stopped",
        ]),
    },
];
/// Puts a file back when dropped.
struct Restore {
    path: PathBuf,
    original: String,
}

impl Drop for Restore {
    fn drop(&mut self) {
        if let Err(e) = std::fs::write(&self.path, &self.original) {
            eprintln!(
                "    !! could not restore {}: {e}; run `git checkout -- {}`",
                self.path.display(),
                self.path.display()
            );
        }
    }
}

fn unchanged_in_git(root: &Path, file: &str) -> Result<(), String> {
    let out = Command::new("git")
        .current_dir(root)
        .args(["status", "--porcelain", "--", file])
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.stdout.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{file} has uncommitted changes: commit or stash them first"
        ))
    }
}

/// Runs `cargo test <args>`: whether it passed, and how many tests ran.
fn host_tests(root: &Path, args: &[&str]) -> Result<(bool, u32), String> {
    let out = Command::new("cargo")
        .current_dir(root)
        .arg("test")
        .args(args)
        .output()
        .map_err(|e| format!("cargo: {e}"))?;
    let (stdout, stderr) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    outcome(out.status.success(), &stdout, &stderr)
}

/// Whether the tests passed, and how many ran; an error if the code didn't
/// build (a mutant that doesn't compile kills nothing: its text needs
/// fixing, found 2026-10-19 when one had gone stale that way).
fn outcome(success: bool, stdout: &str, stderr: &str) -> Result<(bool, u32), String> {
    if stderr.contains("could not compile") || stderr.contains("error[E") {
        return Err("the code doesn't build".into());
    }
    let ran = stdout
        .lines()
        .filter_map(|l| l.strip_prefix("test result: "))
        .filter_map(|l| l.split(". ").nth(1)?.split(' ').next()?.parse::<u32>().ok())
        .sum();
    Ok((success, ran))
}

#[test]
fn a_mutant_that_does_not_build_is_not_a_kill() {
    let failed = "test result: FAILED. 3 passed; 1 failed; 0 ignored";
    assert_eq!(outcome(false, failed, ""), Ok((false, 3)));
    assert!(
        outcome(
            false,
            "",
            "error[E0308]: mismatched types
error: could not compile `x`"
        )
        .is_err()
    );
    assert_eq!(
        outcome(true, "test result: ok. 2 passed; 0 failed", ""),
        Ok((true, 2))
    );
}

/// Runs every host mutant, and the emulator ones with `emu`.
pub fn run(root: &Path, emu: bool) -> Result<(), String> {
    let mut survived = Vec::new();
    let mut ran = 0;
    for m in MUTANTS {
        if matches!(m.check, Check::Emu(_)) && !emu {
            eprintln!("    skipped (needs --emu): {}", m.name);
            continue;
        }
        unchanged_in_git(root, m.file)?;
        let path = root.join(m.file);
        // The check must pass on the real code and actually run tests, or a
        // "kill" means nothing (a filter matching no test name, a suite
        // already failing).
        if let Check::Host(args) = m.check {
            match host_tests(root, args)? {
                (true, n) if n > 0 => {}
                (true, _) => return Err(format!("`{}`: its check runs no tests", m.name)),
                (false, _) => {
                    return Err(format!("`{}`: its check fails without the mutant", m.name));
                }
            }
        }
        let original = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", m.file))?;
        if original.matches(m.from).count() != 1 {
            return Err(format!(
                "stale mutant `{}`: its text isn't in {} exactly once",
                m.name, m.file
            ));
        }
        let guard = Restore {
            path: path.clone(),
            original: original.clone(),
        };
        std::fs::write(&path, original.replacen(m.from, m.to, 1))
            .map_err(|e| format!("{}: {e}", m.file))?;
        let killed = match m.check {
            Check::Host(args) => {
                !host_tests(root, args)
                    .map_err(|e| format!("mutant `{}`: {e} (fix its text)", m.name))?
                    .0
            }
            Check::Emu(f) => crate::cross().is_err() || f(root).is_err(),
        };
        drop(guard);
        if matches!(m.check, Check::Emu(_)) {
            // Never leave images built from a mutant in `target/`, where
            // `cargo xtask load` could find them: rebuild from the real code.
            crate::cross().map_err(|e| format!("rebuilding after `{}`: {e}", m.name))?;
        }
        ran += 1;
        if killed {
            eprintln!("    killed: {}", m.name);
        } else {
            eprintln!("    SURVIVED: {}", m.name);
            survived.push(m.name);
        }
    }
    if survived.is_empty() {
        eprintln!("    all {ran} mutants killed");
        Ok(())
    } else {
        Err(format!(
            "{} mutant(s) survived: {}",
            survived.len(),
            survived.join("; ")
        ))
    }
}

#[test]
fn every_mutant_still_matches_the_code_once() {
    // A refactor that moves the text makes the mutant stale: caught here, in
    // ordinary `cargo test`, not only when someone runs the mutants.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for m in MUTANTS {
        let text = std::fs::read_to_string(root.join(m.file)).unwrap();
        assert_eq!(text.matches(m.from).count(), 1, "stale mutant: {}", m.name);
        assert_ne!(m.from, m.to, "{}", m.name);
    }
}
