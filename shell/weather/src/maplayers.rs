//! The radar's base map as layers, coloured on the device (R184 step 1,
//! sheet docs/research/sheets/r184-radar-map-looks.md, way C; the owner's
//! go 2026-11-04).
//!
//! The PC draws a map once, as layers at the radar view's own pixels
//! (`tools/maps/radar_layers.py`, from Natural Earth and open elevation
//! data): how much of each pixel is land, the terrain's shade, and how much
//! of it each line covers (rivers, borders, roads, the coast, the labels'
//! halo and text). A look (Paper, Ink, Night) is a palette applied to those
//! layers, so every look comes from one file, a look change needs no card
//! read, and nothing is lost to a recolour (Ink made from Night lost the
//! terrain, swelled the inlets and broke thin roads: the owner, 2026-11-04).
//!
//! The file, `RPML`: the magic, a version, the width and height (each two
//! bytes, little-endian), the layer count and each layer's kind, then one
//! zlib stream of the layers one after another, each row stored as the
//! differences from its left neighbour. Every field is checked: it comes
//! from a card or the dock (docs/security.md, hostile input).

/// The file's first bytes.
pub const MAGIC: &[u8; 4] = b"RPML";
/// The format's version.
pub const VERSION: u8 = 1;
/// The view's width and height (the radar's page, as
/// [`crate::tiles::VIEW_W`] and [`crate::tiles::VIEW_H`]).
pub const W: usize = crate::tiles::VIEW_W;
/// See [`W`].
pub const H: usize = crate::tiles::VIEW_H;
/// The layers, in the file's order.
pub const LAYERS: usize = 8;
/// Bytes of the layers unpacked.
pub const BYTES: usize = W * H * LAYERS;
/// The header's length.
pub const HEAD: usize = 4 + 1 + 2 + 2 + 1 + LAYERS;

/// A layer (its index in the file).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Layer {
    /// How much of the pixel is land (lakes are water).
    Land = 0,
    /// The terrain's light, 0 (in shadow) to 255 (facing the light).
    Shade = 1,
    /// Rivers.
    River = 2,
    /// State, province and country lines.
    Border = 3,
    /// Roads: major ones whole, minor ones at half.
    Road = 4,
    /// The coast's line.
    Coast = 5,
    /// The labels' halo (behind their text).
    Halo = 6,
    /// The labels' text and the places' dots.
    Text = 7,
}

/// Why a file was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// Not an `RPML` file of this version, size and layer list.
    Header,
    /// The layers' stream damaged or the wrong length.
    Stream,
    /// The output the wrong size.
    Room,
}

/// Unpacks a `RPML` file into `out` ([`BYTES`]: the layers one after
/// another, a byte a pixel).
///
/// # Errors
/// A header that isn't this format's, a damaged stream, or `out` not
/// [`BYTES`] long.
pub fn unpack(file: &[u8], out: &mut [u8]) -> Result<(), Error> {
    if out.len() != BYTES {
        return Err(Error::Room);
    }
    let head = file.get(..HEAD).ok_or(Error::Header)?;
    let kinds_ok = head[10..]
        .iter()
        .enumerate()
        .all(|(k, &kind)| usize::from(kind) == k);
    if &head[..4] != MAGIC
        || head[4] != VERSION
        || usize::from(u16::from_le_bytes([head[5], head[6]])) != W
        || usize::from(u16::from_le_bytes([head[7], head[8]])) != H
        || usize::from(head[9]) != LAYERS
        || !kinds_ok
    {
        return Err(Error::Header);
    }
    match rp_inflate::zlib(&file[HEAD..], out) {
        Ok(n) if n == BYTES => {}
        _ => return Err(Error::Stream),
    }
    for row in out.as_chunks_mut::<W>().0 {
        for x in 1..W {
            row[x] = row[x].wrapping_add(row[x - 1]);
        }
    }
    Ok(())
}

/// A look of the map.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Look {
    /// Dark land and water, under the smoked glass (the dark mode's map).
    Night,
    /// Warm light land, steel-blue water (the light mode's map).
    Paper,
    /// The newspaper look: off-white land with its terrain, charcoal
    /// water, black lines.
    Ink,
}

/// A look's colours: water, land, how the shade lights the land (the
/// land's colour times `lo + span * shade / 255`, in 256ths), and each
/// line layer's colour and greatest opacity (River to Text, in order).
struct Palette {
    water: u32,
    land: u32,
    lo: u32,
    span: u32,
    lines: [(u32, u32); 6],
}

const fn palette(look: Look) -> Palette {
    match look {
        // The Night map of the mock-ups (radar_basemaps.py), its roads
        // dimmed from white to a light grey (M2: the rain stays the
        // brightest thing on the map).
        Look::Night => Palette {
            water: 0x06_0E18,
            land: 0x22_282E,
            lo: 141,
            span: 230,
            lines: [
                (0x28_506E, 255),
                (0xFF_FFFF, 70),
                (0xB4_BCC4, 70),
                (0x78_B4D7, 220),
                (0x00_0000, 200),
                (0xF0_F2F4, 255),
            ],
        },
        Look::Paper => Palette {
            water: 0x96_B0C4,
            land: 0xE2_DDD2,
            lo: 200,
            span: 87,
            lines: [
                (0x76_96B0, 255),
                (0x6E_6056, 150),
                (0x78_685C, 200),
                (0x5C_788E, 255),
                (0xFF_FFFF, 220),
                (0x22_2226, 255),
            ],
        },
        Look::Ink => Palette {
            water: 0x3A_3E44,
            land: 0xEC_EAE4,
            lo: 215,
            span: 56,
            lines: [
                (0x3A_3E44, 255),
                (0x1E_1E20, 120),
                (0x1E_1E20, 170),
                (0x14_1416, 255),
                (0xFF_FFFF, 230),
                (0x0E_0E10, 255),
            ],
        },
    }
}

/// `b` over `a` at `t` (of 255), each channel.
fn mix(a: u32, b: u32, t: u32) -> u32 {
    let ch = |s: u32| {
        let (x, y) = ((a >> s) & 0xFF, (b >> s) & 0xFF);
        ((x * (255 - t) + y * t + 127) / 255) << s
    };
    ch(16) | ch(8) | ch(0)
}

/// Row `y` of the map in `look` from `layers` (as [`unpack`] leaves them)
/// into `row` (opaque ARGB).
pub fn colour_row(layers: &[u8], y: usize, look: Look, row: &mut [u32]) {
    let p = palette(look);
    let at = |l: Layer, x: usize| {
        u32::from(
            layers
                .get(l as usize * W * H + y * W + x)
                .copied()
                .unwrap_or(0),
        )
    };
    for (x, out) in row.iter_mut().enumerate().take(W) {
        let lit = p.lo + p.span * at(Layer::Shade, x) / 255;
        let ch = |s: u32| (((p.land >> s) & 0xFF) * lit / 256).min(255) << s;
        let land = ch(16) | ch(8) | ch(0);
        let mut c = mix(p.water, land, at(Layer::Land, x));
        for (k, &(colour, alpha)) in p.lines.iter().enumerate() {
            let cover = at(LINES[k], x);
            if cover != 0 {
                c = mix(c, colour, alpha * cover / 255);
            }
        }
        *out = 0xFF00_0000 | c;
    }
}

/// The line layers in the order they are laid down.
const LINES: [Layer; 6] = [
    Layer::River,
    Layer::Border,
    Layer::Road,
    Layer::Coast,
    Layer::Halo,
    Layer::Text,
];

/// The rain's opacity per look (R184 step 2, M6: "a little stronger on
/// the light map"): a factor on RainViewer's own alpha, in 256ths. Night
/// keeps today's rain (256); Paper's and Ink's are fitted by
/// `tools/maps/rain_opacity.py` so the rain's mean colour difference from
/// the land (CIE DeltaE 2000) reaches the same share of what the map allows
/// as Night's (0.625); the test below fits them again.
#[must_use]
pub const fn rain_k(look: Look) -> u32 {
    match look {
        Look::Night => 256,
        Look::Paper => 345,
        Look::Ink => 369,
    }
}

/// A rain pixel `c` (ARGB, straight alpha: RainViewer's) over the map's
/// pixel `m` in `look`, opaque.
#[must_use]
pub fn rain_over(m: u32, c: u32, look: Look) -> u32 {
    let a = (((c >> 24) * rain_k(look) + 128) >> 8).min(255);
    0xFF00_0000 | if a == 0 { m } else { mix(m, c, a) }
}

/// The whole view in `look` into `out` ([`W`] x [`H`]).
pub fn colour(layers: &[u8], look: Look, out: &mut [u32]) {
    for (y, row) in out.as_chunks_mut::<W>().0.iter_mut().enumerate().take(H) {
        colour_row(layers, y, look, row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file as the PC's drawer writes it (each row as differences from
    /// its left, then zlib).
    fn file(layers: &[u8]) -> Vec<u8> {
        let mut diff = Vec::with_capacity(layers.len());
        for row in layers.chunks(W) {
            let mut left = 0u8;
            for &b in row {
                diff.push(b.wrapping_sub(left));
                left = b;
            }
        }
        let mut f = MAGIC.to_vec();
        f.push(VERSION);
        f.extend((W as u16).to_le_bytes());
        f.extend((H as u16).to_le_bytes());
        f.push(LAYERS as u8);
        f.extend(0..LAYERS as u8);
        f.extend(rp_inflate::pack::zlib(&diff));
        f
    }

    /// A map: land on the right half, a road across, a label's text.
    fn layers() -> Vec<u8> {
        let mut l = vec![0u8; BYTES];
        let plane = |k: Layer| k as usize * W * H;
        for y in 0..H {
            for x in 0..W {
                let i = y * W + x;
                l[plane(Layer::Land) + i] = if x >= W / 2 { 255 } else { 0 };
                l[plane(Layer::Shade) + i] = (x * 255 / W) as u8;
            }
            l[plane(Layer::Road) + y * W + 300] = 255;
        }
        l[plane(Layer::Text) + 100 * W + 350] = 255;
        l
    }

    #[test]
    fn layers_unpack_as_they_were_drawn() {
        let l = layers();
        let mut out = vec![0u8; BYTES];
        assert_eq!(unpack(&file(&l), &mut out), Ok(()));
        assert!(out == l);
    }

    /// Hostile input (docs/security.md): every header field and the stream
    /// checked, never a fault.
    #[test]
    fn a_damaged_or_foreign_file_is_refused() {
        let good = file(&layers());
        let mut out = vec![0u8; BYTES];
        for (at, v) in [(0, b'X'), (4, 9), (5, 0), (7, 0), (9, 7), (12, 5)] {
            let mut f = good.clone();
            f[at] = v;
            assert_eq!(unpack(&f, &mut out), Err(Error::Header), "byte {at}");
        }
        let mut f = good.clone();
        let n = f.len();
        f[n - 6] ^= 0x55;
        assert!(unpack(&f, &mut out).is_err(), "a damaged stream");
        assert_eq!(unpack(&good[..HEAD - 1], &mut out), Err(Error::Header));
        assert_eq!(unpack(&good[..HEAD + 20], &mut out), Err(Error::Stream));
        assert_eq!(unpack(&good, &mut out[1..]), Err(Error::Room));
    }

    /// CIE DeltaE 2000 between two sRGB colours (the fit's measure, as
    /// `tools/maps/rain_opacity.py` computes it).
    fn de2000(p: [u32; 3], q: [u32; 3]) -> f64 {
        fn lab(c: [u32; 3]) -> [f64; 3] {
            let lin = |v: u32| {
                let v = f64::from(v) / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            let (r, g, b) = (lin(c[0]), lin(c[1]), lin(c[2]));
            let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
            let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
            let f = |t: f64| {
                if t > 216.0 / 24389.0 {
                    t.cbrt()
                } else {
                    (24389.0 / 27.0 * t + 16.0) / 116.0
                }
            };
            let (fx, fy, fz) = (f(x), f(y), f(z));
            [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
        }
        let ([l1, a1, b1], [l2, a2, b2]) = (lab(p), lab(q));
        let cm = (a1.hypot(b1) + a2.hypot(b2)) / 2.0;
        let g = 0.5 * (1.0 - (cm.powi(7) / (cm.powi(7) + 25f64.powi(7))).sqrt());
        let (a1, a2) = (a1 * (1.0 + g), a2 * (1.0 + g));
        let (c1, c2) = (a1.hypot(b1), a2.hypot(b2));
        let h1 = b1.atan2(a1).to_degrees().rem_euclid(360.0);
        let h2 = b2.atan2(a2).to_degrees().rem_euclid(360.0);
        let (dl, dc) = (l2 - l1, c2 - c1);
        let mut dh = 0.0;
        if c1 * c2 != 0.0 {
            dh = h2 - h1;
            if dh > 180.0 {
                dh -= 360.0;
            } else if dh < -180.0 {
                dh += 360.0;
            }
        }
        let dhh = 2.0 * (c1 * c2).sqrt() * (dh / 2.0).to_radians().sin();
        let (lm, cm) = ((l1 + l2) / 2.0, (c1 + c2) / 2.0);
        let mut hm = h1 + h2;
        if c1 * c2 != 0.0 {
            if (h1 - h2).abs() > 180.0 {
                hm += if h1 + h2 < 360.0 { 360.0 } else { -360.0 };
            }
            hm /= 2.0;
        }
        let cos = |d: f64| d.to_radians().cos();
        let t = 1.0 - 0.17 * cos(hm - 30.0) + 0.24 * cos(2.0 * hm) + 0.32 * cos(3.0 * hm + 6.0)
            - 0.20 * cos(4.0 * hm - 63.0);
        let sl = 1.0 + 0.015 * (lm - 50.0).powi(2) / (20.0 + (lm - 50.0).powi(2)).sqrt();
        let sc = 1.0 + 0.045 * cm;
        let sh = 1.0 + 0.015 * cm * t;
        let rt = -2.0
            * (cm.powi(7) / (cm.powi(7) + 25f64.powi(7))).sqrt()
            * (60.0 * (-((hm - 275.0) / 25.0).powi(2)).exp())
                .to_radians()
                .sin();
        ((dl / sl).powi(2) + (dc / sc).powi(2) + (dhh / sh).powi(2) + rt * (dc / sc) * (dhh / sh))
            .sqrt()
    }

    /// The rain's factors are the fit's (R184 step 2): each light look's
    /// share of its most contrast (every colour opaque) matches Night's at
    /// 256 to within a step of the factor, over every colour of the test
    /// tile (RainViewer's scheme 2).
    #[test]
    fn the_rain_factors_are_fitted() {
        let png = include_bytes!("../tests/data/rainviewer-7-29-52.png");
        let mut tile = vec![0u32; 256 * 256];
        let mut data = vec![0u8; 256 * 1024];
        let mut raw = vec![0u8; 256 * (1 + 256 * 4)];
        assert_eq!(
            crate::png::decode(png, &mut tile, &mut data, &mut raw),
            Ok((256, 256))
        );
        let mut colours: Vec<u32> = tile.iter().copied().filter(|c| c >> 24 != 0).collect();
        colours.sort_unstable();
        colours.dedup();
        assert!(colours.len() > 30, "the tile's colours");
        let rgb = |c: u32| [(c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF];
        let land = |look: Look| {
            let p = palette(look);
            let lit = p.lo + p.span * 128 / 255;
            let ch = |s: u32| (((p.land >> s) & 0xFF) * lit / 256).min(255);
            [ch(16), ch(8), ch(0)]
        };
        let contrast = |look: Look, k: u32| {
            let m = land(look);
            let mm = m[0] << 16 | m[1] << 8 | m[2];
            colours
                .iter()
                .map(|&c| {
                    let a = (((c >> 24) * k + 128) >> 8).min(255);
                    de2000(m, rgb(mix(mm, c, a)))
                })
                .sum::<f64>()
                / colours.len() as f64
        };
        let share = |look: Look, k: u32| contrast(look, k) / contrast(look, 1 << 16);
        let target = share(Look::Night, rain_k(Look::Night));
        for look in [Look::Paper, Look::Ink] {
            let k = rain_k(look);
            assert!(
                share(look, k - 1) < target && target <= share(look, k + 1),
                "{look:?}"
            );
            assert!(k > rain_k(Look::Night), "{look:?}: stronger than on Night");
        }
        // Night's rain is today's: RainViewer's alpha as it came.
        let c = 0x8C3C_BE50;
        assert_eq!(
            rain_over(0x22_282E, c, Look::Night),
            0xFF00_0000 | mix(0x22_282E, c, 0x8C)
        );
        assert_eq!(rain_over(0x22_282E, 0, Look::Paper), 0xFF22_282E);
    }

    /// Every look from the same layers, each keeping what the others do:
    /// water and land apart, the terrain's light, the road and the text
    /// (Ink made from Night lost all four: the owner, 2026-11-04).
    #[test]
    fn every_look_keeps_water_land_terrain_and_lines() {
        let l = layers();
        let lum = |c: u32| ((c >> 16) & 0xFF) * 77 + ((c >> 8) & 0xFF) * 150 + (c & 0xFF) * 29;
        for look in [Look::Night, Look::Paper, Look::Ink] {
            let mut row = [0u32; W];
            colour_row(&l, 100, look, &mut row);
            let (water, lit, dim) = (row[10], row[W - 10], row[W / 2 + 10]);
            assert_ne!(water, lit, "{look:?}: water and land");
            assert!(lum(lit) > lum(dim), "{look:?}: the terrain's light");
            assert_ne!(row[300], row[310], "{look:?}: the road");
            assert_ne!(row[350], row[351], "{look:?}: the text");
            assert!(row.iter().all(|c| c >> 24 == 0xFF), "opaque");
        }
        // The light looks are light, Night dark.
        let mut n = [0u32; W];
        let mut p = [0u32; W];
        colour_row(&l, 100, Look::Night, &mut n);
        colour_row(&l, 100, Look::Paper, &mut p);
        assert!(lum(p[W - 10]) > 3 * lum(n[W - 10]));
    }
}
