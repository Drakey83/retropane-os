//! The organiser apps' screens for `cargo xtask ui-preview` and the
//! emulator check (batch 1 of docs/design/core-apps-list.md): sample
//! records, and the taps that lead to every screen each app shows.
//!
//! The previews' clock reads noon on Monday 1 January; with no year given
//! the shell takes 2024, whose 1 January was a Monday.

use rp_pim::date::{self, Date};
use rp_pim::store::{Kind, tag};
use rp_shell::{Input, Key, Shell, View};

/// One preview: a file name, what it shows, and the inputs after the
/// previous one that lead to it.
pub struct Shot {
    /// The PNG's name.
    pub name: &'static str,
    /// What it shows.
    pub what: &'static str,
    /// The inputs since the previous shot.
    pub steps: Vec<Input>,
}

fn tap(x: u16, y: u16) -> Vec<Input> {
    vec![Input::PenDown { x, y }, Input::PenUp { x, y }]
}

/// A tap in the middle of `r`.
fn tap_in(r: rp_draw::Rect) -> Vec<Input> {
    tap(r.x + r.w / 2, r.y + r.h / 2)
}

/// Start, then the menu's entry `i`.
fn menu(i: u8) -> Vec<Input> {
    [tap_in(rp_shell::START), tap_in(Shell::menu_entry(i))].concat()
}

/// Taps on the on-screen keyboard that type `text` (lower-case letters,
/// digits, space, and `!`, `:`, `/`, `.` through the symbols layer).
fn typed(text: &str) -> Vec<Input> {
    let mut out = Vec::new();
    for c in text.bytes() {
        let (keys, n) = rp_shell::apps::preview::key_taps(c)
            .unwrap_or_else(|| panic!("no key for {}", c as char));
        for step in &keys[..n] {
            out.extend(tap_in(*step));
        }
    }
    out
}

/// Taps on the calculator's keys, written as their signs (`C` clear).
fn calc_keys(keys: &str) -> Vec<Input> {
    keys.chars()
        .flat_map(|c| {
            tap_in(
                rp_shell::apps::preview::calc_key(c)
                    .unwrap_or_else(|| panic!("no calculator key {c}")),
            )
        })
        .collect()
}

/// The softkeys.
const LEFT: (u16, u16) = (80, 620);
const RIGHT: (u16, u16) = (400, 620);

/// The day `n` days after the previews' today.
fn day(n: i32) -> i32 {
    date::days_from(Date {
        year: 2024,
        month: 1,
        day: 1,
    }) + n
}

/// The sample records, in a store loaded as from an empty card.
pub fn sample(shell: &mut Shell) {
    crate::emu::empty_organiser(shell);
    let Some(s) = shell.pim() else { return };
    let n = |v: u32| v.to_le_bytes();
    let at = |d: i32, h: u32, m: u32| n(date::start_of(day(d)) + h * 60 + m);
    for (title, place, start, len, remind) in [
        ("Dentist", "Dr. Lee", at(0, 15, 0), 60, Some(15)),
        ("Lunch with Sam", "Cafe Vita", at(0, 12, 30), 45, None),
        ("Stand-up", "", at(1, 9, 30), 15, Some(5)),
        ("Car service", "Main St garage", at(1, 14, 0), 90, Some(60)),
        ("Book club", "Library", at(3, 18, 30), 120, Some(30)),
        ("Flight to Portland", "SEA", at(8, 7, 15), 60, Some(1440)),
        ("Review with Jo", "", at(0, 15, 30), 30, None),
    ] {
        let fields: Vec<(u8, Vec<u8>)> = [
            (tag::TITLE, title.as_bytes().to_vec()),
            (tag::PLACE, place.as_bytes().to_vec()),
            (tag::START, start.to_vec()),
            (tag::MINUTES, n(len).to_vec()),
        ]
        .into_iter()
        .chain(remind.map(|r: u32| (tag::REMIND, n(r).to_vec())))
        .collect();
        let refs: Vec<(u8, &[u8])> = fields.iter().map(|(t, v)| (*t, v.as_slice())).collect();
        s.put(Kind::Appointment, None, &refs)
            .expect("sample appointment");
    }
    s.put(
        Kind::Appointment,
        None,
        &[
            (tag::TITLE, b"Mum's birthday"),
            (tag::START, &n(date::start_of(day(2)))),
            (tag::ALL_DAY, &n(1)),
        ],
    )
    .expect("sample all-day");
    for (first, last, company, mobile, home, email, address) in [
        (
            "Ada",
            "Lovelace",
            "Analytical Engines",
            "206 555 0142",
            "",
            "ada@example.org",
            "12 Byron Row, London",
        ),
        (
            "Alan",
            "Turing",
            "",
            "206 555 0199",
            "",
            "alan@example.org",
            "",
        ),
        (
            "Grace",
            "Hopper",
            "US Navy",
            "703 555 0107",
            "703 555 0155",
            "grace@example.org",
            "2100 Clarendon Blvd, Arlington, Virginia 22201",
        ),
        ("Dennis", "Ritchie", "Bell Labs", "908 555 0121", "", "", ""),
        ("Ken", "Thompson", "Bell Labs", "908 555 0122", "", "", ""),
        ("Barbara", "Liskov", "MIT", "617 555 0170", "", "", ""),
        ("Edsger", "Dijkstra", "", "", "512 555 0133", "", ""),
        (
            "Margaret",
            "Hamilton",
            "Draper Lab",
            "617 555 0111",
            "",
            "",
            "",
        ),
        ("Linus", "Torvalds", "", "503 555 0164", "", "", ""),
        ("Donald", "Knuth", "Stanford", "650 555 0188", "", "", ""),
        ("John", "Doe", "", "425 555 0100", "", "", ""),
        ("Radia", "Perlman", "", "", "", "radia@example.org", ""),
        ("Tim", "Berners-Lee", "W3C", "617 555 0190", "", "", ""),
        ("Hedy", "Lamarr", "", "310 555 0105", "", "", ""),
        (
            "",
            "",
            "Vita Coffee",
            "206 555 0177",
            "",
            "",
            "1005 E Pike St",
        ),
        ("Katherine", "Johnson", "NASA", "757 555 0116", "", "", ""),
        ("Steve", "Wozniak", "", "408 555 0101", "", "", ""),
        (
            "Sophie",
            "Wilson",
            "Acorn",
            "",
            "",
            "sophie@example.org",
            "",
        ),
    ] {
        s.put(
            Kind::Contact,
            None,
            &[
                (tag::FIRST, first.as_bytes()),
                (tag::LAST, last.as_bytes()),
                (tag::COMPANY, company.as_bytes()),
                (tag::MOBILE, mobile.as_bytes()),
                (tag::HOME, home.as_bytes()),
                (tag::EMAIL, email.as_bytes()),
                (tag::ADDRESS, address.as_bytes()),
            ],
        )
        .expect("sample contact");
    }
    for (title, due, priority, done) in [
        ("Renew passport", Some(-2), 2, false),
        ("Pay rent", Some(0), 2, false),
        ("Call the bank", Some(0), 1, false),
        ("Book flights", Some(1), 1, false),
        ("Water the plants", Some(4), 1, false),
        ("Sort the photos", None, 0, false),
        ("Fix the gate", Some(20), 1, false),
        ("Return library books", Some(-1), 1, true),
    ] {
        let due_b = due.map(|d: i32| n(day(d) as u32));
        let done_b = n(date::start_of(day(0)) + 600);
        let pri = n(priority);
        let mut fields: Vec<(u8, &[u8])> =
            vec![(tag::TITLE, title.as_bytes()), (tag::PRIORITY, &pri)];
        if let Some(d) = &due_b {
            fields.push((tag::DUE, d));
        }
        if done {
            fields.push((tag::DONE, &done_b));
        }
        s.put(Kind::Task, None, &fields).expect("sample task");
    }
    for (text, when) in [
        ("Gate code\n4417, then the star key", (0, 9, 5)),
        (
            "Shopping\nMilk, eggs, coffee, a 9 V battery for the smoke alarm, and stamps",
            (-1, 18, 40),
        ),
        (
            "X51v ideas\nA standby clock face from the panel's 8-colour mode.\nTry the cradle's serial port for a GPS receiver.",
            (-3, 21, 15),
        ),
    ] {
        let (d, h, m) = when;
        s.put(
            Kind::Note,
            None,
            &[(tag::TEXT, text.as_bytes()), (tag::CHANGED, &at(d, h, m))],
        )
        .expect("sample note");
    }
    shell.pim_loaded();
}

/// Every screen the five apps show, from the Today screen.
pub fn shots() -> Vec<Shot> {
    use rp_shell::apps::preview as p;
    let key = |k: Key| vec![Input::Key(k)];
    vec![
        Shot {
            name: "a01-start-menu",
            what: "The Start menu with the five apps",
            steps: tap_in(rp_shell::START),
        },
        Shot {
            name: "a02-calendar-agenda",
            what: "Calendar: the agenda (the first view)",
            steps: tap_in(Shell::menu_entry(1)),
        },
        Shot {
            name: "a03-calendar-day",
            what: "Calendar: the Day chip: today by the hour, two appointments side by side",
            steps: tap_in(p::chip(View::Day)),
        },
        Shot {
            name: "a04-calendar-day-scrolled",
            what: "Day, dragged up: the afternoon",
            steps: vec![
                Input::PenDown { x: 240, y: 520 },
                Input::PenMove { x: 240, y: 400 },
                Input::PenMove { x: 240, y: 280 },
                Input::PenUp { x: 240, y: 280 },
            ],
        },
        Shot {
            name: "a05-calendar-month",
            what: "Calendar: the Month chip: dots, today ringed, the chosen day's list",
            steps: tap_in(p::chip(View::Month)),
        },
        Shot {
            name: "a06-calendar-month-day",
            what: "Month: the 9th tapped: its appointment under the grid",
            steps: tap_in(p::month_cell(8)),
        },
        Shot {
            name: "a07-calendar-new",
            what: "New: the quick-add line on the keyboard, its defaults previewed",
            steps: tap(RIGHT.0, RIGHT.1),
        },
        Shot {
            name: "a08-calendar-new-typed",
            what: "Typed \"gym thu 6pm 1h\": what it understood, before Add",
            steps: typed("gym thu 6pm 1h"),
        },
        Shot {
            name: "a09-calendar-more",
            what: "More details: the editor with the line's reading (steppers)",
            steps: tap_in(p::MORE),
        },
        Shot {
            name: "a10-calendar-edit-typing",
            what: "Where tapped: typing a text field, the row kept above the keyboard",
            steps: [tap_in(p::form_row(1)), typed("city gym")].concat(),
        },
        Shot {
            name: "a11-calendar-added",
            what: "Done: the appointment in the agenda under Thursday",
            steps: tap(RIGHT.0, RIGHT.1),
        },
        Shot {
            name: "a12-calendar-edit",
            what: "Dentist tapped: its details, with Delete",
            steps: tap_in(p::agenda_row(1)),
        },
        Shot {
            name: "a13-calendar-delete-ask",
            what: "Delete tapped once: it asks in place",
            steps: tap_in(p::form_row(7)),
        },
        Shot {
            name: "a14-contacts",
            what: "Cancel, then Contacts from the Start menu: the list and the letter strip",
            steps: [tap(LEFT.0, LEFT.1), menu(2)].concat(),
        },
        Shot {
            name: "a15-contacts-letter",
            what: "The strip's T tapped: the list at T",
            steps: tap_in(p::letter(b'T')),
        },
        Shot {
            name: "a16-contacts-search",
            what: "The search field: \"617\" finds numbers; the list filters as it is typed",
            steps: [tap_in(p::FIELD), typed("617")].concat(),
        },
        Shot {
            name: "a17-contacts-initials",
            what: "Cleared, then \"gh\": initials find Grace Hopper",
            steps: [tap(LEFT.0, LEFT.1), tap_in(p::FIELD), typed("gh")].concat(),
        },
        Shot {
            name: "a18-contact-card",
            what: "Done, then her row: the card, only the fields that are filled",
            steps: [tap(RIGHT.0, RIGHT.1), tap_in(p::list_row(0))].concat(),
        },
        Shot {
            name: "a19-contact-edit",
            what: "Edit: the nine rows",
            steps: tap(RIGHT.0, RIGHT.1),
        },
        Shot {
            name: "a20-contact-new",
            what: "Cancel, Back, clear the search, New: a new card, the keyboard at First",
            steps: [
                tap(LEFT.0, LEFT.1),
                tap(LEFT.0, LEFT.1),
                tap_in(p::FIELD),
                tap(LEFT.0, LEFT.1),
                tap(RIGHT.0, RIGHT.1),
                typed("mae"),
            ]
            .concat(),
        },
        Shot {
            name: "a21-tasks",
            what: "Tasks from the Start menu: late first, done at the bottom",
            steps: [tap(LEFT.0, LEFT.1), tap(LEFT.0, LEFT.1), menu(3)].concat(),
        },
        Shot {
            name: "a22-tasks-add",
            what: "The add field: \"buy stamps fri !\"",
            steps: [tap_in(p::FIELD), typed("buy stamps fri !")].concat(),
        },
        Shot {
            name: "a23-tasks-added",
            what: "Add, the keyboard down: the new task in its place",
            steps: [tap(RIGHT.0, RIGHT.1), tap(LEFT.0, LEFT.1)].concat(),
        },
        Shot {
            name: "a24-tasks-ticked",
            what: "Pay rent's box tapped: done, at the bottom",
            steps: tap_in(p::task_box(1)),
        },
        Shot {
            name: "a25-tasks-hide-done",
            what: "Hide done",
            steps: tap(RIGHT.0, RIGHT.1),
        },
        Shot {
            name: "a26-task-edit",
            what: "A task's row: its editor (a due stepper, priority chips)",
            steps: tap_in(p::list_row(1)),
        },
        Shot {
            name: "a27-notes",
            what: "Cancel, then Notes from the Start menu: newest first",
            steps: [tap(LEFT.0, LEFT.1), menu(4)].concat(),
        },
        Shot {
            name: "a28-notes-search",
            what: "Search \"battery\": the note that holds it",
            steps: [tap_in(p::FIELD), typed("battery")].concat(),
        },
        Shot {
            name: "a29-note",
            what: "Done, then the note: its text wrapped, the caret at its end",
            steps: [tap(RIGHT.0, RIGHT.1), tap_in(p::note_row(0))].concat(),
        },
        Shot {
            name: "a30-note-caret",
            what: "A tap in the second line, then typing there",
            steps: [tap(200, 144), typed("fresh ")].concat(),
        },
        Shot {
            name: "a31-note-new",
            what: "Done, clear the search, New: an empty note",
            steps: [
                tap(RIGHT.0, RIGHT.1),
                tap_in(p::FIELD),
                tap(LEFT.0, LEFT.1),
                tap(RIGHT.0, RIGHT.1),
            ]
            .concat(),
        },
        Shot {
            name: "a32-note-delete-ask",
            what: "Typed, then Delete once: the softkey asks",
            steps: [typed("call mae about the gate"), tap(LEFT.0, LEFT.1)].concat(),
        },
        Shot {
            name: "a33-calculator",
            what: "Done (the note kept), then the Calculator from the Start menu",
            steps: [tap(RIGHT.0, RIGHT.1), menu(5)].concat(),
        },
        Shot {
            name: "a34-calculator-sum",
            what: "12 + 3 x 4: the sum above, the number below",
            steps: calc_keys("12+3x4"),
        },
        Shot {
            name: "a35-calculator-equals",
            what: "Equals: 24, with its question above",
            steps: calc_keys("="),
        },
        Shot {
            name: "a36-calculator-decimal",
            what: "C, then 0.1 + 0.2 =: 0.3",
            steps: calc_keys("C0.1+0.2="),
        },
        Shot {
            name: "a37-calculator-error",
            what: "C, 5 / 0 =: the error in words",
            steps: calc_keys("C5/0="),
        },
        Shot {
            name: "a38-calculator-focus",
            what: "C, 1234567.5, then the D-pad: the focus ring on a key",
            steps: [
                calc_keys("C1234567.5"),
                key(Key::Down),
                key(Key::Down),
                key(Key::Down),
                key(Key::Down),
                key(Key::Right),
            ]
            .concat(),
        },
        Shot {
            name: "a39-today-note-button",
            what: "Home, then Today's Note button: a new note at once",
            steps: [key(Key::Home), tap(58, 558)].concat(),
        },
    ]
}
