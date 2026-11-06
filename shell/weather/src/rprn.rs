//! A rain frame as the dock's relay sends it (R184 step 2, run today-0055:
//! nine PNG tiles a frame took 117 asks and 549 ms of decoding a frame on
//! the X51v). The relay fetches RainViewer's nine tiles, stitches and crops
//! them to the view, and sends the rain as a palette and each row's runs:
//! one answer a frame, ~11 KB (two live frames measured 10,800 and 10,908
//! bytes against 26 and 24 KB of PNGs), expanded at memory speed.
//!
//! Layout (little-endian): `RPRN`, version 1, width and height (u16, the
//! view's), the palette's size `n` (1-255) and its `n` colours (ARGB u32,
//! straight alpha), then each row in order as runs `(length 1-255, index
//! 0..=n)`, index 0 no rain, `k` the palette's colour `k - 1`; a run never
//! crosses a row's end, and nothing follows the last row. Every field is
//! checked (docs/security.md: the relay is a PC we trust today, the parser
//! assumes it isn't).

use crate::tiles::{VIEW_H, VIEW_W};

/// The file's first four bytes.
pub const MAGIC: &[u8; 4] = b"RPRN";
/// The version read.
pub const VERSION: u8 = 1;

/// Why a file was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// Not an RPRN file of this version and the view's size, or its
    /// palette is missing.
    Header,
    /// A run is empty, names no colour, crosses its row, or the rows end
    /// early or late.
    Runs,
    /// `out` is smaller than the view.
    Room,
}

/// `file` expanded into `out` (the view, ARGB, row by row): no rain is 0.
///
/// # Errors
/// The first fault found ([`Error`]); `out` is then partly written.
pub fn decode(file: &[u8], out: &mut [u32]) -> Result<(), Error> {
    let out = out.get_mut(..VIEW_W * VIEW_H).ok_or(Error::Room)?;
    let head = file.get(..10).ok_or(Error::Header)?;
    let u16_at = |i: usize| usize::from(u16::from_le_bytes([head[i], head[i + 1]]));
    let n = usize::from(head[9]);
    if &head[..4] != MAGIC
        || head[4] != VERSION
        || u16_at(5) != VIEW_W
        || u16_at(7) != VIEW_H
        || n == 0
    {
        return Err(Error::Header);
    }
    let pal = file.get(10..10 + 4 * n).ok_or(Error::Header)?;
    let colour = |k: usize| {
        let c = &pal[4 * k..4 * k + 4];
        u32::from_le_bytes([c[0], c[1], c[2], c[3]])
    };
    let mut runs = file[10 + 4 * n..].chunks(2);
    for row in out.chunks_mut(VIEW_W) {
        let mut x = 0;
        while x < VIEW_W {
            let [len, idx] = runs.next().ok_or(Error::Runs)? else {
                return Err(Error::Runs);
            };
            let (len, idx) = (usize::from(*len), usize::from(*idx));
            if len == 0 || idx > n || x + len > VIEW_W {
                return Err(Error::Runs);
            }
            let c = if idx == 0 { 0 } else { colour(idx - 1) };
            row[x..x + len].fill(c);
            x += len;
        }
    }
    if runs.next().is_some() {
        return Err(Error::Runs);
    }
    Ok(())
}

/// The view's rain (`px`, ARGB, row by row) as an RPRN file: the relay's
/// encoding, kept here as the reference its output is tested against.
/// `None` if it holds more than 255 colours.
#[cfg(test)]
#[must_use]
pub fn encode(px: &[u32]) -> Option<std::vec::Vec<u8>> {
    let mut pal: std::vec::Vec<u32> = std::vec::Vec::new();
    let mut runs = std::vec::Vec::new();
    for row in px.chunks(VIEW_W).take(VIEW_H) {
        let mut x = 0;
        while x < VIEW_W {
            let c = if row[x] >> 24 == 0 { 0 } else { row[x] };
            let mut len = 1;
            while x + len < VIEW_W && len < 255 {
                let d = if row[x + len] >> 24 == 0 {
                    0
                } else {
                    row[x + len]
                };
                if d != c {
                    break;
                }
                len += 1;
            }
            let idx = if c == 0 {
                0
            } else if let Some(k) = pal.iter().position(|&p| p == c) {
                k + 1
            } else {
                pal.push(c);
                pal.len()
            };
            if idx > 255 {
                return None;
            }
            runs.extend([len as u8, idx as u8]);
            x += len;
        }
    }
    let mut f = MAGIC.to_vec();
    f.push(VERSION);
    f.extend((VIEW_W as u16).to_le_bytes());
    f.extend((VIEW_H as u16).to_le_bytes());
    f.push(pal.len().max(1) as u8);
    if pal.is_empty() {
        pal.push(0);
    }
    for c in &pal {
        f.extend(c.to_le_bytes());
    }
    f.extend(runs);
    Some(f)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec;

    /// A frame of the test tile placed in every tile of a view (RainViewer's
    /// own colours and alpha).
    fn rain() -> std::vec::Vec<u32> {
        let png = include_bytes!("../tests/data/rainviewer-7-29-52.png");
        let mut tile = vec![0u32; 256 * 256];
        let mut data = vec![0u8; 256 * 1024];
        let mut raw = vec![0u8; 256 * (1 + 256 * 4)];
        assert_eq!(
            crate::png::decode(png, &mut tile, &mut data, &mut raw),
            Ok((256, 256))
        );
        let v = crate::tiles::View::centred(30_664, -97_922);
        let mut view = vec![0u32; VIEW_W * VIEW_H];
        for k in 0..9 {
            v.place(k, &tile, &mut view);
        }
        view
    }

    #[test]
    fn a_frame_comes_back_as_it_was_sent() {
        let view = rain();
        let f = encode(&view).expect("under 256 colours");
        assert!(f.len() < 40_000, "{} bytes", f.len());
        let mut out = vec![1u32; VIEW_W * VIEW_H];
        assert_eq!(decode(&f, &mut out), Ok(()));
        let clear = |c: u32| if c >> 24 == 0 { 0 } else { c };
        assert!(out.iter().zip(&view).all(|(a, b)| *a == clear(*b)));
        // A dry frame: one palette entry, runs of no rain.
        let dry = encode(&vec![0u32; VIEW_W * VIEW_H]).unwrap();
        assert_eq!(decode(&dry, &mut out), Ok(()));
        assert!(out.iter().all(|&c| c == 0));
    }

    /// Hostile input (docs/security.md): every header field, every run
    /// fault and a short or long file refused, never a fault.
    #[test]
    fn a_damaged_or_foreign_frame_is_refused() {
        let good = encode(&rain()).unwrap();
        let mut out = vec![0u32; VIEW_W * VIEW_H];
        for (at, v) in [(0, b'X'), (4, 2), (5, 0), (7, 0), (9, 0)] {
            let mut f = good.clone();
            f[at] = v;
            assert_eq!(decode(&f, &mut out), Err(Error::Header), "byte {at}");
        }
        let n = usize::from(good[9]);
        let runs = 10 + 4 * n;
        for (what, v) in [
            ("an empty run", (0u8, 0u8)),
            ("no such colour", (1, n as u8 + 1)),
            ("past the row", (255, 0)),
        ] {
            let mut f = good.clone();
            // The first row's first run replaced; for "past the row" the
            // row's runs then add up past its end.
            f[runs] = v.0;
            f[runs + 1] = v.1;
            if what == "past the row" {
                f[runs + 2] = 255;
                f[runs + 4] = 255;
            }
            assert_eq!(decode(&f, &mut out), Err(Error::Runs), "{what}");
        }
        assert_eq!(
            decode(&good[..good.len() - 2], &mut out),
            Err(Error::Runs),
            "short"
        );
        let mut long = good.clone();
        long.extend([1, 0]);
        assert_eq!(decode(&long, &mut out), Err(Error::Runs), "long");
        assert_eq!(
            decode(&good[..12], &mut out),
            Err(Error::Header),
            "no palette"
        );
        assert_eq!(decode(&good, &mut out[1..]), Err(Error::Room));
    }
}
