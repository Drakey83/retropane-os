//! The calculator's sums as typed (calculator-rule0.md version 2): an
//! expression of numbers, `+ - x /`, brackets, `%`, powers and functions,
//! worked with school precedence on [`Dec`] (exact decimals: 0.1 + 0.2 is
//! 0.3), and the scientific functions on our own series.
//!
//! Found: Windows Mobile's calculator worked each key as it came, in
//! binary floats; phones evaluate the whole line with precedence; libm
//! gives sin and ln in double precision but is a dependency built for
//! machines with an FPU. The X51v has none: ours keeps + - x / exact in
//! decimal, and computes the transcendental functions in software doubles
//! with range reduction and short series (accurate to the 12 digits the
//! display shows), only when such a key is used.
//!
//! The text is ASCII: digits and `.`, `+ - x /`, `( )`, `%`, `^`, `!`,
//! `~` (negate), `p` (pi), `e`, and the functions `s c t S C T l L q`
//! (sin, cos, tan, their inverses, ln, log10, square root), each followed
//! by its bracketed argument. Angles are degrees unless `radians`.

use crate::calc::{Dec, Error};

/// Why an expression has no value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fault {
    /// The arithmetic failed (division by zero, too large).
    Math(Error),
    /// Not a whole expression yet (an open bracket, a trailing sign).
    Incomplete,
    /// Outside a function's domain (ln of a negative, sqrt of -1).
    Domain,
}

/// The longest expression worked.
pub const MAX: usize = 96;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tok {
    Num(Dec),
    Op(u8),
    Open,
    Close,
    Func(u8),
    Percent,
    Fact,
}

fn prec(op: u8) -> u8 {
    match op {
        b'+' | b'-' => 1,
        b'x' | b'/' => 2,
        b'~' => 3,
        b'^' => 4,
        _ => 0,
    }
}

fn to_f(d: Dec) -> f64 {
    let mut b = [0u8; 32];
    let n = d.show(0, &mut b);
    let (mut v, mut frac, mut seen, mut neg) = (0f64, 1f64, false, false);
    for &c in &b[..n] {
        match c {
            b'-' => neg = true,
            b'.' => seen = true,
            b'0'..=b'9' => {
                if seen {
                    frac /= 10.0;
                    v += f64::from(c - b'0') * frac;
                } else {
                    v = v * 10.0 + f64::from(c - b'0');
                }
            }
            _ => {}
        }
    }
    if neg { -v } else { v }
}

/// `v` to 12 significant digits as a [`Dec`].
fn from_f(v: f64) -> Result<Dec, Fault> {
    if v.is_nan() {
        return Err(Fault::Domain);
    }
    if !v.is_finite() || v.abs() >= 1e12 {
        return Err(Fault::Math(Error::TooLarge));
    }
    if v.abs() < 1e-12 {
        return Ok(Dec::ZERO);
    }
    // Digits after the point so the whole keeps 12 significant ones.
    let mut mag = 0i32;
    let mut a = v.abs();
    while a >= 1.0 {
        a /= 10.0;
        mag += 1;
    }
    let scale = (12 - mag).clamp(0, 11) as u32;
    let m = round(v * pow10(scale));
    let mut t = [0u8; 40];
    let mut n = 0;
    if m < 0 {
        t[n] = b'-';
        n += 1;
    }
    let mut digits = [0u8; 20];
    let mut k = 0;
    let mut u = m.unsigned_abs();
    while u > 0 || k <= scale as usize {
        digits[k] = b'0' + (u % 10) as u8;
        u /= 10;
        k += 1;
    }
    for i in (0..k).rev() {
        t[n] = digits[i];
        n += 1;
        if i == scale as usize && scale > 0 {
            t[n] = b'.';
            n += 1;
        }
    }
    let neg = t[0] == b'-';
    let body = if neg { &t[1..n] } else { &t[..n] };
    let d = Dec::typed(body).ok_or(Fault::Math(Error::TooLarge))?;
    Ok(if neg {
        Dec::ZERO.minus(d).map_err(Fault::Math)?
    } else {
        d
    })
}

fn pow10(n: u32) -> f64 {
    let mut p = 1.0;
    for _ in 0..n {
        p *= 10.0;
    }
    p
}

fn round(v: f64) -> i64 {
    if v >= 0.0 {
        (v + 0.5) as i64
    } else {
        (v - 0.5) as i64
    }
}

const PI: f64 = core::f64::consts::PI;
const LN2: f64 = core::f64::consts::LN_2;
const LN10: f64 = core::f64::consts::LN_10;

/// sin of `x` radians: reduced to [-pi/4, pi/4] by quarter turns, then
/// the Taylor series to x^15 (error under 1e-16 there).
fn sin(x: f64) -> f64 {
    let q = round(x / (PI / 2.0));
    let r = x - q as f64 * (PI / 2.0);
    match q.rem_euclid(4) {
        0 => sin_small(r),
        1 => cos_small(r),
        2 => -sin_small(r),
        _ => -cos_small(r),
    }
}

fn cos(x: f64) -> f64 {
    sin(x + PI / 2.0)
}

fn sin_small(x: f64) -> f64 {
    let (x2, mut term, mut sum) = (x * x, x, x);
    for k in 1..8 {
        term *= -x2 / f64::from((2 * k) * (2 * k + 1));
        sum += term;
    }
    sum
}

fn cos_small(x: f64) -> f64 {
    let (x2, mut term, mut sum) = (x * x, 1.0, 1.0);
    for k in 1..9 {
        term *= -x2 / f64::from((2 * k - 1) * (2 * k));
        sum += term;
    }
    sum
}

/// ln of `x` > 0: x = m 2^k with m in [0.75, 1.5), then
/// ln m = 2 atanh((m-1)/(m+1)), whose series converges fast there.
fn ln(x: f64) -> f64 {
    let (mut m, mut k) = (x, 0i32);
    while m >= 1.5 {
        m /= 2.0;
        k += 1;
    }
    while m < 0.75 {
        m *= 2.0;
        k -= 1;
    }
    let z = (m - 1.0) / (m + 1.0);
    let (z2, mut term, mut sum) = (z * z, z, z);
    for n in 1..20 {
        term *= z2;
        sum += term / f64::from(2 * n + 1);
    }
    2.0 * sum + f64::from(k) * LN2
}

/// e^x: x = k ln2 + r with |r| <= ln2/2, e^r by its series, times 2^k.
fn exp(x: f64) -> f64 {
    let k = round(x / LN2);
    let r = x - k as f64 * LN2;
    let (mut term, mut sum) = (1.0, 1.0);
    for n in 1..20 {
        term *= r / f64::from(n);
        sum += term;
    }
    let mut v = sum;
    let mut k = k;
    while k > 0 {
        v *= 2.0;
        k -= 1;
    }
    while k < 0 {
        v /= 2.0;
        k += 1;
    }
    v
}

fn sqrt(x: f64) -> f64 {
    if x == 0.0 {
        return 0.0;
    }
    let mut g = if x > 1.0 { x / 2.0 } else { 1.0 };
    for _ in 0..60 {
        g = 0.5 * (g + x / g);
    }
    g
}

/// atan by halving the angle until small, then its series.
fn atan(x: f64) -> f64 {
    let (mut v, mut doubled) = (x, 0);
    while v.abs() > 0.2 {
        v /= 1.0 + sqrt(1.0 + v * v);
        doubled += 1;
    }
    let (v2, mut term, mut sum) = (v * v, v, v);
    for n in 1..15 {
        term *= -v2;
        sum += term / f64::from(2 * n + 1);
    }
    sum * f64::from(1 << doubled)
}

fn function(f: u8, a: Dec, radians: bool) -> Result<Dec, Fault> {
    let x = to_f(a);
    let ang = |v: f64| if radians { v } else { v * PI / 180.0 };
    let deg = |v: f64| if radians { v } else { v * 180.0 / PI };
    let v = match f {
        b's' => sin(ang(x)),
        b'c' => cos(ang(x)),
        b't' => {
            let c = cos(ang(x));
            if c.abs() < 1e-15 {
                return Err(Fault::Domain);
            }
            sin(ang(x)) / c
        }
        b'S' | b'C' => {
            if x.abs() > 1.0 {
                return Err(Fault::Domain);
            }
            let s = atan(x / sqrt(1.0 - x * x).max(1e-300));
            let r = if x.abs() == 1.0 { x * PI / 2.0 } else { s };
            deg(if f == b'S' { r } else { PI / 2.0 - r })
        }
        b'T' => deg(atan(x)),
        b'l' | b'L' => {
            if x <= 0.0 {
                return Err(Fault::Domain);
            }
            if f == b'l' { ln(x) } else { ln(x) / LN10 }
        }
        b'q' => {
            if x < 0.0 {
                return Err(Fault::Domain);
            }
            sqrt(x)
        }
        _ => return Err(Fault::Domain),
    };
    // sin 180 is 0, not 1.2e-16: the 12 digits shown decide.
    from_f(v)
}

fn power(a: Dec, b: Dec) -> Result<Dec, Fault> {
    let (x, y) = (to_f(a), to_f(b));
    // Whole powers stay exact.
    if y == round(y) as f64 && y.abs() <= 64.0 {
        let mut r = Dec::whole(1);
        for _ in 0..(y.abs() as i64) {
            r = r.times(a).map_err(Fault::Math)?;
        }
        return if y < 0.0 {
            Dec::whole(1).over(r).map_err(Fault::Math)
        } else {
            Ok(r)
        };
    }
    if x <= 0.0 {
        return Err(Fault::Domain);
    }
    from_f(exp(y * ln(x)))
}

fn factorial(a: Dec) -> Result<Dec, Fault> {
    let x = to_f(a);
    if x < 0.0 || x != round(x) as f64 || x > 20.0 {
        return Err(if x > 20.0 {
            Fault::Math(Error::TooLarge)
        } else {
            Fault::Domain
        });
    }
    let mut r = Dec::whole(1);
    for k in 2..=(x as i64) {
        r = r.times(Dec::whole(k)).map_err(Fault::Math)?;
    }
    Ok(r)
}

fn apply(op: u8, a: Dec, b: Dec) -> Result<Dec, Fault> {
    match op {
        b'+' => a.plus(b).map_err(Fault::Math),
        b'-' => a.minus(b).map_err(Fault::Math),
        b'x' => a.times(b).map_err(Fault::Math),
        b'/' => a.over(b).map_err(Fault::Math),
        b'^' => power(a, b),
        _ => Err(Fault::Incomplete),
    }
}

/// The token `text` holds at `at` (stepped past it; spaces skipped):
/// `None` at its end. Read as they are used, not kept in a list: the list
/// was 2.3 KB of a 4.2 KB frame, over the stack rule's 4,096.
fn token(text: &[u8], at: &mut usize) -> Result<Option<Tok>, Fault> {
    while text.get(*at) == Some(&b' ') {
        *at += 1;
    }
    let Some(&c) = text.get(*at) else {
        return Ok(None);
    };
    let t = match c {
        b'0'..=b'9' | b'.' => {
            let start = *at;
            while text
                .get(*at)
                .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
            {
                *at += 1;
            }
            let d = Dec::typed(&text[start..*at]).ok_or(Fault::Incomplete)?;
            return Ok(Some(Tok::Num(d)));
        }
        b'p' => Tok::Num(Dec::typed(b"3.14159265359").unwrap_or(Dec::ZERO)),
        b'e' => Tok::Num(Dec::typed(b"2.71828182846").unwrap_or(Dec::ZERO)),
        b'+' | b'-' | b'x' | b'/' | b'^' | b'~' => Tok::Op(c),
        b'(' => Tok::Open,
        b')' => Tok::Close,
        b'%' => Tok::Percent,
        b'!' => Tok::Fact,
        b's' | b'c' | b't' | b'S' | b'C' | b'T' | b'l' | b'L' | b'q' => Tok::Func(c),
        _ => return Err(Fault::Incomplete),
    };
    *at += 1;
    Ok(Some(t))
}

/// The value of `text` (see the module), angles in degrees unless
/// `radians`. `200 + 10 %` is 220: a percent after `+` or `-` is of the
/// number before it, otherwise it is hundredths.
pub fn eval(text: &[u8], radians: bool) -> Result<Dec, Fault> {
    // Every token reads, before any is worked (a sum with a letter it
    // does not know is incomplete whatever comes before the letter).
    let mut at = 0;
    while token(text, &mut at)?.is_some() {}
    // Shunting-yard, values evaluated as operators leave the stack.
    let mut vals = [Dec::ZERO; MAX];
    let mut nv = 0;
    let mut ops = [0u8; MAX]; // b'(' marks a bracket, b'F' + func a function
    let mut fns = [0u8; MAX];
    let mut no = 0;
    let reduce = |vals: &mut [Dec; MAX], nv: &mut usize, op: u8| -> Result<(), Fault> {
        if op == b'~' {
            if *nv < 1 {
                return Err(Fault::Incomplete);
            }
            vals[*nv - 1] = Dec::ZERO.minus(vals[*nv - 1]).map_err(Fault::Math)?;
            return Ok(());
        }
        if *nv < 2 {
            return Err(Fault::Incomplete);
        }
        let b = vals[*nv - 1];
        let a = vals[*nv - 2];
        *nv -= 1;
        vals[*nv - 1] = apply(op, a, b)?;
        Ok(())
    };
    let mut expect_value = true;
    // A stack with no room left is a sum too deep to work (96 function
    // letters ran the operators' stack past its end: a panic).
    let full = |n: usize| {
        if n < MAX {
            Ok(())
        } else {
            Err(Fault::Incomplete)
        }
    };
    let (mut at, mut read) = (0, 0);
    while read < MAX {
        let Some(t) = token(text, &mut at)? else {
            break;
        };
        read += 1;
        match t {
            Tok::Num(d) => {
                full(nv)?;
                vals[nv] = d;
                nv += 1;
                expect_value = false;
            }
            Tok::Op(mut op) => {
                // A minus where a value is due is a negation.
                if expect_value && op == b'-' {
                    op = b'~';
                }
                while no > 0 {
                    let top = ops[no - 1];
                    if top == b'(' || top == b'F' {
                        break;
                    }
                    // ^ and ~ bind to the right.
                    let right = op == b'^' || op == b'~';
                    if prec(top) > prec(op) || (!right && prec(top) == prec(op)) {
                        reduce(&mut vals, &mut nv, top)?;
                        no -= 1;
                    } else {
                        break;
                    }
                }
                full(no)?;
                ops[no] = op;
                no += 1;
                expect_value = true;
            }
            Tok::Open => {
                full(no)?;
                ops[no] = b'(';
                no += 1;
                expect_value = true;
            }
            Tok::Func(f) => {
                full(no + 1)?;
                ops[no] = b'F';
                fns[no] = f;
                no += 1;
                // Its bracket follows (or it applies to the next value).
                let mut next = at;
                if !matches!(token(text, &mut next), Ok(Some(Tok::Open))) {
                    ops[no] = b'(';
                    no += 1;
                }
                expect_value = true;
            }
            Tok::Close => {
                while no > 0 && ops[no - 1] != b'(' {
                    let op = ops[no - 1];
                    if op == b'F' {
                        break;
                    }
                    reduce(&mut vals, &mut nv, op)?;
                    no -= 1;
                }
                if no == 0 {
                    return Err(Fault::Incomplete);
                }
                no -= 1; // the bracket
                if no > 0 && ops[no - 1] == b'F' {
                    if nv == 0 {
                        return Err(Fault::Incomplete);
                    }
                    vals[nv - 1] = function(fns[no - 1], vals[nv - 1], radians)?;
                    no -= 1;
                }
                expect_value = false;
            }
            Tok::Percent => {
                if nv == 0 {
                    return Err(Fault::Incomplete);
                }
                let hundredth = vals[nv - 1].over(Dec::whole(100)).map_err(Fault::Math)?;
                // Of the number before when it follows + or -.
                let base =
                    (no > 0 && matches!(ops[no - 1], b'+' | b'-') && nv >= 2).then(|| vals[nv - 2]);
                vals[nv - 1] = match base {
                    Some(b) => b.times(hundredth).map_err(Fault::Math)?,
                    None => hundredth,
                };
            }
            Tok::Fact => {
                if nv == 0 {
                    return Err(Fault::Incomplete);
                }
                vals[nv - 1] = factorial(vals[nv - 1])?;
            }
        }
    }
    while no > 0 {
        let op = ops[no - 1];
        if op == b'(' || op == b'F' {
            return Err(Fault::Incomplete);
        }
        reduce(&mut vals, &mut nv, op)?;
        no -= 1;
    }
    if nv != 1 {
        return Err(Fault::Incomplete);
    }
    Ok(vals[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(t: &[u8]) -> std::string::String {
        let mut b = [0u8; 32];
        match eval(t, false) {
            Ok(d) => {
                let n = d.show(0, &mut b);
                std::string::String::from_utf8_lossy(&b[..n]).into_owned()
            }
            Err(e) => std::format!("{e:?}"),
        }
    }

    #[test]
    fn school_order_brackets_and_exact_decimals() {
        assert_eq!(show(b"2+3x4"), "14");
        assert_eq!(show(b"(2+3)x4"), "20");
        assert_eq!(show(b"0.1+0.2"), "0.3");
        assert_eq!(show(b"2^10"), "1024");
        assert_eq!(show(b"2^3^2"), "512"); // right to left
        assert_eq!(show(b"-3^2"), "-9");
        assert_eq!(show(b"10-2-3"), "5"); // left to right
        assert_eq!(show(b"1/4"), "0.25");
        assert_eq!(show(b"5!"), "120");
    }

    #[test]
    fn percent_as_people_mean_it() {
        assert_eq!(show(b"200+10%"), "220");
        assert_eq!(show(b"200-10%"), "180");
        assert_eq!(show(b"50x10%"), "5");
        assert_eq!(show(b"15%"), "0.15");
    }

    #[test]
    fn half_typed_sums_say_so_and_math_errors_are_named() {
        assert_eq!(show(b"2+"), "Incomplete");
        assert_eq!(show(b"(2+3"), "Incomplete");
        assert_eq!(show(b"1/0"), "Math(DivideByZero)");
        assert_eq!(show(b"q(~4)"), "Domain");
        assert_eq!(show(b"l(0)"), "Domain");
    }

    #[test]
    fn scientific_functions_to_twelve_digits() {
        assert_eq!(show(b"s(30)"), "0.5");
        assert_eq!(show(b"c(60)"), "0.5");
        assert_eq!(show(b"s(180)"), "0");
        assert_eq!(show(b"t(45)"), "1");
        assert_eq!(show(b"q(2)"), "1.41421356237");
        assert_eq!(show(b"l(e)"), "1");
        assert_eq!(show(b"L(1000)"), "3");
        assert_eq!(show(b"S(1)"), "90");
        assert_eq!(show(b"T(1)"), "45");
        assert_eq!(show(b"2^0.5"), "1.41421356237");
        assert_eq!(show(b"s(30)+2^10"), "1024.5");
    }

    /// A sum too deep for the stacks is incomplete, never a panic (96
    /// function letters ran the operators' stack past its end).
    #[test]
    fn a_sum_too_deep_is_incomplete_not_a_panic() {
        for c in *b"s(q~" {
            for n in [MAX - 1, MAX, 2 * MAX] {
                let mut text = std::vec![c; n];
                assert_eq!(
                    eval(&text, false),
                    Err(Fault::Incomplete),
                    "{n} of {}",
                    c as char
                );
                text.push(b'1');
                let _ = eval(&text, false);
            }
        }
        // As long as it fits it is worked: 48 ones added.
        let mut text = std::vec::Vec::new();
        for k in 0..48 {
            if k > 0 {
                text.push(b'+');
            }
            text.push(b'1');
        }
        assert_eq!(eval(&text, false), Ok(Dec::whole(48)));
    }
}
