//! Test P (R289, docs/design/ref-rp.md): what the MBX 3D core does with an
//! object's 1/w word. Every device run before it drew with 1/w = 1.0, so
//! perspective was never seen; ref_rp needs it before its first picture.
//!
//! What Dell's driver sends is read already (2026-11-16): DoomGLES's
//! objects in `local/wm3d/rp-wm3d-v4-low12.bin` carry u, v plain (texture
//! repeats such as 36.0), 1/w as a float of the vertex's own (0.004-0.027)
//! and z as GL's window depth, `z = 1.00029 - 0.96360 x 1/w` over 369
//! vertices (worst residual 0.0002); every vertex lies on the screen (the
//! driver clips; x or y exactly 640.0 where it cut). So the question left
//! is the core's: whether it interpolates u, v with 1/w (perspective
//! correct, the PowerVR way) or straight across the screen (affine).
//!
//! **Rule 0.** The textbook test draws a checkerboard floor and has a
//! person look at it. Ours reads it from numbers in one render a case: an
//! 8 x 8 texture whose every texel has its own grey, so a pixel's grey
//! names the texel the core sampled, and probe pixels chosen where the two
//! models pick different texels by a clear margin; each probe is counted
//! against both, on the device itself, so the run's log line is the
//! verdict. The same idea for the far vertices ref_rp sends unclipped
//! (Dell never does): a flat triangle whose one edge crosses the screen,
//! its corners 600 to 6 million pixels away, probes on both sides of the
//! edge.
//!
//! Pure functions here, shared by the image and the host tests.

use rp_drv_mbx3d::tex::twiddle;

/// Half-Life's near plane (units).
pub const NEAR: f32 = 4.0;
/// Its far plane (units): the depth range z is mapped over.
pub const FAR: f32 = 4096.0;
/// The far edges of the two floors (units): a strong slope, and
/// Half-Life's whole range.
pub const FLOORS: [f32; 2] = [64.0, 4096.0];
/// Focal length (pixels), horizon row, centre column.
const FOCAL: f32 = 1800.0;
const HORIZON: f32 = 160.0;
const CX: f32 = 240.0;
/// Texels a side of the probe texture (format code 10, one byte a texel).
pub const TEXELS: u32 = 8;
/// The texture's coordinates run 0.02-0.98 across the floor.
const U0: f32 = 0.02;
const U1: f32 = 0.98;
/// A probe counts only where each model's texel coordinate lies this far
/// (in texels) from a texel's edge.
const MARGIN: f32 = 0.12;

/// A floor corner as the object holds it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Corner {
    /// Screen position (pixels).
    pub x: f32,
    /// See [`Self::x`].
    pub y: f32,
    /// GL's window depth for [`NEAR`] and [`FAR`] (Dell's form).
    pub z: f32,
    /// 1 / the eye's depth.
    pub rhw: f32,
    /// Texture coordinates, plain (Dell's form).
    pub u: f32,
    /// See [`Self::u`].
    pub v: f32,
}

/// The grey of the texel stored at twiddled index `i` (all 64 apart).
#[must_use]
pub const fn grey(i: u32) -> u32 {
    16 + 3 * i
}

/// The 64 texel bytes in storage order (index `i` at byte `i`).
#[must_use]
pub fn texture() -> [u8; 64] {
    core::array::from_fn(|i| grey(i as u32) as u8)
}

/// The corner at eye position `(xw, depth)` on a floor one unit below the
/// eye, with texture coordinates `(u, v)`.
#[must_use]
pub fn corner(xw: f32, depth: f32, u: f32, v: f32) -> Corner {
    let rhw = 1.0 / depth;
    Corner {
        x: CX + FOCAL * xw * rhw,
        y: HORIZON + FOCAL * rhw,
        z: FAR / (FAR - NEAR) - FAR * NEAR / (FAR - NEAR) * rhw,
        rhw,
        u,
        v,
    }
}

/// A floor half a unit to each side from [`NEAR`] to `far`, as two
/// triangles: near left, near right, far left; near right, far right, far
/// left.
#[must_use]
pub fn floor(far: f32) -> [[Corner; 3]; 2] {
    let nl = corner(-0.5, NEAR, U0, U1);
    let nr = corner(0.5, NEAR, U1, U1);
    let fl = corner(-0.5, far, U0, U0);
    let fr = corner(0.5, far, U1, U0);
    [[nl, nr, fl], [nr, fr, fl]]
}

fn edge(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> f32 {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
}

/// The texel a texture coordinate lands in, if it lies [`MARGIN`] clear of
/// a texel edge.
fn texel(c: f32) -> Option<u32> {
    let t = c * TEXELS as f32;
    if !(0.0..TEXELS as f32).contains(&t) {
        return None;
    }
    let k = t as u32;
    let f = t - k as f32;
    (MARGIN..=1.0 - MARGIN).contains(&f).then_some(k)
}

/// What the pixel `(px, py)` (its centre) of `floor` should show: the grey
/// of the texel a perspective-correct core samples and an affine one's,
/// each `None` where too near a texel edge to tell; `None` off the floor.
#[must_use]
pub fn expect(floor: &[[Corner; 3]; 2], px: u32, py: u32) -> Option<(Option<u32>, Option<u32>)> {
    let p = (px as f32 + 0.5, py as f32 + 0.5);
    for t in floor {
        let pos = |k: usize| (t[k].x, t[k].y);
        let area = edge(pos(0), pos(1), pos(2));
        let l = [
            edge(pos(1), pos(2), p) / area,
            edge(pos(2), pos(0), p) / area,
            edge(pos(0), pos(1), p) / area,
        ];
        if l.iter().any(|&w| w < 0.0) {
            continue;
        }
        let sum = |f: &dyn Fn(&Corner) -> f32| l[0] * f(&t[0]) + l[1] * f(&t[1]) + l[2] * f(&t[2]);
        let r = sum(&|c| c.rhw);
        let pick = |u: f32, v: f32| Some(grey(twiddle(texel(u)?, texel(v)?)));
        let persp = pick(sum(&|c| c.u * c.rhw) / r, sum(&|c| c.v * c.rhw) / r);
        let affine = pick(sum(&|c| c.u), sum(&|c| c.v));
        return Some((persp, affine));
    }
    None
}

/// The probe pixels of the floor reaching `far`: rows from just below its
/// far edge to just above its near one, seven across each.
pub fn probes(far: f32) -> impl Iterator<Item = (u32, u32)> {
    let top = HORIZON + FOCAL / far;
    let bottom = HORIZON + FOCAL / NEAR;
    [
        0.02f32, 0.04, 0.07, 0.1, 0.14, 0.19, 0.25, 0.32, 0.4, 0.49, 0.59, 0.7, 0.81, 0.9, 0.96,
    ]
    .into_iter()
    .flat_map(move |t| {
        let y = top + 2.0 + t * (bottom - top - 4.0);
        let half = 0.5 * FOCAL * (y - HORIZON) / FOCAL;
        [0.07f32, 0.21, 0.36, 0.5, 0.64, 0.79, 0.93]
            .into_iter()
            .map(move |s| ((CX - half + 2.0 * s * half) as u32, y as u32))
    })
}

/// The far-vertex cases: how far the corners lie (pixels from the
/// screen's centre).
pub const FAR_REACH: [f32; 5] = [600.0, 6_000.0, 60_000.0, 600_000.0, 6_000_000.0];

/// The edge across the screen, from (0, 640) to (480, 0): its direction,
/// and the normal into the triangle (up and left), in fifths.
const DIR: (f32, f32) = (3.0, -4.0);
const NORMAL: (f32, f32) = (-4.0, -3.0);

/// A flat triangle with one edge along the screen's diagonal and its
/// corners `reach` pixels from the centre (whole numbers, exact as floats
/// up to 2^24).
#[must_use]
pub fn far_triangle(reach: f32) -> [(f32, f32); 3] {
    // reach / 5 is whole for every case, so each product is exact.
    let at = |d: (f32, f32), k: f32| (CX + k / 5.0 * d.0, 320.0 + k / 5.0 * d.1);
    [at(DIR, -reach), at(DIR, reach), at(NORMAL, reach)]
}

/// Whether the pixel `(px, py)`'s centre lies inside every far triangle
/// (on the normal's side of the edge), or `None` within half a pixel of
/// the edge (the fill rule's business).
#[must_use]
pub fn far_inside(px: u32, py: u32) -> Option<bool> {
    let s = ((px as f32 + 0.5 - CX) * NORMAL.0 + (py as f32 + 0.5 - 320.0) * NORMAL.1) / 5.0;
    (s.abs() >= 0.5).then_some(s > 0.0)
}

/// The far triangles' probe pixels: a 6 x 8 grid over the screen and two
/// pairs hugging the edge.
pub fn far_probes() -> impl Iterator<Item = (u32, u32)> {
    let grid = (0..48u32).map(|k| (20 + 88 * (k % 6), 20 + 84 * (k / 6)));
    grid.chain([(239, 318), (241, 322), (119, 478), (121, 482)])
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    /// The texel a coordinate picks, exactly (f64), for the reference.
    fn exact(floor: &[[Corner; 3]; 2], px: u32, py: u32) -> Option<(f64, f64, f64, f64)> {
        let p = (f64::from(px) + 0.5, f64::from(py) + 0.5);
        for t in floor {
            let c = |k: usize| (f64::from(t[k].x), f64::from(t[k].y));
            let e = |a: (f64, f64), b: (f64, f64)| {
                (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
            };
            let ar = {
                let (a, b, q) = (c(0), c(1), c(2));
                (b.0 - a.0) * (q.1 - a.1) - (b.1 - a.1) * (q.0 - a.0)
            };
            let l = [e(c(1), c(2)) / ar, e(c(2), c(0)) / ar, e(c(0), c(1)) / ar];
            if l.iter().any(|&w| w < 0.0) {
                continue;
            }
            let s = |f: &dyn Fn(&Corner) -> f64| (0..3).map(|k| l[k] * f(&t[k])).sum::<f64>();
            let r = s(&|c| f64::from(c.rhw));
            return Some((
                s(&|c| f64::from(c.u) * f64::from(c.rhw)) / r,
                s(&|c| f64::from(c.v) * f64::from(c.rhw)) / r,
                s(&|c| f64::from(c.u)),
                s(&|c| f64::from(c.v)),
            ));
        }
        None
    }

    fn pick(u: f64, v: f64) -> u32 {
        grey(twiddle((u * 8.0) as u32, (v * 8.0) as u32))
    }

    /// Every probe's two expectations agree with exact arithmetic, and
    /// each floor has enough probes where the models part (the test's
    /// power): at least 15 a floor.
    #[test]
    fn probes_tell_the_models_apart_and_match_exact_arithmetic() {
        for far in FLOORS {
            let fl = floor(far);
            let mut told = 0;
            for (x, y) in probes(far) {
                let Some((p, a)) = expect(&fl, x, y) else {
                    continue;
                };
                let (pu, pv, au, av) = exact(&fl, x, y).expect("on the floor");
                if let Some(p) = p {
                    assert_eq!(p, pick(pu, pv), "perspective at ({x},{y}), far {far}");
                }
                if let Some(a) = a {
                    assert_eq!(a, pick(au, av), "affine at ({x},{y}), far {far}");
                }
                if let (Some(p), Some(a)) = (p, a) {
                    told += u32::from(p != a);
                }
            }
            assert!(
                told >= 15,
                "far {far}: only {told} probes tell the models apart"
            );
        }
    }

    /// z is GL's window depth, as Dell's DoomGLES objects (z = A + B / w,
    /// 0 at the near plane, 1 at the far one); 1/w the eye depth's inverse.
    #[test]
    fn corners_carry_dells_depth_form() {
        let n = corner(0.0, NEAR, 0.0, 0.0);
        let f = corner(0.0, FAR, 0.0, 0.0);
        assert!(n.z.abs() < 1e-6 && (f.z - 1.0).abs() < 1e-6);
        assert!((n.rhw - 0.25).abs() < 1e-7);
    }

    /// The far triangles' corners are whole numbers (exact floats) and
    /// the probes clear of the edge include both sides.
    #[test]
    fn far_corners_are_exact_and_probes_cover_both_sides() {
        for reach in FAR_REACH {
            for (x, y) in far_triangle(reach) {
                assert_eq!(x, x.trunc());
                assert_eq!(y, y.trunc());
                assert!(x.abs() < 16_777_216.0 && y.abs() < 16_777_216.0);
            }
        }
        let sides: std::vec::Vec<bool> =
            far_probes().filter_map(|(x, y)| far_inside(x, y)).collect();
        assert!(sides.iter().filter(|&&s| s).count() >= 15);
        assert!(sides.iter().filter(|&&s| !s).count() >= 15);
    }
}
