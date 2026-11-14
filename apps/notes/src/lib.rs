//! Notes, version 2 ([notes-rule0.md](../../../docs/design/notes-rule0.md)):
//! an app program ([`rp_app::Program`]).
//!
//! Iterate, don't imitate (rule 0): Windows Mobile 5 listed note names and
//! drew ink as jagged one-pixel lines; Keep shows a grid of coloured cards
//! and Apple mixes ink, text and checklists in one note. Ours does both on
//! the X51v: cards with a miniature of the ink, and notes made of blocks.
//! Ink is smoothed (the touch points fitted by a Catmull-Rom curve and
//! laid down as anti-aliased round stamps, width from speed since the
//! TSC2046 has no pressure), painted once into a picture the 2700G only
//! places; a stroke added paints only itself.

#![no_std]
#![forbid(unsafe_code)]

use rp_app::kit::{self, DIM, TEXT, tint};
use rp_app::{
    Argb, Ctx, Event, Field, Frame, Key, MARGIN, Pic, Program, Rect, Reply, Text, W, WIDE,
};
use rp_pim::store::{Kind, tag};

/// The ink panel's picture and a card's miniature (the picture store's
/// classes for Notes).
pub const PIC_CLASSES: [(u16, u16, u8); 2] = [(448, 196, 1), (212, 84, 4)];

/// One block of a note, as a note is written (the previews' samples, and
/// the store's text and ink once parsed).
#[derive(Clone, Copy, Debug)]
pub enum Block {
    /// A paragraph.
    Text(&'static [u8]),
    /// A checklist item, ticked or not.
    Check(&'static [u8], bool),
    /// Handwriting: strokes as point lists in a 448 x 196 box.
    Ink(&'static [&'static [(i16, i16)]]),
}

/// One note, as written (the previews' samples).
#[derive(Clone, Copy, Debug)]
pub struct Note {
    /// Its title.
    pub title: &'static [u8],
    /// Its blocks.
    pub blocks: &'static [Block],
    /// Its colour (an identity colour).
    pub colour: Argb,
    /// Pinned.
    pub pinned: bool,
}

/// The views.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    /// The grid of cards.
    Home,
    /// One note (its place in the store's order).
    Note(u8),
}

/// The line in a note's text that stands for its ink panel.
const INK_LINE: u8 = 0x01;
/// Points a note's ink holds, and strokes.
const POINTS: usize = 900;
const STROKES: usize = 48;

/// The pen's five colours (`STYLE` bits 0-2 choose), dark first.
pub const PEN: [Argb; 5] = [
    0xFF20_2A38,
    0xFF1C_3A6E,
    0xFFC0_3030,
    0xFF2E_8B57,
    0xFFFF_9F2E,
];
/// A stroke's style bit: the marker (wide, see-through yellow).
pub const MARKER: u8 = 0x08;

/// Packs `strokes` into `out` (the store's `INK` field): per stroke its
/// point count, its style (the pen colour 0-4, [`MARKER`]), the first
/// point as two little-endian i16, then each next one as a byte pair of
/// signed steps (a longer step is split, so nothing is lost). The length,
/// or `None` when it doesn't fit. `styles` gives each stroke's style (0
/// when shorter).
pub fn encode_ink(strokes: &[&[(i16, i16)]], out: &mut [u8]) -> Option<usize> {
    encode_styled(strokes, &[], out)
}

/// [`encode_ink`] with each stroke's style.
pub fn encode_styled(strokes: &[&[(i16, i16)]], styles: &[u8], out: &mut [u8]) -> Option<usize> {
    let mut n = 0;
    let mut put = |b: u8, n: &mut usize| -> Option<()> {
        *out.get_mut(*n)? = b;
        *n += 1;
        Some(())
    };
    for (si, st) in strokes.iter().enumerate().filter(|(_, s)| !s.is_empty()) {
        // The points with long steps split into steps of at most 127.
        let mut pts = [(0i16, 0i16); 255];
        let mut k = 0;
        let mut last = st[0];
        pts[0] = last;
        k += 1;
        for &p in &st[1..] {
            while k < pts.len() && ((p.0 - last.0).abs() > 127 || (p.1 - last.1).abs() > 127) {
                last = (
                    last.0 + (p.0 - last.0).clamp(-127, 127),
                    last.1 + (p.1 - last.1).clamp(-127, 127),
                );
                pts[k] = last;
                k += 1;
            }
            if k < pts.len() {
                pts[k] = p;
                k += 1;
                last = p;
            }
        }
        put(k as u8, &mut n)?;
        put(styles.get(si).copied().unwrap_or(0), &mut n)?;
        for b in pts[0].0.to_le_bytes().iter().chain(&pts[0].1.to_le_bytes()) {
            put(*b, &mut n)?;
        }
        for w in pts[..k].windows(2) {
            put((w[1].0 - w[0].0) as i8 as u8, &mut n)?;
            put((w[1].1 - w[0].1) as i8 as u8, &mut n)?;
        }
    }
    Some(n)
}

/// Unpacks the store's `INK` field into `pts`, each stroke's end in
/// `ends` and its style in `styles`: the strokes read (a damaged field
/// stops where it breaks).
pub fn decode_ink(
    ink: &[u8],
    pts: &mut [(i16, i16)],
    ends: &mut [u16],
    styles: &mut [u8],
) -> usize {
    let (mut at, mut np, mut ns) = (0usize, 0usize, 0usize);
    while at + 6 <= ink.len() && ns < ends.len() {
        let count = usize::from(ink[at]);
        if let Some(st) = styles.get_mut(ns) {
            *st = ink[at + 1];
        }
        let x = i16::from_le_bytes([ink[at + 2], ink[at + 3]]);
        let y = i16::from_le_bytes([ink[at + 4], ink[at + 5]]);
        at += 6;
        if count == 0 || np + count > pts.len() || at + 2 * (count - 1) > ink.len() {
            break;
        }
        let (mut px, mut py) = (x, y);
        pts[np] = (px, py);
        for k in 1..count {
            px = px.saturating_add(i16::from(ink[at] as i8));
            py = py.saturating_add(i16::from(ink[at + 1] as i8));
            at += 2;
            pts[np + k] = (px, py);
        }
        np += count;
        ends[ns] = np as u16;
        ns += 1;
    }
    ns
}

/// A note's text and ink for the store from its blocks (the previews put
/// their samples in through it): the text's lines (`[ ] ` and `[x] ` mark
/// checklist items, a line of byte 1 the ink's place) and the packed ink.
/// The lengths.
pub fn fields_of(blocks: &[Block], text: &mut [u8], ink: &mut [u8]) -> (usize, usize) {
    let (mut t, mut i) = (0usize, 0usize);
    let mut push = |b: &[u8], t: &mut usize| {
        for &c in b {
            if *t < text.len() {
                text[*t] = c;
                *t += 1;
            }
        }
    };
    for (k, b) in blocks.iter().enumerate() {
        if k > 0 {
            push(b"\n", &mut t);
        }
        match b {
            Block::Text(s) => push(s, &mut t),
            Block::Check(s, done) => {
                push(if *done { b"[x] " } else { b"[ ] " }, &mut t);
                push(s, &mut t);
            }
            Block::Ink(st) => {
                push(&[INK_LINE], &mut t);
                i = encode_ink(st, ink).unwrap_or(0);
            }
        }
    }
    (t, i)
}

/// A line of a note as a view shows it.
#[derive(Clone, Copy, Debug)]
enum Line {
    Text(Text<48>),
    Check(Text<48>, bool),
    Ink,
}

/// A note as the views show it (copied from the store).
#[derive(Clone, Copy, Debug)]
struct Row {
    id: u32,
    title: Text<32>,
    colour: Argb,
    pinned: bool,
    paper: u8,
    /// Whether the note holds the search typed (title or text).
    found: bool,
    lines: [Line; 12],
    n: usize,
}

impl Row {
    fn of(r: &rp_pim::store::Rec<'_>) -> Self {
        let mut row = Self {
            id: r.id,
            title: Text::of(r.text(tag::TITLE)),
            colour: kit::IDENTITY[(r.num(tag::COLOUR).unwrap_or(1) % 8) as usize],
            pinned: r.num(tag::PINNED) == Some(1),
            paper: r.num(tag::PAPER).unwrap_or(0).min(2) as u8,
            found: true,
            lines: [Line::Ink; 12],
            n: 0,
        };
        for l in r.text(tag::TEXT).split(|&c| c == b'\n') {
            if row.n == row.lines.len() {
                break;
            }
            row.lines[row.n] = if l == [INK_LINE] {
                Line::Ink
            } else if let Some(t) = l.strip_prefix(b"[x] ") {
                Line::Check(Text::of(t), true)
            } else if let Some(t) = l.strip_prefix(b"[ ] ") {
                Line::Check(Text::of(t), false)
            } else {
                Line::Text(Text::of(l))
            };
            row.n += 1;
        }
        row
    }

    fn has_ink(&self) -> bool {
        self.lines[..self.n].iter().any(|l| matches!(l, Line::Ink))
    }
}

/// A tool of the note's strip.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Pen,
    Marker,
    Erase,
    Text,
    List,
}

/// The home's search field and its grid/list chips.
const SEARCH: Rect = Rect::new(MARGIN, 36 + kit::TITLE_BAND + 4, WIDE - 140, 36);
const LAYOUT: Rect = Rect::new(W - MARGIN - 132, 36 + kit::TITLE_BAND + 7, 132, 30);

/// The note's tool strip, its tools in order, and the pen colours' row.
const STRIP: Rect = Rect::new(8, 548, W - 16, 48);
const TOOLS: [&[u8]; 6] = [b"PEN", b"MARKER", b"ERASE", b"TEXT", b"LIST", b"UNDO"];
const PAPER: Rect = Rect::new(8, 40, W - 16, 504);
/// The ink panel's size (the store's coordinates).
const INK_W: u16 = 448;
const INK_H: u16 = 196;
/// Points a stroke being written keeps.
const LIVE: usize = 200;

/// The Notes program.
pub struct Notes {
    rows: [Option<Row>; 24],
    /// The note's tool and pen colour; the stroke being written; where the
    /// ink panel lies on screen; the checklist rows on screen; typing.
    tool: Tool,
    pen: u8,
    live: [(i16, i16); LIVE],
    live_n: usize,
    ink_at: Option<(u16, u16)>,
    checks: [(Rect, u8); 12],
    n_checks: usize,
    typing: bool,
    /// A new note to start on the first frame (the Note button).
    start_new: bool,
    /// The home's search (being typed), grid or list; a note's menu open,
    /// and Delete asked once.
    query: [u8; 32],
    query_n: usize,
    searching: bool,
    as_list: bool,
    menu: bool,
    confirm_delete: bool,
    /// The home grid's cards on screen and the notes they open.
    cards: [(Rect, u8); 12],
    n_cards: usize,
    /// The store's change count the rows were copied at.
    seen: Option<u32>,
    view: View,
    /// The open note's ink and the cards' miniatures, painted once.
    ink: Option<Pic>,
    minis: [Option<Pic>; 4],
    ring: Option<Pic>,
    full: Option<Pic>,
    painted: Option<(View, u32)>,
}

/// Paints `strokes` (in a 448 x 196 box) into `px` (`w` x `h`, scaled to
/// fit) in `c`: each stroke a Catmull-Rom curve through its points, laid
/// as round anti-aliased stamps 2.5 px across at full size.
pub fn paint_ink(px: &mut [Argb], w: u16, h: u16, strokes: &[&[(i16, i16)]], c: Argb) {
    paint_styled(px, w, h, strokes, &[], Some(c));
}

/// [`paint_ink`] with each stroke's style (its pen colour, the marker's
/// width and see-through yellow); `only` paints every stroke in one colour
/// (the cards' miniatures).
pub fn paint_styled(
    px: &mut [Argb],
    w: u16,
    h: u16,
    strokes: &[&[(i16, i16)]],
    styles: &[u8],
    only: Option<Argb>,
) {
    px.iter_mut().for_each(|p| *p = 0);
    let sx = i32::from(w) * 256 / 448;
    let sy = i32::from(h) * 256 / 196;
    let s = sx.min(sy);
    // Stamp radius in 1/16 px (at least a pixel's worth).
    let base_rad = (20 * s / 256).max(10);
    let stamp = |px: &mut [Argb], cx16: i32, cy16: i32, rad: i32, c: Argb| {
        let (x0, x1) = ((cx16 - rad) >> 4, (cx16 + rad) >> 4);
        let (y0, y1) = ((cy16 - rad) >> 4, (cy16 + rad) >> 4);
        for y in y0.max(0)..=y1.min(i32::from(h) - 1) {
            for x in x0.max(0)..=x1.min(i32::from(w) - 1) {
                let dx = x * 16 + 8 - cx16;
                let dy = y * 16 + 8 - cy16;
                let d = dx * dx + dy * dy;
                let r2 = rad * rad;
                if d < r2 {
                    // Coverage falls off over the last pixel of the radius.
                    let edge = (r2 - d) * 255 / (r2 - (rad - 16).max(0).pow(2)).max(1);
                    let a = edge.clamp(0, 255) as u32 * (c >> 24) / 255;
                    let i = (y * i32::from(w) + x) as usize;
                    let old = px[i] >> 24;
                    if a > old {
                        px[i] = (c & 0x00FF_FFFF) | (a << 24);
                    }
                }
            }
        }
    };
    for (si, st) in strokes.iter().enumerate() {
        if st.is_empty() {
            continue;
        }
        let style = styles.get(si).copied().unwrap_or(0);
        let marker = style & MARKER != 0;
        let c = only.unwrap_or(if marker {
            0x70FF_E040
        } else {
            PEN[usize::from(style & 7).min(4)]
        });
        let rad = if marker { base_rad * 3 } else { base_rad };
        let p = |i: isize| {
            let k = i.clamp(0, st.len() as isize - 1) as usize;
            (i32::from(st[k].0) * s / 16, i32::from(st[k].1) * s / 16)
        };
        for i in 0..st.len() as isize - 1 {
            let (p0, p1, p2, p3) = (p(i - 1), p(i), p(i + 1), p(i + 2));
            // Steps by the segment's length, a stamp every half pixel.
            let len = ((p2.0 - p1.0).abs() + (p2.1 - p1.1).abs()) / 8 + 2;
            for t in 0..len {
                let t = t * 256 / len;
                let t2 = t * t / 256;
                let t3 = t2 * t / 256;
                let cr = |a: i32, b: i32, c: i32, d: i32| {
                    (2 * b * 256
                        + (c - a) * t
                        + (2 * a - 5 * b + 4 * c - d) * t2
                        + (3 * b - a - 3 * c + d) * t3)
                        / 512
                };
                stamp(
                    px,
                    cr(p0.0, p1.0, p2.0, p3.0),
                    cr(p0.1, p1.1, p2.1, p3.1),
                    rad,
                    c,
                );
            }
        }
    }
}

/// Paints the ink of note `id` from the store into a new picture of
/// `w` x `h` in `c`.
fn ink_pic(cx: &Ctx, id: u32, w: u16, h: u16, c: Argb) -> Option<Pic> {
    let pics = cx.pics?;
    let guard = cx.pim?.try_borrow().ok()?;
    let store = guard.as_ref()?;
    let rec = store.get(id)?;
    let mut pts = [(0i16, 0i16); POINTS];
    let mut ends = [0u16; STROKES];
    let mut styles = [0u8; STROKES];
    let ns = decode_ink(rec.text(tag::INK), &mut pts, &mut ends, &mut styles);
    let mut strokes: [&[(i16, i16)]; STROKES] = [&[]; STROKES];
    let mut from = 0usize;
    for (k, &e) in ends[..ns].iter().enumerate() {
        strokes[k] = &pts[from..usize::from(e)];
        from = usize::from(e);
    }
    let p = pics.alloc(w, h)?;
    // The open note's ink in its strokes' own colours; a card's in one.
    let only = (w < 448).then_some(c);
    pics.write(p, |px| {
        paint_styled(px, w, h, &strokes[..ns], &styles[..ns], only)
    });
    Some(p)
}

impl Notes {
    /// No notes until the store gives them (`_samples` is the previews'
    /// old way in: the store now holds them).
    #[must_use]
    pub fn new(_samples: &'static [Note]) -> Self {
        Self {
            tool: Tool::Pen,
            pen: 0,
            live: [(0, 0); LIVE],
            live_n: 0,
            ink_at: None,
            checks: [(Rect::new(0, 0, 0, 0), 0); 12],
            n_checks: 0,
            typing: false,
            start_new: false,
            query: [0; 32],
            query_n: 0,
            searching: false,
            as_list: false,
            menu: false,
            confirm_delete: false,
            cards: [(Rect::new(0, 0, 0, 0), 0); 12],
            n_cards: 0,
            rows: [None; 24],
            seen: None,
            view: View::Home,
            ink: None,
            minis: [None; 4],
            ring: None,
            full: None,
            painted: None,
        }
    }

    /// Shows `view`.
    pub fn show(&mut self, view: View) {
        self.view = view;
    }

    /// The notes copied again from the store when its records changed.
    fn refresh(&mut self, cx: &Ctx) {
        let Some(cell) = cx.pim else { return };
        let Ok(guard) = cell.try_borrow() else { return };
        let Some(store) = guard.as_ref() else { return };
        if self.seen == Some(store.changes()) {
            return;
        }
        self.seen = Some(store.changes());
        self.rows = [None; 24];
        for (slot, r) in self.rows.iter_mut().zip(store.all(Kind::Note)) {
            *slot = Some(Row::of(&r));
        }
        self.painted = None;
        // The pictures painted from the old copy go; the new ones are
        // painted from this one.
        if let Some(pics) = cx.pics {
            for p in self.minis.iter_mut().chain(core::iter::once(&mut self.ink)) {
                if let Some(pic) = p.take() {
                    pics.free(pic);
                }
            }
        }
    }

    /// The search typed: each note found when its title or text holds it.
    fn find(&mut self, cx: &Ctx) {
        let q = &self.query[..self.query_n];
        let Some(cell) = cx.pim else { return };
        let Ok(guard) = cell.try_borrow() else { return };
        let Some(store) = guard.as_ref() else { return };
        for row in self.rows.iter_mut().flatten() {
            row.found = q.is_empty()
                || store.get(row.id).is_some_and(|r| {
                    let has = |t: &[u8]| {
                        t.len() >= q.len() && t.windows(q.len()).any(|w| w.eq_ignore_ascii_case(q))
                    };
                    has(r.text(tag::TITLE)) || has(r.text(tag::TEXT))
                });
        }
    }

    fn paint(&mut self, cx: &Ctx) {
        let key = (self.view, self.seen.unwrap_or(0));
        if self.painted == Some(key) {
            return;
        }
        let Some(pics) = cx.pics else { return };
        if self.ring.is_none() {
            self.ring = kit::disc(pics, 24, 0, Some(tint::NOTES));
            self.full = kit::disc(pics, 24, tint::NOTES, None);
        }
        match self.view {
            View::Home => {
                // Pinned first, as the grid shows them.
                let mut k = 0;
                for want in [true, false] {
                    for row in self
                        .rows
                        .iter()
                        .flatten()
                        .filter(|r| r.pinned == want && r.has_ink())
                    {
                        if k < self.minis.len() {
                            if self.minis[k].is_none() {
                                self.minis[k] = ink_pic(cx, row.id, 212, 84, 0xFF20_2A38);
                            }
                            k += 1;
                        }
                    }
                }
            }
            View::Note(i) => {
                if let Some(row) = self.rows.get(usize::from(i)).copied().flatten()
                    && row.has_ink()
                    && self.ink.is_none()
                {
                    self.ink = ink_pic(cx, row.id, 448, 196, 0xFF1C_3A6E);
                }
            }
        }
        self.painted = Some(key);
    }

    fn home(&mut self, f: &mut Frame, cx: &Ctx) {
        self.n_cards = 0;
        let mut s = [0u8; 16];
        let n = self.rows.iter().flatten().count();
        let mut k = 0;
        if n >= 10 {
            s[k] = b'0' + (n / 10) as u8;
            k += 1;
        }
        s[k] = b'0' + (n % 10) as u8;
        k += 1;
        for &c in b" notes" {
            s[k] = c;
            k += 1;
        }
        let mut y = kit::large_title(f, cx, 36, b"Notes", &s[..k], tint::NOTES);
        let field = SEARCH;
        kit::panel(f, cx, field);
        let q = &self.query[..self.query_n];
        if self.searching {
            f.push(rp_app::Cmd::Outline {
                r: field,
                color: kit::FOCUS,
            });
        }
        if q.is_empty() {
            kit::text(
                f,
                field.x + 12,
                field.y + 25,
                cx.fonts.ui,
                DIM,
                b"Search notes",
            );
        } else {
            kit::text(f, field.x + 12, field.y + 25, cx.fonts.ui, TEXT, q);
        }
        kit::chips(
            f,
            cx,
            LAYOUT,
            &[b"GRID", b"LIST"],
            usize::from(self.as_list),
            tint::NOTES,
        );
        y += 48;
        if self.as_list {
            // One row a note: its colour, title and first line.
            for place in 0..self.rows.len() {
                // One row at a time off the stack (a copy of all 24 was
                // 17 KB: the image's stack overflowed, emu-apps 2026-11-14).
                let Some(note) = self.rows[place].filter(|n| n.found) else {
                    continue;
                };
                if y + 50 > 596 {
                    break;
                }
                let r = Rect::new(MARGIN, y, WIDE, 46);
                kit::tinted(f, cx, r, note.colour, 0x40);
                kit::fitted(
                    f,
                    r.x + 22,
                    r.y + 21,
                    cx.fonts.ui,
                    TEXT,
                    if note.title.is_empty() {
                        b"New note"
                    } else {
                        note.title.get()
                    },
                    300,
                );
                let first = note.lines[..note.n].iter().find_map(|l| match l {
                    Line::Text(t) | Line::Check(t, _) if !t.is_empty() => Some(*t),
                    _ => None,
                });
                if let Some(t) = first {
                    kit::fitted(f, r.x + 22, r.y + 41, cx.fonts.ui, DIM, t.get(), 380);
                }
                if self.n_cards < self.cards.len() {
                    self.cards[self.n_cards] = (r, place as u8);
                    self.n_cards += 1;
                }
                y += 50;
            }
            return;
        }
        // Pinned first, then the rest, two columns, heights by content.
        let mut col_y = [y, y];
        let mut mini = 0;
        // Pinned first, then the rest, each with its place in the store's
        // order (what View::Note names).
        // The order as indices (pinned first), each row read in turn: a
        // copy of all the rows was 17 KB on the stack.
        let mut order = [0u8; 24];
        let mut no = 0;
        for want in [true, false] {
            for (k, r) in self.rows.iter().enumerate() {
                if r.is_some_and(|r| r.pinned == want && r.found) {
                    order[no] = k as u8;
                    no += 1;
                }
            }
        }
        for &place in &order[..no] {
            let Some(note) = self.rows[usize::from(place)] else {
                continue;
            };
            let note = &note;
            let c = if col_y[0] <= col_y[1] { 0 } else { 1 };
            let x = MARGIN + c as u16 * 228;
            let ink = note.has_ink();
            let lines = note.lines[..note.n]
                .iter()
                .filter(|b| !matches!(b, Line::Ink))
                .count()
                .min(3) as u16;
            let h = 40 + lines * 22 + if ink { 88 } else { 0 };
            if col_y[c] + h > 596 {
                continue;
            }
            let r = Rect::new(x, col_y[c], 220, h);
            if self.n_cards < self.cards.len() {
                self.cards[self.n_cards] = (r, place);
                self.n_cards += 1;
            }
            kit::tinted(f, cx, r, note.colour, 0x40);
            kit::fitted(
                f,
                r.x + 12,
                r.y + 26,
                cx.fonts.ui,
                TEXT,
                note.title.get(),
                170,
            );
            if note.pinned {
                kit::right(
                    f,
                    r.right() - 10,
                    r.y + 22,
                    cx.fonts.label,
                    note.colour,
                    b"PIN",
                );
            }
            let mut ly = r.y + 48;
            for b in note.lines[..note.n].iter().take(4) {
                match b {
                    Line::Text(t) => {
                        kit::fitted(f, r.x + 12, ly, cx.fonts.ui, DIM, t.get(), 196);
                        ly += 22;
                    }
                    Line::Check(t, done) => {
                        let mark: &[u8] = if *done { b"[x] " } else { b"[ ] " };
                        kit::text(f, r.x + 12, ly, cx.fonts.ui, DIM, mark);
                        kit::fitted(f, r.x + 44, ly, cx.fonts.ui, DIM, t.get(), 164);
                        ly += 22;
                    }
                    Line::Ink => {
                        if let Some(p) = self.minis.get(mini).copied().flatten() {
                            f.push(rp_app::Cmd::Fill {
                                r: Rect::new(r.x + 4, ly - 14, 212, 84),
                                color: 0xC0F4_EFE0,
                            });
                            f.push(rp_app::Cmd::Pic {
                                x: r.x + 4,
                                y: (ly - 14) as i16,
                                pic: p,
                            });
                        }
                        mini += 1;
                        ly += 88;
                    }
                }
            }
            col_y[c] += h + 8;
        }
    }

    fn note(&mut self, f: &mut Frame, cx: &Ctx, i: u8) {
        self.n_checks = 0;
        self.ink_at = None;
        let Some(note) = self.rows.get(usize::from(i)).copied().flatten() else {
            return;
        };
        // The paper: warm, ruled every 28 px.
        let paper = PAPER;
        f.push(rp_app::Cmd::Fill {
            r: paper,
            color: 0xF0F6_F0DC,
        });
        f.push(rp_app::Cmd::Fill {
            r: Rect::new(paper.x, paper.y, 6, paper.h),
            color: note.colour,
        });
        // Ruled, plain or grid paper (the note's choice, Menu).
        if note.paper != 1 {
            for k in 0..17 {
                f.push(rp_app::Cmd::Fill {
                    r: Rect::new(paper.x + 20, paper.y + 60 + k * 28, paper.w - 30, 1),
                    color: 0x5070_90B0,
                });
            }
        }
        if note.paper == 2 {
            for k in 0..16 {
                f.push(rp_app::Cmd::Fill {
                    r: Rect::new(paper.x + 20 + k * 28, paper.y + 60, 1, paper.h - 70),
                    color: 0x5070_90B0,
                });
            }
        }
        let ink: Argb = 0xFF20_2A38;
        let (title, tcol): (&[u8], Argb) = if note.title.is_empty() {
            (b"New note", 0x8020_2A38)
        } else {
            (note.title.get(), ink)
        };
        kit::text(f, paper.x + 24, paper.y + 44, cx.fonts.title, tcol, title);
        let mut y = paper.y + 84;
        for (line_k, b) in note.lines[..note.n].iter().enumerate() {
            match b {
                Line::Text(t) => {
                    let t = t.get();
                    kit::fitted(f, paper.x + 24, y, cx.fonts.ui, ink, t, paper.w - 40);
                    y += 28;
                }
                Line::Check(t, done) => {
                    let t = t.get();
                    if self.n_checks < self.checks.len() {
                        let line = line_k;
                        self.checks[self.n_checks] = (
                            Rect::new(paper.x + 16, y - 24, paper.w - 30, 30),
                            line as u8,
                        );
                        self.n_checks += 1;
                    }
                    kit::round_check(f, cx, paper.x + 24, y - 18, self.ring, self.full, *done);
                    let col = if *done { 0x9020_2A38 } else { ink };
                    kit::text(f, paper.x + 58, y, cx.fonts.ui, col, t);
                    if *done {
                        f.push(rp_app::Cmd::Fill {
                            r: Rect::new(paper.x + 58, y - 7, cx.fonts.ui.width(t), 1),
                            color: col,
                        });
                    }
                    y += 28;
                }
                Line::Ink => {
                    self.ink_at = Some((paper.x + 8, y - 18));
                    if let Some(p) = self.ink {
                        f.push(rp_app::Cmd::Pic {
                            x: paper.x + 8,
                            y: (y - 18) as i16,
                            pic: p,
                        });
                    }
                    y += 196;
                }
            }
        }
        if self.ink_at.is_none() && y + 40 < STRIP.y {
            // Writing below the lines starts the note's ink panel there.
            self.ink_at = Some((paper.x + 8, y - 18));
        }
        // The stroke being written, as the pen goes (painted into the
        // panel's picture when the pen lifts).
        if let Some((ix, iy)) = self.ink_at {
            let c = if self.tool == Tool::Marker {
                0x70FF_E040
            } else {
                PEN[usize::from(self.pen).min(4)]
            };
            for &(x, yy) in &self.live[..self.live_n] {
                f.push(rp_app::Cmd::Fill {
                    r: Rect::new(ix + x as u16, iy + yy as u16, 3, 3),
                    color: c,
                });
            }
        }
        // The menu, when open, over the tool strip.
        if self.menu {
            kit::panel(f, cx, STRIP);
            let words: [&[u8]; 4] = [
                if note.pinned { b"UNPIN" } else { b"PIN" },
                b"COLOUR",
                [&b"RULED"[..], b"PLAIN", b"GRID"][usize::from(note.paper)],
                if self.confirm_delete {
                    b"DELETE?"
                } else {
                    b"DELETE"
                },
            ];
            for (k, w) in words.iter().enumerate() {
                let r = Rect::new(STRIP.x + 6 + k as u16 * 114, STRIP.y + 6, 108, 36);
                kit::key(
                    f,
                    cx,
                    r,
                    if k == 3 {
                        kit::KeyState::Down
                    } else {
                        kit::KeyState::Rest
                    },
                );
                kit::centred(
                    f,
                    r,
                    r.y + 23,
                    cx.fonts.label,
                    if k == 3 { kit::LATE } else { TEXT },
                    w,
                );
            }
            return;
        }
        // The tool strip.
        let strip = STRIP;
        kit::panel(f, cx, strip);
        let chosen = match self.tool {
            Tool::Pen => 0,
            Tool::Marker => 1,
            Tool::Erase => 2,
            Tool::Text => 3,
            Tool::List => 4,
        };
        for (k, t) in TOOLS.iter().enumerate() {
            let r = Rect::new(strip.x + 6 + k as u16 * 76, strip.y + 6, 70, 36);
            kit::key(
                f,
                cx,
                r,
                if k == chosen {
                    kit::KeyState::On
                } else {
                    kit::KeyState::Rest
                },
            );
            kit::centred(f, r, r.y + 23, cx.fonts.label, TEXT, t);
        }
        // The pen's five colours over the strip's left.
        for (k, c) in PEN.iter().enumerate() {
            let h = if k == usize::from(self.pen) { 10 } else { 6 };
            f.push(rp_app::Cmd::Fill {
                r: Rect::new(strip.x + 14 + k as u16 * 16, strip.y - 4 - h, 12, h),
                color: *c,
            });
        }
    }
}

impl Notes {
    /// Note `id`'s record rewritten: its text and ink given to `change`
    /// (each with room to grow), every other field kept.
    fn rewrite(
        &self,
        cx: &Ctx,
        id: u32,
        change: impl FnOnce(&mut [u8], &mut usize, &mut [u8], &mut usize) -> bool,
    ) -> bool {
        let Some(cell) = cx.pim else { return false };
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return false;
        };
        let Some(store) = guard.as_mut() else {
            return false;
        };
        let Some(r) = store.get(id) else { return false };
        let mut title = [0u8; 64];
        let tl = r.text(tag::TITLE).len().min(64);
        title[..tl].copy_from_slice(&r.text(tag::TITLE)[..tl]);
        let mut text = [0u8; rp_pim::store::TEXT_MAX];
        let mut tn = r.text(tag::TEXT).len().min(text.len());
        text[..tn].copy_from_slice(&r.text(tag::TEXT)[..tn]);
        let mut ink = [0u8; rp_pim::store::TEXT_MAX];
        let mut inn = r.text(tag::INK).len().min(ink.len());
        ink[..inn].copy_from_slice(&r.text(tag::INK)[..inn]);
        let colour = r.num(tag::COLOUR).unwrap_or(1).to_le_bytes();
        let pinned = r.num(tag::PINNED).is_some();
        if !change(&mut text, &mut tn, &mut ink, &mut inn) {
            return false;
        }
        let one = 1u32.to_le_bytes();
        let mut fields: [(u8, &[u8]); 5] = [
            (tag::TITLE, &title[..tl]),
            (tag::TEXT, &text[..tn]),
            (tag::INK, &ink[..inn]),
            (tag::COLOUR, &colour),
            (tag::PINNED, &[]),
        ];
        if pinned {
            fields[4].1 = &one;
        }
        store.put(Kind::Note, Some(id), &fields).is_ok()
    }

    /// The stroke just written added to the note's ink (or, with the
    /// eraser, the stroke it crossed taken out).
    fn end_stroke(&mut self, cx: &Ctx, id: u32) {
        let (live, n, tool, pen) = (self.live, self.live_n, self.tool, self.pen);
        self.live_n = 0;
        if n == 0 {
            return;
        }
        self.rewrite(cx, id, |text, tn, ink, inn| {
            let mut pts = [(0i16, 0i16); POINTS];
            let mut ends = [0u16; STROKES];
            let mut styles = [0u8; STROKES];
            let ns = decode_ink(&ink[..*inn], &mut pts, &mut ends, &mut styles);
            let mut strokes: [&[(i16, i16)]; STROKES] = [&[]; STROKES];
            let mut from = 0usize;
            for (k, &e) in ends[..ns].iter().enumerate() {
                strokes[k] = &pts[from..usize::from(e)];
                from = usize::from(e);
            }
            let mut count = ns;
            if tool == Tool::Erase {
                // The first stroke a point of the pen's path came within
                // 8 px of goes.
                let near = |s: &[(i16, i16)]| {
                    s.iter().any(|&(x, y)| {
                        live[..n]
                            .iter()
                            .any(|&(a, b)| (x - a).abs() < 8 && (y - b).abs() < 8)
                    })
                };
                let Some(k) = strokes[..ns].iter().position(|s| near(s)) else {
                    return false;
                };
                strokes.copy_within(k + 1..ns, k);
                styles.copy_within(k + 1..ns, k);
                count -= 1;
            } else {
                if ns == STROKES {
                    return false;
                }
                strokes[ns] = &live[..n];
                styles[ns] = if tool == Tool::Marker { MARKER } else { pen };
                count += 1;
            }
            match encode_styled(&strokes[..count], &styles[..count], ink) {
                Some(len) => *inn = len,
                None => return false,
            }
            // A note with no ink panel yet gets its place at the end.
            if !text[..*tn].split(|&c| c == b'\n').any(|l| l == [INK_LINE]) && *tn + 2 <= text.len()
            {
                if *tn > 0 {
                    text[*tn] = b'\n';
                    *tn += 1;
                }
                text[*tn] = INK_LINE;
                *tn += 1;
            }
            true
        });
    }

    /// The last stroke taken out (UNDO), or the last character typed.
    fn undo(&mut self, cx: &Ctx, id: u32) {
        self.rewrite(cx, id, |_text, tn, ink, inn| {
            if *inn > 0 {
                let mut pts = [(0i16, 0i16); POINTS];
                let mut ends = [0u16; STROKES];
                let mut styles = [0u8; STROKES];
                let ns = decode_ink(&ink[..*inn], &mut pts, &mut ends, &mut styles);
                let mut strokes: [&[(i16, i16)]; STROKES] = [&[]; STROKES];
                let mut from = 0usize;
                for (k, &e) in ends[..ns].iter().enumerate() {
                    strokes[k] = &pts[from..usize::from(e)];
                    from = usize::from(e);
                }
                let keep = ns.saturating_sub(1);
                *inn = encode_styled(&strokes[..keep], &styles[..keep], ink).unwrap_or(0);
                return true;
            }
            if *tn > 0 {
                *tn -= 1;
                return true;
            }
            false
        });
    }

    /// Text typed into the note (a new line, or a checklist item, started
    /// when the tool was chosen).
    fn type_char(&mut self, cx: &Ctx, id: u32, c: u8) {
        self.rewrite(cx, id, |text, tn, _, _| {
            if *tn < text.len() {
                text[*tn] = c;
                *tn += 1;
                true
            } else {
                false
            }
        });
    }

    /// Checklist line `line` ticked or unticked.
    fn toggle(&mut self, cx: &Ctx, id: u32, line: u8) {
        self.rewrite(cx, id, |text, tn, _, _| {
            let mut at = 0;
            for (k, l) in text[..*tn].split(|&c| c == b'\n').enumerate() {
                if k == usize::from(line) {
                    if l.starts_with(b"[ ] ") {
                        text[at + 1] = b'x';
                    } else if l.starts_with(b"[x] ") {
                        text[at + 1] = b' ';
                    }
                    return true;
                }
                at += l.len() + 1;
            }
            false
        });
    }

    /// A new note, empty, its ink panel ready: its place in the list.
    fn new_note(&mut self, cx: &Ctx) -> Option<u8> {
        let cell = cx.pim?;
        let mut guard = cell.try_borrow_mut().ok()?;
        let store = guard.as_mut()?;
        let colour = 1u32.to_le_bytes();
        let id = store
            .put(
                Kind::Note,
                None,
                &[(tag::TEXT, &[INK_LINE]), (tag::COLOUR, &colour)],
            )
            .ok()?;
        store
            .all(Kind::Note)
            .position(|r| r.id == id)
            .map(|k| k as u8)
    }

    /// Note `id`'s field `which` set to the number `v` (or left out for
    /// `None`), the rest kept.
    fn set_num(&self, cx: &Ctx, id: u32, which: u8, v: Option<u32>) {
        let Some(cell) = cx.pim else { return };
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return;
        };
        let Some(store) = guard.as_mut() else { return };
        let Some(r) = store.get(id) else { return };
        let mut text = [0u8; rp_pim::store::TEXT_MAX];
        let tn = r.text(tag::TEXT).len().min(text.len());
        text[..tn].copy_from_slice(&r.text(tag::TEXT)[..tn]);
        let mut ink = [0u8; rp_pim::store::TEXT_MAX];
        let inn = r.text(tag::INK).len().min(ink.len());
        ink[..inn].copy_from_slice(&r.text(tag::INK)[..inn]);
        let mut title = [0u8; 64];
        let tl = r.text(tag::TITLE).len().min(64);
        title[..tl].copy_from_slice(&r.text(tag::TITLE)[..tl]);
        let mut nums = [
            (tag::COLOUR, r.num(tag::COLOUR)),
            (tag::PINNED, r.num(tag::PINNED)),
            (tag::PAPER, r.num(tag::PAPER)),
        ];
        for n in nums.iter_mut() {
            if n.0 == which {
                n.1 = v;
            }
        }
        let b = nums.map(|(_, v)| v.map(u32::to_le_bytes));
        let mut fields: [(u8, &[u8]); 6] = [
            (tag::TITLE, &title[..tl]),
            (tag::TEXT, &text[..tn]),
            (tag::INK, &ink[..inn]),
            (0, &[]),
            (0, &[]),
            (0, &[]),
        ];
        for (k, (t, _)) in nums.iter().enumerate() {
            if let Some(v) = &b[k] {
                fields[3 + k] = (*t, v);
            }
        }
        let _ = store.put(Kind::Note, Some(id), &fields);
    }

    fn open_id(&self) -> Option<u32> {
        match self.view {
            View::Note(i) => self
                .rows
                .get(usize::from(i))
                .copied()
                .flatten()
                .map(|r| r.id),
            View::Home => None,
        }
    }
}

/// Notes as it first opens (the program's entry and the PC's model).
#[must_use]
pub fn opened(_today: i32) -> Notes {
    Notes::new(&[])
}

/// Notes opened for `intent`: [`rp_app::INTENT_NEW`] (the Today screen's
/// Note button) starts a new note, ready for the pen, on its first frame.
#[must_use]
pub fn opened_for(today: i32, intent: u8) -> Notes {
    let mut n = opened(today);
    n.start_new = intent == rp_app::INTENT_NEW;
    n
}

impl Program for Notes {
    fn title(&self) -> &'static str {
        "Notes"
    }

    fn softkeys(&self) -> (&'static str, &'static str) {
        match self.view {
            View::Home => ("Back", "New"),
            View::Note(_) => ("Back", "Menu"),
        }
    }

    fn draw(&mut self, f: &mut Frame, cx: &Ctx) {
        if core::mem::take(&mut self.start_new)
            && let Some(k) = self.new_note(cx)
        {
            self.view = View::Note(k);
        }
        self.refresh(cx);
        self.find(cx);
        self.paint(cx);
        kit::backdrop(f);
        match self.view {
            View::Home => self.home(f, cx),
            View::Note(i) => self.note(f, cx, i),
        }
    }

    fn event(&mut self, e: Event, cx: &Ctx) -> Reply {
        let id = self.open_id();
        match (e, self.view) {
            (Event::Key(Key::SoftLeft), View::Home) => Reply::Close,
            (Event::Key(Key::SoftLeft), _) => {
                if self.menu {
                    self.menu = false;
                } else if self.typing {
                    self.typing = false;
                } else {
                    self.view = View::Home;
                }
                Reply::Redraw
            }
            (Event::Key(Key::SoftRight), View::Note(_)) => {
                self.menu = !self.menu;
                self.confirm_delete = false;
                Reply::Redraw
            }
            (Event::PenUp { x, y }, View::Note(i)) if self.menu && STRIP.contains_point(x, y) => {
                let k = usize::from((x - STRIP.x - 6) / 114).min(3);
                let Some(row) = self.rows.get(usize::from(i)).copied().flatten() else {
                    return Reply::Nothing;
                };
                match k {
                    0 => self.set_num(cx, row.id, tag::PINNED, (!row.pinned).then_some(1)),
                    1 => {
                        let c = kit::IDENTITY
                            .iter()
                            .position(|&x| x == row.colour)
                            .unwrap_or(1);
                        self.set_num(cx, row.id, tag::COLOUR, Some(((c + 1) % 8) as u32));
                    }
                    2 => self.set_num(cx, row.id, tag::PAPER, Some(u32::from((row.paper + 1) % 3))),
                    _ if self.confirm_delete => {
                        if let Some(cell) = cx.pim
                            && let Ok(mut g) = cell.try_borrow_mut()
                            && let Some(store) = g.as_mut()
                        {
                            let _ = store.remove(row.id);
                        }
                        self.menu = false;
                        self.confirm_delete = false;
                        self.view = View::Home;
                    }
                    _ => self.confirm_delete = true,
                }
                Reply::Redraw
            }
            (Event::PenUp { x, y }, View::Home) if SEARCH.contains_point(x, y) => {
                self.searching = true;
                Reply::Redraw
            }
            (Event::PenUp { x, y }, View::Home) if LAYOUT.contains_point(x, y) => {
                self.as_list = x >= LAYOUT.x + LAYOUT.w / 2;
                Reply::Redraw
            }
            (Event::Char(c), View::Home) if self.searching && self.query_n < self.query.len() => {
                self.query[self.query_n] = c;
                self.query_n += 1;
                Reply::Redraw
            }
            (Event::Delete, View::Home) if self.searching => {
                self.query_n = self.query_n.saturating_sub(1);
                Reply::Redraw
            }
            (Event::Done, View::Home) if self.searching => {
                self.searching = false;
                Reply::Redraw
            }
            (Event::Key(Key::SoftRight), View::Home) => {
                if let Some(k) = self.new_note(cx) {
                    self.view = View::Note(k);
                    self.tool = Tool::Pen;
                }
                Reply::Redraw
            }
            (Event::PenUp { x, y }, View::Home) => {
                // A card opens its note.
                match self.cards[..self.n_cards]
                    .iter()
                    .find(|(r, _)| r.contains_point(x, y))
                {
                    Some(&(_, k)) => {
                        self.view = View::Note(k);
                        self.tool = Tool::Pen;
                        Reply::Redraw
                    }
                    None => Reply::Nothing,
                }
            }
            // The tool strip.
            (Event::PenUp { x, y }, View::Note(_)) if STRIP.contains_point(x, y) => {
                let k = usize::from((x - STRIP.x - 6) / 76).min(5);
                match k {
                    0 if self.tool == Tool::Pen => self.pen = (self.pen + 1) % 5,
                    0 => self.tool = Tool::Pen,
                    1 => self.tool = Tool::Marker,
                    2 => self.tool = Tool::Erase,
                    3 | 4 => {
                        self.tool = if k == 3 { Tool::Text } else { Tool::List };
                        self.typing = true;
                        if let Some(id) = id {
                            let start: &[u8] = if k == 3 { b"\n" } else { b"\n[ ] " };
                            for &c in start {
                                self.type_char(cx, id, c);
                            }
                        }
                    }
                    _ => {
                        if let Some(id) = id {
                            self.undo(cx, id);
                        }
                    }
                }
                Reply::Redraw
            }
            // A checklist item ticked.
            (Event::PenUp { x, y }, View::Note(_))
                if self.live_n == 0
                    && self.checks[..self.n_checks]
                        .iter()
                        .any(|(r, _)| r.contains_point(x, y)) =>
            {
                if let (Some(id), Some(&(_, line))) = (
                    id,
                    self.checks[..self.n_checks]
                        .iter()
                        .find(|(r, _)| r.contains_point(x, y)),
                ) {
                    self.toggle(cx, id, line);
                }
                Reply::Redraw
            }
            // Writing: the pen's points inside the ink panel.
            (Event::PenDown { x, y } | Event::PenMove { x, y }, View::Note(_))
                if matches!(self.tool, Tool::Pen | Tool::Marker | Tool::Erase) =>
            {
                let Some((ix, iy)) = self.ink_at else {
                    return Reply::Nothing;
                };
                if x < ix || y < iy || x >= ix + INK_W || y >= iy + INK_H || y >= STRIP.y {
                    return Reply::Nothing;
                }
                if self.live_n < LIVE {
                    self.live[self.live_n] = ((x - ix) as i16, (y - iy) as i16);
                    self.live_n += 1;
                }
                Reply::Redraw
            }
            (Event::PenUp { .. }, View::Note(_)) if self.live_n > 0 => {
                if let Some(id) = id {
                    self.end_stroke(cx, id);
                }
                Reply::Redraw
            }
            (Event::Char(c), View::Note(_)) if self.typing => {
                if let Some(id) = id {
                    self.type_char(cx, id, c);
                }
                Reply::Redraw
            }
            (Event::Delete, View::Note(_)) if self.typing => {
                if let Some(id) = id {
                    self.undo(cx, id);
                }
                Reply::Redraw
            }
            (Event::Done, View::Note(_)) if self.typing => {
                // Return: a new line, a new item in a checklist.
                if let Some(id) = id {
                    let next: &[u8] = if self.tool == Tool::List {
                        b"\n[ ] "
                    } else {
                        b"\n"
                    };
                    for &c in next {
                        self.type_char(cx, id, c);
                    }
                }
                Reply::Redraw
            }
            _ => Reply::Nothing,
        }
    }

    fn keyboard(&self) -> Option<Field> {
        match self.view {
            View::Home if self.searching => Some(Field::Line),
            View::Note(_) if self.typing => Some(Field::Sentence),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
