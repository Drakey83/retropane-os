use super::*;

fn round_trip(data: &[u8]) -> usize {
    let mut c = vec![0u8; max_compressed(data.len())];
    let n = compress(data, &mut c).unwrap();
    let mut d = vec![0u8; data.len()];
    assert_eq!(decompress(&c[..n], &mut d), Ok(data.len()));
    assert_eq!(d, data);
    check_end_rules(&c[..n], data.len());
    n
}

/// The spec's end rules, checked on our own output: the block ends with a
/// literals-only sequence; no match starts within the last 12 bytes or ends
/// within the last 5.
fn check_end_rules(block: &[u8], total: usize) {
    let (mut i, mut o) = (0, 0);
    loop {
        let t = block[i];
        i += 1;
        let lits = length(block, &mut i, usize::from(t >> 4)).unwrap();
        i += lits;
        o += lits;
        if i == block.len() {
            break;
        }
        i += 2;
        let ml = length(block, &mut i, usize::from(t & 0xF)).unwrap() + 4;
        if total >= MF_LIMIT {
            assert!(o + MF_LIMIT <= total, "match starts in the last 12 bytes");
            assert!(
                o + ml + LAST_LITERALS <= total,
                "match ends in the last 5 bytes"
            );
        }
        o += ml;
    }
    assert_eq!(o, total);
}

#[test]
fn decodes_hand_built_blocks_from_the_spec() {
    // Literals only: token 0x50, "hello".
    let mut out = [0u8; 16];
    assert_eq!(decompress(b"\x50hello", &mut out), Ok(5));
    assert_eq!(&out[..5], b"hello");
    // "abcabcabcabc...": 3 literals, then a match at offset 3, length 4+5=9
    // (overlapping: the pattern repeats), then 5 literals.
    let block = b"\x35abc\x03\x00\x50xyzzy";
    let mut out = [0u8; 32];
    assert_eq!(decompress(block, &mut out), Ok(17));
    assert_eq!(&out[..17], b"abcabcabcabcxyzzy");
    // A run: 1 literal, match offset 1 length 15+3+4=22, then 5 literals.
    let block = b"\x1fA\x01\x00\x03\x50ZZZZZ";
    let mut out = [0u8; 32];
    assert_eq!(decompress(block, &mut out), Ok(28));
    assert_eq!(&out[..23], &[b'A'; 23]);
    // Extended literal length: 15 + 255 + 10 = 280 literals.
    let mut block = vec![0xF0, 255, 10];
    block.extend(std::iter::repeat_n(b'q', 280));
    let mut out = vec![0u8; 280];
    assert_eq!(decompress(&block, &mut out), Ok(280));
    assert!(out.iter().all(|&b| b == b'q'));
}

#[test]
fn rejects_hostile_blocks() {
    let mut out = [0u8; 16];
    assert_eq!(decompress(b"", &mut out), Err(Error::Truncated));
    assert_eq!(decompress(b"\x50hel", &mut out), Err(Error::Truncated));
    // Offset 0, and an offset before the start of the output.
    assert_eq!(
        decompress(b"\x11a\x00\x00", &mut out),
        Err(Error::BadOffset)
    );
    assert_eq!(
        decompress(b"\x11a\x05\x00", &mut out),
        Err(Error::BadOffset)
    );
    // Output larger than dst.
    assert_eq!(
        decompress(b"\x1fA\x01\x00\xff\x00", &mut out),
        Err(Error::Overflow)
    );
    assert_eq!(
        decompress(b"\x50hello", &mut out[..4]),
        Err(Error::Overflow)
    );
    // An endless length run.
    assert_eq!(
        decompress(&[0xF0, 255, 255, 255], &mut out),
        Err(Error::Truncated)
    );
}

#[test]
fn hostile_input_never_panics() {
    // Mutate valid blocks and feed random bytes: errors are fine, panics or
    // out-of-bounds writes are not (the slice API makes the latter a panic).
    let mut rng = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let data: Vec<u8> = (0..4096).map(|i| (i % 7 * 31 + i / 100) as u8).collect();
    let mut c = vec![0u8; max_compressed(data.len())];
    let n = compress(&data, &mut c).unwrap();
    let mut out = vec![0u8; 4096];
    for _ in 0..20_000 {
        let mut b = c[..n].to_vec();
        for _ in 0..1 + next() % 4 {
            let at = next() as usize % b.len();
            b[at] = next() as u8;
        }
        let _ = decompress(&b, &mut out);
        let random: Vec<u8> = (0..next() % 64).map(|_| next() as u8).collect();
        let _ = decompress(&random, &mut out);
    }
}

#[test]
fn round_trips_typical_page_contents() {
    let zeros = vec![0u8; 4096];
    let n = round_trip(&zeros);
    assert!(
        n < 40,
        "an all-zero page compresses to almost nothing ({n})"
    );
    let text: Vec<u8> = b"The quick brown fox jumps over the lazy dog. "
        .iter()
        .cycle()
        .take(4096)
        .copied()
        .collect();
    assert!(round_trip(&text) < 200);
    // A page of small integers and pointers, like a heap page.
    let heap: Vec<u8> = (0..1024u32)
        .flat_map(|i| (0xA800_0000 + (i % 37) * 16).to_le_bytes())
        .collect();
    assert!(round_trip(&heap) < 2048);
    // Incompressible data grows, but within the bound.
    let mut x = 1u32;
    let noise: Vec<u8> = (0..4096)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect();
    assert!(round_trip(&noise) <= max_compressed(4096));
}

#[test]
fn round_trips_edge_sizes_and_long_matches() {
    for n in [0, 1, 4, 5, 11, 12, 13, 16, 255, 256, 270, 4095, 4096, 65536] {
        let data: Vec<u8> = (0..n).map(|i| (i / 3) as u8).collect();
        round_trip(&data);
    }
    // A match longer than 15 + 255 + 255 (several extra length bytes).
    let mut long = vec![7u8; 3000];
    long.extend(b"tail!");
    round_trip(&long);
    // Inputs over 64 KiB are refused, not mangled.
    let mut c = vec![0u8; max_compressed(70_000)];
    assert_eq!(compress(&vec![0u8; 70_000], &mut c), Err(Error::Overflow));
}

#[test]
fn compress_reports_a_small_destination() {
    let data = vec![0x55u8; 100];
    let mut tiny = [0u8; 3];
    assert_eq!(compress(&data, &mut tiny), Err(Error::Overflow));
}

#[test]
fn overlapping_copy_patterns() {
    // Every offset 1..=8 against lengths that cross multiple pattern repeats.
    for offset in 1..=8usize {
        for len in 4..=40usize {
            let mut buf = vec![0u8; offset + len];
            for (i, b) in buf.iter_mut().enumerate().take(offset) {
                *b = i as u8 + 1;
            }
            copy_match(&mut buf, offset, offset, len);
            for (i, &b) in buf.iter().enumerate() {
                assert_eq!(
                    b,
                    (i % offset) as u8 + 1,
                    "offset {offset} len {len} at {i}"
                );
            }
        }
    }
}

#[test]
fn bench_page_shape() {
    // The on-target benchmark's page (boot/suite): print its compressed
    // shape so per-sequence costs can be estimated from emulator timings.
    let page: Vec<u8> = (0..1024u32)
        .flat_map(|i| {
            (if i % 3 == 0 {
                0xA800_0000 + (i % 37) * 16
            } else {
                i % 11
            })
            .to_le_bytes()
        })
        .collect();
    let mut c = vec![0u8; max_compressed(4096)];
    let n = compress(&page, &mut c).unwrap();
    let (mut i, mut seqs, mut lit_total, mut match_total) = (0, 0, 0, 0);
    loop {
        let t = c[i];
        i += 1;
        let lits = length(&c, &mut i, usize::from(t >> 4)).unwrap();
        i += lits;
        lit_total += lits;
        seqs += 1;
        if i == n {
            break;
        }
        i += 2;
        match_total += length(&c, &mut i, usize::from(t & 0xF)).unwrap() + 4;
    }
    eprintln!(
        "compressed {n} bytes, {seqs} sequences, {lit_total} literal bytes, {match_total} match bytes"
    );
}

/// The compression a hash multiplier gives on the files under
/// `RP_LZ4_CORPUS` (a folder), in 4 KiB pages as the kernel compresses
/// them: total bytes out for each multiplier. A measurement, not a check:
/// `cargo test -p rp-lz4 -- --ignored --nocapture multipliers_compress`.
#[test]
#[ignore = "a measurement over a local corpus"]
fn multipliers_compress() {
    let Some(dir) = std::env::var_os("RP_LZ4_CORPUS") else {
        return;
    };
    let mut pages: Vec<Vec<u8>> = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(dir)];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(b) = std::fs::read(&p) {
                pages.extend(b.chunks(4096).map(<[u8]>::to_vec));
            }
        }
    }
    fn total<const K: u32>(pages: &[Vec<u8>]) -> usize {
        let mut table = [0u16; TABLE_LEN];
        let mut out = vec![0u8; max_compressed(4096)];
        pages
            .iter()
            .map(|p| compress_k::<K>(&mut table, p, &mut out).unwrap())
            .sum()
    }
    let input: usize = pages.iter().map(Vec::len).sum();
    for (name, n) in [
        ("32-bit 0x9E3779B1", total::<0x9E37_79B1>(&pages)),
        ("27-bit 0x04F1BBCD", total::<0x04F1_BBCD>(&pages)),
        ("16-bit 0x00009E37", total::<0x0000_9E37>(&pages)),
        ("15-bit 0x00004F1B", total::<0x0000_4F1B>(&pages)),
    ] {
        println!(
            "{name}: {n} of {input} bytes ({:.2}%)",
            n as f64 * 100.0 / input as f64
        );
    }
}

#[test]
fn the_hash_multiplier_takes_the_short_multiply() {
    // The XScale takes a cycle longer for a multiplier above 27 bits and
    // LLVM doesn't hide it (docs/hw/cpu.md): compress was 7.8% slower with
    // LZ4's 32-bit constant. Odd, so no input bit is dropped.
    const {
        assert!(
            HASH_K < 1 << 27,
            "the hash multiplier takes the long multiply"
        )
    };
    const { assert!(HASH_K % 2 == 1) };
    assert_eq!(
        HASH_K,
        0x9E37_79B1 >> 5,
        "LZ4's constant, shifted into 27 bits"
    );
}
