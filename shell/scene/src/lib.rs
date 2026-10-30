//! Test image `rp-scene` (R110; the owner's go 2026-10-29 on its sheet,
//! `docs/research/sheets/r110-service-device-run.md`, option C): the
//! graphics service itself (`rp-graphics`, not a bench harness) drives a 3D
//! scene on the X51v, with a 2D bar beside it, so the shipped code is proven
//! as shipped: the scan race (frames started behind the scan), the copy
//! fallback (rendered into screen 1, copied into screen 0 behind the scan),
//! the start-up copy timing (audit C94) and the stall counters.
//!
//! Four scenes through the service's quads (flat, Gouraud, textured and
//! bilinear, blended, depth-tested), each with its own bar colour: four
//! whole pictures, each with its own `DSIG` (measured settled at each
//! phase's start). Frame `n` shows picture `n % 4`; every refresh is judged
//! against the pictures it may show (`rp_drv_mbx_display::sig`). Three
//! phases ([`PHASES`]): Windows Mobile's clock, then ADR-0035's top rung
//! (149.50 / 2, `Graphics::clock_changed`), then the checkerboard order
//! (no frame the scan can lead: every frame by the copy). Each phase runs
//! frames back to back and one-off (asked for at three scan lines).
//!
//! This half holds what the host can test: the scenes, the plan, the tally.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_graphics::{Fill, Handle, Quad};
use rp_mbx3d_render::{self as r3d, Depth, depth, factor};

/// Printed (serial and run log) once every phase has run.
pub const MARKER: &str = "RETROPANE-SCENE-OK";

/// The scene's area `(x, y, w, h)`: whole 8 x 16 tiles (`open_scene`), the
/// screen's width, under the label.
pub const AREA: (u16, u16, u16, u16) = (0, 96, 480, 448);
/// The 2D bar under the area, its colour following the frame.
pub const BAR: (u16, u16, u16, u16) = (0, 560, 480, 48);
/// The bar's colour per picture: exact in RGB565 (the screens' format), so
/// the four pictures differ whatever the scenes draw.
pub const BAR_COLOURS: [u32; SCENES] = [0xFFFF_0000, 0xFF00_FF00, 0xFF00_00FF, 0xFFFF_FFFF];
/// The scenes' backgrounds.
pub const BACKGROUNDS: [u32; SCENES] = [0xFF10_2040, 0xFF20_2020, 0xFF00_3018, 0xFF30_1010];
/// The pictures (scenes) in turn.
pub const SCENES: usize = 4;
/// Quad slots a scene uses (the rest emptied).
pub const SLOTS: usize = 6;
/// The label's rows at the top of the wallpaper.
pub const LABEL_ROWS: u16 = 80;

/// A phase's clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clock {
    /// Windows Mobile's (99.67 / 2).
    Wm,
    /// ADR-0035's top rung (149.50 / 2).
    Top,
    /// 143.00 / 1 (R121's device check: divide 1, held by ADR-0035
    /// amendment D, so only in the [`DIVIDE1`] variant).
    Divide1,
}

/// One run of frames at one clock and tile order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Phase {
    /// Its number in the log (`P{n}`; the 2D flips are P4).
    pub n: u8,
    /// For the log.
    pub name: &'static str,
    /// For the label (capitals: the font's).
    pub label: &'static str,
    /// Its clock.
    pub clock: Clock,
    /// The checkerboard tile order, else rows (top down).
    pub checker: bool,
    /// `Graphics::sdram_cures` (R121: the underlay and the alpha-0 overlay
    /// on renders into screen 1).
    pub cures: bool,
}

/// The phases, in order: the clock moves once, between the first two.
pub const PHASES: [Phase; 3] = [
    Phase {
        n: 1,
        name: "Windows Mobile's clock (99.67 / 2), rows",
        label: "WM CLOCK, ROWS",
        clock: Clock::Wm,
        checker: false,
        cures: false,
    },
    Phase {
        n: 2,
        name: "149.50 / 2, rows",
        label: "149.50 / 2, ROWS",
        clock: Clock::Top,
        checker: false,
        cures: false,
    },
    Phase {
        n: 3,
        name: "149.50 / 2, checkerboard (every frame by the copy)",
        label: "149.50 / 2, CHECKER",
        clock: Clock::Top,
        checker: true,
        cures: false,
    },
];

/// The `RP_SCENE_DIVIDE1` variant's phases (R121's device check through
/// the service, the owner's go 2026-10-30; the sheet
/// `docs/research/sheets/r121-divide1-display-on.md`): R110's P3 (149.50
/// / 2, every frame by the copy: the references P5 is compared with; P1
/// and P2 are proven by R110's runs, and the run log holds four phases),
/// then (after the 2D flips, P4) three at 143.00 / 1: every frame by the copy
/// into SDRAM with the cures; rows, frames led behind the scan (cures off,
/// on die) mixed with the copy's (cures on); the copy without the cures
/// (the control: some refreshes should come out wrong).
pub const DIVIDE1: [Phase; 4] = [
    PHASES[2],
    Phase {
        n: 5,
        name: "143.00 / 1, checkerboard, the cures on (every frame by the copy into SDRAM)",
        label: "143 / 1, CHECKER, CURES",
        clock: Clock::Divide1,
        checker: true,
        cures: true,
    },
    Phase {
        n: 6,
        name: "143.00 / 1, rows, the cures on (behind the scan and by the copy)",
        label: "143 / 1, ROWS, CURES",
        clock: Clock::Divide1,
        checker: false,
        cures: true,
    },
    Phase {
        n: 7,
        name: "143.00 / 1, checkerboard, the cures off (the control)",
        label: "143 / 1, CHECKER, NO CURE",
        clock: Clock::Divide1,
        checker: true,
        cures: false,
    },
];

/// The phases a build runs: [`DIVIDE1`]'s with `divide1`, else
/// [`PHASES`].
#[must_use]
pub const fn plan(divide1: bool) -> &'static [Phase] {
    if divide1 { &DIVIDE1 } else { &PHASES }
}

/// Frames back to back per phase.
pub const B2B: u32 = 150;
/// One-off frames per phase, each asked for at the next of [`ASK_LINES`].
pub const ONE_OFF: u32 = 50;
/// The scan lines one-off frames are asked for at (top, middle, low).
pub const ASK_LINES: [u32; 3] = [0, 213, 426];

/// The picture frame `n` shows.
#[must_use]
pub const fn picture(n: u32) -> usize {
    (n % SCENES as u32) as usize
}

/// A `w` x `h` quad at `(x, y)` (pixels, the area's).
const fn rect(x: i32, y: i32, w: i32, h: i32, fill: Fill) -> Quad {
    Quad::rect(x, y, w, h, fill)
}

/// The 8 x 8 texture ([`texels`]) magnified across 320 pixels, bilinear.
#[must_use]
pub const fn textured(tex: Handle) -> Fill {
    Fill::Texture {
        tex,
        uv: [
            (0, 0),
            (r3d::UV_ONE, 0),
            (r3d::UV_ONE, r3d::UV_ONE),
            (0, r3d::UV_ONE),
        ],
        tint: 0xFFFF_FFFF,
        u_mode: r3d::address::CLAMP,
        v_mode: r3d::address::CLAMP,
        linear: true,
    }
}

/// Scene `k`'s quads slot by slot, `tex` its textured fill ([`textured`]).
#[must_use]
pub fn scene(k: usize, tex: Fill) -> [Option<Quad>; SLOTS] {
    let (w, h) = (i32::from(AREA.2), i32::from(AREA.3));
    let mut q = [None; SLOTS];
    match k % SCENES {
        // Flat and Gouraud.
        0 => {
            q[0] = Some(rect(
                0,
                0,
                w,
                h,
                Fill::Gouraud([0xFF20_4080, 0xFF80_4020, 0xFF20_8040, 0xFF40_2080]),
            ));
            q[1] = Some(rect(40, 40, 120, 120, Fill::Flat(0xFFE0_E000)));
            q[2] = Some(rect(180, 160, 120, 120, Fill::Flat(0xFF00_E0E0)));
            q[3] = Some(rect(320, 280, 120, 120, Fill::Flat(0xFFE0_00E0)));
        }
        // The texture magnified, bilinear, inside a flat frame.
        1 => {
            q[0] = Some(rect(72, 56, 336, 336, Fill::Flat(0xFFC0_C0C0)));
            q[1] = Some(rect(80, 64, 320, 320, tex));
        }
        // Translucent quads over a Gouraud field.
        2 => {
            q[0] = Some(rect(
                0,
                0,
                w,
                h,
                Fill::Gouraud([0xFF00_0000, 0xFFFF_FFFF, 0xFF00_0000, 0xFFFF_FFFF]),
            ));
            for (i, c) in [0x80FF_0000u32, 0x8000_FF00, 0x8000_00FF]
                .into_iter()
                .enumerate()
            {
                let o = 60 + 90 * i as i32;
                q[1 + i] = Some(Quad {
                    blend: Some((factor::SRC_ALPHA, factor::INV_SRC_ALPHA)),
                    ..rect(o, o, 200, 200, Fill::Flat(c))
                });
            }
        }
        // Depth: the nearer quads drawn last are cut by the farther ones
        // drawn first where the test fails.
        _ => {
            for (i, (c, z)) in [
                (0xFFFF_8000u32, 0x2000u16),
                (0xFF00_80FF, 0x6000),
                (0xFF80_FF00, 0x4000),
            ]
            .into_iter()
            .enumerate()
            {
                let o = 60 + 70 * i as i32;
                let mode = if i == 0 { depth::ALWAYS } else { depth::LESS };
                q[i] = Some(Quad {
                    depth: Some(Depth {
                        mode,
                        write: true,
                        z,
                    }),
                    ..rect(o, o, 220, 220, Fill::Flat(c))
                });
            }
        }
    }
    q
}

/// The 8 x 8 texture (RGB565): a checker of two colours with a red corner,
/// so a flip or a shift shows.
#[must_use]
pub fn texels() -> [u16; 64] {
    let mut t = [0u16; 64];
    for (i, px) in t.iter_mut().enumerate() {
        let (x, y) = (i % 8, i / 8);
        *px = if x == 0 && y == 0 {
            0xF800
        } else if (x + y) % 2 == 0 {
            0xFFE0
        } else {
            0x001F
        };
    }
    t
}

/// Times (microseconds) kept for a median and the worst.
#[derive(Clone, Copy, Debug)]
pub struct Times {
    v: [u32; 64],
    n: usize,
    /// Every time added (past the first 64 only the worst counts).
    pub count: u32,
    /// The worst.
    pub worst: u32,
}

impl Times {
    /// None yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            v: [0; 64],
            n: 0,
            count: 0,
            worst: 0,
        }
    }

    /// One more.
    pub fn add(&mut self, us: u32) {
        if self.n < self.v.len() {
            self.v[self.n] = us;
            self.n += 1;
        }
        self.count += 1;
        self.worst = self.worst.max(us);
    }

    /// The median of the first 64 (`None`: none).
    #[must_use]
    pub fn median(&self) -> Option<u32> {
        if self.n == 0 {
            return None;
        }
        let mut s = self.v;
        s[..self.n].sort_unstable();
        Some(s[self.n / 2])
    }
}

impl Default for Times {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The textured fill's stand-in (a handle comes only from the service).
    const TEX: Fill = Fill::Flat(0x1234_5678);

    fn rgb565(c: u32) -> u16 {
        ((c >> 8 & 0xF800) | (c >> 5 & 0x07E0) | (c >> 3 & 0x001F)) as u16
    }

    #[test]
    fn the_four_pictures_differ_in_the_screens_format() {
        // Each bar colour exact in RGB565 and unlike the others: the four
        // composites differ whatever the scenes draw, so their DSIGs can.
        for (i, &a) in BAR_COLOURS.iter().enumerate() {
            let back = rgb565(a);
            let (r, g, b) = (
                u32::from(back >> 11) * 255 / 31,
                u32::from(back >> 5 & 63) * 255 / 63,
                u32::from(back & 31) * 255 / 31,
            );
            let wide = 0xFF00_0000 | r << 16 | g << 8 | b;
            assert_eq!(wide, a, "bar {i} not exact in RGB565");
            for &b in &BAR_COLOURS[i + 1..] {
                assert_ne!(rgb565(a), rgb565(b));
            }
        }
        assert_eq!(
            (0..8).map(picture).collect::<std::vec::Vec<_>>(),
            [0, 1, 2, 3, 0, 1, 2, 3]
        );
    }

    #[test]
    fn every_quad_lies_inside_the_area_and_the_area_is_whole_tiles() {
        let (x, y, w, h) = AREA;
        assert_eq!((x % 8, w % 8, y % 16, h % 16), (0, 0, 0, 0));
        assert!(y >= LABEL_ROWS, "the label stays 2D's");
        assert!(
            u32::from(y) + u32::from(h) <= u32::from(BAR.1),
            "the bar outside the area"
        );
        assert!(u32::from(BAR.1) + u32::from(BAR.3) <= 640);
        for k in 0..SCENES {
            let qs = scene(k, TEX);
            assert!(qs.iter().any(Option::is_some), "scene {k} empty");
            for q in qs.iter().flatten() {
                for p in q.corners {
                    // 1/16 pixel.
                    assert!((0..=i32::from(w) * 16).contains(&p.x), "scene {k}: {p:?}");
                    assert!((0..=i32::from(h) * 16).contains(&p.y), "scene {k}: {p:?}");
                }
            }
        }
        // Each kind the sheet names is drawn: Gouraud, texture, blend, depth.
        let kinds = |k: usize| scene(k, TEX);
        assert!(
            kinds(0)
                .iter()
                .flatten()
                .any(|q| matches!(q.fill, Fill::Gouraud(_)))
        );
        assert!(kinds(1).iter().flatten().any(|q| q.fill == TEX));
        assert!(kinds(2).iter().flatten().any(|q| q.blend.is_some()));
        assert!(
            kinds(3)
                .iter()
                .flatten()
                .filter(|q| q.depth.is_some())
                .count()
                >= 2
        );
    }

    #[test]
    fn the_plan_moves_the_clock_once_and_ends_on_the_checkerboard() {
        assert_eq!(
            PHASES.map(|p| (p.clock, p.checker, p.cures)),
            [
                (Clock::Wm, false, false),
                (Clock::Top, false, false),
                (Clock::Top, true, false)
            ]
        );
        assert_eq!(plan(false), &PHASES[..]);
        // The sheet's 600 frames.
        assert_eq!(PHASES.len() as u32 * (B2B + ONE_OFF), 600);
        assert!(ASK_LINES.iter().all(|&l| l < 645));
    }

    #[test]
    fn the_divide_1_plan_adds_the_cured_copy_the_switching_and_the_control() {
        // R121's sheet: R110's P3 (the references), then at 143.00 / 1 the
        // copy with the cures, rows with the cures (both targets), and the
        // copy without (the control); numbered after the flips (P4).
        let d = plan(true);
        assert_eq!(d[0], PHASES[2]);
        assert_eq!(
            d[1..]
                .iter()
                .map(|p| (p.clock, p.checker, p.cures))
                .collect::<std::vec::Vec<_>>(),
            [
                (Clock::Divide1, true, true),
                (Clock::Divide1, false, true),
                (Clock::Divide1, true, false)
            ]
        );
        assert_eq!(
            d.iter().map(|p| p.n).collect::<std::vec::Vec<_>>(),
            [3, 5, 6, 7]
        );
        assert_eq!(PHASES.map(|p| p.n), [1, 2, 3]);
        // The label: 3x glyphs of 6 pixels from x 12 across 480: 26 a line.
        assert!(d.iter().all(|p| p.label.len() <= 26), "the label fits");
    }

    #[test]
    fn times_give_the_median_and_the_worst() {
        let mut t = Times::new();
        assert_eq!(t.median(), None);
        for us in [30, 10, 20] {
            t.add(us);
        }
        assert_eq!((t.median(), t.worst, t.count), (Some(20), 30, 3));
        for _ in 0..100 {
            t.add(5);
        }
        t.add(99);
        assert_eq!(
            (t.count, t.worst),
            (104, 99),
            "past 64 only the worst and the count"
        );
    }
}
