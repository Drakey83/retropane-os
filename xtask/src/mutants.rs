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
    Mutant {
        name: "today: an idle sleep past the run limit (the watchdog reset the device)",
        file: "shell/today/src/main.rs",
        from: "            let wait_ms = wait_ms.min(rt::check_in_every_ms()).max(1);",
        to: "            let wait_ms = wait_ms.max(1);",
        check: Check::Emu(crate::emu::boot_loader_today),
    },
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
        check: Check::Emu(crate::emu::boot_loader_cf_alone),
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
    Mutant {
        name: "CF card not attached on insertion (the PC sees an empty reader until `cf`)",
        file: "boot/loader/src/main.rs",
        from: "                    cf_attach(&mut cf, &mut gpio, cpld.as_mut(), false);",
        to: "",
        check: Check::Emu(crate::emu::boot_loader_cf_alone),
    },
    Mutant {
        name: "CF card in at start-up attached only after the drive is offered (the PC's \"USB Drive\" with no size)",
        file: "boot/loader/src/main.rs",
        from: "        if cf_in {\n            cf_attach(&mut cf, &mut gpio, cpld.as_mut(), false);\n        }\n",
        to: "",
        check: Check::Emu(crate::emu::boot_loader_cf_alone),
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
        name: "CF slot left powered after the card is removed",
        file: "boot/loader/src/main.rs",
        from: "            cpld.set_cf_power(false, &mut BusyWait);\n            self.powered = false;",
        to: "            self.powered = false;",
        check: Check::Emu(crate::emu::boot_loader_cf_alone),
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
        name: "loader: a new loader sent as an image is reported as a hang (crashlog.txt overwritten)",
        file: "boot/loader/src/main.rs",
        from: "                        devices::log_mark_handover();",
        to: "",
        check: Check::Emu(crate::emu::boot_loader_usb_alone),
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
    let ran = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("test result: "))
        .filter_map(|l| l.split(". ").nth(1)?.split(' ').next()?.parse::<u32>().ok())
        .sum();
    Ok((out.status.success(), ran))
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
            Check::Host(args) => !host_tests(root, args)?.0,
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
