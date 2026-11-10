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

/// Every frame's budget: 33 ms (30 fps).
pub const BUDGET_US: u32 = 33_000;
/// A single whole-screen change's (a look switch, a screen change): one
/// late frame up to ~45 ms (the owner's ruling, 2026-11-10: a whole-screen
/// frame is ~40 ms of 2D engine work on the X51v whatever the caches do,
/// the 4 x 4 dither alone 18.9 ms).
pub const WHOLE_SCREEN_US: u32 = 45_000;
/// The screen's pixels: a frame that dithers this many changed it whole.
const SCREEN_PX: u32 = 480 * 640;

/// The worst frames a replay logged, us.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Worst {
    /// Of frames with nothing moving.
    pub still: u32,
    /// Of frames that move (scrolls, slides, the sky).
    pub moving: u32,
    /// Of single whole-screen changes over 33 ms (held to 45 ms).
    pub whole: u32,
}

/// The frame budget over the look check's replay (R171), the owner's rule
/// of 2026-11-10: 33 ms (30 fps) for every frame, moving ones too, except a
/// single whole-screen change (the frame dithers the whole screen), allowed
/// up to 45 ms as one late frame: the frame right after it (the next frame
/// number) over 33 ms fails it. The folded line for a run of moving frames
/// like the last logged ("slow frames: N more while moving") is held to 33
/// ms. In the emulator the times are guest time, counted from instructions
/// (`-icount`, so a busy PC can't move them; on host time the same still
/// Radar frame read 20 ms idle and 68 ms beside a build, 2026-11-07); on
/// the X51v (`cargo xtask look-judge <card log>`) they are its own.
pub fn frame_budget(log: &str) -> Result<Worst, String> {
    let mut w = Worst::default();
    // The last whole-screen change let through: its frame number.
    let mut late: Option<u64> = None;
    for l in log.lines() {
        if let Some(t) = l.strip_prefix("slow frames: ") {
            let us: u32 = t
                .split("(worst ")
                .nth(1)
                .and_then(|t| t.split(' ').next())
                .and_then(|n| n.parse().ok())
                .ok_or_else(|| format!("an unreadable slow-frames line: {l:?}"))?;
            w.moving = w.moving.max(us);
            if us > BUDGET_US {
                return Err(format!(
                    "moving frames over the {BUDGET_US} us budget: {l:?}"
                ));
            }
            continue;
        }
        let Some(t) = l.strip_prefix("slow frame ") else {
            continue;
        };
        let no: u64 = t
            .split(' ')
            .next()
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| format!("an unreadable slow frame: {l:?}"))?;
        let us: u32 = l
            .split("ms")
            .nth(1)
            .and_then(|t| t.split(": ").nth(1))
            .and_then(|t| t.split(' ').next())
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| format!("an unreadable slow frame: {l:?}"))?;
        let dithered: u32 = l
            .split(" px dithered")
            .next()
            .and_then(|t| t.rsplit(' ').next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        if us > BUDGET_US {
            let whole = dithered >= SCREEN_PX;
            let after_late = late.is_some_and(|n| no == n + 1);
            if !whole || us > WHOLE_SCREEN_US || after_late {
                return Err(format!(
                    "a frame over the {BUDGET_US} us budget{}: {l:?}",
                    if !whole {
                        " (not a whole-screen change)"
                    } else if after_late {
                        " (right after a late whole-screen change: one late frame allowed)"
                    } else {
                        " (a whole-screen change, over its 45,000 us)"
                    }
                ));
            }
            late = Some(no);
            w.whole = w.whole.max(us);
        } else if l.contains("(moving)") {
            w.moving = w.moving.max(us);
        } else {
            w.still = w.still.max(us);
        }
    }
    Ok(w)
}

/// A device run's card log judged as the emulator's look check is: the
/// verdict and the frame budget.
pub fn judge_file(path: &str) -> Result<(), String> {
    let log = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let verdict = judge(&log);
    let w = frame_budget(&log);
    eprintln!("look-judge {path}:");
    eprintln!("  verdict: {verdict:?}");
    eprintln!("  frame budget: {w:?}");
    verdict.and(w.map(|_| ()))
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_budget_fails_any_slow_frame_and_reports_the_worst_of_each() {
        let log = "slow frame 4 at 12622 ms: 16460 us (queueing)
slow frame 9 at 20000 ms (moving): 22368 us (queueing)
";
        let w = |still, moving, whole| Worst {
            still,
            moving,
            whole,
        };
        assert_eq!(frame_budget(log), Ok(w(16460, 22368, 0)));
        assert!(frame_budget("slow frame 5 at 1 ms: 33001 us (x)").is_err());
        assert!(frame_budget("slow frame 6 at 1 ms (moving): 33001 us (x)").is_err());
        assert_eq!(frame_budget("nothing"), Ok(w(0, 0, 0)));
    }

    /// The owner's rule of 2026-11-10: 33 ms for frames that move, a single
    /// whole-screen change up to ~45 ms as one late frame.
    #[test]
    fn a_single_whole_screen_change_may_take_45_ms_and_a_moving_frame_33() {
        let f = |no: u32, us: u32, px: u32| {
            format!(
                "slow frame {no} at 40534 ms (moving): {us} us (queueing {us} us, wait 27 us, background true (69198 px), rows written 33 reused 228, 1 rects, {px} px dithered, 6423 command words, 78 commands drawn; 5 glyph atlases begun (0 evicted), 94 glyphs and 4 pictures uploaded)\n"
            )
        };
        // The look switch on the X51v (today-0082): 39,516 us, the screen.
        let switch = f(68, 39_516, 307_200);
        assert_eq!(
            frame_budget(&switch),
            Ok(Worst {
                still: 0,
                moving: 0,
                whole: 39_516
            })
        );
        assert!(frame_budget(&f(68, 45_000, 307_200)).is_ok());
        // Over 45 ms, even whole: fails (the return's 53,548 us).
        assert!(frame_budget(&f(153, 53_548, 307_200)).is_err());
        // A moving frame over 33 ms that isn't the whole screen: fails.
        assert!(frame_budget(&f(70, 33_001, 120_000)).is_err());
        // Two late whole-screen frames running: only one is allowed.
        assert!(frame_budget(&(f(80, 40_000, 307_200) + &f(81, 34_000, 307_200))).is_err());
        // Two whole-screen changes apart: each its own late frame.
        assert!(frame_budget(&(f(80, 40_000, 307_200) + &f(95, 41_000, 307_200))).is_ok());
        // The folded moving frames are held to 33 ms.
        let folded = |us| {
            format!(
                "slow frames: 114 more while moving, each like the one logged before it (worst {us} us)\n"
            )
        };
        assert_eq!(frame_budget(&folded(21_964)).map(|w| w.moving), Ok(21_964));
        assert!(frame_budget(&folded(33_001)).is_err());
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
