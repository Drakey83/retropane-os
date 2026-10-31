//! Map tiles for the radar (the radar built for real, 2026-10-31): which
//! web-Mercator tiles (256 px, zoom [`ZOOM`]) cover a 480 x 462 view
//! centred on the place shown, and copying each decoded tile into the view.
//! The base map is CARTO's dark tiles and the rain RainViewer's frames,
//! both fetched by the dock's relay and decoded on the device ([`crate::png`]).
//!
//! No floats (the X51v has no FPU): Mercator's y comes from a table of
//! ln(tan(45 + lat/2)) at each whole degree, made on the PC, interpolated;
//! at zoom 7 it is within a pixel of the exact formula (the tests).

/// The zoom: a 256 px tile is ~310 km at the equator; the view ~580 km.
pub const ZOOM: u8 = 7;
/// The view's size (the radar page).
pub const VIEW_W: usize = 480;
/// See [`VIEW_W`].
pub const VIEW_H: usize = 462;
/// Tiles across and down that always cover the view.
pub const GRID: usize = 3;

/// ln(tan(pi/4 + lat/2)) x 1e6 at 0..=85 degrees.
const MERC: [i64; 86] = [
    0, 17454, 34914, 52384, 69870, 87377, 104912, 122478, 140082, 157730, 175426, 193177, 210988,
    228865, 246814, 264842, 282955, 301158, 319458, 337863, 356379, 375012, 393771, 412663, 431695,
    450875, 470213, 489715, 509392, 529253, 549306, 569563, 590033, 610728, 631658, 652837, 674275,
    695988, 717988, 740290, 762910, 785863, 809167, 832841, 856903, 881374, 906275, 931632, 957467,
    983808, 1010683, 1038123, 1066162, 1094833, 1124177, 1154235, 1185051, 1216675, 1249161,
    1282567, 1316958, 1352405, 1388986, 1426788, 1465908, 1506454, 1548547, 1592324, 1637939,
    1685568, 1735415, 1787712, 1842730, 1900787, 1962257, 2027589, 2097324, 2172122, 2252803,
    2340401, 2436246, 2542090, 2660306, 2794219, 2948700, 3131301,
];

/// The world's size in pixels at [`ZOOM`].
const WORLD: i64 = 256 << ZOOM;

/// Mercator y for `lat` (thousandths of a degree), x 1e6.
fn merc(lat: i32) -> i64 {
    let a = i64::from(lat.unsigned_abs().min(84_999));
    let (d, f) = ((a / 1000) as usize, a % 1000);
    let m = MERC[d] + (MERC[d + 1] - MERC[d]) * f / 1000;
    if lat < 0 { -m } else { m }
}

/// The world pixel at zoom [`ZOOM`] of a position (thousandths of a degree).
#[must_use]
pub fn world_px(lat: i32, lon: i32) -> (i64, i64) {
    let x = (i64::from(lon) + 180_000) * WORLD / 360_000;
    // y = (1 - m / pi) / 2 * WORLD
    let y = WORLD / 2 - merc(lat) * WORLD / 2 / 3_141_593;
    (x, y)
}

/// The view centred on a place: its first tile and where the view's top
/// left falls in it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct View {
    /// The top-left tile's column and row.
    pub tx: i64,
    /// See [`View::tx`].
    pub ty: i64,
    /// The view's top left inside that tile, pixels.
    pub ox: i64,
    /// See [`View::ox`].
    pub oy: i64,
}

impl View {
    /// The view centred on (`lat`, `lon`).
    #[must_use]
    pub fn centred(lat: i32, lon: i32) -> Self {
        let (x, y) = world_px(lat, lon);
        let (vx, vy) = (x - VIEW_W as i64 / 2, y - VIEW_H as i64 / 2);
        Self {
            tx: vx.div_euclid(256),
            ty: vy.div_euclid(256),
            ox: vx.rem_euclid(256),
            oy: vy.rem_euclid(256),
        }
    }

    /// Tile `k` of the grid (row by row): its column and row, wrapped
    /// round the world sideways; `None` past the poles.
    #[must_use]
    pub fn tile(&self, k: usize) -> Option<(i64, i64)> {
        let n = 1i64 << ZOOM;
        let (x, y) = (self.tx + (k % GRID) as i64, self.ty + (k / GRID) as i64);
        (0..n).contains(&y).then_some((x.rem_euclid(n), y))
    }

    /// Tile `k`'s 256 x 256 ARGB pixels into the view's.
    pub fn place(&self, k: usize, tile: &[u32], view: &mut [u32]) {
        let (dx, dy) = (
            (k % GRID) as i64 * 256 - self.ox,
            (k / GRID) as i64 * 256 - self.oy,
        );
        for ty in 0..256i64 {
            let vy = dy + ty;
            if !(0..VIEW_H as i64).contains(&vy) {
                continue;
            }
            let x0 = dx.max(0);
            let x1 = (dx + 256).min(VIEW_W as i64);
            if x1 <= x0 {
                continue;
            }
            let src = &tile[(ty * 256 + x0 - dx) as usize..(ty * 256 + x1 - dx) as usize];
            let at = vy as usize * VIEW_W;
            view[at + x0 as usize..at + x1 as usize].copy_from_slice(src);
        }
    }
}

/// A light map's pixel made dark, hue kept: lightness turned over
/// (`c + 255 - max - min` on each channel is an inversion followed by a
/// half turn of hue, in one add), lifted 14 so land is not black. CARTO's
/// dark tiles went behind a key (2026-10-31, every tile its "API key
/// required" picture); OpenStreetMap's standard tiles, free and kept up
/// everywhere, become a dark map this way on the device: white land to
/// near-black, blue water to dark teal, red roads stay red.
#[must_use]
pub fn dark(p: u32) -> u32 {
    let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
    let max = if r > g {
        if r > b { r } else { b }
    } else if g > b {
        g
    } else {
        b
    };
    let min = if r < g {
        if r < b { r } else { b }
    } else if g < b {
        g
    } else {
        b
    };
    // c >= min and 269 > max: never below zero, in this order.
    let lift = |c: u32| (c + 255 + 14 - max - min).min(255);
    (p & 0xFF00_0000) | lift(r) << 16 | lift(g) << 8 | lift(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_light_map_turns_dark_with_its_hues_kept() {
        // OpenStreetMap's land, water and a main road.
        assert_eq!(dark(0xFFF2_EFE9), 0xFF24_211B);
        assert_eq!(dark(0xFFAA_D3DF), 0xFF2E_5763);
        let road = dark(0xFFE8_92A2);
        assert!(
            (road >> 16) & 0xFF > (road >> 8) & 0xFF,
            "red stays red: {road:08x}"
        );
        // Black and white trade places; alpha is kept; nothing overflows.
        assert_eq!(dark(0xFF00_0000), 0xFFFF_FFFF);
        assert_eq!(dark(0x80FF_FFFF), 0x800E_0E0E);
    }

    #[test]
    fn the_table_is_within_a_pixel_of_mercator() {
        for (lat, lon) in [
            (30_664, -97_922),
            (47_606, -122_332),
            (59_913, 10_752),
            (-33_868, 151_209),
            (0, 0),
            (64_150, -21_940),
        ] {
            let (x, y) = world_px(lat, lon);
            let (la, lo) = (f64::from(lat) / 1000.0, f64::from(lon) / 1000.0);
            let w = WORLD as f64;
            let ex = (lo + 180.0) / 360.0 * w;
            let ey = (1.0 - la.to_radians().tan().asinh() / core::f64::consts::PI) / 2.0 * w;
            assert!(
                (x as f64 - ex).abs() <= 1.0 && (y as f64 - ey).abs() <= 1.0,
                "{lat},{lon}: {x},{y} vs {ex},{ey}"
            );
        }
    }

    #[test]
    fn the_view_covers_itself_and_the_place_is_its_centre_pixel() {
        let v = View::centred(30_664, -97_922);
        let (px, py) = world_px(30_664, -97_922);
        // The place's tile is the grid's middle one (the relay's sample
        // tile, 29, 52).
        assert_eq!(v.tile(4), Some((px / 256, py / 256)));
        assert_eq!(v.tile(4), Some((29, 52)));
        let mut view = vec![0u32; VIEW_W * VIEW_H];
        for k in 0..GRID * GRID {
            let (tx, ty) = v.tile(k).unwrap();
            let tile: Vec<u32> = (0..256 * 256)
                .map(|i| ((tx * 100 + ty) as u32) << 16 | i as u32)
                .collect();
            v.place(k, &tile, &mut view);
        }
        let c = view[VIEW_H / 2 * VIEW_W + VIEW_W / 2];
        assert_eq!(c >> 16, 29 * 100 + 52);
        assert_eq!(c & 0xFFFF, ((py % 256) * 256 + px % 256) as u32);
        assert!(view.iter().all(|&p| p != 0), "every view pixel covered");
    }
}
