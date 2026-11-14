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
    Argb, Ctx, Event, Field, Frame, Key, MARGIN, Pic, Pics, Program, Rect, Reply, W, WIDE,
};

/// The ink panel's picture and a card's miniature (the picture store's
/// classes for Notes).
pub const PIC_CLASSES: [(u16, u16, u8); 2] = [(448, 196, 1), (212, 84, 4)];

/// One block of a note.
#[derive(Clone, Copy, Debug)]
pub enum Block {
    /// A paragraph.
    Text(&'static [u8]),
    /// A checklist item, ticked or not.
    Check(&'static [u8], bool),
    /// Handwriting: strokes as point lists in a 448 x 196 box.
    Ink(&'static [&'static [(i16, i16)]]),
}

/// One note.
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
    /// One note.
    Note(u8),
}

/// The Notes program.
pub struct Notes {
    notes: &'static [Note],
    view: View,
    /// The open note's ink and the cards' miniatures, painted once.
    ink: Option<Pic>,
    minis: [Option<Pic>; 4],
    ring: Option<Pic>,
    full: Option<Pic>,
    painted: Option<View>,
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

impl Notes {
    /// The notes `notes`.
    #[must_use]
    pub fn new(notes: &'static [Note]) -> Self {
        Self {
            notes,
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

    fn ink_of(n: &Note) -> Option<&'static [&'static [(i16, i16)]]> {
        n.blocks.iter().find_map(|b| match b {
            Block::Ink(s) => Some(*s),
            _ => None,
        })
    }

    fn paint(&mut self, pics: Option<&Pics<'static>>) {
        if self.painted == Some(self.view) {
            return;
        }
        let Some(pics) = pics else { return };
        if self.ring.is_none() {
            self.ring = kit::disc(pics, 24, 0, Some(tint::NOTES));
            self.full = kit::disc(pics, 24, tint::NOTES, None);
        }
        match self.view {
            View::Home => {
                let mut k = 0;
                for n in self.notes {
                    if let Some(s) = Self::ink_of(n)
                        && k < self.minis.len()
                    {
                        if let Some(p) = pics.alloc(212, 84) {
                            pics.write(p, |px| paint_ink(px, 212, 84, s, 0xFF20_2A38));
                            self.minis[k] = Some(p);
                        }
                        k += 1;
                    }
                }
            }
            View::Note(i) => {
                if let Some(s) = self.notes.get(usize::from(i)).and_then(Self::ink_of)
                    && let Some(p) = pics.alloc(448, 196)
                {
                    pics.write(p, |px| paint_ink(px, 448, 196, s, 0xFF1C_3A6E));
                    self.ink = Some(p);
                }
            }
        }
        self.painted = Some(self.view);
    }

    fn home(&self, f: &mut Frame, cx: &Ctx) {
        let mut s = [0u8; 16];
        let n = self.notes.len();
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
            .notes
            .iter()
            .filter(|n| n.pinned)
            .chain(self.notes.iter().filter(|n| !n.pinned));
        for note in order {
            let c = if col_y[0] <= col_y[1] { 0 } else { 1 };
            let x = MARGIN + c as u16 * 228;
            let ink = Self::ink_of(note).is_some();
            let lines = note
                .blocks
                .iter()
                .filter(|b| !matches!(b, Block::Ink(_)))
                .count()
                .min(3) as u16;
            let h = 40 + lines * 22 + if ink { 88 } else { 0 };
            if col_y[c] + h > 596 {
                continue;
            }
            let r = Rect::new(x, col_y[c], 220, h);
            kit::tinted(f, cx, r, note.colour, 0x40);
            kit::fitted(f, r.x + 12, r.y + 26, cx.fonts.ui, TEXT, note.title, 170);
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
            for b in note.blocks.iter().take(4) {
                match b {
                    Block::Text(t) => {
                        kit::fitted(f, r.x + 12, ly, cx.fonts.ui, DIM, t, 196);
                        ly += 22;
                    }
                    Block::Check(t, done) => {
                        let mark: &[u8] = if *done { b"[x] " } else { b"[ ] " };
                        kit::text(f, r.x + 12, ly, cx.fonts.ui, DIM, mark);
                        kit::fitted(f, r.x + 44, ly, cx.fonts.ui, DIM, t, 164);
                        ly += 22;
                    }
                    Block::Ink(_) => {
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
        let Some(note) = self.notes.get(usize::from(i)) else {
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
            note.title,
        );
        let mut y = paper.y + 84;
        for b in note.blocks {
            match b {
                Block::Text(t) => {
                    kit::fitted(f, paper.x + 24, y, cx.fonts.ui, ink, t, paper.w - 40);
                    y += 28;
                }
                Block::Check(t, done) => {
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
                Block::Ink(_) => {
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
        self.paint(cx.pics);
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
