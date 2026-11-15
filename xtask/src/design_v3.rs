//! Design language version 3: concept screens, drawn by the shell and the
//! 2D renderer the X51v runs (docs/design/design-language.md version 3).
//!
//! The owner, 2026-11-14: the apps are "as basic as it can possibly be",
//! "just a bunch of fields"; the Weather app is the bar. What Weather has
//! and they lacked, read off its screens: a hero (one big number and a
//! picture that answer at a glance), instruments (data drawn: dials, arcs,
//! range bars), a varied grid of cards, and light behind the glass. These
//! are sample-data concepts of each app's home built to that bar, for his
//! pick before the apps are rebuilt on it; every mark is one of the kit's.

use rp_app::kit::{self, DIM, IDENTITY, KeyState, TEXT, tint};
use rp_app::{Argb, Cmd, Ctx, Event, Frame, Pic, Program, Rect, Reply};

/// A sample person: name, organisation, number, which small avatar.
type Person = (&'static [u8], &'static [u8], &'static [u8], usize);
/// A sample note: where, its paper, its title, its lines.
type Sheet = (Rect, Argb, &'static [u8], &'static [&'static [u8]]);

/// A glass card with a spaced-capital label in the app's tint.
fn card(f: &mut Frame, cx: &Ctx, r: Rect, label: &[u8], c: Argb) {
    kit::panel(f, cx, r);
    if !label.is_empty() {
        kit::text(f, r.x + 16, r.y + 22, cx.fonts.label, c, label);
    }
}

/// A gel key with its word.
fn key(f: &mut Frame, cx: &Ctx, r: Rect, word: &[u8], on: bool) {
    kit::key(f, cx, r, if on { KeyState::On } else { KeyState::Rest });
    kit::centred(f, r, r.y + r.h / 2 + 6, cx.fonts.label, TEXT, word);
}

/// The tabs along the foot, as Weather's: words, the chosen one underlined
/// in the tint.
fn tabs(f: &mut Frame, cx: &Ctx, words: &[&[u8]], chosen: usize, c: Argb) {
    let w = 448 / words.len() as u16;
    f.push(Cmd::Fill {
        r: Rect::new(0, 558, 480, 1),
        color: kit::alpha(TEXT, 0x30),
    });
    for (i, word) in words.iter().enumerate() {
        let r = Rect::new(16 + i as u16 * w, 560, w, 36);
        kit::centred(
            f,
            r,
            584,
            cx.fonts.ui,
            if i == chosen { TEXT } else { DIM },
            word,
        );
        if i == chosen {
            f.push(Cmd::Fill {
                r: Rect::new(r.x + 16, 593, w - 32, 3),
                color: c,
            });
        }
    }
}

/// Calendar's home: today as a hero, the day drawn as a ribbon, what is
/// next with its countdown, the week's load at a glance.
#[derive(Default)]
pub struct CalendarHome {
    ring: Option<Pic>,
}

impl Program for CalendarHome {
    fn title(&self) -> &'static str {
        "Calendar"
    }
    fn softkeys(&self) -> (&'static str, &'static str) {
        ("Today", "New")
    }
    fn accent(&self) -> Option<Argb> {
        Some(tint::CALENDAR)
    }
    fn draw(&mut self, f: &mut Frame, cx: &Ctx) {
        let t = tint::CALENDAR;
        let (ui, label) = (cx.fonts.ui, cx.fonts.label);
        // The hero: the date, the weather, the day as a ribbon.
        card(f, cx, Rect::new(16, 44, 448, 154), b"MONDAY", t);
        kit::text(f, 30, 138, cx.fonts.big, TEXT, b"1");
        kit::text(f, 96, 100, cx.fonts.title, TEXT, b"January");
        kit::text(f, 96, 128, ui, DIM, b"2024 \x80 5 today");
        if let Some(w) = cx.forecast.first() {
            if let Some(p) = w.icon {
                f.push(Cmd::Pic {
                    x: 384,
                    y: 56,
                    pic: p,
                });
            }
            kit::right(f, 448, 128, ui, DIM, b"54\x7F / 41\x7F");
        }
        // 6 AM to 10 PM: each appointment where it falls, now marked.
        let at = |minute: u16| 32 + (u32::from(minute.clamp(360, 1320) - 360) * 416 / 960) as u16;
        f.push(Cmd::Fill {
            r: Rect::new(32, 152, 416, 14),
            color: kit::alpha(TEXT, 0x26),
        });
        for (start, len, c) in [
            (570u16, 30u16, IDENTITY[4]),
            (750, 60, IDENTITY[0]),
            (900, 60, IDENTITY[3]),
            (930, 45, IDENTITY[5]),
            (1110, 90, IDENTITY[2]),
        ] {
            let (x0, x1) = (at(start), at(start + len));
            kit::round_rect(f, cx, Rect::new(x0, 152, (x1 - x0).max(8), 14), c);
        }
        f.push(Cmd::Fill {
            r: Rect::new(at(720), 146, 2, 26),
            color: TEXT,
        });
        kit::text(f, 32, 188, label, DIM, b"6 AM");
        kit::centred(
            f,
            Rect::new(at(720) - 30, 172, 60, 18),
            188,
            label,
            TEXT,
            b"NOW",
        );
        kit::right(f, 448, 188, label, DIM, b"10 PM");

        // Next up: a countdown ring, the words, what can be done about it.
        card(f, cx, Rect::new(16, 206, 448, 124), b"", t);
        if self.ring.is_none() {
            self.ring = cx
                .pics
                .and_then(|p| kit::ring(p, 96, 9, 128, kit::painted(cx, t), kit::track(cx)));
        }
        if let Some(p) = self.ring {
            f.push(Cmd::Pic {
                x: 28,
                y: 220,
                pic: p,
            });
        }
        kit::centred(
            f,
            Rect::new(28, 220, 96, 96),
            278,
            cx.fonts.num,
            TEXT,
            b"30",
        );
        kit::centred(f, Rect::new(28, 220, 96, 96), 296, label, DIM, b"MIN");
        kit::text(f, 140, 234, label, t, b"NEXT UP");
        kit::text(f, 140, 264, cx.fonts.title, TEXT, b"Lunch with Sam");
        kit::text(f, 140, 286, ui, DIM, b"12:30 PM \x80 Cafe Vita");
        key(f, cx, Rect::new(140, 294, 124, 28), b"DIRECTIONS", false);
        key(f, cx, Rect::new(272, 294, 88, 28), b"SNOOZE", false);

        // The week: how full each day is.
        kit::section(f, cx, 336, b"THIS WEEK", t);
        let days: [(&[u8], &[u8], &[usize]); 7] = [
            (b"SUN", b"31", &[]),
            (b"MON", b"1", &[4, 0, 3, 5]),
            (b"TUE", b"2", &[0, 2]),
            (b"WED", b"3", &[3, 5, 6]),
            (b"THU", b"4", &[1]),
            (b"FRI", b"5", &[2, 4]),
            (b"SAT", b"6", &[]),
        ];
        for (i, (name, n, load)) in days.iter().enumerate() {
            let r = Rect::new(16 + i as u16 * 64, 358, 60, 86);
            if i == 1 {
                kit::key(f, cx, r, KeyState::On);
            } else {
                kit::panel(f, cx, r);
            }
            kit::centred(f, r, r.y + 20, label, DIM, name);
            kit::centred(f, r, r.y + 46, ui, TEXT, n);
            for (k, &c) in load.iter().enumerate() {
                kit::round_rect(
                    f,
                    cx,
                    Rect::new(r.x + 12, r.y + 54 + k as u16 * 7, 36, 5),
                    IDENTITY[c],
                );
            }
        }
        // Later today.
        kit::section(f, cx, 450, b"LATER TODAY", t);
        for (k, (time, what, c)) in [
            (&b"3:00 PM"[..], &b"Dentist \x80 Dr. Lee"[..], IDENTITY[3]),
            (b"6:30 PM", b"Book club \x80 Library", IDENTITY[2]),
        ]
        .iter()
        .enumerate()
        {
            let r = Rect::new(16, 472 + k as u16 * 42, 448, 38);
            kit::panel(f, cx, r);
            kit::colour_bar(f, r, *c);
            kit::text(f, 36, r.y + 26, ui, DIM, time);
            kit::text(f, 136, r.y + 26, ui, TEXT, what);
        }
        tabs(f, cx, &[b"Today", b"Day", b"Week", b"Month"], 0, t);
    }
    fn event(&mut self, _e: Event, _cx: &Ctx) -> Reply {
        Reply::Nothing
    }
}

/// Tasks' home: the day's progress as a ring, three numbers that matter,
/// the lists as tiles with their own progress.
#[derive(Default)]
pub struct TasksHome {
    ring: Option<Pic>,
    check: Option<Pic>,
    done: Option<Pic>,
}

impl Program for TasksHome {
    fn title(&self) -> &'static str {
        "Tasks"
    }
    fn softkeys(&self) -> (&'static str, &'static str) {
        ("Lists", "New")
    }
    fn accent(&self) -> Option<Argb> {
        Some(tint::TASKS)
    }
    fn draw(&mut self, f: &mut Frame, cx: &Ctx) {
        let t = tint::TASKS;
        let (ui, label) = (cx.fonts.ui, cx.fonts.label);
        if let Some(p) = cx.pics.filter(|_| self.ring.is_none()) {
            self.ring = kit::ring(p, 96, 10, 4 * 256 / 7, kit::painted(cx, t), kit::track(cx));
            self.check = kit::disc(p, 24, 0, Some(kit::painted(cx, t)));
            self.done = kit::disc(p, 24, kit::DONE, None);
        }
        // The hero: today's progress.
        card(f, cx, Rect::new(16, 44, 448, 124), b"", t);
        if let Some(p) = self.ring {
            f.push(Cmd::Pic {
                x: 28,
                y: 58,
                pic: p,
            });
        }
        kit::centred(f, Rect::new(28, 58, 96, 96), 116, cx.fonts.num, TEXT, b"4");
        kit::centred(f, Rect::new(28, 58, 96, 96), 134, label, DIM, b"OF 7");
        kit::text(f, 142, 78, label, t, b"TODAY");
        kit::text(f, 142, 110, cx.fonts.title, TEXT, b"More than halfway");
        kit::text(f, 142, 134, ui, DIM, b"3 to do \x80 1 late");
        kit::round_rect(f, cx, Rect::new(142, 146, 8, 8), kit::AMBER);
        kit::text(f, 158, 156, label, kit::AMBER, b"6-DAY STREAK");
        // Three numbers.
        for (i, (name, n, c)) in [
            (&b"DUE TODAY"[..], &b"3"[..], t),
            (b"LATE", b"1", kit::LATE),
            (b"DONE, 7 DAYS", b"12", kit::DONE),
        ]
        .iter()
        .enumerate()
        {
            let r = Rect::new(16 + i as u16 * 152, 176, 144, 70);
            kit::panel(f, cx, r);
            kit::colour_bar(f, r, *c);
            kit::text(f, r.x + 16, r.y + 22, label, DIM, name);
            kit::text(f, r.x + 16, r.y + 60, cx.fonts.num, TEXT, n);
        }
        // The lists, each its own colour and progress.
        kit::section(f, cx, 252, b"LISTS", t);
        for (i, (name, left, done, of, c)) in [
            (&b"Home"[..], &b"5 left"[..], 2u16, 7u16, IDENTITY[0]),
            (b"Work", b"3 left", 6, 9, IDENTITY[5]),
            (b"Errands", b"3 left", 1, 4, IDENTITY[3]),
            (b"Axim project", b"1 left", 9, 10, IDENTITY[1]),
        ]
        .iter()
        .enumerate()
        {
            let r = Rect::new(
                16 + (i as u16 % 2) * 228,
                274 + (i as u16 / 2) * 72,
                220,
                66,
            );
            kit::tinted(f, cx, r, *c, 0x58);
            kit::text(f, r.x + 16, r.y + 28, ui, TEXT, name);
            kit::right(f, r.x + r.w - 14, r.y + 28, label, DIM, left);
            kit::progress_bar(
                f,
                Rect::new(r.x + 16, r.y + 44, r.w - 32, 6),
                *done,
                *of,
                *c,
            );
        }
        // Today's.
        kit::section(f, cx, 422, b"TODAY", t);
        for (k, (what, when, done, late)) in [
            (&b"Call the dentist back"[..], &b"9:00 AM"[..], true, false),
            (b"Send the Axim photos", b"LATE", false, true),
            (b"Pick up the parts", b"4:00 PM", false, false),
        ]
        .iter()
        .enumerate()
        {
            let r = Rect::new(16, 444 + k as u16 * 42, 448, 38);
            kit::panel(f, cx, r);
            kit::round_check(f, cx, r.x + 10, r.y + 7, self.check, self.done, *done);
            kit::text(
                f,
                r.x + 46,
                r.y + 26,
                ui,
                if *done { DIM } else { TEXT },
                what,
            );
            kit::right(
                f,
                r.x + r.w - 14,
                r.y + 25,
                label,
                if *late { kit::LATE } else { DIM },
                when,
            );
        }
    }
    fn event(&mut self, _e: Event, _cx: &Ctx) -> Reply {
        Reply::Nothing
    }
}

/// Contacts' home: people first (the favourites as big glossy avatars),
/// who was last spoken to with what can be done, the list in grouped
/// cards.
#[derive(Default)]
pub struct ContactsHome {
    big: [Option<Pic>; 5],
    small: [Option<Pic>; 3],
}

impl Program for ContactsHome {
    fn title(&self) -> &'static str {
        "Contacts"
    }
    fn softkeys(&self) -> (&'static str, &'static str) {
        ("Groups", "New")
    }
    fn accent(&self) -> Option<Argb> {
        Some(tint::CONTACTS)
    }
    fn draw(&mut self, f: &mut Frame, cx: &Ctx) {
        let t = tint::CONTACTS;
        let (ui, label) = (cx.fonts.ui, cx.fonts.label);
        let people: [&[u8]; 5] = [
            b"Grace Hopper",
            b"Katherine Johnson",
            b"Ada Lovelace",
            b"Alan Turing",
            b"Margaret Hamilton",
        ];
        if let Some(p) = cx.pics.filter(|_| self.big[0].is_none()) {
            for (slot, name) in self.big.iter_mut().zip(people) {
                *slot = kit::gloss_disc(p, 64, kit::colour_of(name));
            }
            for (slot, name) in self.small.iter_mut().zip([
                &b"Edsger Dijkstra"[..],
                b"Margaret Hamilton",
                b"Grace Hopper",
            ]) {
                *slot = kit::gloss_disc(p, 40, kit::colour_of(name));
            }
        }
        // Search.
        kit::panel(f, cx, Rect::new(16, 44, 448, 40));
        kit::text(f, 32, 70, ui, DIM, b"Search 128 people");
        // The favourites.
        kit::section(f, cx, 90, b"FAVOURITES", t);
        for (i, name) in people[..4].iter().enumerate() {
            let cell = Rect::new(16 + i as u16 * 112, 112, 112, 100);
            kit::avatar(
                f,
                self.big[i],
                cell.x + 24,
                cell.y + 2,
                64,
                cx.fonts.title,
                name,
            );
            let first = name.split(|&b| b == b' ').next().unwrap_or(name);
            kit::centred(f, cell, cell.y + 90, ui, TEXT, first);
        }
        // The last person spoken to.
        card(f, cx, Rect::new(16, 220, 448, 92), b"", t);
        kit::avatar(f, self.big[4], 28, 234, 64, cx.fonts.title, people[4]);
        kit::text(f, 106, 246, label, t, b"RECENT");
        kit::text(f, 106, 274, cx.fonts.title, TEXT, b"Margaret Hamilton");
        kit::text(f, 106, 298, ui, DIM, b"Called yesterday");
        key(f, cx, Rect::new(332, 274, 58, 30), b"CALL", true);
        key(f, cx, Rect::new(396, 274, 58, 30), b"TEXT", false);
        // The list, grouped by letter in one card each.
        let groups: [(&[u8], &[Person]); 2] = [
            (
                b"D",
                &[(b"Edsger Dijkstra", b"TU Eindhoven", b"(512) 555-0133", 0)],
            ),
            (
                b"H",
                &[
                    (b"Margaret Hamilton", b"MIT", b"(617) 555-0111", 1),
                    (b"Grace Hopper", b"US Navy", b"(703) 555-0107", 2),
                ],
            ),
        ];
        let mut y = 320;
        for (letter, rows) in groups {
            kit::section(f, cx, y, letter, t);
            y += 22;
            let r = Rect::new(16, y, 448, rows.len() as u16 * 54);
            kit::panel(f, cx, r);
            for (k, (name, org, phone, pic)) in rows.iter().enumerate() {
                let ry = y + k as u16 * 54;
                if k > 0 {
                    f.push(Cmd::Fill {
                        r: Rect::new(78, ry, 372, 1),
                        color: kit::alpha(TEXT, 0x28),
                    });
                }
                kit::avatar(f, self.small[*pic], 28, ry + 7, 40, ui, name);
                kit::text(f, 82, ry + 24, ui, TEXT, name);
                kit::text(f, 82, ry + 44, label, DIM, org);
                kit::right(f, 450, ry + 24, ui, DIM, phone);
            }
            y += r.h + 4;
        }
    }
    fn event(&mut self, _e: Event, _cx: &Ctx) -> Reply {
        Reply::Nothing
    }
}

/// A paper colour (kept by the look's pass: paper is paper in both looks).
const fn paper(c: Argb) -> Argb {
    kit::keep(c)
}
const INK: Argb = kit::keep(0xFF24_2A36);
const INK_DIM: Argb = kit::keep(0xFF6A_6F7A);

/// A handwritten line: a wave with loops, as points of the ink panel.
fn scribble(x0: i16, y0: i16, n: i16, amp: i16) -> Vec<(i16, i16)> {
    (0..n)
        .map(|k| {
            let up = [0, -amp, -amp / 3, amp / 2, amp, amp / 4, -amp / 2, -amp];
            (x0 + k * 9, y0 + up[(k as usize) % 8])
        })
        .collect()
}

/// Notes' home: the pinned note as a real sheet with its handwriting,
/// the rest as coloured paper in two columns.
#[derive(Default)]
pub struct NotesHome {
    ink: Option<Pic>,
}

impl Program for NotesHome {
    fn title(&self) -> &'static str {
        "Notes"
    }
    fn softkeys(&self) -> (&'static str, &'static str) {
        ("Search", "New")
    }
    fn accent(&self) -> Option<Argb> {
        Some(tint::NOTES)
    }
    fn draw(&mut self, f: &mut Frame, cx: &Ctx) {
        let t = tint::NOTES;
        let (ui, label) = (cx.fonts.ui, cx.fonts.label);
        // The pinned sheet.
        let sheet = Rect::new(16, 44, 448, 196);
        kit::round_rect(f, cx, sheet, paper(0xFFF8_F0D8));
        kit::round_rect(f, cx, Rect::new(16, 44, 10, 196), paper(0xFFB8_8A4A));
        for k in 0..5u16 {
            f.push(Cmd::Fill {
                r: Rect::new(40, 108 + k * 28, 408, 1),
                color: paper(0xFFC4_D4E4),
            });
        }
        kit::text(f, 40, 66, label, paper(0xFF9A_6A20), b"PINNED");
        kit::text(f, 40, 96, cx.fonts.title, INK, b"Ideas for the garden");
        if self.ink.is_none() {
            self.ink = cx
                .pics
                .and_then(|p| p.alloc(kit::INK_PIC.0, kit::INK_PIC.1));
            if let (Some(pics), Some(p)) = (cx.pics, self.ink) {
                let lines = [
                    scribble(20, 34, 30, 9),
                    scribble(20, 62, 22, 8),
                    scribble(20, 90, 34, 9),
                    // A bed, a tree and a sun, drawn.
                    vec![(330, 70), (420, 70), (420, 120), (330, 120), (330, 70)],
                    vec![
                        (360, 120),
                        (362, 96),
                        (350, 84),
                        (364, 74),
                        (378, 84),
                        (366, 96),
                        (368, 120),
                    ],
                    vec![(400, 44), (408, 38), (416, 44), (408, 50), (400, 44)],
                ];
                let refs: Vec<&[(i16, i16)]> = lines.iter().map(Vec::as_slice).collect();
                pics.write(p, |px| {
                    rp_app_notes::paint_styled(
                        px,
                        kit::INK_PIC.0,
                        kit::INK_PIC.1,
                        &refs,
                        &[0, 0, 0, 2, 2, 3],
                        None,
                    );
                });
            }
        }
        if let Some(p) = self.ink {
            f.push(Cmd::Pic {
                x: 24,
                y: 96,
                pic: p,
            });
        }
        kit::right(f, 448, 230, label, INK_DIM, b"EDITED 9:12 AM");
        // The rest, each on its own paper.
        kit::section(f, cx, 246, b"RECENT", t);
        let notes: [Sheet; 4] = [
            (
                Rect::new(16, 268, 220, 128),
                0xFFFF_F3B0,
                b"Portland trip",
                &[
                    b"Flight Thu 7:15 AM",
                    b"[x] Book the hotel",
                    b"[ ] Rain jacket",
                    b"[ ] Axim charger",
                ],
            ),
            (
                Rect::new(244, 268, 220, 92),
                0xFFD6_ECFF,
                b"Shopping",
                &[b"[x] Milk", b"[ ] Coffee beans", b"[ ] Stamps"],
            ),
            (
                Rect::new(244, 368, 220, 100),
                0xFFFF_DCE4,
                b"Book club",
                &[b"Next: The Left Hand", b"of Darkness", b"Bring the cheese"],
            ),
            (
                Rect::new(16, 404, 220, 64),
                0xFFDC_F5D0,
                b"Wi-Fi at the library",
                &[b"Library-Guest"],
            ),
        ];
        for (r, c, title, lines) in notes {
            kit::round_rect(f, cx, r, paper(c));
            // A fold of shade along the foot: a sheet, not a tile.
            f.push(Cmd::Fill {
                r: Rect::new(r.x + 8, r.y + r.h - 3, r.w - 16, 2),
                color: paper(kit::mix(c, 0xFF00_0000, 40)),
            });
            kit::fitted(f, r.x + 12, r.y + 26, ui, INK, title, r.w - 24);
            for (k, l) in lines.iter().enumerate() {
                kit::fitted(
                    f,
                    r.x + 12,
                    r.y + 48 + k as u16 * 20,
                    label,
                    INK_DIM,
                    l,
                    r.w - 24,
                );
            }
        }
        tabs(f, cx, &[b"All", b"Pinned", b"Lists", b"Ink"], 0, t);
    }
    fn event(&mut self, _e: Event, _cx: &Ctx) -> Reply {
        Reply::Nothing
    }
}

/// A note being written: the sheet, and a pen palette that is easy to
/// hit (the owner, 2026-11-14: "the pen and marker colors are hard to
/// select", "there also is no stroke size control").
#[derive(Default)]
pub struct NotesPen {
    ink: Option<Pic>,
    wells: [Option<Pic>; 6],
}

impl Program for NotesPen {
    fn title(&self) -> &'static str {
        "Notes"
    }
    fn softkeys(&self) -> (&'static str, &'static str) {
        ("Done", "Menu")
    }
    fn accent(&self) -> Option<Argb> {
        Some(tint::NOTES)
    }
    fn draw(&mut self, f: &mut Frame, cx: &Ctx) {
        let label = cx.fonts.label;
        let colours: [Argb; 6] = [
            0xFF24_2A36,
            0xFF1C_5FD0,
            0xFFD0_3A2E,
            0xFF1E_9A50,
            0xFFE8_8A1A,
            0xFF8A_3FC8,
        ];
        let sheet = Rect::new(8, 40, 464, 404);
        kit::round_rect(f, cx, sheet, paper(0xFFF8_F0D8));
        kit::round_rect(f, cx, Rect::new(8, 40, 8, 404), paper(0xFFB8_8A4A));
        for k in 0..12u16 {
            f.push(Cmd::Fill {
                r: Rect::new(28, 104 + k * 28, 432, 1),
                color: paper(0xFFC4_D4E4),
            });
        }
        kit::text(f, 30, 84, cx.fonts.title, INK, b"Ideas for the garden");
        if self.ink.is_none() {
            self.ink = cx
                .pics
                .and_then(|p| p.alloc(kit::INK_PIC.0, kit::INK_PIC.1));
            if let (Some(pics), Some(p)) = (cx.pics, self.ink) {
                let lines = [
                    scribble(16, 30, 40, 9),
                    scribble(16, 58, 28, 8),
                    scribble(16, 86, 36, 9),
                    scribble(16, 142, 24, 10),
                    vec![(300, 120), (420, 120), (420, 180), (300, 180), (300, 120)],
                    vec![(10, 150), (240, 150)],
                ];
                let refs: Vec<&[(i16, i16)]> = lines.iter().map(Vec::as_slice).collect();
                pics.write(p, |px| {
                    rp_app_notes::paint_styled(
                        px,
                        kit::INK_PIC.0,
                        kit::INK_PIC.1,
                        &refs,
                        &[0, 0, 1, 2, 3, rp_app_notes::MARKER],
                        None,
                    );
                });
            }
            for (slot, c) in self.wells.iter_mut().zip(colours) {
                *slot = cx.pics.and_then(|p| kit::gloss_disc(p, 40, c));
            }
        }
        if let Some(p) = self.ink {
            f.push(Cmd::Pic {
                x: 16,
                y: 96,
                pic: p,
            });
        }
        // The palette: a glass tray under the sheet. Colour wells a thumb
        // wide (40 px, the chosen one ringed), three stroke sizes drawn at
        // their size, then the tools as gel keys.
        kit::panel(f, cx, Rect::new(8, 450, 464, 146));
        kit::text(f, 24, 472, label, DIM, b"COLOUR");
        for (i, p) in self.wells.iter().enumerate() {
            let x = 22 + i as u16 * 48;
            if i == 1 {
                kit::key(f, cx, Rect::new(x - 5, 477, 50, 50), KeyState::On);
            }
            if let Some(p) = *p {
                f.push(Cmd::Pic { x, y: 482, pic: p });
            }
        }
        kit::text(f, 326, 472, label, DIM, b"SIZE");
        for (i, d) in [6u16, 12, 20].iter().enumerate() {
            let r = Rect::new(322 + i as u16 * 48, 480, 44, 44);
            kit::key(f, cx, r, if i == 1 { KeyState::On } else { KeyState::Rest });
            kit::round_rect(
                f,
                cx,
                Rect::new(r.x + (44 - d) / 2, r.y + (44 - d) / 2, *d, *d),
                if cx.look.light {
                    kit::keep(0xFF28_3038)
                } else {
                    TEXT
                },
            );
        }
        for (i, word) in [&b"PEN"[..], b"MARKER", b"ERASER", b"TEXT", b"LIST", b"UNDO"]
            .iter()
            .enumerate()
        {
            key(
                f,
                cx,
                Rect::new(18 + i as u16 * 75, 540, 70, 44),
                word,
                i == 0,
            );
        }
    }
    fn event(&mut self, _e: Event, _cx: &Ctx) -> Reply {
        Reply::Nothing
    }
}

/// Calculator: a glass display with its tape, gel keys, the operators in
/// the app's amber, equals in the selection's glow.
#[derive(Default)]
pub struct CalcHome;

impl Program for CalcHome {
    fn title(&self) -> &'static str {
        "Calculator"
    }
    fn softkeys(&self) -> (&'static str, &'static str) {
        ("History", "Copy")
    }
    fn accent(&self) -> Option<Argb> {
        Some(tint::CALCULATOR)
    }
    fn draw(&mut self, f: &mut Frame, cx: &Ctx) {
        let t = tint::CALCULATOR;
        let (ui, label, num) = (cx.fonts.ui, cx.fonts.label, cx.fonts.num);
        // The display: the tape above, the answer large.
        card(f, cx, Rect::new(16, 44, 448, 138), b"", t);
        kit::right(f, 444, 72, ui, DIM, b"1,250 x 12 = 15,000");
        kit::right(f, 444, 98, ui, DIM, b"15,000 - 8% = 13,800");
        f.push(Cmd::Fill {
            r: Rect::new(32, 108, 416, 1),
            color: kit::alpha(TEXT, 0x30),
        });
        kit::text(f, 32, 136, label, t, b"13,800 / 4");
        kit::right(f, 444, 166, cx.fonts.big, TEXT, b"3450");
        // The modes.
        kit::chips(
            f,
            cx,
            Rect::new(16, 190, 448, 30),
            &[b"BASIC", b"SCIENTIFIC", b"UNITS", b"TIP"],
            0,
            t,
        );
        // The keys.
        let rows: [[&[u8]; 4]; 5] = [
            [b"AC", b"+/-", b"%", b"/"],
            [b"7", b"8", b"9", b"x"],
            [b"4", b"5", b"6", b"-"],
            [b"1", b"2", b"3", b"+"],
            [b"0", b"", b".", b"="],
        ];
        for (j, row) in rows.iter().enumerate() {
            for (i, word) in row.iter().enumerate() {
                if word.is_empty() {
                    continue;
                }
                let wide = j == 4 && i == 0;
                let r = Rect::new(
                    16 + i as u16 * 113,
                    228 + j as u16 * 74,
                    if wide { 222 } else { 109 },
                    68,
                );
                let op = i == 3 && j < 4;
                if *word == b"=" {
                    kit::key(f, cx, r, KeyState::On);
                } else if op {
                    kit::tinted(f, cx, r, t, 0x90);
                } else {
                    kit::key(f, cx, r, KeyState::Rest);
                }
                let digits = word.iter().all(|c| b"0123456789.+-x/=%".contains(c));
                if digits {
                    kit::centred(f, r, r.y + 47, num, TEXT, word);
                } else {
                    kit::centred(f, r, r.y + 41, ui, DIM, word);
                }
            }
        }
    }
    fn event(&mut self, _e: Event, _cx: &Ctx) -> Reply {
        Reply::Nothing
    }
}
