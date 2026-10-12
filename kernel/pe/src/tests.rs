//! Tests with synthetic images built here (the layout from the public PE
//! spec), plus, when present, real WM6 DLLs from a local ROM extraction.

use super::*;

/// Builds a PE32 ARM image. `sections`: (name, rva, virtual size, raw data,
/// characteristics). Raw data is placed at 0x200-aligned file offsets, as CE
/// linkers do (FileAlignment 0x200, SectionAlignment 0x1000).
struct Builder {
    machine: u16,
    base: u32,
    dll: bool,
    sections: Vec<(&'static str, u32, u32, Vec<u8>, u32)>,
    reloc_dir: Option<(u32, u32)>,
}

impl Builder {
    fn new() -> Self {
        Self {
            machine: MACHINE_THUMB,
            base: 0x0001_0000,
            dll: false,
            sections: Vec::new(),
            reloc_dir: None,
        }
    }

    fn section(mut self, name: &'static str, rva: u32, vsize: u32, raw: Vec<u8>, ch: u32) -> Self {
        self.sections.push((name, rva, vsize, raw, ch));
        self
    }

    fn build(&self) -> Vec<u8> {
        let pe = 0x80usize;
        let opt = pe + 24;
        let opt_size = 96 + 16 * 8;
        let table = opt + opt_size;
        let header_size = (table + 40 * self.sections.len()).next_multiple_of(0x200);
        let mut f = vec![0u8; header_size];
        f[0..2].copy_from_slice(b"MZ");
        f[0x3C..0x40].copy_from_slice(&(pe as u32).to_le_bytes());
        f[pe..pe + 4].copy_from_slice(b"PE\0\0");
        let w16 =
            |f: &mut Vec<u8>, at: usize, v: u16| f[at..at + 2].copy_from_slice(&v.to_le_bytes());
        let w32 =
            |f: &mut Vec<u8>, at: usize, v: u32| f[at..at + 4].copy_from_slice(&v.to_le_bytes());
        w16(&mut f, pe + 4, self.machine);
        w16(&mut f, pe + 6, self.sections.len() as u16);
        w16(&mut f, pe + 20, opt_size as u16);
        w16(&mut f, pe + 22, if self.dll { 0x2102 } else { 0x0102 });
        w16(&mut f, opt, 0x10B);
        w32(&mut f, opt + 16, 0x1000); // entry
        w32(&mut f, opt + 28, self.base);
        w32(&mut f, opt + 32, 0x1000);
        w32(&mut f, opt + 36, 0x200);
        let image_size = self
            .sections
            .iter()
            .map(|s| (s.1 + s.2).next_multiple_of(0x1000))
            .max()
            .unwrap_or(0x1000);
        w32(&mut f, opt + 56, image_size);
        w32(&mut f, opt + 60, header_size as u32);
        w16(&mut f, opt + 68, 9);
        w32(&mut f, opt + 92, 16);
        if let Some((rva, size)) = self.reloc_dir {
            w32(&mut f, opt + 96 + dir::BASERELOC * 8, rva);
            w32(&mut f, opt + 100 + dir::BASERELOC * 8, size);
        }
        for (i, (name, rva, vsize, raw, ch)) in self.sections.iter().enumerate() {
            let at = table + i * 40;
            let off = f.len();
            f[at..at + name.len()].copy_from_slice(name.as_bytes());
            w32(&mut f, at + 8, *vsize);
            w32(&mut f, at + 12, *rva);
            w32(&mut f, at + 16, raw.len() as u32);
            w32(&mut f, at + 20, if raw.is_empty() { 0 } else { off as u32 });
            w32(&mut f, at + 36, *ch);
            f.extend_from_slice(raw);
            let padded = f.len().next_multiple_of(0x200);
            f.resize(padded, 0);
        }
        f
    }
}

const TEXT: u32 = scn::CODE | scn::EXECUTE | scn::READ;
const DATA: u32 = scn::READ | scn::WRITE;
const BSS: u32 = scn::UNINIT | scn::READ | scn::WRITE;

fn pattern(n: usize, seed: u8) -> Vec<u8> {
    (0..n)
        .map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed))
        .collect()
}

fn reader(file: &[u8]) -> impl FnMut(u32, &mut [u8]) -> Result<(), ()> + '_ {
    move |off, buf| {
        let src = file.get(off as usize..off as usize + buf.len()).ok_or(())?;
        buf.copy_from_slice(src);
        Ok(())
    }
}

fn sample() -> (Vec<u8>, Builder) {
    let b = Builder::new()
        .section(".text", 0x1000, 0x1200, pattern(0x1200, 1), TEXT)
        .section(".data", 0x3000, 0x1800, pattern(0x300, 9), DATA)
        .section(".bss", 0x5000, 0x800, Vec::new(), BSS);
    (b.build(), b)
}

#[test]
fn parses_headers_and_sections() {
    let (file, _) = sample();
    let img = Image::parse(&file).unwrap();
    assert_eq!(img.machine, MACHINE_THUMB);
    assert_eq!(img.image_base, 0x10000);
    assert_eq!(img.entry, 0x1000);
    assert_eq!(img.subsystem, 9);
    assert!(!img.is_dll());
    let names: Vec<_> = img.sections().map(|s| s.name).collect();
    assert_eq!(&names[0][..5], b".text");
    assert_eq!(img.sections().count(), 3);
    assert_eq!(img.pages(), 6);
}

#[test]
fn rejects_malformed_images() {
    let (file, _) = sample();
    assert_eq!(Image::parse(&file[..0x40]), Err(Error::Truncated));
    assert_eq!(Image::parse(b"XX"), Err(Error::NotPe));
    let mut x86 = file.clone();
    x86[0x84..0x86].copy_from_slice(&0x014Cu16.to_le_bytes());
    assert_eq!(Image::parse(&x86), Err(Error::Unsupported));
    // Sections out of order / overlapping.
    let bad = Builder::new()
        .section(".a", 0x2000, 0x1000, vec![1], DATA)
        .section(".b", 0x1000, 0x1000, vec![1], DATA)
        .build();
    assert_eq!(Image::parse(&bad), Err(Error::BadLayout));
    // Garbage never panics.
    for cut in 0..file.len().min(0x300) {
        let _ = Image::parse(&file[..cut]);
    }
}

#[test]
fn pages_are_assembled_from_unaligned_file_data() {
    let (file, _) = sample();
    let img = Image::parse(&file).unwrap();
    let text = img.sections().next().copied().unwrap();
    assert_eq!(
        text.raw_offset % 0x200,
        0,
        "file data is 512-byte aligned, not page aligned"
    );
    let mut page = [0u8; 4096];
    // Page 1 = first 4 KiB of .text.
    img.fill_page(1, &mut page, reader(&file)).unwrap();
    assert_eq!(&page[..], &pattern(0x1200, 1)[..4096]);
    // Page 2 = the last 0x200 bytes of .text, then zeros.
    img.fill_page(2, &mut page, reader(&file)).unwrap();
    assert_eq!(&page[..0x200], &pattern(0x1200, 1)[4096..]);
    assert!(page[0x200..].iter().all(|&b| b == 0));
    // .data: 0x300 bytes from the file, the rest of its 0x1800 zero.
    img.fill_page(3, &mut page, reader(&file)).unwrap();
    assert_eq!(&page[..0x300], &pattern(0x300, 9)[..]);
    assert!(page[0x300..].iter().all(|&b| b == 0));
    // .bss: nothing read from the file.
    let mut reads = 0;
    img.fill_page(5, &mut page, |_, _: &mut [u8]| -> Result<(), ()> {
        reads += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(reads, 0);
    assert!(page.iter().all(|&b| b == 0));
    // Page 0 holds the headers.
    img.fill_page(0, &mut page, reader(&file)).unwrap();
    assert_eq!(&page[..2], b"MZ");
    assert_eq!(
        img.fill_page(6, &mut page, reader(&file)),
        Err(FillError::Image(Error::BadLayout))
    );
}

#[test]
fn read_errors_propagate() {
    let (file, _) = sample();
    let img = Image::parse(&file).unwrap();
    let mut page = [0u8; 4096];
    let r = img.fill_page(1, &mut page, |_, _: &mut [u8]| Err::<(), _>("disk"));
    assert_eq!(r, Err(FillError::Read("disk")));
}

#[test]
fn page_protection_follows_sections() {
    let (file, _) = sample();
    let img = Image::parse(&file).unwrap();
    let ro = Protection::default();
    let rx = Protection { exec: true, ..ro };
    let rw = Protection { write: true, ..ro };
    assert_eq!(img.page_protection(0), Some(ro), "headers");
    assert_eq!(img.page_protection(1), Some(rx));
    assert_eq!(img.page_protection(2), Some(rx));
    assert_eq!(img.page_protection(3), Some(rw));
    assert_eq!(img.page_protection(5), Some(rw), ".bss");
    // A section shared by code and data in one page gets both (W^X is the
    // address space's call: legacy opt-in or refuse).
    let mixed = Builder::new()
        .section(".text", 0x1000, 0x100, vec![0; 0x100], TEXT)
        .section(".data", 0x1100, 0x100, vec![0; 0x100], DATA)
        .build();
    let m = Image::parse(&mixed).unwrap();
    assert_eq!(
        m.page_protection(1),
        Some(Protection {
            write: true,
            exec: true,
            shared: false
        })
    );
}

fn block(page: u32, entries: &[u16]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&page.to_le_bytes());
    let size = 8 + 2 * entries.len() as u32;
    b.extend_from_slice(&size.to_le_bytes());
    for e in entries {
        b.extend_from_slice(&e.to_le_bytes());
    }
    b
}

const HIGHLOW: u16 = 3 << 12;

#[test]
fn relocates_one_page_at_a_time() {
    let mut data = block(0x1000, &[HIGHLOW | 0x010, HIGHLOW | 0x800, 0]); // 0 = padding
    data.extend(block(0x3000, &[HIGHLOW | 0x004]));
    let rel = Relocations::new(&data).unwrap();
    let mut page = [0u8; 4096];
    page[0x10..0x14].copy_from_slice(&0x0001_2345u32.to_le_bytes());
    page[0x800..0x804].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    page[0x20..0x24].copy_from_slice(&0x0001_1111u32.to_le_bytes()); // not listed
    rel.apply(1, &mut page, [0; 4], [0; 4], 0x0100_0000);
    assert_eq!(&page[0x10..0x14], &0x0101_2345u32.to_le_bytes());
    assert_eq!(&page[0x800..0x804], &0x0101_0000u32.to_le_bytes());
    assert_eq!(
        &page[0x20..0x24],
        &0x0001_1111u32.to_le_bytes(),
        "untouched"
    );
    // Page 2 has no block: nothing changes.
    let mut p2 = [0xAB; 4096];
    rel.apply(2, &mut p2, [0; 4], [0; 4], 0x0100_0000);
    assert!(p2.iter().all(|&b| b == 0xAB));
    // Delta 0 (loaded at the preferred base): nothing to do at all.
    let mut p1 = [0x11; 4096];
    rel.apply(1, &mut p1, [0; 4], [0; 4], 0);
    assert!(p1.iter().all(|&b| b == 0x11));
}

#[test]
fn values_straddling_a_page_boundary_are_fixed_from_both_sides() {
    // A 32-bit value at RVA 0x1FFE: two bytes in page 1, two in page 2.
    let data = block(0x1000, &[HIGHLOW | 0xFFE]);
    let rel = Relocations::new(&data).unwrap();
    let value = 0x0001_FFF0u32.to_le_bytes(); // + delta carries across bytes
    let delta = 0x0000_0020u32;
    let want = (0x0001_FFF0u32 + delta).to_le_bytes();
    let mut p1 = [0u8; 4096];
    p1[0xFFE..].copy_from_slice(&value[..2]);
    let mut p2 = [0u8; 4096];
    p2[..2].copy_from_slice(&value[2..]);
    let (orig1, orig2) = (p1, p2);
    // Each page relocated on its own, given the other's original bytes.
    let next = [orig2[0], orig2[1], orig2[2], orig2[3]];
    let prev = [orig1[0xFFC], orig1[0xFFD], orig1[0xFFE], orig1[0xFFF]];
    rel.apply(1, &mut p1, [0; 4], next, delta);
    rel.apply(2, &mut p2, prev, [0; 4], delta);
    assert_eq!(&p1[0xFFE..], &want[..2]);
    assert_eq!(&p2[..2], &want[2..], "the carry reached the second page");
    assert_eq!(&p2[2..4], &[0, 0]);
}

#[test]
fn relocation_framing_and_types_are_checked_up_front() {
    assert!(
        Relocations::new(&block(0x1000, &[1 << 12])).is_err(),
        "HIGH not implemented"
    );
    assert!(
        Relocations::new(&block(0x1001, &[])).is_err(),
        "unaligned page"
    );
    let mut twice = block(0x2000, &[]);
    twice.extend(block(0x1000, &[]));
    assert!(Relocations::new(&twice).is_err(), "blocks must ascend");
    let mut short = block(0x1000, &[HIGHLOW]);
    short.truncate(9);
    assert!(Relocations::new(&short).is_err(), "truncated block");
    // Zero padding after the last block is fine.
    let mut padded = block(0x1000, &[HIGHLOW]);
    padded.extend([0u8; 8]);
    assert!(Relocations::new(&padded).is_ok());
}

#[test]
fn view_reads_cover_ranges_across_pages() {
    let (file, _) = sample();
    let img = Image::parse(&file).unwrap();
    let mut four = [0u8; 4];
    img.read_view(0x1FFE, &mut four, reader(&file)).unwrap();
    assert_eq!(&four[..], &pattern(0x1200, 1)[0xFFE..0x1002]);
    img.read_view(0x10_0000, &mut four, reader(&file)).unwrap();
    assert_eq!(four, [0; 4], "past the image reads as zero");
}

#[test]
fn imports_by_name_and_ordinal() {
    // .idata at RVA 0x2000: one descriptor (coredll.dll), lookup table
    // absent (CE style), IAT with an ordinal and a named import.
    let mut idata = vec![0u8; 0x200];
    let w32 = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    // Descriptor: lookup 0, name at 0x2080, IAT at 0x2040; then a zero one.
    w32(&mut idata, 12, 0x2080);
    w32(&mut idata, 16, 0x2040);
    // IAT: ordinal 1234, then name entry at 0x20A0, then 0.
    w32(&mut idata, 0x40, 0x8000_0000 | 1234);
    w32(&mut idata, 0x44, 0x20A0);
    idata[0x80..0x8C].copy_from_slice(b"coredll.dll\0");
    idata[0xA0..0xA2].copy_from_slice(&7u16.to_le_bytes());
    idata[0xA2..0xAE].copy_from_slice(b"CreateFileW\0");
    let mut b = Builder::new().section(".idata", 0x2000, 0x200, idata, DATA);
    let mut file = b.build();
    // Point the import directory at it.
    let opt = 0x80 + 24;
    file[opt + 96 + 8..opt + 100 + 8].copy_from_slice(&0x2000u32.to_le_bytes());
    file[opt + 100 + 8..opt + 104 + 8].copy_from_slice(&40u32.to_le_bytes());
    b.reloc_dir = None;
    let img = Image::parse(&file).unwrap();
    let mut got = Vec::new();
    for_each_import(&img, reader(&file), |dll, imp, slot| {
        let imp = match imp {
            Import::Ordinal(n) => format!("#{n}"),
            Import::Name { hint, name } => format!("{}@{hint}", String::from_utf8_lossy(name)),
        };
        got.push((String::from_utf8_lossy(dll).into_owned(), imp, slot));
    })
    .unwrap();
    assert_eq!(
        got,
        [
            ("coredll.dll".into(), "#1234".into(), 0x2040),
            ("coredll.dll".into(), "CreateFileW@7".into(), 0x2044),
        ]
    );
}

#[test]
fn exports_by_ordinal_and_name() {
    // .edata at 0x2000: base ordinal 5, three functions (one gap, one
    // forwarder), two names sorted: "Alpha" -> index 0, "Beta" -> index 2.
    let mut e = vec![0u8; 0x200];
    let w32 = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    w32(&mut e, 16, 5); // base
    w32(&mut e, 20, 3); // functions
    w32(&mut e, 24, 2); // names
    w32(&mut e, 28, 0x2040); // address table
    w32(&mut e, 32, 0x2060); // name pointers
    w32(&mut e, 36, 0x2070); // name ordinals
    w32(&mut e, 0x40, 0x1234); // ordinal 5
    w32(&mut e, 0x44, 0); // ordinal 6: gap
    w32(&mut e, 0x48, 0x20A0); // ordinal 7: forwarder (inside the directory)
    w32(&mut e, 0x60, 0x2080);
    w32(&mut e, 0x64, 0x2088);
    e[0x70..0x72].copy_from_slice(&0u16.to_le_bytes());
    e[0x72..0x74].copy_from_slice(&2u16.to_le_bytes());
    e[0x80..0x86].copy_from_slice(b"Alpha\0");
    e[0x88..0x8D].copy_from_slice(b"Beta\0");
    e[0xA0..0xAD].copy_from_slice(b"OTHER.Thing\0\0");
    let mut file = Builder::new()
        .section(".edata", 0x2000, 0x200, e, DATA)
        .build();
    let opt = 0x80 + 24;
    file[opt + 96..opt + 100].copy_from_slice(&0x2000u32.to_le_bytes());
    file[opt + 100..opt + 104].copy_from_slice(&0x100u32.to_le_bytes());
    let img = Image::parse(&file).unwrap();
    let ex = Exports::read(&img, reader(&file)).unwrap().unwrap();
    assert_eq!(
        ex.by_ordinal(&img, 5, reader(&file)),
        Ok(Some(Export::Rva(0x1234)))
    );
    assert_eq!(ex.by_ordinal(&img, 6, reader(&file)), Ok(None), "gap");
    assert_eq!(
        ex.by_ordinal(&img, 7, reader(&file)),
        Ok(Some(Export::Forwarder(0x20A0)))
    );
    assert_eq!(ex.by_ordinal(&img, 4, reader(&file)), Ok(None));
    assert_eq!(ex.by_ordinal(&img, 8, reader(&file)), Ok(None));
    assert_eq!(
        ex.by_name(&img, b"Alpha", 0, reader(&file)),
        Ok(Some(Export::Rva(0x1234)))
    );
    // A wrong hint falls back to the binary search.
    assert_eq!(
        ex.by_name(&img, b"Beta", 0, reader(&file)),
        Ok(Some(Export::Forwarder(0x20A0)))
    );
    assert_eq!(ex.by_name(&img, b"Gamma", 1, reader(&file)), Ok(None));
    assert_eq!(ex.by_name(&img, b"Alph", 9, reader(&file)), Ok(None));
}

/// Real WM6 DLLs from a local ROM extraction (`local/` is git-ignored and
/// never distributed): every one must parse and fill without error. Skipped
/// when the files aren't there (CI).
#[test]
fn real_wm6_dlls_parse_and_fill() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../local/drivers-a08");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut checked = 0;
    for e in entries.flatten() {
        let path = e.path();
        if path
            .extension()
            .is_none_or(|x| !x.eq_ignore_ascii_case("dll"))
        {
            continue;
        }
        let file = std::fs::read(&path).unwrap();
        let img =
            Image::parse(&file).unwrap_or_else(|err| panic!("{}: {}", path.display(), err.name()));
        let mut page = [0u8; 4096];
        for p in 0..img.pages() {
            img.fill_page(p, &mut page, reader(&file))
                .unwrap_or_else(|_| panic!("{} page {p}", path.display()));
        }
        let d = img.dirs[dir::BASERELOC];
        if d.size != 0 {
            let mut rel = vec![0u8; d.size as usize];
            img.read_view(d.rva, &mut rel, reader(&file)).unwrap();
            Relocations::new(&rel)
                .unwrap_or_else(|err| panic!("{} relocs: {}", path.display(), err.name()));
        }
        checked += 1;
    }
    eprintln!("checked {checked} real DLLs");
}

/// Hostile input: thousands of randomly mutated images (headers, section
/// table, directories) through every entry point; nothing may panic, and
/// every page fill stays inside the page.
#[test]
fn mutated_images_never_panic() {
    let (base, _) = sample();
    let mut x: u64 = 0x5045_2026;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut page = [0u8; PAGE as usize];
    for i in 0..5000 {
        let mut file = base.clone();
        let region = if i % 2 == 0 {
            0x80..0x200
        } else {
            0..file.len()
        };
        for _ in 0..1 + rnd() % 8 {
            let at = region.start + (rnd() as usize % region.len());
            file[at] = rnd() as u8;
        }
        let Ok(img) = Image::parse(&file) else {
            continue;
        };
        for p in 0..img.pages().min(64) {
            let _ = img.fill_page(p, &mut page, reader(&file));
            let _ = img.page_protection(p);
        }
        let _ = for_each_import(&img, reader(&file), |_, _, _| {});
        if let Ok(Some(e)) = Exports::read(&img, reader(&file)) {
            let _ = e.by_ordinal(&img, 1, reader(&file));
            let _ = e.by_name(&img, b"x", 0, reader(&file));
        }
        // Relocation blocks straight from random bytes, too.
        let raw: Vec<u8> = (0..(rnd() % 64)).map(|_| rnd() as u8).collect();
        if let Ok(r) = Relocations::new(&raw) {
            r.apply(0, &mut page, [0; 4], [0; 4], 0x1234_0000);
        }
    }
}
