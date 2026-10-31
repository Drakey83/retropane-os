//! A small JSON reader for the weather sources' answers (Tempest,
//! Open-Meteo, MET Norway, the NWS): no allocation, one pass, numbers as
//! fixed-point thousandths, nesting bounded. Their answers come over a
//! cable from a relay today and from the internet later, so every reader
//! built on it treats them as untrusted (range-checks every number, leaves
//! missing fields as they were).

/// A cursor over JSON bytes.
#[derive(Clone, Copy)]
pub(crate) struct Json<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Json<'a> {
    /// At the start of `b`.
    pub(crate) const fn new(b: &'a [u8]) -> Self {
        Self { b, i: 0 }
    }

    pub(crate) fn ws(&mut self) {
        while self.b.get(self.i).is_some_and(u8::is_ascii_whitespace) {
            self.i += 1;
        }
    }

    pub(crate) fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.b.get(self.i).copied()
    }

    pub(crate) fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    /// A string's raw bytes (escapes left as they are: the fields read are
    /// plain words).
    pub(crate) fn string(&mut self) -> Option<&'a [u8]> {
        if !self.eat(b'"') {
            return None;
        }
        let start = self.i;
        while let Some(&c) = self.b.get(self.i) {
            match c {
                b'\\' => self.i += 2,
                b'"' => {
                    self.i += 1;
                    return self.b.get(start..self.i - 1);
                }
                _ => self.i += 1,
            }
        }
        None
    }

    /// A number, in thousandths (`17.25` is 17,250), clamped to `i32`.
    pub(crate) fn number(&mut self) -> Option<i64> {
        self.ws();
        let neg = self.b.get(self.i) == Some(&b'-');
        if neg {
            self.i += 1;
        }
        let (mut v, mut frac, mut digits) = (0i64, -1i32, 0);
        while let Some(&c) = self.b.get(self.i) {
            match c {
                b'0'..=b'9' => {
                    if frac < 3 && v < 1 << 50 {
                        v = v * 10 + i64::from(c - b'0');
                        if frac >= 0 {
                            frac += 1;
                        }
                    }
                    digits += 1;
                }
                b'.' if frac < 0 => frac = 0,
                b'e' | b'E' | b'+' => return None,
                _ => break,
            }
            self.i += 1;
        }
        if digits == 0 {
            return None;
        }
        let mut f = frac.max(0);
        while f < 3 {
            v *= 10;
            f += 1;
        }
        Some(if neg { -v } else { v })
    }

    /// Skips one value of any kind (nesting bounded).
    pub(crate) fn skip(&mut self) -> Option<()> {
        match self.peek()? {
            b'"' => self.string().map(|_| ()),
            b'{' | b'[' => {
                let mut depth = 0u32;
                loop {
                    match self.peek()? {
                        b'"' => {
                            self.string()?;
                            continue;
                        }
                        b'{' | b'[' => depth += 1,
                        b'}' | b']' => depth -= 1,
                        _ => {}
                    }
                    self.i += 1;
                    if depth == 0 {
                        return Some(());
                    }
                    if depth > 64 {
                        return None;
                    }
                }
            }
            b't' | b'f' | b'n' => {
                while self.b.get(self.i).is_some_and(u8::is_ascii_alphabetic) {
                    self.i += 1;
                }
                Some(())
            }
            _ => self.number().map(|_| ()),
        }
    }

    /// Each member of the object here: `f(key, cursor at its value)`; the
    /// value is skipped if `f` leaves it.
    pub(crate) fn object(&mut self, mut f: impl FnMut(&'a [u8], &mut Json<'a>)) -> Option<()> {
        self.object_dyn(&mut f)
    }

    /// [`Json::object`]'s one copy: each caller's closure is called through
    /// a pointer, so the walk isn't compiled again per closure (11 copies,
    /// 16.9 KB of the X51v image, 2026-10-31).
    #[inline(never)]
    fn object_dyn(&mut self, f: &mut dyn FnMut(&'a [u8], &mut Json<'a>)) -> Option<()> {
        if !self.eat(b'{') {
            return None;
        }
        if self.eat(b'}') {
            return Some(());
        }
        loop {
            let key = self.string()?;
            if !self.eat(b':') {
                return None;
            }
            let mut v = *self;
            f(key, &mut v);
            self.skip()?;
            if self.eat(b'}') {
                return Some(());
            }
            if !self.eat(b',') {
                return None;
            }
        }
    }

    /// Each element of the array here: `f(index, cursor at it)`.
    pub(crate) fn array(&mut self, mut f: impl FnMut(usize, &mut Json<'a>)) -> Option<()> {
        self.array_dyn(&mut f)
    }

    /// [`Json::array`]'s one copy (as [`Json::object_dyn`]).
    #[inline(never)]
    fn array_dyn(&mut self, f: &mut dyn FnMut(usize, &mut Json<'a>)) -> Option<()> {
        if !self.eat(b'[') {
            return None;
        }
        if self.eat(b']') {
            return Some(());
        }
        let mut k = 0;
        loop {
            let mut v = *self;
            f(k, &mut v);
            self.skip()?;
            k += 1;
            if self.eat(b']') {
                return Some(());
            }
            if !self.eat(b',') {
                return None;
            }
        }
    }
}

/// Tenths of a unit from thousandths, in `lo..=hi` tenths, or `None`.
pub(crate) fn tenths(v: Option<i64>, lo: i64, hi: i64) -> Option<i16> {
    let t = v?.div_euclid(100);
    (lo..=hi).contains(&t).then_some(t as i16)
}

/// A whole number from thousandths, in `lo..=hi`.
pub(crate) fn whole(v: Option<i64>, lo: i64, hi: i64) -> Option<i64> {
    let w = v?.div_euclid(1000);
    (lo..=hi).contains(&w).then_some(w)
}

/// The local day number (days since 1970) and minute of the day of a Unix
/// time at `offset` minutes from UTC.
pub(crate) fn local(epoch: i64, offset: i64) -> (i64, u16) {
    let s = epoch + offset * 60;
    (s.div_euclid(86_400), (s.rem_euclid(86_400) / 60) as u16)
}

