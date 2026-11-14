//! The organiser apps' tests: each app's flows through the shell's own
//! inputs (taps on the targets it shows, the D-pad, the on-screen
//! keyboard), read back from the store. The clock is noon on Monday
//! 1 January 2024.

extern crate std;

use rp_draw::{Frame, MAX_CMDS, Rect};
use rp_pim::date::{self, Date};
use rp_pim::store::{Kind, tag};
use std::string::String;
use std::vec::Vec;

use super::{Act, MEMORY, View, preview};
use crate::tests::{drawn, shell, tap};
use crate::{Action, H, Input, Key, START, Screen, Shell, Target, Time, W};

const NOON: Time = Time {
    hour: 12,
    minute: 0,
    weekday: 0,
    day: 1,
    month: 1,
};
const LEFT: Rect = Rect::new(0, H - 40, 160, 40);
const RIGHT: Rect = Rect::new(W - 160, H - 40, 160, 40);

fn today() -> i32 {
    date::days_from(Date {
        year: 2024,
        month: 1,
        day: 1,
    })
}

/// A shell with the apps' memory and an empty, loaded store.
fn organiser() -> Shell {
    let mut s = shell(false);
    s.set_time(NOON);
    s.give_apps(std::vec![0u8; MEMORY].leak());
    s.pim().unwrap().load_start(None);
    s.pim_loaded();
    drawn(&mut s);
    s
}

fn open(s: &mut Shell, entry: u8) {
    tap(s, START);
    tap(s, Shell::menu_entry(entry));
}

/// Types on the on-screen keyboard.
fn type_text(s: &mut Shell, text: &str) {
    for c in text.bytes() {
        let (keys, n) = preview::key_taps(c).unwrap_or_else(|| panic!("no key for {}", c as char));
        for k in &keys[..n] {
            tap(s, *k);
        }
    }
}

/// Taps the app target doing `act`.
fn act(s: &mut Shell, act: Act) -> Action {
    let t = s
        .app_find(act)
        .unwrap_or_else(|| panic!("no target for {act:?}"));
    tap(s, s.rect_of(t))
}

fn n(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

fn add_event(s: &mut Shell, title: &str, start: u32, remind: Option<u32>) -> u32 {
    let (st, len, rem) = (n(start), n(60), remind.map(n));
    let mut f: Vec<(u8, &[u8])> = std::vec![
        (tag::TITLE, title.as_bytes()),
        (tag::START, &st),
        (tag::MINUTES, &len)
    ];
    if let Some(r) = &rem {
        f.push((tag::REMIND, r));
    }
    let id = s.pim().unwrap().put(Kind::Appointment, None, &f).unwrap();
    s.pim_loaded();
    id
}

fn titles(s: &mut Shell, kind: Kind, field: u8) -> Vec<String> {
    let mut v: Vec<(u32, String)> = s
        .pim()
        .unwrap()
        .all(kind)
        .map(|r| (r.id, String::from_utf8_lossy(r.text(field)).into_owned()))
        .collect();
    v.sort();
    v.into_iter().map(|(_, t)| t).collect()
}

/// The text of every text run of a frame, joined by `|`.
fn words(f: &Frame) -> String {
    let mut out = String::new();
    for c in f.cmds() {
        if let rp_draw::Cmd::Text { text, .. } = c {
            out.push_str(&String::from_utf8_lossy(f.text_of(*text)));
            out.push('|');
        }
    }
    out
}

#[test]
fn the_start_menu_opens_each_app_and_home_leaves_it() {
    let mut s = organiser();
    for (entry, view) in [
        (1, View::Agenda),
        (2, View::Contacts),
        (3, View::Tasks),
        (4, View::Notes),
        (5, View::Calculator),
    ] {
        open(&mut s, entry);
        assert_eq!(s.screen(), Screen::App(view));
        let f = drawn(&mut s);
        assert!(!f.overflowed() && f.cmds().count() < MAX_CMDS, "{view:?}");
        // From one app straight to the next, and Home from any.
    }
    s.input(Input::Key(Key::Home));
    assert_eq!(s.screen(), Screen::Today);
    assert_eq!(s.app_targets(), 0, "no app target is left on Today");
    // Today's left softkey is the Calendar; its first quick button a note.
    tap(&mut s, LEFT);
    assert_eq!(s.screen(), Screen::App(View::Agenda));
    tap(&mut s, LEFT); // Today (the date), still the calendar
    tap(&mut s, Rect::new(0, 36, 64, 58)); // the header's back
    assert_eq!(s.screen(), Screen::Today);
}

#[test]
fn without_memory_the_apps_say_so_and_the_calculator_still_works() {
    let mut s = shell(false);
    s.set_time(NOON);
    open(&mut s, 1);
    assert!(words(&drawn(&mut s)).contains("The organiser has no memory here"));
    open(&mut s, 5);
    for c in "7x6=".chars() {
        tap(&mut s, preview::calc_key(c).unwrap());
    }
    assert!(words(&drawn(&mut s)).contains("|42|"));
    // A store still loading says so, and takes no change.
    let mut s = shell(false);
    s.give_apps(std::vec![0u8; MEMORY].leak());
    open(&mut s, 3);
    assert!(words(&drawn(&mut s)).contains("Reading the card"));
}

#[test]
fn quick_add_keeps_the_appointment_it_previewed() {
    let mut s = organiser();
    open(&mut s, 1);
    assert!(words(&drawn(&mut s)).contains("Nothing planned."));
    tap(&mut s, RIGHT); // New
    assert_eq!(s.screen(), Screen::App(View::NewEvent));
    type_text(&mut s, "gym thu 6pm 1h");
    let shown = words(&drawn(&mut s));
    assert!(
        shown.contains("|Gym|") && shown.contains("THU 4 JAN   6:00 PM - 7:00 PM"),
        "{shown}"
    );
    tap(&mut s, RIGHT); // Add
    assert_eq!(s.screen(), Screen::App(View::Agenda));
    let r = s.pim().unwrap().all(Kind::Appointment).next().unwrap();
    let want = date::start_of(today() + 3) + 18 * 60;
    assert_eq!(
        (
            r.text(tag::TITLE),
            r.num(tag::START),
            r.num(tag::MINUTES),
            r.num(tag::REMIND)
        ),
        (&b"Gym"[..], Some(want), Some(60), Some(15))
    );
    let shown = words(&drawn(&mut s));
    assert!(
        shown.contains("THU 4 JAN") && shown.contains("|6:00 PM|Gym|"),
        "{shown}"
    );
    // The row opens the editor; a stepper moves the day; Done keeps it.
    act(&mut s, Act::Open(1));
    assert_eq!(s.screen(), Screen::App(View::EventEdit));
    act(&mut s, Act::Step(0, 1));
    act(&mut s, Act::Step(1, -1));
    tap(&mut s, RIGHT);
    let r = s.pim().unwrap().get(1).unwrap();
    assert_eq!(r.num(tag::START), Some(want + date::DAY_MIN - 15));
    // Delete asks once, then removes.
    act(&mut s, Act::Open(1));
    act(&mut s, Act::Delete);
    assert!(words(&drawn(&mut s)).contains("Tap again to delete"));
    assert_eq!(s.pim().unwrap().len(), 1);
    act(&mut s, Act::Delete);
    assert!(s.pim().unwrap().is_empty());
    assert_eq!(s.screen(), Screen::App(View::Agenda));
}

#[test]
fn the_three_calendar_views_show_the_same_day() {
    let mut s = organiser();
    let noon = date::start_of(today()) + 12 * 60;
    add_event(&mut s, "Lunch", noon + 30, None);
    add_event(&mut s, "Dentist", noon + 180, None);
    add_event(&mut s, "Review", noon + 210, None);
    add_event(&mut s, "Trip", noon + 8 * date::DAY_MIN, None);
    open(&mut s, 1);
    let agenda = words(&drawn(&mut s));
    assert!(
        agenda.contains("TODAY - MON 1 JAN") && agenda.contains("TUE 9 JAN"),
        "{agenda}"
    );
    act(&mut s, Act::Chip(View::Day));
    let day = words(&drawn(&mut s));
    assert!(
        day.contains("Lunch") && day.contains("Dentist") && !day.contains("Trip"),
        "{day}"
    );
    // Two that overlap sit side by side, neither over the other.
    let (a, b) = (
        s.rect_of(s.app_find(Act::Open(2)).unwrap()),
        s.rect_of(s.app_find(Act::Open(3)).unwrap()),
    );
    assert!(a.intersect(&b).is_empty(), "{a:?} {b:?}");
    // An empty hour starts a new appointment at that hour.
    act(&mut s, Act::Hour(9));
    assert!(words(&drawn(&mut s)).contains("9:00 AM - 10:00 AM"));
    tap(&mut s, LEFT);
    act(&mut s, Act::Chip(View::Month));
    let month = words(&drawn(&mut s));
    assert!(
        month.contains("JANUARY 2024") && month.contains("Dentist"),
        "{month}"
    );
    // The 9th chosen: its list; chosen again: its day; the arrows turn
    // the month and keep the day (or the month's last).
    act(&mut s, Act::Cell(today() + 8));
    assert!(words(&drawn(&mut s)).contains("Trip"));
    act(&mut s, Act::Nav(1));
    assert!(words(&drawn(&mut s)).contains("FEBRUARY 2024"));
    act(&mut s, Act::Nav(-1));
    act(&mut s, Act::Cell(today() + 8));
    assert_eq!(s.screen(), Screen::App(View::Day));
    assert!(words(&drawn(&mut s)).contains("TUE 9 JANUARY 2024"));
    // Today (the softkey) comes back.
    tap(&mut s, LEFT);
    assert!(words(&drawn(&mut s)).contains("MON 1 JANUARY 2024"));
}

#[test]
fn a_reminder_shows_once_on_any_screen_and_is_dismissed_or_opened() {
    let mut s = organiser();
    let at = date::start_of(today()) + 12 * 60 + 20;
    add_event(&mut s, "Call Sam", at, Some(15));
    add_event(&mut s, "Later", at + 600, Some(5));
    add_event(&mut s, "No reminder", at, None);
    assert!(!s.reminder_shown());
    s.set_time(Time { minute: 4, ..NOON });
    assert!(!s.reminder_shown(), "a minute early");
    assert_eq!(s.set_time(Time { minute: 5, ..NOON }), Action::Redraw);
    assert!(s.reminder_shown());
    let shown = words(&drawn(&mut s));
    assert!(
        shown.contains("REMINDER") && shown.contains("Call Sam"),
        "{shown}"
    );
    // On Today, with nothing else of the apps: Dismiss.
    assert_eq!(s.screen(), Screen::Today);
    act(&mut s, Act::Dismiss);
    assert!(!s.reminder_shown());
    // It doesn't come back the next minute, or the next day.
    s.set_time(Time { minute: 6, ..NOON });
    assert!(!s.reminder_shown());
    // The second, from inside another app: Open goes to it.
    open(&mut s, 5);
    s.set_time(Time {
        hour: 22,
        minute: 15,
        ..NOON
    });
    assert!(s.reminder_shown());
    act(&mut s, Act::OpenDue);
    assert_eq!(s.screen(), Screen::App(View::EventEdit));
    assert!(words(&drawn(&mut s)).contains("Later"));
    assert!(!s.reminder_shown());
}

#[test]
fn a_reminder_missed_while_off_shows_at_the_load_unless_the_time_has_passed() {
    let mut s = shell(false);
    s.set_time(NOON);
    s.give_apps(std::vec![0u8; MEMORY].leak());
    s.pim().unwrap().load_start(None);
    let noon = date::start_of(today()) + 12 * 60;
    // Due an hour ago for something still ahead; and one already over.
    for (title, start) in [("Ahead", noon + 30), ("Over", noon - 120)] {
        s.pim()
            .unwrap()
            .put(
                Kind::Appointment,
                None,
                &[
                    (tag::TITLE, title.as_bytes()),
                    (tag::START, &n(start)),
                    (tag::MINUTES, &n(60)),
                    (tag::REMIND, &n(90)),
                ],
            )
            .unwrap();
    }
    s.pim_loaded();
    assert!(s.reminder_shown());
    let shown = words(&drawn(&mut s));
    assert!(
        shown.contains("Ahead") && !shown.contains("Over"),
        "{shown}"
    );
}

fn add_contact(s: &mut Shell, first: &str, last: &str, mobile: &str) -> u32 {
    let id = s
        .pim()
        .unwrap()
        .put(
            Kind::Contact,
            None,
            &[
                (tag::FIRST, first.as_bytes()),
                (tag::LAST, last.as_bytes()),
                (tag::MOBILE, mobile.as_bytes()),
            ],
        )
        .unwrap();
    s.pim_loaded();
    id
}

/// The names the Contacts list shows, in order.
fn listed(s: &mut Shell) -> Vec<String> {
    let f = drawn(s);
    words(&f)
        .split('|')
        .filter(|w| w.contains(' ') && w.chars().next().is_some_and(char::is_uppercase))
        .filter(|w| !w.chars().any(|c| c.is_ascii_digit()))
        // Not the keyboard's or the header's capitals.
        .filter(|w| w.chars().any(char::is_lowercase))
        .map(String::from)
        .collect()
}

#[test]
fn contacts_file_by_last_name_and_search_finds_digits_and_initials() {
    let mut s = organiser();
    for (f, l, m) in [
        ("John", "Doe", "425 555 0100"),
        ("Ada", "Lovelace", "(206) 555-0142"),
        ("Grace", "Hopper", "703 555 0107"),
        ("Alan", "Turing", "206 555 0199"),
        ("grace", "adams", ""),
    ] {
        add_contact(&mut s, f, l, m);
    }
    open(&mut s, 2);
    assert_eq!(
        listed(&mut s),
        [
            "grace adams",
            "John Doe",
            "Grace Hopper",
            "Ada Lovelace",
            "Alan Turing"
        ]
        .iter()
        .filter(|n| n.chars().next().unwrap().is_uppercase())
        .map(|n| String::from(*n))
        .collect::<Vec<_>>()
    );
    // Digits typed any way find a number written any way.
    act(&mut s, Act::Field(9));
    type_text(&mut s, "2065550");
    assert_eq!(listed(&mut s), ["Ada Lovelace", "Alan Turing"]);
    tap(&mut s, LEFT); // Cancel: the search cleared
    assert_eq!(listed(&mut s).len(), 4);
    // Initials, any case; then text anywhere.
    act(&mut s, Act::Field(9));
    type_text(&mut s, "gh");
    assert_eq!(listed(&mut s), ["Grace Hopper"]);
    tap(&mut s, LEFT);
    act(&mut s, Act::Field(9));
    type_text(&mut s, "ring");
    assert_eq!(listed(&mut s), ["Alan Turing"]);
    type_text(&mut s, "zz");
    assert!(words(&drawn(&mut s)).contains("No one matches."));
    tap(&mut s, LEFT);
    // A row opens the card; Edit, a change, Done: kept.
    act(&mut s, Act::Open(3));
    assert_eq!(s.screen(), Screen::App(View::Card));
    assert!(words(&drawn(&mut s)).contains("703 555 0107"));
    tap(&mut s, RIGHT); // Edit
    act(&mut s, Act::Field(2));
    type_text(&mut s, "navy");
    tap(&mut s, RIGHT); // Done
    assert_eq!(s.pim().unwrap().get(3).unwrap().text(tag::COMPANY), b"navy");
    assert_eq!(s.screen(), Screen::App(View::Card));
    // A new card with nothing typed isn't kept.
    tap(&mut s, LEFT);
    tap(&mut s, RIGHT); // New
    tap(&mut s, RIGHT); // Done
    assert_eq!(s.pim().unwrap().len(), 5);
}

#[test]
fn the_keyboards_done_goes_down_a_contacts_fields() {
    let mut s = organiser();
    open(&mut s, 2);
    tap(&mut s, RIGHT); // New: the keyboard at First
    type_text(&mut s, "mae");
    let done = preview::key_taps(b'q').unwrap().0[0];
    let _ = done;
    // The keyboard's big key: the last cap but the strip's two.
    let big = (0..crate::keyboard::CAPS)
        .filter_map(|i| s.kb.cap(i))
        .find(|(c, _)| *c == crate::keyboard::Cap::Done)
        .unwrap()
        .1;
    tap(&mut s, big);
    type_text(&mut s, "west");
    tap(&mut s, RIGHT);
    let r = s.pim().unwrap().all(Kind::Contact).next().unwrap();
    assert_eq!(
        (r.text(tag::FIRST), r.text(tag::LAST)),
        (&b"mae"[..], &b"west"[..])
    );
}

#[test]
fn tasks_order_by_urgency_and_the_box_completes_and_reopens() {
    let mut s = organiser();
    open(&mut s, 3);
    assert!(words(&drawn(&mut s)).contains("Nothing to do."));
    act(&mut s, Act::Field(9));
    for line in [
        "sort photos",
        "pay rent today !",
        "book flights tomorrow",
        "renew passport 12/30/2023",
    ] {
        type_text(&mut s, line);
        tap(&mut s, RIGHT); // Add; the field stays
    }
    tap(&mut s, LEFT);
    let order = |s: &mut Shell| -> Vec<String> {
        words(&drawn(s))
            .split('|')
            .filter(|w| {
                ["Sort", "Pay", "Book", "Renew"]
                    .iter()
                    .any(|p| w.starts_with(p))
            })
            .map(String::from)
            .collect()
    };
    assert_eq!(
        order(&mut s),
        ["Renew passport", "Pay rent", "Book flights", "Sort photos"]
    );
    let shown = words(&drawn(&mut s));
    assert!(
        shown.contains("Late 2 days") && shown.contains("|Today|") && shown.contains("|Tomorrow|")
    );
    // Pay rent (id 2) ticked: done, at the bottom; ticked again: back.
    act(&mut s, Act::Tick(2));
    assert!(s.pim().unwrap().get(2).unwrap().num(tag::DONE).is_some());
    assert_eq!(
        order(&mut s),
        ["Renew passport", "Book flights", "Sort photos", "Pay rent"]
    );
    tap(&mut s, RIGHT); // Hide done
    assert_eq!(order(&mut s).len(), 3);
    tap(&mut s, RIGHT);
    act(&mut s, Act::Tick(2));
    assert_eq!(s.pim().unwrap().get(2).unwrap().num(tag::DONE), None);
    assert_eq!(order(&mut s)[1], "Pay rent");
    // The editor: the due stepper from a date to none, priority chips.
    act(&mut s, Act::Open(3));
    act(&mut s, Act::Step(0, -1));
    act(&mut s, Act::Step(0, -1));
    act(&mut s, Act::Choice(1, 2));
    tap(&mut s, RIGHT);
    let r = s.pim().unwrap().get(3).unwrap();
    assert_eq!((r.num(tag::DUE), r.num(tag::PRIORITY)), (None, Some(2)));
}

#[test]
fn a_note_keeps_itself_on_every_way_out_and_edits_at_the_caret() {
    let mut s = organiser();
    open(&mut s, 4);
    tap(&mut s, RIGHT); // New
    assert_eq!(s.screen(), Screen::App(View::Note));
    type_text(&mut s, "gate cod");
    // Home, mid-note: kept.
    s.input(Input::Key(Key::Home));
    assert_eq!(titles(&mut s, Kind::Note, tag::TEXT), ["gate cod"]);
    // Back in: the caret at the end; left twice, a letter there.
    open(&mut s, 4);
    act(&mut s, Act::Open(1));
    s.set_focus(None);
    s.input(Input::Key(Key::Left));
    s.input(Input::Key(Key::Left));
    type_text(&mut s, "x");
    // The keyboard's big key is Return in a note.
    let big = (0..crate::keyboard::CAPS)
        .filter_map(|i| s.kb.cap(i))
        .find(|(c, _)| *c == crate::keyboard::Cap::Done)
        .unwrap()
        .1;
    tap(&mut s, big);
    tap(&mut s, RIGHT); // Done
    assert_eq!(titles(&mut s, Kind::Note, tag::TEXT), ["gate cx\nod"]);
    assert_eq!(s.screen(), Screen::App(View::Notes));
    // The list: its first line the title; the search finds text inside.
    assert!(words(&drawn(&mut s)).contains("|gate cx|"));
    act(&mut s, Act::Field(9));
    type_text(&mut s, "od");
    assert!(words(&drawn(&mut s)).contains("|gate cx|"));
    type_text(&mut s, "q");
    assert!(words(&drawn(&mut s)).contains("No note has that."));
    tap(&mut s, LEFT);
    // An empty new note isn't kept; one emptied is removed.
    tap(&mut s, RIGHT);
    tap(&mut s, RIGHT);
    assert_eq!(s.pim().unwrap().len(), 1);
    // Delete asks once.
    act(&mut s, Act::Open(1));
    tap(&mut s, LEFT);
    assert_eq!(s.pim().unwrap().len(), 1);
    assert!(words(&drawn(&mut s)).contains("Delete?"));
    tap(&mut s, LEFT);
    assert!(s.pim().unwrap().is_empty());
}

#[test]
fn the_calculators_keys_work_by_tap_and_by_the_d_pad() {
    let mut s = organiser();
    open(&mut s, 5);
    for c in "12+3x4".chars() {
        tap(&mut s, preview::calc_key(c).unwrap());
    }
    let shown = words(&drawn(&mut s));
    assert!(shown.contains("|12 + 3 x |4|"), "{shown}");
    tap(&mut s, preview::calc_key('=').unwrap());
    let shown = words(&drawn(&mut s));
    assert!(shown.contains("|12 + 3 x 4 =|24|"), "{shown}");
    // Leaving and coming back finds the sum as it was.
    s.input(Input::Key(Key::Home));
    open(&mut s, 5);
    assert!(words(&drawn(&mut s)).contains("|24|"));
    // A key press redraws the display and the key, not the screen.
    drawn(&mut s);
    tap(&mut s, preview::calc_key('7').unwrap());
    let f = drawn(&mut s);
    let area: u32 = f
        .dirty()
        .iter()
        .map(|r| u32::from(r.w) * u32::from(r.h))
        .sum();
    assert!(area < u32::from(W) * 200, "{:?}", f.dirty());
    // The D-pad's Enter presses the focused key.
    let five = s.app_find(Act::Key(rp_pim::calc::Key::Digit(5))).unwrap();
    s.set_focus(Some(five));
    s.input(Input::Key(Key::Enter));
    assert!(words(&drawn(&mut s)).contains("|75|"));
}

/// Every app target of the screen up can be reached from Start with the
/// D-pad alone.
fn all_reachable(s: &Shell) -> Result<(), String> {
    let mut seen = std::vec![Target::Start];
    let mut at = 0;
    while at < seen.len() {
        for k in [Key::Up, Key::Down, Key::Left, Key::Right] {
            if let Some(t) = s.neighbour(seen[at], k)
                && !seen.contains(&t)
            {
                seen.push(t);
            }
        }
        at += 1;
    }
    for i in 0..s.app_targets() {
        if !seen.contains(&Target::App(i)) {
            return Err(std::format!(
                "{:?} at {:?} can't be reached",
                s.app_act(i),
                s.app_rect(i)
            ));
        }
    }
    Ok(())
}

#[test]
fn every_screen_fits_a_frame_its_targets_are_big_enough_and_the_d_pad_reaches_them() {
    let mut s = organiser();
    let noon = date::start_of(today()) + 12 * 60;
    for k in 0..40u32 {
        add_event(
            &mut s,
            "Meeting with a long title that is cut",
            noon + k * 97,
            Some(15),
        );
        add_contact(
            &mut s,
            "Firstname",
            "Lastname-Hyphenated",
            "+1 (206) 555-0100",
        );
        s.pim()
            .unwrap()
            .put(
                Kind::Task,
                None,
                &[
                    (tag::TITLE, b"A task with quite a long title to cut"),
                    (tag::DUE, &n(today() as u32)),
                ],
            )
            .unwrap();
        s.pim()
            .unwrap()
            .put(
                Kind::Note,
                None,
                &[(tag::TEXT, &[b'w'; 900]), (tag::CHANGED, &n(noon))],
            )
            .unwrap();
    }
    s.pim_loaded();
    let check = |s: &mut Shell, what: &str| {
        let f = drawn(s);
        assert!(!f.overflowed(), "{what}: the frame overflowed");
        assert!(
            f.cmds().count() < MAX_CMDS,
            "{what}: {} commands",
            f.cmds().count()
        );
        for i in 0..s.app_targets() {
            let r = s.app_rect(i);
            assert!(
                r.x + r.w <= W && r.y + r.h <= H - 40,
                "{what}: {r:?} off the screen"
            );
            // A stylus target: 30 px, but the contacts' strip (17 px
            // letters, contacts-rule0.md) and a short appointment's box.
            let small = matches!(s.app_act(i), Act::Letter(_) | Act::Open(_));
            assert!(
                (r.w >= 30 && r.h >= 30) || (small && r.h >= 17 && r.w >= 22),
                "{what}: {:?} is {r:?}",
                s.app_act(i)
            );
        }
        all_reachable(s).unwrap_or_else(|e| panic!("{what}: {e}"));
    };
    open(&mut s, 1);
    check(&mut s, "agenda");
    act(&mut s, Act::Chip(View::Day));
    check(&mut s, "day");
    act(&mut s, Act::Chip(View::Month));
    check(&mut s, "month");
    tap(&mut s, RIGHT);
    check(&mut s, "new appointment");
    act(&mut s, Act::More);
    check(&mut s, "appointment editor");
    open(&mut s, 2);
    check(&mut s, "contacts");
    act(&mut s, Act::Field(9));
    check(&mut s, "contacts, searching");
    tap(&mut s, LEFT);
    act(&mut s, Act::Open(2));
    check(&mut s, "card");
    tap(&mut s, RIGHT);
    check(&mut s, "contact editor");
    open(&mut s, 3);
    check(&mut s, "tasks");
    act(&mut s, Act::Open(3));
    check(&mut s, "task editor");
    open(&mut s, 4);
    check(&mut s, "notes");
    // The newest note (the last added) is on top.
    act(&mut s, Act::Open(160));
    check(&mut s, "note");
    open(&mut s, 5);
    check(&mut s, "calculator");
}

#[test]
fn a_drag_scrolls_a_list_and_is_not_a_tap() {
    let mut s = organiser();
    for k in 0..30 {
        add_contact(&mut s, "A", &std::format!("Name{k:02}"), "");
    }
    open(&mut s, 2);
    drawn(&mut s);
    let first = s.app_find(Act::Open(1)).map(|t| s.rect_of(t)).unwrap();
    s.input(Input::PenDown { x: 200, y: 400 });
    assert_eq!(
        s.input(Input::PenMove { x: 200, y: 396 }),
        Action::Nothing,
        "inside the slop"
    );
    assert_eq!(s.input(Input::PenMove { x: 200, y: 300 }), Action::Redraw);
    assert_eq!(s.input(Input::PenUp { x: 200, y: 300 }), Action::Nothing);
    assert_eq!(
        s.screen(),
        Screen::App(View::Contacts),
        "the drag opened nothing"
    );
    assert!(s.app_find(Act::Open(1)).map(|t| s.rect_of(t)) != Some(first));
    // Past the end it stops; back past the start it stops.
    for _ in 0..20 {
        s.input(Input::PenDown { x: 200, y: 500 });
        s.input(Input::PenMove { x: 200, y: 200 });
        s.input(Input::PenUp { x: 200, y: 200 });
    }
    assert!(s.app_find(Act::Open(30)).is_some(), "the last row shows");
    for _ in 0..20 {
        s.input(Input::PenDown { x: 200, y: 200 });
        s.input(Input::PenMove { x: 200, y: 560 });
        s.input(Input::PenUp { x: 200, y: 560 });
    }
    assert_eq!(s.app_find(Act::Open(1)).map(|t| s.rect_of(t)), Some(first));
    // The D-pad walks down the list and the list follows.
    s.input(Input::Key(Key::Down));
    for _ in 0..40 {
        s.input(Input::Key(Key::Down));
    }
    assert!(
        s.app_find(Act::Open(30)).is_some()
            || matches!(s.focus(), Some(Target::SoftLeft | Target::SoftRight))
    );
}

#[test]
fn the_day_turning_moves_the_agenda_on() {
    let mut s = organiser();
    let noon = date::start_of(today()) + 12 * 60;
    add_event(&mut s, "Today's", noon + 60, None);
    add_event(&mut s, "Tomorrow's", noon + date::DAY_MIN, None);
    open(&mut s, 1);
    let shown = words(&drawn(&mut s));
    assert!(shown.contains("TODAY - MON 1 JAN") && shown.contains("TOMORROW - TUE 2 JAN"));
    s.set_time(Time {
        weekday: 1,
        day: 2,
        ..NOON
    });
    let shown = words(&drawn(&mut s));
    assert!(
        shown.contains("TODAY - TUE 2 JAN") && !shown.contains("Today's"),
        "{shown}"
    );
    // The year, once said, is the year used.
    s.set_year(2026);
    s.set_time(Time {
        weekday: 4,
        day: 13,
        month: 11,
        ..NOON
    });
    assert!(words(&drawn(&mut s)).contains("FRI 13 NOVEMBER 2026"));
}
