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

/// 10^0 to 10^12.
const POW10: [u64; 13] = {
    let mut t = [1u64; 13];
    let mut k = 1;
    while k < 13 {
        t[k] = t[k - 1] * 10;
        k += 1;
    }
    t
};
/// 10^12: the base of a [`Wide`]'s two parts.
const BASE: u64 = POW10[12];

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
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub const fn says(self) -> &'static str {
        match self {
            Self::DivideByZero => "Cannot divide by zero",
            Self::TooLarge => "Too large to show",
        }
    }
}

/// A magnitude too long for one word while it is worked on: `hi` x 10^12
/// plus `lo`, `lo` under 10^12. Everything is 64-bit arithmetic in tens: the
/// PXA270 has no wide divide, and the library's 128-bit one cost the
/// image more than this whole file.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Wide {
    hi: u64,
    lo: u64,
}

#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn digits(mut m: u64) -> u32 {
    let mut n = 0;
    while m != 0 {
        m /= 10;
        n += 1;
    }
    n
}

impl Wide {
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    const fn of(m: u64) -> Self {
        Self {
            hi: m / BASE,
            lo: m % BASE,
        }
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn digits(self) -> u32 {
        if self.hi == 0 {
            digits(self.lo)
        } else {
            12 + digits(self.hi)
        }
    }

    /// `m` (under 10^12) x 10^`k` (`k` at most 12).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn shifted(m: u64, k: u32) -> Self {
        let cut = POW10[(12 - k) as usize];
        Self {
            hi: m / cut,
            lo: m % cut * POW10[k as usize],
        }
    }

    /// Divided by 10^`k` (`k` at most 12), rounded half away from zero.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn dropped(self, k: u32) -> Self {
        if k == 0 {
            return self;
        }
        let p = POW10[k as usize];
        let rem = self.lo % p;
        let mut lo = self.hi % p * POW10[(12 - k) as usize] + self.lo / p;
        let mut hi = self.hi / p;
        if rem * 2 >= p {
            lo += 1;
            if lo == BASE {
                lo = 0;
                hi += 1;
            }
        }
        Self { hi, lo }
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn plus(self, o: Self) -> Self {
        let lo = self.lo + o.lo;
        Self {
            hi: self.hi + o.hi + lo / BASE,
            lo: lo % BASE,
        }
    }

    /// `self - o` (`o` no larger).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn minus(self, o: Self) -> Self {
        let borrow = u64::from(self.lo < o.lo);
        Self {
            hi: self.hi - o.hi - borrow,
            lo: self.lo + borrow * BASE - o.lo,
        }
    }
}

/// `m` / 10^`scale` (negative when `neg`) as a [`Dec`]: rounded to
/// [`DIGITS`] digits and places, trailing zeros dropped.
#[inline(never)]
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn norm(neg: bool, mut m: Wide, mut scale: i32) -> Result<Dec, Error> {
    // A negative scale is whole tens: they go into the mantissa.
    if scale < 0 {
        let up = scale.unsigned_abs();
        if m.digits() + up > DIGITS {
            return Err(Error::TooLarge);
        }
        m = Wide::of(m.lo * POW10[up as usize]);
        scale = 0;
    }
    // Round away what is past the digits kept, then past the places kept
    // (at most 12 at a step: dropping the first part unrounded can't
    // change which way the last part rounds).
    loop {
        let over = m
            .digits()
            .saturating_sub(DIGITS)
            .min(scale as u32)
            .max((scale as u32).saturating_sub(DIGITS));
        if over == 0 {
            break;
        }
        let step = over.min(12);
        m = if step < over {
            Wide {
                hi: 0,
                lo: m.hi % POW10[step as usize] * POW10[(12 - step) as usize]
                    + m.lo / POW10[step as usize],
            }
            .plus(Wide {
                hi: m.hi / POW10[step as usize],
                lo: 0,
            })
        } else {
            m.dropped(step)
        };
        scale -= step as i32;
    }
    if m.digits() > DIGITS {
        return Err(Error::TooLarge);
    }
    let mut v = m.lo;
    while scale > 0 && v.is_multiple_of(10) {
        v /= 10;
        scale -= 1;
    }
    let v = v as i64;
    Ok(Dec {
        m: if neg { -v } else { v },
        scale: scale as u8,
    })
}

impl Dec {
    /// Zero.
    pub const ZERO: Self = Self { m: 0, scale: 0 };

    /// A whole number.
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub const fn whole(n: i64) -> Self {
        Self { m: n, scale: 0 }
    }

    /// The number typed as digits with at most one point (`"12.50"`);
    /// `None` for anything else.
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn typed(text: &[u8]) -> Option<Self> {
        let (mut m, mut scale, mut point) = (0u64, 0i32, false);
        for &b in text {
            match b {
                b'0'..=b'9' => {
                    m = m.checked_mul(10)?.checked_add(u64::from(b - b'0'))?;
                    scale += i32::from(point);
                }
                b'.' if !point => point = true,
                _ => return None,
            }
        }
        norm(false, Wide::of(m), scale).ok()
    }

    /// Whether it is zero.
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub const fn is_zero(self) -> bool {
        self.m == 0
    }

    /// The same number with the other sign.
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub const fn neg(self) -> Self {
        Self {
            m: -self.m,
            scale: self.scale,
        }
    }

    /// `self + o`.
    ///
    /// # Errors
    /// [`Error::TooLarge`].
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn plus(self, o: Self) -> Result<Self, Error> {
        // Both at the larger scale, then by their signs.
        let s = self.scale.max(o.scale);
        let a = Wide::shifted(self.m.unsigned_abs(), u32::from(s - self.scale));
        let b = Wide::shifted(o.m.unsigned_abs(), u32::from(s - o.scale));
        let (an, bn) = (self.m < 0, o.m < 0);
        if an == bn {
            norm(an, a.plus(b), i32::from(s))
        } else if a >= b {
            norm(an, a.minus(b), i32::from(s))
        } else {
            norm(bn, b.minus(a), i32::from(s))
        }
    }

    /// `self - o`.
    ///
    /// # Errors
    /// [`Error::TooLarge`].
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn minus(self, o: Self) -> Result<Self, Error> {
        self.plus(o.neg())
    }

    /// `self x o`.
    ///
    /// # Errors
    /// [`Error::TooLarge`].
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn times(self, o: Self) -> Result<Self, Error> {
        // Each in two parts of six digits: no partial product passes
        // 10^12.
        const HALF: u64 = POW10[6];
        let (a, b) = (self.m.unsigned_abs(), o.m.unsigned_abs());
        let (a1, a0, b1, b0) = (a / HALF, a % HALF, b / HALF, b % HALF);
        let mid = a1 * b0 + a0 * b1;
        let p = Wide {
            hi: a1 * b1 + mid / HALF,
            lo: mid % HALF * HALF,
        }
        .plus(Wide::of(a0 * b0));
        norm(
            (self.m < 0) != (o.m < 0),
            p,
            i32::from(self.scale) + i32::from(o.scale),
        )
    }

    /// `self / o`, to [`DIGITS`] digits.
    ///
    /// # Errors
    /// [`Error::DivideByZero`], [`Error::TooLarge`].
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn over(self, o: Self) -> Result<Self, Error> {
        if o.m == 0 {
            return Err(Error::DivideByZero);
        }
        // Long division in tens: the whole part, then digit after digit
        // until fourteen that count are had (two more than are kept, so
        // the one rounding in `norm` is the only one that shows).
        let (a, b) = (self.m.unsigned_abs(), o.m.unsigned_abs());
        let (mut q, mut r) = (a / b, a % b);
        let mut places = 0i32;
        let mut had = digits(q);
        while r != 0 && had < DIGITS + 2 && places < 40 {
            r *= 10;
            q = q * 10 + r / b;
            r %= b;
            places += 1;
            had += u32::from(q != 0);
        }
        norm(
            (self.m < 0) != (o.m < 0),
            Wide::of(q),
            i32::from(self.scale) - i32::from(o.scale) + places,
        )
    }

    /// The number as text into `out` (digits grouped in threes before the
    /// point with `group`, 0 for none): its length.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
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
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub const fn sign(self) -> u8 {
        match self {
            Self::Add => b'+',
            Self::Sub => b'-',
            Self::Mul => b'x',
            Self::Div => b'/',
        }
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn apply(self, a: Dec, b: Dec) -> Result<Dec, Error> {
        match self {
            Self::Add => a.plus(b),
            Self::Sub => a.minus(b),
            Self::Mul => a.times(b),
            Self::Div => a.over(b),
        }
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
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
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn default() -> Self {
        Self::new()
    }
}

impl Calc {
    /// A cleared calculator.
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
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

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn entry_value(&self) -> Option<Dec> {
        if self.len == 0 {
            return None;
        }
        let v = Dec::typed(&self.entry[..self.len])?;
        Some(if self.negative { v.neg() } else { v })
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
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
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
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

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn fail(&mut self, e: Error) {
        self.n = 0;
        self.len = 0;
        self.negative = false;
        self.shown = Shown::Error(e);
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
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
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
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
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
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
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
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
