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
/// The bytes a note's ink may take: its two fields (`INK`, then
/// `INK_MORE`). No count of strokes or points besides: the ink is read,
/// painted, erased and added to a stroke at a time, straight from its
/// bytes (the first version unpacked all of it into 900 points and 48
/// strokes, and a note stopped taking strokes at the 48th: the owner on
/// the X51v, 2026-11-14).
const INK_ROOM: usize = 2 * rp_pim::store::TEXT_MAX;

/// The packed stroke `ink` starts with: its length in bytes (`None`
/// where the ink ends, or breaks).
fn stroke_len(ink: &[u8]) -> Option<usize> {
    let count = usize::from(*ink.first()?);
    let len = 6 + 2 * count.checked_sub(1)?;
    (len <= ink.len()).then_some(len)
}

/// A packed stroke ([`stroke_len`]'s bytes) as its points: its style and
/// how many.
fn unpack(s: &[u8], pts: &mut [(i16, i16); 255]) -> (u8, usize) {
    let count = usize::from(s[0]);
    let mut x = i16::from_le_bytes([s[2], s[3]]);
    let mut y = i16::from_le_bytes([s[4], s[5]]);
    pts[0] = (x, y);
    for k in 1..count {
        x = x.saturating_add(i16::from(s[4 + 2 * k] as i8));
        y = y.saturating_add(i16::from(s[5 + 2 * k] as i8));
        pts[k] = (x, y);
    }
    (s[1], count)
}

/// Where `ink` parts into its two fields: after the most whole strokes
/// the first holds.
fn split(ink: &[u8]) -> usize {
    let mut at = 0;
    while let Some(n) = stroke_len(&ink[at..]) {
        if at + n > rp_pim::store::TEXT_MAX {
            break;
        }
        at += n;
    }
    at
}

/// The pages a note's ink may go on to past its own record (`Kind::Ink`
/// records naming it, about 4,000 bytes each): a record held it all at
/// first, and a note written full took no more (R206). With these a note
/// holds some 64 KB of strokes, a quarter of the organiser's whole store.
const PAGES: usize = 15;

/// Note `id`'s ink pages, in the order they were added.
fn pages(store: &rp_pim::store::Store<'_>, id: u32) -> ([u32; PAGES], usize) {
    let mut ids = [0u32; PAGES];
    let mut n = 0;
    for r in store.all(Kind::Ink) {
        if r.num(tag::PARENT) == Some(id) && n < PAGES {
            // Kept in order as they come (the store gives none).
            let at = ids[..n].iter().position(|&i| i > r.id).unwrap_or(n);
            ids.copy_within(at..n, at + 1);
            ids[at] = r.id;
            n += 1;
        }
    }
    (ids, n)
}

/// [`pages`] through the app's store (none where it is busy or away).
fn pages_of(cx: &Ctx, id: u32) -> ([u32; PAGES], usize) {
    cx.pim
        .and_then(|c| c.try_borrow().ok())
        .and_then(|g| g.as_ref().map(|s| pages(s, id)))
        .unwrap_or(([0; PAGES], 0))
}

/// A record's ink (its two fields) copied into `ink`: its length.
fn ink_of(r: &rp_pim::store::Rec<'_>, ink: &mut [u8; INK_ROOM]) -> usize {
    let mut inn = 0;
    for field in [r.text(tag::INK), r.text(tag::INK_MORE)] {
        let n = field.len().min(ink.len() - inn);
        ink[inn..inn + n].copy_from_slice(&field[..n]);
        inn += n;
    }
    inn
}

/// The first stroke a point of the pen's `path` came within 8 px of
/// taken out of `ink`: its bytes closed over.
fn erase_in(ink: &mut [u8], inn: &mut usize, path: &[(i16, i16)]) -> bool {
    let near = |s: &[(i16, i16)]| {
        s.iter().any(|&(x, y)| {
            path.iter()
                .any(|&(a, b)| (x - a).abs() < 8 && (y - b).abs() < 8)
        })
    };
    let mut pts = [(0i16, 0i16); 255];
    let mut at = 0;
    while let Some(len) = stroke_len(&ink[at..*inn]) {
        let (_, k) = unpack(&ink[at..at + len], &mut pts);
        if near(&pts[..k]) {
            ink.copy_within(at + len..*inn, at);
            *inn -= len;
            return true;
        }
        at += len;
    }
    false
}

/// The last stroke taken off `ink` (its start is the ink's new end).
fn drop_last(ink: &[u8], inn: &mut usize) -> bool {
    let (mut at, mut last) = (0, 0);
    while let Some(len) = stroke_len(&ink[at..*inn]) {
        last = at;
        at += len;
    }
    let had = *inn > 0;
    *inn = last;
    had
}

/// Ink page `page`'s strokes given to `change` and written back; a page
/// left empty is removed.
fn page_edit(cx: &Ctx, page: u32, change: impl FnOnce(&mut [u8], &mut usize) -> bool) -> bool {
    let Some(mut guard) = cx.pim.and_then(|c| c.try_borrow_mut().ok()) else {
        return false;
    };
    let Some(store) = guard.as_mut() else {
        return false;
    };
    let Some(r) = store.get(page) else {
        return false;
    };
    let mut ink = [0u8; INK_ROOM];
    let mut inn = ink_of(&r, &mut ink);
    if !change(&mut ink, &mut inn) {
        return false;
    }
    if inn == 0 {
        return store.remove(page).is_ok();
    }
    let part = split(&ink[..inn]);
    store
        .change(
            page,
            &[(tag::INK, &ink[..part]), (tag::INK_MORE, &ink[part..inn])],
        )
        .is_ok()
}

/// The pen's six colours (`STYLE` bits 0-2 choose), dark first.
pub const PEN: [Argb; 6] = [
    kit::keep(0xFF20_2A38),
    kit::keep(0xFF1C_3A6E),
    kit::keep(0xFFC0_3030),
    kit::keep(0xFF2E_8B57),
    kit::keep(0xFFFF_9F2E),
    kit::keep(0xFF7A_3FC0),
];
/// A stroke's style bit: the marker (wide, see-through yellow).
pub const MARKER: u8 = 0x08;
/// A stroke's size, style bits 4-5: 0 medium (every stroke written before
/// sizes), 1 fine, 2 bold (the owner, 2026-11-14: "there also is no
/// stroke size control").
pub const SIZE_FINE: u8 = 0x10;
/// See [`SIZE_FINE`].
pub const SIZE_BOLD: u8 = 0x20;

/// A stroke's points thinned as it is kept (Ramer-Douglas-Peucker, a
/// pixel's tolerance): the pen gives a point every few pixels, a line
/// needs its two ends and a curve a few more, so a note holds several
/// times the writing in the same room and the spline brush draws the same
/// stroke. Returns how many of `pts[..n]` are left, in place.
fn simplify(pts: &mut [(i16, i16); LIVE], n: usize) -> usize {
    if n < 3 {
        return n;
    }
    let mut keep = [false; LIVE];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = [(0usize, 0usize); 24];
    stack[0] = (0, n - 1);
    let mut sp = 1;
    while sp > 0 {
        sp -= 1;
        let (a, b) = stack[sp];
        if b <= a + 1 {
            continue;
        }
        let (ax, ay) = (i64::from(pts[a].0), i64::from(pts[a].1));
        let (dx, dy) = (i64::from(pts[b].0) - ax, i64::from(pts[b].1) - ay);
        let len2 = (dx * dx + dy * dy).max(1);
        // The point farthest from the chord (its distance squared times
        // the chord's length squared: no divide, no root).
        let (mut far, mut at) = (0i64, a);
        for (k, q) in pts.iter().enumerate().take(b).skip(a + 1) {
            let cross = dx * (i64::from(q.1) - ay) - dy * (i64::from(q.0) - ax);
            if cross * cross > far {
                far = cross * cross;
                at = k;
            }
        }
        if far > len2 {
            keep[at] = true;
            if sp + 2 <= stack.len() {
                stack[sp] = (a, at);
                stack[sp + 1] = (at, b);
                sp += 2;
            } else {
                // No room to split further: keep the span as it is.
                keep[a..=b].iter_mut().for_each(|k| *k = true);
            }
        }
    }
    let mut m = 0;
    for k in 0..n {
        if keep[k] {
            pts[m] = pts[k];
            m += 1;
        }
    }
    m
}

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

/// The home's search field, and its filters (the foot's tabs).
const SEARCH: Rect = Rect::new(MARGIN, 44, WIDE, 36);
const FILTERS: [&[u8]; 4] = [b"All", b"Pinned", b"Lists", b"Ink"];
/// Ink on paper: the same in both looks.
const INK: Argb = kit::keep(0xFF20_2A38);
const INK_DIM: Argb = kit::keep(0xFF4E_5664);
const INK_DONE: Argb = kit::keep(0xFF8A_8F98);

/// A note's paper: a pastel of its colour, the same in both looks (paper
/// is paper; version 2's cards were grey glass with a colour bar).
fn paper_of(c: Argb) -> Argb {
    kit::keep(kit::mix(0xFFFF_FAF0, c, 0x50))
}

/// The note's tool strip, its tools in order, and the pen colours' row.
const STRIP: Rect = Rect::new(8, 548, W - 16, 48);
/// The palette tray (design-language.md version 3): the glass under the
/// sheet holding the colour wells, the stroke sizes and, at its foot, the
/// tool strip.
const TRAY: Rect = Rect::new(8, 450, W - 16, 146);
/// Colour well `i` (40 px, a thumb wide: the owner, 2026-11-14, "the pen
/// and marker colors are hard to select").
const fn well(i: u16) -> Rect {
    Rect::new(22 + i * 48, 482, 40, 40)
}
/// Size key `i` (fine, medium, bold).
const fn size_key(i: u16) -> Rect {
    Rect::new(322 + i * 48, 480, 44, 44)
}
const TOOLS: [&[u8]; 6] = [b"PEN", b"MARKER", b"ERASE", b"TEXT", b"LIST", b"UNDO"];
const PAPER: Rect = Rect::new(8, 40, W - 16, 404);
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
    /// The stroke size: 0 fine, 1 medium, 2 bold.
    size: u8,
    /// The colour wells' discs.
    wells: [Option<Pic>; 6],
    /// The last stroke didn't fit the note's ink (said on the tray until
    /// one fits again: never dropped silently).
    ink_full: bool,
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
    /// The home's filter tab; the first note of its page, the pages
    /// before it, the note after its last; the D-pad's note (shown once a
    /// key was used); where the stylus went down.
    filter: u8,
    first: u8,
    pages: [u8; 12],
    n_pages: usize,
    next: u8,
    sel: u8,
    /// The D-pad's target in an open note (R203): its tools, colours,
    /// sizes and tick boxes, or the menu's keys.
    focus: kit::Focus,
    keys: bool,
    down_y: Option<u16>,
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
    minis: [Option<(u32, Pic)>; 4],
    /// The miniatures this frame drew (a bit each).
    mini_used: u8,
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
    paint_over(px, w, h, strokes, styles, only);
}

/// [`paint_styled`] over what is there (the stroke being written, a
/// segment at a time: the same brush as the saved ink, so what is written
/// is what is kept).
pub fn paint_over(
    px: &mut [Argb],
    w: u16,
    h: u16,
    strokes: &[&[(i16, i16)]],
    styles: &[u8],
    only: Option<Argb>,
) {
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
            kit::keep(0x70FF_E040)
        } else {
            PEN[usize::from(style & 7).min(5)]
        });
        let rad = match style & (SIZE_FINE | SIZE_BOLD) {
            SIZE_FINE => base_rad * 5 / 8,
            SIZE_BOLD => base_rad * 2,
            _ => base_rad,
        };
        let rad = if marker { rad * 3 } else { rad };
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
    let (page, np) = pages(store, id);
    let p = pics.alloc(w, h)?;
    // The open note's ink in its strokes' own colours; a card's in one.
    let only = (w < 448).then_some(c);
    pics.write(p, |px| {
        px.iter_mut().for_each(|q| *q = 0);
        let mut pts = [(0i16, 0i16); 255];
        let recs = core::iter::once(rec).chain(page[..np].iter().filter_map(|&i| store.get(i)));
        for field in recs.flat_map(|r| [r.text(tag::INK), r.text(tag::INK_MORE)]) {
            let mut at = 0;
            while let Some(n) = stroke_len(&field[at..]) {
                let (style, k) = unpack(&field[at..at + n], &mut pts);
                paint_over(px, w, h, &[&pts[..k]], &[style], only);
                at += n;
            }
        }
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
            size: 1,
            wells: [None; 6],
            ink_full: false,
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
            filter: 0,
            first: 0,
            pages: [0; 12],
            n_pages: 0,
            next: 0,
            sel: 0,
            focus: kit::Focus::default(),
            keys: false,
            down_y: None,
            menu: false,
            confirm_delete: false,
            cards: [(Rect::new(0, 0, 0, 0), 0); 12],
            n_cards: 0,
            rows: [None; 24],
            seen: None,
            view: View::Home,
            ink: None,
            minis: [None; 4],
            mini_used: 0,
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
            for m in &mut self.minis {
                if let Some((_, pic)) = m.take() {
                    pics.free(pic);
                }
            }
            if let Some(pic) = self.ink.take() {
                pics.free(pic);
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
            for (w, c) in self.wells.iter_mut().zip(PEN) {
                *w = kit::gloss_disc(pics, 40, c | 0xFF00_0000);
            }
        }
        match self.view {
            // The home's miniatures are painted as its pages show them
            // (`mini`).
            View::Home => {}
            View::Note(i) => {
                if let Some(row) = self.rows.get(usize::from(i)).copied().flatten()
                    && row.has_ink()
                    && self.ink.is_none()
                {
                    self.ink = ink_pic(cx, row.id, kit::INK_PIC.0, kit::INK_PIC.1, 0xFF1C_3A6E);
                }
            }
        }
        self.painted = Some(key);
    }

    /// Whether the home shows `r`: the search typed and the filter tab.
    fn shows(&self, r: &Row) -> bool {
        r.found
            && match self.filter {
                1 => r.pinned,
                2 => r.lines[..r.n].iter().any(|l| matches!(l, Line::Check(..))),
                3 => r.has_ink(),
                _ => true,
            }
    }

    /// The notes the home shows, in its order (pinned first), as their
    /// places in the store's order (what `View::Note` names). Indices: a
    /// copy of all the rows was 17 KB on the stack.
    fn order(&self) -> ([u8; 24], usize) {
        let mut order = [0u8; 24];
        let mut no = 0;
        for want in [true, false] {
            for (k, r) in self.rows.iter().enumerate() {
                if r.as_ref()
                    .is_some_and(|r| r.pinned == want && self.shows(r))
                {
                    order[no] = k as u8;
                    no += 1;
                }
            }
        }
        (order, no)
    }

    /// The home from its first page (the search or the filter changed).
    fn first_page(&mut self) {
        self.first = 0;
        self.n_pages = 0;
        self.sel = 0;
    }

    /// The next page of the home, if there is one.
    fn page_on(&mut self) -> bool {
        let (_, no) = self.order();
        if usize::from(self.next) >= no || self.next <= self.first {
            return false;
        }
        if self.n_pages < self.pages.len() {
            self.pages[self.n_pages] = self.first;
            self.n_pages += 1;
        }
        self.first = self.next;
        self.sel = self.first;
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
        self.sel = self.first;
        true
    }

    /// The miniature of note `id`'s ink: painted the first time a page
    /// shows it, kept by the note's id (by position, a note the search
    /// left out gave its neighbour the wrong drawing), its slot given back
    /// when a later page needs one.
    fn mini(&mut self, cx: &Ctx, id: u32) -> Option<Pic> {
        if let Some(k) = self
            .minis
            .iter()
            .position(|m| m.is_some_and(|(i, _)| i == id))
        {
            self.mini_used |= 1 << k;
            return self.minis[k].map(|(_, p)| p);
        }
        let k = (0..self.minis.len()).find(|&k| self.mini_used & (1 << k) == 0)?;
        if let (Some((_, old)), Some(pics)) = (self.minis[k].take(), cx.pics) {
            pics.free(old);
        }
        let p = ink_pic(cx, id, kit::INK_MINI.0, kit::INK_MINI.1, 0xFF20_2A38)?;
        self.minis[k] = Some((id, p));
        self.mini_used |= 1 << k;
        Some(p)
    }

    /// A note's lines on its paper from `y` down, at most `most`, `max`
    /// wide: text, tick boxes drawn as boxes. Returns the next line's y.
    fn paper_lines(
        f: &mut Frame,
        cx: &Ctx,
        note: &Row,
        x: u16,
        mut y: u16,
        max: u16,
        most: usize,
    ) -> u16 {
        let lines = note.lines[..note.n]
            .iter()
            .filter(|l| !matches!(l, Line::Ink))
            .take(most);
        for l in lines {
            match l {
                Line::Text(t) => kit::fitted(f, x, y, cx.fonts.ui, INK_DIM, t.get(), max),
                Line::Check(t, done) => {
                    f.push(rp_app::Cmd::Outline {
                        r: Rect::new(x, y - 13, 13, 13),
                        color: INK_DIM,
                    });
                    if *done {
                        f.push(rp_app::Cmd::Fill {
                            r: Rect::new(x + 3, y - 10, 7, 7),
                            color: INK_DIM,
                        });
                    }
                    let col = if *done { INK_DONE } else { INK_DIM };
                    kit::fitted(f, x + 20, y, cx.fonts.ui, col, t.get(), max - 20);
                }
                Line::Ink => {}
            }
            y += 22;
        }
        y
    }

    fn card_hit(&mut self, r: Rect, place: u8) {
        if self.n_cards < self.cards.len() {
            self.cards[self.n_cards] = (r, place);
            self.n_cards += 1;
        }
    }

    /// The home (design language version 3): search first; the pinned
    /// note leading as a whole sheet with its writing; the rest as sheets
    /// of their own paper, two columns, heights by content; the filters as
    /// tabs along the foot. More notes than a page: a drag or the D-pad
    /// turns the page (no note is out of reach, as the grid's overflow
    /// was).
    fn home(&mut self, f: &mut Frame, cx: &Ctx) {
        self.n_cards = 0;
        self.mini_used = 0;
        let (order, no) = self.order();
        if usize::from(self.first) >= no {
            self.first_page();
        }
        let field = SEARCH;
        kit::panel(f, cx, field);
        if self.searching {
            f.push(rp_app::Cmd::Outline {
                r: field,
                color: kit::FOCUS,
            });
        }
        let q = &self.query[..self.query_n];
        if q.is_empty() {
            let mut s = [0u8; 24];
            let mut k = 0;
            for &c in b"Search " {
                s[k] = c;
                k += 1;
            }
            let all = self.rows.iter().flatten().count();
            if all >= 10 {
                s[k] = b'0' + (all / 10) as u8;
                k += 1;
            }
            s[k] = b'0' + (all % 10) as u8;
            k += 1;
            for &c in if all == 1 { &b" note"[..] } else { b" notes" } {
                s[k] = c;
                k += 1;
            }
            kit::text(f, field.x + 14, field.y + 25, cx.fonts.ui, DIM, &s[..k]);
        } else {
            kit::text(f, field.x + 14, field.y + 25, cx.fonts.ui, TEXT, q);
        }
        let bottom = kit::FOOT.y - 6;
        let mut y = SEARCH.y + SEARCH.h + 8;
        let mut at = usize::from(self.first);
        let focus = |f: &mut Frame, r: Rect| {
            f.push(rp_app::Cmd::Outline {
                r: Rect::new(r.x - 2, r.y - 2, r.w + 4, r.h + 4),
                color: kit::FOCUS,
            });
        };
        // The pinned note leads the first page as a whole sheet.
        if at == 0
            && no > 0
            && let Some(note) = self.rows[usize::from(order[0])].filter(|n| n.pinned)
        {
            let r = Rect::new(MARGIN, y, WIDE, 154);
            let sheet = kit::keep(0xFFF8_F0D8);
            // Its binding in the note's colour, the sheet's rounded edge.
            kit::round_rect(
                f,
                cx,
                r,
                kit::keep(kit::mix(note.colour, 0xFF00_0000, 0x30)),
            );
            kit::round_rect(f, cx, Rect::new(r.x + 10, r.y, r.w - 10, r.h), sheet);
            f.push(rp_app::Cmd::Fill {
                r: Rect::new(r.x + 10, r.y, 14, r.h),
                color: sheet,
            });
            for k in 0..4u16 {
                f.push(rp_app::Cmd::Fill {
                    r: Rect::new(r.x + 24, r.y + 64 + k * 22, r.w - 40, 1),
                    color: kit::keep(0xFFC4_D4E4),
                });
            }
            kit::text(
                f,
                r.x + 24,
                r.y + 22,
                cx.fonts.label,
                kit::keep(kit::mix(note.colour, 0xFF00_0000, 0x60)),
                b"PINNED",
            );
            let title: &[u8] = if note.title.is_empty() {
                b"New note"
            } else {
                note.title.get()
            };
            kit::fitted(f, r.x + 24, r.y + 52, cx.fonts.title, INK, title, r.w - 48);
            let ink = note.has_ink();
            Self::paper_lines(
                f,
                cx,
                &note,
                r.x + 24,
                r.y + 82,
                if ink { 196 } else { r.w - 48 },
                3,
            );
            if ink && let Some(p) = self.mini(cx, note.id) {
                f.push(rp_app::Cmd::Pic {
                    x: r.x + r.w - kit::INK_MINI.0 - 8,
                    y: (r.y + 62) as i16,
                    pic: p,
                });
            }
            if self.keys && self.sel == 0 {
                focus(f, r);
            }
            self.card_hit(r, order[0]);
            y += r.h + 8;
            at = 1;
        }
        // The rest, each on its own paper.
        let mut col_y = [y, y];
        let mut next = at;
        for (k, &place) in order[..no].iter().enumerate().skip(at) {
            let Some(note) = self.rows[usize::from(place)] else {
                continue;
            };
            let c = usize::from(col_y[1] < col_y[0]);
            let mut ink = note.has_ink();
            let mut lines = note.lines[..note.n]
                .iter()
                .filter(|l| !matches!(l, Line::Ink))
                .count()
                .min(3) as u16;
            let mut h = 42 + lines * 22 + if ink { kit::INK_MINI.1 + 4 } else { 0 };
            if col_y[c] + h > bottom {
                // The page's last sheet is cut to the room left (its
                // title and the lines that fit, its drawing on the note
                // itself) rather than leave the room empty.
                let room = bottom.saturating_sub(col_y[c]);
                if room < 64 {
                    break;
                }
                ink = false;
                lines = lines.min((room - 42) / 22);
                h = 42 + lines * 22;
            }
            let r = Rect::new(MARGIN + c as u16 * 228, col_y[c], 220, h);
            let paper = paper_of(note.colour);
            kit::round_rect(f, cx, r, paper);
            // A fold of shade along the foot: a sheet, not a tile.
            f.push(rp_app::Cmd::Fill {
                r: Rect::new(r.x + 10, r.y + r.h - 3, r.w - 20, 2),
                color: kit::keep(kit::mix(paper, 0xFF00_0000, 0x28)),
            });
            let title: &[u8] = if note.title.is_empty() {
                b"New note"
            } else {
                note.title.get()
            };
            let pin = if note.pinned { 40 } else { 0 };
            kit::fitted(
                f,
                r.x + 12,
                r.y + 26,
                cx.fonts.ui,
                INK,
                title,
                r.w - 24 - pin,
            );
            if note.pinned {
                kit::right(
                    f,
                    r.right() - 10,
                    r.y + 22,
                    cx.fonts.label,
                    kit::keep(kit::mix(note.colour, 0xFF00_0000, 0x60)),
                    b"PIN",
                );
            }
            let ly = Self::paper_lines(
                f,
                cx,
                &note,
                r.x + 12,
                r.y + 50,
                r.w - 24,
                usize::from(lines),
            );
            if ink && let Some(p) = self.mini(cx, note.id) {
                f.push(rp_app::Cmd::Pic {
                    x: r.x + 4,
                    y: (ly - 14) as i16,
                    pic: p,
                });
            }
            if self.keys && usize::from(self.sel) == k {
                focus(f, r);
            }
            self.card_hit(r, place);
            col_y[c] += h + 8;
            next = k + 1;
        }
        self.next = next as u8;
        if no == 0 {
            kit::centred(
                f,
                Rect::new(0, 250, W, 30),
                270,
                cx.fonts.ui,
                DIM,
                if self.query_n == 0 {
                    b"No notes here yet: New starts one"
                } else {
                    b"No note holds that"
                },
            );
        } else if next < no || self.first > 0 {
            // Where this page sits: said in the search field's right.
            let mut s = [0u8; 16];
            let mut k = 0;
            let parts: [(usize, &[u8]); 3] = [
                (usize::from(self.first) + 1, b"-"),
                (next, b" of "),
                (no, b""),
            ];
            for (v, sep) in parts {
                if v >= 10 {
                    s[k] = b'0' + (v / 10) as u8;
                    k += 1;
                }
                s[k] = b'0' + (v % 10) as u8;
                k += 1;
                for &c in sep {
                    s[k] = c;
                    k += 1;
                }
            }
            kit::right(
                f,
                field.right() - 14,
                field.y + 24,
                cx.fonts.label,
                DIM,
                &s[..k],
            );
        }
        kit::foot_tabs(f, cx, &FILTERS, usize::from(self.filter), tint::NOTES);
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
            color: kit::keep(0xF0F6_F0DC),
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
                    color: kit::keep(0x5070_90B0),
                });
            }
        }
        if note.paper == 2 {
            for k in 0..16 {
                f.push(rp_app::Cmd::Fill {
                    r: Rect::new(paper.x + 20 + k * 28, paper.y + 60, 1, paper.h - 70),
                    color: kit::keep(0x5070_90B0),
                });
            }
        }
        let ink: Argb = kit::keep(0xFF20_2A38);
        let (title, tcol): (&[u8], Argb) = if note.title.is_empty() {
            (b"New note", kit::keep(0x8020_2A38))
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
                    let col = if *done { kit::keep(0x9020_2A38) } else { ink };
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
        if self.ink_at.is_none() && y + 40 < TRAY.y {
            // Writing below the lines starts the note's ink panel there.
            self.ink_at = Some((paper.x + 8, y - 18));
        }
        // The stroke being written, as the pen goes (painted into the
        // panel's picture when the pen lifts).
        if let Some((ix, iy)) = self.ink_at {
            // A note with no ink yet: the panel's picture holds the
            // stroke being written.
            if !note.lines[..note.n].iter().any(|l| matches!(l, Line::Ink))
                && let Some(p) = self.ink
            {
                f.push(rp_app::Cmd::Pic {
                    x: ix,
                    y: iy as i16,
                    pic: p,
                });
            }
            // Pen and marker strokes are painted into the picture as they
            // go ([`Self::live_paint`]); the eraser's path shows as dots.
            let c = kit::keep(0x80FF_FFFF);
            let shown = if self.tool == Tool::Erase {
                self.live_n
            } else {
                0
            };
            for &(x, yy) in &self.live[..shown] {
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
        // The palette tray: the colour wells (the chosen one in the glow),
        // the three sizes drawn at their size, then the tool strip.
        let strip = STRIP;
        kit::panel(f, cx, TRAY);
        kit::text(f, 24, 472, cx.fonts.label, DIM, b"COLOUR");
        for (i, p) in self.wells.iter().enumerate() {
            let r = well(i as u16);
            if i == usize::from(self.pen) && self.tool != Tool::Marker {
                kit::key(
                    f,
                    cx,
                    Rect::new(r.x - 5, r.y - 5, 50, 50),
                    kit::KeyState::On,
                );
            }
            if let Some(p) = *p {
                f.push(rp_app::Cmd::Pic {
                    x: r.x,
                    y: r.y as i16,
                    pic: p,
                });
            }
        }
        if self.ink_full {
            kit::right(
                f,
                456,
                472,
                cx.fonts.label,
                kit::LATE,
                b"THIS NOTE'S INK IS FULL",
            );
        } else {
            kit::text(f, 326, 472, cx.fonts.label, DIM, b"SIZE");
        }
        for (i, d) in [6u16, 12, 20].iter().enumerate() {
            let r = size_key(i as u16);
            kit::key(
                f,
                cx,
                r,
                if i == usize::from(self.size) {
                    kit::KeyState::On
                } else {
                    kit::KeyState::Rest
                },
            );
            kit::round_rect(
                f,
                cx,
                Rect::new(r.x + (44 - d) / 2, r.y + (44 - d) / 2, *d, *d),
                if cx.look.light {
                    kit::keep(0xFF28_3038)
                } else {
                    TEXT
                },
            );
        }
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
    }
}

impl Notes {
    /// The stroke being written, its newest segment painted into the
    /// note's ink picture with the saved ink's brush (the owner,
    /// 2026-11-14: "the strokes only showed up as dots, not as fluid
    /// strokes"). The last three points: the spline's segment before the
    /// newest is painted again with its neighbour known, the brush keeps
    /// the most coverage, so nothing doubles.
    fn live_paint(&mut self, cx: &Ctx) {
        if self.tool == Tool::Erase {
            return;
        }
        let Some(pics) = cx.pics else { return };
        if self.ink.is_none() {
            let (w, h) = kit::INK_PIC;
            self.ink = pics.alloc(w, h);
            if let Some(p) = self.ink {
                pics.write(p, |px| px.iter_mut().for_each(|q| *q = 0));
            }
        }
        // Renewed before each change: a picture once drawn is final.
        self.ink = self.ink.and_then(|p| pics.renew(p));
        let Some(p) = self.ink else { return };
        let n = self.live_n;
        let pts = &self.live[n.saturating_sub(3)..n];
        let style = self.style();
        pics.write(p, |px| {
            paint_over(px, kit::INK_PIC.0, kit::INK_PIC.1, &[pts], &[style], None);
        });
    }

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
        let mut text = [0u8; rp_pim::store::TEXT_MAX];
        let mut tn = r.text(tag::TEXT).len().min(text.len());
        text[..tn].copy_from_slice(&r.text(tag::TEXT)[..tn]);
        let mut ink = [0u8; INK_ROOM];
        let mut inn = ink_of(&r, &mut ink);
        if !change(&mut text, &mut tn, &mut ink, &mut inn) {
            return false;
        }
        let part = split(&ink[..inn]);
        // Text and ink only: the title, colour, pin, paper and any field
        // a later version adds are kept by the store.
        store
            .change(
                id,
                &[
                    (tag::TEXT, &text[..tn]),
                    (tag::INK, &ink[..part]),
                    (tag::INK_MORE, &ink[part..inn]),
                ],
            )
            .is_ok()
    }

    /// The style of a stroke written now: the pen's colour or the marker,
    /// and the size.
    fn style(&self) -> u8 {
        let size = match self.size {
            0 => SIZE_FINE,
            2 => SIZE_BOLD,
            _ => 0,
        };
        size | if self.tool == Tool::Marker {
            MARKER
        } else {
            self.pen
        }
    }

    /// The stroke just written added to the note's ink (or, with the
    /// eraser, the stroke it crossed taken out).
    fn end_stroke(&mut self, cx: &Ctx, id: u32) {
        let (mut live, tool, style) = (self.live, self.tool, self.style());
        // Thinned as it is kept (the eraser's path as it was: it is only
        // compared).
        let n = if tool == Tool::Erase {
            self.live_n
        } else {
            simplify(&mut live, self.live_n)
        };
        self.live_n = 0;
        if n == 0 {
            return;
        }
        let (page, np) = pages_of(cx, id);
        let kept = if tool == Tool::Erase {
            // The note's own strokes first, then each page's, as they are
            // painted.
            self.rewrite(cx, id, |_, _, ink, inn| erase_in(ink, inn, &live[..n]))
                || page[..np]
                    .iter()
                    .any(|&p| page_edit(cx, p, |ink, inn| erase_in(ink, inn, &live[..n])))
        } else {
            // Packed once (a stroke is 514 bytes at most), then added at
            // the ink's end, nothing else read: the last page's, or the
            // note's own while it has none; where that is full, a new
            // page.
            let mut packed = [0u8; 6 + 2 * 254];
            let Some(len) = encode_styled(&[&live[..n]], &[style], &mut packed) else {
                return;
            };
            let add = |ink: &mut [u8], inn: &mut usize| {
                let Some(to) = ink.get_mut(*inn..*inn + len) else {
                    return false;
                };
                to.copy_from_slice(&packed[..len]);
                *inn += len;
                true
            };
            let on_last = match page[..np].last() {
                Some(&p) => page_edit(cx, p, add),
                None => self.rewrite(cx, id, |text, tn, ink, inn| {
                    // A note with no ink panel yet gets its place at the
                    // end.
                    if !text[..*tn].split(|&c| c == b'\n').any(|l| l == [INK_LINE])
                        && *tn + 2 <= text.len()
                    {
                        if *tn > 0 {
                            text[*tn] = b'\n';
                            *tn += 1;
                        }
                        text[*tn] = INK_LINE;
                        *tn += 1;
                    }
                    add(ink, inn)
                }),
            };
            on_last
                || (np < PAGES
                    && cx
                        .pim
                        .and_then(|c| c.try_borrow_mut().ok())
                        .and_then(|mut g| {
                            g.as_mut()?
                                .put(
                                    Kind::Ink,
                                    None,
                                    &[(tag::PARENT, &id.to_le_bytes()), (tag::INK, &packed[..len])],
                                )
                                .ok()
                        })
                        .is_some())
        };
        // A stroke that didn't fit is said, not dropped in silence (the
        // owner, 2026-11-14: strokes stopped showing with no word why).
        if tool != Tool::Erase {
            self.ink_full = !kept;
        } else if kept {
            self.ink_full = false;
        }
    }

    /// The last stroke taken out (UNDO), or the last character typed.
    fn undo(&mut self, cx: &Ctx, id: u32) {
        // The last stroke is the last page's, or the note's own.
        let (page, np) = pages_of(cx, id);
        if let Some(&p) = page[..np].last() {
            page_edit(cx, p, |ink, inn| drop_last(ink, inn));
            return;
        }
        self.rewrite(cx, id, |_text, tn, ink, inn| {
            if drop_last(ink, inn) {
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
        // Only this field changes: the store keeps every other one.
        let b = v.map(u32::to_le_bytes);
        let _ = store.change(id, &[(which, b.as_ref().map_or(&[][..], |b| &b[..]))]);
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
    fn accent(&self) -> Option<rp_app::Argb> {
        Some(rp_app::kit::tint::NOTES)
    }

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
        match self.view {
            View::Home => self.home(f, cx),
            View::Note(i) => {
                self.note(f, cx, i);
                let (t, n) = self.note_targets();
                self.focus.ring(f, t[..n].iter().copied());
            }
        }
    }

    fn event(&mut self, e: Event, cx: &Ctx) -> Reply {
        let id = self.open_id();
        // The stylus takes over from the D-pad (the centre key's tap is
        // a lift only).
        if matches!(e, Event::PenDown { .. }) {
            self.focus.clear();
        }
        // The D-pad in an open note (R203): the arrows walk what a stylus
        // can tap that is not writing (the tools, the colour wells, the
        // sizes, a list's tick boxes; the menu's keys while it is up), the
        // centre taps it. Windows Mobile's Notes has no keys in a note
        // but the softkeys.
        if let (Event::Key(k), View::Note(_)) = (e, self.view) {
            let (t, n) = self.note_targets();
            if self.focus.step(k, t[..n].iter().copied()) {
                return Reply::Redraw;
            }
            if k == Key::Enter
                && let Some(r) = self.focus.rect()
                && t[..n].contains(&r)
                && let Some((x, y)) = self.focus.point()
            {
                return self.event(Event::PenUp { x, y }, cx);
            }
        }
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
                self.focus.clear();
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
                            // Its ink pages go with it.
                            let (page, np) = pages(store, row.id);
                            for &p in &page[..np] {
                                let _ = store.remove(p);
                            }
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
            (Event::PenDown { y, .. }, View::Home) => {
                self.down_y = Some(y);
                Reply::Nothing
            }
            // A drag up or down turns the page.
            (Event::PenUp { y, .. }, View::Home)
                if self.down_y.is_some_and(|y0| y0.abs_diff(y) > 48) =>
            {
                let up = self.down_y.take().is_some_and(|y0| y < y0);
                let turned = if up { self.page_on() } else { self.page_back() };
                if turned {
                    Reply::Redraw
                } else {
                    Reply::Nothing
                }
            }
            (Event::PenUp { x, y }, View::Home) if kit::FOOT.contains_point(x, y) => {
                self.down_y = None;
                if let Some(i) = (0..FILTERS.len())
                    .find(|&i| kit::foot_tab(FILTERS.len(), i).contains_point(x, y))
                {
                    self.filter = i as u8;
                    self.first_page();
                }
                Reply::Redraw
            }
            // The D-pad: left and right step the filters, up and down the
            // notes (turning the page at its ends), the centre opens.
            (Event::Key(k @ (Key::Left | Key::Right)), View::Home) => {
                let n = FILTERS.len() as u8;
                self.filter = (self.filter + if k == Key::Right { 1 } else { n - 1 }) % n;
                self.first_page();
                Reply::Redraw
            }
            (Event::Key(k @ (Key::Up | Key::Down)), View::Home) => {
                if !self.keys {
                    self.keys = true;
                    self.sel = self.first;
                } else if k == Key::Down {
                    if self.sel + 1 < self.next {
                        self.sel += 1;
                    } else {
                        self.page_on();
                    }
                } else if self.sel > self.first {
                    self.sel -= 1;
                } else {
                    self.page_back();
                }
                Reply::Redraw
            }
            (Event::Key(Key::Enter), View::Home) => {
                let (order, no) = self.order();
                if !self.keys || usize::from(self.sel) >= no {
                    self.keys = true;
                    return Reply::Redraw;
                }
                self.view = View::Note(order[usize::from(self.sel)]);
                self.tool = Tool::Pen;
                Reply::Redraw
            }
            (Event::Char(c), View::Home) if self.searching && self.query_n < self.query.len() => {
                self.query[self.query_n] = c;
                self.query_n += 1;
                self.first_page();
                Reply::Redraw
            }
            (Event::Delete, View::Home) if self.searching => {
                self.query_n = self.query_n.saturating_sub(1);
                self.first_page();
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
            // The palette: a colour well (the pen takes it), a size.
            (Event::PenUp { x, y }, View::Note(_))
                if TRAY.contains_point(x, y) && y < STRIP.y && !self.menu =>
            {
                if let Some(i) = (0..6u16).find(|&i| {
                    let r = well(i);
                    Rect::new(r.x - 4, r.y - 6, 48, 52).contains_point(x, y)
                }) {
                    self.pen = i as u8;
                    if self.tool != Tool::Pen {
                        self.tool = Tool::Pen;
                        self.typing = false;
                    }
                } else if let Some(i) = (0..3u16).find(|&i| size_key(i).contains_point(x, y)) {
                    self.size = i as u8;
                }
                Reply::Redraw
            }
            // The tool strip.
            (Event::PenUp { x, y }, View::Note(_)) if STRIP.contains_point(x, y) => {
                let k = usize::from((x - STRIP.x - 6) / 76).min(5);
                match k {
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
                if x < ix || y < iy || x >= ix + INK_W || y >= iy + INK_H || y >= TRAY.y {
                    return Reply::Nothing;
                }
                if self.live_n < LIVE {
                    self.live[self.live_n] = ((x - ix) as i16, (y - iy) as i16);
                    self.live_n += 1;
                    self.live_paint(cx);
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

impl Notes {
    /// What a stylus can tap in an open note besides writing, for the
    /// D-pad: the menu's four keys while it is up; else the tick boxes,
    /// the colour wells, the sizes and the tools.
    fn note_targets(&self) -> ([Rect; 28], usize) {
        let mut t = [Rect::new(0, 0, 0, 0); 28];
        let mut n = 0;
        let mut add = |r: Rect| {
            if n < t.len() {
                t[n] = r;
                n += 1;
            }
        };
        if self.menu {
            for k in 0..4 {
                add(Rect::new(STRIP.x + 6 + k * 114, STRIP.y + 6, 108, 36));
            }
        } else {
            for (r, _) in &self.checks[..self.n_checks] {
                add(*r);
            }
            for i in 0..6 {
                add(well(i));
            }
            for i in 0..3 {
                add(size_key(i));
            }
            for k in 0..6 {
                add(Rect::new(STRIP.x + 6 + k * 76, STRIP.y + 6, 72, 36));
            }
        }
        (t, n)
    }
}

#[cfg(test)]
mod tests;
