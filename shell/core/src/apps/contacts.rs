//! Contacts (docs/design/contacts-rule0.md): the list with its search
//! field and letter strip, a person's card, and the editor's values.
//!
//! Iterate (rule 0): Windows Mobile's quick find matched name prefixes and
//! its letter tabs took two taps; ours searches every field, phone digits
//! and initials as it is typed, with one letter per position on a strip.

use rp_draw::{Frame, Rect};
use rp_pim::store::{Kind, Rec, tag};

use super::{Act, DIM, FIELD, MARGIN, PITCH, ROW, TEXT, TOP, Typing, View, WIDE, text};
use crate::{Shell, W, panel};

/// The editor's fields, in its rows' order (slot `k` holds `TAGS[k]`).
const TAGS: [u8; 9] = [
    tag::FIRST,
    tag::LAST,
    tag::COMPANY,
    tag::MOBILE,
    tag::HOME,
    tag::WORK,
    tag::EMAIL,
    tag::ADDRESS,
    tag::NOTE,
];
/// The card's labels for them.
const NAMES: [&str; 9] = [
    "First", "Last", "Company", "Mobile", "Home", "Work", "E-mail", "Address", "Notes",
];
/// The letter strip's width and a letter's height.
const STRIP_W: u16 = 22;
const LETTER_H: u16 = 17;

/// What a contact sorts by: the last name, else the first, else the
/// company.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn key<'a>(r: &Rec<'a>) -> &'a [u8] {
    [tag::LAST, tag::FIRST, tag::COMPANY]
        .into_iter()
        .map(|t| r.text(t))
        .find(|t| !t.is_empty())
        .unwrap_or(&[])
}

#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn cmp_text(a: &[u8], b: &[u8]) -> core::cmp::Ordering {
    a.iter()
        .map(u8::to_ascii_lowercase)
        .cmp(b.iter().map(u8::to_ascii_lowercase))
}

/// The strip's letter a contact files under: 0-25, or 26 for anything
/// that doesn't start with a letter.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn letter(r: &Rec<'_>) -> u8 {
    match key(r).first().map(u8::to_ascii_lowercase) {
        Some(c @ b'a'..=b'z') => c - b'a',
        _ => 26,
    }
}

/// Whether `text` holds `q` anywhere, any case.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub(crate) fn holds(text: &[u8], q: &[u8]) -> bool {
    q.is_empty() || text.windows(q.len()).any(|w| w.eq_ignore_ascii_case(q))
}

/// Whether a contact matches the search: any text field, a phone's digits,
/// or its initials.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn matches(r: &Rec<'_>, q: &[u8]) -> bool {
    if q.is_empty() || TAGS.iter().any(|t| holds(r.text(*t), q)) {
        return true;
    }
    // "jd" finds John Doe.
    if let ([a, b], Some(f), Some(l)) = (q, r.text(tag::FIRST).first(), r.text(tag::LAST).first())
        && a.eq_ignore_ascii_case(f)
        && b.eq_ignore_ascii_case(l)
    {
        return true;
    }
    // Digits typed any way find a number written any way.
    let digits = |t: &[u8], k: usize| t.iter().filter(|b| b.is_ascii_digit()).nth(k).copied();
    let n = q.iter().filter(|b| b.is_ascii_digit()).count();
    n >= 3
        && n == q
            .iter()
            .filter(|b| !matches!(b, b' ' | b'-' | b'(' | b')' | b'+'))
            .count()
        && [tag::MOBILE, tag::HOME, tag::WORK].iter().any(|t| {
            let p = r.text(*t);
            let len = p.iter().filter(|b| b.is_ascii_digit()).count();
            (0..len.saturating_sub(n - 1))
                .any(|at| (0..n).all(|k| digits(p, at + k) == digits(q, k)))
        })
}

impl Shell {
    /// The list: the contacts matching the search, in filing order.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn contacts_build(&mut self) {
        let mut q = [0u8; super::SLOT];
        let n = self.apps.line().len();
        q[..n].copy_from_slice(self.apps.line());
        let n = self.apps.fill(Kind::Contact, 0, |r| matches(r, &q[..n]));
        self.apps.sort(0, n, &|a, b| {
            cmp_text(key(a), key(b))
                .then_with(|| cmp_text(a.text(tag::FIRST), b.text(tag::FIRST)))
                .then(a.id.cmp(&b.id))
        });
        // The letters that have someone, for the strip.
        let mut letters = 0u32;
        for k in 0..n {
            if let Some(r) = self.apps.rec(self.apps.entry(k)) {
                letters |= 1 << letter(&r);
            }
        }
        self.apps.letters = letters;
        self.apps.n = n;
        self.apps.height = n as i32 * i32::from(PITCH);
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn strip_shown(&self) -> bool {
        self.apps.typing.is_none() && self.apps.n > 0
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn contacts_targets(&mut self) {
        let v = View::Contacts;
        self.app_target(FIELD, Act::Field(super::LINE_SLOT as u8));
        let first = (self.apps.scroll / i32::from(PITCH)).max(0) as usize;
        for k in first..self.apps.n.min(first + 14) {
            let r = self.row_at(v, k as i32 * i32::from(PITCH), ROW, MARGIN, WIDE - STRIP_W);
            if let (true, Some(id)) = (r.h == ROW, self.apps.rec(self.apps.entry(k)).map(|r| r.id))
            {
                self.app_target(r, Act::Open(id));
            }
        }
        if self.strip_shown() {
            let top = self.list_top(v);
            for l in 0..26u8 {
                self.app_target(
                    Rect::new(
                        W - STRIP_W - 2,
                        top + u16::from(l) * LETTER_H,
                        STRIP_W,
                        LETTER_H,
                    ),
                    Act::Letter(l),
                );
            }
        }
    }

    /// The list scrolled to the first contact at or after letter `l`.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn contacts_letter(&mut self, l: u8) {
        let at = (0..self.apps.n)
            .find(|&k| {
                self.apps
                    .rec(self.apps.entry(k))
                    .is_some_and(|r| letter(&r) >= l)
            })
            .unwrap_or(self.apps.n.saturating_sub(1));
        self.apps.scroll = at as i32 * i32::from(PITCH);
        self.app_refresh();
        self.dirty(Rect::new(0, TOP, W, self.app_bottom() - TOP));
    }

    /// Left and right on the D-pad: the previous or next letter that has
    /// someone.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn contacts_jump(&mut self, by: i8) {
        let top = (self.apps.scroll / i32::from(PITCH)).max(0) as usize;
        let Some(now) = self.apps.rec(self.apps.entry(top)).map(|r| letter(&r)) else {
            return;
        };
        let has = |l: u8| self.apps.letters & (1 << l) != 0;
        let to = if by > 0 {
            (now + 1..27).find(|l| has(*l))
        } else {
            (0..now).rev().find(|l| has(*l))
        };
        if let Some(l) = to {
            self.contacts_letter(l);
        }
    }

    /// "First Last", else the company.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn contact_name(r: &Rec<'_>, out: &mut [u8; 2 * super::SLOT + 1]) -> usize {
        let (f, l) = (r.text(tag::FIRST), r.text(tag::LAST));
        let mut n = 0;
        for part in [f, l] {
            if !part.is_empty() {
                if n > 0 {
                    out[n] = b' ';
                    n += 1;
                }
                let k = part.len().min(super::SLOT);
                out[n..n + k].copy_from_slice(&part[..k]);
                n += k;
            }
        }
        if n == 0 {
            let c = r.text(tag::COMPANY);
            n = c.len().min(super::SLOT);
            out[..n].copy_from_slice(&c[..n]);
        }
        n
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn contacts_scene(&self, f: &mut Frame) {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let v = View::Contacts;
        self.line_field(f, "Search");
        let (top, bottom) = (self.list_top(v), self.app_bottom());
        if self.apps.n == 0 {
            if self.apps.line().is_empty() {
                self.empty_panel(f, top, "No contacts yet.", "New adds the first one.");
            } else {
                self.empty_panel(f, top, "No one matches.", "");
            }
            return;
        }
        f.clip(Some(Rect::new(0, top, W, bottom - top)));
        let first = (self.apps.scroll / i32::from(PITCH)).max(0) as usize;
        let mut name = [0u8; 2 * super::SLOT + 1];
        for k in first..self.apps.n.min(first + 14) {
            let y = i32::from(top) + k as i32 * i32::from(PITCH) - self.apps.scroll;
            let Some(rec) = self.apps.rec(self.apps.entry(k)) else {
                continue;
            };
            if y >= i32::from(bottom) {
                break;
            }
            let r = Rect::new(MARGIN, y.max(0) as u16, WIDE - STRIP_W, ROW);
            self.row_glass(f, r, Act::Open(rec.id));
            // The first number or the company on the right.
            let side = [tag::MOBILE, tag::HOME, tag::WORK, tag::COMPANY]
                .into_iter()
                .map(|t| rec.text(t))
                .find(|t| !t.is_empty())
                .unwrap_or(&[]);
            let n = Self::contact_name(&rec, &mut name);
            let (cut, _) = self.cut(ui, side, 150);
            let side_w = ui.width(&side[..cut]);
            self.text_cut(
                f,
                (r.x + 12, r.y + 25, r.w - 36 - side_w),
                ui,
                TEXT,
                &name[..n],
            );
            if side != &name[..n] {
                self.text_right(f, r.x + r.w - 12, r.y + 25, ui, DIM, &side[..cut]);
            }
            self.app_marks(f, Act::Open(rec.id));
        }
        f.clip(None);
        if self.strip_shown() {
            // The strip on its own glass, so its letters read anywhere.
            panel::draw(
                f,
                Rect::new(W - STRIP_W - 4, top - 2, STRIP_W + 4, 26 * LETTER_H + 4),
                self.light(),
            );
            for l in 0..26u8 {
                let r = Rect::new(
                    W - STRIP_W - 2,
                    top + u16::from(l) * LETTER_H,
                    STRIP_W,
                    LETTER_H,
                );
                let has = self.apps.letters & (1 << l) != 0;
                crate::keyboard::centre_in(f, r, label, if has { TEXT } else { DIM }, &[b'A' + l]);
                self.app_marks(f, Act::Letter(l));
            }
        }
    }

    /// A person's card: only the fields that are filled.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn card_scene(&self, f: &mut Frame) {
        let ui = self.fonts.ui;
        let Some(rec) = self
            .apps
            .open
            .and_then(|id| self.apps.store.as_ref()?.get(id))
        else {
            self.empty_panel(f, TOP + 8, "This contact is gone.", "");
            return;
        };
        let mut name = [0u8; 2 * super::SLOT + 1];
        let n = Self::contact_name(&rec, &mut name);
        let head = Rect::new(MARGIN, TOP + 4, WIDE, 52);
        panel::draw(f, head, self.light());
        self.text_cut(
            f,
            (head.x + 12, head.y + 33, head.w - 24),
            ui,
            TEXT,
            &name[..n],
        );
        let mut y = head.y + head.h + 6;
        for (k, t) in TAGS.iter().enumerate().skip(2) {
            let value = rec.text(*t);
            if value.is_empty() || (k == 2 && value == &name[..n]) {
                continue;
            }
            // Long text (an address, notes) takes a second line.
            let room = WIDE - 110 - 14;
            let (first, next) = text::line_from(value, 0, ui, room);
            let two = next.is_some();
            let r = Rect::new(MARGIN, y, WIDE, if two { 52 } else { ROW });
            if r.y + r.h > self.app_bottom() {
                break;
            }
            panel::draw(f, r, self.light());
            f.text(r.x + 12, r.y + 25, ui, DIM, NAMES[k].as_bytes());
            f.text(
                r.x + 110,
                r.y + 25,
                ui,
                TEXT,
                &value[first.start..first.end],
            );
            if let Some(at) = next {
                let (second, more) = text::line_from(value, at, ui, room);
                let rest = &value[second.start..second.end];
                self.text_cut(
                    f,
                    (
                        r.x + 110,
                        r.y + 45,
                        if more.is_some() { room - 14 } else { room },
                    ),
                    ui,
                    TEXT,
                    rest,
                );
                if more.is_some() {
                    f.text(r.x + 110 + ui.width(rest), r.y + 45, ui, TEXT, b"..");
                }
            }
            y += r.h + 4;
        }
    }

    /// A contact into the editor (`None`: a new one, the keyboard up at
    /// its first field).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn contact_edit(&mut self, id: Option<u32>) {
        self.apps.clear_form();
        if let Some(id) = id {
            let mut all = [[0u8; super::SLOT]; 9];
            let mut lens = [0usize; 9];
            if let Some(r) = self.apps.store.as_ref().and_then(|s| s.get(id)) {
                for (k, t) in TAGS.iter().enumerate() {
                    let v = r.text(*t);
                    lens[k] = v.len().min(super::SLOT);
                    all[k][..lens[k]].copy_from_slice(&v[..lens[k]]);
                }
            }
            for k in 0..9 {
                self.apps.set_slot(k, &all[k][..lens[k]]);
            }
        } else {
            self.apps.typing = Some(Typing::Slot(0));
        }
        self.apps.open = id;
        self.open_app(View::ContactEdit);
    }

    /// The editor's Done: the card kept (one with nothing in it isn't).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn contact_done(&mut self) {
        let mut all = [[0u8; super::SLOT]; 9];
        let mut lens = [0usize; 9];
        for k in 0..9 {
            let s = self.apps.slot(k);
            lens[k] = s.len();
            all[k][..lens[k]].copy_from_slice(s);
        }
        let id = self.apps.open;
        if lens.iter().all(|n| *n == 0) {
            self.apps.typing = None;
            return self.open_app(View::Contacts);
        }
        let Some(store) = self.apps.store.as_mut() else {
            return;
        };
        let mut fields = [(0u8, &[][..]); 9];
        for k in 0..9 {
            fields[k] = (TAGS[k], &all[k][..lens[k]]);
        }
        match store.put(Kind::Contact, id, &fields) {
            Ok(id) => {
                self.apps.open = Some(id);
                self.apps.typing = None;
                self.open_app(View::Card);
            }
            Err(_) => self.show_notice("The organiser is full"),
        }
    }
}
