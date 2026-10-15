//! `_setup.xml`: the provisioning-XML form of a setup script.
//!
//! **Confidence: likely.** Built from Windows Mobile's public provisioning
//! documentation (the `Install`, `FileOperation` and `Registry`
//! characteristics); none of the seven real CABs on the Dell discs uses it
//! (docs/legacysync/activesync-interfaces.md §6), so a real sample must
//! confirm it before the device trusts it. Anything outside this subset is
//! refused, never skipped, so an install is never silently partial:
//!
//! ```xml
//! <wap-provisioningdoc>
//!   <characteristic type="Install">
//!     <parm name="AppName" value="Provider App" />
//!     <parm name="InstallDir" value="%CE1%\App" translation="install" />
//!   </characteristic>
//!   <characteristic type="FileOperation">
//!     <characteristic type="%InstallDir%" translation="install">
//!       <characteristic type="MakeDir" />
//!       <characteristic type="App.exe" translation="install">
//!         <characteristic type="Extract">
//!           <parm name="Source" value="APP~1.001" />
//!         </characteristic>
//!       </characteristic>
//!     </characteristic>
//!     <characteristic type="%CE11%" translation="install">
//!       <characteristic type="App.lnk" translation="install">
//!         <characteristic type="Shortcut">
//!           <parm name="Source" value="%InstallDir%\App.exe" />
//!         </characteristic>
//!       </characteristic>
//!     </characteristic>
//!   </characteristic>
//!   <characteristic type="Registry">
//!     <characteristic type="HKLM\Software\App">
//!       <parm name="Level" value="3" datatype="integer" />
//!       <parm name="Name" value="x" datatype="string" />
//!     </characteristic>
//!   </characteristic>
//! </wap-provisioningdoc>
//! ```
//!
//! The reader is deliberately small and strict (docs/security.md): UTF-8
//! only, no DTD or entity declarations (so no entity expansion attacks), the
//! five predefined entities and numeric references only, nesting at most
//! [`MAX_DEPTH`], every element closed by its own name, and the same size
//! cap as `.000` scripts.

use crate::{CePath, Error, Item, MAX_PATH, MAX_SCRIPT, RegData, Root, member_id};

/// Deepest element nesting accepted (the subset needs 6).
pub const MAX_DEPTH: usize = 8;

/// A decoded attribute value.
#[derive(Clone)]
struct Text {
    buf: [u8; MAX_PATH],
    len: usize,
}

impl Text {
    fn bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// Decodes an attribute's raw bytes: entities and numeric references.
    fn decode(raw: &[u8]) -> Result<Text, Error> {
        let mut t = Text {
            buf: [0; MAX_PATH],
            len: 0,
        };
        let mut i = 0;
        while i < raw.len() {
            let c = raw[i];
            if c == b'<' {
                return Err(Error::BadXml);
            }
            if c != b'&' {
                t.put(&[c])?;
                i += 1;
                continue;
            }
            let semi = raw[i..]
                .iter()
                .position(|&b| b == b';')
                .ok_or(Error::BadXml)?
                + i;
            let name = &raw[i + 1..semi];
            match name {
                b"amp" => t.put(b"&")?,
                b"lt" => t.put(b"<")?,
                b"gt" => t.put(b">")?,
                b"quot" => t.put(b"\"")?,
                b"apos" => t.put(b"'")?,
                _ => {
                    let digits = name.strip_prefix(b"#").ok_or(Error::BadXml)?;
                    let (digits, radix) = match digits.strip_prefix(b"x") {
                        Some(hex) => (hex, 16),
                        None => (digits, 10),
                    };
                    if digits.is_empty() || digits.len() > 8 {
                        return Err(Error::BadXml);
                    }
                    let mut v: u32 = 0;
                    for &d in digits {
                        let n = char::from(d).to_digit(radix).ok_or(Error::BadXml)?;
                        v = v * radix + n;
                    }
                    let ch = char::from_u32(v)
                        .filter(|&c| c != '\0')
                        .ok_or(Error::BadXml)?;
                    let mut utf8 = [0u8; 4];
                    t.put(ch.encode_utf8(&mut utf8).as_bytes())?;
                }
            }
            i = semi + 1;
        }
        Ok(t)
    }

    fn put(&mut self, b: &[u8]) -> Result<(), Error> {
        let end = self.len + b.len();
        if end > MAX_PATH {
            return Err(Error::BadPath);
        }
        self.buf[self.len..end].copy_from_slice(b);
        self.len = end;
        Ok(())
    }
}

/// A start tag: its name and raw attribute text.
#[derive(Clone, Copy)]
struct Tag<'a> {
    name: &'a [u8],
    attrs: &'a [u8],
    /// `<x ... />`.
    empty: bool,
}

impl<'a> Tag<'a> {
    /// The raw value of attribute `want`, if present.
    fn attr(&self, want: &[u8]) -> Result<Option<&'a [u8]>, Error> {
        let mut rest = self.attrs;
        let mut found = None;
        loop {
            rest = trim_start(rest);
            if rest.is_empty() {
                return Ok(found);
            }
            let eq = rest.iter().position(|&b| b == b'=').ok_or(Error::BadXml)?;
            let name = trim_end(&rest[..eq]);
            if !is_name(name) {
                return Err(Error::BadXml);
            }
            rest = trim_start(&rest[eq + 1..]);
            let quote = *rest.first().ok_or(Error::BadXml)?;
            if quote != b'"' && quote != b'\'' {
                return Err(Error::BadXml);
            }
            let end = rest[1..]
                .iter()
                .position(|&b| b == quote)
                .ok_or(Error::BadXml)?
                + 1;
            if name == want {
                if found.is_some() {
                    return Err(Error::BadXml);
                }
                found = Some(&rest[1..end]);
            }
            rest = &rest[end + 1..];
            if !rest.is_empty() && !rest[0].is_ascii_whitespace() {
                return Err(Error::BadXml);
            }
        }
    }

    fn decoded(&self, want: &[u8]) -> Result<Option<Text>, Error> {
        self.attr(want)?.map(Text::decode).transpose()
    }

    fn required(&self, want: &[u8]) -> Result<Text, Error> {
        self.decoded(want)?.ok_or(Error::BadXml)
    }
}

fn trim_start(b: &[u8]) -> &[u8] {
    let n = b.iter().take_while(|c| c.is_ascii_whitespace()).count();
    &b[n..]
}

fn trim_end(b: &[u8]) -> &[u8] {
    let n = b
        .iter()
        .rev()
        .take_while(|c| c.is_ascii_whitespace())
        .count();
    &b[..b.len() - n]
}

fn is_name(b: &[u8]) -> bool {
    !b.is_empty()
        && b.iter()
            .all(|&c| c.is_ascii_alphanumeric() || b"-_:.".contains(&c))
}

/// A token of the document.
enum Tok<'a> {
    Open(Tag<'a>),
    Close(&'a [u8]),
    Eof,
}

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Lexer<'a> {
    fn find(&self, from: usize, pat: &[u8]) -> Result<usize, Error> {
        self.s
            .get(from..)
            .and_then(|r| r.windows(pat.len()).position(|w| w == pat))
            .map(|p| from + p)
            .ok_or(Error::BadXml)
    }

    fn next(&mut self) -> Result<Tok<'a>, Error> {
        loop {
            // Text between tags: only whitespace is meaningful here, and
            // nothing in this subset carries text.
            let Some(lt) = self.s[self.i..].iter().position(|&b| b == b'<') else {
                if self.s[self.i..].iter().all(u8::is_ascii_whitespace) {
                    return Ok(Tok::Eof);
                }
                return Err(Error::BadXml);
            };
            if !self.s[self.i..self.i + lt]
                .iter()
                .all(u8::is_ascii_whitespace)
            {
                return Err(Error::BadXml);
            }
            let at = self.i + lt;
            let rest = &self.s[at..];
            if rest.starts_with(b"<?") {
                self.i = self.find(at + 2, b"?>")? + 2;
            } else if rest.starts_with(b"<!--") {
                self.i = self.find(at + 4, b"-->")? + 3;
            } else if rest.starts_with(b"<!") {
                // DOCTYPE, ENTITY, CDATA: refused.
                return Err(Error::BadXml);
            } else if rest.starts_with(b"</") {
                let end = self.find(at, b">")?;
                let name = trim_end(&self.s[at + 2..end]);
                if !is_name(name) {
                    return Err(Error::BadXml);
                }
                self.i = end + 1;
                return Ok(Tok::Close(name));
            } else {
                // A start tag: '>' may appear inside quoted values.
                let mut j = at + 1;
                let mut quote = 0u8;
                loop {
                    let c = *self.s.get(j).ok_or(Error::BadXml)?;
                    if quote != 0 {
                        if c == quote {
                            quote = 0;
                        }
                    } else if c == b'"' || c == b'\'' {
                        quote = c;
                    } else if c == b'>' {
                        break;
                    } else if c == b'<' {
                        return Err(Error::BadXml);
                    }
                    j += 1;
                }
                let inner = &self.s[at + 1..j];
                let (inner, empty) = match inner.strip_suffix(b"/") {
                    Some(i) => (i, true),
                    None => (inner, false),
                };
                let name_len = inner
                    .iter()
                    .position(u8::is_ascii_whitespace)
                    .unwrap_or(inner.len());
                let tag = Tag {
                    name: &inner[..name_len],
                    attrs: &inner[name_len..],
                    empty,
                };
                if !is_name(tag.name) {
                    return Err(Error::BadXml);
                }
                // Validate every attribute's syntax now.
                tag.attr(b"")?;
                self.i = j + 1;
                return Ok(Tok::Open(tag));
            }
        }
    }
}

/// A parsed, fully validated `_setup.xml`.
pub struct Setup<'a> {
    doc: &'a [u8],
    app_name: Option<Text>,
    install_dir: Option<Text>,
}

impl<'a> Setup<'a> {
    /// Parses and validates `doc` end to end.
    pub fn parse(doc: &'a [u8]) -> Result<Self, Error> {
        if doc.len() > MAX_SCRIPT {
            return Err(Error::TooLarge);
        }
        let doc = doc.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(doc);
        if core::str::from_utf8(doc).is_err() {
            return Err(Error::BadXml);
        }
        let mut setup = Setup {
            doc,
            app_name: None,
            install_dir: None,
        };
        let (mut name, mut dir) = (None, None);
        walk(doc, &mut |_| Ok(()), &mut name, &mut dir)?;
        setup.app_name = name;
        setup.install_dir = dir;
        Ok(setup)
    }

    /// The `Install` characteristic's `AppName`, if given.
    #[must_use]
    pub fn app_name(&self) -> Option<&[u8]> {
        self.app_name.as_ref().map(Text::bytes)
    }

    /// The `Install` characteristic's `InstallDir`, if given.
    #[must_use]
    pub fn install_dir(&self) -> Option<&[u8]> {
        self.install_dir.as_ref().map(Text::bytes)
    }

    /// Calls `visit` with every item, in document order.
    pub fn plan<F>(&self, mut visit: F) -> Result<(), Error>
    where
        F: FnMut(Item<'_>) -> Result<(), Error>,
    {
        walk(self.doc, &mut visit, &mut None, &mut None)
    }
}

type Visit<'v> = dyn FnMut(Item<'_>) -> Result<(), Error> + 'v;

struct Walker<'a, 'v, 'o> {
    lx: Lexer<'a>,
    visit: &'v mut Visit<'v>,
    app_name: &'o mut Option<Text>,
    install_dir: &'o mut Option<Text>,
}

fn walk(
    doc: &[u8],
    visit: &mut Visit<'_>,
    app_name: &mut Option<Text>,
    install_dir: &mut Option<Text>,
) -> Result<(), Error> {
    let mut w = Walker {
        lx: Lexer { s: doc, i: 0 },
        visit,
        app_name,
        install_dir,
    };
    let Tok::Open(root) = w.lx.next()? else {
        return Err(Error::BadXml);
    };
    if root.name != b"wap-provisioningdoc" {
        return Err(Error::BadXml);
    }
    if !root.empty {
        w.children(root.name, 1, |w, tag, depth| {
            let ty = w.characteristic(&tag)?;
            match ty.bytes() {
                b"Install" => w.install(tag, depth),
                b"FileOperation" => w.file_operation(tag, depth),
                b"Registry" => w.children(tag.name, depth, |w, tag, depth| {
                    let key = w.characteristic(&tag)?;
                    let (root, rest) = split_root(key.bytes())?;
                    let mut path = CePath::new();
                    path.push_key(rest)?;
                    w.registry_key(tag, depth, root, &path)
                }),
                _ => Err(Error::BadXml),
            }
        })?;
    }
    match w.lx.next()? {
        Tok::Eof => Ok(()),
        _ => Err(Error::BadXml),
    }
}

fn split_root(key: &[u8]) -> Result<(Root, &[u8]), Error> {
    let slash = key.iter().position(|&b| b == b'\\').ok_or(Error::BadPath)?;
    let root = match &key[..slash] {
        b"HKCR" | b"HKEY_CLASSES_ROOT" => Root::ClassesRoot,
        b"HKCU" | b"HKEY_CURRENT_USER" => Root::CurrentUser,
        b"HKLM" | b"HKEY_LOCAL_MACHINE" => Root::LocalMachine,
        b"HKU" | b"HKEY_USERS" => Root::Users,
        _ => return Err(Error::BadValue),
    };
    Ok((root, &key[slash + 1..]))
}

impl Walker<'_, '_, '_> {
    /// Calls `each` for every child element of the open element `parent`
    /// (at `depth`), up to its matching close tag.
    fn children<F>(&mut self, parent: &[u8], depth: usize, mut each: F) -> Result<(), Error>
    where
        F: FnMut(&mut Self, Tag<'_>, usize) -> Result<(), Error>,
    {
        if depth >= MAX_DEPTH {
            return Err(Error::BadXml);
        }
        loop {
            match self.lx.next()? {
                Tok::Open(tag) => each(self, tag, depth + 1)?,
                Tok::Close(name) if name == parent => return Ok(()),
                _ => return Err(Error::BadXml),
            }
        }
    }

    /// Requires `tag` to be `<characteristic type="...">`; returns the type.
    fn characteristic(&self, tag: &Tag<'_>) -> Result<Text, Error> {
        if tag.name != b"characteristic" {
            return Err(Error::BadXml);
        }
        tag.required(b"type")
    }

    /// Requires `tag` to be a `<parm>` with no children.
    fn parm(&mut self, tag: &Tag<'_>) -> Result<(), Error> {
        if tag.name != b"parm" {
            return Err(Error::BadXml);
        }
        if !tag.empty {
            self.children(tag.name, 0, |_, _, _| Err(Error::BadXml))?;
        }
        Ok(())
    }

    /// An empty characteristic (`<characteristic type="MakeDir" />` or the
    /// same with a close tag).
    fn leaf(&mut self, tag: &Tag<'_>, depth: usize) -> Result<(), Error> {
        if !tag.empty {
            self.children(tag.name, depth, |_, _, _| Err(Error::BadXml))?;
        }
        Ok(())
    }

    fn install(&mut self, tag: Tag<'_>, depth: usize) -> Result<(), Error> {
        if tag.empty {
            return Ok(());
        }
        self.children(tag.name, depth, |w, parm, _| {
            w.parm(&parm)?;
            let value = parm.decoded(b"value")?;
            match parm.required(b"name")?.bytes() {
                b"AppName" => *w.app_name = value,
                b"InstallDir" => *w.install_dir = value,
                // Counts and the install phase: informational.
                _ => {}
            }
            Ok(())
        })
    }

    fn file_operation(&mut self, tag: Tag<'_>, depth: usize) -> Result<(), Error> {
        if tag.empty {
            return Ok(());
        }
        self.children(tag.name, depth, |w, dir_tag, depth| {
            let mut dir = CePath::new();
            dir.push(w.characteristic(&dir_tag)?.bytes())?;
            if dir_tag.empty {
                return Ok(());
            }
            w.children(dir_tag.name, depth, |w, item, depth| {
                let name = w.characteristic(&item)?;
                if name.bytes() == b"MakeDir" {
                    w.leaf(&item, depth)?;
                    return (w.visit)(Item::Directory { path: &dir });
                }
                let mut dest = dir.clone();
                dest.push_name(name.bytes())?;
                if item.empty {
                    return Err(Error::BadXml);
                }
                let mut done = false;
                w.children(item.name, depth, |w, action, depth| {
                    let kind = w.characteristic(&action)?;
                    if done || action.empty {
                        return Err(Error::BadXml);
                    }
                    done = true;
                    let mut source = None;
                    w.children(action.name, depth, |w, parm, _| {
                        w.parm(&parm)?;
                        if parm.required(b"name")?.bytes() != b"Source" || source.is_some() {
                            return Err(Error::BadXml);
                        }
                        source = Some(parm.required(b"value")?);
                        Ok(())
                    })?;
                    let source = source.ok_or(Error::BadXml)?;
                    match kind.bytes() {
                        b"Extract" => {
                            let id = member_id(source.bytes()).ok_or(Error::BadReference)?;
                            (w.visit)(Item::File {
                                id,
                                dest: &dest,
                                flags: 0,
                            })
                        }
                        b"Shortcut" | b"MakeShortcut" => {
                            let mut target = CePath::new();
                            target.push(source.bytes())?;
                            (w.visit)(Item::Shortcut {
                                path: &dest,
                                target: &target,
                            })
                        }
                        _ => Err(Error::BadXml),
                    }
                })?;
                if done { Ok(()) } else { Err(Error::BadXml) }
            })
        })
    }

    fn registry_key(
        &mut self,
        tag: Tag<'_>,
        depth: usize,
        root: Root,
        key: &CePath,
    ) -> Result<(), Error> {
        if tag.empty {
            return Ok(());
        }
        self.children(tag.name, depth, |w, child, depth| {
            if child.name == b"characteristic" {
                // A subkey.
                let mut sub = key.clone();
                sub.push_key(w.characteristic(&child)?.bytes())?;
                return w.registry_key(child, depth, root, &sub);
            }
            w.parm(&child)?;
            let name = child.required(b"name")?;
            let value = child.decoded(b"value")?;
            let datatype = child.decoded(b"datatype")?;
            let value_bytes = value.as_ref().map_or(&[][..], Text::bytes);
            let data = match datatype.as_ref().map_or(&b"string"[..], Text::bytes) {
                b"string" => RegData::String(value_bytes),
                b"integer" => RegData::Dword(parse_u32(value_bytes)?),
                // Binary and multi-string encodings are unconfirmed:
                // refused until a real sample shows them.
                _ => return Err(Error::BadValue),
            };
            (w.visit)(Item::RegValue {
                root,
                key,
                name: name.bytes(),
                data,
                flags: 0,
                expand: true,
            })
        })
    }
}

/// A decimal or `0x` hexadecimal 32-bit number.
fn parse_u32(b: &[u8]) -> Result<u32, Error> {
    let (digits, radix) = match b.strip_prefix(b"0x").or_else(|| b.strip_prefix(b"0X")) {
        Some(h) => (h, 16),
        None => (b, 10),
    };
    if digits.is_empty() {
        return Err(Error::BadValue);
    }
    let mut v: u32 = 0;
    for &d in digits {
        let n = char::from(d).to_digit(radix).ok_or(Error::BadValue)?;
        v = v
            .checked_mul(radix)
            .and_then(|v| v.checked_add(n))
            .ok_or(Error::BadValue)?;
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_and_references_decode() {
        let t = Text::decode(b"a&amp;b&lt;&#65;&#x42;").unwrap();
        assert_eq!(t.bytes(), b"a&b<AB");
        for bad in [
            &b"&bogus;"[..],
            b"&#0;",
            b"&#xD800;",
            b"&amp",
            b"a<b",
            b"&#x110000;",
        ] {
            assert!(Text::decode(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn numbers() {
        assert_eq!(parse_u32(b"42"), Ok(42));
        assert_eq!(parse_u32(b"0xFF"), Ok(255));
        assert_eq!(parse_u32(b"4294967296"), Err(Error::BadValue));
        assert_eq!(parse_u32(b""), Err(Error::BadValue));
        assert_eq!(parse_u32(b"-1"), Err(Error::BadValue));
    }
}
