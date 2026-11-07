//! R185's look check, run with no one watching (sheet
//! `docs/research/sheets/r185-look-check-unattended.md`, the owner's go
//! 2026-11-06): a touch recording made from the shell's own layout, and
//! the run log's verdict.
//!
//! The recording is the path a person would tap, found by driving the
//! shell itself on the PC with the device's settings (the Appearance row
//! moves with the places kept): Weather, then three times Menu,
//! Appearance (Auto, Light, Dark in turn, so the setting ends where it
//! began), back, and every page; then Today, Start, Back to loader. Each
//! tap's screen point goes back to the touch panel's raw readings through
//! the X51v's default alignment (the image's, unless aligned in the run).

use std::fmt::Write as _;
use std::path::Path;

use rp_shell::Calibration;
use rp_shell::{Action, Input, Screen, Shell, Target};

/// The alignment's fixed point (`rp_touch_cal::ONE`).
const ONE: i32 = 1 << 16;

/// Where the device's recording is written (under the repo: the image's
/// build reads it there, `RP_TODAY_REPLAY_FILE`), made with the device's
/// settings.
pub const RECORDING: &str = "target/look-check-taps.txt";
/// The emulator's, made with the default settings (its card holds none).
pub const RECORDING_EMU: &str = "target/look-check-taps-emu.txt";

/// The longest pause the replay keeps between taps (the build's cap,
/// `RP_TODAY_REPLAY_GAP_MS`): the first tap waits this for the image's
/// start (icon pack, pages).
pub const GAP_MS: u32 = 12_000;

/// A tap's samples: one every 7 ms as the panel gives them while pressed.
const SAMPLES: u32 = 12;
const SAMPLE_MS: u32 = 7;
/// The pressure reading a light tap gives (the owner's taps: 1,100-1,900).
const Z1: u16 = 1_450;

/// Raw readings (`rx`, `ry`) that `cal` maps to screen point (`x`, `y`):
/// the inverse of its affine map, rounded.
fn raw_for(cal: &Calibration, x: u16, y: u16) -> (u16, u16) {
    let one = f64::from(ONE);
    let (a, b, c) = (
        f64::from(cal.a) / one,
        f64::from(cal.b) / one,
        f64::from(cal.c) / one,
    );
    let (d, e, f) = (
        f64::from(cal.d) / one,
        f64::from(cal.e) / one,
        f64::from(cal.f) / one,
    );
    let (sx, sy) = (f64::from(x) - c, f64::from(y) - f);
    let det = a * e - b * d;
    let rx = (sx * e - b * sy) / det;
    let ry = (a * sy - d * sx) / det;
    (
        rx.round().clamp(0.0, 4095.0) as u16,
        ry.round().clamp(0.0, 4095.0) as u16,
    )
}

/// The recording being made, and the shell it is checked on.
struct Script {
    shell: Shell,
    cal: Calibration,
    text: String,
    at_ms: u32,
    taps: u32,
}

impl Script {
    /// Taps target `t` (its centre) after `wait_ms`, on the shell and into
    /// the recording: what the shell did.
    fn tap(&mut self, t: Target, wait_ms: u32) -> Action {
        let r = self.shell.rect_of(t);
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        let (rx, ry) = raw_for(&self.cal, x, y);
        assert_eq!(
            self.cal.to_screen(rx, ry, (rp_shell::W, rp_shell::H)),
            (x, y),
            "the raw point maps back to the tap"
        );
        self.at_ms += wait_ms;
        for _ in 0..SAMPLES {
            let _ = writeln!(self.text, "R {} {rx} {ry} {Z1}", self.at_ms);
            self.at_ms += SAMPLE_MS;
        }
        let _ = writeln!(self.text, "R {} - - - - U", self.at_ms);
        self.taps += 1;
        self.shell.input(Input::PenDown { x, y });
        let act = self.shell.input(Input::PenUp { x, y });
        self.shell.settle();
        act
    }

    fn expect(&self, s: Screen, step: &str) -> Result<(), String> {
        if self.shell.screen() == s {
            Ok(())
        } else {
            Err(format!(
                "look-replay: after {step} the shell is on {:?}, not {s:?}",
                self.shell.screen()
            ))
        }
    }
}

/// Makes a recording at `out` from the shell's layout and Weather
/// settings `set` (the device's: `local/weather-settings.bin`, the relay's
/// copy, which places the Appearance row; `None`: the defaults), checking
/// every step on the shell. Returns the taps and the looks gone through.
pub fn recording(root: &Path, set: Option<&[u8]>, out: &str) -> Result<(u32, Vec<bool>), String> {
    let mut shell = Shell::new(rp_today::shell_fonts(), crate::emu::today_icons(), true);
    if let Some(set) = set {
        shell.settings_from_card(set);
    }
    let mut f = rp_draw::Frame::new();
    shell.frame(&mut f);
    let mut s = Script {
        shell,
        cal: Calibration::X51V_DEFAULT,
        text: String::from(
            "# R185 look check (cargo xtask look-replay): made from the shell's layout, not recorded\n",
        ),
        at_ms: 0,
        taps: 0,
    };
    s.expect(Screen::Today, "the start")?;
    s.tap(Target::Hero, GAP_MS);
    s.expect(Screen::Weather, "the hero")?;
    let mut looks = vec![s.shell.light()];
    for round in 0..3 {
        s.tap(Target::SoftRight, 4_000);
        s.expect(Screen::WeatherSettings, "Menu")?;
        let row = s
            .shell
            .appearance_row()
            .ok_or("look-replay: no Appearance row on Weather settings")?;
        s.tap(row, 2_500);
        s.tap(Target::SoftLeft, 2_500);
        s.expect(Screen::Weather, "back from settings")?;
        looks.push(s.shell.light());
        // Every page in the new look; Radar's map and rain take longest.
        for (tab, wait) in [(1u8, 2_500u32), (2, 2_500), (3, 2_500), (0, 5_000)] {
            s.tap(Target::Tab(tab), wait);
            if s.shell.weather_page().0 != tab {
                return Err(format!("look-replay: round {round}: tab {tab} not shown"));
            }
        }
    }
    s.tap(Target::Back, 3_000);
    s.expect(Screen::Today, "back to Today")?;
    s.tap(Target::Start, 2_500);
    let last = Shell::menu_entries(true) - 1;
    if s.tap(Target::Menu(last), 1_500) != Action::BackToLoader {
        return Err("look-replay: the Start menu's last entry isn't Back to loader".into());
    }
    let out = root.join(out);
    std::fs::write(&out, &s.text).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok((s.taps, looks))
}

/// `cargo xtask look-replay`: makes the recording and says what it taps.
pub fn make(root: &Path) -> Result<(), String> {
    let set = std::fs::read(root.join("local/weather-settings.bin")).ok();
    for (out, set) in [(RECORDING, set.as_deref()), (RECORDING_EMU, None)] {
        let (taps, looks) = recording(root, set, out)?;
        let names: Vec<&str> = looks
            .iter()
            .map(|&l| if l { "light" } else { "dark" })
            .collect();
        eprintln!(
            "    {out}: {taps} taps; the looks in turn: {}",
            names.join(", ")
        );
    }
    Ok(())
}

/// The run log's verdict on the look check: `Err` with the reason when a
/// pass saw the wrong look, a look wasn't judged right on every page, or
/// more than a tenth of the passes were unknown. Prints the summary.
pub fn judge(log: &str) -> Result<(), String> {
    let wrong: Vec<&str> = log
        .lines()
        .filter(|l| l.starts_with("LOOK WRONG"))
        .collect();
    let sum = log
        .lines()
        .find(|l| l.starts_with("look check: ") && l.contains(" wrong; "))
        .ok_or("no `look check:` summary (built without RP_TODAY_LOOK_CHECK?)")?;
    eprintln!("      {}", sum.trim_end());
    for l in &wrong {
        eprintln!("      {}", l.trim_end());
    }
    if !wrong.is_empty() || !sum.starts_with("look check: 0 wrong") {
        return Err(format!("the wrong look was shown: {}", sum.trim_end()));
    }
    // "right: dark on pages 0-3 [a, b, c, d], light on pages 0-3 [e, f, g, h]; N passes unknown"
    let nums = |after: &str| -> Option<Vec<u32>> {
        let i = sum.find(after)? + after.len();
        let j = sum[i..].find(']')? + i;
        sum[i..j]
            .split(',')
            .map(|n| n.trim().parse().ok())
            .collect()
    };
    let dark = nums("dark on pages 0-3 [").ok_or("summary unreadable")?;
    let light = nums("light on pages 0-3 [").ok_or("summary unreadable")?;
    let unknown: u32 = sum
        .split("; ")
        .last()
        .and_then(|t| t.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .ok_or("summary unreadable")?;
    if dark.iter().chain(&light).any(|&n| n == 0) {
        return Err(format!(
            "a page wasn't judged in both looks: {}",
            sum.trim_end()
        ));
    }
    let right: u32 = dark.iter().chain(&light).sum();
    if unknown * 10 > right + unknown {
        return Err(format!(
            "over a tenth of the passes unknown: {}",
            sum.trim_end()
        ));
    }
    Ok(())
}

/// The frame budget over the look check's replay (R171, item 4 of the
/// owner's list of 2026-11-06): no still frame's preparation over 33 ms
/// (30 fps) in the emulator's own time. Moving frames (a page sliding in)
/// are reported with their worst, not failed: the Radar page's slide is
/// drawn by the 2D path whole each frame and runs over (46.9 ms measured
/// 2026-11-07; its fix is open, docs/reports/deep-dive-2026-11-06-fix.md).
/// The still worst and the moving worst, us.
pub fn frame_budget(log: &str) -> Result<(u32, u32), String> {
    const BUDGET_US: u32 = 33_000;
    let (mut still, mut moving) = (0u32, 0u32);
    for l in log.lines().filter(|l| l.starts_with("slow frame ")) {
        let us: u32 = l
            .split("ms")
            .nth(1)
            .and_then(|t| t.split(": ").nth(1))
            .and_then(|t| t.split(' ').next())
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| format!("an unreadable slow frame: {l:?}"))?;
        if l.contains("(moving)") {
            moving = moving.max(us);
        } else {
            still = still.max(us);
            if us > BUDGET_US {
                return Err(format!(
                    "a still frame over the {BUDGET_US} us budget: {l:?}"
                ));
            }
        }
    }
    Ok((still, moving))
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_budget_fails_a_slow_still_frame_and_reports_moving_ones() {
        let log = "slow frame 4 at 12622 ms: 16460 us (queueing)
slow frame 9 at 20000 ms (moving): 46924 us (queueing)
";
        assert_eq!(frame_budget(log), Ok((16460, 46924)));
        assert!(frame_budget("slow frame 5 at 1 ms: 33001 us (x)").is_err());
        assert_eq!(frame_budget("nothing"), Ok((0, 0)));
    }

    #[test]
    fn the_verdict_needs_no_wrong_and_both_looks_on_every_page() {
        let ok = "look check: 0 wrong; right: dark on pages 0-3 [5, 6, 7, 8], light on pages 0-3 [1, 2, 3, 4]; 2 passes unknown (too few points agreeing)";
        assert_eq!(judge(ok), Ok(()));
        assert!(
            judge(&format!(
                "LOOK WRONG at 5 ms: x
{ok}"
            ))
            .is_err()
        );
        assert!(judge(&ok.replace("0 wrong", "1 wrong")).is_err());
        assert!(judge(&ok.replace("[1, 2, 3, 4]", "[1, 0, 3, 4]")).is_err());
        assert!(judge(&ok.replace("2 passes", "9 passes")).is_err());
        assert!(judge("nothing").is_err());
    }
    use super::*;

    /// Every screen point goes to raw readings that the image's default
    /// alignment maps back to that same point.
    #[test]
    fn raw_points_map_back_to_their_taps() {
        let cal = Calibration::X51V_DEFAULT;
        for y in (0..640).step_by(37) {
            for x in (0..480).step_by(29) {
                let (rx, ry) = raw_for(&cal, x, y);
                assert_eq!(cal.to_screen(rx, ry, (480, 640)), (x, y));
            }
        }
    }
}
