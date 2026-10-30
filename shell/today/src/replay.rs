//! The touch replay (`RP_TODAY_REPLAY`, the ratchet's touch row: the
//! owner's hybrid, docs/design/perf-ratchet.md): recorded pen samples
//! through the image's own filter, shell and drawing, so a tap's answer is
//! measured the same way every run.
//!
//! The recording is the owner's taps of 2026-10-17
//! (`drivers/touch-cal/tests/data/owner-2026-10-17-12k.txt`, at today's
//! sample rate: 29 touches, 28 of them presses, through the start menu;
//! the last touch, ending that run, left out), packed by the build script. Within
//! a touch the image's own loop sets the pace (one recorded sample a pass,
//! the real sample's conversions still made), so a slower loop shows as a
//! slower verdict, as it did in C58; between touches the recording's pause,
//! shortened to at most 500 ms.

use rp_touch_cal::Sample;

/// One recorded sample: the wait before it (non-zero only where a touch
/// begins) and the sample, `None` where the pen lifted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    /// Milliseconds to wait before this sample, the pen up.
    pub wait_ms: u16,
    /// What the screen read: `None`, no pressure (the touch ended).
    pub sample: Option<Sample>,
}

/// The packed recording.
pub static DATA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/replay.bin"));

/// The steps of a packed recording, in order.
pub fn steps(data: &'static [u8]) -> impl Iterator<Item = Step> {
    data.as_chunks::<8>().0.iter().map(|c| {
        let v = |i: usize| u16::from_le_bytes([c[2 * i], c[2 * i + 1]]);
        Step {
            wait_ms: v(0),
            sample: (v(3) != 0).then(|| Sample {
                x: v(1),
                y: v(2),
                z1: v(3),
            }),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_shell::{Action, Input, Shell};
    use rp_touch_cal::{Calibration, Filter, Pen};

    const TEXT: &str =
        include_str!("../../../drivers/touch-cal/tests/data/owner-2026-10-17-12k.txt");

    /// The recording read straight from its text, independently of the
    /// build script's packing: (ms, sample).
    fn recorded() -> Vec<(u32, Option<Sample>)> {
        TEXT.lines()
            .take_while(|l| l.split_whitespace().nth(1) != Some("73534"))
            .map(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                let ms = f[1].parse().unwrap();
                let s = (f[2] != "-").then(|| Sample {
                    x: f[2].parse().unwrap(),
                    y: f[3].parse().unwrap(),
                    z1: f[4].parse().unwrap(),
                });
                (ms, s)
            })
            .collect()
    }

    /// The steps the recording means, worked out here from the text (a
    /// gap over 100 ms is the pen up; a lift with no line gets one).
    fn expected() -> Vec<Step> {
        let mut out = Vec::new();
        let mut prev: Option<(u32, bool)> = None;
        for (ms, s) in recorded() {
            if let Some((at, pressed)) = prev
                && ms - at > 100
            {
                if pressed {
                    out.push(Step {
                        wait_ms: 0,
                        sample: None,
                    });
                }
                out.push(Step {
                    wait_ms: (ms - at).min(500) as u16,
                    sample: s,
                });
            } else {
                let wait_ms = if prev.is_none() { 500 } else { 0 };
                out.push(Step { wait_ms, sample: s });
            }
            prev = Some((ms, s.is_some()));
        }
        if prev.is_some_and(|(_, p)| p) {
            out.push(Step {
                wait_ms: 0,
                sample: None,
            });
        }
        out
    }

    #[test]
    fn the_packed_recording_is_the_text() {
        let got: Vec<Step> = steps(DATA).collect();
        assert_eq!(DATA.len() % 8, 0, "no stray bytes");
        assert_eq!(got, expected());
        // Every recorded sample is in it, in order.
        let kept: Vec<Option<Sample>> = got
            .iter()
            .map(|s| s.sample)
            .filter(Option::is_some)
            .collect();
        let rec: Vec<Option<Sample>> = recorded()
            .into_iter()
            .map(|r| r.1)
            .filter(Option::is_some)
            .collect();
        assert_eq!(kept, rec);
    }

    #[test]
    fn the_replay_is_29_touches_each_lifted_before_the_next() {
        let all: Vec<Step> = steps(DATA).collect();
        // 28 presses and a brush (a touch the filter never pressed); the
        // recording's 30th touch, the owner ending the run, is left out.
        assert_eq!(all.iter().filter(|s| s.wait_ms > 0).count(), 29);
        assert!(
            all.last().is_some_and(|s| s.sample.is_none()),
            "ends lifted"
        );
        for w in all.windows(2) {
            if w[1].wait_ms > 0 {
                assert!(w[0].sample.is_none(), "a pause follows a lift");
            }
        }
    }

    /// Each replayed answer's changed area, printed (R115 part 2,
    /// 2026-10-30: presses 1-3% of the screen, the Start menu's opening
    /// and closing about half): `cargo test -p rp-today
    /// the_replays_changed_areas -- --ignored --nocapture`.
    #[test]
    #[ignore = "exploration: prints each answer's changed area"]
    fn the_replays_changed_areas() {
        let cal = Calibration::X51V_DEFAULT;
        let mut shell = Shell::new(crate::shell_fonts(), rp_shell::Icons::NONE, true);
        let mut filter = Filter::new();
        let mut frame = rp_draw::Frame::new();
        let mut now = 0u32;
        shell.frame(&mut frame);
        for st in steps(DATA) {
            now += u32::from(st.wait_ms).max(7);
            shell.tick(now);
            let ev = filter.sample(st.sample, now);
            let map = |x, y| cal.to_screen(x, y, (crate::W as u16, crate::H as u16));
            let what = match ev {
                Pen::Down(x, y) => {
                    let (x, y) = map(x, y);
                    shell.input(Input::PenDown { x, y });
                    format!("down {x},{y}")
                }
                Pen::Up(x, y) => {
                    let (x, y) = map(x, y);
                    shell.input(Input::PenUp { x, y });
                    format!("up {x},{y}")
                }
                _ => String::new(),
            };
            if shell.needs_frame() {
                shell.frame(&mut frame);
                let px: u32 = frame
                    .dirty()
                    .iter()
                    .map(|r| u32::from(r.w) * u32::from(r.h))
                    .sum();
                std::println!(
                    "{now} {what}: {} rects {px} px ({}%) {:?}",
                    frame.dirty().len(),
                    px * 100 / (480 * 640),
                    frame.dirty()
                );
            }
        }
    }

    /// The scenario through the image's filter and the shell (the same
    /// calls as `main`'s loop, without the drawing): it must stay a run
    /// of the Today screen: no return to the loader, no align screen, and
    /// its taps answered. A change to the shell that moves where these
    /// taps land changes the scenario; this says so before the ratchet does.
    #[test]
    fn the_scenario_stays_on_the_today_screen() {
        let cal = Calibration::X51V_DEFAULT;
        let mut shell = Shell::new(crate::shell_fonts(), rp_shell::Icons::NONE, true);
        let mut filter = Filter::new();
        let (mut now, mut downs, mut redraws) = (0u32, 0, 0);
        for st in steps(DATA) {
            now += u32::from(st.wait_ms).max(7);
            shell.tick(now);
            let ev = filter.sample(st.sample, now);
            assert!(
                !shell.wants_raw(),
                "the replay opened the align screen at {now} ms"
            );
            let map = |x, y| cal.to_screen(x, y, (crate::W as u16, crate::H as u16));
            let act = match ev {
                Pen::Down(x, y) => {
                    downs += 1;
                    let (x, y) = map(x, y);
                    shell.input(Input::PenDown { x, y })
                }
                Pen::Up(x, y) => {
                    let (x, y) = map(x, y);
                    shell.input(Input::PenUp { x, y })
                }
                _ => Action::Nothing,
            };
            assert!(
                !matches!(act, Action::BackToLoader | Action::Calibrated(_)),
                "the replay left the Today screen at {now} ms: {act:?}"
            );
            redraws += u32::from(act == Action::Redraw);
        }
        assert_eq!(downs, 28, "presses");
        assert!(redraws >= 50, "{redraws} answers");
    }
}
