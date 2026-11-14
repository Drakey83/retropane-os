//! The calculator's arithmetic and keys (docs/design/calculator-rule0.md).
//!
//! Numbers are decimal, not binary: a whole-number mantissa and a count of
//! decimal places, so 0.1 + 0.2 is 0.3 and money adds up. Results keep
//! [`DIGITS`] significant digits, rounded half away from zero; only
//! division and very long products round at all.
//!
//! The keys build an expression with the usual precedence (2 + 3 x 4 is
//! 14), shown above the number as it is typed.

/// Significant digits a number keeps and the display shows.
pub const DIGITS: u32 = 12;
/// Terms an expression holds before it folds what it has.
const TERMS: usize = 8;

const fn pow10(n: u32) -> i128 {
    let mut p = 1i128;
    let mut k = 0;
    while k < n {
        p *= 10;
        k += 1;
    }
    p
}

/// A decimal number: `m` / 10^`scale`, at most [`DIGITS`] digits.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Dec {
    m: i64,
    scale: u8,
}

/// Why there is no result.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// A division by zero.
    DivideByZero,
    /// More than [`DIGITS`] digits before the point.
    TooLarge,
}

impl Error {
    /// In words, for the display.
    #[must_use]
    pub const fn says(self) -> &'static str {
        match self {
            Self::DivideByZero => "Cannot divide by zero",
            Self::TooLarge => "Too large to show",
        }
    }
}

fn digits(mut m: i128) -> u32 {
    let mut n = 0;
    while m != 0 {
        m /= 10;
        n += 1;
    }
    n
}

/// `m` / 10^`scale` as a [`Dec`]: rounded to [`DIGITS`] digits and places,
/// trailing zeros dropped.
fn norm(mut m: i128, mut scale: i32) -> Result<Dec, Error> {
    // A negative scale is whole tens: they go into the mantissa.
    if scale < 0 {
        if digits(m) + scale.unsigned_abs() > DIGITS {
            return Err(Error::TooLarge);
        }
        m *= pow10(scale.unsigned_abs());
        scale = 0;
    }
    // Round away what is past the digits kept, then past the places kept.
    for pass in 0..2 {
        let over = if pass == 0 {
            digits(m).saturating_sub(DIGITS).min(scale as u32)
        } else {
            (scale as u32).saturating_sub(DIGITS)
        };
        if over > 0 {
            let p = pow10(over);
            let half = if m < 0 { -(p / 2) } else { p / 2 };
            m = (m + half) / p;
            scale -= over as i32;
        }
    }
    if digits(m) > DIGITS {
        return Err(Error::TooLarge);
    }
    while scale > 0 && m % 10 == 0 {
        m /= 10;
        scale -= 1;
    }
    Ok(Dec {
        m: m as i64,
        scale: scale as u8,
    })
}

impl Dec {
    /// Zero.
    pub const ZERO: Self = Self { m: 0, scale: 0 };

    /// A whole number.
    #[must_use]
    pub const fn whole(n: i64) -> Self {
        Self { m: n, scale: 0 }
    }

    /// The number typed as digits with at most one point (`"12.50"`);
    /// `None` for anything else.
    #[must_use]
    pub fn typed(text: &[u8]) -> Option<Self> {
        let (mut m, mut scale, mut point) = (0i128, 0i32, false);
        for &b in text {
            match b {
                b'0'..=b'9' => {
                    m = m * 10 + i128::from(b - b'0');
                    scale += i32::from(point);
                }
                b'.' if !point => point = true,
                _ => return None,
            }
        }
        norm(m, scale).ok()
    }

    /// Whether it is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.m == 0
    }

    /// The same number with the other sign.
    #[must_use]
    pub const fn neg(self) -> Self {
        Self {
            m: -self.m,
            scale: self.scale,
        }
    }

    fn both(self, o: Self) -> (i128, i128, i32) {
        let s = self.scale.max(o.scale);
        (
            i128::from(self.m) * pow10(u32::from(s - self.scale)),
            i128::from(o.m) * pow10(u32::from(s - o.scale)),
            i32::from(s),
        )
    }

    /// `self + o`.
    ///
    /// # Errors
    /// [`Error::TooLarge`].
    pub fn plus(self, o: Self) -> Result<Self, Error> {
        let (a, b, s) = self.both(o);
        norm(a + b, s)
    }

    /// `self - o`.
    ///
    /// # Errors
    /// [`Error::TooLarge`].
    pub fn minus(self, o: Self) -> Result<Self, Error> {
        self.plus(o.neg())
    }

    /// `self x o`.
    ///
    /// # Errors
    /// [`Error::TooLarge`].
    pub fn times(self, o: Self) -> Result<Self, Error> {
        norm(
            i128::from(self.m) * i128::from(o.m),
            i32::from(self.scale) + i32::from(o.scale),
        )
    }

    /// `self / o`, to [`DIGITS`] digits.
    ///
    /// # Errors
    /// [`Error::DivideByZero`], [`Error::TooLarge`].
    pub fn over(self, o: Self) -> Result<Self, Error> {
        if o.m == 0 {
            return Err(Error::DivideByZero);
        }
        // Twenty more places than needed, so the one rounding in `norm`
        // is the only one that shows.
        const MORE: u32 = 20;
        let q = i128::from(self.m) * pow10(MORE) / i128::from(o.m);
        norm(q, i32::from(self.scale) - i32::from(o.scale) + MORE as i32)
    }

    /// The number as text into `out` (digits grouped in threes before the
    /// point with `group`, 0 for none): its length.
    pub fn show(self, group: u8, out: &mut [u8; 32]) -> usize {
        let mut digits = [0u8; 20];
        let mut n = 0;
        let mut m = self.m.unsigned_abs();
        while m != 0 || n <= usize::from(self.scale) {
            digits[n] = b'0' + (m % 10) as u8;
            m /= 10;
            n += 1;
        }
        let mut at = 0;
        let mut put = |b: u8| {
            out[at] = b;
            at += 1;
        };
        if self.m < 0 {
            put(b'-');
        }
        let whole = n - usize::from(self.scale);
        for k in (0..n).rev() {
            put(digits[k]);
            let left = k - usize::from(self.scale).min(k);
            if k >= usize::from(self.scale) && left > 0 && left % 3 == 0 && group != 0 && whole > 3
            {
                put(group);
            }
            if k == usize::from(self.scale) && k > 0 {
                put(b'.');
            }
        }
        at
    }
}

/// The four operations.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    /// Plus.
    Add,
    /// Minus.
    Sub,
    /// Times.
    Mul,
    /// Divided by.
    Div,
}

impl Op {
    /// The sign shown.
    #[must_use]
    pub const fn sign(self) -> u8 {
        match self {
            Self::Add => b'+',
            Self::Sub => b'-',
            Self::Mul => b'x',
            Self::Div => b'/',
        }
    }

    fn apply(self, a: Dec, b: Dec) -> Result<Dec, Error> {
        match self {
            Self::Add => a.plus(b),
            Self::Sub => a.minus(b),
            Self::Mul => a.times(b),
            Self::Div => a.over(b),
        }
    }

    const fn binds(self) -> bool {
        matches!(self, Self::Mul | Self::Div)
    }
}

/// A calculator key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// 0-9.
    Digit(u8),
    /// The decimal point.
    Point,
    /// An operation.
    Op(Op),
    /// Equals.
    Equals,
    /// Clear: everything.
    Clear,
    /// Delete: the last digit typed.
    Delete,
    /// Plus or minus: the number's sign.
    Negate,
    /// Percent.
    Percent,
}

/// What the display shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shown {
    /// The number being typed (`entry`), possibly none yet.
    Entry,
    /// A result: the next digit starts a new sum, an operation goes on
    /// from it.
    Result(Dec),
    /// No result: Clear or a digit starts again.
    Error(Error),
}

/// The calculator: the expression so far and the number being typed.
#[derive(Clone, Copy, Debug)]
pub struct Calc {
    terms: [(Dec, Op); TERMS],
    n: usize,
    entry: [u8; 16],
    len: usize,
    negative: bool,
    shown: Shown,
    /// The finished expression's text for the line above the result.
    last: [u8; 96],
    last_len: usize,
}

impl Default for Calc {
    fn default() -> Self {
        Self::new()
    }
}

impl Calc {
    /// A cleared calculator.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            terms: [(Dec::ZERO, Op::Add); TERMS],
            n: 0,
            entry: [0; 16],
            len: 0,
            negative: false,
            shown: Shown::Entry,
            last: [0; 96],
            last_len: 0,
        }
    }

    fn entry_value(&self) -> Option<Dec> {
        if self.len == 0 {
            return None;
        }
        let v = Dec::typed(&self.entry[..self.len])?;
        Some(if self.negative { v.neg() } else { v })
    }

    fn set_entry(&mut self, v: Dec) {
        let mut out = [0u8; 32];
        let n = v.show(0, &mut out);
        let text = &out[..n];
        self.negative = text.first() == Some(&b'-');
        let text = &text[usize::from(self.negative)..];
        self.len = text.len().min(self.entry.len());
        self.entry[..self.len].copy_from_slice(&text[..self.len]);
        self.shown = Shown::Entry;
    }

    /// The terms and `last` as one value: times and divided-by first.
    fn value(terms: &[(Dec, Op)], last: Dec) -> Result<Dec, Error> {
        let mut sum = Dec::ZERO;
        let mut sign = Op::Add;
        // The product being built, and the operation after it.
        let mut product: Option<(Dec, Op)> = None;
        for &(v, op) in terms.iter().chain(&[(last, Op::Add)]) {
            let v = match product {
                Some((p, o)) => o.apply(p, v)?,
                None => v,
            };
            if op.binds() {
                product = Some((v, op));
            } else {
                sum = sign.apply(sum, v)?;
                sign = op;
                product = None;
            }
        }
        Ok(sum)
    }

    fn fail(&mut self, e: Error) {
        self.n = 0;
        self.len = 0;
        self.negative = false;
        self.shown = Shown::Error(e);
    }

    fn expression(&self, last: Option<Dec>, out: &mut [u8; 96]) -> usize {
        let mut at = 0;
        let mut put = |s: &[u8]| {
            let k = s.len().min(out.len() - at);
            out[at..at + k].copy_from_slice(&s[..k]);
            at += k;
        };
        let mut num = [0u8; 32];
        for &(v, op) in &self.terms[..self.n] {
            let k = v.show(b',', &mut num);
            put(&num[..k]);
            put(&[b' ', op.sign(), b' ']);
        }
        if let Some(v) = last {
            let k = v.show(b',', &mut num);
            put(&num[..k]);
            put(b" =");
        }
        at
    }

    /// A key pressed.
    pub fn press(&mut self, key: Key) {
        if let Shown::Error(_) = self.shown {
            // Only Clear or a new number leaves an error.
            match key {
                Key::Clear | Key::Digit(_) | Key::Point => *self = Self::new(),
                _ => return,
            }
        }
        match key {
            Key::Clear => *self = Self::new(),
            Key::Digit(d) => {
                if let Shown::Result(_) = self.shown {
                    *self = Self::new();
                }
                let digits = self.entry[..self.len]
                    .iter()
                    .filter(|b| **b != b'.')
                    .count();
                if &self.entry[..self.len] == b"0" {
                    self.len = 0;
                }
                if digits < DIGITS as usize && d <= 9 {
                    self.entry[self.len] = b'0' + d;
                    self.len += 1;
                }
            }
            Key::Point => {
                if let Shown::Result(_) = self.shown {
                    *self = Self::new();
                }
                if !self.entry[..self.len].contains(&b'.') && self.len + 2 <= self.entry.len() {
                    if self.len == 0 {
                        self.entry[0] = b'0';
                        self.len = 1;
                    }
                    self.entry[self.len] = b'.';
                    self.len += 1;
                }
            }
            Key::Delete => match self.shown {
                Shown::Result(_) => *self = Self::new(),
                _ => {
                    self.len = self.len.saturating_sub(1);
                    if self.len == 0 {
                        self.negative = false;
                    }
                }
            },
            Key::Negate => {
                if let Shown::Result(v) = self.shown {
                    self.n = 0;
                    self.last_len = 0;
                    self.set_entry(v.neg());
                } else if self.len > 0 {
                    self.negative = !self.negative;
                }
            }
            Key::Percent => {
                let v = match self.shown {
                    Shown::Result(v) => {
                        self.n = 0;
                        self.last_len = 0;
                        Some(v)
                    }
                    _ => self.entry_value(),
                };
                let Some(v) = v else { return };
                let hundred = Dec::whole(100);
                // After plus or minus it is a share of what came before
                // (200 + 10 % is 220); otherwise a hundredth.
                let r = match self.terms[..self.n].last() {
                    Some((_, Op::Add | Op::Sub)) => {
                        let (init, last) = self.terms[..self.n].split_at(self.n - 1);
                        Self::value(init, last[0].0)
                            .and_then(|base| base.times(v))
                            .and_then(|x| x.over(hundred))
                    }
                    _ => v.over(hundred),
                };
                match r {
                    Ok(x) => self.set_entry(x),
                    Err(e) => self.fail(e),
                }
            }
            Key::Op(op) => {
                let v = match self.shown {
                    Shown::Result(v) => {
                        self.n = 0;
                        Some(v)
                    }
                    _ => self.entry_value(),
                };
                match v {
                    Some(v) => {
                        if self.n == TERMS {
                            // Full: what is there becomes one number.
                            match Self::value(&self.terms, v) {
                                Ok(x) => {
                                    self.terms[0] = (x, op);
                                    self.n = 1;
                                }
                                Err(e) => return self.fail(e),
                            }
                        } else {
                            self.terms[self.n] = (v, op);
                            self.n += 1;
                        }
                    }
                    // No number yet: the operation replaces the last one,
                    // or starts from zero.
                    None => match self.n {
                        0 => {
                            self.terms[0] = (Dec::ZERO, op);
                            self.n = 1;
                        }
                        n => self.terms[n - 1].1 = op,
                    },
                }
                self.len = 0;
                self.negative = false;
                self.last_len = 0;
                self.shown = Shown::Entry;
            }
            Key::Equals => {
                let last = match (self.shown, self.entry_value()) {
                    (Shown::Result(_), _) => return,
                    (_, Some(v)) => v,
                    // "5 + =" is 5 + 5, as pocket calculators do.
                    (_, None) => match self.terms[..self.n].last() {
                        Some(&(v, _)) => v,
                        None => return,
                    },
                };
                let mut text = [0u8; 96];
                let k = self.expression(Some(last), &mut text);
                match Self::value(&self.terms[..self.n], last) {
                    Ok(v) => {
                        self.last = text;
                        self.last_len = k;
                        self.n = 0;
                        self.len = 0;
                        self.negative = false;
                        self.shown = Shown::Result(v);
                    }
                    Err(e) => self.fail(e),
                }
            }
        }
    }

    /// The big line: the number being typed, the result, or zero; `Err`
    /// with the words when there is no result.
    ///
    /// # Errors
    /// The error shown.
    pub fn number(&self, out: &mut [u8; 32]) -> Result<usize, Error> {
        match self.shown {
            Shown::Error(e) => Err(e),
            Shown::Result(v) => Ok(v.show(b',', out)),
            Shown::Entry => {
                if self.len == 0 {
                    out[0] = b'0';
                    return Ok(1);
                }
                // As typed (a trailing point and zeros stay), grouped.
                let whole = self.entry[..self.len]
                    .iter()
                    .position(|b| *b == b'.')
                    .unwrap_or(self.len);
                let mut at = 0;
                if self.negative {
                    out[0] = b'-';
                    at = 1;
                }
                for (k, &b) in self.entry[..self.len].iter().enumerate() {
                    out[at] = b;
                    at += 1;
                    if k + 1 < whole && (whole - k - 1) % 3 == 0 {
                        out[at] = b',';
                        at += 1;
                    }
                }
                Ok(at)
            }
        }
    }

    /// The small line above: the expression so far, or the finished one.
    pub fn line(&self, out: &mut [u8; 96]) -> usize {
        if let Shown::Result(_) = self.shown {
            *out = self.last;
            return self.last_len;
        }
        self.expression(None, out)
    }
}

#[cfg(test)]
#[path = "calc_tests.rs"]
mod tests;
