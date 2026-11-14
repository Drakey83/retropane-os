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

/// Packs `strokes` into `out` (the store's `INK` field): per stroke its
/// point count, the first point as two little-endian i16, then each next
/// one as a byte pair of signed steps (a longer step is split, so nothing
/// is lost). The length, or `None` when it doesn't fit.
pub fn encode_ink(strokes: &[&[(i16, i16)]], out: &mut [u8]) -> Option<usize> {
    let mut n = 0;
    let mut put = |b: u8, n: &mut usize| -> Option<()> {
        *out.get_mut(*n)? = b;
        *n += 1;
        Some(())
    };
    for st in strokes.iter().filter(|s| !s.is_empty()) {
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
/// `ends`: the strokes read (a damaged field stops where it breaks).
pub fn decode_ink(ink: &[u8], pts: &mut [(i16, i16)], ends: &mut [u16]) -> usize {
    let (mut at, mut np, mut ns) = (0usize, 0usize, 0usize);
    while at + 5 <= ink.len() && ns < ends.len() {
        let count = usize::from(ink[at]);
        let x = i16::from_le_bytes([ink[at + 1], ink[at + 2]]);
        let y = i16::from_le_bytes([ink[at + 3], ink[at + 4]]);
        at += 5;
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

/// The Notes program.
pub struct Notes {
    rows: [Option<Row>; 24],
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
    px.iter_mut().for_each(|p| *p = 0);
    let sx = i32::from(w) * 256 / 448;
    let sy = i32::from(h) * 256 / 196;
    let s = sx.min(sy);
    // Stamp radius in 1/16 px (at least a pixel's worth).
    let rad = (20 * s / 256).max(10);
    let stamp = |px: &mut [Argb], cx16: i32, cy16: i32| {
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
    for st in strokes {
        if st.len() < 2 {
            continue;
        }
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
                stamp(px, cr(p0.0, p1.0, p2.0, p3.0), cr(p0.1, p1.1, p2.1, p3.1));
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
    let ns = decode_ink(rec.text(tag::INK), &mut pts, &mut ends);
    let mut strokes: [&[(i16, i16)]; STROKES] = [&[]; STROKES];
    let mut from = 0usize;
    for (k, &e) in ends[..ns].iter().enumerate() {
        strokes[k] = &pts[from..usize::from(e)];
        from = usize::from(e);
    }
    let p = pics.alloc(w, h)?;
    pics.write(p, |px| paint_ink(px, w, h, &strokes[..ns], c));
    Some(p)
}

impl Notes {
    /// No notes until the store gives them (`_samples` is the previews'
    /// old way in: the store now holds them).
    #[must_use]
    pub fn new(_samples: &'static [Note]) -> Self {
        Self {
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

    fn home(&self, f: &mut Frame, cx: &Ctx) {
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
        let field = Rect::new(MARGIN, y + 4, WIDE, 36);
        kit::panel(f, cx, field);
        kit::text(
            f,
            field.x + 12,
            field.y + 25,
            cx.fonts.ui,
            DIM,
            b"Search notes",
        );
        y += 48;
        // Pinned first, then the rest, two columns, heights by content.
        let mut col_y = [y, y];
        let mut mini = 0;
        let order = self
            .rows
            .iter()
            .flatten()
            .filter(|n| n.pinned)
            .chain(self.rows.iter().flatten().filter(|n| !n.pinned));
        for note in order {
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

    fn note(&self, f: &mut Frame, cx: &Ctx, i: u8) {
        let Some(note) = self.rows.get(usize::from(i)).copied().flatten() else {
            return;
        };
        // The paper: warm, ruled every 28 px.
        let paper = Rect::new(8, 40, W - 16, 504);
        f.push(rp_app::Cmd::Fill {
            r: paper,
            color: 0xF0F6_F0DC,
        });
        f.push(rp_app::Cmd::Fill {
            r: Rect::new(paper.x, paper.y, 6, paper.h),
            color: note.colour,
        });
        for k in 0..17 {
            f.push(rp_app::Cmd::Fill {
                r: Rect::new(paper.x + 20, paper.y + 60 + k * 28, paper.w - 30, 1),
                color: 0x5070_90B0,
            });
        }
        let ink: Argb = 0xFF20_2A38;
        kit::text(
            f,
            paper.x + 24,
            paper.y + 44,
            cx.fonts.title,
            ink,
            note.title.get(),
        );
        let mut y = paper.y + 84;
        for b in &note.lines[..note.n] {
            match b {
                Line::Text(t) => {
                    let t = t.get();
                    kit::fitted(f, paper.x + 24, y, cx.fonts.ui, ink, t, paper.w - 40);
                    y += 28;
                }
                Line::Check(t, done) => {
                    let t = t.get();
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
        // The tool strip.
        let strip = Rect::new(8, 548, W - 16, 48);
        kit::panel(f, cx, strip);
        let tools: [&[u8]; 6] = [b"PEN", b"MARKER", b"ERASE", b"TEXT", b"LIST", b"UNDO"];
        for (k, t) in tools.iter().enumerate() {
            let r = Rect::new(strip.x + 6 + k as u16 * 76, strip.y + 6, 70, 36);
            if k == 0 {
                kit::tinted(f, cx, r, tint::NOTES, 0xB0);
            }
            kit::centred(f, r, r.y + 23, cx.fonts.label, TEXT, t);
        }
        // The pen's five colours over the strip's left.
        for (k, c) in [
            0xFF20_2A38u32,
            0xFF1C_3A6E,
            0xFFC0_3030,
            0xFF2E_8B57,
            0xFFFF_9F2E,
        ]
        .iter()
        .enumerate()
        {
            f.push(rp_app::Cmd::Fill {
                r: Rect::new(strip.x + 14 + k as u16 * 12, strip.y - 8, 10, 6),
                color: *c,
            });
        }
    }
}

/// Notes as it first opens (the program's entry and the PC's model).
#[must_use]
pub fn opened(_today: i32) -> Notes {
    Notes::new(&[])
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
        self.refresh(cx);
        self.paint(cx);
        kit::backdrop(f);
        match self.view {
            View::Home => self.home(f, cx),
            View::Note(i) => self.note(f, cx, i),
        }
    }

    fn event(&mut self, e: Event, _cx: &Ctx) -> Reply {
        match (e, self.view) {
            (Event::Key(Key::SoftLeft), View::Home) => Reply::Close,
            (Event::Key(Key::SoftLeft), _) => {
                self.view = View::Home;
                Reply::Redraw
            }
            _ => Reply::Nothing,
        }
    }

    fn keyboard(&self) -> Option<Field> {
        None
    }
}
