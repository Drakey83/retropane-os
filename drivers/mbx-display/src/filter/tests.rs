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
