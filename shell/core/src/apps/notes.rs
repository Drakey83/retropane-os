//! Notes (docs/design/notes-rule0.md): the list with its search, and a
//! note being written: wrapped text with a caret above the keyboard.
//!
//! Iterate (rule 0): Windows Mobile's notes were files called "Note1"
//! that had to be saved; ours take their first line as the title, keep
//! themselves on every way out, and are searched through all their text.

use rp_draw::{Cmd, Frame, Rect};
use rp_pim::date;
use rp_pim::store::{Kind, TEXT_MAX, tag};
use rp_weather::Line;

use super::contacts::holds;
use super::{Act, DIM, FIELD, FOCUS, MARGIN, TEXT, TOP, Typing, View, WIDE, num, text};
use crate::{Key, Shell, W, panel};

/// A row of the list: two lines, and the gap under it.
const NOTE_ROW: u16 = 52;
const NOTE_PITCH: u16 = 56;
/// The text's box in the editor, its padding and a line's height.
const BOX: Rect = Rect::new(MARGIN, TOP + 2, WIDE, 282);
const PAD: u16 = 12;
const LINE_H: u16 = 24;
/// Lines the box shows.
const LINES: usize = 11;
/// The width text wraps at.
const WRAP: u16 = WIDE - 2 * PAD;

impl Shell {
    /// The list: the notes holding the search text, newest first.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn notes_build(&mut self) {
        let mut q = [0u8; super::SLOT];
        let n = self.apps.line().len();
        q[..n].copy_from_slice(self.apps.line());
        let n = self
            .apps
            .fill(Kind::Note, 0, |r| holds(r.text(tag::TEXT), &q[..n]));
        self.apps.sort(0, n, &|a, b| {
            (b.num(tag::CHANGED), b.id).cmp(&(a.num(tag::CHANGED), a.id))
        });
        self.apps.n = n;
        self.apps.height = n as i32 * i32::from(NOTE_PITCH);
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn notes_targets(&mut self) {
        let v = View::Notes;
        self.app_target(FIELD, Act::Field(super::LINE_SLOT as u8));
        let first = (self.apps.scroll / i32::from(NOTE_PITCH)).max(0) as usize;
        for k in first..self.apps.n.min(first + 10) {
            let r = self.row_at(v, k as i32 * i32::from(NOTE_PITCH), NOTE_ROW, MARGIN, WIDE);
            if let (true, Some(id)) = (
                r.h == NOTE_ROW,
                self.apps.rec(self.apps.entry(k)).map(|r| r.id),
            ) {
                self.app_target(r, Act::Open(id));
            }
        }
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn notes_scene(&self, f: &mut Frame) {
        let ui = self.fonts.ui;
        let v = View::Notes;
        self.line_field(f, "Search notes");
        let (top, bottom) = (self.list_top(v), self.app_bottom());
        if self.apps.n == 0 {
            if self.apps.line().is_empty() {
                self.empty_panel(f, top, "No notes yet.", "New starts one.");
            } else {
                self.empty_panel(f, top, "No note has that.", "");
            }
            return;
        }
        f.clip(Some(Rect::new(0, top, W, bottom - top)));
        let first = (self.apps.scroll / i32::from(NOTE_PITCH)).max(0) as usize;
        for k in first..self.apps.n.min(first + 10) {
            let y = i32::from(top) + k as i32 * i32::from(NOTE_PITCH) - self.apps.scroll;
            let Some(rec) = self.apps.rec(self.apps.entry(k)) else {
                continue;
            };
            if y >= i32::from(bottom) {
                break;
            }
            let r = Rect::new(MARGIN, y.max(0) as u16, WIDE, NOTE_ROW);
            self.row_glass(f, r, Act::Open(rec.id));
            // The first line is the title; under it the day and what follows.
            let body = rec.text(tag::TEXT);
            let mut parts = body.split(|b| *b == b'\n').filter(|l| !l.is_empty());
            let title = parts.next().unwrap_or(&[]);
            let rest = parts.next().unwrap_or(&[]);
            self.text_cut(f, (r.x + 12, r.y + 22, r.w - 24), ui, TEXT, title);
            let day = date::day_of(rec.num(tag::CHANGED).unwrap_or(0));
            let d = date::date_of(day);
            let when = Line::new()
                .num(i32::from(d.day))
                .push(b" ")
                .push(date::MONTHS[usize::from(d.month - 1)].as_bytes())
                .push(if rest.is_empty() { b"" } else { b" - " });
            f.text(r.x + 12, r.y + 44, ui, DIM, when.as_bytes());
            let x = r.x + 12 + ui.width(when.as_bytes());
            self.text_cut(
                f,
                (x, r.y + 44, (r.x + r.w - 12).saturating_sub(x)),
                ui,
                DIM,
                rest,
            );
            self.app_marks(f, Act::Open(rec.id));
        }
    }

    // ---- A note being written ---------------------------------------------

    /// Opens a note (`None`: a new one) with the keyboard up and the
    /// caret at its end.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn note_open(&mut self, id: Option<u32>) {
        self.apps.clear_form();
        let mut n = 0;
        {
            let apps = &mut self.apps;
            if let (Some(id), Some(store), Some(scratch)) =
                (id, apps.store.as_ref(), apps.scratch.as_deref_mut())
                && let Some(r) = store.get(id)
            {
                let t = r.text(tag::TEXT);
                n = t.len().min(TEXT_MAX);
                scratch[super::TABLE..super::TABLE + n].copy_from_slice(&t[..n]);
            }
        }
        self.apps.text_n = n;
        self.apps.caret = n;
        self.apps.open = id;
        self.apps.len[super::LINE_SLOT] = 0;
        self.apps.typing = Some(Typing::Note);
        self.open_app(View::Note);
        self.note_follow();
    }

    /// Keeps the note as it stands: every way out of it comes through
    /// here. An empty one isn't kept (and one emptied is removed).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn note_keep(&mut self) {
        let now = num(self.minute_now());
        let apps = &mut self.apps;
        let (Some(store), Some(scratch)) = (apps.store.as_mut(), apps.scratch.as_deref()) else {
            return;
        };
        let text = &scratch[super::TABLE..super::TABLE + apps.text_n];
        let blank = text.iter().all(|b| matches!(b, b' ' | b'\n'));
        match (apps.open, blank) {
            (Some(id), true) => {
                let _ = store.remove(id);
            }
            (None, true) => {}
            (id, false) => {
                // Unchanged: nothing is written (its date stays).
                if id
                    .and_then(|id| store.get(id))
                    .is_some_and(|r| r.text(tag::TEXT) == text)
                {
                    return;
                }
                if let Ok(id) =
                    store.put(Kind::Note, id, &[(tag::TEXT, text), (tag::CHANGED, &now)])
                {
                    apps.open = Some(id);
                }
            }
        }
    }

    /// The left softkey: the first tap asks, the second removes the note.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn note_delete(&mut self) {
        if !self.apps.confirm {
            self.apps.confirm = true;
            self.dirty(Rect::new(0, crate::H - 40, W, 40));
            return;
        }
        if let (Some(id), Some(s)) = (self.apps.open, self.apps.store.as_mut()) {
            let _ = s.remove(id);
        }
        self.apps.open = None;
        self.apps.text_n = 0;
        self.apps.typing = None;
        self.open_app(View::Notes);
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn note_targets(&mut self) {
        self.app_target(BOX, Act::Text);
    }

    /// The first line shown follows the caret.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn note_follow(&mut self) {
        let (line, _) = text::caret_at(self.apps.note(), self.apps.caret, self.fonts.ui, WRAP);
        if line < self.apps.text_top {
            self.apps.text_top = line;
        } else if line >= self.apps.text_top + LINES {
            self.apps.text_top = line + 1 - LINES;
        }
        self.dirty(BOX);
    }

    /// A character typed at the caret.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn note_type(&mut self, c: u8) {
        let (n, at) = (self.apps.text_n, self.apps.caret.min(self.apps.text_n));
        if n >= TEXT_MAX {
            return self.show_notice("This note is full");
        }
        let buf = self.apps.note_buf();
        if buf.len() < TEXT_MAX {
            return;
        }
        buf.copy_within(at..n, at + 1);
        buf[at] = c;
        self.apps.text_n = n + 1;
        self.apps.caret = at + 1;
        self.apps.confirm = false;
        self.note_follow();
    }

    /// The character before the caret removed.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn note_delete_back(&mut self) {
        let (n, at) = (self.apps.text_n, self.apps.caret.min(self.apps.text_n));
        if at == 0 {
            return;
        }
        let buf = self.apps.note_buf();
        if buf.len() < TEXT_MAX {
            return;
        }
        buf.copy_within(at..n, at - 1);
        self.apps.text_n = n - 1;
        self.apps.caret = at - 1;
        self.note_follow();
    }

    /// The D-pad moves the caret: a character, or a line.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn note_move(&mut self, k: Key) {
        let ui = self.fonts.ui;
        let (n, at) = (self.apps.text_n, self.apps.caret);
        self.apps.caret = match k {
            Key::Left => at.saturating_sub(1),
            Key::Right => (at + 1).min(n),
            _ => {
                let (line, x) = text::caret_at(self.apps.note(), at, ui, WRAP);
                let to = if k == Key::Up {
                    match line.checked_sub(1) {
                        Some(l) => l,
                        None => return,
                    }
                } else {
                    line + 1
                };
                if to >= text::count(self.apps.note(), ui, WRAP) {
                    n
                } else {
                    text::offset_at(self.apps.note(), to, x, ui, WRAP)
                }
            }
        };
        self.note_follow();
    }

    /// A tap in the text: the caret goes there.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn note_tap(&mut self, x: u16, y: u16) {
        let line = self.apps.text_top + usize::from(y.saturating_sub(BOX.y + PAD - 4) / LINE_H);
        self.apps.caret = text::offset_at(
            self.apps.note(),
            line,
            x.saturating_sub(BOX.x + PAD),
            self.fonts.ui,
            WRAP,
        );
        if self.apps.typing.is_none() {
            self.app_typing(Some(Typing::Note));
        }
        self.note_follow();
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn note_scene(&self, f: &mut Frame) {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        panel::draw(f, BOX, self.light());
        let body = self.apps.note();
        let top = self.apps.text_top;
        let caret = text::caret_at(body, self.apps.caret, ui, WRAP);
        if body.is_empty() {
            f.text(BOX.x + PAD, BOX.y + PAD + 18, ui, DIM, b"Write a note");
        }
        text::lines(body, ui, WRAP, |n, l| {
            if n >= top && n < top + LINES {
                let y = BOX.y + PAD + (n - top) as u16 * LINE_H;
                f.text(BOX.x + PAD, y + 18, ui, TEXT, &body[l.start..l.end]);
            }
        });
        if caret.0 >= top && caret.0 < top + LINES {
            f.push(Cmd::Fill {
                r: Rect::new(
                    BOX.x + PAD + caret.1,
                    BOX.y + PAD + (caret.0 - top) as u16 * LINE_H,
                    2,
                    22,
                ),
                color: FOCUS,
            });
        }
        // The room left, once it is short.
        let left = TEXT_MAX - body.len();
        if left < 200 {
            let words = Line::new().num(left as i32).push(b" LEFT");
            self.text_right(
                f,
                BOX.x + BOX.w - PAD,
                BOX.y + BOX.h - 6,
                label,
                DIM,
                words.as_bytes(),
            );
        }
        self.app_marks(f, Act::Text);
    }
}
