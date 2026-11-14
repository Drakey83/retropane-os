//! The quick-add line's tests. Today is Friday 13 November 2026.

extern crate std;
use super::*;
use std::string::String;

const TODAY: i32 = 20_770;

fn read(line: &str) -> (Quick, String) {
    let mut title = [0u8; TITLE_MAX];
    let (q, n) = parse(line.as_bytes(), TODAY, true, &mut title);
    (q, String::from_utf8(title[..n].to_vec()).unwrap())
}

fn day(year: i32, month: u8, day: u8) -> Option<i32> {
    Some(date::days_from(Date { year, month, day }))
}

#[test]
fn today_is_the_friday_the_tests_assume() {
    assert_eq!(day(2026, 11, 13), Some(TODAY));
    assert_eq!(date::weekday(TODAY), 4);
}

#[test]
fn a_day_a_time_and_a_length_come_out_and_the_title_stays() {
    let (q, t) = read("Dentist tue 3pm 1h");
    assert_eq!(t, "Dentist");
    assert_eq!(
        q,
        Quick {
            day: day(2026, 11, 17),
            time: Some(15 * 60),
            minutes: Some(60),
            priority: None
        }
    );
    // Any order, any case, the little words taken with what they lead.
    let (q, t) = read("at NOON lunch with Sam on Tomorrow for 90min");
    assert_eq!(t, "Lunch with Sam");
    assert_eq!(
        (q.day, q.time, q.minutes),
        (Some(TODAY + 1), Some(720), Some(90))
    );
    // Nothing to take: the whole line is the title.
    assert_eq!(
        read("buy milk"),
        (Quick::default(), String::from("Buy milk"))
    );
    assert_eq!(read(""), (Quick::default(), String::new()));
    assert_eq!(read("   "), (Quick::default(), String::new()));
}

#[test]
fn weekdays_mean_the_coming_one_today_included() {
    assert_eq!(read("x fri").0.day, Some(TODAY));
    assert_eq!(read("x friday").0.day, Some(TODAY));
    assert_eq!(read("x sat").0.day, Some(TODAY + 1));
    assert_eq!(read("x thu").0.day, Some(TODAY + 6));
    assert_eq!(read("x Monday").0.day, Some(TODAY + 3));
    assert_eq!(read("x today").0.day, Some(TODAY));
    // Only the first day counts; a second one stays in the title.
    let (q, t) = read("Call Sun on mon");
    assert_eq!((q.day, t.as_str()), (Some(TODAY + 2), "Call on mon"));
}

#[test]
fn dates_by_name_and_by_slash_roll_to_next_year_once_passed() {
    assert_eq!(read("x 14 nov").0.day, day(2026, 11, 14));
    assert_eq!(read("x Nov 14").0.day, day(2026, 11, 14));
    assert_eq!(read("x november 13").0.day, Some(TODAY));
    assert_eq!(read("x 12 nov").0.day, day(2027, 11, 12));
    assert_eq!(read("x 1 jan").0.day, day(2027, 1, 1));
    assert_eq!(read("x 11/14").0.day, day(2026, 11, 14));
    assert_eq!(read("x 11/14/2027").0.day, day(2027, 11, 14));
    assert_eq!(read("x 2/29/28").0.day, day(2028, 2, 29));
    // Day first outside the US.
    let mut title = [0u8; TITLE_MAX];
    assert_eq!(
        parse(b"x 14/11", TODAY, false, &mut title).0.day,
        day(2026, 11, 14)
    );
    // Dates that don't exist are left in the title.
    for line in ["x 31 nov", "x 2/30", "x 13/45", "x 0/5", "x 29 feb"] {
        let (q, t) = read(line);
        if line == "x 29 feb" {
            // 2027 has none either (the next is 2028): not taken.
            assert_eq!(q.day, None, "{line}");
        }
        assert_eq!(
            (q.day, t.to_lowercase()),
            (None, String::from(line)),
            "{line}"
        );
    }
}

#[test]
fn times_on_both_dials_and_bare_numbers_left_alone() {
    for (line, want) in [
        ("x 3pm", Some(900)),
        ("x 3PM", Some(900)),
        ("x 3:30pm", Some(930)),
        ("x 3 pm", Some(900)),
        ("x 3:05 am", Some(185)),
        ("x 12am", Some(0)),
        ("x 12pm", Some(720)),
        ("x 12:59am", Some(59)),
        ("x 15:00", Some(900)),
        ("x 0:00", Some(0)),
        ("x 23:59", Some(1439)),
        ("x midnight", Some(0)),
        // Not times.
        ("x 3", None),
        ("x 15", None),
        ("x 24:00", None),
        ("x 13pm", None),
        ("x 0pm", None),
        ("x 3:60pm", None),
        ("x 3:5pm", None),
        ("x 12345", None),
        ("x pm", None),
    ] {
        assert_eq!(read(line).0.time, want, "{line}");
    }
    // A number that is no time stays in the title.
    assert_eq!(read("Room 12 review").1, "Room 12 review");
    assert_eq!(read("Flight 3 pm").1, "Flight");
}

#[test]
fn lengths_and_priority() {
    for (line, want) in [
        ("x 1h", Some(60)),
        ("x 2hr", Some(120)),
        ("x 1.5h", Some(90)),
        ("x 30m", Some(30)),
        ("x 45min", Some(45)),
        ("x 24h", Some(1440)),
        ("x 25h", None),
        ("x 0m", None),
        ("x 1.25h", None),
        ("x h", None),
        ("x 5k", None),
    ] {
        assert_eq!(read(line).0.minutes, want, "{line}");
    }
    assert_eq!(read("Pay rent fri !").0.priority, Some(2));
    assert_eq!(read("Pay rent !high").0.priority, Some(2));
    assert_eq!(
        read("Sort photos !low"),
        (
            Quick {
                priority: Some(0),
                ..Quick::default()
            },
            String::from("Sort photos")
        )
    );
    assert_eq!(read("Wow!").0.priority, None);
}

#[test]
fn long_lines_are_cut_at_a_word_and_never_overrun() {
    let long = "word ".repeat(40);
    let (q, t) = read(&long);
    assert_eq!(q, Quick::default());
    assert!(t.len() <= TITLE_MAX && t.ends_with("word") && t.starts_with("Word"));
    // More than 24 words with a time past the 24th: it stays in the title.
    let line = std::format!("{}3pm", "a ".repeat(30));
    assert_eq!(read(&line).0.time, None);
    // One word longer than the title: nothing kept, nothing overrun.
    let (_, t) = read(&"x".repeat(200));
    assert_eq!(t, "");
    // Every byte value in a line: never a panic.
    let mut title = [0u8; TITLE_MAX];
    for seed in 0..300u32 {
        let mut x = seed.wrapping_mul(2_654_435_761) | 1;
        let mut line = [0u8; 80];
        for b in &mut line {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *b = if x % 5 == 0 { b' ' } else { x as u8 };
        }
        let (_, n) = parse(&line, TODAY, seed % 2 == 0, &mut title);
        assert!(n <= TITLE_MAX);
    }
}
