//! The JPEG reader against an independent decoder (Pillow's, made by
//! jpeg_fixtures/make.py): each file's square at 24 x 24 means within a
//! few levels of Pillow's, whole or fed in small pieces; and hostile files
//! (cut at every length, junk, lying lengths) that must never panic.

extern crate std;
use super::*;
use std::vec;
use std::vec::Vec;

macro_rules! fixture {
    ($n:literal) => {
        (
            &include_bytes!(concat!("jpeg_fixtures/", $n, ".jpg"))[..],
            &include_bytes!(concat!("jpeg_fixtures/", $n, ".rgb"))[..],
        )
    };
}

/// The file decoded, fed `piece` bytes at a time (resumed where each feed
/// said): the decoder and its square.
fn decode(file: &[u8], piece: usize) -> Result<(Decoder, Vec<u32>), Error> {
    let mut d = Decoder::new();
    let mut acc = vec![0u32; WORDS];
    let mut at = 0;
    loop {
        // The first piece holds every header (the device's first answer
        // is 256 KB).
        let len = if at == 0 {
            file.len().min(piece.max(64 * 1024))
        } else {
            piece
        };
        let end = (at + len).min(file.len());
        match d.feed(&file[at..end], end == file.len(), &mut acc)? {
            Progress::Done => return Ok((d, acc)),
            Progress::More(n) => {
                assert!(n > 0 || end > at, "no progress");
                at += n;
                assert!(at <= file.len());
            }
        }
    }
}

/// The mean difference from Pillow's square, per colour, in levels.
fn difference(d: &Decoder, acc: &[u32], want: &[u8]) -> f64 {
    let mut total = 0f64;
    for by in 0..24 {
        for bx in 0..24 {
            let mut sum = [0u32; 3];
            for y in 0..4 {
                for x in 0..4 {
                    let (r, g, b) = d.rgb(acc, bx * 4 + x, by * 4 + y);
                    sum[0] += u32::from(r);
                    sum[1] += u32::from(g);
                    sum[2] += u32::from(b);
                }
            }
            for c in 0..3 {
                let ours = f64::from(sum[c]) / 16.0;
                total += (ours - f64::from(want[(by * 24 + bx) * 3 + c])).abs();
            }
        }
    }
    total / (24.0 * 24.0 * 3.0)
}

#[test]
fn every_kind_matches_an_independent_decoder_whole_and_in_pieces() {
    for (name, (file, want), limit) in [
        (
            "4:2:0, decoded at 8 pixels a block",
            fixture!("base420"),
            6.0,
        ),
        ("4:4:4, at 2 pixels a block", fixture!("base444"), 6.0),
        ("grey, smaller than the square", fixture!("grey"), 6.0),
        ("progressive, its DC scan alone", fixture!("prog"), 9.0),
        ("restart markers", fixture!("restart"), 6.0),
        (
            "a camera's thumbnail, turned upright",
            fixture!("camera"),
            7.0,
        ),
    ] {
        for piece in [usize::MAX, 4096, 701] {
            let (d, acc) = decode(file, piece).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let diff = difference(&d, &acc, want);
            assert!(
                diff < limit,
                "{name}, pieces of {piece}: {diff:.2} levels off"
            );
        }
    }
}

#[test]
fn a_camera_file_is_read_from_its_thumbnail_not_its_blue_picture() {
    let (file, _) = fixture!("camera");
    let (d, acc) = decode(file, usize::MAX).unwrap();
    // The thumbnail's top-left red block, turned a quarter clockwise, is
    // at the top right.
    let (r, g, b) = d.rgb(&acc, 90, 5);
    assert!(r > 180 && g < 80 && b < 80, "{r} {g} {b}");
    assert_eq!(d.size(), (160, 120));
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u8 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (self.0 >> 33) as u8
    }
}

#[test]
fn hostile_files_never_panic() {
    let mut acc = vec![0u32; WORDS];
    assert_eq!(
        Decoder::new().feed(b"", true, &mut acc),
        Err(Error::NotJpeg)
    );
    assert_eq!(
        Decoder::new().feed(b"GIF89a", true, &mut acc),
        Err(Error::NotJpeg)
    );
    assert_eq!(
        Decoder::new().feed(&[0xFF, 0xD8], true, &mut acc),
        Err(Error::Headers)
    );
    assert_eq!(
        Decoder::new().feed(&[0xFF, 0xD8, 0xFF, 0xC0], true, &mut acc),
        Err(Error::Headers)
    );
    // The caller's square too small.
    assert_eq!(
        Decoder::new().feed(b"x", true, &mut [0u32; 10]),
        Err(Error::Damaged)
    );
    let files = [
        fixture!("base420").0,
        fixture!("grey").0,
        fixture!("restart").0,
        fixture!("prog").0,
        fixture!("camera").0,
    ];
    // Cut at every length (as a last piece: a truncated file).
    for f in files {
        let step = (f.len() / 700).max(1);
        for cut in (0..f.len()).step_by(step) {
            acc.fill(0);
            let _ = Decoder::new().feed(&f[..cut], true, &mut acc);
        }
    }
    // Junk spliced in, and lengths that lie.
    let mut r = Lcg(5);
    for _ in 0..40 {
        for f in files {
            let mut b = f.to_vec();
            for _ in 0..12 {
                let at = usize::from(r.next()) * b.len() / 256;
                b[at] = r.next();
            }
            acc.fill(0);
            let _ = Decoder::new().feed(&b, true, &mut acc);
        }
        let junk: Vec<u8> = [0xFF, 0xD8]
            .into_iter()
            .chain((0..600).map(|_| r.next()))
            .collect();
        let _ = Decoder::new().feed(&junk, true, &mut acc);
    }
    // A frame of 65535 x 65535 decodes what is there and stops.
    let mut big = fixture!("grey").0.to_vec();
    let sof = big.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    big[sof + 5..sof + 9].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    acc.fill(0);
    let _ = Decoder::new().feed(&big, true, &mut acc);
    // Pieces of one byte after the headers: never stuck.
    let (_, _) = decode(fixture!("restart").0, 1).unwrap();
}
