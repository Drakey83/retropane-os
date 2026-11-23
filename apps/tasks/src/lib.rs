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
    /// Its time that day (minutes from midnight), if it has one: the
    /// shell reminds of it then.
    pub time: Option<u16>,
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
    /// A repeating task's past completions (it moves on to its next day
    /// when ticked, so these count its done days).
    pub log: Log,
}

/// The days a repeating task was done (`tag::DONE_LOG`), newest last.
#[derive(Clone, Copy, Debug, Default)]
pub struct Log {
    days: [u16; tag::DONE_LOG_MAX],
    n: u8,
}

impl Log {
    /// Read from the field's bytes.
    #[must_use]
    pub fn of(b: &[u8]) -> Self {
        let mut l = Self::default();
        for c in b.as_chunks::<2>().0.iter().take(tag::DONE_LOG_MAX) {
            l.days[usize::from(l.n)] = u16::from_le_bytes(*c);
            l.n += 1;
        }
        l
    }

    /// Whether it was done on `day`.
    #[must_use]
    pub fn has(&self, day: i32) -> bool {
        self.days[..usize::from(self.n)]
            .iter()
            .any(|&d| i32::from(d) == day)
    }

    /// With `day` added (the oldest dropped when full), as the field's
    /// bytes and their length.
    #[must_use]
    pub fn with(&self, day: i32) -> ([u8; 2 * tag::DONE_LOG_MAX], usize) {
        let mut out = [0u8; 2 * tag::DONE_LOG_MAX];
        let kept = &self.days[..usize::from(self.n)];
        let kept = &kept[kept.len().saturating_sub(tag::DONE_LOG_MAX - 1)..];
        let mut k = 0;
        for &d in kept.iter().chain(&[day.clamp(0, 0xFFFF) as u16]) {
            out[k..k + 2].copy_from_slice(&d.to_le_bytes());
            k += 2;
        }
        (out, k)
    }
}

/// The views.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    /// The smart tiles and the lists.
    Home,
    /// One list (index), or Today when none.
    List(Option<u8>),
    /// The lists themselves: made, renamed, recoloured, deleted.
    Lists,
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
    /// The time row of the task whose actions show (a touch sets its
    /// time), and the pad setting a task's time, if it is up.
    time_keys: [Rect; 1],
    pad: Option<(u32, rp_app::timepad::TimePad)>,
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
    /// A list's page: the list it is of, the first task of it, the pages
    /// before it, the task after its last, how many the list shows;
    /// whether the done ones are shown under the open ones, the row that
    /// says how many are done, and where the stylus went down (a drag up
    /// or down turns the page).
    listed: Option<Option<u8>>,
    first: u8,
    pages: [u8; 12],
    n_pages: usize,
    next: u8,
    total: u8,
    show_done: bool,
    /// The list shown is the search: every list's tasks, the done ones
    /// too, whose title holds what is typed in the line.
    find: bool,
    done_row: Option<Rect>,
    down_y: Option<u16>,
    /// The lists' screen: the list being renamed (the line holds its
    /// name), the one Delete was pressed on once, and its keys on screen
    /// (the list and what the key does: 0 open, 1 colour, 2 name, 3
    /// delete).
    renaming: Option<u8>,
    ask_delete: Option<u8>,
    list_keys: [(Rect, u8, u8); 32],
    n_keys: usize,
    /// The steps of the task whose actions show, on screen: each one's
    /// record and whether it is done.
    steps_at: [(Rect, u32, bool); 6],
    n_steps: usize,
}

/// What a stylus tap on the home hits (design-language.md version 3:
/// everything drawn answers the stylus; the version 2 home answered none).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    /// The lists' own screen opened.
    Lists,
    /// The search across every list opened.
    Find,
    /// A list opened (`None`: Today).
    List(Option<u8>),
    /// A task ticked.
    Tick(u32),
    /// A task opened in Today's list: its actions, time and steps.
    Open(u32),
}

/// The home's SEARCH key (on the LISTS line's right).
const FIND_KEY: Rect = Rect::new(MARGIN + WIDE - 96, 249, 96, 22);

/// The home's EDIT key (the lists' own screen), beside SEARCH.
const LISTS_KEY: Rect = Rect::new(FIND_KEY.x - 76, 249, 70, 22);

/// Where `hay` holds `needle`, whatever its capitals (an empty one: at
/// the start).
fn holds(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    (hay.len() >= needle.len())
        .then(|| {
            hay.windows(needle.len())
                .position(|w| w.eq_ignore_ascii_case(needle))
        })
        .flatten()
}

const FLAGS: [u32; 4] = [0, kit::IDENTITY[4], kit::AMBER, LATE];

/// `m` minutes from midnight in the clock chosen: "3:05 PM" or "15:05"
/// (R383: the one writer every app shares).
fn clock(m: u16, clock12: bool, out: &mut [u8; 8]) -> usize {
    let t = rp_app::clock::time(m, clock12);
    let n = t.get().len();
    out[..n].copy_from_slice(t.get());
    n
}

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
            time_keys: [Rect::new(0, 0, 0, 0); 1],
            pad: None,
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
            listed: None,
            first: 0,
            pages: [0; 12],
            n_pages: 0,
            next: 0,
            total: 0,
            show_done: false,
            find: false,
            done_row: None,
            down_y: None,
            renaming: None,
            ask_delete: None,
            list_keys: [(Rect::new(0, 0, 0, 0), 0, 0); 32],
            n_keys: 0,
            steps_at: [(Rect::new(0, 0, 0, 0), 0, false); 6],
            n_steps: 0,
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
                time: r.num(tag::REMIND_AT).map(|m| (m % date::DAY_MIN) as u16),
                flag: r.num(tag::PRIORITY).unwrap_or(0).min(3) as u8,
                list,
                done: r.num(tag::DONE).is_some(),
                done_day: r.num(tag::DONE).map(|m| (m / 1440) as i32),
                steps: (done, of),
                repeats: r.num(tag::REPEAT).is_some(),
                log: Log::of(r.text(tag::DONE_LOG)),
            });
        }
    }

    /// The typed line kept as a task: the quick parser's date and "!"
    /// priority (`!1` high .. `!3` low, as Todoist writes them; a bare `!`
    /// high), `#list` names a list.
    fn add_typed(&mut self, cx: &Ctx) -> bool {
        // The repeat first ("every mon wed", "every 3 days after done"):
        // its words out of the line before the date words are read, so a
        // weekday in it is a repeat and not a due date.
        let (rest, rest_n, repeat) = Self::take_repeat(&self.line[..self.line_n]);
        let line = &rest[..rest_n];
        let mut title = [0u8; rp_pim::quick::TITLE_MAX];
        let (mut q, tn) = rp_pim::quick::parse(line, cx.now.day, cx.us, &mut title);
        // Weekly on named days with no date typed: due on the first of
        // them from today.
        if let Some(r) = repeat
            && q.day.is_none()
            && r.every == rp_pim::repeat::Every::Week
            && r.days != 0
        {
            q.day = (cx.now.day..cx.now.day + 7).find(|&d| r.days & (1 << date::weekday(d)) != 0);
        }
        // The title without "#list" and "!n" words the parser left in.
        let mut clean = [0u8; rp_pim::quick::TITLE_MAX];
        let mut cn = 0;
        let mut flag = q.priority.map_or(0u32, |p| if p >= 2 { 3 } else { 1 });
        // The list it is typed in (it went to the first list whatever
        // list was open, unless "#name" said otherwise).
        let open = match self.view {
            View::List(Some(i)) => usize::from(i),
            _ => 0,
        };
        let mut list_id = self.lists[open.min(self.n_lists.saturating_sub(1))].id;
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
        // A time typed ("3pm", "15:30"): the task's time that day, and
        // when the shell's banner reminds of it.
        let at = q
            .time
            .map(|t| (date::start_of(due_day) + u32::from(t)).to_le_bytes());
        let mut fields: [(u8, &[u8]); 6] = [
            (tag::TITLE, &clean[..cn]),
            (tag::DUE, &due),
            (tag::PRIORITY, &[]),
            (tag::LIST, &[]),
            (tag::REPEAT, &[]),
            (tag::REMIND_AT, &[]),
        ];
        if let Some(a) = &at {
            fields[5].1 = a;
        }
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
        // A step of the task whose actions show: its title and its task.
        let parent = self.revealed.map(u32::to_le_bytes);
        let ok = match &parent {
            Some(p) => store
                .put(
                    Kind::Task,
                    None,
                    &[(tag::TITLE, &clean[..cn]), (tag::PARENT, p)],
                )
                .is_ok(),
            None => store.put(Kind::Task, None, &fields).is_ok(),
        };
        if ok {
            self.seen = None;
            self.line_n = 0;
        }
        ok
    }

    /// The line with its repeat phrase ("every" and the words
    /// [`rp_pim::repeat::phrase`] reads after it) taken out, and the rule.
    fn take_repeat(line: &[u8]) -> ([u8; 64], usize, Option<rp_pim::repeat::Rule>) {
        let cut = Self::repeat_words(line);
        let words = line.split(|&b| b == b' ').filter(|w| !w.is_empty());
        let mut out = [0u8; 64];
        let mut k = 0;
        for (i, w) in words.enumerate() {
            if cut.is_some_and(|(a, b, _)| i >= a && i <= b) {
                continue;
            }
            for &c in w.iter().chain(b" ") {
                if k < out.len() {
                    out[k] = c;
                    k += 1;
                }
            }
        }
        (out, k.saturating_sub(1), cut.map(|(_, _, r)| r))
    }

    /// Where the line's repeat phrase lies: its first and last word (the
    /// "every" and the last word [`rp_pim::repeat::phrase`] took), and the
    /// rule. The add line highlights exactly these words.
    fn repeat_words(line: &[u8]) -> Option<(usize, usize, rp_pim::repeat::Rule)> {
        let mut words: [&[u8]; 16] = [b""; 16];
        let mut n = 0;
        for w in line.split(|&b| b == b' ').filter(|w| !w.is_empty()) {
            if n < words.len() {
                words[n] = w;
                n += 1;
            }
        }
        (0..n).find_map(|i| {
            if !words[i].eq_ignore_ascii_case(b"every") {
                return None;
            }
            rp_pim::repeat::phrase(&words[i + 1..n]).map(|(rule, used)| (i, i + used, rule))
        })
    }

    /// The lists kept in the store, if they were only the ones a new
    /// device shows (no records yet): a list needs its record before it
    /// can be renamed or a task can name it.
    fn ensure_lists(&mut self, cx: &Ctx) {
        if self.lists[..self.n_lists].iter().any(|l| l.id != 0) {
            return;
        }
        if let Some(mut guard) = cx.pim.and_then(|c| c.try_borrow_mut().ok())
            && let Some(store) = guard.as_mut()
        {
            for l in &self.lists[..self.n_lists] {
                let c = kit::IDENTITY
                    .iter()
                    .position(|&x| x == l.colour)
                    .unwrap_or(4) as u32;
                let _ = store.put(
                    Kind::List,
                    None,
                    &[(tag::NAME, l.name.get()), (tag::COLOUR, &c.to_le_bytes())],
                );
            }
        }
        self.seen = None;
        self.refresh(cx);
    }

    /// The line kept on the lists' screen: the new name of the list
    /// being renamed, or a new list (eight at most).
    fn keep_list(&mut self, cx: &Ctx) {
        let renaming = self.renaming.take();
        self.typing = false;
        if self.line_n == 0 {
            return;
        }
        self.ensure_lists(cx);
        let (name, n) = (self.line, self.line_n.min(24));
        if let Some(mut guard) = cx.pim.and_then(|c| c.try_borrow_mut().ok())
            && let Some(store) = guard.as_mut()
        {
            match renaming.and_then(|i| self.lists.get(usize::from(i))) {
                Some(l) => {
                    let _ = store.change(l.id, &[(tag::NAME, &name[..n])]);
                }
                None if self.n_lists < self.lists.len() => {
                    let c = (self.n_lists as u32 % 8).to_le_bytes();
                    let _ = store.put(
                        Kind::List,
                        None,
                        &[(tag::NAME, &name[..n]), (tag::COLOUR, &c)],
                    );
                }
                None => {}
            }
        }
        self.line_n = 0;
        self.seen = None;
    }

    /// A key of the lists' screen pressed on list `i`: 0 opens it, 1
    /// gives it the next colour, 2 renames it, 3 deletes it (asked twice;
    /// its tasks go to another list, and the last list stays).
    fn list_key(&mut self, cx: &Ctx, i: u8, what: u8) {
        if what != 3 {
            self.ask_delete = None;
        }
        match what {
            0 => {
                self.view = View::List(Some(i));
                self.focus.clear();
            }
            2 => {
                let name = self.lists[usize::from(i)].name;
                self.set_line(name.get());
                self.renaming = Some(i);
                self.typing = true;
            }
            3 if self.ask_delete != Some(i) => {
                if self.n_lists > 1 {
                    self.ask_delete = Some(i);
                }
            }
            _ => {
                self.ensure_lists(cx);
                let Some(l) = self.lists.get(usize::from(i)).copied() else {
                    return;
                };
                if let Some(mut guard) = cx.pim.and_then(|c| c.try_borrow_mut().ok())
                    && let Some(store) = guard.as_mut()
                {
                    if what == 1 {
                        let c = kit::IDENTITY
                            .iter()
                            .position(|&x| x == l.colour)
                            .unwrap_or(4);
                        let c = ((c + 1) % 8) as u32;
                        let _ = store.change(l.id, &[(tag::COLOUR, &c.to_le_bytes())]);
                    } else if let Some(to) =
                        self.lists[..self.n_lists].iter().find(|o| o.id != l.id)
                    {
                        // Its tasks first (one at a time: an id list of
                        // them all would be a second copy of the store's).
                        let to = to.id.to_le_bytes();
                        loop {
                            let next = store
                                .all(Kind::Task)
                                .find(|r| r.num(tag::LIST) == Some(l.id))
                                .map(|r| r.id);
                            match next {
                                Some(id) if store.change(id, &[(tag::LIST, &to)]).is_ok() => {}
                                _ => break,
                            }
                        }
                        let _ = store.remove(l.id);
                    }
                }
                self.ask_delete = None;
                self.seen = None;
            }
        }
    }

    /// The lists' own screen (the first version showed four lists a
    /// build gave it and let none be made or named): every list a row,
    /// its name opening it, its keys changing it; the line makes a new
    /// one or renames. Windows Mobile's Tasks has categories in a menu
    /// of a menu; a phone's keeps list editing behind a long press.
    #[inline(never)]
    fn lists_view(&mut self, f: &mut Frame, cx: &Ctx) {
        self.n_keys = 0;
        let (ui, label) = (cx.fonts.ui, cx.fonts.label);
        kit::large_title(
            f,
            cx,
            36,
            b"Lists",
            b"A name opens its list; the keys change it",
            tint::TASKS,
        );
        kit::panel(f, cx, ADD_LINE);
        if self.typing {
            f.push(rp_app::Cmd::Outline {
                r: ADD_LINE,
                color: kit::FOCUS,
            });
        }
        let line = &self.line[..self.line_n];
        if line.is_empty() {
            let hint: &[u8] = if self.renaming.is_some() {
                b"Its new name"
            } else if self.n_lists == self.lists.len() {
                b"Eight lists at most"
            } else {
                b"+ New list: its name"
            };
            kit::text(f, ADD_LINE.x + 14, ADD_LINE.y + 27, ui, DIM, hint);
        } else {
            kit::text(f, ADD_LINE.x + 14, ADD_LINE.y + 27, ui, TEXT, line);
        }
        let mut y = ADD_LINE.y + ADD_LINE.h + 10;
        for i in 0..self.n_lists {
            let l = self.lists[i];
            let r = Rect::new(MARGIN, y, WIDE, 44);
            kit::panel(f, cx, r);
            kit::colour_bar(f, r, l.colour);
            let left = self
                .tasks
                .iter()
                .flatten()
                .filter(|t| usize::from(t.list) == i && !t.done)
                .count() as u32;
            kit::fitted(f, r.x + 20, r.y + 21, ui, TEXT, l.name.get(), 196);
            let mut b = [0u8; 16];
            let mut bn = num(&mut b, left);
            for &c in b" left" {
                b[bn] = c;
                bn += 1;
            }
            kit::text(f, r.x + 20, r.y + 39, label, DIM, &b[..bn]);
            let keys = r.w - 3 * 76 - 4;
            let mut hit = |r: Rect, what: u8| {
                if self.n_keys < self.list_keys.len() {
                    self.list_keys[self.n_keys] = (r, i as u8, what);
                    self.n_keys += 1;
                }
            };
            hit(Rect::new(r.x, r.y, keys, r.h), 0);
            let asked = self.ask_delete == Some(i as u8);
            let words: [&[u8]; 3] = [b"COLOR", b"NAME", if asked { b"SURE?" } else { b"DELETE" }];
            for (k, w) in words.iter().enumerate() {
                let a = Rect::new(r.x + keys + k as u16 * 76, r.y + 5, 72, r.h - 10);
                // The COLOR key is the list's colour itself, so a press
                // shows (a 4 px bar changing between two soft colours read
                // as nothing happening: the owner on the X51v, 2026-11-15).
                if k == 0 {
                    kit::round_rect(f, cx, a, l.colour);
                    kit::centred(f, a, a.y + 22, label, kit::keep(0xFF10_1418), w);
                    hit(a, 1);
                    continue;
                }
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
                    a.y + 22,
                    label,
                    if k == 2 { kit::LATE } else { TEXT },
                    w,
                );
                hit(a, k as u8 + 1);
            }
            y += 48;
        }
    }

    /// An event on the lists' screen.
    fn lists_event(&mut self, e: Event, cx: &Ctx) -> Reply {
        match e {
            Event::PenDown { .. } => {
                self.focus.clear();
                Reply::Nothing
            }
            Event::PenUp { x, y } => {
                if ADD_LINE.contains_point(x, y) {
                    self.typing = true;
                    return Reply::Redraw;
                }
                let hit = self.list_keys[..self.n_keys]
                    .iter()
                    .find(|k| k.0.contains_point(x, y))
                    .map(|k| (k.1, k.2));
                match hit {
                    Some((i, what)) => self.list_key(cx, i, what),
                    None => self.ask_delete = None,
                }
                Reply::Redraw
            }
            Event::Char(c) if self.typing && self.line_n < 24 => {
                self.line[self.line_n] = c;
                self.line_n += 1;
                Reply::Redraw
            }
            Event::Delete if self.typing => {
                self.line_n = self.line_n.saturating_sub(1);
                Reply::Redraw
            }
            Event::Done => {
                self.keep_list(cx);
                Reply::Redraw
            }
            Event::Key(Key::SoftRight) => {
                if self.typing {
                    self.keep_list(cx);
                } else {
                    self.typing = true;
                    self.renaming = None;
                    self.line_n = 0;
                }
                Reply::Redraw
            }
            Event::Key(Key::SoftLeft) => {
                if self.typing {
                    self.typing = false;
                    self.renaming = None;
                    self.line_n = 0;
                } else {
                    self.ask_delete = None;
                    self.view = View::Home;
                    self.focus.clear();
                }
                Reply::Redraw
            }
            Event::Key(Key::Enter) => match self.focus.point() {
                Some((x, y)) => self.lists_event(Event::PenUp { x, y }, cx),
                None => Reply::Nothing,
            },
            Event::Key(k) if !self.typing => {
                let mut focus = self.focus;
                let moved = focus.step(
                    k,
                    core::iter::once(ADD_LINE)
                        .chain(self.list_keys[..self.n_keys].iter().map(|k| k.0)),
                );
                self.focus = focus;
                if moved { Reply::Redraw } else { Reply::Nothing }
            }
            _ => Reply::Nothing,
        }
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

    /// Task `id` ticked done (with the minute), or opened again. A
    /// repeating one moves on to its next day instead, today added to its
    /// log and its steps opened again (Things resets a checklist the same
    /// way); once its rule has ended it is done like any other.
    fn tick(&mut self, cx: &Ctx, id: u32) {
        let (done, repeats) = cx
            .pim
            .and_then(|c| c.try_borrow().ok())
            .and_then(|g| {
                g.as_ref()
                    .and_then(|s| s.get(id))
                    .map(|r| (r.num(tag::DONE).is_some(), r.num(tag::REPEAT).is_some()))
            })
            .unwrap_or((false, false));
        let minute = (date::start_of(cx.now.day) + u32::from(cx.now.minute)).to_le_bytes();
        if !done && repeats && self.roll(cx, id) {
            return;
        }
        self.set_field(cx, id, tag::DONE, (!done).then_some(&minute[..]));
    }

    /// Repeating task `t` done today: moved to its next day. False when
    /// its rule has ended (or the store is busy): then it is done.
    fn roll(&mut self, cx: &Ctx, id: u32) -> bool {
        let Some(cell) = cx.pim else { return false };
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return false;
        };
        let Some(store) = guard.as_mut() else {
            return false;
        };
        let today = cx.now.day;
        let Some(r) = store.get(id) else { return false };
        let due = r.num(tag::DUE).map_or(today, |d| d as i32);
        let time = r.num(tag::REMIND_AT).map(|m| m % date::DAY_MIN);
        let log = Log::of(r.text(tag::DONE_LOG));
        let Some(rule) = r
            .num(tag::REPEAT)
            .and_then(|v| rp_pim::repeat::Rule::unpack(v, due))
        else {
            return false;
        };
        let Some(next) = rule.next_after(due, due, today) else {
            return false;
        };
        // The rule kept from its new day (its end stays the same day:
        // `pack` counts it from the day it is given). The new day is one
        // of its days, so the every-N beat stays the same. On its last
        // day it is a plain task (an end on the start day packs as none).
        let packed = rule.pack(next).to_le_bytes();
        let packed: &[u8] = if rule.until == Some(next) {
            &[]
        } else {
            &packed
        };
        let (log, log_n) = log.with(today);
        let day = (next as u32).to_le_bytes();
        let at = time.map(|m| (date::start_of(next) + m).to_le_bytes());
        let mut fields: [(u8, &[u8]); 4] = [
            (tag::DUE, &day),
            (tag::REPEAT, packed),
            (tag::DONE_LOG, &log[..log_n]),
            (tag::REMIND_AT, &[]),
        ];
        let n = match &at {
            Some(a) => {
                fields[3].1 = a;
                4
            }
            None => 3,
        };
        if store.change(id, &fields[..n]).is_err() {
            return false;
        }
        // Its steps open again for the next time.
        loop {
            let step = store
                .all(Kind::Task)
                .find(|r| r.num(tag::PARENT) == Some(id) && r.num(tag::DONE).is_some())
                .map(|r| r.id);
            match step {
                Some(s) if store.change(s, &[(tag::DONE, &[])]).is_ok() => {}
                _ => break,
            }
        }
        self.seen = None;
        true
    }

    /// A swipe's action on task `id`: 0 tomorrow, 1 next week, 2 delete.
    fn act(&mut self, cx: &Ctx, id: u32, a: u8) {
        if a == 2 {
            if let Some(cell) = cx.pim
                && let Ok(mut g) = cell.try_borrow_mut()
                && let Some(store) = g.as_mut()
            {
                // Its steps go with it.
                loop {
                    let step = store
                        .all(Kind::Task)
                        .find(|r| r.num(tag::PARENT) == Some(id))
                        .map(|r| r.id);
                    match step {
                        Some(s) if store.remove(s).is_ok() => {}
                        _ => break,
                    }
                }
                if store.remove(id).is_ok() {
                    self.seen = None;
                }
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
        // Its time goes with it to the new day.
        if let Some(t) = self
            .tasks
            .iter()
            .flatten()
            .find(|t| t.id == id)
            .and_then(|t| t.time)
        {
            let at = (date::start_of(day) + u32::from(t)).to_le_bytes();
            self.set_field(cx, id, tag::REMIND_AT, Some(&at));
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
                .filter(|t| t.done_day == Some(day) || t.log.has(day))
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
        // Search across every list (Windows Mobile's Tasks has none; a
        // phone's searches a list at a time).
        kit::key(f, cx, FIND_KEY, kit::KeyState::Rest);
        kit::centred(f, FIND_KEY, FIND_KEY.y + 16, label, TEXT, b"SEARCH");
        self.hits.add(FIND_KEY, Hit::Find);
        kit::key(f, cx, LISTS_KEY, kit::KeyState::Rest);
        kit::centred(f, LISTS_KEY, LISTS_KEY.y + 16, label, TEXT, b"EDIT");
        self.hits.add(LISTS_KEY, Hit::Lists);
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
            self.hits.add(r, Hit::Open(task.id));
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
        self.n_steps = 0;
        self.done_row = None;
        // Another list: from its first page, the done ones folded away.
        if self.listed != Some(which) {
            self.listed = Some(which);
            self.first = 0;
            self.n_pages = 0;
            self.show_done = false;
        }
        let (name, colour): (&[u8], u32) = match which {
            Some(i) => self
                .lists
                .get(usize::from(i))
                .map_or((b"List", tint::TASKS), |l| (l.name.get(), l.colour)),
            None => (b"Today", tint::TASKS),
        };
        let find = self.find;
        let (name, sub): (&[u8], &[u8]) = if find {
            (b"Search", b"Every list, the done ones too")
        } else {
            (name, b"Tap the circle when it's done")
        };
        // (A copy: the line is read while the rows are recorded.)
        let (typed, typed_n) = (self.line, self.line_n);
        let q = &typed[..typed_n];
        let mut y = kit::large_title(f, cx, 36, name, sub, colour);
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
                if find {
                    b"A word of the task"
                } else if self.revealed.is_some() {
                    b"+ Add a step to this task"
                } else {
                    b"+ Add a task: Pay rent fri 3pm !"
                },
            );
        } else {
            let mut x = field.x + 14;
            let rep = Self::repeat_words(&self.line[..self.line_n]);
            for (i, w) in self.line[..self.line_n]
                .split(|&b| b == b' ')
                .filter(|w| !w.is_empty())
                .enumerate()
            {
                let understood = !find
                    && (rep.is_some_and(|(a, b, _)| i >= a && i <= b)
                        || w.starts_with(b"!")
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
                        ]
                        .contains(&w));
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
            _ if find => holds(t.title.get(), q).is_some(),
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
        // The done ones after them, once asked for (a task ticked by
        // mistake could not be seen again, so never unticked).
        if self.show_done || find {
            for (k, t) in self.tasks.iter().enumerate() {
                if t.as_ref().is_some_and(|t| mine(&t) && t.done) {
                    open[n] = k as u8;
                    n += 1;
                }
            }
        }
        // A page of them (a list with more than fit was cut off at the
        // screen's foot: the rest out of the stylus's reach and the
        // D-pad's): a drag or the D-pad turns the page.
        if usize::from(self.first) >= n {
            self.first = 0;
            self.n_pages = 0;
        }
        self.total = n as u8;
        let mut next = n;
        let mut late_said = false;
        let mut done_said = false;
        for (j, &k) in open[..n].iter().enumerate().skip(usize::from(self.first)) {
            let Some(t) = self.tasks[usize::from(k)] else {
                continue;
            };
            let t = &t;
            let head = (!late_said && !t.done && t.due.is_some_and(|d| d < today))
                || (!done_said && t.done);
            if y + 50 + if head { 22 } else { 0 } > 556 {
                next = j;
                break;
            }
            if !late_said && !t.done && t.due.is_some_and(|d| d < today) {
                kit::section(f, cx, y, b"LATE", LATE);
                y += 22;
                late_said = true;
            }
            if !done_said && t.done {
                kit::section(f, cx, y, b"DONE", DIM);
                y += 22;
                done_said = true;
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
                t.done || frame.is_some_and(|k| k >= 3),
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
            let ink = if t.done { DIM } else { TEXT };
            kit::fitted(f, r.x + 48, r.y + 21, cx.fonts.ui, ink, t.title.get(), 250);
            // The match in the title, in the system accent.
            if find
                && !q.is_empty()
                && let Some(m) = holds(t.title.get(), q)
            {
                let title = t.title.get();
                let x = r.x + 48 + cx.fonts.ui.width(&title[..m]);
                if x < r.x + 48 + 250 {
                    kit::text(
                        f,
                        x,
                        r.y + 21,
                        cx.fonts.ui,
                        kit::FOCUS,
                        &title[m..m + q.len()],
                    );
                }
            }
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
                // Its time (typed "3pm" in the line was the only way, and
                // a task already made had none: the owner on the X51v,
                // 2026-11-15, "no way to assign a time still"; then keys a
                // quarter of an hour apart: "should be able to be selected
                // by the min"). The shell reminds of it then.
                let tr = Rect::new(MARGIN + 30, y, WIDE - 30, 40);
                kit::panel(f, cx, tr);
                kit::text(f, tr.x + 12, tr.y + 26, cx.fonts.label, DIM, b"TIME");
                let mut tb = [0u8; 8];
                let (shown, col): (&[u8], _) = match t.time {
                    Some(m) => {
                        let n = clock(m, cx.clock12, &mut tb);
                        (&tb[..n], TEXT)
                    }
                    None => (b"None", DIM),
                };
                kit::text(f, tr.x + 70, tr.y + 27, cx.fonts.ui, col, shown);
                // The whole row is the key: a touch brings the time pad
                // (to the minute, typed: timepad.rs says whose design).
                let a = Rect::new(tr.right() - 112, tr.y + 4, 108, 32);
                kit::key(f, cx, a, kit::KeyState::Rest);
                kit::centred(f, a, a.y + 21, cx.fonts.label, TEXT, b"SET TIME");
                self.time_keys[0] = tr;
                y += 44;
                // Its steps, each with its own circle (the row said "2 of
                // 5" and there was no way to make or tick one); the line
                // above adds one.
                if let Some(guard) = cx.pim.and_then(|c| c.try_borrow().ok())
                    && let Some(store) = guard.as_ref()
                {
                    for c in store
                        .all(Kind::Task)
                        .filter(|c| c.num(tag::PARENT) == Some(t.id))
                    {
                        if self.n_steps == self.steps_at.len() || y + 38 > 556 {
                            break;
                        }
                        let sr = Rect::new(MARGIN + 30, y, WIDE - 30, 34);
                        let done = c.num(tag::DONE).is_some();
                        kit::panel(f, cx, sr);
                        kit::round_check(
                            f,
                            cx,
                            sr.x + 8,
                            sr.y + 5,
                            self.ring[ci],
                            self.full[ci],
                            done,
                        );
                        kit::fitted(
                            f,
                            sr.x + 42,
                            sr.y + 23,
                            cx.fonts.ui,
                            if done { DIM } else { TEXT },
                            c.text(tag::TITLE),
                            sr.w - 56,
                        );
                        self.steps_at[self.n_steps] = (sr, c.id, done);
                        self.n_steps += 1;
                        y += 38;
                    }
                }
                continue;
            }
            let mut w = [0u8; 16];
            let (wn, late) = Self::due_words(t.due, today, &mut w);
            let mut sub = [0u8; 48];
            let mut sn = 0;
            for &c in &w[..wn] {
                sub[sn] = c;
                sn += 1;
            }
            // Its time after its day: "Today 3:00 PM".
            if let Some(m) = t.time {
                let mut tb = [0u8; 8];
                let tn = clock(m, cx.clock12, &mut tb);
                let gap = usize::from(sn > 0);
                for &c in b" "[..gap].iter().chain(&tb[..tn]) {
                    sub[sn] = c;
                    sn += 1;
                }
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
        self.next = next as u8;
        // The foot: how many are done (a tap shows or folds them away),
        // and where this page sits.
        let done = self
            .tasks
            .iter()
            .flatten()
            .filter(mine)
            .filter(|t| t.done)
            .count();
        let foot = Rect::new(MARGIN, 560, WIDE, 36);
        let mut b = [0u8; 40];
        if done > 0 && !find {
            kit::panel(f, cx, foot);
            let ci = self.colour_index(which.unwrap_or(0));
            kit::round_check(
                f,
                cx,
                foot.x + 12,
                foot.y + 6,
                self.ring[ci],
                self.full[ci],
                true,
            );
            let mut bn = num(&mut b, done as u32);
            for &c in b" done".iter().chain(if self.show_done {
                &b" \x80 hide"[..]
            } else {
                b" \x80 show"
            }) {
                b[bn] = c;
                bn += 1;
            }
            kit::text(f, foot.x + 48, foot.y + 25, cx.fonts.ui, DIM, &b[..bn]);
            self.done_row = Some(foot);
        }
        if next < n || self.first > 0 {
            let mut bn = num(&mut b, u32::from(self.first) + 1);
            b[bn] = b'-';
            bn += 1;
            bn += num(&mut b[bn..], next as u32);
            for &c in b" of " {
                b[bn] = c;
                bn += 1;
            }
            bn += num(&mut b[bn..], n as u32);
            kit::right(
                f,
                foot.right() - 14,
                foot.y + 24,
                cx.fonts.label,
                DIM,
                &b[..bn],
            );
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
            _ if self.pad.is_some() => ("Cancel", "Set"),
            View::Home => ("Back", "New"),
            View::List(_) if self.find => ("Back", "Find"),
            View::List(_) => ("Back", "Add"),
            View::Lists => ("Back", "New"),
        }
    }

    fn draw(&mut self, f: &mut Frame, cx: &Ctx) {
        self.paint(cx);
        self.refresh(cx);
        if let Some((_, pad)) = &self.pad {
            pad.draw(f, cx, b"TIME");
            return;
        }
        match self.view {
            View::Home => {
                self.home(f, cx);
                self.focus.ring(f, self.hits.rects());
            }
            View::List(w) => {
                self.list(f, cx, w);
                let (t, n) = self.list_targets();
                self.focus.ring(f, t[..n].iter().copied());
            }
            View::Lists => {
                self.lists_view(f, cx);
                self.focus.ring(
                    f,
                    core::iter::once(ADD_LINE)
                        .chain(self.list_keys[..self.n_keys].iter().map(|k| k.0)),
                );
            }
        }
    }

    fn event(&mut self, e: Event, cx: &Ctx) -> Reply {
        // The time pad takes everything while it is up.
        if let Some((id, mut pad)) = self.pad {
            use rp_app::timepad::Answer;
            return match pad.event(e) {
                Answer::Set(v) => {
                    self.pad = None;
                    self.set_time(cx, id, v.map(|m| m as u16));
                    Reply::Redraw
                }
                Answer::Cancel => {
                    self.pad = None;
                    Reply::Redraw
                }
                Answer::Redraw => {
                    self.pad = Some((id, pad));
                    Reply::Redraw
                }
                Answer::Nothing => Reply::Nothing,
            };
        }
        if self.view == View::Lists {
            return self.lists_event(e, cx);
        }
        // The D-pad (R203): the arrows walk what is drawn, the centre taps
        // it (kit::Focus); typing keeps the keys.
        if let Event::Key(k) = e
            && !(self.typing && matches!(self.view, View::List(_)))
        {
            let list = matches!(self.view, View::List(_));
            let (t, n) = self.list_targets();
            let moved = match self.view {
                View::Home => self.focus.step(k, self.hits.rects()),
                _ => self.focus.step(k, t[..n].iter().copied()),
            };
            if moved {
                return Reply::Redraw;
            }
            // Past a list's last row or its first: the next page, or the
            // one before.
            if list && ((k == Key::Down && self.page_on()) || (k == Key::Up && self.page_back())) {
                self.focus.clear();
                return Reply::Redraw;
            }
            // The centre on a task's row shows its actions (as a swipe
            // left does), which then take the arrows; on anything else it
            // taps.
            if k == Key::Enter
                && list
                && self.revealed.is_none()
                && let Some(&(_, id)) = self.rows_at[..self.n_rows]
                    .iter()
                    .find(|(r, _)| Some(*r) == self.focus.rect())
            {
                self.revealed = Some(id);
                self.focus.clear();
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
                self.focus.clear();
                self.down_y = Some(y);
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
                    // A step ticked, or the line to add one: the task
                    // stays open. An action, or a tap elsewhere, closes it.
                    let step = self.steps_at[..self.n_steps]
                        .iter()
                        .find(|s| s.0.contains_point(x, y))
                        .map(|s| (s.1, s.2));
                    if let Some((step, done)) = step {
                        let minute =
                            (date::start_of(cx.now.day) + u32::from(cx.now.minute)).to_le_bytes();
                        self.set_field(cx, step, tag::DONE, (!done).then_some(&minute[..]));
                        return Reply::Redraw;
                    }
                    if ADD_LINE.contains_point(x, y) {
                        self.typing = true;
                        return Reply::Redraw;
                    }
                    if self.time_keys[0].contains_point(x, y) {
                        let time = self
                            .tasks
                            .iter()
                            .flatten()
                            .find(|t| t.id == id)
                            .and_then(|t| t.time);
                        self.pad = Some((id, rp_app::timepad::TimePad::clock(time, cx.clock12)));
                        return Reply::Redraw;
                    }
                    self.revealed = None;
                    self.typing = false;
                    self.line_n = 0;
                    if let Some(a) = self.actions.iter().position(|r| r.contains_point(x, y)) {
                        self.act(cx, id, a as u8);
                    }
                    return Reply::Redraw;
                }
                // A drag up or down turns the page (a swipe is sideways).
                if let Some(y0) = self.down_y.take()
                    && y0.abs_diff(y) > 48
                    && press.is_none_or(|(x0, _)| x.abs_diff(x0) < 70)
                {
                    let turned = if y < y0 {
                        self.page_on()
                    } else {
                        self.page_back()
                    };
                    return if turned {
                        Reply::Redraw
                    } else {
                        Reply::Nothing
                    };
                }
                // The done ones shown, or folded away.
                if self.done_row.is_some_and(|r| r.contains_point(x, y)) {
                    self.show_done = !self.show_done;
                    self.first = 0;
                    self.n_pages = 0;
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
                // A touch on a task itself opens it: its actions, its time
                // and its steps (they opened only by a swipe to the left
                // or the D-pad's centre, which nothing on the screen said:
                // the owner on the X51v, 2026-11-15, after the TIME row
                // was built, "Still no way to assign a time inside
                // tasks!").
                if let Some((_, id)) = press {
                    self.revealed = Some(id);
                    return Reply::Redraw;
                }
                Reply::Nothing
            }
            (Event::Key(Key::SoftRight) | Event::Done, View::List(_)) if self.typing => {
                if self.find {
                    self.typing = false;
                } else {
                    self.add_typed(cx);
                }
                Reply::Redraw
            }
            (Event::Key(Key::SoftRight), View::List(_)) => {
                self.typing = true;
                Reply::Redraw
            }
            (Event::PenUp { x, y }, View::Home) => match self.hits.at(x, y) {
                Some(Hit::Lists) => {
                    self.view = View::Lists;
                    self.line_n = 0;
                    self.typing = false;
                    self.renaming = None;
                    self.ask_delete = None;
                    self.focus.clear();
                    Reply::Redraw
                }
                Some(Hit::Find) => {
                    self.find = true;
                    self.typing = true;
                    self.line_n = 0;
                    self.listed = None;
                    self.view = View::List(None);
                    Reply::Redraw
                }
                Some(Hit::List(which)) => {
                    self.view = View::List(which);
                    Reply::Redraw
                }
                Some(Hit::Tick(id)) => {
                    self.tick(cx, id);
                    Reply::Redraw
                }
                Some(Hit::Open(id)) => {
                    self.view = View::List(None);
                    self.revealed = Some(id);
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
                    // (The search keeps its word: only the keyboard goes.)
                    self.typing = false;
                    if !self.find {
                        self.line_n = 0;
                    }
                } else {
                    if self.find {
                        self.find = false;
                        self.line_n = 0;
                    }
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
                (self.first, self.n_pages) = (0, 0);
                Reply::Redraw
            }
            (Event::Delete, View::List(_)) if self.line_n > 0 => {
                self.line_n -= 1;
                (self.first, self.n_pages) = (0, 0);
                Reply::Redraw
            }
            _ => Reply::Nothing,
        }
    }

    fn keyboard(&self) -> Option<Field> {
        match self.view {
            View::Lists => self.typing,
            View::List(_) => self.typing || (self.line_n > 0 && !self.find),
            View::Home => false,
        }
        .then_some(Field::Line)
    }
}

impl Tasks {
    /// What a stylus can tap in a list, for the D-pad: a row's actions
    /// while they show; else the add line, the rows, their tick boxes and
    /// the done row.
    fn list_targets(&self) -> ([Rect; 28], usize) {
        let mut t = [Rect::new(0, 0, 0, 0); 28];
        let mut n = 0;
        let mut add = |r: Rect| {
            if n < t.len() {
                t[n] = r;
                n += 1;
            }
        };
        if self.revealed.is_some() {
            add(ADD_LINE);
            self.actions.iter().for_each(|&r| add(r));
            self.time_keys.iter().for_each(|&r| add(r));
            self.steps_at[..self.n_steps].iter().for_each(|r| add(r.0));
        } else {
            add(ADD_LINE);
            self.rows_at[..self.n_rows].iter().for_each(|r| add(r.0));
            self.boxes[..self.n_boxes].iter().for_each(|r| add(r.0));
            if let Some(r) = self.done_row {
                add(r);
            }
        }
        (t, n)
    }

    /// The time pad up on `minutes`, on the 12- or 24-hour clock (the
    /// previews: on the device a touch on a task's TIME row brings it).
    pub fn show_time_pad(&mut self, minutes: Option<u16>, clock12: bool) {
        self.pad = Some((0, rp_app::timepad::TimePad::clock(minutes, clock12)));
    }

    /// Task `id`'s time set to `minutes` from midnight (`None`: no time).
    /// A task with no day is today's from then.
    fn set_time(&mut self, cx: &Ctx, id: u32, minutes: Option<u16>) {
        let Some(t) = self.tasks.iter().flatten().find(|t| t.id == id).copied() else {
            return;
        };
        let Some(m) = minutes else {
            self.set_field(cx, id, tag::REMIND_AT, None);
            return;
        };
        let day = t.due.unwrap_or(cx.now.day);
        if t.due.is_none() {
            self.set_field(cx, id, tag::DUE, Some(&(day as u32).to_le_bytes()));
        }
        let at = (date::start_of(day) + u32::from(m)).to_le_bytes();
        self.set_field(cx, id, tag::REMIND_AT, Some(&at));
    }

    /// The list's next page, if there is one.
    fn page_on(&mut self) -> bool {
        if self.next >= self.total || self.next <= self.first {
            return false;
        }
        if self.n_pages < self.pages.len() {
            self.pages[self.n_pages] = self.first;
            self.n_pages += 1;
        }
        self.first = self.next;
        true
    }

    /// The page before, if there is one.
    fn page_back(&mut self) -> bool {
        if self.n_pages == 0 {
            if self.first == 0 {
                return false;
            }
            self.first = 0;
        } else {
            self.n_pages -= 1;
            self.first = self.pages[self.n_pages];
        }
        true
    }
}

#[cfg(test)]
mod tests;
