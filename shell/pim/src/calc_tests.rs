//! The calculator's tests: decimal arithmetic at its edges, and the keys.

extern crate std;
use super::*;
use std::string::String;

fn dec(text: &str) -> Dec {
    let (neg, digits) = match text.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, text),
    };
    let v = Dec::typed(digits.as_bytes()).unwrap();
    if neg { v.neg() } else { v }
}

fn show(v: Dec) -> String {
    let mut out = [0u8; 32];
    let n = v.show(b',', &mut out);
    String::from_utf8(out[..n].to_vec()).unwrap()
}

/// Presses keys written as text: digits, `.`, `+ - x /`, `=`, `C`, `<`
/// (delete), `n` (negate), `%`. The display's two lines after.
fn keys(c: &mut Calc, text: &str) -> (String, String) {
    for ch in text.chars() {
        c.press(match ch {
            '0'..='9' => Key::Digit(ch as u8 - b'0'),
            '.' => Key::Point,
            '+' => Key::Op(Op::Add),
            '-' => Key::Op(Op::Sub),
            'x' => Key::Op(Op::Mul),
            '/' => Key::Op(Op::Div),
            '=' => Key::Equals,
            'C' => Key::Clear,
            '<' => Key::Delete,
            'n' => Key::Negate,
            '%' => Key::Percent,
            ' ' => continue,
            _ => panic!("no key {ch}"),
        });
    }
    let mut line = [0u8; 96];
    let k = c.line(&mut line);
    let mut num = [0u8; 32];
    let number = match c.number(&mut num) {
        Ok(n) => String::from_utf8(num[..n].to_vec()).unwrap(),
        Err(e) => String::from(e.says()),
    };
    (String::from_utf8(line[..k].to_vec()).unwrap(), number)
}

fn result(text: &str) -> String {
    keys(&mut Calc::new(), text).1
}

#[test]
fn decimal_sums_are_exact_where_binary_floats_are_not() {
    assert_eq!(show(dec("0.1").plus(dec("0.2")).unwrap()), "0.3");
    assert_eq!(show(dec("1.10").minus(dec("1")).unwrap()), "0.1");
    assert_eq!(show(dec("19.99").times(dec("3")).unwrap()), "59.97");
    assert_eq!(show(dec("0.07").times(dec("100")).unwrap()), "7");
    assert_eq!(show(dec("1").over(dec("8")).unwrap()), "0.125");
    // A hundred dimes are ten dollars.
    let mut sum = Dec::ZERO;
    for _ in 0..100 {
        sum = sum.plus(dec("0.10")).unwrap();
    }
    assert_eq!(show(sum), "10");
}

#[test]
fn division_rounds_half_away_from_zero_at_twelve_digits() {
    assert_eq!(show(dec("1").over(dec("3")).unwrap()), "0.333333333333");
    assert_eq!(show(dec("2").over(dec("3")).unwrap()), "0.666666666667");
    assert_eq!(show(dec("-2").over(dec("3")).unwrap()), "-0.666666666667");
    assert_eq!(show(dec("10").over(dec("4")).unwrap()), "2.5");
    assert_eq!(show(dec("1").over(dec("7")).unwrap()), "0.142857142857");
    assert_eq!(show(dec("200").over(dec("3")).unwrap()), "66.6666666667");
    // A third times three: the twelve digits shown, not a false 1.
    let third = dec("1").over(dec("3")).unwrap();
    assert_eq!(show(third.times(dec("3")).unwrap()), "0.999999999999");
    assert_eq!(dec("5").over(Dec::ZERO), Err(Error::DivideByZero));
    assert_eq!(Dec::ZERO.over(Dec::ZERO), Err(Error::DivideByZero));
    assert_eq!(show(Dec::ZERO.over(dec("5")).unwrap()), "0");
}

#[test]
fn the_largest_and_smallest_numbers_and_what_lies_past_them() {
    let max = dec("999999999999");
    assert_eq!(show(max), "999,999,999,999");
    assert_eq!(max.plus(dec("1")), Err(Error::TooLarge));
    assert_eq!(max.times(dec("10")), Err(Error::TooLarge));
    assert_eq!(show(max.neg().minus(dec("0")).unwrap()), "-999,999,999,999");
    assert_eq!(max.neg().minus(dec("1")), Err(Error::TooLarge));
    assert_eq!(dec("1").over(dec("0.000000000001")), Err(Error::TooLarge));
    // The smallest step kept is 10^-12; smaller rounds to it or to zero.
    assert_eq!(show(dec("0.000000000001")), "0.000000000001");
    assert_eq!(
        show(dec("0.000001").times(dec("0.000001")).unwrap()),
        "0.000000000001"
    );
    assert_eq!(show(dec("0.000001").times(dec("0.0000004")).unwrap()), "0");
    assert_eq!(
        show(dec("0.000001").times(dec("0.0000005")).unwrap()),
        "0.000000000001"
    );
    // Digits past the twelfth round: 123456789012.5 isn't kept whole.
    assert_eq!(
        show(dec("123456789012").plus(dec("0.5")).unwrap()),
        "123,456,789,013"
    );
    assert_eq!(
        show(dec("123456789012").plus(dec("0.4")).unwrap()),
        "123,456,789,012"
    );
    assert_eq!(Dec::typed(b"1.2.3"), None);
    assert_eq!(Dec::typed(b"12a"), None);
    assert_eq!(Dec::typed(b""), Some(Dec::ZERO));
}

#[test]
fn numbers_show_grouped_with_their_places() {
    for (text, want) in [
        ("0", "0"),
        ("7", "7"),
        ("999", "999"),
        ("1000", "1,000"),
        ("1234567", "1,234,567"),
        ("1234.5", "1,234.5"),
        ("0.5", "0.5"),
        ("0.05", "0.05"),
        ("-0.05", "-0.05"),
        ("-1000", "-1,000"),
        ("12.50", "12.5"),
    ] {
        assert_eq!(show(dec(text)), want, "{text}");
    }
    let mut out = [0u8; 32];
    let n = dec("1234567.25").show(0, &mut out);
    assert_eq!(&out[..n], b"1234567.25");
}

#[test]
fn times_and_divided_by_come_before_plus_and_minus() {
    assert_eq!(result("2+3x4="), "14");
    assert_eq!(result("2x3+4="), "10");
    assert_eq!(result("10-6/2="), "7");
    assert_eq!(result("10/4x2="), "5");
    assert_eq!(result("1+2x3-4/2="), "5");
    assert_eq!(result("2x3x4="), "24");
    assert_eq!(result("100-1-1-1="), "97");
    // The line above shows the sum as typed, then with its equals.
    let mut c = Calc::new();
    assert_eq!(
        keys(&mut c, "12+3x"),
        (String::from("12 + 3 x "), String::from("0"))
    );
    assert_eq!(
        keys(&mut c, "4"),
        (String::from("12 + 3 x "), String::from("4"))
    );
    assert_eq!(
        keys(&mut c, "="),
        (String::from("12 + 3 x 4 ="), String::from("24"))
    );
}

#[test]
fn after_equals_a_digit_starts_again_and_an_operation_goes_on() {
    let mut c = Calc::new();
    assert_eq!(keys(&mut c, "2+3=").1, "5");
    // A digit: a new sum, the old line gone.
    assert_eq!(keys(&mut c, "7"), (String::new(), String::from("7")));
    assert_eq!(keys(&mut c, "x6=").1, "42");
    // An operation: on from the result.
    assert_eq!(
        keys(&mut c, "+8"),
        (String::from("42 + "), String::from("8"))
    );
    assert_eq!(keys(&mut c, "=").1, "50");
    // Equals again changes nothing; delete after a result clears.
    assert_eq!(keys(&mut c, "=").1, "50");
    assert_eq!(keys(&mut c, "<"), (String::new(), String::from("0")));
    // Plus or minus on a result takes it as the number, negated.
    assert_eq!(keys(&mut c, "9=n").1, "-9");
    assert_eq!(keys(&mut c, "+10=").1, "1");
}

#[test]
fn typing_a_number_its_point_delete_and_sign() {
    assert_eq!(result("007"), "7");
    assert_eq!(result("0.50"), "0.50");
    assert_eq!(result("."), "0.");
    assert_eq!(result("1..2"), "1.2");
    assert_eq!(result("1234567"), "1,234,567");
    assert_eq!(result("1234.5678"), "1,234.5678");
    assert_eq!(result("12<"), "1");
    assert_eq!(result("12<<"), "0");
    assert_eq!(result("12<<<"), "0");
    assert_eq!(result("5n"), "-5");
    assert_eq!(result("5nn"), "5");
    assert_eq!(result("n"), "0");
    assert_eq!(result("5n<"), "0");
    // Twelve digits and no more, the point not counted.
    assert_eq!(result("1234567890123456"), "123,456,789,012");
    assert_eq!(result("0.1234567890123456"), "0.12345678901");
    assert_eq!(result("5n+3="), "-2");
    assert_eq!(result("0.1+0.2="), "0.3");
}

#[test]
fn operations_without_a_number_and_equals_alone() {
    // A second operation replaces the first; one at the start works on 0.
    assert_eq!(result("5+x3="), "15");
    assert_eq!(result("-5="), "-5");
    assert_eq!(result("="), "0");
    // "5 + =" adds the number to itself, as pocket calculators do.
    assert_eq!(result("5+="), "10");
    assert_eq!(result("5x="), "25");
    // Nine terms: the first eight fold into one, the sum stays right.
    assert_eq!(result("1+1+1+1+1+1+1+1+1+1="), "10");
    assert_eq!(result("1+2x3+4x5+6x7+8x9+1="), "142");
}

#[test]
fn percent_is_a_share_after_plus_or_minus_and_a_hundredth_otherwise() {
    assert_eq!(result("200+10%"), "20");
    assert_eq!(result("200+10%="), "220");
    assert_eq!(result("200-10%="), "180");
    assert_eq!(result("200x10%="), "20");
    assert_eq!(result("50%"), "0.5");
    assert_eq!(result("50+50=%"), "1");
    assert_eq!(result("100+50+10%="), "165");
    assert_eq!(result("%"), "0");
}

#[test]
fn an_error_stays_until_clear_or_a_new_number() {
    let mut c = Calc::new();
    assert_eq!(keys(&mut c, "5/0=").1, "Cannot divide by zero");
    // Operations, equals, delete, percent, sign: still the error.
    assert_eq!(keys(&mut c, "+=<%n").1, "Cannot divide by zero");
    assert_eq!(keys(&mut c, "C"), (String::new(), String::from("0")));
    assert_eq!(keys(&mut c, "5/0=7").1, "7");
    assert_eq!(keys(&mut c, "C999999999999x10=").1, "Too large to show");
    assert_eq!(keys(&mut c, "3+4=").1, "7");
    // Clear in the middle of a sum forgets all of it.
    assert_eq!(
        keys(&mut c, "C8x9C+1="),
        (String::from("0 + 1 ="), String::from("1"))
    );
}
