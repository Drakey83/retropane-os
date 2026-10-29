extern crate std;

use super::*;
use crate::layers::SCALER_FILTER;

#[test]
fn mbxfbs_set_unpacked_packs_back_to_its_words() {
    // The format's reading, both directions: the half kernels packed low
    // byte first are mbxfb's fifteen words exactly.
    assert_eq!(Filter::MBXFB.words(), SCALER_FILTER);
}

#[test]
fn mbxfbs_set_sums_to_one_at_every_phase() {
    // The evidence for the reading: a windowed sinc whose taps sum to 64
    // at all eight phases, both directions, and is 0 at whole pixels.
    assert_eq!(phase_sums(&Filter::MBXFB.vertical, 4), [ONE; 8]);
    assert_eq!(phase_sums(&Filter::MBXFB.horizontal, 8), [ONE; 8]);
    for d in [8, 16] {
        assert_eq!(Filter::MBXFB.vertical[V_LEN - 1 - d], 0);
    }
    for d in [8, 16, 24, 32] {
        assert_eq!(Filter::MBXFB.horizontal[H_LEN - 1 - d], 0);
    }
}

#[test]
fn fitted_sets_sum_to_one_at_every_phase_at_every_scale() {
    // Every output size from half to eight times a 480 x 640 source, a
    // step at a time near the edges (both directions, the 2:1 limit and
    // 1:1 included).
    let outs = (240..=250).chain([300, 359, 360, 361, 479, 480, 481, 960, 3840]);
    for o in outs {
        for src in [480u32, 640] {
            if src > 2 * o {
                continue;
            }
            let f = Filter::fitted((src, src), (o, o));
            assert_eq!(phase_sums(&f.horizontal, 8), [ONE; 8], "{src} -> {o}");
            assert_eq!(phase_sums(&f.vertical, 4), [ONE; 8], "{src} -> {o}");
            // Symmetric kernels, so the centre is the largest weight.
            assert!(f.horizontal.iter().all(|&w| w <= f.horizontal[H_LEN - 1]));
            assert!(f.vertical.iter().all(|&w| w <= f.vertical[V_LEN - 1]));
        }
    }
}

#[test]
fn fitted_shrinks_with_a_wider_kernel_and_enlarges_as_at_one_to_one() {
    let one = Filter::fitted((480, 640), (480, 640));
    // Enlarging uses the 1:1 kernel, whatever the factor.
    for out in [(481, 641), (960, 1280), (2048, 2048)] {
        assert_eq!(Filter::fitted((480, 640), out), one, "{out:?}");
    }
    // 1:1: Keys' cubic, zero past two pixels and at whole pixels.
    assert!(one.horizontal[..H_LEN - 17].iter().all(|&w| w == 0));
    for d in [8, 16] {
        assert_eq!(one.horizontal[H_LEN - 1 - d], 0);
        assert_eq!(one.vertical[V_LEN - 1 - d], 0);
    }
    assert_eq!(one.horizontal[H_LEN - 1], ONE as i8);
    // Half size vertically: a tent over two source pixels each side, a
    // straight ramp 0, 2, ... 32 (exact before the phase fix-up).
    let half = Filter::fitted((480, 640), (240, 320));
    let ramp: [i8; V_LEN] = core::array::from_fn(|k| (2 * k) as i8);
    assert_eq!(half.vertical, ramp);
    // Half size across: the cubic stretched to four pixels each side, so
    // it reaches the outermost taps mbxfb's 1:1 kernel leaves at 0, and
    // its centre is half the 1:1 one's.
    assert!(half.horizontal[..8].iter().any(|&w| w != 0));
    assert_eq!(half.horizontal[H_LEN - 1], 32);
    // Each direction its own factor.
    let wide = Filter::fitted((480, 640), (240, 640));
    assert_eq!(wide.horizontal, half.horizontal);
    assert_eq!(wide.vertical, one.vertical);
}

#[test]
fn stretched_sets_sum_to_one_and_widen_with_the_stretch() {
    // 4/3 is fitted's at 3/4 size; 2 is fitted's at half size.
    assert_eq!(
        Filter::stretched(4, 3),
        Filter::fitted((480, 640), (360, 480))
    );
    assert_eq!(
        Filter::stretched(2, 1),
        Filter::fitted((480, 640), (240, 320))
    );
    let mut centre = i8::MAX;
    for (num, den) in [(4, 3), (5, 3), (2, 1)] {
        let f = Filter::stretched(num, den);
        assert_eq!(phase_sums(&f.horizontal, 8), [ONE; 8], "{num}/{den}");
        assert_eq!(phase_sums(&f.vertical, 4), [ONE; 8], "{num}/{den}");
        // Wider: the centre weight falls as the stretch grows.
        assert!(f.horizontal[H_LEN - 1] < centre, "{num}/{den}");
        centre = f.horizontal[H_LEN - 1];
    }
}

#[test]
#[should_panic(expected = "shrinks past 2:1")]
fn stretched_refuses_past_two() {
    let _ = Filter::stretched(7, 3);
}

#[test]
#[should_panic(expected = "a stretch under 1")]
fn stretched_refuses_under_one() {
    let _ = Filter::stretched(2, 3);
}

#[test]
#[should_panic(expected = "shrinks past 2:1")]
fn fitted_refuses_past_two_to_one() {
    let _ = Filter::fitted((480, 640), (239, 640));
}

#[test]
#[should_panic(expected = "a size of 0")]
fn fitted_refuses_a_size_of_zero() {
    let _ = Filter::fitted((480, 640), (480, 0));
}

#[test]
fn the_proof_sets_are_what_they_say() {
    // Nearest: the centre alone, so every phase is one tap of 64.
    let n = Filter::NEAREST;
    assert_eq!(n.vertical[V_LEN - 1], 64);
    assert_eq!(n.vertical.iter().map(|&w| i32::from(w)).sum::<i32>(), 64);
    assert_eq!(n.horizontal.iter().map(|&w| i32::from(w)).sum::<i32>(), 64);
    assert_eq!(phase_sums(&n.horizontal, 8)[0], 64);
    assert_eq!(phase_sums(&n.horizontal, 8)[1..], [0; 7]);
    // Halved: every phase sums to about 32 (half brightness if 64 is one).
    let h = Filter::MBXFB.halved();
    for s in phase_sums(&h.horizontal, 8)
        .into_iter()
        .chain(phase_sums(&h.vertical, 4))
    {
        assert!((30..=34).contains(&s), "{s}");
    }
    // Words: SVCTRL first, then the coefficients in register order.
    let w = Filter::NEAREST.words();
    assert_eq!(w[0], (crate::layers::reg::SVCTRL, SVCTRL_MBXFB));
    assert_eq!(w[5], (crate::layers::reg::VSCOEFF0 + 16, 0x40));
    assert_eq!(w[14], (crate::layers::reg::HSCOEFF0 + 32, 0x40));
    assert!(w[1..5].iter().chain(&w[6..14]).all(|&(_, v)| v == 0));
}

/// One phase's taps of a half kernel, in tap order (as the hardware
/// applies them: offsets `8 i - p` eighths).
fn taps(half: &[i8], taps: i32, p: i32) -> std::vec::Vec<(i32, f64)> {
    let n = half.len() as i32 - 1;
    (-taps..=taps)
        .map(|i| 8 * i - p)
        .filter(|d| d.abs() <= n)
        .map(|d| (i32::from(half[(n - d.abs()) as usize]), f64::from(d) / 8.0))
        .collect()
}

/// The largest |response| over the phases at `f` cycles per source
/// pixel, one being 1.
fn worst(half: &[i8], t: i32, f: f64) -> f64 {
    (0..8)
        .map(|p| {
            let (mut re, mut im) = (0.0, 0.0);
            for (w, x) in taps(half, t, p) {
                let a = 2.0 * core::f64::consts::PI * f * x;
                re += f64::from(w) * a.cos();
                im -= f64::from(w) * a.sin();
            }
            (re * re + im * im).sqrt() / 64.0
        })
        .fold(0.0, f64::max)
}

#[test]
fn designed_tables_keep_their_exact_rules() {
    use super::designed::DESIGNED;
    for ((n, d), f) in DESIGNED {
        // Every phase sums to one, both directions.
        assert_eq!(phase_sums(&f.horizontal, 8), [ONE; 8], "{n}/{d}");
        assert_eq!(phase_sums(&f.vertical, 4), [ONE; 8], "{n}/{d}");
        // Down: one-pixel lines removed exactly (even and odd taps 32
        // each, every phase).
        for p in 0..8 {
            let alt: i32 = taps(&f.vertical, 2, p)
                .iter()
                .enumerate()
                .map(|(i, (w, _))| if i % 2 == 0 { *w } else { -*w })
                .sum();
            assert_eq!(alt, 0, "{n}/{d} phase {p}");
        }
    }
    // Ratios rise, all shrinking, from 1/2.
    assert_eq!(DESIGNED[0].0, (1, 2));
    for w in DESIGNED.windows(2) {
        let ((a, b), (c, d)) = (w[0].0, w[1].0);
        assert!(u32::from(a) * u32::from(d) < u32::from(c) * u32::from(b));
        assert!(c < d);
    }
}

#[test]
fn designed_beats_the_cubic_at_three_quarters() {
    // P9d at 3/4: one-pixel stripes left by the cubic across (32-35% of
    // them) and the tent down (39%). The designed tables, at the top of
    // the band.
    let cubic = Filter::fitted((480, 640), (360, 480));
    let ours = Filter::designed((480, 640), (360, 480));
    let top = |h: &[i8], t| worst(h, t, 0.45).max(worst(h, t, 0.5));
    assert!(
        top(&cubic.horizontal, 8) > 0.3,
        "{}",
        top(&cubic.horizontal, 8)
    );
    assert!(
        top(&ours.horizontal, 8) < 0.05,
        "{}",
        top(&ours.horizontal, 8)
    );
    assert!(worst(&ours.vertical, 4, 0.5) < 0.01);
    assert!(top(&ours.vertical, 4) < top(&cubic.vertical, 4));
    // And keeps detail the smaller picture can show (0.6 of its limit):
    // across as well as the cubic or better, down better than the tent.
    let keep = |h: &[i8], t| worst(h, t, 0.6 * 0.375);
    assert!(keep(&ours.horizontal, 8) >= keep(&cubic.horizontal, 8) - 0.02);
    assert!(keep(&ours.vertical, 4) > keep(&cubic.vertical, 4));
}

#[test]
fn designed_picks_never_filtering_less_than_the_scale_needs() {
    use super::designed::DESIGNED;
    // Exactly a designed ratio: that table; just under it: the one below.
    let at = |o: u32| Filter::designed((480, 640), (o, 640)).horizontal;
    assert_eq!(at(360), DESIGNED[5].1.horizontal); // 3/4
    assert_eq!(at(359), DESIGNED[4].1.horizontal); // just under 3/4: 11/16
    assert_eq!(at(240), DESIGNED[0].1.horizontal); // 1/2
    assert_eq!(at(479), DESIGNED[8].1.horizontal); // just under 1: 15/16
    // 1:1 and enlarging other than 2x: mbxfb's half; exactly 2x: R79's
    // fitted half (the owner's ruling 2026-10-29); each direction its own.
    let fit = Filter::from_2x(FIT_2X_H, FIT_2X_V);
    assert_eq!(at(480), Filter::MBXFB.horizontal);
    assert_eq!(at(960), fit.horizontal);
    assert_eq!(at(959), Filter::MBXFB.horizontal);
    assert_eq!(at(961), Filter::MBXFB.horizontal);
    assert_eq!(at(720), Filter::MBXFB.horizontal);
    let mixed = Filter::designed((480, 640), (240, 1280));
    assert_eq!(mixed.horizontal, DESIGNED[0].1.horizontal);
    assert_eq!(mixed.vertical, fit.vertical);
    let mixed = Filter::designed((480, 640), (960, 960));
    assert_eq!(mixed.horizontal, fit.horizontal);
    assert_eq!(mixed.vertical, Filter::MBXFB.vertical);
    // The game path's smooth route: 240 x 320 to the screen, both fitted.
    assert_eq!(Filter::designed((240, 320), (480, 640)), fit);
}

#[test]
fn the_2x_fit_fills_every_phase_and_keeps_its_own_values() {
    // R79's fitted values (tools/scaler-filter/fit2x.py, 2026-10-29).
    let (h, v) = (FIT_2X_H, FIT_2X_V);
    let f = Filter::from_2x(h, v);
    assert_eq!(phase_sums(&f.horizontal, 4), [ONE; 8]);
    assert_eq!(phase_sums(&f.vertical, 2), [ONE; 8]);
    // The two phases 2x uses hold the fit itself.
    for (k, &want) in h.iter().enumerate() {
        assert_eq!(
            f.horizontal[H_LEN - 1 - (2 + 4 * k)],
            want,
            "across at {}",
            2 + 4 * k
        );
    }
    for (k, &want) in v.iter().enumerate() {
        assert_eq!(
            f.vertical[V_LEN - 1 - (2 + 4 * k)],
            want,
            "down at {}",
            2 + 4 * k
        );
    }
}
