//! Against Python's `zlib` (the packer): fixtures made by it, decoded
//! byte for byte; damage of every kind refused without a panic or a write
//! past the output.

use super::*;

/// (name, packed, original): made by `zlib.compress` at the level named
/// (0: stored blocks; `fixed`: the fixed code, `Z_FIXED`; the rest dynamic).
const CASES: [(&str, &[u8], &[u8]); 6] = [
    (
        "text9",
        include_bytes!("data/text9.zz"),
        include_bytes!("data/text9.raw"),
    ),
    (
        "text1",
        include_bytes!("data/text1.zz"),
        include_bytes!("data/text1.raw"),
    ),
    (
        "rand0",
        include_bytes!("data/rand0.zz"),
        include_bytes!("data/rand0.raw"),
    ),
    (
        "rand6",
        include_bytes!("data/rand6.zz"),
        include_bytes!("data/rand6.raw"),
    ),
    (
        "runs9",
        include_bytes!("data/runs9.zz"),
        include_bytes!("data/runs9.raw"),
    ),
    (
        "fixed",
        include_bytes!("data/fixed.zz"),
        include_bytes!("data/fixed.raw"),
    ),
];

#[test]
fn every_block_type_decodes_as_zlib_made_it() {
    for (name, z, raw) in CASES {
        let mut out = vec![0u8; raw.len()];
        assert_eq!(zlib(z, &mut out), Ok(raw.len()), "{name}");
        assert_eq!(out, raw, "{name}");
    }
}

#[test]
fn too_small_an_output_is_full_never_a_write_past_it() {
    for (name, z, raw) in CASES {
        let mut out = vec![0u8; raw.len() - 1];
        assert_eq!(zlib(z, &mut out), Err(Error::Full), "{name}");
    }
}

#[test]
fn every_truncation_is_refused() {
    for (name, z, raw) in CASES {
        for n in 0..z.len() {
            let mut out = vec![0u8; raw.len()];
            assert!(zlib(&z[..n], &mut out).is_err(), "{name} cut at {n}");
        }
    }
}

/// Every single bit flipped in every fixture: an error or a result inside
/// the output, never a panic. Adler-32 is the format's only check and
/// isn't perfect (a flip in a dynamic block's code table can change many
/// bytes whose sums cancel): changed output may pass at most once in
/// 10,000 flips (measured: 1 of ~136,000).
#[test]
fn every_bit_flipped_is_refused_or_caught() {
    let (mut flips, mut passed_changed) = (0u32, 0u32);
    for (name, z, raw) in CASES {
        let mut bad = z.to_vec();
        for i in 0..z.len() {
            for bit in 0..8 {
                bad[i] ^= 1 << bit;
                flips += 1;
                let mut out = vec![0u8; raw.len() + 64];
                if let Ok(n) = zlib(&bad, &mut out) {
                    assert!(n <= out.len(), "{name} {i}:{bit}");
                    passed_changed += u32::from(&out[..n] != raw);
                }
                bad[i] ^= 1 << bit;
            }
        }
    }
    assert!(
        passed_changed * 10_000 <= flips,
        "{passed_changed} of {flips}"
    );
}

#[test]
fn headers_the_format_forbids_are_refused() {
    let (_, z, raw) = CASES[0];
    let mut out = vec![0u8; raw.len()];
    for head in [[0x78, 0x9D], [0x79, 0x9C], [0x88, 0x9C], [0x78, 0xBB]] {
        let mut bad = z.to_vec();
        bad[..2].copy_from_slice(&head);
        assert_eq!(zlib(&bad, &mut out), Err(Error::Header), "{head:02x?}");
    }
    assert_eq!(zlib(&[0x78], &mut out), Err(Error::Truncated));
}

#[test]
fn a_reference_before_the_start_is_refused() {
    // Fixed block: literal 'a', then length 3 (code 257) at distance 2
    // (code 1): one byte back is all there is.
    // Bits (first first): 1 (last) 10 (fixed), 'a' = 0x61 -> 8-bit code
    // 0x30 + 0x61 = 0x91, length 257 -> 7-bit code 1, distance code 1 (5
    // bits).
    let mut bits = Vec::new();
    let mut put = |v: u32, n: u32, msb_first: bool| {
        for k in 0..n {
            let s = if msb_first { n - 1 - k } else { k };
            bits.push((v >> s) & 1);
        }
    };
    put(1, 1, false);
    put(1, 2, false);
    put(0x91, 8, true);
    put(1, 7, true);
    put(1, 5, true);
    put(0, 7, true);
    let mut bytes = vec![0u8; bits.len().div_ceil(8)];
    for (i, b) in bits.iter().enumerate() {
        bytes[i / 8] |= (*b as u8) << (i % 8);
    }
    let mut out = [0u8; 16];
    assert_eq!(inflate(&bytes, &mut out), Err(Error::Distance));
}

#[test]
fn paeth_undone_restores_the_plane() {
    let (w, h) = (37, 23);
    let plane: Vec<u8> = (0..w * h)
        .map(|i| ((i * 7919 + (i / w) * 31) % 251) as u8)
        .collect();
    // Filtered as the packer does.
    let at = |x: usize, y: usize| plane[y * w + x];
    let mut f = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let a = if x > 0 { at(x - 1, y) } else { 0 };
            let b = if y > 0 { at(x, y - 1) } else { 0 };
            let c = if x > 0 && y > 0 { at(x - 1, y - 1) } else { 0 };
            f[y * w + x] = at(x, y).wrapping_sub(paeth(a, b, c));
        }
    }
    unpaeth(&mut f, w, h);
    assert_eq!(f, plane);
    // Too short: left alone.
    let mut short = vec![9u8; 10];
    unpaeth(&mut short, w, h);
    assert_eq!(short, vec![9u8; 10]);
}

#[test]
fn adler32_matches_rfc_1950s_example() {
    assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    assert_eq!(adler32(b""), 1);
}

/// Our packer's output, read back: every fixture, empty input, and a
/// photo-like plane through the Paeth filter.
#[test]
fn what_the_packer_makes_decodes_to_the_original() {
    let plane: Vec<u8> = (0..480 * 64)
        .map(|i: usize| ((i % 480) / 3 + (i / 480) * 2 + ((i * 2_654_435_761) >> 29) % 5) as u8)
        .collect();
    let filtered = pack::paeth_filter(&plane, 480, 64);
    let inputs: Vec<&[u8]> = CASES
        .iter()
        .map(|c| c.2)
        .chain([&[][..], &filtered[..], &[7u8; 70_000][..]])
        .collect();
    for (k, raw) in inputs.iter().enumerate() {
        let z = pack::zlib(raw);
        let mut out = vec![0u8; raw.len()];
        assert_eq!(zlib(&z, &mut out), Ok(raw.len()), "input {k}");
        assert_eq!(&out, raw, "input {k}");
    }
    let mut back = filtered.clone();
    unpaeth(&mut back, 480, 64);
    assert_eq!(back, plane);
    // Packs: the filtered plane well under half.
    assert!(pack::zlib(&filtered).len() * 2 < filtered.len());
}
