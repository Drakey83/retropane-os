//! Tasks, version 2 ([tasks-rule0.md](../../../docs/design/tasks-rule0.md)):
//! an app program ([`rp_app::Program`]).
//!
//! Iterate, don't imitate (rule 0): Windows Mobile 5 had one list, a sort
//! menu and a "!" for priority; Things and Reminders open on smart lists
//! with counts and colour every list. Ours opens on smart tiles (Today,
//! Upcoming, Late, Flagged) over the user's coloured lists with progress,
//! ticks with a round box that fills, flags priority in colour and word,
//! and highlights what the typed line understood.

#![no_std]
#![forbid(unsafe_code)]

use rp_app::kit::{self, DIM, LATE, TEXT, tint};
use rp_app::{Ctx, Event, Field, Frame, Key, MARGIN, Pic, Program, Rect, Reply, WIDE};

/// Tasks a view holds.
pub const MAX: usize = 64;

/// A list: a name and its colour.
#[derive(Clone, Copy, Debug)]
pub struct List {
    /// Its name.
    pub name: &'static [u8],
    /// Its identity colour.
    pub colour: u32,
}

/// One task.
#[derive(Clone, Copy, Debug)]
pub struct Task {
    /// Its title.
    pub title: &'static [u8],
    /// Due day (days from 1970), if any.
    pub due: Option<i32>,
    /// Priority: 0 none, 1 low, 2 medium, 3 high.
    pub flag: u8,
    /// Which list.
    pub list: u8,
    /// Done.
    pub done: bool,
    /// Subtasks (done, of).
    pub steps: (u8, u8),
    /// It repeats.
    pub repeats: bool,
}

/// The views.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    /// The smart tiles and the lists.
    Home,
    /// One list (index), or Today when none.
    List(Option<u8>),
}

/// The Tasks program.
pub struct Tasks {
    lists: &'static [List],
    tasks: [Option<Task>; MAX],
    view: View,
    line: [u8; 64],
    line_n: usize,
    ring: [Option<Pic>; 8],
    full: [Option<Pic>; 8],
    painted: bool,
}

const FLAGS: [u32; 4] = [0, kit::IDENTITY[4], kit::AMBER, LATE];

fn num(out: &mut [u8], v: u32) -> usize {
    let mut d = [0u8; 10];
    let mut i = d.len();
    let mut v = v;
    loop {
        i -= 1;
        d[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    let n = d.len() - i;
    out[..n].copy_from_slice(&d[i..]);
    n
}

impl Tasks {
    /// No tasks yet.
    #[must_use]
    pub fn new(lists: &'static [List]) -> Self {
        Self {
            lists,
            tasks: [None; MAX],
            view: View::Home,
            line: [0; 64],
            line_n: 0,
            ring: [None; 8],
            full: [None; 8],
            painted: false,
        }
    }

    /// Adds a task (false when full).
    pub fn add(&mut self, t: Task) -> bool {
        match self.tasks.iter_mut().find(|s| s.is_none()) {
            Some(s) => {
                *s = Some(t);
                true
            }
            None => false,
        }
    }

    /// Shows `view`.
    pub fn show(&mut self, view: View) {
        self.view = view;
    }

    /// Sets the typed line (the previews).
    pub fn set_line(&mut self, s: &[u8]) {
        let n = s.len().min(self.line.len());
        self.line[..n].copy_from_slice(&s[..n]);
        self.line_n = n;
    }

    fn paint(&mut self, cx: &Ctx) {
        if self.painted {
            return;
        }
        if let Some(p) = cx.pics {
            for k in 0..8 {
                self.ring[k] = kit::disc(p, 24, 0, Some(kit::IDENTITY[k]));
                self.full[k] = kit::disc(p, 24, kit::IDENTITY[k], None);
            }
        }
        self.painted = true;
    }

    fn colour_index(&self, list: u8) -> usize {
        let c = self
            .lists
            .get(usize::from(list))
            .map_or(kit::IDENTITY[4], |l| l.colour);
        kit::IDENTITY.iter().position(|&x| x == c).unwrap_or(4)
    }

    fn counts(&self, today: i32) -> (u32, u32, u32, u32) {
        let open = || self.tasks.iter().flatten().filter(|t| !t.done);
        let today_n = open().filter(|t| t.due.is_some_and(|d| d <= today)).count() as u32;
        let up = open().filter(|t| t.due.is_some_and(|d| d > today)).count() as u32;
        let late = open().filter(|t| t.due.is_some_and(|d| d < today)).count() as u32;
        let flagged = open().filter(|t| t.flag >= 3).count() as u32;
        (today_n, up, late, flagged)
    }

    fn home(&self, f: &mut Frame, cx: &Ctx) {
        let today = cx.now.day;
        let (t, u, l, fl) = self.counts(today);
        let mut s = [0u8; 40];
        let mut k = num(&mut s, t);
        for &c in b" due today \x80 " {
            s[k] = c;
            k += 1;
        }
        k += num(&mut s[k..], l);
        for &c in b" late" {
            s[k] = c;
            k += 1;
        }
        let mut y = kit::large_title(f, cx, 36, b"Tasks", &s[..k], tint::TASKS);
        y += 6;
        let tiles: [(&[u8], u32, u32); 4] = [
            (b"TODAY", t, tint::TASKS),
            (b"UPCOMING", u, kit::IDENTITY[6]),
            (b"LATE", l, LATE),
            (b"FLAGGED", fl, kit::AMBER),
        ];
        for (i, (name, n, c)) in tiles.iter().enumerate() {
            let r = Rect::new(
                MARGIN + (i as u16 % 2) * 228,
                y + (i as u16 / 2) * 86,
                220,
                80,
            );
            kit::tinted(f, cx, r, *c, 0x48);
            kit::text(f, r.x + 14, r.y + 26, cx.fonts.label, TEXT, name);
            let mut b = [0u8; 10];
            let bn = num(&mut b, *n);
            kit::right(f, r.right() - 14, r.y + 66, cx.fonts.num, TEXT, &b[..bn]);
        }
        y += 178;
        kit::section(f, cx, y, b"MY LISTS", tint::TASKS);
        y += 24;
        for (i, list) in self.lists.iter().enumerate() {
            if y + 52 > 596 {
                break;
            }
            let all = self
                .tasks
                .iter()
                .flatten()
                .filter(|t| usize::from(t.list) == i)
                .count() as u16;
            let done = self
                .tasks
                .iter()
                .flatten()
                .filter(|t| usize::from(t.list) == i && t.done)
                .count() as u16;
            let r = Rect::new(MARGIN, y, WIDE, 48);
            kit::panel(f, cx, r);
            kit::colour_bar(f, r, list.colour);
            kit::text(f, r.x + 22, r.y + 30, cx.fonts.ui, TEXT, list.name);
            let mut b = [0u8; 16];
            let mut bn = num(&mut b, u32::from(all - done));
            for &c in b" open" {
                b[bn] = c;
                bn += 1;
            }
            kit::right(f, r.right() - 14, r.y + 24, cx.fonts.ui, DIM, &b[..bn]);
            kit::progress_bar(
                f,
                Rect::new(r.right() - 110, r.y + 34, 96, 4),
                done,
                all,
                list.colour,
            );
            y += 52;
        }
    }

    fn due_words(due: Option<i32>, today: i32, out: &mut [u8; 16]) -> (usize, bool) {
        let Some(d) = due else { return (0, false) };
        let w: &[u8] = match d - today {
            0 => b"Today",
            1 => b"Tomorrow",
            x if x < 0 => {
                let mut n = 0;
                for &c in b"Late " {
                    out[n] = c;
                    n += 1;
                }
                n += num(&mut out[n..], (-x) as u32);
                for &c in if x == -1 { &b" day"[..] } else { &b" days"[..] } {
                    out[n] = c;
                    n += 1;
                }
                return (n, true);
            }
            2..=6 => [&b"Mon"[..], b"Tue", b"Wed", b"Thu", b"Fri", b"Sat", b"Sun"]
                [((d + 3).rem_euclid(7)) as usize],
            _ => b"Later",
        };
        out[..w.len()].copy_from_slice(w);
        (w.len(), false)
    }

    fn list(&self, f: &mut Frame, cx: &Ctx, which: Option<u8>) {
        let today = cx.now.day;
        let (name, colour): (&[u8], u32) = match which {
            Some(i) => self
                .lists
                .get(usize::from(i))
                .map_or((b"List", tint::TASKS), |l| (l.name, l.colour)),
            None => (b"Today", tint::TASKS),
        };
        let mut y = kit::large_title(f, cx, 36, name, b"Tap the circle when it's done", colour);
        // The add line, the understood words in the tint.
        let field = Rect::new(MARGIN, y + 4, WIDE, 40);
        kit::panel(f, cx, field);
        if self.line_n == 0 {
            kit::text(
                f,
                field.x + 14,
                field.y + 27,
                cx.fonts.ui,
                DIM,
                b"+ Add a task: Pay rent fri !",
            );
        } else {
            let mut x = field.x + 14;
            for w in self.line[..self.line_n]
                .split(|&b| b == b' ')
                .filter(|w| !w.is_empty())
            {
                let understood = w.starts_with(b"!")
                    || w.starts_with(b"#")
                    || [
                        &b"today"[..],
                        b"tomorrow",
                        b"mon",
                        b"tue",
                        b"wed",
                        b"thu",
                        b"fri",
                        b"sat",
                        b"sun",
                        b"every",
                        b"week",
                        b"month",
                    ]
                    .contains(&w);
                kit::text(
                    f,
                    x,
                    field.y + 27,
                    cx.fonts.ui,
                    if understood { colour } else { TEXT },
                    w,
                );
                x += cx.fonts.ui.width(w) + cx.fonts.ui.width(b" ");
            }
            f.push(rp_app::Cmd::Outline {
                r: field,
                color: kit::FOCUS,
            });
        }
        y += 52;
        let mine = |t: &&Task| match which {
            Some(i) => t.list == i,
            None => t.due.is_some_and(|d| d <= today),
        };
        let mut open = [None; MAX];
        let mut n = 0;
        for t in self.tasks.iter().flatten().filter(mine).filter(|t| !t.done) {
            open[n] = Some(*t);
            n += 1;
        }
        open[..n].sort_unstable_by_key(|t| {
            t.map_or((i32::MAX, 0), |t| {
                (t.due.unwrap_or(i32::MAX), 3 - i32::from(t.flag))
            })
        });
        let mut late_said = false;
        for t in open[..n].iter().flatten() {
            if y + 50 > 560 {
                break;
            }
            if !late_said && t.due.is_some_and(|d| d < today) {
                kit::section(f, cx, y, b"LATE", LATE);
                y += 22;
                late_said = true;
            }
            let r = Rect::new(MARGIN, y, WIDE, 46);
            kit::panel(f, cx, r);
            let ci = self.colour_index(t.list);
            kit::round_check(
                f,
                cx,
                r.x + 12,
                r.y + 11,
                self.ring[ci],
                self.full[ci],
                false,
            );
            kit::fitted(f, r.x + 48, r.y + 21, cx.fonts.ui, TEXT, t.title, 250);
            let mut w = [0u8; 16];
            let (wn, late) = Self::due_words(t.due, today, &mut w);
            let mut sub = [0u8; 40];
            let mut sn = 0;
            for &c in &w[..wn] {
                sub[sn] = c;
                sn += 1;
            }
            if t.steps.1 > 0 {
                for &c in if sn > 0 { &b" \x80 "[..] } else { &b""[..] } {
                    sub[sn] = c;
                    sn += 1;
                }
                sn += num(&mut sub[sn..], u32::from(t.steps.0));
                for &c in b" of " {
                    sub[sn] = c;
                    sn += 1;
                }
                sn += num(&mut sub[sn..], u32::from(t.steps.1));
            }
            if t.repeats {
                for &c in b" \x80 repeats" {
                    sub[sn] = c;
                    sn += 1;
                }
            }
            kit::text(
                f,
                r.x + 48,
                r.y + 40,
                cx.fonts.ui,
                if late { LATE } else { DIM },
                &sub[..sn],
            );
            if t.flag > 0 {
                let fr = Rect::new(r.right() - 26, r.y + 12, 6, 22);
                f.push(rp_app::Cmd::Fill {
                    r: fr,
                    color: FLAGS[usize::from(t.flag.min(3))],
                });
                f.push(rp_app::Cmd::Fill {
                    r: Rect::new(fr.x + 6, fr.y, 10, 10),
                    color: FLAGS[usize::from(t.flag.min(3))],
                });
            }
            y += 50;
        }
        let done = self
            .tasks
            .iter()
            .flatten()
            .filter(mine)
            .filter(|t| t.done)
            .count();
        if done > 0 && y + 40 <= 596 {
            let r = Rect::new(MARGIN, y, WIDE, 36);
            kit::panel(f, cx, r);
            let ci = self.colour_index(which.unwrap_or(0));
            kit::round_check(f, cx, r.x + 12, r.y + 6, self.ring[ci], self.full[ci], true);
            let mut b = [0u8; 16];
            let mut bn = num(&mut b, done as u32);
            for &c in b" done" {
                b[bn] = c;
                bn += 1;
            }
            kit::text(f, r.x + 48, r.y + 25, cx.fonts.ui, DIM, &b[..bn]);
        }
    }
}

impl Program for Tasks {
    fn title(&self) -> &'static str {
        "Tasks"
    }

    fn softkeys(&self) -> (&'static str, &'static str) {
        match self.view {
            View::Home => ("Back", "New"),
            View::List(_) => ("Back", "Add"),
        }
    }

    fn draw(&mut self, f: &mut Frame, cx: &Ctx) {
        self.paint(cx);
        kit::backdrop(f);
        match self.view {
            View::Home => self.home(f, cx),
            View::List(w) => self.list(f, cx, w),
        }
    }

    fn event(&mut self, e: Event, _cx: &Ctx) -> Reply {
        match (e, self.view) {
            (Event::Key(Key::SoftLeft), View::Home) => Reply::Close,
            (Event::Key(Key::SoftLeft), _) => {
                self.view = View::Home;
                Reply::Redraw
            }
            (Event::Key(Key::SoftRight), View::Home) => {
                self.view = View::List(None);
                Reply::Redraw
            }
            (Event::Char(c), View::List(_)) if self.line_n < self.line.len() => {
                self.line[self.line_n] = c;
                self.line_n += 1;
                Reply::Redraw
            }
            (Event::Delete, View::List(_)) if self.line_n > 0 => {
                self.line_n -= 1;
                Reply::Redraw
            }
            _ => Reply::Nothing,
        }
    }

    fn keyboard(&self) -> Option<Field> {
        (matches!(self.view, View::List(_)) && self.line_n > 0).then_some(Field::Line)
    }
}
