use super::*;

#[test]
fn xxh32_matches_the_published_vectors() {
    // xxHash's reference results (xxhsum, XXH32, seed 0 unless named).
    assert_eq!(xxh32(b"", 0), 0x02CC_5D05);
    assert_eq!(xxh32(b"a", 0), 0x550D_7456);
    assert_eq!(xxh32(b"abc", 0), 0x32D1_53FF);
    assert_eq!(
        xxh32(b"Nobody inspects the spammish repetition", 0),
        0xE229_3B2F
    );
    assert_eq!(xxh32(b"", 1), 0x0B2C_B792);
    // Every length 0..=64 differs from its neighbours (the 16-byte stripes,
    // the 4-byte words and the byte tail all reached).
    let data: Vec<u8> = (0..64u8).map(|i| i.wrapping_mul(37)).collect();
    let all: Vec<u32> = (0..=64).map(|n| xxh32(&data[..n], 7)).collect();
    for w in all.windows(2) {
        assert_ne!(w[0], w[1]);
    }
}

fn sample(buf: &mut [u8], seq: u32) -> usize {
    let mut w = Writer::new(buf).unwrap();
    w.section(Kind::Process, &[1, 2, 3, 4, 5]).unwrap();
    w.section(Kind::Thread, &[0xAA; 68]).unwrap();
    w.section(Kind::Service, &[]).unwrap();
    w.section_with(Kind::TierMap, 9, |out| {
        for (i, b) in out.iter_mut().enumerate() {
            *b = i as u8 + 1;
        }
    })
    .unwrap();
    w.finish(seq)
}

#[test]
fn a_record_reads_back_section_by_section() {
    let mut buf = vec![0xEEu8; 1024];
    let len = sample(&mut buf, 9);
    assert_eq!(len, HEADER + (8 + 8) + (8 + 68) + 8 + (8 + 12));
    let r = Record::parse(&buf).unwrap();
    assert_eq!((r.seq(), r.len(), r.is_empty()), (9, 4, false));
    let got: Vec<(Kind, Vec<u8>)> = r.sections().map(|(k, p)| (k, p.to_vec())).collect();
    assert_eq!(
        got,
        vec![
            (Kind::Process, vec![1, 2, 3, 4, 5]),
            (Kind::Thread, vec![0xAA; 68]),
            (Kind::Service, vec![]),
            (Kind::TierMap, (1..=9).collect()),
        ]
    );
    // An empty record is a record.
    let mut e = vec![0u8; 64];
    let w = Writer::new(&mut e).unwrap();
    assert_eq!(w.finish(1), HEADER);
    let r = Record::parse(&e).unwrap();
    assert!(r.is_empty() && r.sections().next().is_none());
}

#[test]
fn every_single_bit_flip_is_refused() {
    let mut buf = vec![0u8; 512];
    let len = sample(&mut buf, 3);
    for byte in 0..len {
        for bit in 0..8 {
            let mut b = buf.clone();
            b[byte] ^= 1 << bit;
            assert!(
                Record::parse(&b).is_err(),
                "byte {byte} bit {bit} got through"
            );
        }
    }
    // Bytes past the record are not its business.
    let mut b = buf.clone();
    b[len] ^= 0xFF;
    assert!(Record::parse(&b).is_ok());
}

/// A header with a body length and a section of our choosing, hashes
/// made right, so only the bounds checks stand in the way.
fn forged(body: &[u8], count: u16, body_len: u32, seq: u32) -> Vec<u8> {
    let mut b = vec![0u8; HEADER + body.len()];
    b[HEADER..].copy_from_slice(body);
    b[..4].copy_from_slice(b"RPGM");
    b[4..6].copy_from_slice(&VERSION.to_le_bytes());
    b[6..8].copy_from_slice(&count.to_le_bytes());
    b[8..12].copy_from_slice(&body_len.to_le_bytes());
    b[12..16].copy_from_slice(&seq.to_le_bytes());
    let n = (body_len as usize).min(body.len());
    let bh = xxh32(&body[..n], seq);
    b[16..20].copy_from_slice(&bh.to_le_bytes());
    let hh = xxh32(&b[4..28], 0);
    b[28..32].copy_from_slice(&hh.to_le_bytes());
    b
}

fn section(kind: u16, flags: u16, len: u32, payload: &[u8]) -> Vec<u8> {
    let mut s = Vec::new();
    s.extend_from_slice(&kind.to_le_bytes());
    s.extend_from_slice(&flags.to_le_bytes());
    s.extend_from_slice(&len.to_le_bytes());
    s.extend_from_slice(payload);
    s
}

#[test]
fn lengths_and_kinds_are_checked_before_use() {
    let ok = section(1, 0, 4, &[1, 2, 3, 4]);
    assert!(
        Record::parse(&forged(&ok, 1, 12, 5)).is_ok(),
        "the forger is right"
    );
    // The body longer than the buffer, unaligned, or u32::MAX.
    assert_eq!(
        Record::parse(&forged(&ok, 1, 16, 5)).err(),
        Some(Error::Length)
    );
    assert_eq!(
        Record::parse(&forged(&ok, 1, 10, 5)).err(),
        Some(Error::Length)
    );
    assert_eq!(
        Record::parse(&forged(&ok, 1, u32::MAX, 5)).err(),
        Some(Error::Length)
    );
    // A section's length past the body, and the largest lengths.
    for len in [5, 8, 0xFFFF_FFFC, u32::MAX] {
        let s = section(1, 0, len, &[1, 2, 3, 4]);
        assert_eq!(
            Record::parse(&forged(&s, 1, 12, 5)).err(),
            Some(Error::Section),
            "{len}"
        );
    }
    // Padding that isn't zero, flags, a short section header.
    let s = section(1, 0, 3, &[1, 2, 3, 9]);
    assert_eq!(
        Record::parse(&forged(&s, 1, 12, 5)).err(),
        Some(Error::Section)
    );
    let s = section(1, 1, 4, &[1, 2, 3, 4]);
    assert_eq!(
        Record::parse(&forged(&s, 1, 12, 5)).err(),
        Some(Error::Section)
    );
    let s = vec![1, 0, 0, 0];
    assert_eq!(
        Record::parse(&forged(&s, 1, 4, 5)).err(),
        Some(Error::Section)
    );
    // The count must match.
    assert_eq!(
        Record::parse(&forged(&ok, 2, 12, 5)).err(),
        Some(Error::Section)
    );
    assert_eq!(
        Record::parse(&forged(&ok, 0, 12, 5)).err(),
        Some(Error::Section)
    );
    // Kinds this kernel doesn't know.
    for kind in [0u16, 5, u16::MAX] {
        let s = section(kind, 0, 4, &[1, 2, 3, 4]);
        assert_eq!(
            Record::parse(&forged(&s, 1, 12, 5)).err(),
            Some(Error::Kind)
        );
    }
    // The reserved header bytes must be zero, even with the hash made right
    // (kept for a later format, never read as anything now).
    let mut r = forged(&ok, 1, 12, 5);
    r[23] = 1;
    let hh = xxh32(&r[4..28], 0);
    r[28..32].copy_from_slice(&hh.to_le_bytes());
    assert_eq!(Record::parse(&r).err(), Some(Error::HeaderHash));
    // Another version, and nothing at all.
    let mut v = forged(&ok, 1, 12, 5);
    v[4] = 2;
    assert_eq!(Record::parse(&v).err(), Some(Error::Version));
    assert_eq!(Record::parse(&[0u8; 64]).err(), Some(Error::Empty));
    assert_eq!(Record::parse(&[0u8; 31]).err(), Some(Error::TooSmall));
}

#[test]
fn a_stale_body_under_a_newer_header_is_refused() {
    let mut a = vec![0u8; 256];
    sample(&mut a, 4);
    // The header renumbered (5) and its own hash made right: the body's
    // hash was seeded with 4, so it no longer matches.
    let mut c = a.clone();
    c[12..16].copy_from_slice(&5u32.to_le_bytes());
    let hh = xxh32(&c[4..28], 0);
    c[28..32].copy_from_slice(&hh.to_le_bytes());
    assert_eq!(Record::parse(&c).err(), Some(Error::BodyHash));
}

#[test]
fn a_record_too_big_for_its_half_is_refused_while_written() {
    let mut buf = vec![0u8; 64];
    let mut w = Writer::new(&mut buf).unwrap();
    w.section(Kind::Thread, &[0; 24]).unwrap(); // 32 + 8 + 24 = 64
    assert_eq!(w.section(Kind::Thread, &[]), Err(Error::Full));
    assert_eq!(
        w.section_with(Kind::Thread, usize::MAX, |_| {}),
        Err(Error::Full)
    );
    assert_eq!(w.finish(1), 64);
    assert!(Record::parse(&buf).is_ok());
    assert!(Writer::new(&mut [0u8; 31]).is_err());
}

#[test]
fn commit_alternates_halves_and_numbers_them() {
    let mut reserve = vec![0u8; 2048];
    assert!(newest(&reserve).is_none());
    let put =
        |r: &mut Vec<u8>, tag: u8| commit(r, &mut |w| w.section(Kind::Service, &[tag; 6])).unwrap();
    assert_eq!(put(&mut reserve, 1).0, 1);
    assert_eq!(newest(&reserve).map(|(h, r)| (h, r.seq())), Some((0, 1)));
    assert_eq!(put(&mut reserve, 2).0, 2);
    assert_eq!(newest(&reserve).map(|(h, r)| (h, r.seq())), Some((1, 2)));
    assert_eq!(put(&mut reserve, 3).0, 3);
    let (h, r) = newest(&reserve).unwrap();
    assert_eq!((h, r.seq()), (0, 3));
    assert_eq!(r.sections().next().unwrap().1, &[3; 6]);
    // A failed build leaves its half empty and the old record newest.
    let e = commit(&mut reserve, &mut |w| w.section(Kind::Service, &[0; 4096]));
    assert_eq!(e, Err(Error::Full));
    assert_eq!(newest(&reserve).map(|(h, r)| (h, r.seq())), Some((0, 3)));
    // Damage to the newest falls back to the other.
    reserve[HEADER + 9] ^= 1;
    assert!(
        newest(&reserve).is_none(),
        "half 1 was emptied by the failed build"
    );
    // With neither good, numbering starts again in half 0.
    put(&mut reserve, 4);
    assert_eq!(newest(&reserve).map(|(h, r)| (h, r.seq())), Some((0, 1)));
}

#[test]
fn sequence_numbers_wrap() {
    assert!(newer(1, 0) && newer(0, u32::MAX) && !newer(u32::MAX, 0) && !newer(5, 5));
    let mut reserve = vec![0u8; 512];
    {
        let (a, _) = reserve.split_at_mut(256);
        let w = Writer::new(a).unwrap();
        w.finish(u32::MAX);
    }
    let (seq, _) = commit(&mut reserve, &mut |_| Ok(())).unwrap();
    assert_eq!(seq, 0);
    assert_eq!(newest(&reserve).map(|(h, r)| (h, r.seq())), Some((1, 0)));
}

/// The bytes of the half a commit writes, in the order it writes them:
/// the magic cleared, the body, the header's fields, the magic last.
fn write_order(new: &[u8], len: usize) -> Vec<(usize, u8)> {
    let mut steps: Vec<(usize, u8)> = (0..4).map(|i| (i, 0)).collect();
    steps.extend((HEADER..len).map(|i| (i, new[i])));
    steps.extend((4..HEADER).map(|i| (i, new[i])));
    steps.extend((0..4).map(|i| (i, new[i])));
    steps
}

#[test]
fn a_reset_at_any_byte_of_a_commit_leaves_the_last_good_record() {
    let mut reserve = vec![0u8; 1024];
    commit(&mut reserve, &mut |w| w.section(Kind::Process, &[1; 10])).unwrap();
    commit(&mut reserve, &mut |w| w.section(Kind::Process, &[2; 10])).unwrap();
    // The next commit goes into half 0 (seq 3), over record 1.
    let before = reserve.clone();
    let mut after = reserve.clone();
    let (seq, len) = commit(&mut after, &mut |w| {
        w.section(Kind::Thread, &[3; 40])?;
        w.section(Kind::TierMap, &[4; 17])
    })
    .unwrap();
    assert_eq!(seq, 3);
    let steps = write_order(&after[..512], len);
    for k in 0..steps.len() {
        let mut torn = before.clone();
        for &(i, v) in &steps[..k] {
            torn[i] = v;
        }
        let (h, r) = newest(&torn).expect("some record survives");
        assert_eq!(
            (h, r.seq()),
            (1, 2),
            "reset after {k} of {} writes",
            steps.len()
        );
    }
    let mut whole = before.clone();
    for &(i, v) in &steps {
        whole[i] = v;
    }
    assert_eq!(newest(&whole).map(|(h, r)| (h, r.seq())), Some((0, 3)));
    assert_eq!(whole, after, "the order covers every byte the commit wrote");
}
