//! A PNG reader for map and radar tiles (the radar, built for real: the
//! owner, 2026-10-31: "radar needs to work"), no allocation: the caller
//! lends the buffers. It reads what tile servers send: 8-bit RGBA and RGB,
//! and palette pictures of 1, 2, 4 or 8 bits with their transparency,
//! not interlaced; anything else is refused. Every length is checked
//! before it is used: a tile comes over a cable (and later the internet)
//! and is untrusted.
//!
//! Iterate, don't imitate (rule 0): Windows Mobile decoded PNG through a
//! COM imaging library into a DIB; ours inflates the tile's data once with
//! the kernel's own inflater and writes straight into the ARGB words the
//! 2D engine draws.

use rp_inflate::paeth;

/// Why a tile wasn't read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PngError {
    /// Not a PNG, or cut short.
    Damaged,
    /// A kind this reader doesn't take (16-bit, grey, interlaced).
    Unsupported,
    /// The lent buffers are too small.
    TooBig,
}

/// The size a tile's header names, without decoding it.
#[must_use]
pub fn size(png: &[u8]) -> Option<(u32, u32)> {
    let h = png.get(..24)?;
    (h[..8] == SIGNATURE && &h[12..16] == b"IHDR").then(|| {
        (
            u32::from_be_bytes([h[16], h[17], h[18], h[19]]),
            u32::from_be_bytes([h[20], h[21], h[22], h[23]]),
        )
    })
}

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

/// `png` decoded into `out` as ARGB words (`0xAARRGGBB`), `w` by `h`, row
/// after row; `data` and `raw` are scratch (the image data gathered, then
/// inflated: at least `h * (1 + w * 4)` bytes). Its width and height.
///
/// # Errors
/// [`PngError`].
pub fn decode(
    png: &[u8],
    out: &mut [u32],
    data: &mut [u8],
    raw: &mut [u8],
) -> Result<(u32, u32), PngError> {
    let (w, h) = size(png).ok_or(PngError::Damaged)?;
    let hd = png.get(24..29).ok_or(PngError::Damaged)?;
    // Bit depth, colour type, compression, filter method, interlace.
    let (depth, color, interlace) = (hd[0], hd[1], hd[4]);
    if w == 0 || h == 0 || w > 4096 || h > 4096 || interlace != 0 || hd[2] != 0 || hd[3] != 0 {
        return Err(PngError::Unsupported);
    }
    // Channels, and bits a pixel.
    let chans: u32 = match (color, depth) {
        (6, 8) => 4,
        (2, 8) => 3,
        (3, 1 | 2 | 4 | 8) => 1,
        _ => return Err(PngError::Unsupported),
    };
    let bpp_bits = chans * u32::from(depth);
    let row_bytes = (w * bpp_bits).div_ceil(8) as usize;
    let filt_bpp = (bpp_bits as usize).div_ceil(8).max(1);
    let (w, h) = (w as usize, h as usize);
    let need = h * (1 + row_bytes);
    if out.len() < w * h || raw.len() < need {
        return Err(PngError::TooBig);
    }
    // The chunks: the palette, its transparency, the data gathered.
    let mut palette = [0xFF00_0000u32; 256];
    let mut at = 8;
    let mut got = 0usize;
    loop {
        let len = png.get(at..at + 4).ok_or(PngError::Damaged)?;
        let len = u32::from_be_bytes([len[0], len[1], len[2], len[3]]) as usize;
        let kind = png.get(at + 4..at + 8).ok_or(PngError::Damaged)?;
        let body = png.get(at + 8..at + 8 + len).ok_or(PngError::Damaged)?;
        match kind {
            b"PLTE" => {
                for (k, c) in body.chunks_exact(3).take(256).enumerate() {
                    palette[k] = 0xFF00_0000
                        | u32::from(c[0]) << 16
                        | u32::from(c[1]) << 8
                        | u32::from(c[2]);
                }
            }
            b"tRNS" if color == 3 => {
                for (k, &a) in body.iter().take(256).enumerate() {
                    palette[k] = (palette[k] & 0x00FF_FFFF) | u32::from(a) << 24;
                }
            }
            b"IDAT" => {
                let d = data.get_mut(got..got + len).ok_or(PngError::TooBig)?;
                d.copy_from_slice(body);
                got += len;
            }
            b"IEND" => break,
            _ => {}
        }
        at += 12 + len;
    }
    if rp_inflate::zlib(&data[..got], &mut raw[..need]).map_err(|_| PngError::Damaged)? != need {
        return Err(PngError::Damaged);
    }
    // The filters undone, row by row (the previous row already plain).
    for y in 0..h {
        let line = y * (1 + row_bytes);
        let filter = raw[line];
        for i in 0..row_bytes {
            let p = line + 1 + i;
            let a = if i >= filt_bpp { raw[p - filt_bpp] } else { 0 };
            let b = if y > 0 { raw[p - 1 - row_bytes] } else { 0 };
            let c = if y > 0 && i >= filt_bpp {
                raw[p - 1 - row_bytes - filt_bpp]
            } else {
                0
            };
            raw[p] = raw[p].wrapping_add(match filter {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                4 => paeth(a, b, c),
                _ => return Err(PngError::Damaged),
            });
        }
    }
    // Out as ARGB words.
    for y in 0..h {
        let src = &raw[y * (1 + row_bytes) + 1..(y + 1) * (1 + row_bytes)];
        let dst = &mut out[y * w..(y + 1) * w];
        for (x, o) in dst.iter_mut().enumerate() {
            *o = match chans {
                4 => {
                    let p = &src[x * 4..x * 4 + 4];
                    u32::from(p[3]) << 24
                        | u32::from(p[0]) << 16
                        | u32::from(p[1]) << 8
                        | u32::from(p[2])
                }
                3 => {
                    let p = &src[x * 3..x * 3 + 3];
                    0xFF00_0000 | u32::from(p[0]) << 16 | u32::from(p[1]) << 8 | u32::from(p[2])
                }
                _ => {
                    let d = usize::from(depth);
                    let bit = x * d;
                    let byte = src[bit / 8];
                    let shift = 8 - d - bit % 8;
                    let k = (byte >> shift) & ((1u16 << d) - 1) as u8;
                    palette[usize::from(k)]
                }
            };
        }
    }
    Ok((w as u32, h as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &[u8] = include_bytes!("../tests/data/carto-dark-7-29-52.png");
    const RAIN: &[u8] = include_bytes!("../tests/data/rainviewer-7-29-52.png");

    fn crc(words: &[u32]) -> u32 {
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        crate::store::crc32(&bytes)
    }

    #[test]
    fn real_tiles_decode_to_what_a_reference_decoder_gives() {
        let mut out = vec![0u32; 256 * 256];
        let mut data = vec![0u8; 64 * 1024];
        let mut raw = vec![0u8; 256 * (1 + 256 * 4)];
        // The map: a 4-bit palette picture.
        assert_eq!(decode(BASE, &mut out, &mut data, &mut raw), Ok((256, 256)));
        assert_eq!(out[57 * 256 + 170], 0xFF22_2324);
        assert_eq!(crc(&out), 0xce55_4933);
        // The radar: 8-bit RGBA, mostly clear.
        assert_eq!(decode(RAIN, &mut out, &mut data, &mut raw), Ok((256, 256)));
        assert_eq!(out[6 * 256 + 255], 0x1463_6159);
        assert_eq!(crc(&out), 0x9b5c_4b6a);
    }

    #[test]
    fn damaged_or_unsupported_tiles_are_refused_never_trusted() {
        let mut out = vec![0u32; 256 * 256];
        let mut data = vec![0u8; 64 * 1024];
        let mut raw = vec![0u8; 256 * (1 + 256 * 4)];
        for n in [0, 8, 24, 33, 100, BASE.len() / 2, BASE.len() - 12] {
            assert!(
                decode(&BASE[..n], &mut out, &mut data, &mut raw).is_err(),
                "cut at {n}"
            );
        }
        let mut bad = BASE.to_vec();
        bad[24] = 16; // 16-bit
        assert_eq!(
            decode(&bad, &mut out, &mut data, &mut raw),
            Err(PngError::Unsupported)
        );
        let mut small = vec![0u32; 100];
        assert_eq!(
            decode(BASE, &mut small, &mut data, &mut raw),
            Err(PngError::TooBig)
        );
        assert_eq!(
            decode(
                b"not a png at all, not at all",
                &mut out,
                &mut data,
                &mut raw
            ),
            Err(PngError::Damaged)
        );
    }
}
