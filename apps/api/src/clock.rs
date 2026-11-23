//! Times of day as every app writes them, in the clock the user chose
//! ([`crate::Ctx::clock12`]): "3:05 PM" or "15:05" (R383).
//!
//! Iterate, don't imitate (rule 0). Windows Mobile 5 read a time-style
//! string from Regional Settings and each program formatted with it (or
//! didn't: Pocket Outlook's day view kept "8a"/"9a" labels whatever was
//! set); Android and iOS keep a "24-hour time" switch that defaults to the
//! locale and every app asks one formatter. Ours is the latter in 2 KB:
//! one writer here, the switch in the settings beside the region, and no
//! app with a time format of its own, so a screen can't be left behind.

use crate::Text;

/// `m` minutes from midnight: "3:05 PM" on the 12-hour clock, "15:05" on
/// the 24-hour one (the hour two digits, as the 24-hour clock is read).
#[must_use]
pub fn time(m: u16, clock12: bool) -> Text<8> {
    let (h, min) = (m / 60 % 24, m % 60);
    let mut b = [0u8; 8];
    let mut n = 0;
    let mut put = |c: u8| {
        b[n] = c;
        n += 1;
    };
    if clock12 {
        let h12 = (h + 11) % 12 + 1;
        if h12 >= 10 {
            put(b'1');
        }
        put(b'0' + (h12 % 10) as u8);
    } else {
        put(b'0' + (h / 10) as u8);
        put(b'0' + (h % 10) as u8);
    }
    put(b':');
    put(b'0' + (min / 10) as u8);
    put(b'0' + (min % 10) as u8);
    if clock12 {
        for &c in if h < 12 { b" AM" } else { b" PM" } {
            put(c);
        }
    }
    Text::of(&b[..n])
}

/// Hour `h` as a day view's line names it: "3 PM", or "15:00".
#[must_use]
pub fn hour(h: u16, clock12: bool) -> Text<8> {
    if clock12 {
        let h12 = (h % 24 + 11) % 12 + 1;
        let mut b = [0u8; 5];
        let mut n = 0;
        if h12 >= 10 {
            b[n] = b'1';
            n += 1;
        }
        b[n] = b'0' + (h12 % 10) as u8;
        n += 1;
        b[n..n + 3].copy_from_slice(if h % 24 < 12 { b" AM" } else { b" PM" });
        Text::of(&b[..n + 3])
    } else {
        time(h * 60, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_clocks_at_their_edges() {
        for (m, twelve, twenty_four) in [
            (0, "12:00 AM", "00:00"),
            (7, "12:07 AM", "00:07"),
            (9 * 60 + 5, "9:05 AM", "09:05"),
            (11 * 60 + 59, "11:59 AM", "11:59"),
            (12 * 60, "12:00 PM", "12:00"),
            (13 * 60 + 30, "1:30 PM", "13:30"),
            (23 * 60 + 59, "11:59 PM", "23:59"),
            // A time past midnight (an end that runs over) wraps.
            (24 * 60 + 15, "12:15 AM", "00:15"),
        ] {
            assert_eq!(time(m, true).get(), twelve.as_bytes());
            assert_eq!(time(m, false).get(), twenty_four.as_bytes());
        }
        for (h, twelve, twenty_four) in [
            (0, "12 AM", "00:00"),
            (6, "6 AM", "06:00"),
            (12, "12 PM", "12:00"),
            (15, "3 PM", "15:00"),
            (23, "11 PM", "23:00"),
        ] {
            assert_eq!(hour(h, true).get(), twelve.as_bytes());
            assert_eq!(hour(h, false).get(), twenty_four.as_bytes());
        }
    }
}
