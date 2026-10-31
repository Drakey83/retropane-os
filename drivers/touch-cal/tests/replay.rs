//! The owner's real taps (X51v, 2026-10-13), replayed through the filter.
//!
//! `data/owner-2026-10-13.txt` is `rp-today`'s recorder output: one line a
//! sample, `R <ms> <x> <y> <z1> <z2> <verdict>`, `-` for no pressure. The
//! session followed a script, so where each touch was meant to land is
//! known: the five quick buttons left to right, then holds on Note and
//! Games (the softkey taps after them never reached the program).

use rp_touch_cal::{Calibration, Filter, Pen, Sample};

/// A touch: its samples (`None`: no pressure), each with its time.
type Touch = Vec<(u32, Option<Sample>)>;

/// A screen rectangle: x, y, width, height.
type Area = (u16, u16, u16, u16);

fn touches() -> Vec<Touch> {
    let text = include_str!("data/owner-2026-10-13.txt");
    let mut out: Vec<Touch> = Vec::new();
    let mut cur: Touch = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let ms: u32 = f[1].parse().unwrap();
        if f[2] == "-" {
            cur.push((ms, None));
            out.push(std::mem::take(&mut cur));
        } else {
            let n = |i: usize| f[i].parse::<u16>().unwrap();
            cur.push((
                ms,
                Some(Sample {
                    x: n(2),
                    y: n(3),
                    z1: n(4),
                }),
            ));
        }
    }
    out
}

/// The screen rectangles the script asked for, in order (rp-shell's
/// layout: quick buttons at x 16 + 90 i, y 416, 82 square).
fn targets() -> Vec<(&'static str, Area)> {
    let quick = |i: u16| (16 + 90 * i, 416, 82, 82);
    vec![
        ("Note", quick(0)),
        ("Record", quick(1)),
        ("Reader", quick(2)),
        ("Media", quick(3)),
        ("Games", quick(4)),
        ("Note (held)", quick(0)),
        ("Games (held)", quick(4)),
    ]
}

fn inside(r: Area, p: (u16, u16)) -> bool {
    p.0 >= r.0 && p.0 < r.0 + r.2 && p.1 >= r.1 && p.1 < r.1 + r.3
}

#[test]
fn every_recorded_touch_lands_where_it_was_meant_and_soon() {
    let cal = Calibration::X51V_DEFAULT;
    let map = |x, y| cal.to_screen(x, y, (480, 640));
    let touches = touches();
    let targets = targets();
    assert_eq!(touches.len(), targets.len());
    let mut report = Vec::new();
    for (touch, (name, rect)) in touches.iter().zip(&targets) {
        let mut f = Filter::new();
        let start = touch[0].0;
        let (mut down, mut up) = (None, None);
        for &(ms, s) in touch {
            match f.sample(s, ms) {
                Pen::Down(x, y) => down = Some((ms - start, map(x, y))),
                Pen::Up(x, y) => up = Some(map(x, y)),
                _ => {}
            }
        }
        report.push(format!("{name}: down {down:?} up {up:?}"));
        eprintln!("{}", report.last().unwrap());
        let (after, at) = down.unwrap_or_else(|| panic!("{name}: never down"));
        assert!(
            inside(*rect, at),
            "{name}: down at {at:?}\n{}",
            report.join("\n")
        );
        let up = up.unwrap_or_else(|| panic!("{name}: never up"));
        assert!(
            inside(*rect, up),
            "{name}: up at {up:?}\n{}",
            report.join("\n")
        );
        // Samples came ~11 ms apart: down by the third after the first
        // (the second where it lands where the first was).
        assert!(
            after <= 22,
            "{name}: down {after} ms after the touch\n{}",
            report.join("\n")
        );
    }
}

/// The owner's free tapping (2026-10-13, after the scripted session): 26
/// touches anywhere. Every touch that gives three samples with steady
/// pressure must be recognised within 45 ms of landing (the 50 ms
/// promise), and no press may wander from its release by more than a
/// button's half-width.
#[test]
fn free_tapping_is_recognised_promptly() {
    let text = include_str!("data/owner-2026-10-13-free.txt");
    let cal = Calibration::X51V_DEFAULT;
    let map = |x, y| cal.to_screen(x, y, (480, 640));
    let mut f = Filter::new();
    let (mut landed, mut down) = (None::<u32>, None::<(u32, (u16, u16))>);
    let mut slow = Vec::new();
    let mut touches = 0;
    let mut seen: Vec<&str> = Vec::new();
    for line in text.lines() {
        seen.push(line);
        let v: Vec<&str> = line.split_whitespace().collect();
        let ms: u32 = v[1].parse().unwrap();
        let s = (v[2] != "-").then(|| {
            let n = |i: usize| v[i].parse::<u16>().unwrap();
            Sample {
                x: n(2),
                y: n(3),
                z1: n(4),
            }
        });
        if s.is_some_and(|s| s.z1 >= rp_touch_cal::Z_MIN) && landed.is_none() {
            landed = Some(ms);
        }
        match f.sample(s, ms) {
            Pen::Down(x, y) => down = Some((ms, map(x, y))),
            Pen::Up(x, y) => {
                touches += 1;
                let (at, p) = down.expect("up without down");
                let after = at - landed.unwrap();
                if after > 45 {
                    slow.push(after);
                    eprintln!(
                        "late {after} ms:
  {}",
                        seen.join(
                            "
  "
                        )
                    );
                }
                seen.clear();
                let q = map(x, y);
                assert!(
                    p.0.abs_diff(q.0) < 41 && p.1.abs_diff(q.1) < 41,
                    "pressed {p:?}, released {q:?}"
                );
                landed = None;
                down = None;
            }
            _ => {}
        }
        if s.is_none_or(|s| s.z1 < rp_touch_cal::Z_MIN) && down.is_none() {
            landed = None;
        }
    }
    assert!(touches >= 24, "{touches} touches");
    // The program was busy redrawing during one touch (a 56 ms gap
    // between its samples): at most that one may be late.
    assert!(slow.len() <= 1, "late: {slow:?} ms");
}

/// Every recording, as the filter saw it: `(file, ms, sample, verdict)`.
///
/// The recorder logs a sample with no pressure only when it ends a
/// reported touch, so a touch that lifted before it was reported runs on
/// into the next one in the log; the filter saw them apart. A gap of more
/// than 30 ms between samples before anything was reported (the loop
/// samples every ~11 ms while down) is taken as that lift: with it, the
/// filter of 2026-10-13 replays the device's own verdicts exactly on the
/// GPU runs.
fn all_samples() -> Vec<(&'static str, u32, Option<Sample>, char)> {
    let files = [
        ("scripted", include_str!("data/owner-2026-10-13.txt")),
        ("free", include_str!("data/owner-2026-10-13-free.txt")),
        (
            "session3",
            include_str!("data/owner-2026-10-13-session3.txt"),
        ),
        ("gpu2", include_str!("data/owner-2026-10-14-gpu2.txt")),
        ("gpu3", include_str!("data/owner-2026-10-14-gpu3.txt")),
        ("gpu4", include_str!("data/owner-2026-10-14-gpu4.txt")),
        ("merge", include_str!("data/owner-2026-10-14-merge.txt")),
    ];
    let mut out = Vec::new();
    for (name, text) in files {
        let (mut last, mut reported) = (None::<u32>, false);
        for line in text.lines() {
            let v: Vec<&str> = line.split_whitespace().collect();
            let ms: u32 = v[1].parse().unwrap();
            let verdict = v[6].chars().next().unwrap();
            let s = (v[2] != "-").then(|| {
                let n = |i: usize| v[i].parse::<u16>().unwrap();
                Sample {
                    x: n(2),
                    y: n(3),
                    z1: n(4),
                }
            });
            if s.is_some() && !reported && last.is_some_and(|t| ms - t > 30) {
                out.push((name, last.unwrap() + 1, None, 'n'));
            }
            reported = match verdict {
                'D' | 'M' | 'H' => true,
                'U' => false,
                _ => reported && s.is_some(),
            };
            last = s.map(|_| ms);
            out.push((name, ms, s, verdict));
        }
    }
    out
}

/// One touch replayed: when it landed, when and where it was pressed,
/// where it was released (screen pixels).
#[derive(Debug)]
struct Replayed {
    file: &'static str,
    landed: u32,
    down: Option<(u32, (u16, u16))>,
    up: Option<(u16, u16)>,
}

fn replay_all() -> Vec<Replayed> {
    let cal = Calibration::X51V_DEFAULT;
    let map = |x, y| cal.to_screen(x, y, (480, 640));
    let mut f = Filter::new();
    let mut out = Vec::new();
    let mut cur: Option<Replayed> = None;
    for (file, ms, s, _) in all_samples() {
        let pressed = s.is_some_and(|s| s.z1 >= rp_touch_cal::Z_MIN);
        if pressed && cur.is_none() {
            cur = Some(Replayed {
                file,
                landed: ms,
                down: None,
                up: None,
            });
        }
        let ev = f.sample(s, ms);
        if let (Pen::Down(x, y), Some(t)) = (ev, cur.as_mut()) {
            t.down = Some((ms - t.landed, map(x, y)));
        }
        if !pressed && let Some(mut t) = cur.take() {
            if let Pen::Up(x, y) = ev {
                t.up = Some(map(x, y));
            }
            out.push(t);
        }
    }
    out
}

/// The owner's 262 recorded touches (2026-10-13 and 14, seven sessions):
/// each must be pressed as soon as the rule of 2026-10-14 presses it,
/// and no press may land away from where the stylus came to rest. The
/// numbers are the replay's when that rule was adopted (docs/hw/touch.md:
/// the previous rule took a median of 22 ms, p90 33): a change that makes
/// taps slower, adds or loses taps, or makes presses jump fails here.
#[test]
fn every_recorded_touch_is_pressed_promptly_and_in_place() {
    let touches = replay_all();
    let pressed: Vec<&Replayed> = touches.iter().filter(|t| t.down.is_some()).collect();
    // 6 touches never steadied (brushes, a stylus bouncing off): no tap.
    assert_eq!(touches.len(), 262, "touches");
    assert_eq!(pressed.len(), 256, "taps (none new, none lost)");
    let mut after: Vec<u32> = pressed.iter().map(|t| t.down.unwrap().0).collect();
    after.sort_unstable();
    let at = |q: usize| after[(after.len() - 1) * q / 100];
    eprintln!(
        "landing to press: median {} ms, p90 {}, worst {}",
        at(50),
        at(90),
        at(100)
    );
    // Samples ~11 ms apart: most presses come on the second sample. The
    // seventh session ("merge", the rule's first device run, two screen
    // alignments) slid more on landing: 6 of its 31 taps pressed on the
    // second sample, its own median 22 ms; with it the median is 12.
    assert!(at(50) <= 12, "median {} ms", at(50));
    assert!(at(90) <= 23, "p90 {} ms", at(90));
    assert!(at(100) <= 45, "worst {} ms", at(100));
    // Press to release, in pixels (the larger axis). Five are deliberate
    // drags (up to 340 px); every tap stays within a few pixels.
    let jumps: Vec<(&str, u32, u16)> = pressed
        .iter()
        .filter_map(|t| {
            let (_, p) = t.down?;
            let q = t.up?;
            Some((t.file, t.landed, p.0.abs_diff(q.0).max(p.1.abs_diff(q.1))))
        })
        .filter(|j| j.2 > 20)
        .collect();
    assert!(
        jumps.len() <= 5,
        "pressed more than 20 px from release: {jumps:?}"
    );
}

/// The owner's drags and swipes on the X51v (2026-10-30, two runs of the
/// plane path's Weather app, `data/owner-2026-10-30-drags.txt`): every
/// long stroke follows the pen. The taps' rule alone (two samples within
/// `STABLE_WITHIN`, 95% of peak pressure) gave them 0-3 moves.
#[test]
fn the_owners_drags_follow_the_pen() {
    let text = include_str!("data/owner-2026-10-30-drags.txt");
    let mut strokes: Vec<Touch> = Vec::new();
    let mut cur: Touch = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let ms: u32 = f[1].parse().unwrap();
        // A stroke ends where the pen leaves (no pressure, or under the
        // filter's contact floor: the log runs touches together).
        let p = (f[2] != "-").then(|| {
            let n = |i: usize| f[i].parse::<u16>().unwrap();
            Sample {
                x: n(2),
                y: n(3),
                z1: n(4),
            }
        });
        match p.filter(|p| p.z1 >= rp_touch_cal::Z_MIN) {
            Some(p) => cur.push((ms, Some(p))),
            None if !cur.is_empty() => {
                cur.push((ms, None));
                strokes.push(std::mem::take(&mut cur));
            }
            None => {}
        }
    }
    let mut long = 0;
    for (k, s) in strokes.iter().enumerate() {
        // Where the pen was pressing, first to last: samples at the drag
        // rule's pressure (the landing's and the lift's light ones out).
        let peak = s
            .iter()
            .filter_map(|&(_, p)| p)
            .map(|p| u32::from(p.z1))
            .max()
            .unwrap_or(0);
        let pressed: Vec<Sample> = s
            .iter()
            .filter_map(|&(_, p)| p)
            .filter(|p| u32::from(p.z1) * 256 >= rp_touch_cal::Z_HELD_DRAG * peak)
            .collect();
        if pressed.len() < 3 {
            continue;
        }
        // Where it started: the middle of the first three pressed samples
        // (a landing can read one wild position first: x 1,541 then 2,865).
        let mid = |a: u16, b: u16, c: u16| a.max(b).min(a.min(b).max(c));
        let (p0, p1, p2) = (pressed[0], pressed[1], pressed[2]);
        let first = &Sample {
            x: mid(p0.x, p1.x, p2.x),
            y: mid(p0.y, p1.y, p2.y),
            z1: p2.z1,
        };
        // Where it stopped: the last sample before the pressure collapses
        // into the lift (above a third of the peak: a slide eases off long
        // before it lifts, the owner's down to a third).
        let Some(last) = s
            .iter()
            .rev()
            .filter_map(|&(_, p)| p)
            .find(|p| u32::from(p.z1) * 3 > peak)
        else {
            continue;
        };
        let travel = first.x.abs_diff(last.x).max(first.y.abs_diff(last.y));
        if travel < 800 || pressed.len() < 10 {
            continue;
        }
        long += 1;
        let mut f = Filter::new();
        let (mut moves, mut at) = (0, None);
        let mut trace = String::new();
        for &(ms, p) in s {
            let v = f.sample(p, ms);
            if let Some(p) = p {
                trace += &format!(" {},{},{}:{v:?}", p.x, p.y, p.z1);
            }
            match v {
                Pen::Move(x, y) => {
                    moves += 1;
                    at = Some((x, y));
                }
                Pen::Down(x, y) => at = Some((x, y)),
                _ => {}
            }
        }
        let (x, y) = at.unwrap_or_else(|| panic!("stroke {k}: never down"));
        assert!(
            moves >= 4,
            "stroke {k} (travel {travel}): only {moves} moves:{trace}"
        );
        let off = x.abs_diff(last.x).max(y.abs_diff(last.y));
        assert!(
            off <= 200,
            "stroke {k}: ended {off} raw from where the pen stopped:{trace}"
        );
    }
    assert!(long >= 20, "{long} long strokes recorded");
}
