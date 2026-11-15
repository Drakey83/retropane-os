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
    /// The day it was done (days from 1970), for the day's progress, the
    /// week's count and the streak.
    pub done_day: Option<i32>,
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
    /// The home's progress ring: its picture, the part filled and the look
    /// it was painted for.
    gauge: Option<(Pic, u16, bool)>,
    /// What a tap on the home hits, as last drawn.
    hits: kit::Hits<Hit, 24>,
    /// The D-pad's place among what is drawn.
    focus: kit::Focus,
}

/// What a stylus tap on the home hits (design-language.md version 3:
/// everything drawn answers the stylus; the version 2 home answered none).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    /// A list opened (`None`: Today).
    List(Option<u8>),
    /// A task ticked.
    Tick(u32),
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
            gauge: None,
            hits: kit::Hits::new(),
            focus: kit::Focus::default(),
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
                done_day: r.num(tag::DONE).map(|m| (m / 1440) as i32),
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
        // Only this field changes; the store keeps every other one whole
        // (a copy here cut each to 64 bytes and dropped fields it didn't
        // list: a long note was truncated by a tick, 2026-11-14).
        if which == 0 {
            return;
        }
        if store.change(id, &[(which, value.unwrap_or(&[]))]).is_ok() {
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

    /// The home (design-language.md version 3, "Every app's home has"):
    /// today's progress as a ring, three numbers, the lists as tiles with
    /// their own progress, today's tasks with their boxes. Everything
    /// drawn is a target.
    fn home(&mut self, f: &mut Frame, cx: &Ctx) {
        self.hits.clear();
        let t = tint::TASKS;
        let (ui, label) = (cx.fonts.ui, cx.fonts.label);
        let today = cx.now.day;
        let (due, _, late, _) = self.counts(today);
        let done_on = |day: i32| {
            self.tasks
                .iter()
                .flatten()
                .filter(|t| t.done_day == Some(day))
                .count() as u32
        };
        let done_today = done_on(today);
        let of = due + done_today;
        let week: u32 = (0..7).map(|k| done_on(today - k)).sum();
        // The streak: days in a row with something done, up to today (or
        // yesterday, while today is still open).
        let mut streak = 0u32;
        let mut day = if done_today > 0 { today } else { today - 1 };
        while streak < 999 && done_on(day) > 0 {
            streak += 1;
            day -= 1;
        }
        // The hero.
        let hero = Rect::new(MARGIN, 44, WIDE, 124);
        kit::card(f, cx, hero, b"", t);
        self.hits.add(hero, Hit::List(None));
        let part = (done_today * 256).checked_div(of).unwrap_or(0) as u16;
        if self
            .gauge
            .is_none_or(|(_, p, l)| p != part || l != cx.look.light)
            && let Some(pics) = cx.pics
        {
            if let Some((old, _, _)) = self.gauge.take() {
                pics.free(old);
            }
            self.gauge = kit::ring(pics, 96, 10, part, kit::painted(cx, t), kit::track(cx))
                .map(|p| (p, part, cx.look.light));
        }
        if let Some((p, _, _)) = self.gauge {
            f.push(rp_app::Cmd::Pic {
                x: 28,
                y: 58,
                pic: p,
            });
        }
        let dial = Rect::new(28, 58, 96, 96);
        let mut b = [0u8; 24];
        let n = num(&mut b, done_today);
        kit::centred(f, dial, 116, cx.fonts.num, TEXT, &b[..n]);
        let mut k = 0;
        for &c in b"OF " {
            b[k] = c;
            k += 1;
        }
        k += num(&mut b[k..], of);
        kit::centred(f, dial, 134, label, DIM, &b[..k]);
        kit::text(f, 142, 78, label, t, b"TODAY");
        let words: &[u8] = match (done_today, of) {
            (_, 0) => b"Nothing due today",
            (d, o) if d == o => b"All done",
            (0, _) => b"A fresh start",
            (d, o) if d * 2 >= o => b"More than halfway",
            _ => b"Under way",
        };
        kit::text(f, 142, 110, cx.fonts.title, TEXT, words);
        let mut s = [0u8; 40];
        let mut k = num(&mut s, due);
        for &c in b" to do \x80 " {
            s[k] = c;
            k += 1;
        }
        k += num(&mut s[k..], late);
        for &c in b" late" {
            s[k] = c;
            k += 1;
        }
        kit::text(f, 142, 134, ui, DIM, &s[..k]);
        if streak > 0 {
            kit::round_rect(f, cx, Rect::new(142, 146, 8, 8), kit::AMBER);
            let mut k = num(&mut s, streak);
            for &c in b"-DAY STREAK" {
                s[k] = c;
                k += 1;
            }
            kit::text(f, 158, 156, label, kit::AMBER, &s[..k]);
        }
        // Three numbers.
        for (i, (name, n, c)) in [
            (&b"DUE TODAY"[..], due, t),
            (b"LATE", late, LATE),
            (b"DONE, 7 DAYS", week, kit::DONE),
        ]
        .iter()
        .enumerate()
        {
            let r = Rect::new(MARGIN + i as u16 * 152, 176, 144, 70);
            let mut b = [0u8; 10];
            let bn = num(&mut b, *n);
            kit::stat(f, cx, r, name, &b[..bn], *c);
            self.hits.add(r, Hit::List(None));
        }
        // The lists, each its own colour and progress.
        kit::section(f, cx, 252, b"LISTS", t);
        let rows = self.n_lists.min(4).div_ceil(2) as u16;
        for (i, list) in self.lists[..self.n_lists.min(4)].iter().enumerate() {
            let of_list = |done: bool| {
                self.tasks
                    .iter()
                    .flatten()
                    .filter(|t| usize::from(t.list) == i && (!done || t.done))
                    .count() as u16
            };
            let (all, done) = (of_list(false), of_list(true));
            let r = Rect::new(
                MARGIN + (i as u16 % 2) * 228,
                274 + (i as u16 / 2) * 72,
                220,
                66,
            );
            kit::tinted(f, cx, r, list.colour, 0x58);
            kit::fitted(f, r.x + 16, r.y + 28, ui, TEXT, list.name.get(), 120);
            let mut b = [0u8; 16];
            let mut bn = num(&mut b, u32::from(all - done));
            for &c in b" left" {
                b[bn] = c;
                bn += 1;
            }
            kit::right(f, r.x + r.w - 14, r.y + 28, label, DIM, &b[..bn]);
            kit::progress_bar(
                f,
                Rect::new(r.x + 16, r.y + 44, r.w - 32, 6),
                done,
                all,
                list.colour,
            );
            self.hits.add(r, Hit::List(Some(i as u8)));
        }
        // Today's: the late ones first; a box ticks, the row opens Today.
        let top = 274 + rows * 72 + 4;
        kit::section(f, cx, top, b"TODAY", t);
        let mut open = [None; 3];
        let mut n_open = 0;
        for want_late in [true, false] {
            for task in self.tasks.iter().flatten() {
                let is_late = task.due.is_some_and(|d| d < today);
                if !task.done
                    && task.due.is_some_and(|d| d <= today)
                    && is_late == want_late
                    && n_open < open.len()
                {
                    open[n_open] = Some(*task);
                    n_open += 1;
                }
            }
        }
        let mut y = top + 22;
        for task in open.iter().flatten() {
            if y + 38 > 596 {
                break;
            }
            let r = Rect::new(MARGIN, y, WIDE, 38);
            kit::panel(f, cx, r);
            let c = self.colour_index(task.list);
            kit::round_check(f, cx, r.x + 10, r.y + 7, self.ring[c], self.full[c], false);
            kit::fitted(f, r.x + 46, r.y + 26, ui, TEXT, task.title.get(), 300);
            let mut w = [0u8; 16];
            let (wn, is_late) = Self::due_words(task.due, today, &mut w);
            kit::right(
                f,
                r.x + r.w - 14,
                r.y + 25,
                label,
                if is_late { LATE } else { DIM },
                &w[..wn],
            );
            self.hits.add(r, Hit::List(None));
            self.hits
                .add(Rect::new(r.x, r.y, 44, r.h), Hit::Tick(task.id));
            y += 42;
        }
        if n_open == 0 && y + 38 <= 596 {
            let r = Rect::new(MARGIN, y, WIDE, 38);
            kit::panel(f, cx, r);
            kit::text(f, 36, r.y + 26, ui, DIM, b"Nothing due. Enjoy it.");
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
    fn accent(&self) -> Option<rp_app::Argb> {
        Some(rp_app::kit::tint::TASKS)
    }

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
        match self.view {
            View::Home => {
                self.home(f, cx);
                self.focus.ring(f, self.hits.rects());
            }
            View::List(w) => {
                self.list(f, cx, w);
                self.focus
                    .ring(f, self.rows_at[..self.n_rows].iter().map(|(r, _)| *r));
            }
        }
    }

    fn event(&mut self, e: Event, cx: &Ctx) -> Reply {
        // The D-pad (R203): the arrows walk what is drawn, the centre taps
        // it (kit::Focus); typing keeps the keys.
        if let Event::Key(k) = e
            && !(self.typing && matches!(self.view, View::List(_)))
        {
            let moved = match self.view {
                View::Home => self.focus.step(k, self.hits.rects()),
                View::List(_) => self
                    .focus
                    .step(k, self.rows_at[..self.n_rows].iter().map(|(r, _)| *r)),
            };
            if moved {
                return Reply::Redraw;
            }
            if k == Key::Enter
                && let Some((x, y)) = self.focus.point()
            {
                self.event(Event::PenDown { x, y }, cx);
                return self.event(Event::PenUp { x, y }, cx);
            }
        }
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
            (Event::PenUp { x, y }, View::Home) => match self.hits.at(x, y) {
                Some(Hit::List(which)) => {
                    self.view = View::List(which);
                    Reply::Redraw
                }
                Some(Hit::Tick(id)) => {
                    self.tick(cx, id);
                    Reply::Redraw
                }
                None => Reply::Nothing,
            },
            // The D-pad's centre opens Today (R203: the D-pad works in
            // every app).
            (Event::Key(Key::Enter), View::Home) => {
                self.view = View::List(None);
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
