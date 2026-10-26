//! MTP's wire types [MTP 3.2]: little-endian integers, counted UTF-16
//! strings and arrays, DateTime strings. Encoding goes through [`Writer`],
//! which can also only count (the first pass of a list whose length must be
//! sent before its first byte); decoding through [`Reader`], which bounds
//! every field (input from the cable is hostile until checked).

/// Malformed or cut-short data from the cable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bad;

/// Most UTF-16 units in an MTP string, the terminator included
/// [MTP 3.2.3].
pub const STR_MAX: usize = 255;

/// A name or text as UTF-16 units (at most [`STR_MAX`] - 1, no NUL).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Text {
    units: [u16; STR_MAX],
    len: u8,
}

impl Default for Text {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for Text {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for c in char::decode_utf16(self.units().iter().copied()) {
            write!(f, "{}", c.unwrap_or(char::REPLACEMENT_CHARACTER))?;
        }
        Ok(())
    }
}

impl Text {
    /// The empty text.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            units: [0; STR_MAX],
            len: 0,
        }
    }

    /// From UTF-16 units; `None` if longer than MTP can carry or holding a
    /// NUL.
    #[must_use]
    pub fn from_units(u: &[u16]) -> Option<Self> {
        if u.len() >= STR_MAX || u.contains(&0) {
            return None;
        }
        let mut t = Self::new();
        t.units[..u.len()].copy_from_slice(u);
        t.len = u.len() as u8;
        Some(t)
    }

    /// From a `str` (`None` if it holds a NUL or is longer than MTP can carry).
    #[must_use]
    pub fn from_text(s: &str) -> Option<Self> {
        let mut t = Self::new();
        for u in s.encode_utf16() {
            if u == 0 || usize::from(t.len) >= STR_MAX - 1 {
                return None;
            }
            t.units[usize::from(t.len)] = u;
            t.len += 1;
        }
        Some(t)
    }

    /// The units.
    #[must_use]
    pub fn units(&self) -> &[u16] {
        &self.units[..usize::from(self.len)]
    }

    /// Whether it is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether it equals `s`, ignoring ASCII case.
    #[must_use]
    pub fn eq_ascii_ci(&self, s: &str) -> bool {
        let mut it = s.encode_utf16();
        for &u in self.units() {
            match it.next() {
                Some(v) if lower(u) == lower(v) => {}
                _ => return false,
            }
        }
        it.next().is_none()
    }

    /// The extension after the last dot, lowercased ASCII (at most 8
    /// bytes), for the format table.
    #[must_use]
    pub fn extension(&self) -> ([u8; 8], usize) {
        let u = self.units();
        let mut out = [0u8; 8];
        let Some(dot) = u.iter().rposition(|&c| c == u16::from(b'.')) else {
            return (out, 0);
        };
        let ext = &u[dot + 1..];
        if ext.len() > out.len() {
            return (out, 0);
        }
        for (o, &c) in out.iter_mut().zip(ext) {
            if c >= 0x80 {
                return ([0; 8], 0);
            }
            *o = (lower(c) & 0x7F) as u8;
        }
        (out, ext.len())
    }
}

fn lower(u: u16) -> u16 {
    if (u16::from(b'A')..=u16::from(b'Z')).contains(&u) {
        u + 32
    } else {
        u
    }
}

/// A date and time as MTP carries it [MTP 3.2.5]: `YYYYMMDDThhmmss[.s]`
/// with an optional zone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Time {
    /// Year (four digits).
    pub year: u16,
    /// 1..=12.
    pub month: u8,
    /// 1..=31.
    pub day: u8,
    /// 0..=23.
    pub hour: u8,
    /// 0..=59.
    pub minute: u8,
    /// 0..=59.
    pub second: u8,
    /// Tenths of a second, when known (0..=9).
    pub tenths: Option<u8>,
    /// Offset from UTC in minutes, when known (FAT has none; exFAT may).
    pub zone: Option<i16>,
}

impl Time {
    /// Whether every field is in range (the DateTime pattern of
    /// [MTP 3.2.5]).
    #[must_use]
    pub fn valid(&self) -> bool {
        self.year <= 9999
            && (1..=12).contains(&self.month)
            && (1..=31).contains(&self.day)
            && self.hour <= 23
            && self.minute <= 59
            && self.second <= 59
            && self.tenths.is_none_or(|t| t <= 9)
            && self.zone.is_none_or(|z| z.unsigned_abs() <= 23 * 60 + 59)
    }

    /// The string form, into `out` (ASCII); returns its length.
    #[must_use]
    pub fn format(&self, out: &mut [u8; 22]) -> usize {
        fn put(out: &mut [u8; 22], n: &mut usize, v: u32, digits: u32) {
            for i in (0..digits).rev() {
                out[*n] = b'0' + ((v / 10u32.pow(i)) % 10) as u8;
                *n += 1;
            }
        }
        let mut n = 0;
        put(out, &mut n, u32::from(self.year), 4);
        put(out, &mut n, u32::from(self.month), 2);
        put(out, &mut n, u32::from(self.day), 2);
        out[n] = b'T';
        n += 1;
        put(out, &mut n, u32::from(self.hour), 2);
        put(out, &mut n, u32::from(self.minute), 2);
        put(out, &mut n, u32::from(self.second), 2);
        if let Some(t) = self.tenths {
            out[n] = b'.';
            n += 1;
            put(out, &mut n, u32::from(t), 1);
        }
        match self.zone {
            Some(0) => {
                out[n] = b'Z';
                n += 1;
            }
            Some(z) => {
                out[n] = if z < 0 { b'-' } else { b'+' };
                n += 1;
                let a = u32::from(z.unsigned_abs());
                put(out, &mut n, a / 60, 2);
                put(out, &mut n, a % 60, 2);
            }
            None => {}
        }
        n
    }

    /// Parses the string form (an empty string is `None`; anything off the
    /// pattern is an error).
    ///
    /// # Errors
    /// `Err(Bad)` when it doesn't follow [MTP 3.2.5].
    pub fn parse(t: &Text) -> Result<Option<Self>, Bad> {
        let u = t.units();
        if u.is_empty() {
            return Ok(None);
        }
        let digit = |i: usize| -> Result<u32, Bad> {
            match u.get(i) {
                Some(&c) if (u16::from(b'0')..=u16::from(b'9')).contains(&c) => {
                    Ok(u32::from(c - u16::from(b'0')))
                }
                _ => Err(Bad),
            }
        };
        let num = |at: usize, n: usize| -> Result<u32, Bad> {
            (at..at + n).try_fold(0, |v, i| Ok(v * 10 + digit(i)?))
        };
        if u.get(8) != Some(&u16::from(b'T')) {
            return Err(Bad);
        }
        let mut time = Self {
            year: num(0, 4)? as u16,
            month: num(4, 2)? as u8,
            day: num(6, 2)? as u8,
            hour: num(9, 2)? as u8,
            minute: num(11, 2)? as u8,
            second: num(13, 2)? as u8,
            tenths: None,
            zone: None,
        };
        let mut i = 15;
        if u.get(i) == Some(&u16::from(b'.')) {
            time.tenths = Some(digit(i + 1)? as u8);
            i += 2;
        }
        match u.get(i) {
            None => {}
            Some(&c) if c == u16::from(b'Z') && u.len() == i + 1 => time.zone = Some(0),
            Some(&c) if (c == u16::from(b'+') || c == u16::from(b'-')) && u.len() == i + 5 => {
                let m = (num(i + 1, 2)? * 60 + num(i + 3, 2)?) as i16;
                time.zone = Some(if c == u16::from(b'-') { -m } else { m });
            }
            Some(_) => return Err(Bad),
        }
        if time.valid() {
            Ok(Some(time))
        } else {
            Err(Bad)
        }
    }
}

/// Writes MTP data into a buffer, or only counts it (`buf` `None`), so one
/// piece of code both sizes a dataset and fills it.
pub struct Writer<'a> {
    buf: Option<&'a mut [u8]>,
    pos: usize,
    /// Something didn't fit: the bytes written are incomplete.
    overflow: bool,
}

impl<'a> Writer<'a> {
    /// Writes into `buf`.
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self {
            buf: Some(buf),
            pos: 0,
            overflow: false,
        }
    }

    /// Only counts.
    #[must_use]
    pub fn counter() -> Writer<'static> {
        Writer {
            buf: None,
            pos: 0,
            overflow: false,
        }
    }

    /// Bytes written (or counted).
    #[must_use]
    pub fn len(&self) -> usize {
        self.pos
    }

    /// Whether nothing was written.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pos == 0
    }

    /// Room left (unbounded when counting).
    #[must_use]
    pub fn room(&self) -> usize {
        self.buf
            .as_ref()
            .map_or(usize::MAX, |b| b.len().saturating_sub(self.pos))
    }

    /// Whether something didn't fit.
    #[must_use]
    pub fn overflowed(&self) -> bool {
        self.overflow
    }

    /// Raw bytes.
    pub fn bytes(&mut self, b: &[u8]) {
        match &mut self.buf {
            None => self.pos += b.len(),
            Some(buf) => {
                if let Some(dst) = buf.get_mut(self.pos..self.pos + b.len()) {
                    dst.copy_from_slice(b);
                    self.pos += b.len();
                } else {
                    self.overflow = true;
                }
            }
        }
    }

    /// A byte.
    pub fn u8(&mut self, v: u8) {
        self.bytes(&[v]);
    }
    /// A 16-bit value.
    pub fn u16(&mut self, v: u16) {
        self.bytes(&v.to_le_bytes());
    }
    /// A 32-bit value.
    pub fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    /// A 64-bit value.
    pub fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    /// A 128-bit value.
    pub fn u128(&mut self, v: u128) {
        self.bytes(&v.to_le_bytes());
    }

    /// A string: count byte (characters with the terminator), units, NUL;
    /// the empty string is the single byte 0 [MTP 3.2.3].
    pub fn text(&mut self, t: &Text) {
        let u = t.units();
        if u.is_empty() {
            self.u8(0);
            return;
        }
        self.u8(u.len() as u8 + 1);
        for &c in u {
            self.u16(c);
        }
        self.u16(0);
    }

    /// An ASCII string.
    pub fn ascii(&mut self, s: &[u8]) {
        let s = &s[..s.len().min(STR_MAX - 1)];
        if s.is_empty() {
            self.u8(0);
            return;
        }
        self.u8(s.len() as u8 + 1);
        for &c in s {
            self.u16(u16::from(c));
        }
        self.u16(0);
    }

    /// A DateTime string (empty for `None`).
    pub fn time(&mut self, t: Option<&Time>) {
        match t {
            Some(t) => {
                let mut b = [0u8; 22];
                let n = t.format(&mut b);
                self.ascii(&b[..n]);
            }
            None => self.u8(0),
        }
    }

    /// An array of 16-bit values.
    pub fn a16(&mut self, v: &[u16]) {
        self.u32(v.len() as u32);
        for &x in v {
            self.u16(x);
        }
    }

    /// An array of 32-bit values.
    pub fn a32(&mut self, v: &[u32]) {
        self.u32(v.len() as u32);
        for &x in v {
            self.u32(x);
        }
    }
}

/// Reads MTP data from received bytes; every read is bounds-checked and
/// fails with `Err(Bad)` past the end.
pub struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Reads `b`.
    #[must_use]
    pub fn new(b: &'a [u8]) -> Self {
        Self { b, pos: 0 }
    }

    /// Bytes left.
    #[must_use]
    pub fn left(&self) -> usize {
        self.b.len() - self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Bad> {
        let s = self
            .b
            .get(self.pos..self.pos.checked_add(n).ok_or(Bad)?)
            .ok_or(Bad)?;
        self.pos += n;
        Ok(s)
    }

    /// A byte.
    ///
    /// # Errors
    /// Past the end.
    pub fn u8(&mut self) -> Result<u8, Bad> {
        Ok(self.take(1)?[0])
    }
    /// A 16-bit value.
    ///
    /// # Errors
    /// Past the end.
    pub fn u16(&mut self) -> Result<u16, Bad> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    /// A 32-bit value.
    ///
    /// # Errors
    /// Past the end.
    pub fn u32(&mut self) -> Result<u32, Bad> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    /// A 64-bit value.
    ///
    /// # Errors
    /// Past the end.
    pub fn u64(&mut self) -> Result<u64, Bad> {
        let s = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Ok(u64::from_le_bytes(a))
    }

    /// Skips `n` bytes.
    ///
    /// # Errors
    /// Past the end.
    pub fn skip(&mut self, n: usize) -> Result<(), Bad> {
        self.take(n).map(|_| ())
    }

    /// A string: the count includes the terminator, which must be there;
    /// no NUL inside [MTP 3.2.3].
    ///
    /// # Errors
    /// Past the end, a missing terminator or an embedded NUL.
    pub fn text(&mut self) -> Result<Text, Bad> {
        let n = usize::from(self.u8()?);
        if n == 0 {
            return Ok(Text::new());
        }
        let mut t = Text::new();
        for unit in &mut t.units[..n - 1] {
            match self.u16()? {
                0 => return Err(Bad),
                c => *unit = c,
            }
        }
        if self.u16()? != 0 {
            return Err(Bad);
        }
        t.len = (n - 1) as u8;
        Ok(t)
    }

    /// A value of simple type `ty` skipped (for datasets carrying fields we
    /// don't use).
    ///
    /// # Errors
    /// An unknown type or past the end.
    pub fn skip_value(&mut self, ty: u16) -> Result<(), Bad> {
        let size = |t: u16| -> Option<usize> {
            Some(match t {
                0x0001 | 0x0002 => 1,
                0x0003 | 0x0004 => 2,
                0x0005 | 0x0006 => 4,
                0x0007 | 0x0008 => 8,
                0x0009 | 0x000A => 16,
                _ => return None,
            })
        };
        if ty == 0xFFFF {
            return self.text().map(|_| ());
        }
        if let Some(n) = size(ty) {
            return self.skip(n);
        }
        if ty & 0x4000 != 0 {
            let n = size(ty & !0x4000).ok_or(Bad)?;
            let count = self.u32()? as usize;
            return self.skip(count.checked_mul(n).ok_or(Bad)?);
        }
        Err(Bad)
    }
}
