//! The CeAppMgr stand-in end to end: synthetic installers built here, and
//! the owner's real ones from `local/legacysync/real` when present.

use std::path::{Path, PathBuf};

use rp_legacysync::{select, stage};

/// A fresh, empty folder under the target directory.
fn temp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn checksum(data: &[u8], seed: u32) -> u32 {
    rp_cab::checksum(data, seed)
}

/// A one-folder uncompressed cabinet holding `members`.
fn cabinet(members: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let stream: Vec<u8> = members.iter().flat_map(|(_, d)| d.clone()).collect();
    let mut files = Vec::new();
    let mut off = 0u32;
    for (name, data) in members {
        files.extend_from_slice(&(data.len() as u32).to_le_bytes());
        files.extend_from_slice(&off.to_le_bytes());
        files.extend_from_slice(&[0, 0, 0x21, 0x50, 0, 0x60, 0x20, 0]);
        files.extend_from_slice(name.as_bytes());
        files.push(0);
        off += data.len() as u32;
    }
    let blocks: Vec<&[u8]> = stream.chunks(4096).collect();
    let data_at = 36 + 8 + files.len();
    let mut data = Vec::new();
    for b in &blocks {
        let lens = [
            (b.len() as u16).to_le_bytes(),
            (b.len() as u16).to_le_bytes(),
        ]
        .concat();
        data.extend_from_slice(&checksum(&lens, checksum(b, 0)).to_le_bytes());
        data.extend_from_slice(&lens);
        data.extend_from_slice(b);
    }
    let total = data_at + data.len();
    let mut c = Vec::new();
    c.extend_from_slice(b"MSCF\0\0\0\0");
    c.extend_from_slice(&(total as u32).to_le_bytes());
    c.extend_from_slice(&[0; 4]);
    c.extend_from_slice(&44u32.to_le_bytes());
    c.extend_from_slice(&[0; 4]);
    c.extend_from_slice(&[3, 1]);
    c.extend_from_slice(&1u16.to_le_bytes());
    c.extend_from_slice(&(members.len() as u16).to_le_bytes());
    c.extend_from_slice(&[0, 0, 0x34, 0x12, 0, 0]);
    c.extend_from_slice(&(data_at as u32).to_le_bytes());
    c.extend_from_slice(&(blocks.len() as u16).to_le_bytes());
    c.extend_from_slice(&0u16.to_le_bytes());
    c.extend_from_slice(&files);
    c.extend_from_slice(&data);
    assert_eq!(c.len(), total);
    c
}

/// A `.000` script: one folder, one file, for `processor` and CE 4.20-5.99.
fn script(processor: u32) -> Vec<u8> {
    let mut body = Vec::new();
    let strings_at = 100 + body.len();
    body.extend_from_slice(&[1, 0, 15, 0]);
    body.extend_from_slice(b"%CE1%\\Pinball\0");
    body.push(0);
    let dirs_at = 100 + body.len();
    body.extend_from_slice(&[1, 0, 4, 0, 1, 0, 0, 0]);
    let files_at = 100 + body.len();
    body.extend_from_slice(&[1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 12, 0]);
    body.extend_from_slice(b"Pinball.exe\0");
    let app_at = 100 + body.len();
    body.extend_from_slice(b"Pinball\0");
    let mut h = Vec::new();
    h.extend_from_slice(b"MSCE\0\0\0\0");
    h.extend_from_slice(&((100 + body.len()) as u32).to_le_bytes());
    h.extend_from_slice(&[0, 0, 0, 0, 1, 0, 0, 0]);
    for v in [processor, 4, 20, 5, 99, 0, 0xE000_0000] {
        h.extend_from_slice(&v.to_le_bytes());
    }
    for n in [1u16, 1, 1, 0, 0, 0] {
        h.extend_from_slice(&n.to_le_bytes());
    }
    for o in [strings_at, dirs_at, files_at, 0, 0, 0] {
        h.extend_from_slice(&(o as u32).to_le_bytes());
    }
    for (at, len) in [(app_at, 8usize), (0, 0), (0, 0)] {
        h.extend_from_slice(&(at as u16).to_le_bytes());
        h.extend_from_slice(&(len as u16).to_le_bytes());
    }
    h.extend_from_slice(&[0; 4]);
    h.extend_from_slice(&body);
    h
}

#[test]
fn the_arm_cab_is_chosen_and_a_broken_one_reported() {
    let dir = temp("synthetic");
    let exe = b"MZ pretend program".to_vec();
    std::fs::write(
        dir.join("Pinball.MIPS.CAB"),
        cabinet(&[("PIN~1.000", script(4000)), ("Pinball.001", exe.clone())]),
    )
    .unwrap();
    let mut broken = cabinet(&[("PIN~1.000", script(2577)), ("Pinball.001", exe.clone())]);
    let last = broken.len() - 1;
    broken[last] ^= 1; // fails its checksum
    std::fs::write(dir.join("Pinball.SA.CAB"), broken).unwrap();
    std::fs::write(
        dir.join("Pinball.ARM.CAB"),
        cabinet(&[("PIN~1.000", script(2577)), ("Pinball.001", exe)]),
    )
    .unwrap();
    std::fs::write(
        dir.join("Pinball.ini"),
        "[CEAppManager]\r\nVersion = 1.0\r\nComponent = Pinball\r\n\r\n[Pinball]\r\nDescription = Pinball\r\nCabFiles = Pinball.MIPS.CAB,Pinball.SA.CAB,Pinball.ARM.CAB\r\n",
    )
    .unwrap();
    let outbox = dir.join("outbox");
    let staged = stage::stage(&dir.join("Pinball.ini"), &outbox, &select::X51V).unwrap();
    assert_eq!(staged.len(), 1);
    assert!(staged[0].cab.ends_with("Pinball.ARM.CAB"));
    assert_eq!(
        std::fs::read(&staged[0].cab).unwrap(),
        std::fs::read(dir.join("Pinball.ARM.CAB")).unwrap()
    );
    let plan = std::fs::read_to_string(staged[0].folder.join("plan.txt")).unwrap();
    assert!(
        plan.contains("Pinball.MIPS.CAB: another processor (won't run)"),
        "{plan}"
    );
    assert!(
        plan.contains("Pinball.SA.CAB: refused: folder 0: checksum mismatch"),
        "{plan}"
    );
    assert!(plan.contains("place file    %CE1%\\Pinball\\Pinball.exe  (\\Program Files\\Pinball\\Pinball.exe)  <- Pinball.001"), "{plan}");
}

#[test]
fn a_script_naming_a_file_the_cab_lacks_is_refused() {
    let dir = temp("missing");
    std::fs::write(dir.join("a.cab"), cabinet(&[("PIN~1.000", script(2577))])).unwrap();
    std::fs::write(
        dir.join("a.ini"),
        "[CEAppManager]\nVersion=1.0\nComponent=A\n[A]\nCabFiles=a.cab\n",
    )
    .unwrap();
    let err = stage::stage(&dir.join("a.ini"), &dir.join("outbox"), &select::X51V).unwrap_err();
    assert!(
        err.contains("places file 1, which isn't in the CAB"),
        "{err}"
    );
    assert!(!dir.join("outbox").exists(), "nothing staged");
}

#[test]
fn cab_names_escaping_the_ini_folder_are_refused() {
    let dir = temp("escape");
    std::fs::write(
        dir.join("a.ini"),
        "[CEAppManager]\nVersion=1.0\nComponent=A\n[A]\nCabFiles=..\\a.cab\n",
    )
    .unwrap();
    let err = stage::stage(&dir.join("a.ini"), &dir.join("outbox"), &select::X51V).unwrap_err();
    assert!(err.contains("must be a plain name beside the INI"), "{err}");
}

#[test]
fn the_owners_real_installers_stage() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../local/legacysync/real");
    let Ok(dirs) = std::fs::read_dir(&root) else {
        eprintln!("skipped: no real installers");
        return;
    };
    let outbox = temp("real");
    let mut n = 0;
    for d in dirs.flatten() {
        for e in std::fs::read_dir(d.path()).unwrap().flatten() {
            let p = e.path();
            if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ini")) {
                continue;
            }
            let staged = stage::stage(&p, &outbox, &select::X51V)
                .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            for s in staged {
                eprintln!(
                    "{} -> {} ({} warnings)",
                    p.display(),
                    s.folder.display(),
                    s.warnings.len()
                );
                if s.app == "GeoRally" {
                    assert_eq!(
                        s.warnings.len(),
                        1,
                        "GeoRally's 4.0-4.99 range excludes CE 5.1"
                    );
                }
                n += 1;
            }
        }
    }
    eprintln!("{n} real installers staged");
}
