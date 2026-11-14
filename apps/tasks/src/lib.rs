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
use rp_app::{Ctx, Event, Field, Frame, Key, MARGIN, Pic, Program, Rect, Reply, Text, WIDE};
use rp_pim::date;
use rp_pim::store::{Kind, tag};

/// The add line under a list's title.
const ADD_LINE: Rect = Rect::new(MARGIN, 36 + kit::TITLE_BAND + 4, WIDE, 40);

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

/// A list as the store holds it.
#[derive(Clone, Copy, Debug)]
struct ListRow {
    id: u32,
    name: Text<24>,
    colour: u32,
}

/// One task.
#[derive(Clone, Copy, Debug)]
pub struct Task {
    /// Its record.
    pub id: u32,
    /// Its title.
    pub title: Text<48>,
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
    lists: [ListRow; 8],
    n_lists: usize,
    tasks: [Option<Task>; MAX],
    /// The store's change count the lists and tasks were copied at.
    seen: Option<u32>,
    /// Where each open task's round box was drawn, to tick it by a tap.
    boxes: [(Rect, u32); 12],
    n_boxes: usize,
    /// The add line is being typed.
    typing: bool,
    /// The rows on screen (for swipes), where the pen went down, the row
    /// whose actions a left swipe showed and their keys, and a tick in
    /// motion (the task and its frame).
    rows_at: [(Rect, u32); 12],
    n_rows: usize,
    press: Option<(u16, u32)>,
    revealed: Option<u32>,
    actions: [Rect; 3],
    ticking: Option<(u32, u8)>,
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
        let mut rows = [ListRow {
            id: 0,
            name: Text::EMPTY,
            colour: kit::IDENTITY[4],
        }; 8];
        for (r, l) in rows.iter_mut().zip(lists) {
            r.name = Text::of(l.name);
            r.colour = l.colour;
        }
        Self {
            lists: rows,
            n_lists: lists.len().clamp(1, 8),
            tasks: [None; MAX],
            seen: None,
            boxes: [(Rect::new(0, 0, 0, 0), 0); 12],
            n_boxes: 0,
            typing: false,
            rows_at: [(Rect::new(0, 0, 0, 0), 0); 12],
            n_rows: 0,
            press: None,
            revealed: None,
            actions: [Rect::new(0, 0, 0, 0); 3],
            ticking: None,
            view: View::Home,
            line: [0; 64],
            line_n: 0,
            ring: [None; 8],
            full: [None; 8],
            painted: false,
        }
    }

    /// The lists and tasks copied again from the store when its records
    /// changed: a task's steps are counted from the tasks naming it as
    /// their parent.
    fn refresh(&mut self, cx: &Ctx) {
        let Some(cell) = cx.pim else { return };
        let Ok(guard) = cell.try_borrow() else { return };
        let Some(store) = guard.as_ref() else { return };
        if self.seen == Some(store.changes()) {
            return;
        }
        self.seen = Some(store.changes());
        let mut n = 0;
        for r in store.all(Kind::List) {
            if n < self.lists.len() {
                self.lists[n] = ListRow {
                    id: r.id,
                    name: Text::of(r.text(tag::NAME)),
                    colour: kit::IDENTITY[(r.num(tag::COLOUR).unwrap_or(4) % 8) as usize],
                };
                n += 1;
            }
        }
        if n > 0 {
            self.n_lists = n;
        }
        self.tasks = [None; MAX];
        for (k, r) in store
            .all(Kind::Task)
            .filter(|r| r.num(tag::PARENT).is_none())
            .take(MAX)
            .enumerate()
        {
            let steps = store
                .all(Kind::Task)
                .filter(|c| c.num(tag::PARENT) == Some(r.id));
            let (mut done, mut of) = (0u8, 0u8);
            for c in steps {
                of = of.saturating_add(1);
                if c.num(tag::DONE).is_some() {
                    done = done.saturating_add(1);
                }
            }
            let list = r
                .num(tag::LIST)
                .and_then(|id| self.lists[..self.n_lists].iter().position(|l| l.id == id))
                .unwrap_or(0) as u8;
            self.tasks[k] = Some(Task {
                id: r.id,
                title: Text::of(r.text(tag::TITLE)),
                due: r.num(tag::DUE).map(|d| d as i32),
                flag: r.num(tag::PRIORITY).unwrap_or(0).min(3) as u8,
                list,
                done: r.num(tag::DONE).is_some(),
                steps: (done, of),
                repeats: r.num(tag::REPEAT).is_some(),
            });
        }
    }

    /// The typed line kept as a task: the quick parser's date and "!"
    /// priority (`!1` high .. `!3` low, as Todoist writes them; a bare `!`
    /// high), `#list` names a list.
    fn add_typed(&mut self, cx: &Ctx) -> bool {
        let line = &self.line[..self.line_n];
        let mut title = [0u8; rp_pim::quick::TITLE_MAX];
        let (q, tn) = rp_pim::quick::parse(line, cx.now.day, cx.us, &mut title);
        // The title without "#list" and "!n" words the parser left in.
        let mut clean = [0u8; rp_pim::quick::TITLE_MAX];
        let mut cn = 0;
        let mut flag = q.priority.map_or(0u32, |p| if p >= 2 { 3 } else { 1 });
        let mut list_id = self.lists[0].id;
        let mut repeat: Option<rp_pim::repeat::Rule> = None;
        let mut every_next = false;
        for w in title[..tn].split(|&b| b == b' ').filter(|w| !w.is_empty()) {
            if let Some(name) = w.strip_prefix(b"#") {
                if let Some(l) = self.lists[..self.n_lists]
                    .iter()
                    .find(|l| l.name.get().eq_ignore_ascii_case(name))
                {
                    list_id = l.id;
                }
                continue;
            }
            if w.len() == 2 && w[0] == b'!' && (b'1'..=b'3').contains(&w[1]) {
                flag = u32::from(b'4' - w[1]);
                continue;
            }
            // "every day/weekday/week/month/year": the repeat, and its
            // words out of the title.
            if w.eq_ignore_ascii_case(b"every") {
                every_next = true;
                continue;
            }
            if every_next {
                every_next = false;
                use rp_pim::repeat::Every;
                let (e, days) = match w {
                    b"day" | b"daily" => (Some(Every::Day), 0),
                    b"weekday" | b"weekdays" => (Some(Every::Week), 0b1_1111),
                    b"week" | b"weekly" => (Some(Every::Week), 0),
                    b"month" | b"monthly" => (Some(Every::MonthDate), 0),
                    b"year" | b"yearly" => (Some(Every::Year), 0),
                    _ => (None, 0),
                };
                if let Some(every) = e {
                    repeat = Some(rp_pim::repeat::Rule {
                        every,
                        n: 1,
                        days,
                        until: None,
                    });
                    continue;
                }
            }
            for &c in w.iter().chain(b" ") {
                if cn < clean.len() {
                    clean[cn] = c;
                    cn += 1;
                }
            }
        }
        let cn = cn.saturating_sub(1);
        if cn == 0 {
            return false;
        }
        let n = |v: u32| v.to_le_bytes();
        let (due, fl, li) = (n(q.day.unwrap_or(cx.now.day) as u32), n(flag), n(list_id));
        let due_day = q.day.unwrap_or(cx.now.day);
        let rep = repeat.map(|r| r.pack(due_day).to_le_bytes());
        let mut fields: [(u8, &[u8]); 5] = [
            (tag::TITLE, &clean[..cn]),
            (tag::DUE, &due),
            (tag::PRIORITY, &[]),
            (tag::LIST, &[]),
            (tag::REPEAT, &[]),
        ];
        if let Some(r) = &rep {
            fields[4].1 = r;
        }
        if flag > 0 {
            fields[2].1 = &fl;
        }
        if list_id != 0 {
            fields[3].1 = &li;
        }
        let Some(cell) = cx.pim else { return false };
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return false;
        };
        let Some(store) = guard.as_mut() else {
            return false;
        };
        let ok = store.put(Kind::Task, None, &fields).is_ok();
        if ok {
            self.seen = None;
            self.line_n = 0;
        }
        ok
    }

    /// Task `id` ticked done (its record kept with the minute it was
    /// done), or opened again.
    /// Task `id`'s record kept with field `which` set to `value` (or
    /// left out for `None`), every other field as it was.
    fn set_field(&mut self, cx: &Ctx, id: u32, which: u8, value: Option<&[u8]>) {
        let Some(cell) = cx.pim else { return };
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return;
        };
        let Some(store) = guard.as_mut() else { return };
        let Some(r) = store.get(id) else { return };
        let mut keep = [(0u8, [0u8; 64], 0usize); 12];
        let mut k = 0;
        for t in [
            tag::TITLE,
            tag::DUE,
            tag::PRIORITY,
            tag::LIST,
            tag::PARENT,
            tag::REPEAT,
            tag::NOTE,
            tag::REMIND_AT,
            tag::DONE,
        ] {
            let v = if t == which {
                value.unwrap_or(&[])
            } else {
                r.text(t)
            };
            if !v.is_empty() && k < keep.len() {
                let n = v.len().min(64);
                keep[k].0 = t;
                keep[k].1[..n].copy_from_slice(&v[..n]);
                keep[k].2 = n;
                k += 1;
            }
        }
        if which != 0
            && value.is_some()
            && !keep[..k].iter().any(|(t, _, _)| *t == which)
            && k < keep.len()
        {
            let v = value.unwrap_or(&[]);
            let n = v.len().min(64);
            keep[k] = (which, [0; 64], n);
            keep[k].1[..n].copy_from_slice(&v[..n]);
            k += 1;
        }
        let mut fields: [(u8, &[u8]); 12] = [(0, &[]); 12];
        for (i, (t, b, n)) in keep[..k].iter().enumerate() {
            fields[i] = (*t, &b[..*n]);
        }
        if store.put(Kind::Task, Some(id), &fields[..k]).is_ok() {
            self.seen = None;
        }
    }

    /// Task `id` ticked done (with the minute), or opened again.
    fn tick(&mut self, cx: &Ctx, id: u32) {
        let done = self.tasks.iter().flatten().any(|t| t.id == id && t.done);
        let minute = (date::start_of(cx.now.day) + u32::from(cx.now.minute)).to_le_bytes();
        self.set_field(cx, id, tag::DONE, (!done).then_some(&minute[..]));
    }

    /// A swipe's action on task `id`: 0 tomorrow, 1 next week, 2 delete.
    fn act(&mut self, cx: &Ctx, id: u32, a: u8) {
        if a == 2 {
            if let Some(cell) = cx.pim
                && let Ok(mut g) = cell.try_borrow_mut()
                && let Some(store) = g.as_mut()
                && store.remove(id).is_ok()
            {
                self.seen = None;
            }
            return;
        }
        let day = cx.now.day
            + if a == 0 {
                1
            } else {
                7 - i32::from(date::weekday(cx.now.day))
            };
        self.set_field(cx, id, tag::DUE, Some(&(day as u32).to_le_bytes()));
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
            kit::right(f, r.right() - 14, r.y + 66, cx.fonts.num, *c, &b[..bn]);
        }
        y += 178;
        kit::section(f, cx, y, b"MY LISTS", tint::TASKS);
        y += 24;
        for (i, list) in self.lists[..self.n_lists].iter().enumerate() {
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
            kit::text(f, r.x + 22, r.y + 30, cx.fonts.ui, TEXT, list.name.get());
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

    fn list(&mut self, f: &mut Frame, cx: &Ctx, which: Option<u8>) {
        let today = cx.now.day;
        self.n_boxes = 0;
        self.n_rows = 0;
        let (name, colour): (&[u8], u32) = match which {
            Some(i) => self
                .lists
                .get(usize::from(i))
                .map_or((b"List", tint::TASKS), |l| (l.name.get(), l.colour)),
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
        // The open tasks as indices, sorted by urgency (a copy of the
        // tasks themselves was 6 KB on the stack, which the programs share
        // with the shell).
        let mut open = [0u8; MAX];
        let mut n = 0;
        for (k, t) in self.tasks.iter().enumerate() {
            if t.as_ref().is_some_and(|t| mine(&t) && !t.done) {
                open[n] = k as u8;
                n += 1;
            }
        }
        let key = |k: &u8| {
            self.tasks[usize::from(*k)].map_or((i32::MAX, 0), |t| {
                (t.due.unwrap_or(i32::MAX), 3 - i32::from(t.flag))
            })
        };
        open[..n].sort_unstable_by_key(key);
        let mut late_said = false;
        for &k in &open[..n] {
            let Some(t) = self.tasks[usize::from(k)] else {
                continue;
            };
            let t = &t;
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
            // A tick in motion: the box fills over five frames (150 ms),
            // then the task moves to the done group.
            let frame = self.ticking.filter(|(id, _)| *id == t.id).map(|(_, k)| k);
            kit::round_check(
                f,
                cx,
                r.x + 12,
                r.y + 11,
                self.ring[ci],
                self.full[ci],
                frame.is_some_and(|k| k >= 3),
            );
            if let Some(k) = frame.filter(|&k| k < 3) {
                let d = 4 + u16::from(k) * 5;
                f.push(rp_app::Cmd::Fill {
                    r: Rect::new(r.x + 24 - d / 2, r.y + 23 - d / 2, d, d),
                    color: kit::IDENTITY[ci],
                });
            }
            if self.n_rows < self.rows_at.len() {
                self.rows_at[self.n_rows] = (r, t.id);
                self.n_rows += 1;
            }
            if self.n_boxes < self.boxes.len() {
                // The box's target: the row's left 44 px, a stylus' width.
                self.boxes[self.n_boxes] = (Rect::new(r.x, r.y, 44, r.h), t.id);
                self.n_boxes += 1;
            }
            kit::fitted(f, r.x + 48, r.y + 21, cx.fonts.ui, TEXT, t.title.get(), 250);
            if self.revealed == Some(t.id) {
                let words: [&[u8]; 3] = [b"TOMORROW", b"NEXT WEEK", b"DELETE"];
                for (k, w) in words.iter().enumerate() {
                    let a = Rect::new(r.right() - 3 * 98 + k as u16 * 98, r.y + 4, 94, r.h - 8);
                    kit::key(
                        f,
                        cx,
                        a,
                        if k == 2 {
                            kit::KeyState::Down
                        } else {
                            kit::KeyState::Rest
                        },
                    );
                    kit::centred(
                        f,
                        a,
                        a.y + a.h / 2 + 6,
                        cx.fonts.label,
                        if k == 2 { kit::LATE } else { TEXT },
                        w,
                    );
                    self.actions[k] = a;
                }
                y += 50;
                continue;
            }
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

/// The list a first opening shows.
pub static FIRST_LISTS: [List; 1] = [List {
    name: b"Tasks",
    colour: kit::IDENTITY[4],
}];

/// Tasks as it first opens (the program's entry and the PC's model).
#[must_use]
pub fn opened(_today: i32) -> Tasks {
    Tasks::new(&FIRST_LISTS)
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
        self.refresh(cx);
        kit::backdrop(f);
        match self.view {
            View::Home => self.home(f, cx),
            View::List(w) => self.list(f, cx, w),
        }
    }

    fn event(&mut self, e: Event, cx: &Ctx) -> Reply {
        match (e, self.view) {
            (Event::PenDown { x, y }, View::List(_)) => {
                self.press = self.rows_at[..self.n_rows]
                    .iter()
                    .find(|(r, _)| r.contains_point(x, y))
                    .map(|&(_, id)| (x, id));
                Reply::Nothing
            }
            (Event::Tick, View::List(_)) => match self.ticking {
                Some((id, k)) if k >= 5 => {
                    self.ticking = None;
                    self.tick(cx, id);
                    Reply::Redraw
                }
                Some((id, k)) => {
                    self.ticking = Some((id, k + 1));
                    Reply::Animate
                }
                None => Reply::Nothing,
            },
            (Event::PenUp { x, y }, View::List(_)) => {
                let press = self.press.take();
                // A revealed row's actions.
                if let Some(id) = self.revealed {
                    self.revealed = None;
                    if let Some(a) = self.actions.iter().position(|r| r.contains_point(x, y)) {
                        self.act(cx, id, a as u8);
                    }
                    return Reply::Redraw;
                }
                // A swipe: right ticks, left shows the row's actions.
                if let Some((x0, id)) = press {
                    let dx = i32::from(x) - i32::from(x0);
                    if dx >= 70 {
                        self.ticking = Some((id, 0));
                        return Reply::Animate;
                    }
                    if dx <= -70 {
                        self.revealed = Some(id);
                        return Reply::Redraw;
                    }
                }
                if ADD_LINE.contains_point(x, y) {
                    self.typing = true;
                    return Reply::Redraw;
                }
                let hit = self.boxes[..self.n_boxes]
                    .iter()
                    .find(|(r, _)| r.contains_point(x, y));
                if let Some(&(_, id)) = hit {
                    let done = self.tasks.iter().flatten().any(|t| t.id == id && t.done);
                    if done {
                        self.tick(cx, id);
                        return Reply::Redraw;
                    }
                    self.ticking = Some((id, 0));
                    return Reply::Animate;
                }
                Reply::Nothing
            }
            (Event::Key(Key::SoftRight) | Event::Done, View::List(_)) if self.typing => {
                self.add_typed(cx);
                Reply::Redraw
            }
            (Event::Key(Key::SoftRight), View::List(_)) => {
                self.typing = true;
                Reply::Redraw
            }
            (Event::Key(Key::SoftLeft), View::Home) => Reply::Close,
            (Event::Key(Key::SoftLeft), _) => {
                if self.typing {
                    self.typing = false;
                    self.line_n = 0;
                } else {
                    self.view = View::Home;
                }
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
        (matches!(self.view, View::List(_)) && (self.typing || self.line_n > 0))
            .then_some(Field::Line)
    }
}

#[cfg(test)]
mod tests;
