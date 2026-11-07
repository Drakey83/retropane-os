//! R185's look check (sheet `docs/research/sheets/r185-look-check-unattended.md`,
//! the owner's go 2026-11-06): which look, light or dark, the panel shows
//! in the Weather app, judged from a few pixels read back from what the
//! display scans out (`Graphics::shown_at`), so an unattended run sees a
//! page of the wrong look the way the owner's eyes did (run r184s3c).
//!
//! The points and their thresholds are fitted, not guessed: from the
//! emulator's screens of every Weather page in both looks (`cargo xtask
//! look-replay` driving the image, `emu-today-watch`'s shots), the points
//! whose brightness splits the two looks widest on every page, each
//! threshold halfway across its gap. A vote of the points decides; too
//! few agreeing is "unknown", never a guess.

/// A point the check reads, and how its brightness says the look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    /// Screen column.
    pub x: u16,
    /// Screen row.
    pub y: u16,
    /// The brightness (0-255, [`luma`]) between the two looks there.
    pub split: u8,
    /// Whether the light look is the brighter side (glass: yes; text on
    /// it: no).
    pub light_above: bool,
}

/// The chrome's points (the tab bar, on every page), fitted over every
/// page's shots: gaps of 187-206 brightness levels between the looks.
pub const CHROME: [Point; 5] = [
    Point {
        x: 441,
        y: 570,
        split: 130,
        light_above: true,
    },
    Point {
        x: 275,
        y: 563,
        split: 124,
        light_above: true,
    },
    Point {
        x: 390,
        y: 569,
        split: 124,
        light_above: true,
    },
    Point {
        x: 325,
        y: 564,
        split: 122,
        light_above: true,
    },
    Point {
        x: 147,
        y: 574,
        split: 120,
        light_above: true,
    },
];

/// Each page's points on its glass (Now, Hourly, 10 Day), fitted on that
/// page's shots: gaps of 68-95. Radar has none: its only glass inside the
/// page, the loading chip, goes once the map is in (its look is judged by
/// the chrome alone).
pub const PAGE: [[Point; 9]; 3] = [
    [
        Point {
            x: 219,
            y: 211,
            split: 112,
            light_above: true,
        },
        Point {
            x: 165,
            y: 183,
            split: 113,
            light_above: true,
        },
        Point {
            x: 120,
            y: 186,
            split: 111,
            light_above: true,
        },
        Point {
            x: 338,
            y: 274,
            split: 95,
            light_above: true,
        },
        Point {
            x: 413,
            y: 278,
            split: 97,
            light_above: true,
        },
        Point {
            x: 85,
            y: 279,
            split: 96,
            light_above: true,
        },
        Point {
            x: 453,
            y: 270,
            split: 97,
            light_above: true,
        },
        Point {
            x: 24,
            y: 274,
            split: 95,
            light_above: true,
        },
        Point {
            x: 140,
            y: 274,
            split: 96,
            light_above: true,
        },
    ],
    [
        Point {
            x: 299,
            y: 170,
            split: 116,
            light_above: true,
        },
        Point {
            x: 203,
            y: 246,
            split: 100,
            light_above: true,
        },
        Point {
            x: 381,
            y: 147,
            split: 125,
            light_above: true,
        },
        Point {
            x: 430,
            y: 147,
            split: 125,
            light_above: true,
        },
        Point {
            x: 375,
            y: 187,
            split: 112,
            light_above: true,
        },
        Point {
            x: 423,
            y: 190,
            split: 112,
            light_above: true,
        },
        Point {
            x: 329,
            y: 246,
            split: 102,
            light_above: true,
        },
        Point {
            x: 109,
            y: 248,
            split: 104,
            light_above: true,
        },
        Point {
            x: 408,
            y: 250,
            split: 103,
            light_above: true,
        },
    ],
    [
        Point {
            x: 361,
            y: 504,
            split: 96,
            light_above: true,
        },
        Point {
            x: 266,
            y: 505,
            split: 98,
            light_above: true,
        },
        Point {
            x: 351,
            y: 368,
            split: 121,
            light_above: true,
        },
        Point {
            x: 63,
            y: 508,
            split: 94,
            light_above: true,
        },
        Point {
            x: 318,
            y: 510,
            split: 94,
            light_above: true,
        },
        Point {
            x: 447,
            y: 513,
            split: 85,
            light_above: true,
        },
        Point {
            x: 300,
            y: 384,
            split: 86,
            light_above: true,
        },
        Point {
            x: 24,
            y: 462,
            split: 101,
            light_above: true,
        },
        Point {
            x: 256,
            y: 428,
            split: 85,
            light_above: true,
        },
    ],
];

/// The points of page `tab`'s glass (none for Radar or past it).
#[must_use]
pub fn page_points(tab: u8) -> &'static [Point] {
    PAGE.get(usize::from(tab)).map_or(&[], |p| p.as_slice())
}

/// The share of the points that must agree for a verdict (two thirds).
const AGREE_NUM: usize = 2;
const AGREE_DEN: usize = 3;

/// The brightness of 0xRRGGBB (Rec. 601 weights, integer).
#[must_use]
pub const fn luma(rgb: u32) -> u8 {
    let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
    ((r * 299 + g * 587 + b * 114 + 500) / 1000) as u8
}

/// The fewest points read for a verdict.
const MIN_READ: usize = 3;

/// The look the pixels at `points` show (`px[i]` read at `points[i]`,
/// `None` where it couldn't be read or the page doesn't cover it yet: a
/// page growing in, emulator run of 2026-11-06): `Some(true)` light,
/// `Some(false)` dark, `None` when fewer than [`MIN_READ`] were read or
/// fewer than two thirds of those read agree.
#[must_use]
pub fn verdict(points: &[Point], px: &[Option<u32>]) -> Option<bool> {
    let (mut light, mut dark) = (0usize, 0usize);
    for (p, v) in points.iter().zip(px) {
        let Some(v) = v else { continue };
        if (luma(*v) > p.split) == p.light_above {
            light += 1;
        } else {
            dark += 1;
        }
    }
    let n = light + dark;
    if n < MIN_READ {
        return None;
    }
    if light * AGREE_DEN >= n * AGREE_NUM {
        Some(true)
    } else if dark * AGREE_DEN >= n * AGREE_NUM {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: [Point; 3] = [
        Point {
            x: 0,
            y: 0,
            split: 128,
            light_above: true,
        },
        Point {
            x: 1,
            y: 0,
            split: 100,
            light_above: true,
        },
        Point {
            x: 2,
            y: 0,
            split: 90,
            light_above: false,
        },
    ];

    #[test]
    fn the_points_vote_and_too_few_agreeing_is_unknown() {
        let w = Some(0xFF_FFFF);
        let k = Some(0);
        // Light: glass bright, text dark.
        assert_eq!(verdict(&P, &[w, w, k]), Some(true));
        // Dark: the other way.
        assert_eq!(verdict(&P, &[k, k, w]), Some(false));
        // Two of three agree: enough.
        assert_eq!(verdict(&P, &[w, w, w]), Some(true));
        // One unread: too few read.
        assert_eq!(verdict(&P, &[w, None, k]), None);
        // One each way of four read... (two thirds needs three of four).
        let p4 = [P[0], P[1], P[0], P[1]];
        assert_eq!(verdict(&p4, &[w, w, k, k]), None);
        assert_eq!(verdict(&p4, &[w, w, w, k]), Some(true));
        assert_eq!(verdict(&p4, &[w, None, w, w]), Some(true));
        assert_eq!(verdict(&P, &[None, None, None]), None);
        assert_eq!(verdict(&[], &[]), None);
    }

    #[test]
    fn luma_spans_black_to_white() {
        assert_eq!(luma(0), 0);
        assert_eq!(luma(0xFF_FFFF), 255);
        assert_eq!(luma(0x00_FF00), 150);
    }
}
