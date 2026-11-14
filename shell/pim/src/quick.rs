//! The quick-add line (docs/design/calendar-rule0.md): one typed line
//! becomes an appointment or a task. `Dentist tue 3pm 1h` is "Dentist",
//! next Tuesday, 3:00 PM, an hour long.
//!
//! Words that say when (a day, a date, a time, a length) or how much it
//! matters (`!`) are taken out; what is left is the title. Nothing is
//! guessed from bare numbers: `Room 12` stays a title. Whole words only,
//! any case.

use crate::date::{self, Date};

/// What a line says.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Quick {
    /// The day (days from 1970), when the line names one.
    pub day: Option<i32>,
    /// The time of day (minutes from midnight).
    pub time: Option<u16>,
    /// How long, minutes.
    pub minutes: Option<u16>,
    /// The priority: 2 for `!` or `!high`, 0 for `!low`.
    pub priority: Option<u8>,
}

/// The longest title kept.
pub const TITLE_MAX: usize = 64;

#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn lower(b: u8) -> u8 {
    b.to_ascii_lowercase()
}

#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn is(word: &[u8], name: &str) -> bool {
    word.eq_ignore_ascii_case(name.as_bytes())
}

/// A whole number of at most four digits.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn number(w: &[u8]) -> Option<u32> {
    if w.is_empty() || w.len() > 4 || !w.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(w.iter().fold(0, |n, b| n * 10 + u32::from(b - b'0')))
}

/// `mon`..`sun` or the full name: 0 Monday ..= 6 Sunday.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn weekday(w: &[u8]) -> Option<u8> {
    const FULL: [&str; 7] = [
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
    ];
    (0..7u8).find(|&k| is(w, FULL[usize::from(k)]) || is(w, date::WEEKDAYS[usize::from(k)]))
}

/// `jan`..`dec` or the full name: 1-12.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn month(w: &[u8]) -> Option<u8> {
    (0..12u8)
        .find(|&k| is(w, date::MONTHS[usize::from(k)]) || is(w, date::MONTHS_LONG[usize::from(k)]))
        .map(|k| k + 1)
}

/// `am` or `pm`: whether it is the afternoon.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn half(w: &[u8]) -> Option<bool> {
    if is(w, "am") || is(w, "a.m.") {
        Some(false)
    } else if is(w, "pm") || is(w, "p.m.") {
        Some(true)
    } else {
        None
    }
}

/// A clock time on a 12-hour dial as minutes from midnight.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn twelve(hour: u32, minute: u32, pm: bool) -> Option<u16> {
    if !(1..=12).contains(&hour) || minute > 59 {
        return None;
    }
    Some(((hour % 12 + if pm { 12 } else { 0 }) * 60 + minute) as u16)
}

/// `3pm`, `3:30pm`, `15:00`, `noon`, `midnight`; or `3` / `3:30` when
/// `next` is `am` or `pm` (then the second word is used too).
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn time(w: &[u8], next: Option<&[u8]>) -> Option<(u16, bool)> {
    if is(w, "noon") {
        return Some((12 * 60, false));
    }
    if is(w, "midnight") {
        return Some((0, false));
    }
    // A trailing am or pm on the word itself.
    let (digits, own) = match w.len().checked_sub(2).map(|k| w.split_at(k)) {
        Some((d, h)) if half(h).is_some() => (d, half(h)),
        _ => (w, None),
    };
    let (h, m, colon) = match digits.iter().position(|b| *b == b':') {
        Some(k) => (number(&digits[..k])?, number(&digits[k + 1..])?, true),
        None => (number(digits)?, 0, false),
    };
    if colon && digits.len() - digits.iter().position(|b| *b == b':')? - 1 != 2 {
        return None;
    }
    match (own, next.and_then(half)) {
        (Some(pm), _) => Some((twelve(h, m, pm)?, false)),
        (None, Some(pm)) => Some((twelve(h, m, pm)?, true)),
        // 24-hour times need their colon: a bare number is no time.
        (None, None) if colon && h < 24 && m < 60 => Some(((h * 60 + m) as u16, false)),
        _ => None,
    }
}

/// `1h`, `30m`, `90min`, `1.5h`, `2hr`: minutes (1 to 24 hours).
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn length(w: &[u8]) -> Option<u16> {
    let unit_at = w.iter().position(|b| b.is_ascii_alphabetic())?;
    let (n, unit) = w.split_at(unit_at);
    let hours = ["h", "hr", "hrs", "hour", "hours"]
        .iter()
        .any(|u| is(unit, u));
    let mins = ["m", "min", "mins", "minutes"].iter().any(|u| is(unit, u));
    let minutes = if hours {
        match n.iter().position(|b| *b == b'.') {
            Some(k) if &n[k + 1..] == b"5" => number(&n[..k])? * 60 + 30,
            Some(_) => return None,
            None => number(n)? * 60,
        }
    } else if mins {
        number(n)?
    } else {
        return None;
    };
    (1..=24 * 60).contains(&minutes).then_some(minutes as u16)
}

/// `11/14` or `11/14/2027` (month first when `month_first`, as in the US;
/// day first elsewhere): month, day, year.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn slashed(w: &[u8], month_first: bool) -> Option<(u8, u8, Option<i32>)> {
    let mut parts = w.split(|b| *b == b'/');
    let (a, b) = (number(parts.next()?)?, number(parts.next()?)?);
    let year = match parts.next() {
        Some(y) => {
            let y = number(y)?;
            Some(if y < 100 { 2000 + y } else { y } as i32)
        }
        None => None,
    };
    if parts.next().is_some() {
        return None;
    }
    let (m, d) = if month_first { (a, b) } else { (b, a) };
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((m as u8, d as u8, year))
}

/// The day a month and day name: this year's, or next year's once it has
/// passed; `None` for a date that doesn't exist.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn day_named(today: i32, month: u8, day: u8, year: Option<i32>) -> Option<i32> {
    let this = date::date_of(today).year;
    for year in year.map_or(this..this + 2, |y| y..y + 1) {
        let d = Date { year, month, day };
        if date::valid(d) && (date::days_from(d) >= today || year != this) {
            return Some(date::days_from(d));
        }
    }
    None
}

/// Reads `line` on day `today`: what it says, and its title into `title`
/// (the words not taken, single-spaced, at most [`TITLE_MAX`] bytes): the
/// title's length.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub fn parse(
    line: &[u8],
    today: i32,
    month_first: bool,
    title: &mut [u8; TITLE_MAX],
) -> (Quick, usize) {
    // At most 24 words are looked at; the rest stay in the title.
    let mut words = [&[][..]; 24];
    let mut n = 0;
    let mut rest: &[u8] = &[];
    let mut at = 0;
    while at < line.len() {
        if line[at] == b' ' {
            at += 1;
            continue;
        }
        if n == words.len() {
            rest = &line[at..];
            break;
        }
        let end = line[at..]
            .iter()
            .position(|b| *b == b' ')
            .map_or(line.len(), |k| at + k);
        words[n] = &line[at..end];
        n += 1;
        at = end;
    }
    let words = &words[..n];
    let mut taken = [false; 24];
    let mut q = Quick::default();
    let mut k = 0;
    while k < n {
        let w = words[k];
        let next = words.get(k + 1).copied();
        let mut used = 0;
        if q.day.is_none() {
            if is(w, "today") {
                q.day = Some(today);
                used = 1;
            } else if is(w, "tomorrow") {
                q.day = Some(today + 1);
                used = 1;
            } else if let Some(wd) = weekday(w) {
                // The coming one, today included.
                let ahead = (i32::from(wd) - i32::from(date::weekday(today))).rem_euclid(7);
                q.day = Some(today + ahead);
                used = 1;
            } else if let (Some(m), Some(d)) = (month(w), next.and_then(number)) {
                // "nov 14"
                if let Some(day) = day_named(today, m, d.min(99) as u8, None) {
                    q.day = Some(day);
                    used = 2;
                }
            } else if let (Some(d), Some(m)) = (number(w), next.and_then(month)) {
                // "14 nov"
                if let Some(day) = day_named(today, m, d.min(99) as u8, None) {
                    q.day = Some(day);
                    used = 2;
                }
            } else if let Some((m, d, y)) = slashed(w, month_first)
                && let Some(day) = day_named(today, m, d, y)
            {
                q.day = Some(day);
                used = 1;
            }
        }
        if used == 0
            && q.time.is_none()
            && let Some((t, two)) = time(w, next)
        {
            q.time = Some(t);
            used = 1 + usize::from(two);
        }
        if used == 0
            && q.minutes.is_none()
            && let Some(m) = length(w)
        {
            q.minutes = Some(m);
            used = 1;
        }
        if used == 0 && q.priority.is_none() {
            if w == b"!" || is(w, "!high") {
                q.priority = Some(2);
                used = 1;
            } else if is(w, "!low") {
                q.priority = Some(0);
                used = 1;
            }
        }
        if used > 0 {
            // "at 3pm", "on fri", "for 1h": the little word goes too.
            if k > 0
                && !taken[k - 1]
                && ["at", "on", "for", "@"].iter().any(|p| is(words[k - 1], p))
            {
                taken[k - 1] = true;
            }
            for t in &mut taken[k..k + used] {
                *t = true;
            }
            k += used;
        } else {
            k += 1;
        }
    }
    let mut len = 0;
    let kept = words
        .iter()
        .zip(&taken)
        .filter(|(_, t)| !**t)
        .map(|(w, _)| *w)
        .chain(rest.split(|b| *b == b' ').filter(|w| !w.is_empty()));
    for w in kept {
        let sep = usize::from(len > 0);
        if len + sep + w.len() > TITLE_MAX {
            break;
        }
        if sep == 1 {
            title[len] = b' ';
        }
        title[len + sep..len + sep + w.len()].copy_from_slice(w);
        len += sep + w.len();
    }
    // A sentence starts with a capital.
    if let Some(first) = title[..len].first_mut()
        && lower(*first) == *first
    {
        *first = first.to_ascii_uppercase();
    }
    (q, len)
}

#[cfg(test)]
#[path = "quick_tests.rs"]
mod tests;
