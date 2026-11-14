//! `cargo xtask emu-apps`: the organiser's app programs (version 2,
//! docs/design/app-scope.md) on the image in the emulator.
//!
//! The card holds what `cargo xtask load` puts on the X51v's: the icon
//! pack, the clock file, and the five programs as PE files (`APCAL` ...,
//! rp_today::program::PROGRAMS). The image is driven through Start to each
//! app in turn: it reads the program's file, lays it out relocated in its
//! program slot, calls its entry and hosts it; each app's first screen is
//! compared pixel-exact with the PC's model, which opens the same program
//! from its library ([`model_program`]). Home between apps releases the
//! one open. The run log must name each program read and loaded.
//!
//! The card's clock is the previews' (noon, Monday 1 January 2024).

#[allow(clippy::wildcard_imports)]
use super::*;
use rp_shell::{Input, Key, Shell};
use rp_today::organiser;

/// The previews' clock in local minutes from 1970: 12:00 on Monday
/// 1 January 2024 (19,723 days).
const NOON: i64 = 19_723 * 24 * 60 + 12 * 60;
/// How long a tap is held and the pause after it, in the image's time.
const HOLD_MS: u32 = 150;
const GAP_MS: u32 = 250;

/// The programs' crates, in `rp_today::program::PROGRAMS`' order.
const CRATES: [&str; 5] = [
    "rp-app-calendar",
    "rp-app-contacts",
    "rp-app-tasks",
    "rp-app-notes",
    "rp-app-calculator",
];

/// A card file's parts as the image reads them (`<stem>A.PAK`, ..., and
/// an empty one after the last).
fn parts(stem: &str, data: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = data
        .chunks(rp_today::PACK_PART)
        .enumerate()
        .map(|(p, c)| {
            (
                format!("{stem}{}.PAK", (b'A' + p as u8) as char),
                c.to_vec(),
            )
        })
        .collect();
    out.push((
        format!("{stem}{}.PAK", (b'A' + out.len() as u8) as char),
        Vec::new(),
    ));
    out
}

/// The card: the default icon pack, the clock, the five programs and the
/// organiser's files (an empty store). Where its partition starts.
fn card(release: &Path) -> Result<(Disk, u32), String> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let pack = today_output(release, "rp-icons0.pak").unwrap_or_default();
    if !pack.is_empty() {
        files.extend(parts("RPIC0", &pack));
    }
    files.extend(parts("RPTIM", &rp_today::clock::card_time(NOON, 0)));
    for (krate, (_, stem)) in CRATES.iter().zip(rp_today::program::PROGRAMS) {
        let pe = release.join(format!("{krate}.pe"));
        let bytes = std::fs::read(&pe).map_err(|e| format!("{}: {e}", pe.display()))?;
        files.extend(parts(&String::from_utf8_lossy(&stem), &bytes));
    }
    let entries: Vec<rp_fat::format::Entry> = files
        .iter()
        .map(|(name, data)| rp_fat::format::Entry { name, data })
        .collect();
    let img = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: (SD_IMAGE_BYTES / 512) as u32,
        sectors_per_cluster: 8,
        files: &entries,
        blocked_clusters: &[],
        ballast: 0,
    });
    let start = u32::from_le_bytes([img[454], img[455], img[456], img[457]]);
    let mut disk = Disk(img);
    organiser::find(&mut disk, start, &mut vec![0u8; 32 * 1024])
        .map_err(|e| format!("apps: the card's organiser files: {e}"))?;
    Ok((disk, start))
}

/// Pixels of an app's screen that differ, the title bar's clock left out
/// (the image's minutes pass).
fn app_differ(got: &[u8], want: &[u8]) -> usize {
    got.chunks(3)
        .zip(want.chunks(3))
        .enumerate()
        .filter(|(i, (a, b))| {
            let (x, y) = ((i % rp_today::W) as u16, (i / rp_today::W) as u16);
            a != b && !rp_shell::TITLE_CLOCK.contains_point(x, y)
        })
        .count()
}

/// What a run needs to tap the image and look at its screen.
struct Hands<'a> {
    root: &'a Path,
    wsl: bool,
}

impl Hands<'_> {
    fn dump(&self, name: &str) -> Result<Vec<u8>, String> {
        let shot = scratch(self.root, &format!("apps-{name}.ppm"))?;
        let guest = if self.wsl {
            to_wsl_path(&shot)
        } else {
            shot.display().to_string()
        };
        let _ = std::fs::remove_file(&shot);
        crate::screen::Monitor::connect(monitor_port())?.screendump(&guest, &shot)?;
        let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
        Ok(img.rgb)
    }

    fn hold(&self, ms: u32) -> Result<(), String> {
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            ms,
            "apps",
        )
    }

    /// A button pressed and let go, until the screen answers it, held in
    /// the PC's time (an idle image's clock moves only when it wakes).
    fn press_key(&self, qmp: &mut crate::screen::Qmp, code: &str) -> Result<(), String> {
        let before = self.dump("key")?;
        for _ in 0..4 {
            qmp.key(code, true)?;
            poll_pause(Duration::from_millis(120));
            qmp.key(code, false)?;
            let deadline = Instant::now() + Duration::from_secs(4);
            while Instant::now() < deadline {
                poll_pause(Duration::from_millis(150));
                if self.dump("key")? != before {
                    return self.hold(GAP_MS);
                }
            }
        }
        Err(format!("apps: no answer on screen to the {code} button"))
    }

    /// The inputs as the stylus and the buttons.
    fn play(&self, qmp: &mut crate::screen::Qmp, steps: &[Input]) -> Result<(), String> {
        for step in steps {
            match *step {
                Input::PenDown { x, y } | Input::PenMove { x, y } => {
                    touch(qmp, &Action::TouchDown(x, y))?;
                    self.hold(HOLD_MS)?;
                }
                Input::PenUp { .. } => {
                    touch(qmp, &Action::TouchUp)?;
                    self.hold(GAP_MS)?;
                }
                Input::Key(Key::Home) => self.press_key(qmp, "f4")?,
                _ => return Err(format!("apps: an input the check can't play: {step:?}")),
            }
        }
        Ok(())
    }

    /// Polls the screen until it is `want` by `differ`.
    fn until(
        &self,
        name: &str,
        want: &[u8],
        differ: &dyn Fn(&[u8], &[u8]) -> usize,
    ) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            poll_pause(Duration::from_millis(300));
            let n = differ(&self.dump(name)?, want);
            if n == 0 {
                return Ok(());
            }
            if Instant::now() > deadline {
                let mut ppm = format!("P6\n{} {}\n255\n", rp_today::W, rp_today::H).into_bytes();
                ppm.extend_from_slice(want);
                let _ = std::fs::write(scratch(self.root, &format!("apps-{name}-want.ppm"))?, ppm);
                return Err(format!(
                    "apps: the {name} screen differs from the model in {n} pixels (target/emulator/apps-{name}.ppm, expected apps-{name}-want.ppm)"
                ));
            }
        }
    }
}

fn tap(r: rp_draw::Rect) -> [Input; 2] {
    let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
    [Input::PenDown { x, y }, Input::PenUp { x, y }]
}

/// `cargo xtask emu-apps`.
pub fn apps(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-today.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-today"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let wall = today_wallpaper(&release)?;
    let map = today_map(&release);
    PREVIEW_CLOCK.with(|c| c.set(None));
    let (disk, _) = card(&release)?;
    let sd = scratch(root, "apps-card.img")?;
    std::fs::write(&sd, &disk.0).map_err(|e| format!("{}: {e}", sd.display()))?;
    let hands = Hands {
        root,
        wsl: matches!(emulator(root)?, Emulator::Wsl),
    };
    // Each app: Start, its entry (the menu's rows 1-5), then Home.
    let mut steps: Vec<Input> = Vec::new();
    let mut at = Vec::new();
    for n in 0..5u8 {
        steps.extend(tap(rp_shell::START));
        steps.extend(tap(Shell::menu_entry(1 + n)));
        at.push(steps.len());
        steps.push(Input::Key(Key::Home));
    }
    let prepare = |s: &mut Shell| {
        s.give_pics(preview_pics());
        s.use_programs(true);
        empty_organiser(s);
    };
    let all = today_screens(&wall, map.as_deref(), &steps, true, None, &prepare);
    // The first screen that differs, kept while the run still goes back
    // to the loader, so its log (what the image said about each program)
    // comes back with the failure.
    let failed: std::cell::RefCell<Option<String>> = std::cell::RefCell::new(None);
    let run = || -> Result<(), String> {
        hands.until("today", &all[0], &today_differ)?;
        let mut qmp = crate::screen::Qmp::connect(monitor_port() + 1)?;
        let mut from = 0;
        for (n, &k) in at.iter().enumerate() {
            hands.play(&mut qmp, &steps[from..k])?;
            let (name, _) = rp_today::program::PROGRAMS[n];
            if let Err(e) = hands.until(&name.to_ascii_lowercase(), &all[k], &app_differ) {
                *failed.borrow_mut() = Some(e);
                break;
            }
            hands.play(&mut qmp, &steps[k..=k])?;
            from = k + 1;
        }
        if failed.borrow().is_none()
            && let Err(e) = hands.until("today-again", &all[steps.len()], &today_differ)
        {
            *failed.borrow_mut() = Some(e);
        }
        // Back to the loader through Start (Home first: an app may be up).
        hands.press_key(&mut qmp, "f4").ok();
        hands.hold(1000)?;
        let entries = Shell::menu_entries(true);
        hands.play(&mut qmp, &tap(rp_shell::START))?;
        hands.hold(1000)?;
        hands.play(&mut qmp, &tap(Shell::menu_entry(entries - 1)))?;
        hands.hold(3000)
    };
    let out = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &["-icount", "shift=0"],
            input: Some((&rp_loadproto::frame(&payload), rp_today::MARKER)),
            during: Some(&run),
            then: Some("RPL END"),
            timeout: Duration::from_secs(1800),
        },
    )?;
    let log = out
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("apps: the image never came back to the loader")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("apps: the image didn't return cleanly:{log}"));
    }
    for l in log.lines().filter(|l| l.contains("program:")) {
        eprintln!("    {}", l.trim());
    }
    if let Some(e) = failed.borrow().clone() {
        return Err(e);
    }
    check_stack(log)?;
    for (name, _) in rp_today::program::PROGRAMS {
        let line = log
            .lines()
            .find(|l| l.contains("program:") && l.contains(name))
            .ok_or_else(|| format!("apps: the log never names {name} loaded:{log}"))?;
        if !line.contains("loaded in") {
            return Err(format!("apps: {name} didn't load: {line}"));
        }
        eprintln!("    {}", line.trim());
    }
    eprintln!("    apps OK: the five programs read from the card, loaded, hosted, pixel-exact");
    Ok(())
}
