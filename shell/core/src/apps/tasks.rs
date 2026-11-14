//! Tasks (docs/design/tasks-rule0.md): the list in order of urgency with
//! its add field, the tick box, and the editor's values.
//!
//! Iterate (rule 0): Windows Mobile made the user pick a sort from a menu
//! and tick a 12 px box; ours has one order (late, today, later, undated;
//! done at the bottom), a 30 px box, and the quick-add line shared with
//! Calendar ("Pay rent fri !").

use rp_draw::{Cmd, Frame, Rect};
use rp_pim::date;
use rp_pim::quick;
use rp_pim::store::{Kind, Rec, tag};
use rp_weather::Line;

use super::form::{NO_DAY, due_or_none};
use super::{Act, DIM, DONE, FIELD, LATE, MARGIN, PITCH, ROW, TEXT, View, WIDE, num};
use crate::{GLASS_EDGE, Shell, W};

/// The tick box's part of a row.
const BOX_W: u16 = 44;

#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn done(r: &Rec<'_>) -> bool {
    r.num(tag::DONE).is_some()
}

#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn priority(r: &Rec<'_>) -> u32 {
    r.num(tag::PRIORITY).unwrap_or(1)
}

/// Open before done; then by due day (the undated last); then the higher
/// priority; then as added.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn by_urgency(a: &Rec<'_>, b: &Rec<'_>) -> core::cmp::Ordering {
    let k = |r: &Rec<'_>| {
        (
            done(r),
            r.num(tag::DUE).unwrap_or(u32::MAX),
            2 - priority(r).min(2),
            r.id,
        )
    };
    k(a).cmp(&k(b))
}

impl Shell {
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn tasks_build(&mut self) {
        let hide = self.apps.hide_done;
        let n = self.apps.fill(Kind::Task, 0, |r| !(hide && done(r)));
        self.apps.sort(0, n, &by_urgency);
        self.apps.n = n;
        self.apps.height = n as i32 * i32::from(PITCH);
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn tasks_targets(&mut self) {
        let v = View::Tasks;
        self.app_target(FIELD, Act::Field(super::LINE_SLOT as u8));
        let first = (self.apps.scroll / i32::from(PITCH)).max(0) as usize;
        for k in first..self.apps.n.min(first + 14) {
            let r = self.row_at(v, k as i32 * i32::from(PITCH), ROW, MARGIN, WIDE);
            if let (true, Some(id)) = (r.h == ROW, self.apps.rec(self.apps.entry(k)).map(|r| r.id))
            {
                self.app_target(Rect::new(r.x, r.y, BOX_W, r.h), Act::Tick(id));
                self.app_target(Rect::new(r.x + BOX_W, r.y, r.w - BOX_W, r.h), Act::Open(id));
            }
        }
    }

    /// The due day as a person says it, and whether it is late.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn due_words(&self, due: i32) -> (Line, bool) {
        let today = self.today();
        match due - today {
            d if d < -1 => (Line::new().push(b"Late ").num(-d).push(b" days"), true),
            -1 => (Line::new().push(b"Late 1 day"), true),
            0 => (Line::new().push(b"Today"), false),
            1 => (Line::new().push(b"Tomorrow"), false),
            2..=6 => (
                Line::new().push(date::WEEKDAYS[usize::from(date::weekday(due))].as_bytes()),
                false,
            ),
            _ => {
                let d = date::date_of(due);
                (
                    Line::new()
                        .num(i32::from(d.day))
                        .push(b" ")
                        .push(date::MONTHS[usize::from(d.month - 1)].as_bytes()),
                    false,
                )
            }
        }
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn tasks_scene(&self, f: &mut Frame) {
        let ui = self.fonts.ui;
        let v = View::Tasks;
        self.line_field(f, "Add a task: Pay rent fri !");
        let (top, bottom) = (self.list_top(v), self.app_bottom());
        if self.apps.n == 0 {
            self.empty_panel(f, top, "Nothing to do.", "Type a task above.");
            return;
        }
        f.clip(Some(Rect::new(0, top, W, bottom - top)));
        let first = (self.apps.scroll / i32::from(PITCH)).max(0) as usize;
        for k in first..self.apps.n.min(first + 14) {
            let y = i32::from(top) + k as i32 * i32::from(PITCH) - self.apps.scroll;
            let Some(rec) = self.apps.rec(self.apps.entry(k)) else {
                continue;
            };
            if y >= i32::from(bottom) {
                break;
            }
            let r = Rect::new(MARGIN, y.max(0) as u16, WIDE, ROW);
            self.row_glass(f, r, Act::Open(rec.id));
            let is_done = done(&rec);
            // The box: an outline, filled green when done.
            let b = Rect::new(r.x + 12, r.y + 8, 20, 20);
            if self.app_pressed(Act::Tick(rec.id)) {
                f.push(Cmd::Fill {
                    r: Rect::new(r.x + 2, r.y + 2, BOX_W - 4, r.h - 4),
                    color: crate::PRESSED_UNDER,
                });
            }
            f.push(Cmd::Outline {
                r: b,
                color: if is_done { DONE } else { TEXT },
            });
            if is_done {
                f.push(Cmd::Fill {
                    r: Rect::new(b.x + 4, b.y + 4, 12, 12),
                    color: DONE,
                });
            }
            // The due words on the right.
            let mut right = r.x + r.w - 12;
            if let (Some(due), false) = (rec.num(tag::DUE), is_done) {
                let (words, late) = self.due_words(due as i32);
                self.text_right(
                    f,
                    right,
                    r.y + 25,
                    ui,
                    if late { LATE } else { DIM },
                    words.as_bytes(),
                );
                right -= ui.width(words.as_bytes()) + 12;
            }
            let mut x = r.x + BOX_W + 4;
            let p = priority(&rec);
            if p >= 2 && !is_done {
                f.text(x, r.y + 25, ui, LATE, b"!");
                x += ui.width(b"! ");
            }
            self.text_cut(
                f,
                (x, r.y + 25, right.saturating_sub(x)),
                ui,
                if is_done || p == 0 { DIM } else { TEXT },
                rec.text(tag::TITLE),
            );
            if is_done {
                // Struck through: done reads at a glance, not by colour alone.
                let w = ui.width(rec.text(tag::TITLE)).min(right.saturating_sub(x));
                f.push(Cmd::Fill {
                    r: Rect::new(x, r.y + 19, w, 1),
                    color: GLASS_EDGE,
                });
            }
            self.app_marks(f, Act::Tick(rec.id));
            self.app_marks(f, Act::Open(rec.id));
        }
    }

    /// A task written to the store: every field given.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn task_put(
        &mut self,
        id: Option<u32>,
        (title, note): (&[u8], &[u8]),
        due: i32,
        priority: u32,
        done_at: Option<u32>,
    ) -> bool {
        let (due_b, pri_b, done_b) = (
            num(due.max(0) as u32),
            num(priority),
            num(done_at.unwrap_or(0)),
        );
        let Some(store) = self.apps.store.as_mut() else {
            return false;
        };
        store
            .put(
                Kind::Task,
                id,
                &[
                    (tag::TITLE, title),
                    (tag::NOTE, note),
                    (tag::DUE, if due == NO_DAY { &[] } else { &due_b }),
                    (tag::PRIORITY, &pri_b),
                    (tag::DONE, if done_at.is_some() { &done_b } else { &[] }),
                ],
            )
            .is_ok()
    }

    /// The add field's line as a new task; the field stays for the next.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn task_add(&mut self) {
        let mut title = [0u8; quick::TITLE_MAX];
        let (q, n) = quick::parse(
            self.apps.line(),
            self.today(),
            self.month_first(),
            &mut title,
        );
        if n == 0 {
            return self.app_typing(None);
        }
        let ok = self.task_put(
            None,
            (&title[..n], &[]),
            q.day.unwrap_or(NO_DAY),
            u32::from(q.priority.unwrap_or(1)),
            None,
        );
        if !ok {
            return self.show_notice("The organiser is full");
        }
        self.apps.len[super::LINE_SLOT] = 0;
        self.app_changed();
    }

    /// The box tapped: done now, or open again.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn task_tick(&mut self, id: u32) {
        let now = self.minute_now();
        let mut text = [[0u8; super::SLOT]; 2];
        let mut lens = [0usize; 2];
        let Some((due, pri, was)) = self.apps.store.as_ref().and_then(|s| s.get(id)).map(|r| {
            for (k, t) in [tag::TITLE, tag::NOTE].into_iter().enumerate() {
                let v = r.text(t);
                lens[k] = v.len().min(super::SLOT);
                text[k][..lens[k]].copy_from_slice(&v[..lens[k]]);
            }
            (due_or_none(r.num(tag::DUE)), priority(&r), done(&r))
        }) else {
            return;
        };
        self.task_put(
            Some(id),
            (&text[0][..lens[0]], &text[1][..lens[1]]),
            due,
            pri,
            (!was).then_some(now),
        );
        self.app_changed();
    }

    /// A task into the editor.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn task_edit(&mut self, id: u32) {
        self.apps.clear_form();
        let mut text = [[0u8; super::SLOT]; 2];
        let mut lens = [0usize; 2];
        let Some(nums) = self.apps.store.as_ref().and_then(|s| s.get(id)).map(|r| {
            for (k, t) in [tag::TITLE, tag::NOTE].into_iter().enumerate() {
                let v = r.text(t);
                lens[k] = v.len().min(super::SLOT);
                text[k][..lens[k]].copy_from_slice(&v[..lens[k]]);
            }
            [due_or_none(r.num(tag::DUE)), priority(&r) as i32, 0, 0]
        }) else {
            return;
        };
        for k in 0..2 {
            self.apps.set_slot(k, &text[k][..lens[k]]);
        }
        self.apps.nums = nums;
        self.apps.open = Some(id);
        self.open_app(View::TaskEdit);
    }

    /// The editor's Done.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn task_done(&mut self) {
        let mut text = [[0u8; super::SLOT]; 2];
        let mut lens = [0usize; 2];
        for k in 0..2 {
            let s = self.apps.slot(k);
            lens[k] = s.len();
            text[k][..lens[k]].copy_from_slice(s);
        }
        let id = self.apps.open;
        // Done stays as it was: the editor doesn't change it.
        let was = id
            .and_then(|id| self.apps.store.as_ref()?.get(id))
            .and_then(|r| r.num(tag::DONE));
        let [due, pri, ..] = self.apps.nums;
        let title: &[u8] = if lens[0] == 0 {
            b"Task"
        } else {
            &text[0][..lens[0]]
        };
        if !self.task_put(
            id,
            (title, &text[1][..lens[1]]),
            due,
            pri.clamp(0, 2) as u32,
            was,
        ) {
            return self.show_notice("The organiser is full");
        }
        self.apps.typing = None;
        self.apps.open = None;
        self.open_app(View::Tasks);
    }
}
