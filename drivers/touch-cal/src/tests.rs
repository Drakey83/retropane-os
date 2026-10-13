use super::*;

/// A panel: swapped and mirrored like the X51v's, with a little rotation.
fn panel(sx: u16, sy: u16) -> (u16, u16) {
    let (x, y) = (f64::from(sx), f64::from(sy));
    // Raw X follows the vertical, raw Y the horizontal mirrored; 1 degree.
    let rx = 120.0 + y * 4.2 + x * 0.07;
    let ry = 3700.0 - x * 7.0 + y * 0.05;
    (rx.round() as u16, ry.round() as u16)
}

fn points(targets: &[(u16, u16)]) -> Vec<Point> {
    targets
        .iter()
        .map(|&(sx, sy)| {
            let (rx, ry) = panel(sx, sy);
            Point { rx, ry, sx, sy }
        })
        .collect()
}

const FIVE: [(u16, u16); 5] = [(48, 64), (432, 64), (48, 576), (432, 576), (240, 320)];

#[test]
fn a_fit_maps_every_point_back() {
    let p = points(&FIVE);
    let cal = Calibration::fit(&p).unwrap();
    for q in &p {
        let (x, y) = cal.map(q.rx, q.ry);
        assert!(
            (x - i64::from(q.sx)).abs() <= 1 && (y - i64::from(q.sy)).abs() <= 1,
            "{q:?} -> {x} {y}"
        );
    }
    // And points it never saw.
    for (sx, sy) in [(10, 10), (470, 630), (100, 500), (300, 200)] {
        let (rx, ry) = panel(sx, sy);
        let (x, y) = cal.map(rx, ry);
        assert!(
            (x - i64::from(sx)).abs() <= 1 && (y - i64::from(sy)).abs() <= 1,
            "{sx},{sy} -> {x} {y}"
        );
    }
}

#[test]
fn the_left_out_point_shows_a_linear_panel() {
    let p = points(&FIVE);
    assert!(Calibration::check(&p, 4).unwrap() <= 1);
    // A panel bulging in the middle: the centre is missed.
    let mut bent = p.clone();
    bent[4].rx += 80;
    assert!(Calibration::check(&bent, 4).unwrap() >= 15);
}

#[test]
fn degenerate_points_give_no_fit() {
    assert!(Calibration::fit(&points(&FIVE[..2])).is_none());
    // The same raw reading for every target: nothing to solve.
    let same: Vec<Point> = FIVE
        .iter()
        .map(|&(sx, sy)| Point {
            rx: 1000,
            ry: 1000,
            sx,
            sy,
        })
        .collect();
    assert!(Calibration::fit(&same).is_none());
    // In a line.
    let line: Vec<Point> = (0..5u16)
        .map(|i| Point {
            rx: 100 * i,
            ry: 200 * i,
            sx: 10 * i,
            sy: 20 * i,
        })
        .collect();
    assert!(Calibration::fit(&line).is_none());
}

#[test]
fn per_axis_maps_match_the_old_calibration() {
    let cal = Calibration::X51V_DEFAULT;
    assert_eq!(cal.to_screen(99, 3719, (480, 640)), (0, 0));
    assert_eq!(cal.to_screen(2741, 319, (480, 640)), (479, 639));
    // The owner's centre tap.
    let (x, y) = cal.to_screen(1423, 2017, (480, 640));
    assert!(x.abs_diff(240) < 12 && y.abs_diff(320) < 12, "{x} {y}");
    // Off the panel: clamped.
    assert_eq!(cal.to_screen(0, 4095, (480, 640)), (0, 0));
    assert_eq!(cal.to_screen(4095, 0, (480, 640)), (479, 639));
}

#[test]
fn medians() {
    assert_eq!(median(&mut []), 0);
    assert_eq!(median(&mut [5]), 5);
    assert_eq!(median(&mut [9, 1, 5]), 5);
    assert_eq!(median(&mut [4000, 2000, 2010, 1990, 0]), 2000);
}

#[test]
fn the_most_points_fit_without_overflow() {
    // Sixteen points at the extremes of both ranges.
    let pts: Vec<Point> = (0..MAX_POINTS as u16)
        .map(|i| Point {
            rx: if i % 2 == 0 { 0 } else { 4095 },
            ry: if i % 4 < 2 { 0 } else { 4095 },
            sx: if i % 4 < 2 { 4095 } else { 0 },
            sy: if i % 2 == 0 { 0 } else { 4095 },
        })
        .collect();
    let cal = Calibration::fit(&pts).unwrap();
    assert_eq!(cal.map(4095, 0), (4095, 4095));
    assert_eq!(cal.map(0, 4095), (0, 0));
    assert!(Calibration::fit(&[pts.as_slice(), &pts[..1]].concat()).is_none());
}

#[test]
fn rounding_divides_both_signs() {
    assert_eq!(scaled_div(1, 2), Some(32768));
    assert_eq!(scaled_div(-1, 2), Some(-32768));
    assert_eq!(scaled_div(3, -4), Some(-49152));
    // Too big to scale up: the divisor goes down instead.
    assert_eq!(scaled_div(i64::MAX / 2, i64::MAX / 4), Some(131_072));
}

#[test]
fn the_landing_is_ignored_and_positions_need_agreement() {
    let mut f = Filter::new();
    // Landing: creeping readings in the first 15 ms say nothing.
    assert_eq!(f.sample(Some((1200, 800)), 0), Pen::Nothing);
    assert_eq!(f.sample(Some((2400, 800)), 5), Pen::Nothing);
    assert_eq!(f.sample(Some((2700, 810)), 10), Pen::Nothing);
    // Settled: three agreeing samples report their average.
    assert_eq!(f.sample(Some((2860, 816)), 15), Pen::Nothing);
    assert_eq!(f.sample(Some((2866, 817)), 20), Pen::Nothing);
    assert_eq!(f.sample(Some((2870, 815)), 25), Pen::Down(2865, 816));
    // One wild sample (a lift starting) changes nothing.
    assert_eq!(f.sample(Some((0, 816)), 30), Pen::Nothing);
    // Lifted: at the last stable position, not the wild one.
    assert_eq!(f.sample(None, 35), Pen::Up(2865, 816));
    assert!(!f.down());
}

#[test]
fn a_drag_moves_and_a_tap_too_short_to_settle_is_dropped() {
    let mut f = Filter::new();
    for (t, x) in [(0, 1000), (15, 1000), (20, 1002), (25, 1001)] {
        f.sample(Some((x, 2000)), t);
    }
    assert_eq!(f.sample(Some((1001, 2000)), 30), Pen::Hold(1001, 2000));
    // Moving: reported once samples agree on the new place.
    f.sample(Some((1100, 2000)), 35);
    f.sample(Some((1102, 2000)), 40);
    assert_eq!(f.sample(Some((1101, 2000)), 45), Pen::Move(1101, 2000));
    assert_eq!(f.sample(None, 50), Pen::Up(1101, 2000));
    // A touch that never settled: nothing at all, not a tap somewhere odd.
    let mut f = Filter::new();
    f.sample(Some((100, 100)), 0);
    f.sample(Some((900, 100)), 20);
    assert_eq!(f.sample(None, 25), Pen::Nothing);
    // Its landing time is known while down (for latency).
    f.sample(Some((5, 5)), 100);
    assert_eq!(f.landed_at(), Some(100));
}
