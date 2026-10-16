//! Application Manager's job, up to the point where the device takes over:
//! read the INI, check every CAB it lists, choose the one for the device,
//! and stage it with its install plan in an outbox folder. Until the device
//! link exists (docs/legacysync/device-link.md) the outbox is where it ends.
//!
//! Every CAB is checked whole before it is chosen: every data block's
//! checksum, the setup script parsed end to end, and every file the script
//! places present in the CAB. A CAB that fails is reported and skipped.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component as PathPart, Path, PathBuf};

use rp_cab::{Cabinet, Scratch, Source};
use rp_ce_setup::xml::Setup;
use rp_ce_setup::{CePath, Item, MAX_SCRIPT, RegData, Script, member_id, pocket_pc_folder};

use crate::ini::{self, Component};
use crate::select::{self, Device, Fit, Requirements};

/// A file read through [`Source`].
pub struct FileSource {
    file: File,
    len: u64,
}

impl FileSource {
    /// Opens `path`.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        Ok(FileSource { file, len })
    }
}

impl Source for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), rp_cab::Error> {
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|_| rp_cab::Error::Io)?;
        self.file.read_exact(buf).map_err(|_| rp_cab::Error::Io)
    }
}

/// One CAB, checked.
#[derive(Debug)]
pub struct Checked {
    /// Where it is.
    pub path: PathBuf,
    /// The script's requirements.
    pub requirements: Requirements,
    /// The app and provider names (from the script, or the XML's AppName).
    pub app: String,
    /// The provider (`.000` scripts only).
    pub provider: String,
    /// The install plan, as text lines.
    pub plan: Vec<String>,
    /// Its setup DLL's member name, if it has one (runs on the device).
    pub setup_dll: Option<String>,
}

/// Bytes as text (scripts are in the PC's ANSI code page; the real ones
/// are ASCII).
fn text(b: &[u8]) -> String {
    match std::str::from_utf8(b) {
        Ok(s) => s.to_owned(),
        Err(_) => b.iter().map(|&c| char::from(c)).collect(),
    }
}

/// A path with its leading `%CEn%` shown expanded, for people reading the
/// plan (the device does the real expansion).
fn shown(p: &CePath) -> String {
    let t = text(p.as_bytes());
    if let Some(rest) = t.strip_prefix("%CE")
        && let Some((n, tail)) = rest.split_once('%')
        && let Some(folder) = n.parse().ok().and_then(pocket_pc_folder)
    {
        return format!("{t}  ({folder}{tail})");
    }
    t
}

fn describe(item: &Item<'_>, members: &[(u16, String)]) -> String {
    match item {
        Item::Directory { path } => format!("make folder   {}", shown(path)),
        Item::File { id, dest, flags } => {
            let member = members
                .iter()
                .find(|(m, _)| m == id)
                .map_or("?", |(_, n)| n.as_str());
            let shared = if flags & 0x4000_0000 != 0 {
                ", shared"
            } else {
                ""
            };
            let reg = if flags & 0x8000_0000 != 0 {
                ", self-registers"
            } else {
                ""
            };
            format!("place file    {}  <- {member}{shared}{reg}", shown(dest))
        }
        Item::RegValue {
            root,
            key,
            name,
            data,
            flags,
            ..
        } => {
            let name = if name.is_empty() {
                "(default)".to_owned()
            } else {
                text(name)
            };
            let data = match data {
                RegData::String(s) => format!("\"{}\"", text(s)),
                RegData::MultiString(s) => format!("[{}]", text(s).replace('\0', ", ")),
                RegData::Dword(v) => format!("{v} (0x{v:x})"),
                RegData::Binary(b) => format!("{} bytes", b.len()),
            };
            let keep = if flags & rp_ce_setup::REG_NO_CLOBBER != 0 {
                "  (keeps an existing value)"
            } else {
                ""
            };
            format!(
                "set registry  {}\\{} : {name} = {data}{keep}",
                root.short(),
                text(key.as_bytes())
            )
        }
        Item::Shortcut { path, target } => format!(
            "make shortcut {}  -> {}",
            shown(path),
            text(target.as_bytes())
        ),
    }
}

/// Checks one CAB end to end.
pub fn check_cab(path: &Path, scratch: &mut Scratch) -> Result<Checked, String> {
    let mut src = FileSource::open(path).map_err(|e| format!("can't open: {e}"))?;
    let cab = Cabinet::open(&mut src).map_err(|e| format!("not a usable cabinet: {}", e.name()))?;
    let mut members = Vec::new();
    let mut entries = Vec::new();
    for f in cab.files(&mut src) {
        let f = f.map_err(|e| format!("bad file list: {}", e.name()))?;
        let name = text(f.name());
        if let Some(id) = member_id(f.name()) {
            members.push((id, name.clone()));
        }
        entries.push(f);
    }
    // Every folder whole: checksums and decompression.
    for i in 0..cab.folder_count() {
        cab.extract_folder(&mut src, i, scratch, |_, _| Ok(rp_cab::Flow::Continue))
            .map_err(|e| format!("folder {i}: {}", e.name()))?;
    }
    let script_entry = entries
        .iter()
        .find(|f| member_id(f.name()) == Some(0))
        .or_else(|| {
            entries
                .iter()
                .find(|f| f.name().eq_ignore_ascii_case(b"_setup.xml"))
        })
        .ok_or("no setup script (.000 or _setup.xml) inside")?;
    if script_entry.size as usize > MAX_SCRIPT {
        return Err("setup script too large".to_owned());
    }
    let mut script = Vec::with_capacity(script_entry.size as usize);
    cab.extract_file(&mut src, script_entry, scratch, |_, b| {
        script.extend_from_slice(b);
        Ok(())
    })
    .map_err(|e| format!("setup script: {}", e.name()))?;

    let mut plan = Vec::new();
    let mut missing = None;
    let mut visit = |item: Item<'_>| {
        if let Item::File { id, .. } = item
            && !members.iter().any(|(m, _)| *m == id)
        {
            missing = Some(id);
        }
        plan.push(describe(&item, &members));
        Ok(())
    };
    let (requirements, app, provider) = if member_id(script_entry.name()) == Some(0) {
        let s = Script::parse(&script).map_err(|e| format!("setup script: {}", e.name()))?;
        s.plan(&mut visit)
            .map_err(|e| format!("setup script: {}", e.name()))?;
        let req = Requirements {
            processor: s.processor,
            os_min: s.os_min,
            os_max: s.os_max,
            unsupported: s.unsupported_platforms().map(text).collect(),
        };
        (req, text(s.app_name()), text(s.provider()))
    } else {
        let s = Setup::parse(&script).map_err(|e| format!("_setup.xml: {}", e.name()))?;
        s.plan(&mut visit)
            .map_err(|e| format!("_setup.xml: {}", e.name()))?;
        // The XML form carries no processor or version range here.
        let req = Requirements {
            processor: 0,
            os_min: (0, 0),
            os_max: (0, 0),
            unsupported: Vec::new(),
        };
        (
            req,
            s.app_name().map(text).unwrap_or_default(),
            String::new(),
        )
    };
    if let Some(id) = missing {
        return Err(format!(
            "setup script places file {id}, which isn't in the CAB"
        ));
    }
    let setup_dll = members
        .iter()
        .find(|(id, _)| *id == 999)
        .map(|(_, n)| n.clone());
    Ok(Checked {
        path: path.to_owned(),
        requirements,
        app,
        provider,
        plan,
        setup_dll,
    })
}

/// A CAB name from an INI as a path under the INI's folder: relative, no
/// `..`, no drive or root.
///
/// The name is split as Windows splits it (`\` and `/`) on every system:
/// INIs are written for Windows, and a host that splits only on `/` would
/// take `..\a.cab` as one plain name.
fn cab_path(ini_dir: &Path, name: &str) -> Result<PathBuf, String> {
    let parts: Vec<&str> = name.split(['\\', '/']).collect();
    if name.contains(':')
        || parts
            .iter()
            .any(|p| p.is_empty() || *p == "." || *p == "..")
    {
        return Err(format!(
            "CAB name {name:?} must be a plain name beside the INI"
        ));
    }
    Ok(parts
        .iter()
        .fold(ini_dir.to_path_buf(), |dir, p| dir.join(p)))
}

/// A folder name for an app: letters, digits and ` ._-`, never empty or a
/// reserved device name.
#[must_use]
pub fn folder_name(app: &str) -> String {
    let mut s: String = app
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || " ._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    s = s.trim_matches(|c| c == ' ' || c == '.').to_owned();
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4);
    if s.is_empty() {
        return "app".to_owned();
    }
    if reserved {
        s.insert_str(0, "app ");
    }
    s
}

/// The result of staging one component.
#[derive(Debug)]
pub struct Staged {
    /// The component's name in the INI.
    pub component: String,
    /// The app as the script names it.
    pub app: String,
    /// The folder it was staged in.
    pub folder: PathBuf,
    /// The CAB chosen.
    pub cab: PathBuf,
    /// Warnings shown to the user (e.g. an OS version outside the range).
    pub warnings: Vec<String>,
}

fn fit_text(f: &Fit) -> String {
    let cpu = match f.cpu {
        select::CpuFit::Arm => "ARM",
        select::CpuFit::OlderArm => "older ARM",
        select::CpuFit::Any => "any processor",
        select::CpuFit::Other => "another processor (won't run)",
    };
    let mut s = cpu.to_owned();
    if !f.version_ok {
        s.push_str(", OS version outside its range");
    }
    if !f.platform_ok {
        s.push_str(", refuses Pocket PC");
    }
    s
}

/// Stages every component of the INI at `ini_path` under `outbox`.
pub fn stage(ini_path: &Path, outbox: &Path, device: &Device) -> Result<Vec<Staged>, String> {
    let bytes = std::fs::read(ini_path).map_err(|e| format!("{}: {e}", ini_path.display()))?;
    let components = ini::components(&bytes)?;
    let ini_dir = ini_path.parent().unwrap_or(Path::new("."));
    let mut scratch = Box::new(Scratch::new());
    let mut out = Vec::new();
    for comp in &components {
        out.push(stage_component(
            ini_path,
            ini_dir,
            comp,
            outbox,
            device,
            &mut scratch,
        )?);
    }
    Ok(out)
}

fn stage_component(
    ini_path: &Path,
    ini_dir: &Path,
    comp: &Component,
    outbox: &Path,
    device: &Device,
    scratch: &mut Scratch,
) -> Result<Staged, String> {
    let mut checked = Vec::new();
    let mut report = Vec::new();
    for name in &comp.cab_files {
        let r = cab_path(ini_dir, name).and_then(|p| check_cab(&p, scratch));
        match &r {
            Ok(c) => {
                let f = select::assess(&c.requirements, device);
                report.push(format!("  {name}: {}", fit_text(&f)));
                checked.push(Some((f, r.unwrap())));
            }
            Err(e) => {
                report.push(format!("  {name}: refused: {e}"));
                checked.push(None);
            }
        }
    }
    let fits: Vec<Option<Fit>> = checked
        .iter()
        .map(|c| c.as_ref().map(|(f, _)| *f))
        .collect();
    let Some(pick) = select::choose(&fits) else {
        return Err(format!(
            "component {:?}: no CAB for this device\n{}",
            comp.name,
            report.join("\n")
        ));
    };
    let (fit, cab) = checked.swap_remove(pick).expect("chosen CABs were checked");
    let mut warnings = Vec::new();
    if !fit.version_ok {
        let r = &cab.requirements;
        warnings.push(format!(
            "{} says it needs Windows CE {}.{:02}-{}.{:02}; RetroPane reports {}.{:02}. It may still work.",
            cab.app, r.os_min.0, r.os_min.1, r.os_max.0, r.os_max.1, device.os.0, device.os.1
        ));
    }
    let title = if cab.provider.is_empty() {
        cab.app.clone()
    } else {
        format!("{} {}", cab.provider, cab.app)
    };
    let folder = outbox.join(folder_name(&title));
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let cab_name = cab.path.file_name().ok_or("CAB has no file name")?;
    let staged_cab = folder.join(cab_name);
    std::fs::copy(&cab.path, &staged_cab).map_err(|e| format!("copying the CAB: {e}"))?;

    let mut t = String::new();
    let _ = writeln!(t, "LegacySync install plan");
    let _ = writeln!(t, "=======================\n");
    let _ = writeln!(t, "App:        {}", cab.app);
    let _ = writeln!(t, "Provider:   {}", cab.provider);
    let _ = writeln!(
        t,
        "From:       {} (component {:?})",
        ini_path.display(),
        comp.name
    );
    if let Some(d) = &comp.description {
        let _ = writeln!(t, "Described:  {d}");
    }
    if let Some(d) = &comp.device_file {
        let _ = writeln!(t, "Program:    {d}");
    }
    if let Some(u) = &comp.uninstall {
        let _ = writeln!(t, "Removed as: {u}");
    }
    let _ = writeln!(t, "CAB chosen: {}", cab_name.to_string_lossy());
    let _ = writeln!(t, "\nCABs in the INI:\n{}", report.join("\n"));
    for w in &warnings {
        let _ = writeln!(t, "\nNote: {w}");
    }
    if let Some(dll) = &cab.setup_dll {
        let _ = writeln!(
            t,
            "\nSetup DLL: {dll} (ARM code the device runs during install, in the app sandbox)"
        );
    }
    let _ = writeln!(
        t,
        "\nWhat the device will do ({} steps; %CEn% folders shown as on a Pocket PC):",
        cab.plan.len()
    );
    for line in &cab.plan {
        let _ = writeln!(t, "  {line}");
    }
    std::fs::write(folder.join("plan.txt"), t).map_err(|e| format!("writing the plan: {e}"))?;
    Ok(Staged {
        component: comp.name.clone(),
        app: cab.app,
        folder,
        cab: staged_cab,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cab_names_stay_beside_the_ini() {
        let dir = Path::new("C:\\stage");
        assert!(cab_path(dir, "Toy Golf.CAB").is_ok());
        // Split as Windows splits it, on every host (CI runs on Linux).
        assert_eq!(
            cab_path(dir, "cabs\\a.cab"),
            Ok(dir.join("cabs").join("a.cab"))
        );
        for bad in [
            "..\\a.cab",
            "../a.cab",
            "cabs\\..\\..\\a.cab",
            "C:\\Windows\\a.cab",
            "\\a.cab",
            "c:a.cab",
            "a\\..\\..\\b.cab",
            ".",
        ] {
            assert!(cab_path(dir, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn folder_names_are_safe() {
        assert_eq!(folder_name("Fathammer Toy Golf"), "Fathammer Toy Golf");
        assert_eq!(folder_name("a/b\\c:d*e"), "a_b_c_d_e");
        assert_eq!(folder_name("CON"), "app CON");
        assert_eq!(folder_name("com1.txt"), "app com1.txt");
        assert_eq!(folder_name(".."), "app");
        assert_eq!(folder_name(""), "app");
        assert_eq!(folder_name(&"x".repeat(100)).len(), 64);
    }
}
