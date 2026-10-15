//! rp-ce-setup against scripts built here, hostile variants of them, and the
//! owner's real CABs' scripts when present in `local/legacysync/real`
//! (never committed; skipped otherwise).

use rp_ce_setup::xml::Setup;
use rp_ce_setup::{Error, Item, RegData, Root, Script};

/// Builds a `.000` script with the layout in
/// docs/legacysync/activesync-interfaces.md §6.
#[derive(Default, Clone)]
struct Builder {
    processor: u32,
    strings: Vec<(u16, Vec<u8>)>,
    dirs: Vec<(u16, Vec<u16>)>,
    files: Vec<(u16, u16, u32, Vec<u8>)>,
    hives: Vec<(u16, u16, Vec<u16>)>,
    values: Vec<(u16, u16, u16, u32, Vec<u8>)>,
    links: Vec<(u16, u16, u16, u16, Vec<u16>)>,
}

fn ids(list: &[u16], terminated: bool) -> Vec<u8> {
    let mut v: Vec<u8> = list.iter().flat_map(|i| i.to_le_bytes()).collect();
    if terminated {
        v.extend_from_slice(&[0, 0]);
    }
    v
}

fn put16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn put32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn nul(s: &[u8]) -> Vec<u8> {
    let mut v = s.to_vec();
    v.push(0);
    v
}

impl Builder {
    fn build(&self) -> Vec<u8> {
        let mut body = Vec::new();
        let base = 100usize;
        let app = nul(b"Pinball");
        let prov = nul(b"RetroPane Tests");
        let unsup = b"HPC\0JUPITER\0\0".to_vec();
        let app_at = base + body.len();
        body.extend_from_slice(&app);
        let prov_at = base + body.len();
        body.extend_from_slice(&prov);
        let unsup_at = base + body.len();
        body.extend_from_slice(&unsup);
        let mut offsets = [0u32; 6];
        let mut table = |t: usize, entries: Vec<Vec<u8>>, body: &mut Vec<u8>| {
            offsets[t] = (base + body.len()) as u32;
            for e in entries {
                body.extend_from_slice(&e);
            }
        };
        table(
            0,
            self.strings
                .iter()
                .map(|(id, s)| {
                    let s = nul(s);
                    let mut e = Vec::new();
                    put16(&mut e, *id);
                    put16(&mut e, s.len() as u16);
                    e.extend_from_slice(&s);
                    e
                })
                .collect(),
            &mut body,
        );
        table(
            1,
            self.dirs
                .iter()
                .map(|(id, l)| {
                    let l = ids(l, true);
                    let mut e = Vec::new();
                    put16(&mut e, *id);
                    put16(&mut e, l.len() as u16);
                    e.extend_from_slice(&l);
                    e
                })
                .collect(),
            &mut body,
        );
        table(
            2,
            self.files
                .iter()
                .map(|(id, dir, flags, name)| {
                    let n = nul(name);
                    let mut e = Vec::new();
                    put16(&mut e, *id);
                    put16(&mut e, *dir);
                    put16(&mut e, 0);
                    put32(&mut e, *flags);
                    put16(&mut e, n.len() as u16);
                    e.extend_from_slice(&n);
                    e
                })
                .collect(),
            &mut body,
        );
        table(
            3,
            self.hives
                .iter()
                .map(|(id, root, l)| {
                    let l = ids(l, true);
                    let mut e = Vec::new();
                    put16(&mut e, *id);
                    put16(&mut e, *root);
                    put16(&mut e, 0);
                    put16(&mut e, l.len() as u16);
                    e.extend_from_slice(&l);
                    e
                })
                .collect(),
            &mut body,
        );
        table(
            4,
            self.values
                .iter()
                .map(|(id, hive, subst, flags, data)| {
                    let mut e = Vec::new();
                    put16(&mut e, *id);
                    put16(&mut e, *hive);
                    put16(&mut e, *subst);
                    put32(&mut e, *flags);
                    put16(&mut e, data.len() as u16);
                    e.extend_from_slice(data);
                    e
                })
                .collect(),
            &mut body,
        );
        table(
            5,
            self.links
                .iter()
                .map(|(id, base_dir, target, kind, names)| {
                    let n = ids(names, false);
                    let mut e = Vec::new();
                    put16(&mut e, *id);
                    put16(&mut e, *base_dir);
                    put16(&mut e, 0);
                    put16(&mut e, *target);
                    put16(&mut e, *kind);
                    put16(&mut e, n.len() as u16);
                    e.extend_from_slice(&n);
                    e
                })
                .collect(),
            &mut body,
        );
        let total = base + body.len();
        let mut h = Vec::new();
        h.extend_from_slice(b"MSCE");
        put32(&mut h, 0);
        put32(&mut h, total as u32);
        put32(&mut h, 0);
        put32(&mut h, 1);
        put32(&mut h, self.processor);
        for v in [4, 20, 5, 99, 0, 0xE000_0000] {
            put32(&mut h, v);
        }
        for n in [
            self.strings.len(),
            self.dirs.len(),
            self.files.len(),
            self.hives.len(),
            self.values.len(),
            self.links.len(),
        ] {
            put16(&mut h, n as u16);
        }
        for o in offsets {
            put32(&mut h, o);
        }
        for (at, len) in [
            (app_at, app.len()),
            (prov_at, prov.len()),
            (unsup_at, unsup.len()),
        ] {
            put16(&mut h, at as u16);
            put16(&mut h, len as u16);
        }
        put32(&mut h, 0);
        assert_eq!(h.len(), 100);
        h.extend_from_slice(&body);
        h
    }
}

/// A script shaped like the real ones: an install folder, a Start menu
/// shortcut, a setting of every value type.
fn pinball() -> Builder {
    let s = |id, t: &[u8]| (id, t.to_vec());
    let value = |name: &[u8], data: &[u8]| {
        let mut v = nul(name);
        v.extend_from_slice(data);
        v
    };
    Builder {
        processor: 2577,
        strings: vec![
            s(1, b"%CE1%\\RetroPane"),
            s(2, b"Pinball"),
            s(3, b"%CE11%"),
            s(4, b"Software"),
            s(5, b"Pinball.lnk"),
        ],
        dirs: vec![(7, vec![1, 2]), (8, vec![3])],
        files: vec![
            (1, 7, 0x4000_0002, b"Pinball.exe".to_vec()),
            (2, 7, 0, b"Table 1.dat".to_vec()),
        ],
        hives: vec![(1, 3, vec![4, 2])],
        values: vec![
            (1, 1, 1, 0x0000_0000, value(b"Path", b"%InstallDir%\0")),
            (2, 1, 0, 0x0001_0001, value(b"Level", &3u32.to_le_bytes())),
            (3, 1, 0, 0x0001_0000, value(b"Tables", b"one\0two\0\0")),
            (4, 1, 0, 0x0000_0001, value(b"", &[1, 2, 3])),
            (5, 1, 0, 0x0000_0002, value(b"Keep", b"x\0")),
        ],
        links: vec![(1, 8, 1, 1, vec![5])],
    }
}

/// The plan as text lines, for comparisons.
fn lines(script: &Script<'_>) -> Result<Vec<String>, Error> {
    let mut out = Vec::new();
    script.plan(|item| {
        out.push(describe(&item));
        Ok(())
    })?;
    Ok(out)
}

fn describe(item: &Item<'_>) -> String {
    let s = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    match item {
        Item::Directory { path } => format!("dir {}", s(path.as_bytes())),
        Item::File { id, dest, flags } => {
            format!("file {id} -> {} ({flags:#x})", s(dest.as_bytes()))
        }
        Item::RegValue {
            root,
            key,
            name,
            data,
            flags,
            expand,
        } => {
            let d = match data {
                RegData::String(v) => format!("sz {}", s(v)),
                RegData::MultiString(v) => format!("multi {}", s(v).replace('\0', "|")),
                RegData::Dword(v) => format!("dword {v}"),
                RegData::Binary(v) => format!("bin {v:?}"),
            };
            format!(
                "reg {}\\{} [{}] = {d} ({flags:#x}, expand {expand})",
                root.short(),
                s(key.as_bytes()),
                s(name)
            )
        }
        Item::Shortcut { path, target } => {
            format!("link {} -> {}", s(path.as_bytes()), s(target.as_bytes()))
        }
    }
}

#[test]
fn a_whole_script_becomes_a_plan() {
    let bytes = pinball().build();
    let script = Script::parse(&bytes).unwrap();
    assert_eq!(script.processor, 2577);
    assert_eq!(script.os_min, (4, 20));
    assert_eq!(script.app_name(), b"Pinball");
    assert_eq!(script.provider(), b"RetroPane Tests");
    assert_eq!(
        script.unsupported_platforms().collect::<Vec<_>>(),
        [&b"HPC"[..], b"JUPITER"]
    );
    assert_eq!(
        lines(&script).unwrap(),
        [
            "dir %CE1%\\RetroPane\\Pinball",
            "dir %CE11%",
            "file 1 -> %CE1%\\RetroPane\\Pinball\\Pinball.exe (0x40000002)",
            "file 2 -> %CE1%\\RetroPane\\Pinball\\Table 1.dat (0x0)",
            "reg HKLM\\Software\\Pinball [Path] = sz %InstallDir% (0x0, expand true)",
            "reg HKLM\\Software\\Pinball [Level] = dword 3 (0x10001, expand false)",
            "reg HKLM\\Software\\Pinball [Tables] = multi one|two (0x10000, expand false)",
            "reg HKLM\\Software\\Pinball [] = bin [1, 2, 3] (0x1, expand false)",
            "reg HKLM\\Software\\Pinball [Keep] = sz x (0x2, expand false)",
            "link %CE11%\\Pinball.lnk -> %CE1%\\RetroPane\\Pinball\\Pinball.exe",
        ]
    );
}

#[test]
fn bad_references_and_paths_are_refused_up_front() {
    let mut b = pinball();
    b.files[1].1 = 99; // no such directory
    assert_eq!(Script::parse(&b.build()).unwrap_err(), Error::BadReference);
    let mut b = pinball();
    b.dirs[0].1 = vec![1, 6]; // no string 6
    assert_eq!(Script::parse(&b.build()).unwrap_err(), Error::BadReference);
    let mut b = pinball();
    b.strings[1].1 = b"..".to_vec();
    assert_eq!(Script::parse(&b.build()).unwrap_err(), Error::BadPath);
    let mut b = pinball();
    b.files[0].3 = b"..\\..\\Windows\\evil.dll".to_vec();
    assert_eq!(Script::parse(&b.build()).unwrap_err(), Error::BadPath);
    let mut b = pinball();
    b.values[1].4 = nul(b"Level").into_iter().chain([1, 2]).collect(); // DWORD of 2 bytes
    assert_eq!(Script::parse(&b.build()).unwrap_err(), Error::BadValue);
    let mut b = pinball();
    b.hives[0].1 = 9; // no such root
    assert_eq!(Script::parse(&b.build()).unwrap_err(), Error::BadValue);
    let mut b = pinball();
    b.links[0].3 = 7; // a shortcut kind that isn't file or directory
    assert_eq!(Script::parse(&b.build()).unwrap_err(), Error::BadReference);
    let mut b = pinball();
    b.strings[4].1 = vec![b'x'; 300]; // shortcut path past MAX_PATH
    assert_eq!(Script::parse(&b.build()).unwrap_err(), Error::BadPath);
}

#[test]
fn headers_counts_and_sizes_are_checked() {
    let good = pinball().build();
    assert_eq!(Script::parse(b"MSCF").unwrap_err(), Error::NotScript);
    let mut bad = good.clone();
    bad[8..12].copy_from_slice(&(good.len() as u32 + 1).to_le_bytes());
    assert_eq!(Script::parse(&bad).unwrap_err(), Error::Truncated);
    let mut bad = good.clone();
    bad[52..54].copy_from_slice(&5000u16.to_le_bytes()); // files count
    assert_eq!(Script::parse(&bad).unwrap_err(), Error::TooLarge);
    let mut bad = good.clone();
    bad[52..54].copy_from_slice(&40u16.to_le_bytes()); // more files than stored
    assert!(Script::parse(&bad).is_err());
    assert_eq!(
        Script::parse(&vec![0u8; 70_000]).unwrap_err(),
        Error::TooLarge
    );
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

/// Parses, and if valid walks the plan again: neither may panic, and a
/// parsed script's plan must not fail.
fn survive(bytes: &[u8]) -> bool {
    match Script::parse(bytes) {
        Ok(s) => {
            assert!(lines(&s).is_ok(), "a parsed script's plan failed");
            true
        }
        Err(_) => false,
    }
}

fn mutate_all(bases: &[Vec<u8>], cases: usize) -> usize {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut refused = 0;
    for case in 0..cases {
        let mut v = bases[case % bases.len()].clone();
        for _ in 0..1 + rng.next() % 6 {
            let at = (rng.next() as usize) % v.len();
            v[at] = match rng.next() % 4 {
                0 => 0,
                1 => 0xFF,
                _ => rng.next() as u8,
            };
        }
        if !survive(&v) {
            refused += 1;
        }
    }
    refused
}

#[test]
fn hostile_scripts_never_panic() {
    let good = pinball().build();
    for n in 0..good.len() {
        assert!(!survive(&good[..n]) || n >= 100, "length {n}");
    }
    let refused = mutate_all(&[good], 20_000);
    assert!(refused > 5_000, "only {refused} refused");
}

/// The `.000` script of every real CAB in local/legacysync/real.
fn real_scripts() -> Vec<(String, Vec<u8>)> {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../local/legacysync/real");
    let mut out = Vec::new();
    let Ok(dirs) = std::fs::read_dir(&root) else {
        return out;
    };
    let mut scratch = Box::new(rp_cab::Scratch::new());
    for dir in dirs.flatten() {
        for entry in std::fs::read_dir(dir.path()).unwrap().flatten() {
            let path = entry.path();
            if !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("cab"))
            {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            let mut src = &bytes[..];
            let cab = rp_cab::Cabinet::open(&mut src).unwrap();
            let files: Vec<_> = cab.files(&mut src).collect::<Result<_, _>>().unwrap();
            let f = files
                .iter()
                .find(|f| rp_ce_setup::member_id(f.name()) == Some(0))
                .unwrap();
            let mut script = Vec::new();
            cab.extract_file(&mut src, f, &mut scratch, |_, b| {
                script.extend_from_slice(b);
                Ok(())
            })
            .unwrap();
            out.push((
                path.file_name().unwrap().to_string_lossy().into_owned(),
                script,
            ));
        }
    }
    out
}

#[test]
fn the_owners_real_scripts_parse_whole() {
    let scripts = real_scripts();
    if scripts.is_empty() {
        eprintln!("skipped: no real CABs");
        return;
    }
    for (name, bytes) in &scripts {
        let s = Script::parse(bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let plan = lines(&s).unwrap();
        let [_, dirs, files, _, values, links] = s.counts();
        let count = |p: &str| plan.iter().filter(|l| l.starts_with(p)).count();
        assert_eq!(count("dir "), usize::from(dirs), "{name}");
        assert_eq!(count("file "), usize::from(files), "{name}");
        assert_eq!(count("reg "), usize::from(values), "{name}");
        assert_eq!(count("link "), usize::from(links), "{name}");
        eprintln!(
            "{name}: {} {} items",
            String::from_utf8_lossy(s.app_name()),
            plan.len()
        );
    }
    // Mutations of real scripts too.
    let bases: Vec<Vec<u8>> = scripts.into_iter().map(|(_, b)| b).collect();
    mutate_all(&bases, 10_000);
}

const SETUP_XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!-- made by hand from the public provisioning documentation -->
<wap-provisioningdoc>
  <characteristic type="Install">
    <parm name="InstallPhase" value="install" />
    <parm name="AppName" value="RetroPane Pinball" />
    <parm name="InstallDir" value="%CE1%\Pinball" translation="install" />
  </characteristic>
  <characteristic type="FileOperation">
    <characteristic type="%InstallDir%" translation="install">
      <characteristic type="MakeDir" />
      <characteristic type="Pinball.exe" translation="install">
        <characteristic type="Extract">
          <parm name="Source" value="PINBAL~1.001" />
        </characteristic>
      </characteristic>
      <characteristic type="Tom &amp; Jerry.dat" translation="install">
        <characteristic type="Extract"><parm name='Source' value="TOMAND~1.002"></parm></characteristic>
      </characteristic>
    </characteristic>
    <characteristic type="%CE11%" translation="install">
      <characteristic type="Pinball.lnk" translation="install">
        <characteristic type="Shortcut">
          <parm name="Source" value="%InstallDir%\Pinball.exe" />
        </characteristic>
      </characteristic>
    </characteristic>
  </characteristic>
  <characteristic type="Registry">
    <characteristic type="HKLM\Software\Pinball">
      <parm name="Level" value="0x10" datatype="integer" />
      <parm name="Name" value="Ball &#x41;" datatype="string" />
      <characteristic type="Tables">
        <parm name="Count" value="2" datatype="integer" />
      </characteristic>
    </characteristic>
  </characteristic>
</wap-provisioningdoc>
"#;

fn xml_lines(doc: &[u8]) -> Result<Vec<String>, Error> {
    let setup = Setup::parse(doc)?;
    let mut out = Vec::new();
    setup.plan(|item| {
        out.push(describe(&item));
        Ok(())
    })?;
    Ok(out)
}

#[test]
fn setup_xml_becomes_the_same_plan_items() {
    let setup = Setup::parse(SETUP_XML.as_bytes()).unwrap();
    assert_eq!(setup.app_name(), Some(&b"RetroPane Pinball"[..]));
    assert_eq!(setup.install_dir(), Some(&b"%CE1%\\Pinball"[..]));
    assert_eq!(
        xml_lines(SETUP_XML.as_bytes()).unwrap(),
        [
            "dir %InstallDir%",
            "file 1 -> %InstallDir%\\Pinball.exe (0x0)",
            "file 2 -> %InstallDir%\\Tom & Jerry.dat (0x0)",
            "link %CE11%\\Pinball.lnk -> %InstallDir%\\Pinball.exe",
            "reg HKLM\\Software\\Pinball [Level] = dword 16 (0x0, expand true)",
            "reg HKLM\\Software\\Pinball [Name] = sz Ball A (0x0, expand true)",
            "reg HKLM\\Software\\Pinball\\Tables [Count] = dword 2 (0x0, expand true)",
        ]
    );
}

#[test]
fn setup_xml_outside_the_subset_is_refused() {
    let swap = |from: &str, to: &str| {
        assert!(SETUP_XML.contains(from), "{from}");
        SETUP_XML.replacen(from, to, 1)
    };
    let cases = [
        // Entity declarations: refused outright (no expansion attacks).
        swap(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>",
            "<!DOCTYPE x [<!ENTITY a \"aaaa\">]>",
        ),
        swap(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>",
            "<!DOCTYPE wap-provisioningdoc>",
        ),
        swap("value=\"install\" />", "value=\"&a;\" />"),
        swap("</wap-provisioningdoc>", "</wap-provisioningdocx>"),
        swap("</wap-provisioningdoc>", ""),
        swap(
            "<characteristic type=\"MakeDir\" />",
            "<characteristic type=\"RemoveDir\" />",
        ),
        swap("type=\"Extract\">", "type=\"Delete\">"),
        swap("value=\"PINBAL~1.001\"", "value=\"PINBALL.EXE\""),
        swap("type=\"Pinball.exe\"", "type=\"..\\..\\evil.exe\""),
        swap("type=\"Pinball.exe\"", "type=\"sub\\evil.exe\""),
        swap(
            "type=\"HKLM\\Software\\Pinball\"",
            "type=\"HKXX\\Software\\Pinball\"",
        ),
        swap("datatype=\"string\"", "datatype=\"binary\""),
        swap("value=\"0x10\"", "value=\"0x1FFFFFFFF\""),
        swap("type=\"Registry\"", "type=\"Registry\" type=\"Install\""),
        swap(
            "<parm name=\"InstallPhase\" value=\"install\" />",
            "<parm name=\"x\" value=\"y\">text</parm>",
        ),
        swap("<wap-provisioningdoc>", "<wap-provisioningdoc>stray text"),
        swap("type=\"Install\"", "type=\"Install"),
        swap(
            "<characteristic type=\"Install\">",
            "<characteristic type=\"Install\"><![CDATA[x]]>",
        ),
    ];
    for (i, doc) in cases.iter().enumerate() {
        assert!(xml_lines(doc.as_bytes()).is_err(), "case {i} was accepted");
    }
    // Nesting past MAX_DEPTH.
    let deep = format!(
        "<wap-provisioningdoc><characteristic type=\"Registry\">{}{}</characteristic></wap-provisioningdoc>",
        "<characteristic type=\"HKLM\\k\">".repeat(10),
        "</characteristic>".repeat(10)
    );
    assert!(xml_lines(deep.as_bytes()).is_err());
    assert!(
        Setup::parse(&[0xFF, 0xFE, b'<', 0]).is_err(),
        "UTF-16 refused"
    );
}

#[test]
fn hostile_setup_xml_never_panics() {
    let base = SETUP_XML.as_bytes();
    for n in 0..base.len() {
        let _ = xml_lines(&base[..n]);
    }
    let mut rng = Rng(7);
    let mut refused = 0;
    for _ in 0..20_000 {
        let mut v = base.to_vec();
        for _ in 0..1 + rng.next() % 4 {
            let at = (rng.next() as usize) % v.len();
            v[at] = b"<>/=\"'&;#x \\%."[(rng.next() % 14) as usize];
        }
        if xml_lines(&v).is_err() {
            refused += 1;
        }
    }
    assert!(refused > 10_000, "only {refused} refused");
}

#[test]
fn roots_are_named() {
    assert_eq!(Root::LocalMachine.short(), "HKLM");
}
