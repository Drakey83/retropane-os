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

/// A new note's ink panel (Notes: the paper's x + 8, its first line's
/// y - 18; `rp_app::kit::INK_PIC` in size).
const INK_PANEL: rp_draw::Rect =
    rp_draw::Rect::new(16, 106, rp_app::kit::INK_PIC.0, rp_app::kit::INK_PIC.1);

/// Ink drawn through the emulator's touch screen lands a pixel or two from
/// the model's exact points (the real filter and calibration), so the ink
/// panel is compared by how much ink it holds (within a tenth), the rest
/// of the screen pixel for pixel ([`app_differ`]).
fn ink_differ(got: &[u8], want: &[u8]) -> usize {
    let ink = |px: &[u8]| {
        px.chunks(3)
            .enumerate()
            .filter(|(i, c)| {
                let (x, y) = ((i % rp_today::W) as u16, (i / rp_today::W) as u16);
                INK_PANEL.contains_point(x, y)
                    && u32::from(c[0]) + u32::from(c[1]) + u32::from(c[2]) < 300
            })
            .count()
    };
    let (g, w) = (ink(got), ink(want));
    let off = if g.abs_diff(w) * 10 > w.max(1) {
        g.abs_diff(w).max(1)
    } else {
        0
    };
    let outside = got
        .chunks(3)
        .zip(want.chunks(3))
        .enumerate()
        .filter(|(i, (a, b))| {
            let (x, y) = ((i % rp_today::W) as u16, (i / rp_today::W) as u16);
            a != b && !rp_shell::TITLE_CLOCK.contains_point(x, y) && !INK_PANEL.contains_point(x, y)
        })
        .count();
    // No ink at all on either side proves nothing.
    outside + off + usize::from(w == 0)
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
                // The program buttons (docs/hw/keypad.md's emulator keys).
                Input::Key(Key::Calendar) => self.press_key(qmp, "f1")?,
                Input::Key(Key::Contacts) => self.press_key(qmp, "f2")?,
                Input::Key(Key::Record) => self.press_key(qmp, "f5")?,
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

/// The files Notes exported on the image (Markdown, then the ink as SVG)
/// read off the card: the Markdown names the ink's file, the SVG holds the
/// stroke written.
fn notes_exported(sd: &Path) -> Result<(), String> {
    let data = std::fs::read(sd).map_err(|e| format!("{}: {e}", sd.display()))?;
    let mut vol = rp_fat::AnyVolume::mount(card::Disk(data), rp_fat::format::PARTITION_START)
        .map_err(|e| format!("card image: {}", e.name()))?;
    let mut read = |name: &str| -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        vol.read_range(name, 0, u64::MAX, &mut [0u8; 4096], &mut |b| {
            out.extend_from_slice(b)
        })
        .map_err(|e| format!("notes export: {name} on the card: {}", e.name()))?;
        Ok(out)
    };
    let md = read("RetroPane/Notes/Note.md")?;
    if md != b"![Ink 1](Note-1.svg)\n" {
        return Err(format!(
            "notes export: Note.md is {:?}",
            String::from_utf8_lossy(&md)
        ));
    }
    let svg = String::from_utf8(read("RetroPane/Notes/Note-1.svg")?).map_err(|e| e.to_string())?;
    if !(svg.starts_with("<?xml") && svg.contains("<path") && svg.trim_end().ends_with("</svg>")) {
        return Err(format!("notes export: Note-1.svg is {svg}"));
    }
    eprintln!(
        "    notes export: Note.md ({} B) and Note-1.svg ({} B) on the card",
        md.len(),
        svg.len()
    );
    Ok(())
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
    let mut at: Vec<(String, usize)> = Vec::new();
    for n in 0..5u8 {
        steps.extend(tap(rp_shell::START));
        steps.extend(tap(Shell::menu_entry(1 + n)));
        let (name, _) = rp_today::program::PROGRAMS[usize::from(n)];
        at.push((name.to_ascii_lowercase(), steps.len()));
        steps.push(Input::Key(Key::Home));
    }
    // What the owner found on the X51v, 2026-11-14, each checked against
    // the model: the Record button opens a new note, and a stroke stays
    // after the pen lifts ("the lines dissapear as soon as you lift");
    // the Calendar button from inside Notes; Calendar's Week tab answers
    // the stylus; the Start menu over an app ("the full screen app is
    // covering the start menu"); the Contacts button.
    steps.push(Input::Key(Key::Record));
    at.push(("notes-new".into(), steps.len()));
    steps.push(Input::PenDown { x: 100, y: 200 });
    for k in 1..=5u16 {
        steps.push(Input::PenMove {
            x: 100 + k * 40,
            y: 200 + (k % 2) * 20,
        });
    }
    steps.push(Input::PenUp { x: 300, y: 220 });
    at.push(("notes-ink".into(), steps.len()));
    // Group D (notes-rule0.md, Export): the note written as Markdown,
    // then its ink as SVG, through the image's own file step on the card
    // (rp_today::appfiles); the files are read off the card after the run.
    let menu = rp_draw::Rect::new(rp_today::W as u16 - 160, rp_today::H as u16 - 40, 160, 40);
    for format in [1, 2] {
        steps.extend(tap(menu));
        steps.extend(tap(rp_app_notes::menu_key(3, 6)));
        steps.extend(tap(rp_app_notes::menu_key(format, 4)));
    }
    steps.push(Input::Key(Key::Calendar));
    at.push(("calendar-button".into(), steps.len()));
    // Calendar's views along the foot: Week is the fourth of five.
    steps.extend(tap(rp_app::kit::foot_tab(5, 3)));
    at.push(("calendar-week".into(), steps.len()));
    steps.extend(tap(rp_shell::START));
    at.push(("menu-over-app".into(), steps.len()));
    steps.extend(tap(rp_shell::START));
    steps.push(Input::Key(Key::Contacts));
    at.push(("contacts-button".into(), steps.len()));
    steps.push(Input::Key(Key::Home));
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
        for (name, k) in &at {
            hands.play(&mut qmp, &steps[from..*k])?;
            let differ: &dyn Fn(&[u8], &[u8]) -> usize = if name == "notes-ink" {
                &ink_differ
            } else {
                &app_differ
            };
            if let Err(e) = hands.until(name, &all[*k], differ) {
                *failed.borrow_mut() = Some(e);
                break;
            }
            from = *k;
        }
        if failed.borrow().is_none() {
            hands.play(&mut qmp, &steps[from..])?;
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
    notes_exported(&sd)?;
    // The stack's depth is required, not only checked when present: a run
    // whose image never reported it proves nothing about the programs'
    // stack (the overflow of 2026-11-14 was found here).
    let (used, size) = check_stack(log)?
        .ok_or_else(|| format!("apps: the image never reported its stack depth:{log}"))?;
    eprintln!(
        "    stack: deepest {used} of {size} bytes ({}%)",
        used * 100 / size.max(1)
    );
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
