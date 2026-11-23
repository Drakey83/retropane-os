//! The organiser apps' version 2 screens for `cargo xtask ui-preview`
//! (design-language.md version 2; each app's sheet): the app programs
//! themselves (rp-app), hosted by the shell and drawn by the image's own 2D
//! renderer code, with sample records. So a "mock-up" here is the real
//! program's screen, and only what the X51v draws can appear in it.

use rp_app::DayWeather;
use rp_app::kit::IDENTITY;
use rp_app_calculator::{Calculator, Mode};
use rp_app_calendar::{Cal, Calendar, View};
use rp_app_contacts::{Contacts, EditField};
use rp_app_notes::{Block, Note, Notes};
use rp_app_tasks::{List, Tasks};

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

pub(crate) static CALS: [Cal; 4] = [
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

/// The sample week's appointments: (day from today, start minute,
/// length (0: all day), title, place, calendar), as records of the store.
/// One sample appointment.
type Sample = (i32, u32, u32, &'static [u8], &'static [u8], usize);

const WEEK: [Sample; 12] = [
    (0, 9 * 60 + 30, 30, b"Stand-up", b"Zoom", 1),
    (0, 12 * 60 + 30, 60, b"Lunch with Sam", b"Cafe Vita", 0),
    (0, 15 * 60, 60, b"Dentist", b"Dr. Lee, 410 Pine St", 0),
    (0, 15 * 60 + 30, 45, b"Review with Jo", b"Room 4B", 1),
    (0, 18 * 60 + 30, 90, b"Book club", b"Library", 2),
    (1, 8 * 60, 120, b"Car service", b"Main St garage", 0),
    (1, 18 * 60, 60, b"Gym", b"City gym", 0),
    (2, 0, 0, b"Mum's birthday", b"", 3),
    (2, 10 * 60, 60, b"Design review", b"Room 2", 1),
    (
        3,
        7 * 60 + 15,
        135,
        b"Flight to Portland",
        b"SEA gate C9",
        0,
    ),
    (4, 19 * 60, 120, b"Dinner with the Parks", b"Their place", 2),
    (6, 11 * 60, 60, b"Brunch", b"The Diner", 2),
];

/// The shell's store given and loaded (empty), then the four calendars
/// and the sample week put in it, as the app keeps them.
pub fn sample_week(s: &mut rp_shell::Shell) {
    crate::emu::empty_organiser(s);
    put_week(s);
}

/// The sample calendars and week put in the shell's store as it is.
fn put_week(s: &mut rp_shell::Shell) {
    use rp_pim::store::{Kind, tag};
    let Some(store) = s.pim() else { return };
    let n = |v: u32| v.to_le_bytes();
    let mut ids = Vec::new();
    for (k, c) in CALS.iter().enumerate() {
        let colour = IDENTITY.iter().position(|&x| x == c.colour).unwrap_or(k) as u32;
        let col = n(colour);
        ids.push(
            store
                .put(
                    Kind::Calendar,
                    None,
                    &[(tag::NAME, c.name), (tag::COLOUR, &col)],
                )
                .expect("the sample calendars fit"),
        );
    }
    for (day, start, len, title, place, cal) in WEEK {
        let at = n(rp_pim::date::start_of(TODAY + day) + start);
        let minutes = n(len.max(1));
        let all = n(1);
        let calid = n(ids[cal]);
        let remind = n(15);
        let mut fields: Vec<(u8, &[u8])> = vec![
            (tag::TITLE, title),
            (tag::PLACE, place),
            (tag::START, &at),
            (tag::MINUTES, &minutes),
            (tag::CAL, &calid),
            (tag::REMIND, &remind),
        ];
        if len == 0 {
            fields.push((tag::ALL_DAY, &all));
        }
        store
            .put(Kind::Appointment, None, &fields)
            .expect("the sample week fits");
    }
    s.pim_loaded();
}

/// The agenda's first row, where a tap opens it in the editor.
const EDIT_TAP_Y: u16 = 278;

fn calendar(view: View, line: &[u8]) -> Calendar {
    let mut c = Calendar::new(&CALS, TODAY);
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

/// The look is the mock's ([`mocks`] sets it, both looks drawn).
fn open(shell: &mut rp_shell::Shell, c: impl rp_app::Program + 'static) {
    shell.give_pics(crate::emu::preview_pics());
    forecast(shell);
    shell.open_program(Box::leak(Box::new(c)));
}

/// The sample people: first, last, company, mobile (E.164), favourite,
/// and Grace's e-mail, work number and address.
/// One sample person.
type SamplePerson = (
    &'static [u8],
    &'static [u8],
    &'static [u8],
    &'static [u8],
    bool,
);

const PEOPLE: [SamplePerson; 9] = [
    (b"Grace", b"Hopper", b"US Navy", b"+17035550107", true),
    (
        b"Ada",
        b"Lovelace",
        b"Analytical Engines",
        b"+12065550142",
        true,
    ),
    (b"Alan", b"Turing", b"", b"+442079460958", true),
    (
        b"Katherine",
        b"Johnson",
        b"NASA Langley",
        b"+17575550116",
        true,
    ),
    (b"Edsger", b"Dijkstra", b"", b"+15125550133", false),
    (b"Margaret", b"Hamilton", b"MIT", b"+16175550111", false),
    (b"Donald", b"Knuth", b"Stanford", b"+16505550188", false),
    (b"Barbara", b"Liskov", b"MIT", b"+16175550170", false),
    (b"Hedy", b"Lamarr", b"", b"+13105550105", false),
];

/// The shell's store given and loaded (empty), then the sample people put
/// in it, as Contacts keeps them.
pub fn sample_people(s: &mut rp_shell::Shell) {
    use rp_pim::store::{Kind, tag};
    crate::emu::empty_organiser(s);
    let Some(store) = s.pim() else { return };
    let fav = 1u32.to_le_bytes();
    for (first, last, company, mobile, favourite) in PEOPLE {
        let mut fields: Vec<(u8, &[u8])> = vec![
            (tag::FIRST, first),
            (tag::LAST, last),
            (tag::COMPANY, company),
            (tag::MOBILE, mobile),
        ];
        if favourite {
            fields.push((tag::FAVOURITE, &fav));
        }
        if first == b"Grace" {
            fields.push((tag::EMAIL, b"grace@example.org"));
            fields.push((tag::WORK, b"+17035550155"));
            fields.push((tag::ADDRESS, b"2100 Clarendon Blvd\nArlington, VA 22201"));
        }
        store
            .put(Kind::Contact, None, &fields)
            .expect("the sample people fit");
    }
    s.pim_loaded();
}

/// Grace's map page: the shell asked for her place's map (as the image's
/// loop asks), then given the relay's layers from `RP_PREVIEW_MAP`.
fn map_page(s: &mut rp_shell::Shell) {
    sample_people(s);
    let mut c = contacts(rp_app_contacts::View::List);
    // 2100 Clarendon Blvd, Arlington: where Photon puts it.
    c.mock_map(0, (38_887_100, -77_094_700), None);
    open(s, c);
    let Some(key) = s.prog_map_wanted() else {
        return;
    };
    let Some(file) = std::env::var("RP_PREVIEW_MAP")
        .ok()
        .and_then(|p| std::fs::read(p).ok())
    else {
        return;
    };
    let mut layers = vec![0u8; rp_weather::maplayers::BYTES];
    if rp_weather::maplayers::unpack(&file, &mut layers).is_ok() {
        s.prog_map_layers(key, &layers);
    }
}

/// The sample people, Ada with every field of version 3 (a job, a
/// number and an e-mail of their own labels, the address in its parts, her
/// birthday, a website) and Grace twice (the double the merge offers).
pub fn sample_people_v3(s: &mut rp_shell::Shell) {
    use rp_pim::store::{Kind, tag};
    sample_people(s);
    let Some(store) = s.pim() else { return };
    let ada = store
        .all(Kind::Contact)
        .find(|r| r.text(tag::FIRST) == b"Ada")
        .map(|r| r.id);
    let mut more = [0u8; 200];
    let mut n = 0;
    for (kind, label, value) in [
        (
            rp_pim::contact::Kind::Phone,
            &b"Cabin"[..],
            &b"+13605550199"[..],
        ),
        (
            rp_pim::contact::Kind::Email,
            b"School",
            b"ada@school.example",
        ),
    ] {
        n = rp_pim::contact::push_entry(
            &mut more,
            n,
            rp_pim::contact::Entry { kind, label, value },
        )
        .expect("fits");
    }
    let bday = rp_pim::contact::Day {
        year: Some(1815),
        month: 12,
        day: 10,
    }
    .field()
    .expect("a day");
    if let Some(id) = ada {
        let _ = store.change(
            id,
            &[
                (tag::JOB, b"Programmer"),
                (tag::HOME, b"+12065550143"),
                (tag::EMAIL, b"ada@example.com"),
                (tag::MORE, &more[..n]),
                (tag::ADDRESS, b"12 St James Sq\nSeattle\nWA\n98101\n"),
                (tag::BIRTHDAY, &bday),
                (tag::WEBSITE, b"ada.example"),
            ],
        );
    }
    let _ = store.put(
        Kind::Contact,
        None,
        &[
            (tag::FIRST, b"G."),
            (tag::LAST, b"Hopper"),
            (tag::EMAIL, b"Grace@Example.org"),
            (tag::HOME, b"+17035550190"),
        ],
    );
}

/// The Today screen at noon on the previews' Monday with the sample week
/// and tasks in the store, the tasks line in place `t`.
fn today_panel(s: &mut rp_shell::Shell, t: rp_shell::TodayTasks) {
    sample_tasks(s);
    put_week(s);
    s.set_year(2024);
    s.set_time(rp_shell::Time {
        hour: 12,
        minute: 0,
        weekday: 0,
        day: 1,
        month: 1,
    });
    s.set_today_tasks(t);
}

fn contacts(view: rp_app_contacts::View) -> Contacts {
    let mut c = Contacts::new();
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

/// The sample tasks: title, due (days from today), flag (0-3), list,
/// done, repeats, steps (done, of).
/// One sample task.
type SampleTask = (&'static [u8], Option<i32>, u32, usize, bool, bool, (u8, u8));

const TASKS: [SampleTask; 11] = [
    (b"Renew passport", Some(-2), 3, 3, false, false, (0, 0)),
    (b"Pay rent", Some(0), 3, 0, false, true, (0, 0)),
    (b"Call the bank", Some(0), 0, 2, false, false, (0, 0)),
    (b"Pack for Portland", Some(1), 2, 3, false, false, (2, 5)),
    (b"Book flights", Some(1), 2, 3, false, false, (0, 0)),
    (b"Water the plants", Some(4), 0, 0, false, false, (0, 0)),
    (b"Quarterly report", Some(3), 2, 1, false, false, (0, 0)),
    (b"Fix the gate", Some(20), 0, 0, false, false, (0, 0)),
    (b"Return library books", Some(-1), 0, 2, true, false, (0, 0)),
    (b"Buy stamps", Some(0), 0, 2, true, false, (0, 0)),
    (b"Email Jo the slides", Some(0), 0, 1, true, false, (0, 0)),
];

/// The shell's store given and loaded, then the four lists and the sample
/// tasks (with "Pack for Portland"'s five steps) put in it.
pub fn sample_tasks(s: &mut rp_shell::Shell) {
    use rp_pim::store::{Kind, tag};
    crate::emu::empty_organiser(s);
    let Some(store) = s.pim() else { return };
    let n = |v: u32| v.to_le_bytes();
    let mut ids = Vec::new();
    for (k, l) in LISTS.iter().enumerate() {
        let colour = n(IDENTITY.iter().position(|&x| x == l.colour).unwrap_or(k) as u32);
        ids.push(
            store
                .put(
                    Kind::List,
                    None,
                    &[(tag::NAME, l.name), (tag::COLOUR, &colour)],
                )
                .expect("lists fit"),
        );
    }
    let noon = rp_pim::date::start_of(TODAY) + 12 * 60;
    for (title, due, flag, list, done, repeats, steps) in TASKS {
        let d = n(due.map_or(0, |d| (TODAY + d) as u32));
        let (fl, li, dn) = (n(flag), n(ids[list]), n(noon));
        let rule = n(rp_pim::repeat::Rule {
            every: rp_pim::repeat::Every::MonthDate,
            n: 1,
            days: 0,
            until: None,
            after: false,
        }
        .pack(TODAY));
        let mut fields: Vec<(u8, &[u8])> = vec![(tag::TITLE, title), (tag::LIST, &li)];
        if due.is_some() {
            fields.push((tag::DUE, &d));
        }
        if flag > 0 {
            fields.push((tag::PRIORITY, &fl));
        }
        if done {
            fields.push((tag::DONE, &dn));
        }
        if repeats {
            fields.push((tag::REPEAT, &rule));
        }
        let id = store.put(Kind::Task, None, &fields).expect("tasks fit");
        let parent = n(id);
        for k in 0..steps.1 {
            let mut step: Vec<(u8, &[u8])> = vec![(tag::TITLE, b"A step"), (tag::PARENT, &parent)];
            if k < steps.0 {
                step.push((tag::DONE, &dn));
            }
            store.put(Kind::Task, None, &step).expect("steps fit");
        }
    }
    s.pim_loaded();
}

fn tasks(view: rp_app_tasks::View, line: &[u8]) -> Tasks {
    let mut t = Tasks::new(&LISTS);
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

/// The sample notes put in the shell's store as Notes keeps them: the
/// title, colour, pin, the text's lines and the packed ink.
fn notes(s: &mut rp_shell::Shell, view: rp_app_notes::View) -> Notes {
    use rp_pim::store::{Kind, tag};
    crate::emu::empty_organiser(s);
    for note in sample_notes() {
        let mut text = vec![0u8; rp_pim::store::TEXT_MAX];
        let mut ink = vec![0u8; rp_pim::store::TEXT_MAX];
        let (t, i) = rp_app_notes::fields_of(note.blocks, &mut text, &mut ink);
        let colour =
            (IDENTITY.iter().position(|&x| x == note.colour).unwrap_or(1) as u32).to_le_bytes();
        let pin = 1u32.to_le_bytes();
        let mut fields: Vec<(u8, &[u8])> = vec![
            (tag::TITLE, note.title),
            (tag::TEXT, &text[..t]),
            (tag::INK, &ink[..i]),
            (tag::COLOUR, &colour),
        ];
        if note.pinned {
            fields.push((tag::PINNED, &pin));
        }
        if let Some(store) = s.pim() {
            store
                .put(Kind::Note, None, &fields)
                .expect("the sample notes fit");
        }
    }
    s.pim_loaded();
    let mut n = Notes::new(&[]);
    n.show(view);
    n
}

/// A stroke packed with widths from its own speed (a step's length, as the
/// pen filter gives them): the sample's handwriting.
fn packed(st: &[(i16, i16)], style: u8, block: u8, out: &mut Vec<u8>) {
    let mut w = Vec::with_capacity(st.len());
    let mut last = st[0];
    for &p in st {
        let d = (p.0 - last.0).abs() + (p.1 - last.1).abs();
        w.push((11 - d / 2).clamp(5, 11) as u8);
        last = p;
    }
    let widths = (style & rp_app_notes::MARKER == 0).then_some(&w[..]);
    let mut buf = [0u8; 1024];
    let n = rp_app_notes::encode_stroke(st, widths, style, block, &mut buf).expect("a stroke fits");
    out.extend_from_slice(&buf[..n]);
}

/// The blocks sample: "Saturday market" with an ink block, a paragraph, a
/// checklist and a second ink block (a sketch with a marker band), in look
/// `design`, with the preview's tool, menu row and typing.
fn block_note(
    s: &mut rp_shell::Shell,
    design: u8,
    tool: u8,
    menu: Option<u8>,
    typing: Option<usize>,
) -> Notes {
    use rp_pim::store::{Kind, tag};
    crate::emu::empty_organiser(s);
    let mut ink = Vec::new();
    packed(&cursive(40, 20, 230), 0, 0, &mut ink);
    packed(&cursive(40, 256, 380), 0, 0, &mut ink);
    packed(&cursive(96, 20, 170), 1, 0, &mut ink);
    let circle: Vec<(i16, i16)> = (0..=40i16)
        .map(|k| {
            let a = f32::from(k) * 0.165;
            (
                (300.0 + 46.0 * a.cos()) as i16,
                (80.0 + 36.0 * a.sin()) as i16,
            )
        })
        .collect();
    packed(&circle, 2, 1, &mut ink);
    packed(&[(40, 96), (210, 98)], rp_app_notes::MARKER, 1, &mut ink);
    packed(&cursive(90, 40, 200), 0, 1, &mut ink);
    packed(&[(206, 70), (240, 76), (252, 80)], 2, 1, &mut ink);
    packed(&[(240, 70), (254, 80), (240, 90)], 2, 1, &mut ink);
    let text: &[u8] = b"\x01\nBring the big basket and cash for the bakery; they don't take cards.\n[ ] Tomatoes\n[x] Honey\n[ ] Flowers for Mom\n\x01!";
    // (Typing: a word half written, for the suggestions.)
    let typed: Vec<u8> = if typing.is_some() {
        let mut t = text.to_vec();
        let at = t.windows(6).position(|w| w == b"cards.").expect("the line") + 6;
        t.splice(at..at, b" Ask if they open tomor".iter().copied());
        t
    } else {
        text.to_vec()
    };
    if let Some(store) = s.pim() {
        store
            .put(
                Kind::Note,
                None,
                &[
                    (tag::TITLE, b"Saturday market"),
                    (tag::TEXT, &typed),
                    (tag::INK, &ink),
                    (tag::COLOUR, &5u32.to_le_bytes()),
                ],
            )
            .expect("the sample fits");
    }
    s.pim_loaded();
    let mut n = Notes::new(&[]);
    n.show(rp_app_notes::View::Note(0));
    n.design = design;
    n.preview(tool, menu, typing);
    if let Some(store) = s.pim()
        && let Some(r) = store.all(Kind::Note).next()
    {
        let id = r.id;
        n.suggest_in(store, id);
    }
    n
}

/// The sample notes, as written./// The sample notes, as written.
fn sample_notes() -> &'static [Note] {
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
    list
}

/// A calculator in `mode` with `keys` pressed, as taps would.
fn calc_typed(mode: Mode, keys: &[&[u8]]) -> Calculator {
    let mut c = Calculator::new(mode, TODAY);
    c.type_keys(keys);
    c
}

/// The calendar synced through the shell's own question path: the PC's
/// list of links answered, then link 0's file (a Google calendar,
/// canned). Seattle's winter offset, so the UTC times land as they would.
fn calendar_synced(s: &mut rp_shell::Shell, view: View) {
    sample_week(s);
    s.set_utc_offset(-480);
    let mut c = calendar(View::Calendars, b"");
    // The "Get calendars" key, as a tap.
    let _ = rp_app::Program::event(&mut c, rp_app::Event::PenUp { x: 240, y: 560 }, &mock_ctx());
    open(s, c);
    let answers: [&[u8]; 2] = [
        b"0|Work (Google)
",
        include_bytes!("../../shell/weather/tests/data/google-work.ics"),
    ];
    // The Weather app's own questions come first (no relay here: each
    // refused), then the program's, each answered as the relay would.
    for a in answers {
        while let Some(q) = s.next_ask() {
            if q.ask == rp_shell::Ask::Program {
                s.net_answer(true, a);
                break;
            }
            s.net_answer(false, &[]);
        }
    }
    if view != View::Calendars {
        // Back to the chosen view, as the Back softkey and its chip would.
        s.input(rp_shell::Input::PenDown { x: 120, y: 620 });
        s.input(rp_shell::Input::PenUp { x: 120, y: 620 });
    }
}

/// A context for driving a program before the shell hosts it (no store,
/// no pictures: only for events that don't need them).
fn mock_ctx() -> rp_app::Ctx<'static> {
    let fonts: &'static rp_app::Fonts = Box::leak(Box::new(rp_app::Fonts {
        ui: &rp_today::fonts::UI,
        label: &rp_today::fonts::LABEL,
        big: &rp_today::fonts::BIG,
        num: &rp_today::fonts::NUM,
        title: &rp_today::fonts::TITLE,
    }));
    let look: &'static rp_app::Look = Box::leak(Box::new(rp_app::Look {
        light: false,
        panel: rp_shell::panel::art(false),
        pressed: rp_shell::panel::pressed(false),
        key: rp_shell::panel::art(false),
        key_down: rp_shell::panel::art(false),
        on: rp_shell::panel::art(false),
    }));
    rp_app::Ctx {
        fonts,
        look,
        now: rp_app::Now {
            day: TODAY,
            minute: 720,
        },
        pics: None,
        forecast: &[],
        us: true,
        utc_offset: 0,
        shapes: Box::leak(Box::new(rp_app::Shapes::new())),
        clip: Box::leak(Box::new(core::cell::RefCell::new(rp_app::Text::EMPTY))),
        pim: None,
    }
}

/// Every version 2 mock-up.
/// Every mock-up in both looks (the owner, 2026-11-14: "make sure we
/// are designing for both light and dark mode"): `<name>` in the light
/// look, `<name>-dark` in the dark.
pub fn mocks() -> Vec<Mock> {
    use rp_weather::settings::Appearance;
    let mut all = Vec::new();
    for (suffix, look) in [("", Appearance::Light), ("-dark", Appearance::Dark)] {
        for m in base() {
            let prepare = m.prepare;
            all.push(Mock {
                name: Box::leak(format!("{}{suffix}", m.name).into_boxed_str()),
                what: m.what,
                prepare: Box::new(move |s| {
                    // (The look first too: what a program is lent while it
                    // is prepared, a map's colours, follows it.)
                    s.set_appearance(look);
                    prepare(s);
                    s.set_appearance(look);
                    // Light behind the glass (design-language.md version 3).
                    s.use_backdrops(true);
                }),
            });
        }
    }
    // The window body's three (R201, the owner, 2026-11-14: the apps in
    // their own window, no wallpaper behind): one screen of each look per
    // body, beside the current one.
    use rp_shell::WindowBody;
    for (bname, body) in [
        ("solid", WindowBody::Solid),
        ("aero", WindowBody::Aero),
        ("tinted", WindowBody::Tinted),
    ] {
        for m in base().into_iter().filter(|m| {
            matches!(
                m.name,
                "v2-calendar-agenda" | "v2-contacts-list" | "v2-notes-home"
            )
        }) {
            let prepare: std::rc::Rc<dyn Fn(&mut rp_shell::Shell)> = std::rc::Rc::from(m.prepare);
            for (suffix, look) in [("", Appearance::Light), ("-dark", Appearance::Dark)] {
                let prepare = prepare.clone();
                all.push(Mock {
                    name: Box::leak(format!("body-{bname}-{}{suffix}", m.name).into_boxed_str()),
                    what: m.what,
                    prepare: Box::new(move |s| {
                        prepare(s);
                        s.set_appearance(look);
                        s.set_window_body(body);
                    }),
                });
            }
        }
    }
    all
}

/// A reminder due at 12:15 on the previews' Monday, the banner
/// (docs/design/notifications.md: the owner's pick, look A), over
/// Calendar's agenda (Lunch with Sam, 12:30, 15 minutes before) or over
/// Tasks (a task at 12:15).
fn banner(s: &mut rp_shell::Shell, task: bool) {
    use rp_pim::store::{Kind, tag};
    if task {
        sample_tasks(s);
        let at = (rp_pim::date::start_of(TODAY) + 12 * 60 + 15).to_le_bytes();
        if let Some(p) = s.pim() {
            let _ = p.put(
                Kind::Task,
                None,
                &[
                    (tag::TITLE, b"Call the bank about the card"),
                    (tag::REMIND_AT, &at),
                ],
            );
        }
        open(s, tasks(rp_app_tasks::View::Home, b""));
    } else {
        sample_week(s);
        open(s, calendar(View::Agenda, b""));
    }
    s.set_time(rp_shell::Time {
        hour: 12,
        minute: 15,
        weekday: 0,
        day: 1,
        month: 1,
    });
}

fn base() -> Vec<Mock> {
    vec![
        // The reminder banner (docs/design/notifications.md).
        Mock {
            name: "notify-banner",
            what: "The reminder banner: a glass card under the title bar, what and when over four keys (5 MIN, 15 MIN, DISMISS, OPEN), over Calendar",
            prepare: Box::new(|s| banner(s, false)),
        },
        Mock {
            name: "notify-banner-task",
            what: "The reminder banner over Tasks: a task due now",
            prepare: Box::new(|s| banner(s, true)),
        },
        // Half-Life's two screens for the owner's pick (hl_mockups.rs).
        Mock {
            name: "hl-files-steps",
            what: "Half-Life opened with no game files on the card, A: where they go and how, as three steps",
            prepare: Box::new(|s| open(s, crate::hl_mockups::HlFiles { steps: true })),
        },
        Mock {
            name: "hl-files-short",
            what: "Half-Life opened with no game files on the card, B: the folder first, the how in two lines",
            prepare: Box::new(|s| open(s, crate::hl_mockups::HlFiles { steps: false })),
        },
        Mock {
            name: "hl-controls-doomgles",
            what: "Half-Life's default controls in landscape, A: as DoomGLES (the pad moves and turns, the stylus looks)",
            prepare: Box::new(|s| open(s, crate::hl_mockups::HlControls { strafe: false })),
        },
        Mock {
            name: "hl-controls-strafe",
            what: "Half-Life's default controls in landscape, B: the pad moves and steps sideways, the stylus turns and looks",
            prepare: Box::new(|s| open(s, crate::hl_mockups::HlControls { strafe: true })),
        },
        // Design language version 3: the concept homes (design_v3.rs).
        Mock {
            name: "v3-calendar",
            what: "Version 3 concept, Calendar: today as a hero, the day's ribbon, next up with its countdown, the week's load",
            prepare: Box::new(|s| open(s, crate::design_v3::CalendarHome::default())),
        },
        Mock {
            name: "v3-tasks",
            what: "Version 3 concept, Tasks: the day's progress ring, three numbers, the lists as tiles",
            prepare: Box::new(|s| open(s, crate::design_v3::TasksHome::default())),
        },
        Mock {
            name: "v3-notes",
            what: "Version 3 concept, Notes: the pinned sheet with its handwriting, the rest as coloured paper",
            prepare: Box::new(|s| open(s, crate::design_v3::NotesHome::default())),
        },
        Mock {
            name: "v3-notes-pen",
            what: "Version 3 concept, Notes writing: the sheet and the pen palette (colour wells, stroke sizes, tools)",
            prepare: Box::new(|s| open(s, crate::design_v3::NotesPen::default())),
        },
        Mock {
            name: "v3-calculator",
            what: "Version 3 concept, Calculator: the glass display with its tape, gel keys",
            prepare: Box::new(|s| open(s, crate::design_v3::CalcHome)),
        },
        Mock {
            name: "v3-contacts",
            what: "Version 3 concept, Contacts: favourites as big glossy avatars, the last person spoken to, grouped cards",
            prepare: Box::new(|s| open(s, crate::design_v3::ContactsHome::default())),
        },
        // Today's calendar panel from the store, the tasks line in each of
        // its three places for the owner's pick (tasks-rule0.md).
        Mock {
            name: "today-tasks-a-header",
            what: "Today's calendar panel from the store, A: the tasks line on the panel's top line, right of TODAY",
            prepare: Box::new(|s| today_panel(s, rp_shell::TodayTasks::Header)),
        },
        Mock {
            name: "today-tasks-b-row",
            what: "Today's calendar panel from the store, B: the tasks as the panel's second row (one appointment row left)",
            prepare: Box::new(|s| today_panel(s, rp_shell::TodayTasks::Row)),
        },
        Mock {
            name: "today-tasks-c-extra",
            what: "Today's calendar panel from the store, C: a third row for the tasks, the panel 28 px taller",
            prepare: Box::new(|s| today_panel(s, rp_shell::TodayTasks::Extra)),
        },
        Mock {
            name: "app-calendar-home",
            what: "Calendar (the app itself): the day's board, from the store's records",
            prepare: Box::new(|s| {
                sample_week(s);
                open(s, calendar(View::Home, b""));
            }),
        },
        Mock {
            name: "v2-calendar-agenda",
            what: "Calendar: the agenda under the large title and the week strip, the forecast on each day",
            prepare: Box::new(|s| {
                sample_week(s);
                open(s, calendar(View::Agenda, b""));
            }),
        },
        Mock {
            name: "v2-calendar-day",
            what: "Calendar: Day, blocks sized to their length, overlaps side by side, the now line",
            prepare: Box::new(|s| {
                sample_week(s);
                open(s, calendar(View::Day, b""));
            }),
        },
        Mock {
            name: "v2-calendar-week",
            what: "Calendar: Week, seven columns of coloured blocks",
            prepare: Box::new(|s| {
                sample_week(s);
                open(s, calendar(View::Week, b""));
            }),
        },
        Mock {
            name: "v2-calendar-month",
            what: "Calendar: Month with coloured bars and the chosen day's list",
            prepare: Box::new(|s| {
                sample_week(s);
                open(s, calendar(View::Month, b""));
            }),
        },
        // Group E's motion (calendar-rule0.md): frames of the real code
        // for the owner's pick of the looks.
        Mock {
            name: "e-strip-drag",
            what: "Week strip (the owner's pick B): dragged left, the week arriving named on a glass chip",
            prepare: Box::new(|s| {
                sample_week(s);
                let mut c = calendar(View::Day, b"");
                c.mock_motion(-170, None, true);
                open(s, c);
            }),
        },
        Mock {
            name: "e-strip-back",
            what: "Week strip: dragged right into last year's last week, the chip naming both months",
            prepare: Box::new(|s| {
                sample_week(s);
                let mut c = calendar(View::Agenda, b"");
                c.mock_motion(250, None, true);
                open(s, c);
            }),
        },
    ]
    .into_iter()
    // The map page (contacts-rule0.md "Where they are"): Grace's address
    // on a map the relay's drawer made (`RP_PREVIEW_MAP=<file.rpml>`,
    // tools/maps/radar_layers.py at her place, zoom 10), coloured by the
    // shell's own path (prog_map.rs); without the file, the page waiting.
    .chain(std::iter::once(Mock {
        name: "e-map",
        what: "Map (the owner's pick A): the map with the pin and the zoom keys over it, the address on glass under it",
        prepare: Box::new(map_page),
    }))
    .chain(vec![
        Mock {
            name: "v2-calendar-new",
            what: "Calendar: quick add, the understood words in the tint, the preview card",
            prepare: Box::new(|s| {
                sample_week(s);
                open(s, calendar(View::New, b"Dentist tue 3pm 1h"));
            }),
        },
        Mock {
            name: "v2-calendar-edit-rule",
            what: "Calendar: the editor with a repeat its chip says only in part, the whole rule in words beside REPEAT (R375)",
            prepare: Box::new(|s| {
                use rp_pim::repeat::{Every, Rule};
                use rp_pim::store::{Kind, tag};
                sample_week(s);
                if let Some(store) = s.pim() {
                    let at = rp_pim::date::start_of(TODAY) + 8 * 60 + 30;
                    let rule = Rule {
                        every: Every::Week,
                        n: 2,
                        days: 1 | 1 << 2 | 1 << 4,
                        until: Some(TODAY + 61),
                        after: false,
                    };
                    store
                        .put(
                            Kind::Appointment,
                            None,
                            &[
                                (tag::TITLE, b"Team standup"),
                                (tag::START, &at.to_le_bytes()),
                                (tag::MINUTES, &15u32.to_le_bytes()),
                                (tag::REPEAT, &rule.pack(TODAY).to_le_bytes()),
                            ],
                        )
                        .expect("the standup fits");
                }
                open(s, calendar(View::Agenda, b""));
                // The agenda drawn once (its rows are where it drew them),
                // then its row, the day's first, as a tap.
                s.frame(&mut Box::new(rp_draw::Frame::new()));
                s.input(rp_shell::Input::PenDown {
                    x: 240,
                    y: EDIT_TAP_Y,
                });
                s.input(rp_shell::Input::PenUp {
                    x: 240,
                    y: EDIT_TAP_Y,
                });
            }),
        },
        Mock {
            name: "v2-calendar-calendars",
            what: "Calendar: the calendars, a Google one got through its private link and synced",
            prepare: Box::new(|s| calendar_synced(s, View::Calendars)),
        },
        Mock {
            name: "v2-calendar-synced",
            what: "Calendar: the agenda with the Google calendar's events in its own colour",
            prepare: Box::new(|s| calendar_synced(s, View::Agenda)),
        },
        Mock {
            name: "v2-contacts-list",
            what: "Contacts: favourites grid, avatars with initials, formatted numbers, the letter strip",
            prepare: Box::new(|s| {
                sample_people(s);
                open(s, contacts(rp_app_contacts::View::List));
            }),
        },
        Mock {
            name: "v2-contacts-card",
            what: "Contacts: the card with its action row and grouped details",
            prepare: Box::new(|s| {
                sample_people(s);
                open(s, contacts(rp_app_contacts::View::Card(0)));
            }),
        },
        Mock {
            name: "v2-contacts-picture",
            what: "Contacts: a person's picture chosen from the photos folder on the PC",
            prepare: Box::new(|s| {
                sample_people(s);
                let mut c = contacts(rp_app_contacts::View::Photo(0));
                c.set_pictures_from(
                    b"0|Grace at the lab
1|Grace 1984
2|Navy portrait
3|Conference
4|Family 2
",
                    0,
                );
                open(s, c);
            }),
        },
        Mock {
            name: "v2-contacts-phone",
            what: "Contacts: the phone pad, the number formatted as typed",
            prepare: Box::new(|s| {
                sample_people(s);
                let mut c = contacts(rp_app_contacts::View::Edit(EditField::Mobile));
                c.typed(b"Mae", b"2065550", b"");
                open(s, c);
            }),
        },
        Mock {
            name: "v2-contacts-address",
            what: "Contacts: address suggestions while typing, capitals at each word",
            prepare: Box::new(|s| {
                sample_people(s);
                let mut c = contacts(rp_app_contacts::View::Edit(EditField::Street));
                c.typed(b"Mae", b"2065550142", b"410 Pi");
                c.set_places_from(include_bytes!(
                    "../../shell/weather/tests/data/photon-410-pi.json"
                ));
                open(s, c);
            }),
        },
        // Version 3, group A (2026-11-23): the records and files rows.
        Mock {
            name: "v3-contacts-card",
            what: "Contacts: the card with a job title, numbers and e-mails of their own labels, the birthday and the website, a page at a time",
            prepare: Box::new(|s| {
                sample_people_v3(s);
                open(s, contacts(rp_app_contacts::View::Card(1)));
            }),
        },
        Mock {
            name: "v3-contacts-editor-more",
            what: "Contacts: + Add under the phones and e-mails, each extra with its own label",
            prepare: Box::new(|s| {
                sample_people(s);
                let mut c = contacts(rp_app_contacts::View::Edit(EditField::Extra(0)));
                c.typed(b"Ada", b"2065550142", b"");
                c.typed_field(EditField::Home, b"2065550143");
                c.typed_extra(true, b"Cabin", b"3605550199");
                c.typed_extra(false, b"School", b"ada@school.example");
                open(s, c);
            }),
        },
        Mock {
            name: "v3-contacts-editor-label",
            what: "Contacts: an extra number's label typed, or picked from the strip over the keyboard",
            prepare: Box::new(|s| {
                sample_people(s);
                let mut c = contacts(rp_app_contacts::View::Edit(EditField::Label(0)));
                c.typed(b"Ada", b"2065550142", b"");
                c.typed_extra(true, b"Cab", b"3605550199");
                open(s, c);
            }),
        },
        Mock {
            name: "v3-contacts-editor-address",
            what: "Contacts: the address in its parts, the days typed the region's way, the website",
            prepare: Box::new(|s| {
                sample_people(s);
                let mut c = contacts(rp_app_contacts::View::Edit(EditField::Anniversary));
                c.typed(b"Ada", b"2065550142", b"12 St James Sq");
                for (f, t) in [
                    (EditField::City, &b"Seattle"[..]),
                    (EditField::Region, b"WA"),
                    (EditField::Postal, b"98101"),
                    (EditField::Birthday, b"12/10/1815"),
                ] {
                    c.typed_field(f, t);
                }
                c.typed_bad_day(EditField::Anniversary, b"02/30");
                open(s, c);
            }),
        },
        Mock {
            name: "v3-contacts-tools",
            what: "Contacts: import and export (vCard 3.0 or 4.0, the card or the PC) and the doubles found",
            prepare: Box::new(|s| {
                sample_people_v3(s);
                open(s, contacts(rp_app_contacts::View::Tools));
            }),
        },
        Mock {
            name: "v3-contacts-import",
            what: "Contacts: the .vcf files on the card and the PC, to import",
            prepare: Box::new(|s| {
                sample_people(s);
                let mut c = contacts(rp_app_contacts::View::List);
                c.ask_import();
                let cx = mock_ctx();
                let _ = rp_app::Program::card_ask(&mut c);
                rp_app::Program::net_answer(
                    &mut c,
                    true,
                    b"0|people.vcf\n1|From my old phone.vcf\n",
                    &cx,
                );
                let _ = rp_app::Program::net_ask(&mut c);
                rp_app::Program::net_answer(
                    &mut c,
                    true,
                    b"0|Outlook contacts.vcf\n1|Google contacts.vcf\n",
                    &cx,
                );
                open(s, c);
            }),
        },
        Mock {
            name: "v3-contacts-merge",
            what: "Contacts: a double offered as one: the two side by side, what they share, what the one card keeps",
            prepare: Box::new(|s| {
                sample_people_v3(s);
                open(s, contacts(rp_app_contacts::View::Merge));
            }),
        },
        Mock {
            name: "v3-calendar-calendars",
            what: "Calendar: the time zone's row (from the PC, or chosen) and Import / Export .ics",
            prepare: Box::new(|s| {
                sample_week(s);
                s.set_utc_offset(-480);
                open(s, calendar(View::Calendars, b""));
            }),
        },
        Mock {
            name: "v3-calendar-zones",
            what: "Calendar: the time zone chosen from every zone on Earth, by typing a city",
            prepare: Box::new(|s| {
                sample_week(s);
                s.set_utc_offset(-480);
                open(s, calendar(View::Zones, b""));
            }),
        },
        Mock {
            name: "v3-calendar-import",
            what: "Calendar: the .ics files on the card and the PC, to import into a calendar of their own",
            prepare: Box::new(|s| {
                sample_week(s);
                let mut c = calendar(View::Calendars, b"");
                let cx = mock_ctx();
                let k = rp_app_calendar::IMPORT_KEY_FOR_PREVIEW;
                let _ = rp_app::Program::event(
                    &mut c,
                    rp_app::Event::PenUp {
                        x: k.x + 20,
                        y: k.y + 20,
                    },
                    &cx,
                );
                let _ = rp_app::Program::card_ask(&mut c);
                rp_app::Program::net_answer(&mut c, true, b"0|Work.ics\n1|School year.ics\n", &cx);
                let _ = rp_app::Program::net_ask(&mut c);
                rp_app::Program::net_answer(&mut c, true, b"0|US holidays 2027.ics\n", &cx);
                open(s, c);
            }),
        },
        Mock {
            name: "v3-calendar-export",
            what: "Calendar: every appointment written as one .ics, to the card or the PC",
            prepare: Box::new(|s| {
                sample_week(s);
                let mut c = calendar(View::Calendars, b"");
                let cx = mock_ctx();
                let k = rp_app_calendar::EXPORT_KEY_FOR_PREVIEW;
                let _ = rp_app::Program::event(
                    &mut c,
                    rp_app::Event::PenUp {
                        x: k.x + 20,
                        y: k.y + 20,
                    },
                    &cx,
                );
                open(s, c);
            }),
        },
        Mock {
            name: "v2-tasks-home",
            what: "Tasks: smart tiles with counts over the coloured lists and their progress",
            prepare: Box::new(|s| {
                sample_tasks(s);
                open(s, tasks(rp_app_tasks::View::Home, b""));
            }),
        },
        Mock {
            name: "v2-tasks-lists",
            what: "Tasks: the lists' own screen, each made, renamed, recoloured or deleted",
            prepare: Box::new(|s| {
                sample_tasks(s);
                open(s, tasks(rp_app_tasks::View::Lists, b""));
            }),
        },
        Mock {
            name: "v2-tasks-today",
            what: "Tasks: Today, late first, round ticks in the list's colour, flags, subtasks",
            prepare: Box::new(|s| {
                sample_tasks(s);
                open(s, tasks(rp_app_tasks::View::List(None), b""));
            }),
        },
        Mock {
            name: "v2-tasks-time",
            what: "Tasks: a task's time typed to the minute (hour and minute fields, AM/PM, quick minutes)",
            prepare: Box::new(|s| {
                sample_tasks(s);
                let mut t = tasks(rp_app_tasks::View::List(None), b"");
                t.show_time_pad(Some(15 * 60 + 7));
                open(s, t);
            }),
        },
        Mock {
            name: "v2-calendar-alert",
            what: "Calendar: an alert's lead as a number and its unit",
            prepare: Box::new(|s| {
                sample_week(s);
                let mut c = calendar(View::Agenda, b"");
                c.show_alert_pad(45);
                open(s, c);
            }),
        },
        Mock {
            name: "v2-tasks-add",
            what: "Tasks: the add line, the understood words in the tint",
            prepare: Box::new(|s| {
                sample_tasks(s);
                open(
                    s,
                    tasks(
                        rp_app_tasks::View::List(None),
                        b"Pay rent fri !1 every month #home",
                    ),
                );
            }),
        },
        Mock {
            name: "v2-notes-home",
            what: "Notes: pinned first, cards in their colours with ink miniatures",
            prepare: Box::new(|s| {
                let n = notes(s, rp_app_notes::View::Home);
                open(s, n);
            }),
        },
        Mock {
            name: "v2-notes-note",
            what: "Notes: text, a checklist and handwriting in one note on ruled paper",
            prepare: Box::new(|s| {
                let n = notes(s, rp_app_notes::View::Note(0));
                open(s, n);
            }),
        },
        // Group D (2026-11-23): a note as blocks, in the three looks the
        // owner picks from, and the ink tools.
        Mock {
            name: "v3-notes-blocks-a-open",
            what: "Notes blocks, look A (open sheet): ink, a paragraph, a checklist and a second ink block on one ruled sheet, the ink blocks unmarked like paper",
            prepare: Box::new(|s| {
                let n = block_note(s, 0, 0, None, None);
                open(s, n);
            }),
        },
        Mock {
            name: "v3-notes-blocks-b-framed",
            what: "Notes blocks, look B (framed pads): each ink block a sunken pad of the paper a shade deeper",
            prepare: Box::new(|s| {
                let n = block_note(s, 1, 0, None, None);
                open(s, n);
            }),
        },
        Mock {
            name: "v3-notes-blocks-c-margin",
            what: "Notes blocks, look C (margin marks): each ink block marked by a bar in the note's colour in the margin",
            prepare: Box::new(|s| {
                let n = block_note(s, 2, 0, None, None);
                open(s, n);
            }),
        },
        Mock {
            name: "v3-notes-ink-marker",
            what: "Notes ink tools: the marker chosen, its see-through band over each colour well, an orange hue picked",
            prepare: Box::new(|s| {
                let n = block_note(s, 0, 1, None, None);
                open(s, n);
            }),
        },
        Mock {
            name: "v3-notes-ink-eraser",
            what: "Notes ink tools: the stroke eraser part way along its path (its trail and the ring where it is)",
            prepare: Box::new(|s| {
                let n = block_note(s, 0, 2, None, None);
                open(s, n);
            }),
        },
        Mock {
            name: "v3-notes-export",
            what: "Notes: Menu, Export: text, Markdown, SVG or PNG to RetroPane\\Notes on the card",
            prepare: Box::new(|s| {
                let n = block_note(s, 0, 0, Some(1), None);
                open(s, n);
            }),
        },
        Mock {
            name: "v3-notes-typing",
            what: "Notes: typing a paragraph, the caret at its end, word suggestions on the keyboard's strip",
            prepare: Box::new(|s| {
                let n = block_note(s, 0, 0, None, Some(1));
                open(s, n);
            }),
        },
        Mock {
            name: "v2-calc-basic",
            what: "Calculator: the tape, the sum with its answer live, memory keys, orange operators",
            prepare: Box::new(|s| {
                open(
                    s,
                    calc_typed(
                        Mode::Basic,
                        &[
                            b"1", b"2", b"5", b"0", b"x", b"1", b"2", b"=", b"/", b"5", b"2", b"=",
                            b"M+", b"+", b"1", b"5", b"%",
                        ],
                    ),
                )
            }),
        },
        Mock {
            name: "v2-calc-sci",
            what: "Calculator: the scientific keys",
            prepare: Box::new(|s| {
                open(
                    s,
                    calc_typed(
                        Mode::Scientific,
                        &[b"s(", b"3", b"0", b")", b"+", b"2", b"^", b"1", b"0"],
                    ),
                )
            }),
        },
        Mock {
            name: "v2-calc-money",
            what: "Calculator: money at the ECB's daily rates, kept for offline",
            prepare: Box::new(|s| {
                open(s, {
                    let mut c = calc_typed(Mode::Money, &[]);
                    c.set_rates_from(include_bytes!(
                        "../../shell/weather/tests/data/ecb-daily.xml"
                    ));
                    c.set_entry(b"120");
                    c
                })
            }),
        },
        Mock {
            name: "v2-calc-tip",
            what: "Calculator: tip and split",
            prepare: Box::new(|s| {
                open(s, {
                    let mut c = calc_typed(Mode::Tip, &[]);
                    c.set_entry(b"86.40");
                    c
                })
            }),
        },
        Mock {
            name: "v2-calc-dates",
            what: "Calculator: days between dates",
            prepare: Box::new(|s| {
                open(s, {
                    let mut c = calc_typed(Mode::Dates, &[]);
                    c.set_dates(TODAY, TODAY + 359);
                    c
                })
            }),
        },
    ])
    .collect()
}
