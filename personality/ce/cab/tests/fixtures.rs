//! rp-cab against synthetic cabinets (fixtures/make.py, compressed by
//! Python's zlib), hostile variants of them, and the owner's real CABs when
//! present in `local/legacysync/real` (never committed; skipped otherwise).

use rp_cab::{Cabinet, Error, Scratch, Unsupported};

const STORED: &[u8] = include_bytes!("../fixtures/stored.mscf");
const MSZIP: &[u8] = include_bytes!("../fixtures/mszip.mscf");
const RESERVE: &[u8] = include_bytes!("../fixtures/reserve.mscf");

/// Mirrors make.py's `gen`.
fn content(seed: u64, n: usize) -> Vec<u8> {
    const WORDS: [&[u8]; 8] = [
        b"golf", b"toy", b"rally", b"pocket", b"axim", b"cab", b"stunt", b"pool",
    ];
    let mut out = Vec::new();
    let mut x = seed;
    while out.len() < n {
        x = (x * 1_103_515_245 + 12_345) & 0x7FFF_FFFF;
        out.extend_from_slice(WORDS[((x >> 16) % 8) as usize]);
        out.push(b' ');
    }
    out.truncate(n);
    out
}

/// A file's name and bytes.
type Extracted = (Vec<u8>, Vec<u8>);

/// Every file's name and bytes, or the first error.
fn extract_all(bytes: &[u8]) -> Result<Vec<Extracted>, Error> {
    let mut src = bytes;
    let cab = Cabinet::open(&mut src)?;
    let files: Vec<_> = cab.files(&mut src).collect::<Result<_, _>>()?;
    let mut scratch = Box::new(Scratch::new());
    let mut out = Vec::new();
    for f in &files {
        let mut data = Vec::new();
        cab.extract_file(&mut src, f, &mut scratch, |at, b| {
            assert_eq!(at as usize, data.len(), "pieces arrive in order");
            data.extend_from_slice(b);
            Ok(())
        })?;
        assert_eq!(data.len(), f.size as usize);
        out.push((f.name().to_vec(), data));
    }
    Ok(out)
}

#[test]
fn uncompressed_folders_and_files_across_block_edges() {
    let got = extract_all(STORED).unwrap();
    assert_eq!(got.len(), 3);
    assert_eq!(got[0], (b"APP~1.001".to_vec(), content(1, 2500)));
    assert_eq!(got[1], (b"Read me.002".to_vec(), content(2, 10)));
    assert_eq!(got[2], (b"data.003".to_vec(), content(3, 1800)));
}

#[test]
fn mszip_with_history_across_blocks_and_an_empty_file() {
    let got = extract_all(MSZIP).unwrap();
    assert_eq!(got[0], (b"big.001".to_vec(), content(4, 80_000)));
    assert_eq!(got[1], (b"small.002".to_vec(), content(5, 300)));
    assert_eq!(got[2], (b"empty.003".to_vec(), Vec::new()));
}

#[test]
fn reserved_areas_are_skipped() {
    let got = extract_all(RESERVE).unwrap();
    assert_eq!(got, vec![(b"res.001".to_vec(), content(6, 5000))]);
}

/// Offsets of each data block header in a cabinet without reserved areas.
fn block_headers(bytes: &[u8]) -> Vec<usize> {
    let mut src = bytes;
    let cab = Cabinet::open(&mut src).unwrap();
    let mut out = Vec::new();
    for i in 0..cab.folder_count() {
        let f = cab.folder(&mut src, i).unwrap();
        let mut at = f.data_offset as usize;
        for _ in 0..f.blocks {
            out.push(at);
            at += 8 + usize::from(u16::from_le_bytes([bytes[at + 4], bytes[at + 5]]));
        }
    }
    out
}

#[test]
fn a_changed_byte_fails_the_checksum() {
    let mut bad = STORED.to_vec();
    let at = block_headers(STORED)[1] + 8 + 17;
    bad[at] ^= 0x40;
    assert_eq!(extract_all(&bad).unwrap_err(), Error::Checksum);
}

#[test]
fn spanning_sets_are_refused_by_flag_and_by_file_entry() {
    let mut bad = STORED.to_vec();
    bad[30] |= 0x02;
    assert_eq!(
        extract_all(&bad).unwrap_err(),
        Error::Unsupported(Unsupported::Spanning)
    );
    let mut bad = STORED.to_vec();
    let files_at = u32::from_le_bytes(bad[16..20].try_into().unwrap()) as usize;
    bad[files_at + 8..files_at + 10].copy_from_slice(&0xFFFDu16.to_le_bytes());
    assert_eq!(
        extract_all(&bad).unwrap_err(),
        Error::Unsupported(Unsupported::Spanning)
    );
}

#[test]
fn lzx_and_quantum_are_named() {
    let mut bad = STORED.to_vec();
    bad[36 + 6] = 3;
    assert_eq!(
        extract_all(&bad).unwrap_err(),
        Error::Unsupported(Unsupported::Lzx)
    );
    bad[36 + 6] = 2;
    assert_eq!(
        extract_all(&bad).unwrap_err(),
        Error::Unsupported(Unsupported::Quantum)
    );
}

#[test]
fn a_file_past_its_folder_end_is_truncated() {
    let mut bad = STORED.to_vec();
    let files_at = u32::from_le_bytes(bad[16..20].try_into().unwrap()) as usize;
    // First file: size 2500 -> 4000 (the folder holds 2510 bytes).
    bad[files_at..files_at + 4].copy_from_slice(&4000u32.to_le_bytes());
    assert_eq!(extract_all(&bad).unwrap_err(), Error::Truncated);
}

#[test]
fn every_truncation_is_an_error_not_a_panic() {
    for fixture in [STORED, RESERVE, MSZIP] {
        let step = if fixture.len() > 10_000 { 7 } else { 1 };
        for n in (0..fixture.len()).step_by(step) {
            assert!(extract_all(&fixture[..n]).is_err(), "length {n}");
        }
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// The MSZIP fixture with every checksum zeroed, so corrupted compressed
/// data reaches the decoder instead of failing the checksum.
fn mszip_unchecked() -> Vec<u8> {
    let mut v = MSZIP.to_vec();
    for at in block_headers(MSZIP) {
        v[at..at + 4].fill(0);
    }
    v
}

#[test]
fn hostile_mutations_never_panic_or_hang() {
    let unchecked = mszip_unchecked();
    assert!(extract_all(&unchecked).is_ok());
    let bases: [&[u8]; 4] = [STORED, RESERVE, MSZIP, &unchecked];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut errors = 0;
    for case in 0..20_000 {
        let base = bases[case % bases.len()];
        let mut v = base.to_vec();
        for _ in 0..1 + rng.next() % 8 {
            let at = (rng.next() as usize) % v.len();
            v[at] = match rng.next() % 4 {
                0 => 0,
                1 => 0xFF,
                _ => rng.next() as u8,
            };
        }
        if extract_all(&v).is_err() {
            errors += 1;
        }
    }
    // Most mutations must be caught; the rest decoded (e.g. a changed date).
    assert!(
        errors > 5_000,
        "only {errors} of 20000 mutations were refused"
    );
}

#[test]
fn random_bytes_behind_a_valid_signature_never_panic() {
    let mut rng = Rng(42);
    for _ in 0..5_000 {
        let len = 36 + (rng.next() as usize) % 400;
        let mut v: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        v[..4].copy_from_slice(b"MSCF");
        v[24] = 3;
        v[25] = 1;
        v[8..12].copy_from_slice(&(len as u32).to_le_bytes());
        let _ = extract_all(&v);
    }
}

#[test]
fn the_owners_real_cabs_extract_whole() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../local/legacysync/real");
    let Ok(dirs) = std::fs::read_dir(&root) else {
        eprintln!("skipped: no real CABs in {}", root.display());
        return;
    };
    let mut cabs = 0;
    for dir in dirs.flatten() {
        for entry in std::fs::read_dir(dir.path()).unwrap().flatten() {
            let path = entry.path();
            let is_cab = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("cab"));
            if !is_cab {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            let files = extract_all(&bytes).unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
            let script = files
                .iter()
                .find(|(n, _)| n.ends_with(b".000"))
                .expect("a .000 setup script");
            assert_eq!(&script.1[..4], b"MSCE", "{}", path.display());
            cabs += 1;
        }
    }
    eprintln!("{cabs} real CABs extracted");
}
