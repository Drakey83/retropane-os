//! The organiser apps' version 2 screens for `cargo xtask ui-preview`
//! (design-language.md version 2; each app's sheet): the app programs
//! themselves (rp-app), hosted by the shell and drawn by the image's own 2D
//! renderer code, with sample records. So a "mock-up" here is the real
//! program's screen, and only what the X51v draws can appear in it.

use rp_app::DayWeather;
use rp_app::kit::IDENTITY;
use rp_app_calculator::{Calculator, Mode};
use rp_app_calendar::{Appt, Cal, Calendar, View};
use rp_app_contacts::{Contacts, EditField, Person, Place};
use rp_app_notes::{Block, Note, Notes};
use rp_app_tasks::{List, Task, Tasks};

/// One mock-up: a file name, what it shows, and how the shell is set up.
pub struct Mock {
    /// The PNG's name.
    pub name: &'static str,
    /// What it shows.
    pub what: &'static str,
    /// The shell made ready to show it.
    pub prepare: Box<dyn Fn(&mut rp_shell::Shell)>,
}

/// The previews' today: Monday 1 January 2024 (their clock reads noon).
const TODAY: i32 = 19_723;

static CALS: [Cal; 4] = [
    Cal {
        name: b"Personal",
        colour: IDENTITY[0],
    },
    Cal {
        name: b"Work",
        colour: IDENTITY[4],
    },
    Cal {
        name: b"Family",
        colour: IDENTITY[2],
    },
    Cal {
        name: b"Birthdays",
        colour: IDENTITY[7],
    },
];

fn appt(
    day: i32,
    start: u16,
    len: u16,
    title: &'static [u8],
    place: &'static [u8],
    cal: u8,
) -> Appt {
    Appt {
        day: TODAY + day,
        start,
        len,
        title,
        place,
        cal,
        repeats: false,
    }
}

/// The sample week.
fn calendar(view: View, line: &[u8]) -> Calendar {
    let mut c = Calendar::new(&CALS, TODAY);
    for a in [
        appt(0, 9 * 60 + 30, 30, b"Stand-up", b"Zoom", 1),
        appt(0, 12 * 60 + 30, 60, b"Lunch with Sam", b"Cafe Vita", 0),
        appt(0, 15 * 60, 60, b"Dentist", b"Dr. Lee, 410 Pine St", 0),
        appt(0, 15 * 60 + 30, 45, b"Review with Jo", b"Room 4B", 1),
        appt(0, 18 * 60 + 30, 90, b"Book club", b"Library", 2),
        appt(1, 8 * 60, 120, b"Car service", b"Main St garage", 0),
        appt(1, 18 * 60, 60, b"Gym", b"City gym", 0),
        appt(2, 0, 0, b"Mum's birthday", b"", 3),
        appt(2, 10 * 60, 60, b"Design review", b"Room 2", 1),
        appt(
            3,
            7 * 60 + 15,
            135,
            b"Flight to Portland",
            b"SEA gate C9",
            0,
        ),
        appt(4, 19 * 60, 120, b"Dinner with the Parks", b"Their place", 2),
        appt(6, 11 * 60, 60, b"Brunch", b"The Diner", 2),
    ] {
        c.add(a);
    }
    c.show(view);
    c.set_line(line);
    c
}

/// The forecast the shell lends: five days of the previews' weather with
/// the Weather app's own row icons.
fn forecast(shell: &mut rp_shell::Shell) {
    let icons = [3u8, 1, 9, 4, 2];
    let hi_lo = [(54, 41), (58, 43), (49, 40), (51, 38), (56, 42)];
    let days: Vec<DayWeather> = (0..5)
        .map(|i| DayWeather {
            day: TODAY + i,
            high: hi_lo[i as usize].0,
            low: hi_lo[i as usize].1,
            icon: shell.row_icon(icons[i as usize]),
        })
        .collect();
    shell.set_app_forecast(&days);
}

fn open(shell: &mut rp_shell::Shell, c: impl rp_app::Program + 'static) {
    shell.set_appearance(rp_weather::settings::Appearance::Dark);
    shell.give_pics(crate::emu::preview_pics());
    forecast(shell);
    shell.open_program(Box::leak(Box::new(c)));
}

fn person(
    first: &'static [u8],
    last: &'static [u8],
    company: &'static [u8],
    mobile: &'static [u8],
    fav: bool,
) -> Person {
    Person {
        first,
        last,
        company,
        phones: [(b"MOBILE", mobile), (b"HOME", b"")],
        email: b"",
        address: [b"", b""],
        favourite: fav,
    }
}

fn contacts(view: rp_app_contacts::View) -> Contacts {
    let mut c = Contacts::new();
    for p in [
        Person {
            email: b"grace@example.org",
            address: [b"2100 Clarendon Blvd", b"Arlington, VA 22201"],
            phones: [(b"MOBILE", b"+17035550107"), (b"WORK", b"+17035550155")],
            ..person(b"Grace", b"Hopper", b"US Navy", b"", true)
        },
        person(
            b"Ada",
            b"Lovelace",
            b"Analytical Engines",
            b"+12065550142",
            true,
        ),
        person(b"Alan", b"Turing", b"", b"+442079460958", true),
        person(
            b"Katherine",
            b"Johnson",
            b"NASA Langley",
            b"+17575550116",
            true,
        ),
        person(b"Edsger", b"Dijkstra", b"", b"+15125550133", false),
        person(b"Margaret", b"Hamilton", b"MIT", b"+16175550111", false),
        person(b"Donald", b"Knuth", b"Stanford", b"+16505550188", false),
        person(b"Barbara", b"Liskov", b"MIT", b"+16175550170", false),
        person(b"Hedy", b"Lamarr", b"", b"+13105550105", false),
    ] {
        c.add(p);
    }
    c.show(view);
    c
}

static LISTS: [List; 4] = [
    List {
        name: b"Home",
        colour: IDENTITY[2],
    },
    List {
        name: b"Work",
        colour: IDENTITY[4],
    },
    List {
        name: b"Errands",
        colour: IDENTITY[1],
    },
    List {
        name: b"Trip to Portland",
        colour: IDENTITY[6],
    },
];

fn task(title: &'static [u8], due: Option<i32>, flag: u8, list: u8, done: bool) -> Task {
    Task {
        title,
        due: due.map(|d| TODAY + d),
        flag,
        list,
        done,
        steps: (0, 0),
        repeats: false,
    }
}

fn tasks(view: rp_app_tasks::View, line: &[u8]) -> Tasks {
    let mut t = Tasks::new(&LISTS);
    for x in [
        task(b"Renew passport", Some(-2), 3, 3, false),
        Task {
            repeats: true,
            ..task(b"Pay rent", Some(0), 3, 0, false)
        },
        task(b"Call the bank", Some(0), 0, 2, false),
        Task {
            steps: (2, 5),
            ..task(b"Pack for Portland", Some(1), 2, 3, false)
        },
        task(b"Book flights", Some(1), 2, 3, false),
        task(b"Water the plants", Some(4), 0, 0, false),
        task(b"Quarterly report", Some(3), 2, 1, false),
        task(b"Fix the gate", Some(20), 0, 0, false),
        task(b"Return library books", Some(-1), 0, 2, true),
        task(b"Buy stamps", Some(0), 0, 2, true),
        task(b"Email Jo the slides", Some(0), 0, 1, true),
    ] {
        t.add(x);
    }
    t.show(view);
    t.set_line(line);
    t
}

/// A handwriting-like line: a looping trochoid across the box (the shape
/// cursive makes), and a circled sketch.
fn cursive(y: i16, x0: i16, x1: i16) -> Vec<(i16, i16)> {
    (0..=((x1 - x0) / 3))
        .map(|k| {
            let t = f32::from(k) * 0.42;
            let x = f32::from(x0) + f32::from(k) * 3.0 + 7.0 * t.cos();
            let yy = f32::from(y) - 9.0 * t.sin() - 4.0 * (t * 0.31).sin();
            (x as i16, yy as i16)
        })
        .collect()
}

fn ink() -> &'static [&'static [(i16, i16)]] {
    let mut strokes: Vec<&'static [(i16, i16)]> = vec![
        Box::leak(cursive(40, 20, 210).into_boxed_slice()),
        Box::leak(cursive(40, 236, 330).into_boxed_slice()),
        Box::leak(cursive(92, 20, 150).into_boxed_slice()),
        Box::leak(cursive(92, 176, 300).into_boxed_slice()),
    ];
    // A circle around a sketch and an arrow to it.
    let circle: Vec<(i16, i16)> = (0..=40i16)
        .map(|k| {
            let a = f32::from(k) * 0.165;
            (
                (390.0 + 40.0 * a.cos()) as i16,
                (130.0 + 34.0 * a.sin()) as i16,
            )
        })
        .collect();
    strokes.push(Box::leak(circle.into_boxed_slice()));
    strokes.push(Box::leak(
        vec![(250, 160), (300, 150), (346, 136)].into_boxed_slice(),
    ));
    strokes.push(Box::leak(
        vec![(334, 128), (348, 136), (336, 146)].into_boxed_slice(),
    ));
    strokes.push(Box::leak(
        vec![(372, 146), (378, 112), (392, 126), (404, 108), (410, 146)].into_boxed_slice(),
    ));
    Box::leak(strokes.into_boxed_slice())
}

fn notes(view: rp_app_notes::View) -> Notes {
    let ink = ink();
    let blocks = |b: Vec<Block>| -> &'static [Block] { Box::leak(b.into_boxed_slice()) };
    let list: &'static [Note] = Box::leak(
        vec![
            Note {
                title: b"Portland trip",
                blocks: blocks(vec![
                    Block::Text(b"Flight Thu 7:15 AM, gate C9"),
                    Block::Check(b"Book the hotel", true),
                    Block::Check(b"Rain jacket", false),
                    Block::Check(b"Charger for the Axim", false),
                    Block::Ink(ink),
                ]),
                colour: IDENTITY[6],
                pinned: true,
            },
            Note {
                title: b"Ideas for the garden",
                blocks: blocks(vec![Block::Ink(ink)]),
                colour: IDENTITY[2],
                pinned: true,
            },
            Note {
                title: b"Shopping",
                blocks: blocks(vec![
                    Block::Check(b"Milk", true),
                    Block::Check(b"Coffee beans", false),
                    Block::Check(b"Stamps", false),
                ]),
                colour: IDENTITY[1],
                pinned: false,
            },
            Note {
                title: b"Book club",
                blocks: blocks(vec![
                    Block::Text(b"Next: The Left Hand of Darkness"),
                    Block::Text(b"Bring the cheese board"),
                ]),
                colour: IDENTITY[0],
                pinned: false,
            },
            Note {
                title: b"Wi-Fi at the library",
                blocks: blocks(vec![Block::Text(b"Network: Library-Guest")]),
                colour: IDENTITY[4],
                pinned: false,
            },
            Note {
                title: b"Gate measurements",
                blocks: blocks(vec![
                    Block::Text(b"Width 46 in, height 52 in"),
                    Block::Ink(ink),
                ]),
                colour: IDENTITY[3],
                pinned: false,
            },
        ]
        .into_boxed_slice(),
    );
    let mut n = Notes::new(list);
    n.show(view);
    n
}

static TAPE: [(&[u8], &[u8]); 2] = [
    (b"1,250 x 12 =", b"15,000"),
    (b"15,000 / 52 =", b"288.461538462"),
];

/// Every version 2 mock-up.
pub fn mocks() -> Vec<Mock> {
    vec![
        Mock {
            name: "v2-calendar-agenda",
            what: "Calendar: the agenda under the large title and the week strip, the forecast on each day",
            prepare: Box::new(|s| open(s, calendar(View::Agenda, b""))),
        },
        Mock {
            name: "v2-calendar-day",
            what: "Calendar: Day, blocks sized to their length, overlaps side by side, the now line",
            prepare: Box::new(|s| open(s, calendar(View::Day, b""))),
        },
        Mock {
            name: "v2-calendar-week",
            what: "Calendar: Week, seven columns of coloured blocks",
            prepare: Box::new(|s| open(s, calendar(View::Week, b""))),
        },
        Mock {
            name: "v2-calendar-month",
            what: "Calendar: Month with coloured bars and the chosen day's list",
            prepare: Box::new(|s| open(s, calendar(View::Month, b""))),
        },
        Mock {
            name: "v2-calendar-new",
            what: "Calendar: quick add, the understood words in the tint, the preview card",
            prepare: Box::new(|s| open(s, calendar(View::New, b"Dentist tue 3pm 1h"))),
        },
        Mock {
            name: "v2-contacts-list",
            what: "Contacts: favourites grid, avatars with initials, formatted numbers, the letter strip",
            prepare: Box::new(|s| open(s, contacts(rp_app_contacts::View::List))),
        },
        Mock {
            name: "v2-contacts-card",
            what: "Contacts: the card with its action row and grouped details",
            prepare: Box::new(|s| {
                open(s, contacts(rp_app_contacts::View::Card(0)));
            }),
        },
        Mock {
            name: "v2-contacts-phone",
            what: "Contacts: the phone pad, the number formatted as typed",
            prepare: Box::new(|s| {
                let mut c = contacts(rp_app_contacts::View::Edit(EditField::Mobile));
                c.typed(b"Mae", b"2065550", b"");
                open(s, c);
            }),
        },
        Mock {
            name: "v2-contacts-address",
            what: "Contacts: address suggestions while typing, capitals at each word",
            prepare: Box::new(|s| {
                let mut c = contacts(rp_app_contacts::View::Edit(EditField::Street));
                c.typed(b"Mae", b"2065550142", b"410 Pi");
                c.set_places(&[
                    Place {
                        street: b"410 Pine St",
                        city: b"Seattle, WA 98101",
                    },
                    Place {
                        street: b"410 Pike St",
                        city: b"Seattle, WA 98101",
                    },
                    Place {
                        street: b"410 Pinehurst Way NE",
                        city: b"Seattle, WA 98125",
                    },
                ]);
                open(s, c);
            }),
        },
        Mock {
            name: "v2-tasks-home",
            what: "Tasks: smart tiles with counts over the coloured lists and their progress",
            prepare: Box::new(|s| open(s, tasks(rp_app_tasks::View::Home, b""))),
        },
        Mock {
            name: "v2-tasks-today",
            what: "Tasks: Today, late first, round ticks in the list's colour, flags, subtasks",
            prepare: Box::new(|s| open(s, tasks(rp_app_tasks::View::List(None), b""))),
        },
        Mock {
            name: "v2-tasks-add",
            what: "Tasks: the add line, the understood words in the tint",
            prepare: Box::new(|s| {
                open(
                    s,
                    tasks(
                        rp_app_tasks::View::List(None),
                        b"Pay rent fri !1 every month #home",
                    ),
                )
            }),
        },
        Mock {
            name: "v2-notes-home",
            what: "Notes: pinned first, cards in their colours with ink miniatures",
            prepare: Box::new(|s| open(s, notes(rp_app_notes::View::Home))),
        },
        Mock {
            name: "v2-notes-note",
            what: "Notes: text, a checklist and handwriting in one note on ruled paper",
            prepare: Box::new(|s| open(s, notes(rp_app_notes::View::Note(0)))),
        },
        Mock {
            name: "v2-calc-basic",
            what: "Calculator: the tape, the sum with its answer live, memory keys, orange operators",
            prepare: Box::new(|s| {
                open(
                    s,
                    Calculator::new(Mode::Basic, b"288.46 + 15% =", b"331.73", &TAPE, true),
                )
            }),
        },
        Mock {
            name: "v2-calc-sci",
            what: "Calculator: the scientific keys",
            prepare: Box::new(|s| {
                open(
                    s,
                    Calculator::new(
                        Mode::Scientific,
                        b"sin(30) + 2^10",
                        b"1,024.5",
                        &TAPE,
                        false,
                    ),
                )
            }),
        },
        Mock {
            name: "v2-calc-money",
            what: "Calculator: money at the ECB's daily rates, kept for offline",
            prepare: Box::new(|s| open(s, Calculator::new(Mode::Money, b"", b"", &[], false))),
        },
        Mock {
            name: "v2-calc-tip",
            what: "Calculator: tip and split",
            prepare: Box::new(|s| open(s, Calculator::new(Mode::Tip, b"", b"", &[], false))),
        },
        Mock {
            name: "v2-calc-dates",
            what: "Calculator: days between dates",
            prepare: Box::new(|s| open(s, Calculator::new(Mode::Dates, b"", b"", &[], false))),
        },
    ]
}
