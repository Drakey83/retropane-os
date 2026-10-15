//! Application Manager INI files (docs/legacysync/activesync-interfaces.md
//! §3).
//!
//! Outside input: an installer wrote it. Size-capped, any encoding an old
//! installer might use (ANSI, UTF-8 with or without a mark, UTF-16 with a
//! mark), malformed lines ignored the way Windows' own INI reader ignores
//! them, and every value we use checked where it's used.

/// Largest INI accepted (real ones: 131-262 bytes).
pub const MAX_INI: usize = 64 * 1024;

/// A parsed INI: sections of `key = value` pairs, in file order.
#[derive(Debug, Default)]
pub struct Ini {
    sections: Vec<(String, Vec<(String, String)>)>,
}

impl Ini {
    /// Parses `bytes`.
    pub fn parse(bytes: &[u8]) -> Result<Ini, String> {
        if bytes.len() > MAX_INI {
            return Err(format!("INI larger than {MAX_INI} bytes"));
        }
        let text = decode(bytes)?;
        let mut ini = Ini::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                ini.sections.push((name.trim().to_owned(), Vec::new()));
            } else if let Some((k, v)) = line.split_once('=')
                && let Some((_, pairs)) = ini.sections.last_mut()
            {
                pairs.push((k.trim().to_owned(), v.trim().to_owned()));
            }
        }
        Ok(ini)
    }

    /// The first value of `key` in `section`, both matched without case.
    #[must_use]
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .iter()
            .filter(|(s, _)| s.eq_ignore_ascii_case(section))
            .flat_map(|(_, pairs)| pairs)
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

/// Text of an INI in whatever encoding an installer used. ANSI is read as
/// Windows-1252's Latin-1 subset (the real INIs are ASCII).
fn decode(bytes: &[u8]) -> Result<String, String> {
    if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        return String::from_utf16(&units).map_err(|_| "INI: bad UTF-16".to_owned());
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    Ok(match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    })
}

/// One component an INI asks to install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    /// Its section name.
    pub name: String,
    /// `Description`.
    pub description: Option<String>,
    /// `CabFiles`, as written (relative to the INI's folder).
    pub cab_files: Vec<String>,
    /// `Uninstall`: the name it is removed by.
    pub uninstall: Option<String>,
    /// `DeviceFile`: the main program on the device.
    pub device_file: Option<String>,
    /// `IconFile` (PC side).
    pub icon_file: Option<String>,
    /// `IconIndex`.
    pub icon_index: Option<i32>,
}

/// The components of an Application Manager INI.
pub fn components(bytes: &[u8]) -> Result<Vec<Component>, String> {
    let ini = Ini::parse(bytes)?;
    if ini.get("CEAppManager", "Version").is_none() {
        return Err("not an Application Manager INI (no [CEAppManager] Version)".to_owned());
    }
    let list = ini
        .get("CEAppManager", "Component")
        .ok_or("INI names no Component")?;
    let mut out = Vec::new();
    for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        let get = |k: &str| {
            ini.get(name, k)
                .map(str::to_owned)
                .filter(|v| !v.is_empty())
        };
        let cab_files: Vec<String> = get("CabFiles")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_owned)
            .collect();
        if cab_files.is_empty() {
            return Err(format!("component {name:?} lists no CabFiles"));
        }
        out.push(Component {
            name: name.to_owned(),
            description: get("Description"),
            cab_files,
            uninstall: get("Uninstall"),
            device_file: get("DeviceFile"),
            icon_file: get("IconFile"),
            icon_index: get("IconIndex").and_then(|v| v.parse().ok()),
        });
    }
    if out.is_empty() {
        return Err("INI names no Component".to_owned());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_real_shapes_parse() {
        // Shapes of the owner's real INIs: spaces or none around '=',
        // IconIndex -1, a GUID uninstall name, CAB names with spaces.
        let toy = b"[CEAppManager]\r\nVersion=1.0\r\nComponent=ISCOMPONENT\r\n[ISCOMPONENT]\r\nIconFile=toygolf.ico\r\nIconIndex=-1\r\nDescription=Fathammer Toy Golf\r\nDeviceFile=toygolf.exe\r\nCabFiles=Fathammer Toy Golf.CAB\r\nUninstall={ECC51961-496D-4E12-B0FE-AD68C25AFFC1}\r\n";
        let c = components(toy).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].cab_files, ["Fathammer Toy Golf.CAB"]);
        assert_eq!(c[0].icon_index, Some(-1));
        assert_eq!(c[0].device_file.as_deref(), Some("toygolf.exe"));
        let vpm = b"[CEAppManager]\nVersion      = 1.0\nComponent    = Virtual Pool Mobile\n\n[Virtual Pool Mobile]\nDescription  = Virtual Pool Mobile\nUninstall    = VPMobile\nCabFiles     = VPMobileFullPPC.SA.cab\n";
        let c = components(vpm).unwrap();
        assert_eq!(c[0].name, "Virtual Pool Mobile");
        assert_eq!(c[0].uninstall.as_deref(), Some("VPMobile"));
    }

    #[test]
    fn several_components_and_cabs() {
        let ini = b"[ceappmanager]\nversion=1.0\ncomponent=Game, Dictionary\n[Game]\nCabFiles=a.ARM.CAB,a.MIPS.CAB , a.SH3.CAB\n[dictionary]\ncabfiles=dict.cab\n";
        let c = components(ini).unwrap();
        assert_eq!(c[0].cab_files, ["a.ARM.CAB", "a.MIPS.CAB", "a.SH3.CAB"]);
        assert_eq!(c[1].cab_files, ["dict.cab"]);
    }

    #[test]
    fn encodings() {
        let utf16: Vec<u8> = b"\xFF\xFE"
            .iter()
            .copied()
            .chain(
                "[CEAppManager]\nVersion=1.0\nComponent=X\n[X]\nCabFiles=x.cab\n"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes),
            )
            .collect();
        assert_eq!(components(&utf16).unwrap()[0].cab_files, ["x.cab"]);
        let ansi =
            b"[CEAppManager]\nVersion=1.0\nComponent=X\n[X]\nDescription=Caf\xE9\nCabFiles=x.cab\n";
        assert_eq!(
            components(ansi).unwrap()[0].description.as_deref(),
            Some("Caf\u{e9}")
        );
    }

    #[test]
    fn refusals() {
        assert!(components(b"[Other]\nx=1\n").is_err());
        assert!(components(b"[CEAppManager]\nVersion=1.0\n").is_err());
        assert!(
            components(b"[CEAppManager]\nVersion=1.0\nComponent=X\n[X]\nDescription=no cabs\n")
                .is_err()
        );
        assert!(components(&vec![b'x'; MAX_INI + 1]).is_err());
        // Junk lines are ignored, not fatal.
        let ini =
            b"garbage\n[CEAppManager]\nVersion=1.0\nnoequals\nComponent=X\n[X]\nCabFiles=x.cab\n";
        assert!(components(ini).is_ok());
    }
}
