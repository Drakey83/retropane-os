//! `cargo xtask emu-apps`: the organiser apps (batch 1 of
//! docs/design/core-apps-list.md) on the image in the emulator, with an SD
//! card that holds the sample records in the store's own files.
//!
//! Run 1: the image loads the records from the card, and every screen the
//! five apps show (the previews' taps, `apps_preview::shots`) is compared
//! pixel-exact with the PC's model. What the taps added is then read back
//! from the card image with the store's own code.
//!
//! Run 2, the same card a restart later, its clock a minute before the
//! Dentist's reminder: the reminder banner over Today, its Open, and the
//! Calendar, Tasks and Notes lists, against a model whose store is read
//! from that card: what was saved is what comes back.
//!
//! The card's clock is the previews' (noon, Monday 1 January 2024), so the
//! references are the pictures `cargo xtask ui-preview` writes.

#[allow(clippy::wildcard_imports)]
use super::*;
use rp_shell::{Input, Key, Shell};
use rp_today::organiser;
use std::cell::RefCell;

/// The previews' clock in local minutes from 1970: 12:00 on Monday
/// 1 January 2024 (19,723 days).
const NOON: i64 = 19_723 * 24 * 60 + 12 * 60;
/// Run 2's start: 14:44 that day (the Dentist's reminder is due 14:45).
const BEFORE_REMINDER: i64 = 19_723 * 24 * 60 + 14 * 60 + 44;
/// How long a tap is held and the pause after it, in the image's time.
const HOLD_MS: u32 = 150;
const GAP_MS: u32 = 250;

/// The card: the default icon pack's parts if the build made it, the time
/// file for `minutes`, and the organiser's files holding the sample
/// records. Where its partition starts, and the store's files.
fn card(release: &Path) -> Result<(Disk, u32, organiser::Files), String> {
    let pack = today_output(release, "rp-icons0.pak").unwrap_or_default();
    let mut parts: Vec<(String, &[u8])> = pack
        .chunks(rp_today::PACK_PART)
        .enumerate()
        .map(|(p, c)| (format!("RPIC0{}.PAK", (b'A' + p as u8) as char), c))
        .collect();
    if !parts.is_empty() {
        parts.push((
            format!("RPIC0{}.PAK", (b'A' + parts.len() as u8) as char),
            &[],
        ));
    }
    let time = rp_today::clock::card_time(NOON, 0);
    parts.push(("RPTIMA.PAK".to_string(), &time[..]));
    parts.push(("RPTIMB.PAK".to_string(), &[]));
    let files: Vec<rp_fat::format::Entry> = parts
        .iter()
        .map(|(name, data)| rp_fat::format::Entry { name, data })
        .collect();
    let img = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: (SD_IMAGE_BYTES / 512) as u32,
        sectors_per_cluster: 8,
        files: &files,
        blocked_clusters: &[],
        ballast: 0,
    });
    let start = u32::from_le_bytes([img[454], img[455], img[456], img[457]]);
    let mut disk = Disk(img);
    // The sample records, written by the store's own code as the image
    // writes them.
    let (slots, _) = organiser::find(&mut disk, start, &mut vec![0u8; 32 * 1024])
        .map_err(|e| format!("apps: the card's organiser files: {e}"))?;
    let mut model = Shell::new(rp_today::shell_fonts(), today_icons(), true);
    crate::apps_preview::sample(&mut model);
    let store = model.pim().ok_or("apps: the model has no store")?;
    let (_, failed) = organiser::save(&mut disk, &slots, store, usize::MAX);
    if failed || store.dirty() {
        return Err("apps: the sample records didn't fit the card".into());
    }
    Ok((disk, start, slots))
}

/// A store read from the card's files, as the image reads it.
fn read_store(disk: &mut Disk, slots: &organiser::Files, shell: &mut Shell) {
    shell.give_apps(vec![0u8; rp_shell::apps::MEMORY].leak());
    if let Some(s) = shell.pim() {
        organiser::begin(&mut *disk, slots, s);
        let mut buf = vec![0u8; organiser::BUF];
        while !s.loaded() {
            organiser::load(&mut *disk, slots, s, &mut buf);
        }
    }
    shell.pim_loaded();
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

    /// A button pressed and let go, until the screen answers it. Held
    /// 120 ms of the PC's time, not the image's: an idle image's clock, as
    /// the monitor reads it under `-icount`, moves only when the image next
    /// wakes, so a hold measured on it lasted about a second and the D-pad
    /// repeated (500 ms held, then 20 a second: one Down ran the focus to
    /// the bottom). A press the starved image never looked at is pressed
    /// again: every button here changes the screen.
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

    /// The inputs as the stylus and the buttons, each held in the image's
    /// own time. A drag rests before the lift, so the list stops where the
    /// pen left it (the model's drag has no speed).
    fn play(&self, qmp: &mut crate::screen::Qmp, steps: &[Input]) -> Result<(), String> {
        let mut moved = false;
        for step in steps {
            match *step {
                Input::PenDown { x, y } => {
                    touch(qmp, &Action::TouchDown(x, y))?;
                    self.hold(HOLD_MS)?;
                    moved = false;
                }
                Input::PenMove { x, y } => {
                    touch(qmp, &Action::TouchDown(x, y))?;
                    self.hold(HOLD_MS)?;
                    moved = true;
                }
                Input::PenUp { .. } => {
                    if moved {
                        self.hold(600)?;
                    }
                    touch(qmp, &Action::TouchUp)?;
                    self.hold(GAP_MS)?;
                }
                Input::Key(k) => {
                    let code = match k {
                        Key::Up => "up",
                        Key::Down => "down",
                        Key::Left => "left",
                        Key::Right => "right",
                        Key::Enter => "ret",
                        Key::Home => "f4",
                    };
                    self.press_key(qmp, code)?;
                }
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

    /// Start, then "Back to loader": waits until the image has left.
    fn leave(&self, qmp: &mut crate::screen::Qmp) -> Result<(), String> {
        let entries = Shell::menu_entries(true);
        let (s, back) = (rp_shell::START, Shell::menu_entry(entries - 1));
        let before = self.dump("leaving")?;
        self.play(
            qmp,
            &[
                Input::PenDown {
                    x: s.x + s.w / 2,
                    y: s.y + s.h / 2,
                },
                Input::PenUp { x: 0, y: 0 },
            ],
        )?;
        self.hold(1000)?;
        self.play(
            qmp,
            &[
                Input::PenDown {
                    x: back.x + back.w / 2,
                    y: back.y + back.h / 2,
                },
                Input::PenUp { x: 0, y: 0 },
            ],
        )?;
        // The softkey bar's rows are the loader's again.
        let strip = |s: &[u8]| s[600 * rp_today::W * 3..].to_vec();
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            poll_pause(Duration::from_millis(500));
            if strip(&self.dump("left")?) != strip(&before) {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err("apps: still showing after Back to loader".into());
            }
        }
    }
}

/// The run log's organiser lines, checked: it came back cleanly, loaded
/// from the card, and left nothing unsaved. The log's summary line.
fn log_of(output: &str, run: &str) -> Result<String, String> {
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or(format!("apps: {run} never came back to the loader"))?;
    if std::env::var_os("RETROPANE_TODAY_LOG").is_some() {
        eprintln!("{log}");
    }
    if crashed(log) || log.contains("did not return") {
        return Err(format!("apps: {run} didn't return cleanly:{log}"));
    }
    check_stack(log)?;
    if !log.contains("organiser: loaded,") {
        return Err(format!(
            "apps: {run} never loaded the store from the card:{log}"
        ));
    }
    let line = log
        .lines()
        .find(|l| l.contains("organiser:") && l.contains("sectors saved"))
        .ok_or_else(|| format!("apps: {run} has no organiser summary in its log:{log}"))?;
    if line.contains("NOT SAVED") {
        return Err(format!("apps: {run} left changes unsaved: {line}"));
    }
    Ok(line.trim().to_string())
}

/// `cargo xtask emu-apps`.
pub fn apps(root: &Path) -> Result<(), String> {
    use rp_pim::store::{Kind, tag};
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-today.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-today"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let wall = today_wallpaper(&release)?;
    let map = today_map(&release);
    // The references at the previews' clock (noon), the card's.
    PREVIEW_CLOCK.with(|c| c.set(None));
    let (disk, start, slots) = card(&release)?;
    let sd = scratch(root, "apps-card.img")?;
    std::fs::write(&sd, &disk.0).map_err(|e| format!("{}: {e}", sd.display()))?;
    let hands = Hands {
        root,
        wsl: matches!(emulator(root)?, Emulator::Wsl),
    };

    // ---- Run 1: every screen ------------------------------------------
    let shots = crate::apps_preview::shots();
    let steps: Vec<Input> = shots.iter().flat_map(|s| s.steps.iter().copied()).collect();
    let all = today_screens(
        &wall,
        map.as_deref(),
        &steps,
        true,
        None,
        &crate::apps_preview::sample,
    );
    let run1 = || -> Result<(), String> {
        hands.until("today", &all[0], &today_differ)?;
        let mut qmp = crate::screen::Qmp::connect(monitor_port() + 1)?;
        let mut at = 0;
        for (k, s) in shots.iter().enumerate() {
            hands.play(&mut qmp, &s.steps)?;
            at += s.steps.len();
            // The Start menu lies over Today (its clocks masked); an app's
            // screen is compared whole but for the title bar's clock.
            let differ: &dyn Fn(&[u8], &[u8]) -> usize =
                if k == 0 { &today_differ } else { &app_differ };
            hands.until(s.name, &all[at], differ)?;
        }
        // The changes reach the card while nothing moves.
        hands.hold(3000)?;
        hands.press_key(&mut qmp, "f4")?;
        hands.hold(2000)?;
        hands.leave(&mut qmp)
    };
    let boot_with = |during: &dyn Fn() -> Result<(), String>| {
        boot(
            root,
            &Boot {
                image: &loader,
                marker: LOADER_MARKER,
                screenshot: None,
                sd_image: Some(&sd),
                steps: &[],
                extra: &["-icount", "shift=0"],
                input: Some((&rp_loadproto::frame(&payload), rp_today::MARKER)),
                during: Some(during),
                then: Some("RPL END"),
                timeout: Duration::from_secs(2400),
            },
        )
    };
    let line1 = log_of(&boot_with(&run1)?, "run 1")?;

    // ---- The card after run 1, read on the PC -------------------------
    let mut disk = Disk(std::fs::read(&sd).map_err(|e| format!("{}: {e}", sd.display()))?);
    let (again, made) = organiser::find(&mut disk, start, &mut vec![0u8; 32 * 1024])
        .map_err(|e| format!("apps: the card after run 1: {e}"))?;
    if again != slots || made != 0 {
        return Err("apps: the image moved or remade the organiser's files".into());
    }
    let mut after = Shell::new(rp_today::shell_fonts(), today_icons(), true);
    read_store(&mut disk, &slots, &mut after);
    let store = after.pim().ok_or("apps: no store")?;
    let has = |kind: Kind, field: u8, text: &str| {
        store.all(kind).any(|r| {
            r.text(field)
                .to_ascii_lowercase()
                .windows(text.len())
                .any(|w| w == text.as_bytes())
        })
    };
    let counts =
        [Kind::Appointment, Kind::Contact, Kind::Task, Kind::Note].map(|k| store.all(k).count());
    for (kind, field, text, what) in [
        (
            Kind::Appointment,
            tag::TITLE,
            "gym",
            "the appointment added by quick add",
        ),
        (
            Kind::Appointment,
            tag::PLACE,
            "city gym",
            "its place typed in the editor",
        ),
        (Kind::Task, tag::TITLE, "buy stamps", "the task added"),
        (
            Kind::Note,
            tag::TEXT,
            "call mae about the gate",
            "the new note",
        ),
        (
            Kind::Note,
            tag::TEXT,
            "fresh ",
            "the words typed into a note",
        ),
    ] {
        if !has(kind, field, text) {
            return Err(format!(
                "apps: the card after run 1 lacks {what} ({text:?})"
            ));
        }
    }
    if !store
        .all(Kind::Task)
        .any(|r| r.text(tag::TITLE) == b"Pay rent" && r.num(tag::DONE).is_some())
    {
        return Err("apps: the card after run 1 doesn't show Pay rent done".into());
    }
    // The sample's 8, 18, 8 and 3, and one more appointment, task and note.
    if counts != [9, 18, 9, 4] {
        return Err(format!(
            "apps: the card after run 1 holds {counts:?} appointments, contacts, tasks, notes; expected [9, 18, 9, 4]"
        ));
    }

    // ---- Run 2: a restart on the same card, a minute before a reminder --
    {
        let mut v = rp_fat::AnyVolume::mount(&mut disk, start)
            .map_err(|e| format!("apps: the card's volume: {e:?}"))?;
        v.put(
            "RPTIMA.PAK",
            &rp_today::clock::card_time(BEFORE_REMINDER, 0),
            &mut vec![0u8; 32 * 1024],
        )
        .map_err(|e| format!("apps: the card's time file: {e:?}"))?;
    }
    std::fs::write(&sd, &disk.0).map_err(|e| format!("{}: {e}", sd.display()))?;
    PREVIEW_CLOCK.with(|c| {
        c.set(Some(rp_shell::Time {
            hour: 14,
            minute: 45,
            weekday: 0,
            day: 1,
            month: 1,
        }));
    });
    let tap_in = |r: rp_draw::Rect| {
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        vec![Input::PenDown { x, y }, Input::PenUp { x, y }]
    };
    let menu = |i: u8| [tap_in(rp_shell::START), tap_in(Shell::menu_entry(i))].concat();
    let left = tap_in(rp_draw::Rect::new(40, 600, 80, 40));
    let later: [(&str, Vec<Input>); 5] = [
        (
            "reminder-open",
            tap_in(rp_shell::apps::preview::REMINDER_OPEN),
        ),
        ("after-cancel", left),
        ("calendar-kept", menu(1)),
        ("tasks-kept", menu(3)),
        ("notes-kept", menu(4)),
    ];
    let steps2: Vec<Input> = later.iter().flat_map(|(_, s)| s.iter().copied()).collect();
    let card2 = RefCell::new(Disk(disk.0.clone()));
    let all2 = today_screens(&wall, map.as_deref(), &steps2, true, None, &|shell| {
        read_store(&mut card2.borrow_mut(), &slots, shell);
    });
    PREVIEW_CLOCK.with(|c| c.set(None));
    let run2 = || -> Result<(), String> {
        // The banner comes with the image's next minute.
        hands.until("reminder", &all2[0], &today_differ)?;
        let mut qmp = crate::screen::Qmp::connect(monitor_port() + 1)?;
        let mut at = 0;
        for (name, s) in &later {
            hands.play(&mut qmp, s)?;
            at += s.len();
            hands.until(name, &all2[at], &app_differ)?;
        }
        hands.press_key(&mut qmp, "f4")?;
        hands.hold(2000)?;
        hands.leave(&mut qmp)
    };
    let line2 = log_of(&boot_with(&run2)?, "run 2")?;
    eprintln!(
        "    apps OK: {} screens of the five apps pixel-exact against the PC's model (the title clock masked), the records loaded from the card; after it the card holds {counts:?} appointments, contacts, tasks and notes with what the taps added; a restart on that card showed the Dentist's reminder over Today, its Open, and the Calendar, Tasks and Notes lists pixel-exact against a model read from the same card.\n      run 1: {line1}\n      run 2: {line2}",
        shots.len()
    );
    Ok(())
}
