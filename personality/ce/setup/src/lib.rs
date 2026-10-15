//! Pocket PC setup scripts: what a CAB installs where.
//!
//! A device CAB carries a setup script beside its files: the binary `.000`
//! file ("MSCE", every real CAB seen so far) or, from newer tools,
//! `_setup.xml` ([`xml`]). Both become the same [`Item`] stream: directories
//! to make, files to place, registry values to write, shortcuts to make.
//! The CE personality's installer (ADR-0028 L2) carries it out on the
//! device; LegacySync shows it on the PC before anything is sent.
//!
//! The `.000` layout is documented, from seven real Dell CABs, in
//! docs/legacysync/activesync-interfaces.md §6 (facts only, clean-room).
//!
//! **Outside input** (docs/security.md): the script comes from a CAB from
//! anywhere. [`Script::parse`] checks every offset, length, count and
//! cross-reference, and walks the whole plan once, so a script is either
//! valid end to end or refused before the first file is placed (no
//! half-done install from an entry near the end). Paths are checked too:
//! no `..` or `.` pieces, no empty pieces, no characters Windows forbids in
//! names, no file or shortcut name smuggling a `\`. Sizes are capped
//! ([`MAX_SCRIPT`], [`MAX_ENTRIES`]) so the linear lookups stay bounded
//! (real scripts: under 4 KiB, at most 67 entries a table). No heap.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod xml;

/// Why a script was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// Not a setup script (bad signature or header).
    NotScript,
    /// A table or entry runs past the end of the script.
    Truncated,
    /// Bigger than [`MAX_SCRIPT`] or a table longer than [`MAX_ENTRIES`].
    TooLarge,
    /// An entry names a string, directory, file or hive that doesn't exist.
    BadReference,
    /// A path is too long or has a forbidden piece or character.
    BadPath,
    /// A registry value's type or data is malformed.
    BadValue,
    /// The XML is malformed or uses something this reader refuses (a DTD,
    /// an unknown entity).
    BadXml,
    /// The caller's visitor refused an item.
    Sink,
}

impl Error {
    /// A short name for logs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Error::NotScript => "not a setup script",
            Error::Truncated => "truncated",
            Error::TooLarge => "too large",
            Error::BadReference => "bad reference",
            Error::BadPath => "bad path",
            Error::BadValue => "bad registry value",
            Error::BadXml => "bad XML",
            Error::Sink => "refused",
        }
    }
}

/// Largest script accepted (real ones: 363 bytes to 3 KiB).
pub const MAX_SCRIPT: usize = 64 * 1024;
/// Most entries in one table (real ones: at most 67).
pub const MAX_ENTRIES: u16 = 4096;
/// Longest path, in bytes (Windows CE's `MAX_PATH`).
pub const MAX_PATH: usize = 260;

/// A path being built for a plan item: pieces joined with `\`, each
/// checked.
#[derive(Clone)]
pub struct CePath {
    buf: [u8; MAX_PATH],
    len: usize,
}

impl CePath {
    /// An empty path.
    #[must_use]
    pub const fn new() -> Self {
        CePath {
            buf: [0; MAX_PATH],
            len: 0,
        }
    }

    /// The path's bytes (the script's code page, usually Windows-1252).
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// Appends `piece` (which may itself hold `\`-separated pieces) after a
    /// `\`, checking every piece as a file-system name.
    pub fn push(&mut self, piece: &[u8]) -> Result<(), Error> {
        check_pieces(piece, b"/:*?\"<>|")?;
        self.append(piece)
    }

    /// Appends registry key pieces. Key names may hold any printable
    /// character but `\` (CityTime's key ends `\Items\"CityTime"`), and
    /// `.`/`..` mean nothing there; empty pieces and control characters are
    /// still refused.
    pub fn push_key(&mut self, piece: &[u8]) -> Result<(), Error> {
        check_pieces(piece, b"")?;
        self.append(piece)
    }

    fn append(&mut self, piece: &[u8]) -> Result<(), Error> {
        let sep = usize::from(self.len > 0);
        let new_len = self.len + sep + piece.len();
        if new_len > MAX_PATH {
            return Err(Error::BadPath);
        }
        if sep == 1 {
            self.buf[self.len] = b'\\';
        }
        self.buf[self.len + sep..new_len].copy_from_slice(piece);
        self.len = new_len;
        Ok(())
    }

    /// Appends one name (a file or shortcut name): a single piece, no `\`.
    pub fn push_name(&mut self, name: &[u8]) -> Result<(), Error> {
        if name.contains(&b'\\') {
            return Err(Error::BadPath);
        }
        self.push(name)
    }
}

impl Default for CePath {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for CePath {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for &b in self.as_bytes() {
            core::fmt::Write::write_char(f, char::from(b))?;
        }
        Ok(())
    }
}

/// Checks `\`-separated pieces: a leading `\` (an absolute path) is fine;
/// empty pieces elsewhere, control characters and the `forbidden` ones are
/// not; for file-system names (`forbidden` not empty) neither are `.` and
/// `..`.
fn check_pieces(path: &[u8], forbidden: &[u8]) -> Result<(), Error> {
    let body = path.strip_prefix(b"\\").unwrap_or(path);
    let names = !forbidden.is_empty();
    for piece in body.split(|&b| b == b'\\') {
        let bad_char = piece
            .iter()
            .any(|&b| b < 0x20 || b == 0x7F || forbidden.contains(&b));
        let dots = names && (piece == b"." || piece == b"..");
        if piece.is_empty() || dots || bad_char {
            return Err(Error::BadPath);
        }
    }
    Ok(())
}

/// [`CePath::push`] or [`CePath::push_key`].
type PushFn = fn(&mut CePath, &[u8]) -> Result<(), Error>;

/// A registry root.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Root {
    /// `HKEY_CLASSES_ROOT`.
    ClassesRoot,
    /// `HKEY_CURRENT_USER`.
    CurrentUser,
    /// `HKEY_LOCAL_MACHINE`.
    LocalMachine,
    /// `HKEY_USERS`.
    Users,
}

impl Root {
    /// The usual short name (`HKLM`).
    #[must_use]
    pub const fn short(self) -> &'static str {
        match self {
            Root::ClassesRoot => "HKCR",
            Root::CurrentUser => "HKCU",
            Root::LocalMachine => "HKLM",
            Root::Users => "HKU",
        }
    }
}

/// A registry value's data.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegData<'a> {
    /// A string (without its NUL).
    String(&'a [u8]),
    /// Strings separated by NULs (without the final double NUL).
    MultiString(&'a [u8]),
    /// A 32-bit number.
    Dword(u32),
    /// Raw bytes.
    Binary(&'a [u8]),
}

/// Registry value flag: keep an existing value.
pub const REG_NO_CLOBBER: u32 = 0x0000_0002;

/// One thing to do, in the order the installer does them: directories,
/// files, registry values, shortcuts.
#[derive(Debug)]
pub enum Item<'p> {
    /// Make a directory.
    Directory {
        /// Its path, macros (`%CE1%`, `%InstallDir%`) unexpanded.
        path: &'p CePath,
    },
    /// Place a file.
    File {
        /// The file's number: the CAB member whose name ends `.NNN` (see
        /// [`member_id`]).
        id: u16,
        /// Where it goes, name included.
        dest: &'p CePath,
        /// Copy flags (INF `CopyFiles` meanings; `0x4000_0000` shared,
        /// `0x8000_0000` self-registering).
        flags: u32,
    },
    /// Write a registry value.
    RegValue {
        /// The root key.
        root: Root,
        /// The key below the root.
        key: &'p CePath,
        /// The value's name (empty for the key's default value).
        name: &'p [u8],
        /// The data.
        data: RegData<'p>,
        /// [`REG_NO_CLOBBER`] and others.
        flags: u32,
        /// Expand `%..%` macros in the data first.
        expand: bool,
    },
    /// Make a shortcut.
    Shortcut {
        /// The shortcut file's path (`...\Name.lnk`).
        path: &'p CePath,
        /// What it points at.
        target: &'p CePath,
    },
}

/// The file number of a CAB member: `CITYTI~1.000` is 0 (the script),
/// `CityTime.011` is 11, `...999` the setup DLL.
#[must_use]
pub fn member_id(name: &[u8]) -> Option<u16> {
    let dot = name.iter().rposition(|&b| b == b'.')?;
    let ext = &name[dot + 1..];
    if ext.len() != 3 || !ext.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(ext.iter().fold(0u16, |n, &d| n * 10 + u16::from(d - b'0')))
}

/// What Pocket PC's `%CEn%` folder macros stand for (public CAB Wizard
/// documentation; confidence "likely" until checked on the device).
#[must_use]
pub const fn pocket_pc_folder(n: u8) -> Option<&'static str> {
    Some(match n {
        1 => "\\Program Files",
        2 => "\\Windows",
        3 => "\\Windows\\Desktop",
        4 => "\\Windows\\StartUp",
        5 => "\\My Documents",
        6 => "\\Program Files\\Accessories",
        7 => "\\Program Files\\Communication",
        8 => "\\Program Files\\Games",
        9 => "\\Program Files\\Pocket Outlook",
        10 => "\\Program Files\\Office",
        11 => "\\Windows\\Start Menu\\Programs",
        12 => "\\Windows\\Start Menu\\Programs\\Accessories",
        13 => "\\Windows\\Start Menu\\Programs\\Communications",
        14 => "\\Windows\\Start Menu\\Programs\\Games",
        15 => "\\Windows\\Fonts",
        16 => "\\Windows\\Recent",
        17 => "\\Windows\\Start Menu",
        _ => return None,
    })
}

const HEADER: usize = 100;
const T_STRINGS: usize = 0;
const T_DIRS: usize = 1;
const T_FILES: usize = 2;
const T_HIVES: usize = 3;
const T_VALUES: usize = 4;
const T_LINKS: usize = 5;

fn u16_at(b: &[u8], at: usize) -> Result<u16, Error> {
    let s = b.get(at..at + 2).ok_or(Error::Truncated)?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], at: usize) -> Result<u32, Error> {
    let s = b.get(at..at + 4).ok_or(Error::Truncated)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Strips one trailing NUL (and any after it), as the script stores text.
fn text(b: &[u8]) -> &[u8] {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    &b[..end]
}

/// A parsed, fully validated `.000` script.
#[derive(Clone, Copy, Debug)]
pub struct Script<'a> {
    bytes: &'a [u8],
    /// Processor type the CAB is for (0 = any, 2577 = ARM/StrongARM).
    pub processor: u32,
    /// Oldest OS version supported (major, minor).
    pub os_min: (u32, u32),
    /// Newest OS version supported (major, minor).
    pub os_max: (u32, u32),
    /// Oldest and newest OS build supported.
    pub build: (u32, u32),
    counts: [u16; 6],
    offsets: [usize; 6],
    app_name: &'a [u8],
    provider: &'a [u8],
    unsupported: &'a [u8],
}

/// One entry of a table, by its fields' positions.
#[derive(Clone, Copy)]
struct Entry {
    id: u16,
    at: usize,
    len: usize,
}

impl<'a> Script<'a> {
    /// Parses and validates `bytes` (a CAB's `.000` member) end to end.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_SCRIPT {
            return Err(Error::TooLarge);
        }
        if bytes.len() < HEADER || &bytes[..4] != b"MSCE" {
            return Err(Error::NotScript);
        }
        let declared = u32_at(bytes, 8)? as usize;
        if declared < HEADER || declared > bytes.len() {
            return Err(Error::Truncated);
        }
        let bytes = &bytes[..declared];
        let mut counts = [0u16; 6];
        let mut offsets = [0usize; 6];
        for t in 0..6 {
            counts[t] = u16_at(bytes, 48 + 2 * t)?;
            offsets[t] = u32_at(bytes, 60 + 4 * t)? as usize;
            if counts[t] > MAX_ENTRIES {
                return Err(Error::TooLarge);
            }
            if counts[t] > 0 && (offsets[t] < HEADER || offsets[t] >= declared) {
                return Err(Error::Truncated);
            }
        }
        let field = |at: usize| -> Result<&'a [u8], Error> {
            let off = usize::from(u16_at(bytes, at)?);
            let len = usize::from(u16_at(bytes, at + 2)?);
            if len == 0 {
                return Ok(&[]);
            }
            if off < HEADER {
                return Err(Error::Truncated);
            }
            bytes.get(off..off + len).ok_or(Error::Truncated)
        };
        let script = Script {
            bytes,
            processor: u32_at(bytes, 20)?,
            os_min: (u32_at(bytes, 24)?, u32_at(bytes, 28)?),
            os_max: (u32_at(bytes, 32)?, u32_at(bytes, 36)?),
            build: (u32_at(bytes, 40)?, u32_at(bytes, 44)?),
            counts,
            offsets,
            app_name: text(field(84)?),
            provider: text(field(88)?),
            unsupported: field(92)?,
        };
        // Walk every table (bounds), then the whole plan (references and
        // paths): after this nothing in the script can fail.
        for t in 0..6 {
            for e in script.entries(t) {
                e?;
            }
        }
        script.plan(|_| Ok(()))?;
        Ok(script)
    }

    /// The program's name.
    #[must_use]
    pub const fn app_name(&self) -> &'a [u8] {
        self.app_name
    }

    /// Who makes it.
    #[must_use]
    pub const fn provider(&self) -> &'a [u8] {
        self.provider
    }

    /// Platforms the CAB refuses (`HPC`, `JUPITER`, ...).
    pub fn unsupported_platforms(&self) -> impl Iterator<Item = &'a [u8]> {
        self.unsupported
            .split(|&b| b == 0)
            .filter(|s| !s.is_empty())
    }

    /// Entry counts: strings, directories, files, hives, values, shortcuts.
    #[must_use]
    pub const fn counts(&self) -> [u16; 6] {
        self.counts
    }

    /// Walks table `t`: each entry is an id and a length-prefixed body,
    /// except files, values and shortcuts whose bodies have fixed fields
    /// before the length.
    fn entries(&self, t: usize) -> impl Iterator<Item = Result<Entry, Error>> + '_ {
        let mut at = self.offsets[t];
        let mut left = self.counts[t];
        let mut failed = false;
        core::iter::from_fn(move || {
            if left == 0 || failed {
                return None;
            }
            left -= 1;
            // Bytes of fixed fields between the id and the length field.
            let fixed = match t {
                T_STRINGS | T_DIRS => 0,
                T_FILES => 8,
                T_HIVES => 4,
                T_VALUES => 8,
                _ => 8,
            };
            let r = (|| {
                let id = u16_at(self.bytes, at)?;
                let len_at = at + 2 + fixed;
                let len = usize::from(u16_at(self.bytes, len_at)?);
                let body = len_at + 2;
                if body + len > self.bytes.len() {
                    return Err(Error::Truncated);
                }
                let e = Entry { id, at, len };
                at = body + len;
                Ok(e)
            })();
            failed = r.is_err();
            Some(r)
        })
    }

    /// Finds entry `id` in table `t`.
    fn find(&self, t: usize, id: u16) -> Result<Entry, Error> {
        for e in self.entries(t) {
            let e = e?;
            if e.id == id {
                return Ok(e);
            }
        }
        Err(Error::BadReference)
    }

    /// The text of string `id`.
    fn string(&self, id: u16) -> Result<&'a [u8], Error> {
        let e = self.find(T_STRINGS, id)?;
        Ok(text(&self.bytes[e.at + 4..e.at + 4 + e.len]))
    }

    /// Appends the strings named by the id list in `ids` (u16s, ending at
    /// the first 0 or the end) to `path`.
    fn push_ids(&self, path: &mut CePath, ids: &[u8], push: PushFn) -> Result<(), Error> {
        for pair in ids.chunks(2) {
            let &[lo, hi] = pair else {
                return Err(Error::Truncated);
            };
            let id = u16::from_le_bytes([lo, hi]);
            if id == 0 {
                break;
            }
            push(path, self.string(id)?)?;
        }
        Ok(())
    }

    fn dir_path(&self, id: u16) -> Result<CePath, Error> {
        let e = self.find(T_DIRS, id)?;
        let mut p = CePath::new();
        self.push_ids(
            &mut p,
            &self.bytes[e.at + 4..e.at + 4 + e.len],
            CePath::push,
        )?;
        if p.len == 0 {
            return Err(Error::BadPath);
        }
        Ok(p)
    }

    fn file_dest(&self, e: Entry) -> Result<(CePath, u32), Error> {
        let b = self.bytes;
        let dir = u16_at(b, e.at + 2)?;
        let flags = u32_at(b, e.at + 6)?;
        let name = text(&b[e.at + 12..e.at + 12 + e.len]);
        let mut p = self.dir_path(dir)?;
        p.push_name(name)?;
        Ok((p, flags))
    }

    /// Calls `visit` with every item, in install order. [`Script::parse`]
    /// has already run this once, so on a parsed script only `visit` can
    /// make it fail.
    pub fn plan<F>(&self, mut visit: F) -> Result<(), Error>
    where
        F: FnMut(Item<'_>) -> Result<(), Error>,
    {
        let b = self.bytes;
        for e in self.entries(T_DIRS) {
            let e = e?;
            let path = self.dir_path(e.id)?;
            visit(Item::Directory { path: &path })?;
        }
        for e in self.entries(T_FILES) {
            let e = e?;
            let (dest, flags) = self.file_dest(e)?;
            visit(Item::File {
                id: e.id,
                dest: &dest,
                flags,
            })?;
        }
        for e in self.entries(T_VALUES) {
            let e = e?;
            let hive = self.find(T_HIVES, u16_at(b, e.at + 2)?)?;
            let root = match u16_at(b, hive.at + 2)? {
                1 => Root::ClassesRoot,
                2 => Root::CurrentUser,
                3 => Root::LocalMachine,
                4 => Root::Users,
                _ => return Err(Error::BadValue),
            };
            let mut key = CePath::new();
            self.push_ids(
                &mut key,
                &b[hive.at + 8..hive.at + 8 + hive.len],
                CePath::push_key,
            )?;
            let expand = u16_at(b, e.at + 4)? != 0;
            let flags = u32_at(b, e.at + 6)?;
            let body = &b[e.at + 12..e.at + 12 + e.len];
            let nul = body.iter().position(|&c| c == 0).ok_or(Error::BadValue)?;
            let (name, raw) = (&body[..nul], &body[nul + 1..]);
            let data = match flags & 0x0001_0001 {
                0x0000_0000 => RegData::String(text(raw)),
                0x0001_0000 => {
                    let mut end = raw.len();
                    while end > 0 && raw[end - 1] == 0 {
                        end -= 1;
                    }
                    RegData::MultiString(&raw[..end])
                }
                0x0001_0001 => {
                    let &[a, b2, c, d] = raw else {
                        return Err(Error::BadValue);
                    };
                    RegData::Dword(u32::from_le_bytes([a, b2, c, d]))
                }
                _ => RegData::Binary(raw),
            };
            visit(Item::RegValue {
                root,
                key: &key,
                name,
                data,
                flags,
                expand,
            })?;
        }
        for e in self.entries(T_LINKS) {
            let e = e?;
            let base = u16_at(b, e.at + 2)?;
            let target_id = u16_at(b, e.at + 6)?;
            let kind = u16_at(b, e.at + 8)?;
            let mut path = if base == 0 {
                CePath::new()
            } else {
                self.dir_path(base)?
            };
            let names = &b[e.at + 12..e.at + 12 + e.len];
            let before = path.len;
            for pair in names.chunks(2) {
                let &[lo, hi] = pair else {
                    return Err(Error::Truncated);
                };
                let id = u16::from_le_bytes([lo, hi]);
                if id == 0 {
                    break;
                }
                path.push_name(self.string(id)?)?;
            }
            if path.len == before {
                return Err(Error::BadPath);
            }
            let target = match kind {
                0 => self.dir_path(target_id)?,
                1 => self.file_dest(self.find(T_FILES, target_id)?)?.0,
                _ => return Err(Error::BadReference),
            };
            visit(Item::Shortcut {
                path: &path,
                target: &target,
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_ids() {
        assert_eq!(member_id(b"CITYTI~1.000"), Some(0));
        assert_eq!(member_id(b"CityTime.011"), Some(11));
        assert_eq!(member_id(b"000Setup.999"), Some(999));
        assert_eq!(member_id(b"readme.txt"), None);
        assert_eq!(member_id(b"noext"), None);
    }

    #[test]
    fn paths_refuse_escapes_and_odd_characters() {
        let mut p = CePath::new();
        assert!(p.push(b"%CE1%\\CodeCity\\CityTime").is_ok());
        assert!(p.push_name(b"CityTime.exe").is_ok());
        assert_eq!(p.as_bytes(), b"%CE1%\\CodeCity\\CityTime\\CityTime.exe");
        assert!(CePath::new().push(b"\\Program Files\\Axim").is_ok());
        for bad in [
            &b"..\\x"[..],
            b"a\\..",
            b"a\\\\b",
            b".",
            b"a:b",
            b"a/b",
            b"a\x01",
            b"",
            b"a\\",
        ] {
            assert_eq!(CePath::new().push(bad), Err(Error::BadPath), "{bad:?}");
        }
        assert_eq!(CePath::new().push_name(b"a\\b"), Err(Error::BadPath));
        // Registry keys: quotes and dots are just characters there.
        assert!(
            CePath::new()
                .push_key(b"Today\\Items\\\"CityTime\"")
                .is_ok()
        );
        assert!(CePath::new().push_key(b"a\\..\\b").is_ok());
        assert_eq!(CePath::new().push_key(b"a\\\\b"), Err(Error::BadPath));
        assert_eq!(CePath::new().push_key(b"a\x07"), Err(Error::BadPath));
        let long = [b'x'; MAX_PATH + 1];
        assert_eq!(CePath::new().push(&long), Err(Error::BadPath));
    }
}
